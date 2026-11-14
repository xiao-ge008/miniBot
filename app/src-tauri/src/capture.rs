//! 截图：捕获当前显示器，再由已解码的前端帧驱动遮罩显示。
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

/// mode: capture（截图）、color（取色）、chat（聊天附图）、record（录屏选区）。
pub fn start(app: &AppHandle, mode: &str) {
    let id = crate::payload::new_id();
    // 在派生线程前占用会话，覆盖抓图及前端解码期间的重复热键。
    if !app
        .state::<crate::AppState>()
        .capture_session
        .lock()
        .unwrap()
        .begin(&id)
    {
        return;
    }
    let app = app.clone();
    let mode = mode.to_owned();
    std::thread::spawn(move || {
        let hwnd = crate::winutil::foreground_hwnd();
        let src = crate::winutil::foreground_process();
        let (cx, cy) = crate::winutil::cursor_pos();
        // 优先使用光标所在显示器；当光标处于多屏边缘极值像素导致 from_point 失败时，回退到距离最近或主显示器。
        let mon = match xcap::Monitor::from_point(cx, cy).or_else(|e| {
            let mons = xcap::Monitor::all()?;
            mons.into_iter()
                .min_by_key(|m| {
                    let mx = m.x().unwrap_or(0);
                    let my = m.y().unwrap_or(0);
                    let mw = m.width().unwrap_or(1) as i32;
                    let mh = m.height().unwrap_or(1) as i32;
                    let dx = if cx < mx {
                        mx - cx
                    } else if cx >= mx + mw {
                        cx - (mx + mw - 1)
                    } else {
                        0
                    };
                    let dy = if cy < my {
                        my - cy
                    } else if cy >= my + mh {
                        cy - (my + mh - 1)
                    } else {
                        0
                    };
                    (dx as i64) * (dx as i64) + (dy as i64) * (dy as i64)
                })
                .ok_or(e)
        }) {
            Ok(mon) => mon,
            Err(e) => {
                fail(
                    &app,
                    &id,
                    &format!("无法确定光标所在显示器 ({cx}, {cy}): {e}"),
                );
                return;
            }
        };
        let img = match mon.capture_image() {
            Ok(img) if img.width() > 0 && img.height() > 0 => img,
            Ok(_) => {
                #[cfg(target_os = "macos")]
                if !crate::winutil::check_screen_capture_permission() {
                    let _ = crate::winutil::request_screen_capture_permission();
                }
                fail(&app, &id, "屏幕捕获返回空图像");
                return;
            }
            Err(e) => {
                #[cfg(target_os = "macos")]
                if !crate::winutil::check_screen_capture_permission() {
                    let _ = crate::winutil::request_screen_capture_permission();
                }
                fail(&app, &id, &format!("屏幕捕获失败: {e}"));
                return;
            }
        };
        let frame = Frame {
            x: mon.x().unwrap_or(0),
            y: mon.y().unwrap_or(0),
            width: img.width(),
            height: img.height(),
            scale: mon.scale_factor().unwrap_or(1.0).max(1.0),
        };
        let png = crate::imgutil::encode_png(&img);
        let windows = window_rects(&frame);
        // Windows 为物理像素；macOS 的全局光标、显示器原点均为逻辑点。
        let cursor_scale = if cfg!(target_os = "macos") {
            frame.scale
        } else {
            1.0
        };
        let mut p = Payload::text(String::new(), Origin::Capture, src, hwnd);
        p.kind = PayloadKind::Image;
        p.meta = serde_json::json!({
            "mode": mode, "frame": frame, "windows": windows, "t": id,
            "session_id": id,
            "cursor": { "x": (cx - frame.x) as f32 * cursor_scale, "y": (cy - frame.y) as f32 * cursor_scale }
        });

        // Windows WebView2 建窗不能发生在主线程回调中；正常情况下复用预创建窗口。
        #[cfg(not(target_os = "macos"))]
        let window = match crate::wins::get_or_build(&app, "capture") {
            Some(w) => w,
            None => {
                fail(&app, &id, "创建截图窗口失败");
                return;
            }
        };
        let main_app = app.clone();
        let main_id = id.clone();
        if let Err(e) = app.run_on_main_thread(move || {
            if !main_app
                .state::<crate::AppState>()
                .capture_session
                .lock()
                .unwrap()
                .is_active(&main_id)
            {
                return;
            }
            #[cfg(target_os = "macos")]
            let window = match crate::wins::get_or_build(&main_app, "capture") {
                Some(w) => w,
                None => {
                    close_now(&main_app, &main_id, Some("创建截图窗口失败"));
                    return;
                }
            };
            let geometry = (|| -> tauri::Result<()> {
                #[cfg(target_os = "macos")]
                {
                    let s = frame.scale as f64;
                    window.set_position(tauri::LogicalPosition::new(
                        frame.x as f64,
                        frame.y as f64,
                    ))?;
                    window.set_size(tauri::LogicalSize::new(
                        frame.width as f64 / s,
                        frame.height as f64 / s,
                    ))?;
                }
                #[cfg(not(target_os = "macos"))]
                {
                    window.set_position(tauri::PhysicalPosition::new(frame.x, frame.y))?;
                    // 高度增加 1 物理像素，防止 Windows 10/11 DWM 将恰好等于屏幕尺寸的 WS_POPUP 窗口
                    // 提升为独占全屏（DirectFlip / Independent Flip / MPO），避免显卡驱动冻结硬件鼠标光标。
                    window.set_size(tauri::PhysicalSize::new(
                        frame.width,
                        frame.height.saturating_add(1),
                    ))?;
                }
                Ok(())
            })();
            if let Err(e) = geometry {
                close_now(
                    &main_app,
                    &main_id,
                    Some(&format!("设置截图窗口位置失败: {e}")),
                );
                return;
            }
            let st = main_app.state::<crate::AppState>();
            let mut session = st.capture_session.lock().unwrap();
            if !session.prepare(&main_id) {
                return;
            }
            *st.capture_png.lock().unwrap() = Some(png);
            drop(session);
            crate::wins::deliver(&main_app, "capture", p);
            crate::log_msg(
                &main_app,
                format!(
                    "[capture] session={main_id} frame={}x{} origin=({}, {}) scale={} mode={mode}",
                    frame.width, frame.height, frame.x, frame.y, frame.scale
                ),
            );
        }) {
            fail(&app, &id, &format!("调度截图窗口失败: {e}"));
            return;
        }

        // 超时只取消仍未就绪的当前会话，不强行显示空白/旧帧，更不重复抢焦点。
        let timeout_app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(15));
            let check_app = timeout_app.clone();
            let _ = timeout_app.run_on_main_thread(move || {
                if check_app
                    .state::<crate::AppState>()
                    .capture_session
                    .lock()
                    .unwrap()
                    .is_preparing(&id)
                {
                    close_now(&check_app, &id, Some("截图帧加载超时，已取消，请重试"));
                }
            });
        });
        // 纯截图阶段不加载模型，避免与 WebView2/系统合成器争用 GPU 和内存。
        // 实际识图请求仍会通过 LlmManager::ensure_ready 按需启动。
    });
}

fn window_rects(f: &Frame) -> Vec<[i32; 4]> {
    #[cfg(windows)]
    {
        crate::winutil::fast_window_rects(f.x, f.y, f.width, f.height)
    }
    #[cfg(not(windows))]
    {
        let _ = f;
        vec![]
    }
}

pub fn ready(app: &AppHandle, id: &str) {
    let main_app = app.clone();
    let id = id.to_owned();
    let _ = app.run_on_main_thread(move || {
        let st = main_app.state::<crate::AppState>();
        if !st.capture_session.lock().unwrap().show(&id) {
            return;
        }
        let Some(w) = main_app.get_webview_window("capture") else {
            close_now(&main_app, &id, Some("截图窗口已丢失"));
            return;
        };
        let result = (|| -> tauri::Result<()> {
            w.set_always_on_top(true)?;
            w.show()?;
            w.set_focus()?;
            #[cfg(windows)]
            crate::winutil::prepare_capture_window(crate::wins::hwnd_of(&w));
            Ok(())
        })();
        if let Err(e) = result {
            close_now(&main_app, &id, Some(&format!("显示截图窗口失败: {e}")));
        } else {
            crate::log_msg(&main_app, format!("[capture] session={id} visible"));
        }
    });
}

pub fn fail(app: &AppHandle, id: &str, message: &str) {
    finish(app, id, Some(message.to_owned()));
}

pub fn close_session(app: &AppHandle, id: &str) {
    finish(app, id, None);
}

/// Rust 内部动作关闭调用时的会话，异步执行时也不能误关下一轮截图。
pub fn close(app: &AppHandle) {
    let id = app
        .state::<crate::AppState>()
        .capture_session
        .lock()
        .unwrap()
        .current_id();
    if let Some(id) = id {
        close_session(app, &id);
    }
}

fn finish(app: &AppHandle, id: &str, error: Option<String>) {
    let main_app = app.clone();
    let id = id.to_owned();
    let _ = app.run_on_main_thread(move || close_now(&main_app, &id, error.as_deref()));
}

/// 仅在主线程清理窗口、帧和负载，避免旧清理任务覆盖新会话。
fn close_now(app: &AppHandle, id: &str, error: Option<&str>) {
    let st = app.state::<crate::AppState>();
    let mut session = st.capture_session.lock().unwrap();
    if !session.close(id) {
        return;
    }
    *st.capture_png.lock().unwrap() = None;
    st.pending.lock().unwrap().remove("capture");
    // 保持会话锁直到窗口隐藏，防止新热键在清理中途进入。
    if let Some(w) = app.get_webview_window("capture") {
        let _ = w.hide();
    }
    let _ = app.emit_to(
        "capture",
        "minibot://capture-reset",
        serde_json::json!({ "session_id": id }),
    );
    drop(session);
    if let Some(error) = error {
        crate::log_msg(app, format!("[capture] session={id} error={error}"));
    }
}
