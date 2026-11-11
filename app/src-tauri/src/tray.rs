//! 系统托盘：状态显示 + 常用操作
use crate::llm::Status;
use std::sync::OnceLock;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

static STATUS_ITEM: OnceLock<MenuItem<Wry>> = OnceLock::new();
static TIP_ITEM: OnceLock<CheckMenuItem<Wry>> = OnceLock::new();

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let hk = app.state::<crate::AppState>().settings.get().hotkeys;
    let status = MenuItem::with_id(app, "status", "⚪ 模型：未加载", false, None::<&str>)?;
    let capture = MenuItem::with_id(app, "capture", format!("📷 截图\t{}", hk.capture), true, None::<&str>)?;
    let record = MenuItem::with_id(app, "record", format!("🔴 录屏\t{}", hk.record), true, None::<&str>)?;
    let launcher = MenuItem::with_id(app, "launcher", format!("💬 AI 聊天\t{}", hk.launcher), true, None::<&str>)?;
    let color = MenuItem::with_id(app, "color", format!("🎨 取色\t{}", hk.color), true, None::<&str>)?;
    let qr = MenuItem::with_id(app, "qr", "▣ 生成二维码", true, None::<&str>)?;
    let localsend = MenuItem::with_id(app, "localsend", "📡 发送局域网文件 (LocalSend)", true, None::<&str>)?;
    let history = MenuItem::with_id(app, "history", "🕘 划词/识图记录", true, None::<&str>)?;
    let tip = CheckMenuItem::with_id(app, "tip_pause", "⏸ 暂停复制 Tip 1 小时", true, false, None::<&str>)?;
    let release = MenuItem::with_id(app, "release", "🔻 立即释放模型", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "⚙ 设置", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "❌ 退出", true, None::<&str>)?;
    let sep = || PredefinedMenuItem::separator(app);
    let menu = Menu::with_items(
        app,
        &[&status, &sep()?, &capture, &record, &launcher, &color, &qr, &localsend, &history, &sep()?, &tip, &release, &settings, &quit],
    )?;
    let _ = STATUS_ITEM.set(status);
    let _ = TIP_ITEM.set(tip);

    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("miniBot")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, e| {
            let app = app.clone();
            match e.id().as_ref() {
                "capture" => crate::actions::trigger(&app, "capture"),
                "record" => crate::actions::trigger(&app, "record"),
                "launcher" => crate::actions::trigger(&app, "launcher"),
                "color" => crate::actions::trigger(&app, "color"),
                "qr" => crate::wins::open_qr(&app, String::new()),
                "localsend" => {
                    let p = crate::payload::Payload::text(String::new(), crate::payload::Origin::Launcher, None, 0);
                    crate::wins::open_localsend(&app, p);
                }
                "history" => crate::wins::open_page(&app, "history", "历史记录"),
                "settings" => crate::wins::open_page(&app, "settings", "设置"),
                "tip_pause" => {
                    let st = app.state::<crate::AppState>();
                    let paused = TIP_ITEM.get().and_then(|i| i.is_checked().ok()).unwrap_or(false);
                    st.clip.pause_for(if paused { 3600 } else { 0 });
                }
                "release" => {
                    tauri::async_runtime::spawn(async move {
                        let st = app.state::<crate::AppState>();
                        st.llm.release(&app, Status::Released, "手动释放").await;
                    });
                }
                "quit" => app.exit(0),
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                crate::actions::trigger(tray.app_handle(), "launcher");
            }
        })
        .build(app)?;
    Ok(())
}

/// 刷新托盘状态文字
pub fn refresh(app: &AppHandle) {
    let Some(item) = STATUS_ITEM.get() else { return };
    let st = app.state::<crate::AppState>();
    let info = st.llm.status_info(&st.settings.get().llm);
    let kb_q = st.kb_queue.status();
    let base_text = if info.api_enabled {
        let m = if info.api_model.is_empty() { "OpenAI API" } else { &info.api_model };
        format!("☁️ 模型：云端 API（{m}）")
    } else {
        match info.status {
            Status::Released => "⚪ 模型：已释放".to_string(),
            Status::Loading => "🔵 模型：加载中…".to_string(),
            Status::Ready => format!("🟢 模型：已加载（{}:{:02} 后释放）", info.remaining_secs / 60, info.remaining_secs % 60),
            Status::Yielded => "🟡 模型：已让道（显卡忙）".to_string(),
        }
    };
    let text = if kb_q.total_in_flight > 0 {
        format!("{base_text} | 📥 入库队列 {}/{}", kb_q.total_in_flight, kb_q.max_size)
    } else {
        base_text
    };
    let _ = item.set_text(text);
    if let Some(t) = app.tray_by_id("main") {
        let tip_label = if info.api_enabled {
            "云端 API 就绪"
        } else {
            match info.status {
                Status::Released => "空闲",
                Status::Loading => "加载中",
                Status::Ready => "模型就绪",
                Status::Yielded => "已让道",
            }
        };
        let tip = if kb_q.total_in_flight > 0 {
            let cur = kb_q
                .active
                .as_ref()
                .map(|a| a.stage_label.as_str())
                .unwrap_or("排队处理中…");
            format!(
                "miniBot · {tip_label}\n📥 知识库后台处理 {}/{}: {cur}",
                kb_q.total_in_flight, kb_q.max_size
            )
        } else {
            format!("miniBot · {tip_label}")
        };
        let _ = t.set_tooltip(Some(tip));
    }
}
