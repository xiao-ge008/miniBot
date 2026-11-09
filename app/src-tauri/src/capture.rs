//! 截图：抓取光标所在显示器 → 显示预创建的全屏遮罩窗口
use crate::payload::{Origin, Payload, PayloadKind};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

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
        #[cfg(target_os = "macos")]
        if st.capture_png.lock().unwrap().is_some() {
            return;
        }
        #[cfg(not(target_os = "macos"))]
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

        #[cfg(target_os = "macos")]
        {
            let app_main = app.clone();
            let frame_c = frame.clone();
            let mode_c = mode.clone();
            let _ = app.run_on_main_thread(move || {
                let Some(w) = crate::wins::get_or_build(&app_main, "capture") else {
                    crate::log_msg(&app_main, "[capture] get_or_build capture 失败");
                    return;
                };
                let s = frame_c.scale.max(1.0) as f64;
                let _ = w.set_position(tauri::LogicalPosition::new(frame_c.x as f64, frame_c.y as f64));
                let _ = w.set_size(tauri::LogicalSize::new(frame_c.width as f64 / s, frame_c.height as f64 / s));
                crate::wins::deliver(&app_main, "capture", p);
                crate::log_msg(&app_main, format!("[capture] 已投递截图帧 {}x{} mode={mode_c}", frame_c.width, frame_c.height));
            });
        }
        #[cfg(not(target_os = "macos"))]
        {
            let Some(w) = crate::wins::get_or_build(&app, "capture") else {
                crate::log_msg(&app, "[capture] get_or_build capture 失败");
                return;
            };
            let _ = w.set_position(tauri::PhysicalPosition::new(frame.x, frame.y));
            let _ = w.set_size(tauri::PhysicalSize::new(frame.width, frame.height));
            crate::wins::deliver(&app, "capture", p);
            crate::log_msg(&app, format!("[capture] 已投递截图帧 {}x{} mode={mode}", frame.width, frame.height));
        }

        // 容灾兜底：前端解码绘制完会立即主动调用 capture_ready 显示窗口；此处 2500ms 超时仅作为防死锁极端兜底，杜绝过早显示旧帧
        let app_show = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(2500));
            if app_show.state::<crate::AppState>().capture_png.lock().unwrap().is_some() {
                ready(&app_show);
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
    // macOS 下跳过全量窗口枚举（避免高频调用 CGWindowListCopyWindowInfo 阻塞 WindowServer）
    #[cfg(target_os = "macos")]
    {
        let _ = f;
        vec![]
    }
    // Windows 下使用原生 Win32 枚举，耗时 1~3ms（避免 xcap::Window::all() 跨进程遍历导致数百毫秒卡顿）
    #[cfg(windows)]
    {
        crate::winutil::fast_window_rects(f.x, f.y, f.width, f.height)
    }
    #[cfg(all(not(target_os = "macos"), not(windows)))]
    {
        let _ = f;
        vec![]
    }
}

pub fn ready(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    {
        let app2 = app.clone();
        let _ = app.run_on_main_thread(move || {
            if let Some(w) = app2.get_webview_window("capture") {
                let _ = w.show();
                let _ = w.set_always_on_top(true);
                let _ = w.set_focus();
            }
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        let app2 = app.clone();
        let _ = app.run_on_main_thread(move || {
            if let Some(w) = app2.get_webview_window("capture") {
                let _ = w.show();
                let _ = w.set_always_on_top(true);
                let _ = w.set_focus();
                crate::winutil::focus_hwnd(crate::wins::hwnd_of(&w));
            }
        });
    }
}

pub fn close(app: &AppHandle) {
    *app.state::<crate::AppState>().capture_png.lock().unwrap() = None;
    let _ = app.emit_to("capture", "minibot://capture-reset", ());
    #[cfg(target_os = "macos")]
    {
        let app2 = app.clone();
        let _ = app.run_on_main_thread(move || {
            if let Some(w) = app2.get_webview_window("capture") {
                let _ = w.hide();
            }
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        let app2 = app.clone();
        let _ = app.run_on_main_thread(move || {
            if let Some(w) = app2.get_webview_window("capture") {
                let _ = w.hide();
            }
        });
    }
}
