//! 前端可调用的命令
use crate::llm::{ChatEvent, LlmError, Status, StatusInfo};
use crate::payload::Payload;
use crate::settings::Settings;
use crate::AppState;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State, Window};

type R<T> = Result<T, String>;

fn e<E: std::fmt::Display>(err: E) -> String {
    err.to_string()
}

// ---------- Payload / 窗口 ----------

#[tauri::command]
pub fn take_payload(window: Window, st: State<AppState>) -> Option<Payload> {
    st.pending.lock().unwrap().remove(window.label())
}

#[tauri::command]
pub fn hide_self(app: AppHandle, window: tauri::WebviewWindow) {
    let label = window.label().to_string();
    let _ = window.hide();
    crate::winutil::hide_hwnd(crate::wins::hwnd_of(&window));
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = app2.get_webview_window(&label) {
            let _ = w.hide();
            crate::winutil::hide_hwnd(crate::wins::hwnd_of(&w));
        }
    });
}

#[tauri::command]
pub fn minimize_self(app: AppHandle, window: tauri::WebviewWindow) {
    let label = window.label().to_string();
    let _ = window.minimize();
    crate::winutil::minimize_hwnd(crate::wins::hwnd_of(&window));
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = app2.get_webview_window(&label) {
            let _ = w.minimize();
            crate::winutil::minimize_hwnd(crate::wins::hwnd_of(&w));
        }
    });
}

#[tauri::command]
pub fn open_result(app: AppHandle, payload: Payload) {
    crate::wins::open_result(&app, payload);
}

#[tauri::command]
pub fn open_page(app: AppHandle, route: String, title: String) {
    crate::wins::open_page(&app, &route, &title);
}

#[tauri::command]
pub fn close_page(app: AppHandle, window: Window) {
    crate::wins::close_page(&app, window.label());
}

#[tauri::command]
pub fn open_qr(app: AppHandle, text: String) {
    crate::wins::open_qr(&app, text);
}

#[tauri::command]
pub fn open_localsend(app: AppHandle, payload: Option<Payload>) {
    let p = payload.unwrap_or_else(|| {
        Payload::text(String::new(), crate::payload::Origin::Launcher, None, 0)
    });
    crate::wins::open_localsend(&app, p);
}

#[tauri::command]
pub fn trigger(app: AppHandle, name: String) {
    crate::actions::trigger(&app, &name);
}

#[tauri::command]
pub fn replace_text(app: AppHandle, hwnd: isize, text: String) {
    crate::actions::replace_text(&app, hwnd, text);
}

// ---------- 设置 ----------

#[tauri::command]
pub fn get_settings(st: State<AppState>) -> Settings {
    st.settings.get()
}

#[tauri::command]
pub fn save_settings(app: AppHandle, st: State<AppState>, mut settings: Settings) -> R<Vec<String>> {
    use tauri::Emitter;
    settings.hotkeys.launcher = crate::hotkeys::normalize_shortcut(&settings.hotkeys.launcher);
    settings.hotkeys.capture = crate::hotkeys::normalize_shortcut(&settings.hotkeys.capture);
    settings.hotkeys.record = crate::hotkeys::normalize_shortcut(&settings.hotkeys.record);
    settings.hotkeys.selection = crate::hotkeys::normalize_shortcut(&settings.hotkeys.selection);
    settings.hotkeys.color = crate::hotkeys::normalize_shortcut(&settings.hotkeys.color);
    let old = st.settings.get();
    st.settings.set(settings.clone()).map_err(e)?;
    {
        use tauri_plugin_autostart::ManagerExt;
        let al = app.autolaunch();
        let _ = if settings.autostart { al.enable() } else { al.disable() };
    }
    if old.api.port != settings.api.port || old.api.lan != settings.api.lan || old.api.enabled != settings.api.enabled {
        st.gateway_error.lock().unwrap().replace("网关配置已修改，重启 miniBot 后生效".into());
    }
    if let Some(w) = app.get_webview_window("result") {
        let rw = settings.ui.result_width.clamp(360.0, 1400.0);
        let rh = settings.ui.result_height.clamp(240.0, 1200.0);
        let _ = w.set_size(tauri::LogicalSize::new(rw, rh));
    }
    // 开启云端 API 模型时，若本地推理引擎正在运行则自动释放显存
    if settings.llm.api_enabled && !old.llm.api_enabled {
        let app2 = app.clone();
        tauri::async_runtime::spawn(async move {
            let st2 = app2.state::<AppState>();
            st2.llm.release(&app2, Status::Released, "已切换至云端 API 模型").await;
        });
    } else {
        st.llm.emit_status(&app, "");
    }
    let _ = app.emit("minibot://settings", &settings);
    Ok(crate::hotkeys::register(&app))
}

#[tauri::command]
pub fn hotkeys_suspend(app: AppHandle, suspend: bool) {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    if suspend {
        let _ = app.global_shortcut().unregister_all();
    } else {
        let _ = crate::hotkeys::register(&app);
    }
}

#[derive(serde::Serialize)]
pub struct AppInfo {
    pub version: String,
    pub api_url: String,
    pub gateway_error: Option<String>,
    pub hotkey_errors: Vec<String>,
    pub config_dir: String,
    pub agents_dir: String,
}

#[tauri::command]
pub fn app_info(app: AppHandle, st: State<AppState>) -> AppInfo {
    let s = st.settings.get();
    AppInfo {
        version: app.package_info().version.to_string(),
        api_url: format!("http://127.0.0.1:{}/v1", s.api.port),
        gateway_error: st.gateway_error.lock().unwrap().clone(),
        hotkey_errors: st.hotkey_errors.lock().unwrap().clone(),
        config_dir: app.path().app_config_dir().unwrap_or_default().to_string_lossy().into(),
        agents_dir: agents_dir(&app).to_string_lossy().into(),
    }
}

// ---------- 模型 ----------

#[tauri::command]
pub async fn llm_chat(
    app: AppHandle,
    st: State<'_, AppState>,
    id: String,
    messages: serde_json::Value,
    params: Option<serde_json::Value>,
    force: Option<bool>,
    on_event: Channel<ChatEvent>,
) -> Result<String, LlmError> {
    let llm = st.llm.clone();
    llm.chat(&app, id, messages, params.unwrap_or_default(), force.unwrap_or(false), on_event).await
}

#[tauri::command]
pub fn llm_cancel(st: State<AppState>, id: String) {
    st.llm.cancel(&id);
}

#[tauri::command]
pub fn llm_status(st: State<AppState>) -> StatusInfo {
    st.llm.status_info(&st.settings.get().llm)
}

#[tauri::command]
pub async fn llm_test_api(base_url: String, api_key: String, model: String) -> R<crate::llm::CloudTestResult> {
    crate::llm::test_cloud_api(&base_url, &api_key, &model).await
}

#[tauri::command]
pub async fn llm_fetch_models(base_url: String, api_key: String) -> R<Vec<String>> {
    crate::llm::fetch_cloud_models(&base_url, &api_key).await
}

#[derive(serde::Serialize)]
pub struct PortCheckResult {
    pub available: bool,
    pub port: u16,
    pub suggested_port: u16,
}

#[tauri::command]
pub async fn llm_check_port(port: u16) -> R<PortCheckResult> {
    let available = std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
        || crate::llm::probe_health(port).await;
    let suggested = if available {
        port
    } else {
        crate::llm::find_available_port(port.saturating_add(1), 30).unwrap_or(port)
    };
    Ok(PortCheckResult {
        available,
        port,
        suggested_port: suggested,
    })
}

#[tauri::command]
pub async fn llm_release(app: AppHandle, st: State<'_, AppState>) -> R<()> {
    st.llm.release(&app, Status::Released, "手动释放").await;
    Ok(())
}

#[tauri::command]
pub async fn llm_start(app: AppHandle, st: State<'_, AppState>) -> R<()> {
    st.llm
        .ensure_ready(&app, true)
        .await
        .map_err(|err| match err {
            LlmError::Other { message } => message,
            LlmError::PortBusy { port } => format!("端口 {port} 被占用"),
            LlmError::GpuBusy { .. } => "显卡繁忙".into(),
            LlmError::Cancelled => "已取消".into(),
        })
}

#[tauri::command]
pub async fn engine_overview(app: AppHandle) -> R<crate::engine::EngineOverview> {
    Ok(crate::engine::get_overview(&app).await)
}

#[tauri::command]
pub async fn engine_activate_model(app: AppHandle, model_id: String) -> R<crate::engine::EngineOverview> {
    crate::engine::activate_model(&app, &model_id).await.map_err(e)
}

#[tauri::command]
pub async fn engine_update_model_params(
    app: AppHandle,
    model_id: String,
    params: crate::settings::ModelParams,
) -> R<crate::engine::EngineOverview> {
    crate::engine::update_model_params(&app, model_id, params).await.map_err(e)
}

#[tauri::command]
pub async fn engine_scan_custom_folder(
    st: State<'_, AppState>,
) -> R<Option<crate::engine::ScannedFolderResult>> {
    let s = st.settings.get();
    let cur_dir = s
        .llm
        .custom_models
        .last()
        .map(|m| m.folder_path.clone())
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| {
            crate::settings::default_engine_dir()
                .join("models")
                .to_string_lossy()
                .into_owned()
        });
    let res = tokio::task::spawn_blocking(move || crate::engine::scan_custom_folder_dialog(&cur_dir))
        .await
        .map_err(e)?;
    Ok(res)
}

#[tauri::command]
pub async fn engine_save_custom_model(
    app: AppHandle,
    model: crate::settings::CustomLocalModel,
) -> R<crate::engine::EngineOverview> {
    crate::engine::save_custom_model(&app, model).await.map_err(e)
}

#[tauri::command]
pub async fn engine_delete_custom_model(
    app: AppHandle,
    model_id: String,
) -> R<crate::engine::EngineOverview> {
    crate::engine::delete_custom_model(&app, model_id).await.map_err(e)
}

#[tauri::command]
pub async fn engine_pick_models_dir(
    app: AppHandle,
    st: State<'_, AppState>,
) -> R<Option<(crate::engine::EngineOverview, usize)>> {
    let s = st.settings.get();
    let cur_dir = if s.llm.models_dir.trim().is_empty() {
        crate::settings::default_engine_dir()
            .join("models")
            .to_string_lossy()
            .into_owned()
    } else {
        s.llm.models_dir
    };
    let picked = tokio::task::spawn_blocking(move || crate::engine::pick_folder_dialog(&cur_dir))
        .await
        .map_err(e)?;
    let Some(new_dir) = picked else {
        return Ok(None);
    };
    let res = crate::engine::change_models_dir(&app, new_dir).await.map_err(e)?;
    Ok(Some(res))
}

#[tauri::command]
pub async fn engine_download_model(
    app: AppHandle,
    model_id: String,
    mirror: String,
    on_progress: Channel<crate::engine::DownloadProgress>,
) -> R<crate::engine::EngineOverview> {
    crate::engine::download_model(&app, model_id, mirror, on_progress).await.map_err(e)
}

#[tauri::command]
pub async fn engine_download_runtime(
    app: AppHandle,
    backend: String,
    on_progress: Channel<crate::engine::DownloadProgress>,
) -> R<crate::engine::EngineOverview> {
    crate::engine::download_engine(&app, backend, on_progress).await.map_err(e)
}

#[tauri::command]
pub fn engine_cancel_download() {
    crate::engine::cancel_download();
}

#[tauri::command]
pub fn gpu_snapshot() -> crate::gpu::GpuSnapshot {
    crate::gpu::snapshot()
}

// ---------- 截图 / 图片 ----------

#[tauri::command]
pub fn capture_ready(app: AppHandle, session_id: String) {
    crate::capture::ready(&app, &session_id);
}

#[tauri::command]
pub fn capture_failed(app: AppHandle, session_id: String) {
    crate::capture::fail(&app, &session_id, "前端无法加载截图帧，请重试");
}

#[tauri::command]
pub fn capture_close(app: AppHandle, session_id: String) {
    crate::capture::close_session(&app, &session_id);
}

#[tauri::command]
pub fn send_to_chat(app: AppHandle, image: String, auto_send: bool) {
    use tauri::Emitter;
    crate::capture::close(&app);
    crate::wins::open_launcher(&app);
    let _ = app.emit(
        "minibot://chat-capture",
        serde_json::json!({ "image": image, "auto_send": auto_send }),
    );
}

#[tauri::command]
pub async fn web_search(query: String, max_results: Option<usize>) -> crate::websearch::SearchResponse {
    crate::websearch::search(&query, max_results.unwrap_or(5)).await
}

fn rgba_from(data: &str) -> R<image::RgbaImage> {
    let bytes = crate::imgutil::decode_b64(data).map_err(e)?;
    crate::imgutil::decode_rgba(&bytes).map_err(e)
}

#[tauri::command]
pub fn copy_text(st: State<AppState>, text: String) -> R<()> {
    st.clip.suppress(800);
    if !crate::clip::set_text_retry(text) {
        return Err("写入剪贴板失败".into());
    }
    st.clip.mark_self_write();
    Ok(())
}

#[tauri::command]
pub async fn copy_image(st: State<'_, AppState>, data: String) -> R<()> {
    let img = rgba_from(&data)?;
    let w = img.width() as usize;
    let h = img.height() as usize;
    let raw = img.into_raw();
    st.clip.suppress(1500);
    st.clip.record_self_image(w, h, raw.len());
    let mut cb = arboard::Clipboard::new().map_err(e)?;
    let mut last_err = None;
    for i in 0..6 {
        match cb.set_image(arboard::ImageData {
            width: w,
            height: h,
            bytes: std::borrow::Cow::Borrowed(&raw),
        }) {
            Ok(()) => {
                st.clip.mark_self_write();
                return Ok(());
            }
            Err(err) => {
                last_err = Some(err);
                if i < 5 {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
            }
        }
    }
    Err(e(last_err.unwrap()))
}

#[tauri::command]
pub async fn save_image(app: AppHandle, st: State<'_, AppState>, data: String) -> R<String> {
    let cur_dir = st.settings.get().save_dir;
    let _ = std::fs::create_dir_all(&cur_dir);
    let cap_win = app.get_webview_window("capture");
    let cap_was_visible = cap_win
        .as_ref()
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);
    if cap_was_visible {
        if let Some(w) = &cap_win {
            let _ = w.set_always_on_top(false);
        }
    }
    let res_win = app.get_webview_window("result");
    let res_was_aot = res_win
        .as_ref()
        .and_then(|w| w.is_always_on_top().ok())
        .unwrap_or(false);
    if res_was_aot {
        if let Some(w) = &res_win {
            let _ = w.set_always_on_top(false);
        }
    }
    let picked = tokio::task::spawn_blocking(move || {
        crate::engine::pick_folder_dialog_with_title(&cur_dir, "选择图片保存文件夹")
    })
    .await
    .map_err(e)?;

    if res_was_aot {
        if let Some(w) = &res_win {
            let _ = w.set_always_on_top(true);
            let _ = w.set_focus();
        }
    }

    let Some(dir_str) = picked else {
        if cap_was_visible {
            if let Some(w) = &cap_win {
                let _ = w.set_always_on_top(true);
                let _ = w.set_focus();
            }
        }
        return Ok(String::new());
    };

    let dir = std::path::PathBuf::from(&dir_str);
    std::fs::create_dir_all(&dir).map_err(e)?;
    let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let mut path = dir.join(format!("miniBot_{ts}.png"));
    if path.exists() {
        let ms = chrono::Local::now().timestamp_subsec_millis();
        path = dir.join(format!("miniBot_{ts}_{ms:03}.png"));
    }
    let bytes = crate::imgutil::decode_b64(&data).map_err(e)?;
    std::fs::write(&path, bytes).map_err(e)?;

    let mut s = st.settings.get();
    if s.save_dir != dir_str {
        s.save_dir = dir_str;
        let _ = st.settings.set(s);
    }
    Ok(path.to_string_lossy().into())
}

#[tauri::command]
pub async fn ocr_image(data: String) -> R<crate::ocr::OcrResult> {
    let img = rgba_from(&data)?;
    tokio::task::spawn_blocking(move || crate::ocr::recognize(&img)).await.map_err(e)?.map_err(e)
}

#[tauri::command]
pub async fn qr_decode(data: String) -> R<Vec<crate::qr::QrResult>> {
    let img = rgba_from(&data)?;
    tokio::task::spawn_blocking(move || crate::qr::decode_rgba(&img)).await.map_err(e)
}

#[tauri::command]
pub fn qr_encode(text: String, ec: String, dark: String, light: String) -> R<String> {
    crate::qr::encode_svg(&text, &ec, &dark, &light).map_err(e)
}

#[tauri::command]
pub fn pin_image(app: AppHandle, data: String, x: i32, y: i32, w: f64, h: f64) {
    crate::wins::open_pin(&app, data, x, y, w, h);
}

#[tauri::command]
pub async fn image_search(data: String, engine: Option<String>) -> R<crate::imgsearch::ImageSearchResponse> {
    crate::imgsearch::search_by_image(&data, engine.as_deref().unwrap_or("baidu")).await
}

#[tauri::command]
pub async fn fetch_remote_image(url: String) -> R<String> {
    crate::imgsearch::fetch_remote_as_data_uri(&url).await
}

// ---------- 历史 ----------

#[tauri::command]
pub fn history_add(st: State<AppState>, entry: crate::history::Entry, image: Option<String>) -> R<i64> {
    let keep = st.settings.get().history_keep_images;
    st.history.as_ref().ok_or("历史库不可用")?.add(entry, image, keep).map_err(e)
}

#[tauri::command]
pub fn history_list(st: State<AppState>, query: String, limit: u32) -> R<Vec<crate::history::Entry>> {
    st.history.as_ref().ok_or("历史库不可用")?.list(&query, limit).map_err(e)
}

#[tauri::command]
pub fn history_delete(st: State<AppState>, id: i64) -> R<()> {
    st.history.as_ref().ok_or("历史库不可用")?.delete(id).map_err(e)
}

#[tauri::command]
pub fn history_clear(st: State<AppState>) -> R<()> {
    st.history.as_ref().ok_or("历史库不可用")?.clear().map_err(e)
}

#[tauri::command]
pub fn read_image_file(path: String) -> R<String> {
    let bytes = std::fs::read(&path).map_err(e)?;
    let img = crate::imgutil::decode_rgba(&bytes).map_err(e)?;
    Ok(crate::imgutil::to_data_uri(&crate::imgutil::encode_png(&img)))
}

// ---------- 永久多轮 AI 聊天会话 ----------

#[tauri::command]
pub fn chat_session_list(st: State<AppState>) -> R<Vec<crate::history::ChatSession>> {
    st.history.as_ref().ok_or("历史库不可用")?.chat_session_list().map_err(e)
}

#[tauri::command]
pub fn chat_session_create(st: State<AppState>, title: String) -> R<crate::history::ChatSession> {
    st.history.as_ref().ok_or("历史库不可用")?.chat_session_create(title).map_err(e)
}

#[tauri::command]
pub fn chat_session_rename(st: State<AppState>, id: String, title: String) -> R<()> {
    st.history.as_ref().ok_or("历史库不可用")?.chat_session_rename(&id, &title).map_err(e)
}

#[tauri::command]
pub fn chat_session_delete(st: State<AppState>, id: String) -> R<()> {
    st.history.as_ref().ok_or("历史库不可用")?.chat_session_delete(&id).map_err(e)
}

#[tauri::command]
pub fn chat_message_list(st: State<AppState>, session_id: String) -> R<Vec<crate::history::ChatMessageRecord>> {
    st.history.as_ref().ok_or("历史库不可用")?.chat_message_list(&session_id).map_err(e)
}

#[tauri::command]
pub fn chat_message_add(
    st: State<AppState>,
    session_id: String,
    role: String,
    content: String,
    attachments: Vec<crate::history::ChatAttachment>,
    stats: Option<crate::llm::ChatStats>,
    kb_hits: Option<Vec<crate::kb::KbSearchHit>>,
) -> R<i64> {
    st.history
        .as_ref()
        .ok_or("历史库不可用")?
        .chat_message_add(
            &session_id,
            &role,
            &content,
            &attachments,
            stats.as_ref(),
            kb_hits.as_deref(),
        )
        .map_err(e)
}

#[tauri::command]
pub fn chat_message_delete_from(st: State<AppState>, session_id: String, from_id: i64) -> R<()> {
    st.history
        .as_ref()
        .ok_or("历史库不可用")?
        .chat_message_delete_from(&session_id, from_id)
        .map_err(e)
}

// ---------- 智能体 ----------

pub fn agents_dir(app: &AppHandle) -> std::path::PathBuf {
    app.path().app_config_dir().unwrap_or_default().join("agents")
}

/// 读取用户自定义智能体（agents/*.json）
#[tauri::command]
pub fn list_user_agents(app: AppHandle) -> Vec<serde_json::Value> {
    let dir = agents_dir(&app);
    let _ = std::fs::create_dir_all(&dir);
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for f in rd.flatten() {
            if f.path().extension().map(|x| x == "json").unwrap_or(false) {
                if let Ok(v) = std::fs::read_to_string(f.path()).map(|s| serde_json::from_str::<serde_json::Value>(&s)) {
                    if let Ok(v) = v {
                        out.push(v);
                    }
                }
            }
        }
    }
    out
}

#[tauri::command]
pub fn save_user_agent(app: AppHandle, agent: serde_json::Value) -> R<()> {
    let id = agent.get("id").and_then(|v| v.as_str()).ok_or("缺少 id")?;
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("id 只能包含字母、数字、- 和 _".into());
    }
    let dir = agents_dir(&app);
    std::fs::create_dir_all(&dir).map_err(e)?;
    std::fs::write(dir.join(format!("{id}.json")), serde_json::to_string_pretty(&agent).map_err(e)?).map_err(e)
}

#[tauri::command]
pub fn open_agents_dir(app: AppHandle) -> R<()> {
    use tauri_plugin_opener::OpenerExt;
    let dir = agents_dir(&app);
    std::fs::create_dir_all(&dir).map_err(e)?;
    app.opener().open_path(dir.to_string_lossy(), None::<&str>).map_err(e)
}

#[tauri::command]
pub fn delete_user_agent(app: AppHandle, id: String) -> R<()> {
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("非法 id".into());
    }
    std::fs::remove_file(agents_dir(&app).join(format!("{id}.json"))).map_err(e)
}

// ---------- Edge TTS 语音朗读 ----------

#[tauri::command]
pub fn tts_voices() -> Vec<crate::tts::VoiceOption> {
    crate::tts::builtin_voices()
}

#[tauri::command]
pub async fn tts_speak(
    st: State<'_, AppState>,
    text: String,
    voice: Option<String>,
    rate: Option<String>,
) -> R<String> {
    let cfg = st.settings.get().tts;
    let resolved_voice = match voice.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(v) => v.to_string(),
        None => crate::tts::resolve_voice(&text, &cfg.voice, cfg.auto_lang),
    };
    let resolved_rate = rate
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&cfg.rate)
        .to_string();
    crate::tts::synthesize(&text, &resolved_voice, &resolved_rate, &cfg.volume)
        .await
        .map_err(e)
}

#[tauri::command]
pub fn tts_broadcast(app: AppHandle, payload: serde_json::Value) {
    use tauri::Emitter;
    let _ = app.emit("minibot://tts", payload);
}

// ---------- LocalSend 局域网文件/文本/截图快传 ----------

#[derive(serde::Serialize)]
pub struct LocalSendSelfInfo {
    pub alias: String,
    pub lan_ips: Vec<String>,
    pub context_menu_enabled: bool,
    pub os_model: String,
}

#[tauri::command]
pub fn localsend_info() -> LocalSendSelfInfo {
    LocalSendSelfInfo {
        alias: crate::localsend::self_alias(),
        lan_ips: crate::localsend::local_lan_ips()
            .into_iter()
            .map(|ip| ip.to_string())
            .collect(),
        context_menu_enabled: crate::localsend::is_context_menu_registered(),
        os_model: crate::localsend::os_model_name().to_string(),
    }
}

#[tauri::command]
pub async fn localsend_scan(extra_ips: Option<Vec<String>>) -> Vec<crate::localsend::LocalSendPeer> {
    crate::localsend::scan_peers(extra_ips.unwrap_or_default()).await
}

#[tauri::command]
pub async fn localsend_probe(ip: String, port: Option<u16>) -> R<crate::localsend::LocalSendPeer> {
    let clean = ip.trim();
    let (host, p) = match clean.split_once(':') {
        Some((h, pt)) => (
            h.trim().to_string(),
            pt.trim().parse::<u16>().unwrap_or(crate::localsend::LOCALSEND_PORT),
        ),
        None => (
            clean.to_string(),
            port.unwrap_or(crate::localsend::LOCALSEND_PORT),
        ),
    };
    crate::localsend::probe_peer(&host, p).await.map_err(e)
}

#[tauri::command]
pub async fn localsend_pick_files() -> R<Vec<crate::localsend::LocalSendItem>> {
    let paths = tokio::task::spawn_blocking(crate::localsend::pick_files_dialog)
        .await
        .map_err(e)?;
    Ok(crate::localsend::resolve_paths_to_items(paths))
}

#[tauri::command]
pub async fn localsend_pick_folder() -> R<Vec<crate::localsend::LocalSendItem>> {
    let picked = tokio::task::spawn_blocking(crate::localsend::pick_folder_for_localsend)
        .await
        .map_err(e)?;
    match picked {
        Some(dir) => Ok(crate::localsend::resolve_paths_to_items(vec![dir])),
        None => Ok(Vec::new()),
    }
}

#[tauri::command]
pub fn localsend_resolve_paths(paths: Vec<String>) -> Vec<crate::localsend::LocalSendItem> {
    crate::localsend::resolve_paths_to_items(paths)
}

#[tauri::command]
pub async fn localsend_send(
    peer: crate::localsend::LocalSendPeer,
    items: Vec<crate::localsend::LocalSendItem>,
    pin: Option<String>,
    on_progress: Channel<crate::localsend::LocalSendProgress>,
) -> R<()> {
    crate::localsend::send_items(peer, items, pin, on_progress)
        .await
        .map_err(e)
}

#[tauri::command]
pub fn localsend_cancel() {
    crate::localsend::cancel_send();
}

#[tauri::command]
pub fn localsend_set_context_menu(enable: bool) -> R<bool> {
    crate::localsend::set_context_menu_registered(enable).map_err(e)
}

// ---------- 版本检测与自动下载安装更新 ----------

#[tauri::command]
pub async fn app_check_update(app: AppHandle) -> R<crate::updater::UpdateInfo> {
    let cur = app.package_info().version.to_string();
    crate::updater::check_update(&cur).await
}

#[tauri::command]
pub async fn app_download_update(
    download_url: String,
    asset_name: String,
    use_mirror: bool,
    on_progress: Channel<crate::updater::UpdateDownloadProgress>,
) -> R<String> {
    crate::updater::download_update(download_url, asset_name, use_mirror, on_progress).await
}

#[tauri::command]
pub fn app_cancel_update_download() {
    crate::updater::cancel_update_download();
}

#[tauri::command]
pub async fn app_install_update(app: AppHandle, installer_path: String) -> R<()> {
    crate::updater::install_update(&app, installer_path).await
}

#[tauri::command]
pub fn open_external_url(app: AppHandle, url: String) -> R<()> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().open_url(url, None::<&str>).map_err(e)
}

// ---------- 极简双端录屏 ----------

#[tauri::command]
pub fn recorder_start(
    app: AppHandle,
    args: crate::recorder::RecordStartArgs,
) -> R<crate::recorder::RecorderStateInfo> {
    crate::recorder::start_recording(&app, args)
}

#[tauri::command]
pub fn recorder_pause(app: AppHandle) -> R<crate::recorder::RecorderStateInfo> {
    crate::recorder::pause_recording(&app)
}

#[tauri::command]
pub fn recorder_resume(app: AppHandle) -> R<crate::recorder::RecorderStateInfo> {
    crate::recorder::resume_recording(&app)
}

#[tauri::command]
pub fn recorder_stop(app: AppHandle) -> R<crate::recorder::RecorderStateInfo> {
    crate::recorder::stop_recording(&app)
}

#[tauri::command]
pub fn recorder_cancel(app: AppHandle) -> R<()> {
    crate::recorder::cancel_recording(&app)
}

#[tauri::command]
pub fn recorder_status() -> crate::recorder::RecorderStateInfo {
    crate::recorder::get_status()
}

#[tauri::command]
pub fn recorder_reveal(path: String) -> R<()> {
    crate::recorder::reveal_video_in_folder(&path)
}

#[tauri::command]
pub fn recorder_copy_file(app: AppHandle, path: String) -> R<()> {
    crate::recorder::copy_video_file_to_clipboard(&app, &path)
}

// ---------- 本地知识库 ----------

#[tauri::command]
pub async fn kb_save_clip(
    app: AppHandle,
    text: String,
    source_app: Option<String>,
    url: Option<String>,
    distilled_summary: Option<String>,
) -> R<String> {
    let p = crate::kb::save_clip(
        &app,
        &text,
        source_app.as_deref(),
        url.as_deref(),
        distilled_summary.as_deref(),
    )
    .await
    .map_err(e)?;
    Ok(p.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn kb_save_image(
    app: AppHandle,
    img_data: String,
    ocr_text: Option<String>,
    vision_desc: Option<String>,
    source: Option<String>,
) -> R<String> {
    let p = crate::kb::save_image(&app, &img_data, ocr_text.as_deref(), vision_desc.as_deref(), source.as_deref()).await.map_err(e)?;
    Ok(p.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn kb_save_web(
    app: AppHandle,
    url: String,
    title: Option<String>,
    content: String,
    distilled_summary: Option<String>,
) -> R<String> {
    let p = crate::kb::save_web(&app, &url, title.as_deref(), &content, distilled_summary.as_deref())
        .await
        .map_err(e)?;
    Ok(p.to_string_lossy().to_string())
}

#[tauri::command]
pub fn kb_get_stats(app: AppHandle) -> R<crate::kb::KbStats> {
    crate::kb::get_kb_stats(&app).map_err(e)
}

#[tauri::command]
pub fn kb_open_folder(app: AppHandle) -> R<()> {
    crate::kb::open_kb_folder(&app).map_err(e)
}

#[tauri::command]
pub fn kb_search(
    app: AppHandle,
    query: String,
    limit: Option<usize>,
) -> R<crate::kb::KbSearchResponse> {
    crate::kb::search_kb(&app, &query, limit.unwrap_or(20)).map_err(e)
}

#[tauri::command]
pub async fn kb_pick_folder(app: AppHandle) -> R<Option<crate::kb::KbSyncSummary>> {
    let cur_dir = crate::kb::get_kb_root(&app).to_string_lossy().to_string();
    let picked = tokio::task::spawn_blocking(move || {
        crate::engine::pick_folder_dialog_with_title(&cur_dir, "选择个人知识库根文件夹")
    })
    .await
    .map_err(e)?;

    if let Some(new_path) = picked {
        let st = app.state::<crate::AppState>();
        let mut s = st.settings.get();
        s.knowledge.base_path = new_path.clone();
        let _ = st.settings.set(s);
        let path_obj = std::path::PathBuf::from(&new_path);
        let _ = crate::kb::ensure_kb_dirs(&path_obj);

        // 自动触发全量扫描并构建新文件夹的文本索引
        let summary = tokio::task::spawn_blocking(move || {
            crate::kb::sync_kb_index(&path_obj)
        })
        .await
        .map_err(e)?
        .map_err(e)?;

        Ok(Some(summary))
    } else {
        Ok(None)
    }
}

#[tauri::command]
pub async fn kb_sync_index(app: AppHandle) -> R<crate::kb::KbSyncSummary> {
    let root = crate::kb::get_kb_root(&app);
    tokio::task::spawn_blocking(move || {
        crate::kb::sync_kb_index(&root)
    })
    .await
    .map_err(e)?
    .map_err(e)
}

#[tauri::command]
pub fn kb_laya_status(app: AppHandle) -> R<crate::kb::LayaStatus> {
    Ok(crate::kb::check_laya_status(&app))
}

#[tauri::command]
pub async fn kb_download_laya(
    app: AppHandle,
    on_progress: Channel<crate::engine::DownloadProgress>,
) -> R<crate::kb::LayaStatus> {
    crate::kb::download_laya_model(&app, on_progress).await.map_err(e)
}

#[tauri::command]
pub fn kb_delete_laya(app: AppHandle) -> R<()> {
    crate::kb::delete_laya_model(&app).map_err(e)
}

#[tauri::command]
pub fn kb_open_file(path: String) -> R<()> {
    crate::kb::open_kb_file(&path).map_err(e)
}

#[tauri::command]
pub fn kb_reveal_file(path: String) -> R<()> {
    crate::kb::reveal_kb_file(&path).map_err(e)
}

#[tauri::command]
pub fn kb_read_image_base64(path: String) -> R<String> {
    crate::kb::read_image_base64(&path).map_err(e)
}

#[tauri::command]
pub fn kb_get_prompt(app: AppHandle, category: String, id: String) -> R<crate::kb::PromptTemplate> {
    crate::kb::get_prompt_template(&app, &category, &id).map_err(e)
}

#[tauri::command]
pub fn kb_list_prompts(app: AppHandle) -> R<Vec<crate::kb::PromptTemplate>> {
    crate::kb::list_prompt_templates(&app).map_err(e)
}

#[tauri::command]
pub fn kb_reset_prompt(
    app: AppHandle,
    category: Option<String>,
    id: Option<String>,
) -> R<Vec<crate::kb::PromptTemplate>> {
    crate::kb::reset_prompt_template(&app, category.as_deref(), id.as_deref()).map_err(e)
}

#[tauri::command]
pub fn kb_create_prompt(
    app: AppHandle,
    category: String,
    id: String,
    name: Option<String>,
) -> R<crate::kb::PromptTemplate> {
    crate::kb::create_prompt_template(&app, &category, &id, name.as_deref()).map_err(e)
}

#[tauri::command]
pub fn kb_open_prompts_folder(app: AppHandle, target: Option<String>) -> R<String> {
    crate::kb::open_prompts_folder(&app, target.as_deref()).map_err(e)
}

#[tauri::command]
pub fn kb_enqueue_save(
    app: AppHandle,
    st: State<'_, AppState>,
    input: crate::kb_queue::KbQueueInput,
) -> crate::kb_queue::KbEnqueueResult {
    st.kb_queue.enqueue(&app, input)
}

#[tauri::command]
pub fn kb_queue_status(st: State<'_, AppState>) -> crate::kb_queue::KbQueueStatus {
    st.kb_queue.status()
}

// ---------- 文档编辑器 (Markdown 查看与修改器) ----------

#[tauri::command]
pub fn doc_read_file(app: AppHandle, path: String) -> R<crate::kb::DocFileInfo> {
    crate::kb::read_doc_file(&app, &path).map_err(e)
}

#[tauri::command]
pub fn doc_write_file(app: AppHandle, path: String, content: String) -> R<crate::kb::DocWriteResult> {
    crate::kb::write_doc_file(&app, &path, &content).map_err(e)
}

#[tauri::command]
pub fn doc_open_editor(app: AppHandle, path: Option<String>, query: Option<String>) {
    crate::wins::open_doc_editor(&app, path, query);
}

#[tauri::command]
pub fn doc_get_pending(app: AppHandle) -> Option<(String, Option<String>)> {
    let st = app.state::<crate::AppState>();
    st.pending_doc.lock().unwrap().take()
}

#[tauri::command]
pub fn doc_set_context_menu(enable: bool) -> R<bool> {
    crate::kb::set_md_context_menu(enable).map_err(e)
}

#[tauri::command]
pub fn doc_is_context_menu_registered() -> bool {
    crate::kb::is_md_context_menu_registered()
}



