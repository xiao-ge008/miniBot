//! miniBot 主入口：托盘常驻的桌面 AI 小工具
mod actions;
mod capture;
mod capture_session;
mod clip;
mod commands;
mod engine;
mod gateway;
mod gpu;
mod history;
mod hotkeys;
mod imgsearch;
mod imgutil;
mod jobobj;
mod kb;
mod kb_queue;
mod llm;
mod localsend;
mod ocr;
mod payload;
mod qr;
mod recorder;
mod settings;
mod tray;
mod tts;
mod updater;
mod websearch;
mod winutil;
mod wins;

use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager, RunEvent};

/// 写入运行日志到 %LOCALAPPDATA%\com.minibot.app\logs\app.log，方便排查窗口/剪贴板/热键状态
pub fn log_msg(app: &AppHandle, msg: impl AsRef<str>) {
    let s = msg.as_ref();
    eprintln!("{s}");
    if let Ok(dir) = app.path().app_log_dir() {
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("app.log")) {
            let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
            let _ = writeln!(f, "[{ts}] {s}");
        }
    }
}

pub struct AppState {
    pub settings: settings::SettingsStore,
    pub llm: Arc<llm::LlmManager>,
    pub kb_queue: Arc<kb_queue::KbQueueManager>,
    pub clip: clip::ClipWatcher,
    pub pending: Mutex<HashMap<String, payload::Payload>>,
    pub capture_png: Mutex<Option<Vec<u8>>>,
    pub capture_session: Mutex<capture_session::CaptureLifecycle>,
    pub history: Option<history::History>,
    pub gateway_error: Mutex<Option<String>>,
    pub hotkey_map: Mutex<HashMap<u32, String>>,
    pub hotkey_errors: Mutex<Vec<String>>,
    pub pending_doc: Mutex<Option<(String, Option<String>)>>,
}

/// 解析命令行启动参数中的待编辑 Markdown 文件路径
pub fn extract_edit_path(args: &[String]) -> Option<String> {
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        if arg == "--edit" {
            if let Some(next) = iter.next() {
                if !next.starts_with('-') {
                    return Some(next.clone());
                }
            }
        } else if let Some(stripped) = arg.strip_prefix("--edit=") {
            if !stripped.is_empty() {
                return Some(stripped.to_string());
            }
        } else if !arg.starts_with('-') {
            let p = std::path::Path::new(arg);
            if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                let lext = ext.to_lowercase();
                if (lext == "md" || lext == "markdown" || lext == "mdown" || lext == "mdx" || lext == "txt") && p.exists() {
                    return Some(arg.clone());
                }
            }
        }
    }
    None
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if let Some(edit_path) = extract_edit_path(&args) {
                wins::open_doc_editor(app, Some(edit_path), None);
                return;
            }
            let ls_paths = localsend::extract_cli_paths(&args);
            if !ls_paths.is_empty() {
                let mut p = payload::Payload::text(String::new(), payload::Origin::External, None, 0);
                p.kind = payload::PayloadKind::File;
                p.meta = serde_json::json!({ "files": ls_paths });
                wins::open_localsend(app, p);
            } else {
                wins::open_launcher(app);
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
        .plugin(hotkeys::plugin())
        // 截图帧通过自定义协议传给前端，避免大图 base64 走 IPC
        .register_uri_scheme_protocol("mbcap", |ctx, req| {
            let st = ctx.app_handle().state::<AppState>();
            // 旧请求不能读到下一次截图，避免异步解码串帧。
            let requested = req.uri().query().and_then(|q| q.split('&').find_map(|p| p.strip_prefix("t=")));
            let session = st.capture_session.lock().unwrap();
            let valid = requested.is_some_and(|id| session.is_active(id));
            let data = if valid { st.capture_png.lock().unwrap().clone() } else { None };
            let status = if data.is_some() { 200 } else { 410 };
            let data = data.unwrap_or_default();
            tauri::http::Response::builder()
                .status(status)
                .header("content-type", "image/png")
                .header("access-control-allow-origin", "*")
                .header("cache-control", "no-store")
                .body(data)
                .unwrap()
        })
        .setup(|app| {
            let handle = app.handle().clone();
            let cfg_dir = app.path().app_config_dir()?;
            let data_dir = app.path().app_data_dir()?;
            let store = settings::SettingsStore::load(cfg_dir);
            let history = history::History::open(data_dir).ok();
            app.manage(AppState {
                settings: store,
                llm: Arc::new(llm::LlmManager::default()),
                kb_queue: Arc::new(kb_queue::KbQueueManager::default()),
                clip: clip::ClipWatcher::default(),
                pending: Mutex::new(HashMap::new()),
                capture_png: Mutex::new(None),
                capture_session: Mutex::new(capture_session::CaptureLifecycle::default()),
                history,
                gateway_error: Mutex::new(None),
                hotkey_map: Mutex::new(HashMap::new()),
                hotkey_errors: Mutex::new(vec![]),
                pending_doc: Mutex::new(None),
            });

            tray::setup(&handle)?;
            winutil::refresh_shell_icon_cache();
            // 预创建失败不致命：窗口会在首次使用时按需创建
            if let Err(e) = wins::precreate(&handle) {
                eprintln!("[setup] 预创建窗口失败：{e}");
            }
            hotkeys::register(&handle);
            llm::LlmManager::spawn_watchdog(handle.clone());
            handle.state::<AppState>().kb_queue.spawn_worker(handle.clone());
            clip::ClipWatcher::spawn(handle.clone());
            gateway::spawn(handle.clone());
            localsend::spawn_daemon();

            // 自动注册系统文件/文件夹右键菜单「发送局域网文件 (LocalSend)」与「使用 miniBot 编辑 Markdown」
            let _ = localsend::set_context_menu_registered(true);
            let _ = kb::set_md_context_menu(true);
            // 确保提示词模板目录及默认 .md 提示词文件就位
            let _ = kb::ensure_prompts_dir(&handle);

            // 若通过命令行或右键菜单传入 --edit 或直接打开 Markdown 文件
            let cli_args: Vec<String> = std::env::args().collect();
            let edit_target = extract_edit_path(&cli_args);
            let ls_paths = localsend::extract_cli_paths(&cli_args);
            let is_autostart = cli_args.iter().any(|a| a == "--autostart" || a == "--silent");
            if let Some(target) = edit_target {
                wins::open_doc_editor(&handle, Some(target), None);
            } else if !ls_paths.is_empty() {
                let mut p = payload::Payload::text(String::new(), payload::Origin::External, None, 0);
                p.kind = payload::PayloadKind::File;
                p.meta = serde_json::json!({ "files": ls_paths });
                wins::open_localsend(&handle, p);
            } else if !is_autostart {
                // 手动双击启动（非开机自启静默模式）时，自动打开 AI 聊天主窗口
                wins::open_launcher(&handle);
            }

            // 划词/截图临时历史清理（默认保留一个月 30 天，不影响永久 AI 聊天记录）：启动时一次，之后每 24 小时
            let h2 = handle.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    {
                        let st = h2.state::<AppState>();
                        if let Some(h) = &st.history {
                            let days = st.settings.get().history_days.clamp(1, 30);
                            let _ = h.cleanup(days);
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(86400)).await;
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::take_payload,
            commands::hide_self,
            commands::minimize_self,
            commands::open_result,
            commands::open_page,
            commands::close_page,
            commands::open_qr,
            commands::open_localsend,
            commands::trigger,
            commands::replace_text,
            commands::get_settings,
            commands::save_settings,
            commands::hotkeys_suspend,
            commands::app_info,
            commands::llm_chat,
            commands::llm_cancel,
            commands::llm_status,
            commands::llm_check_port,
            commands::llm_test_api,
            commands::llm_fetch_models,
            commands::llm_release,
            commands::llm_start,
            commands::engine_overview,
            commands::engine_activate_model,
            commands::engine_update_model_params,
            commands::engine_scan_custom_folder,
            commands::engine_save_custom_model,
            commands::engine_delete_custom_model,
            commands::engine_pick_models_dir,
            commands::engine_download_model,
            commands::engine_download_runtime,
            commands::engine_cancel_download,
            commands::gpu_snapshot,
            commands::capture_ready,
            commands::capture_failed,
            commands::capture_close,
            commands::send_to_chat,
            commands::web_search,
            commands::copy_text,
            commands::copy_image,
            commands::save_image,
            commands::ocr_image,
            commands::qr_decode,
            commands::qr_encode,
            commands::pin_image,
            commands::image_search,
            commands::fetch_remote_image,
            commands::history_add,
            commands::history_list,
            commands::history_delete,
            commands::history_clear,
            commands::read_image_file,
            commands::chat_session_list,
            commands::chat_session_create,
            commands::chat_session_rename,
            commands::chat_session_delete,
            commands::chat_message_list,
            commands::chat_message_add,
            commands::chat_message_delete_from,
            commands::list_user_agents,
            commands::save_user_agent,
            commands::delete_user_agent,
            commands::open_agents_dir,
            commands::tts_voices,
            commands::tts_speak,
            commands::tts_broadcast,
            commands::localsend_info,
            commands::localsend_scan,
            commands::localsend_probe,
            commands::localsend_pick_files,
            commands::localsend_pick_folder,
            commands::localsend_resolve_paths,
            commands::localsend_send,
            commands::localsend_cancel,
            commands::localsend_set_context_menu,
            commands::app_check_update,
            commands::app_download_update,
            commands::app_cancel_update_download,
            commands::app_install_update,
            commands::open_external_url,
            commands::recorder_start,
            commands::recorder_pause,
            commands::recorder_resume,
            commands::recorder_stop,
            commands::recorder_cancel,
            commands::recorder_status,
            commands::recorder_reveal,
            commands::recorder_copy_file,
            commands::kb_save_clip,
            commands::kb_save_image,
            commands::kb_save_web,
            commands::kb_get_stats,
            commands::kb_open_folder,
            commands::kb_search,
            commands::kb_pick_folder,
            commands::kb_sync_index,
            commands::kb_laya_status,
            commands::kb_download_laya,
            commands::kb_delete_laya,
            commands::kb_open_file,
            commands::kb_reveal_file,
            commands::kb_read_image_base64,
            commands::kb_get_prompt,
            commands::kb_list_prompts,
            commands::kb_reset_prompt,
            commands::kb_create_prompt,
            commands::kb_open_prompts_folder,
            commands::kb_enqueue_save,
            commands::kb_queue_status,
            commands::doc_read_file,
            commands::doc_write_file,
            commands::doc_open_editor,
            commands::doc_get_pending,
            commands::doc_set_context_menu,
            commands::doc_is_context_menu_registered,
        ])
        .build(tauri::generate_context!())
        .expect("miniBot 启动失败");

    app.run(|app, event| match event {
        // 托盘常驻：关闭所有窗口不退出
        RunEvent::ExitRequested { api, code: None, .. } => {
            api.prevent_exit();
        }
        // macOS：点击 Dock 栏应用图标时，自动唤起并聚焦 AI 聊天主窗口（若设置/历史页正打开则优先聚焦）
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => {
            for page in ["settings", "history"] {
                if let Some(w) = app.get_webview_window(page) {
                    if w.is_visible().unwrap_or(false) {
                        let _ = w.unminimize();
                        let _ = w.show();
                        let _ = w.set_focus();
                        return;
                    }
                }
            }
            wins::open_launcher(app);
        }
        _ => {
            let _ = app;
        }
    });
}
