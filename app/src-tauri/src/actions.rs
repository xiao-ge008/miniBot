//! 热键/托盘触发的顶层动作
use crate::payload::{Origin, Payload};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

pub fn trigger(app: &AppHandle, name: &str) {
    match name {
        "capture" => crate::capture::start(app, "capture"),
        "record" => {
            let st = crate::recorder::get_status();
            if st.status == "recording" || st.status == "paused" {
                let _ = crate::recorder::stop_recording(app);
            } else {
                crate::capture::start(app, "record");
            }
        }
        "capture_chat" => {
            if let Some(w) = app.get_webview_window("launcher") {
                let _ = w.hide();
            }
            let app2 = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(150));
                crate::capture::start(&app2, "chat");
            });
        }
        "color" => crate::capture::start(app, "color"),
        "launcher" => crate::wins::open_launcher(app),
        "selection" => selection(app),
        _ => {}
    }
}

/// 读取当前选中文字：模拟 Ctrl+C，读取后恢复原剪贴板
pub fn grab_selection(app: &AppHandle) -> Option<String> {
    grab_selection_inner(app, false)
}

/// 鼠标划词松开后轻量读取选中文字
pub fn grab_selection_clean(app: &AppHandle) -> Option<String> {
    if crate::winutil::any_modifier_down() {
        return None;
    }
    grab_selection_inner(app, true)
}

fn grab_selection_inner(app: &AppHandle, clean: bool) -> Option<String> {
    let st = app.state::<crate::AppState>();
    let old = crate::clip::read_text_retry();
    let seq0 = crate::winutil::clipboard_seq();
    st.clip.suppress(2000); // 避免触发复制 Tip
    if clean {
        crate::winutil::send_ctrl_c_clean();
    } else {
        crate::winutil::send_ctrl('c');
    }
    let timeout = if clean { 240 } else { 420 };
    let start = Instant::now();
    while crate::winutil::clipboard_seq() == seq0 && start.elapsed() < Duration::from_millis(timeout) {
        std::thread::sleep(Duration::from_millis(15));
    }
    if crate::winutil::clipboard_seq() == seq0 {
        st.clip.suppress(0);
        return None;
    }
    std::thread::sleep(Duration::from_millis(30));
    let text = crate::clip::read_text_retry();
    if let Some(o) = old {
        let _ = crate::clip::set_text_retry(o);
    }
    st.clip.mark_self_write();
    st.clip.suppress(350);
    text.filter(|t| !t.trim().is_empty())
}

fn selection(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let hwnd = crate::winutil::foreground_hwnd();
        let src = crate::winutil::foreground_process();
        // 1. 优先抓取当前鼠标选区文字；2. 若未触发新剪贴板序号（如用户已提前 Ctrl+C 复制过），自动回退读取当前剪贴板文字
        let text = grab_selection(&app).or_else(|| {
            crate::clip::read_text_retry()
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
        });
        if let Some(t) = text {
            crate::log_msg(&app, format!("[selection] Alt+W 获取文本 ({} chars) -> show_tip", t.chars().count()));
            let mut p = Payload::text(t, Origin::Selection, src, hwnd);
            p.meta = serde_json::json!({ "sticky": true });
            crate::wins::show_tip(&app, p);
        } else if let Some(img) = crate::clip::read_image_retry() {
            if let Some(rgba) = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned()) {
                let png = crate::imgutil::encode_png(&rgba);
                let mut p = Payload::image(png, Origin::Selection, src, hwnd);
                let codes = crate::qr::decode_rgba(&rgba);
                p.meta = serde_json::json!({ "sticky": true, "qr": codes });
                crate::wins::show_tip(&app, p);
                return;
            }
            crate::wins::open_launcher(&app);
        } else {
            crate::wins::open_launcher(&app);
        }
    });
}

/// 替换原文：写剪贴板 → 聚焦来源窗口 → Ctrl+V → 恢复剪贴板
pub fn replace_text(app: &AppHandle, hwnd: isize, text: String) {
    let app = app.clone();
    std::thread::spawn(move || {
        let st = app.state::<crate::AppState>();
        let old = crate::clip::read_text_retry();
        st.clip.suppress(3000);
        if !crate::clip::set_text_retry(text) {
            st.clip.suppress(0);
            return;
        }
        crate::wins::hide(&app, "result");
        crate::wins::hide(&app, "tip");
        crate::winutil::focus_hwnd(hwnd);
        std::thread::sleep(Duration::from_millis(120));
        crate::winutil::send_ctrl('v');
        std::thread::sleep(Duration::from_millis(400));
        if let Some(o) = old {
            let _ = crate::clip::set_text_retry(o);
        }
        st.clip.mark_self_write();
        st.clip.suppress(0);
    });
}
