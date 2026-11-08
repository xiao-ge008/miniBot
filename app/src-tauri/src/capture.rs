//! 截图：抓取光标所在显示器 → 显示预创建的全屏遮罩窗口
use crate::payload::{Origin, Payload, PayloadKind};
use serde::Serialize;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize)]
pub struct Frame {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}

/// mode: "capture"（截图）| "color"（只取色）
pub fn start(app: &AppHandle, mode: &str) {
    let app = app.clone();
    let mode = mode.to_string();
    std::thread::spawn(move || {
        let st = app.state::<crate::AppState>();
        if app.get_webview_window("capture").map(|w| w.is_visible().unwrap_or(false)).unwrap_or(false) {
            return;
        }
        let hwnd = crate::winutil::foreground_hwnd();
        let src = crate::winutil::foreground_process();
        let (cx, cy) = crate::winutil::cursor_pos();
        let mon = xcap::Monitor::from_point(cx, cy)
            .ok()
            .or_else(|| xcap::Monitor::all().ok().and_then(|v| v.into_iter().next()));
        let Some(mon) = mon else {
            crate::log_msg(&app, format!("[capture] 获取显示器失败 ({cx}, {cy})"));
            return;
        };
        let Ok(img) = mon.capture_image() else {
            crate::log_msg(&app, "[capture] capture_image 失败");
            return;
        };
        let frame = Frame {
            x: mon.x().unwrap_or(0),
            y: mon.y().unwrap_or(0),
            width: img.width(),
            height: img.height(),
            scale: mon.scale_factor().unwrap_or(1.0),
        };
        let png = crate::imgutil::encode_png(&img);
        *st.capture_png.lock().unwrap() = Some(png);
        let windows = window_rects(&frame);

        let mut p = Payload::text(String::new(), Origin::Capture, src, hwnd);
        p.kind = PayloadKind::Image;
        p.meta = serde_json::json!({ "mode": mode, "frame": frame, "windows": windows, "t": crate::payload::new_id() });

        let Some(w) = crate::wins::get_or_build(&app, "capture") else {
            crate::log_msg(&app, "[capture] get_or_build capture 失败");
            return;
        };
        #[cfg(target_os = "macos")]
        {
            let s = frame.scale.max(1.0) as f64;
            let _ = w.set_position(tauri::LogicalPosition::new(frame.x as f64, frame.y as f64));
            let _ = w.set_size(tauri::LogicalSize::new(frame.width as f64 / s, frame.height as f64 / s));
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = w.set_position(tauri::PhysicalPosition::new(frame.x, frame.y));
            let _ = w.set_size(tauri::PhysicalSize::new(frame.width, frame.height));
        }
        crate::wins::deliver(&app, "capture", p);
        crate::log_msg(&app, format!("[capture] 已投递截图帧 {}x{} mode={mode}", frame.width, frame.height));

        // 兜底显示：前端解码完会立即调 capture_ready；若隐藏窗口在极个别环境下延迟响应，100ms 后强制显示避免无反应
        let app_show = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if let Some(w) = app_show.get_webview_window("capture") {
                if !w.is_visible().unwrap_or(false) && app_show.state::<crate::AppState>().capture_png.lock().unwrap().is_some() {
                    ready(&app_show);
                }
            }
        });

        // 截图时顺手预热模型（显卡忙则静默放弃）
        if mode == "capture" || mode == "chat" {
            let app3 = app.clone();
            tauri::async_runtime::spawn(async move {
                let st = app3.state::<crate::AppState>();
                st.llm.prewarm(&app3).await;
            });
        }
    });
}

/// 可见窗口矩形（相对当前显示器的物理像素，按 Z 序从上到下），用于「单击选窗口」
fn window_rects(f: &Frame) -> Vec<[i32; 4]> {
    let Ok(list) = xcap::Window::all() else { return vec![] };
    let mut out = vec![];
    #[cfg(target_os = "macos")]
    let scale = f.scale.max(1.0);
    #[cfg(not(target_os = "macos"))]
    let scale = 1.0f32;

    for w in list {
        if w.is_minimized().unwrap_or(true) {
            continue;
        }
        let title = w.title().unwrap_or_default();
        if title.is_empty() || title.starts_with("miniBot") {
            continue;
        }
        let (Ok(x), Ok(y), Ok(ww), Ok(hh)) = (w.x(), w.y(), w.width(), w.height()) else { continue };
        if ww < 40 || hh < 40 {
            continue;
        }
        let rx0 = (((x - f.x) as f32) * scale).round() as i32;
        let ry0 = (((y - f.y) as f32) * scale).round() as i32;
        let rw = ((ww as f32) * scale).round() as i32;
        let rh = ((hh as f32) * scale).round() as i32;
        let x0 = rx0.max(0);
        let y0 = ry0.max(0);
        let x1 = (rx0 + rw).min(f.width as i32);
        let y1 = (ry0 + rh).min(f.height as i32);
        if x1 - x0 < 40 || y1 - y0 < 40 {
            continue;
        }
        out.push([x0, y0, x1 - x0, y1 - y0]);
    }
    out
}

pub fn ready(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("capture") {
        let _ = w.show();
        let _ = w.set_always_on_top(true);
        let _ = w.set_focus();
        crate::winutil::focus_hwnd(crate::wins::hwnd_of(&w));
    }
}

pub fn close(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("capture") {
        let _ = w.hide();
    }
    *app.state::<crate::AppState>().capture_png.lock().unwrap() = None;
}
