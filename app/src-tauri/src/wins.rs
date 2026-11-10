//! 窗口管理：Tip / 结果浮窗 / 输入框 / 截图遮罩 / 钉图 / 二维码 / 设置 / 历史
use crate::payload::{Origin, Payload};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub const EVT_PAYLOAD: &str = "minibot://payload";
static RETURN_TO_CHAT: AtomicBool = AtomicBool::new(false);

pub fn hwnd_of(w: &WebviewWindow) -> isize {
    #[cfg(windows)]
    {
        w.hwnd().map(|h| h.0 as isize).unwrap_or(0)
    }
    #[cfg(not(windows))]
    {
        let _ = w;
        0
    }
}

/// 将指定窗口标记为「从屏幕捕获/录屏中排除」（Windows WDA_EXCLUDEFROMCAPTURE / macOS NSWindowSharingNone）
pub fn exclude_from_capture(w: &WebviewWindow, exclude: bool) {
    #[cfg(windows)]
    {
        crate::winutil::set_exclude_from_capture(hwnd_of(w), exclude);
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(ptr) = w.ns_window() {
            crate::winutil::set_exclude_from_capture(ptr as isize, exclude);
        }
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        let _ = (w, exclude);
    }
}

/// 跨平台判断屏幕坐标 (x, y) 是否在指定窗口矩形内（Windows 走物理像素 HWND，macOS 走逻辑点坐标）
pub fn point_in_win(w: &WebviewWindow, x: i32, y: i32) -> bool {
    #[cfg(windows)]
    {
        crate::winutil::point_in_hwnd(hwnd_of(w), x, y)
    }
    #[cfg(not(windows))]
    {
        if !w.is_visible().unwrap_or(false) {
            return false;
        }
        let Ok(pos) = w.outer_position() else {
            return false;
        };
        let Ok(size) = w.outer_size() else {
            return false;
        };
        let scale = w.scale_factor().unwrap_or(1.0).max(1.0);
        let lx = (pos.x as f64 / scale).round() as i32;
        let ly = (pos.y as f64 / scale).round() as i32;
        let lw = (size.width as f64 / scale).round() as i32;
        let lh = (size.height as f64 / scale).round() as i32;
        x >= lx && x <= lx + lw && y >= ly && y <= ly + lh
    }
}

/// 存放待取走的 Payload，并通知对应窗口
pub fn deliver(app: &AppHandle, label: &str, p: Payload) {
    let st = app.state::<crate::AppState>();
    st.pending.lock().unwrap().insert(label.to_string(), p);
    let _ = app.emit_to(label, EVT_PAYLOAD, ());
}

fn builder<'a>(app: &'a AppHandle, label: &str, route: &str) -> WebviewWindowBuilder<'a, tauri::Wry, AppHandle> {
    let b = WebviewWindowBuilder::new(app, label, WebviewUrl::App(route.into()));
    if let Some(ic) = app.default_window_icon() {
        if let Ok(b_with_icon) = b.icon(ic.clone()) {
            return b_with_icon;
        }
    }
    WebviewWindowBuilder::new(app, label, WebviewUrl::App(route.into()))
}

fn prevent_destroy_on_close(w: &WebviewWindow) {
    let w_clone = w.clone();
    w.on_window_event(move |evt| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = evt {
            api.prevent_close();
            let _ = w_clone.hide();
        }
    });
}

fn build_tip(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let w = builder(app, "tip", "tip")
        .title("miniBot Tip")
        .inner_size(480.0, 52.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .build()?;
    prevent_destroy_on_close(&w);
    Ok(w)
}

fn build_recorder(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let w = builder(app, "recorder", "recorder")
        .title("miniBot 录屏")
        .inner_size(480.0, 56.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .build()?;
    exclude_from_capture(&w, true);
    prevent_destroy_on_close(&w);
    Ok(w)
}

fn build_recorder_border(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let w = builder(app, "recorder_border", "recorder")
        .title("miniBot 录屏区域")
        .inner_size(400.0, 300.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .build()?;
    let _ = w.set_ignore_cursor_events(true);
    exclude_from_capture(&w, true);
    #[cfg(windows)]
    {
        let h = hwnd_of(&w);
        crate::winutil::make_click_through(h);
        crate::winutil::set_exclude_from_capture(h, true);
    }
    prevent_destroy_on_close(&w);
    Ok(w)
}

fn build_result(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let ui = app.state::<crate::AppState>().settings.get().ui;
    let rw = ui.result_width.clamp(360.0, 1400.0);
    let rh = ui.result_height.clamp(240.0, 1200.0);
    let w = builder(app, "result", "result")
        .title("miniBot")
        .inner_size(rw, rh)
        .min_inner_size(340.0, 220.0)
        .decorations(false)
        .shadow(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()?;
    prevent_destroy_on_close(&w);
    Ok(w)
}

fn build_capture(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let w = builder(app, "capture", "capture")
        .title("miniBot 截图")
        .decorations(false)
        .shadow(false)
        .resizable(cfg!(target_os = "macos"))
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()?;
    prevent_destroy_on_close(&w);
    Ok(w)
}

fn build_launcher(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let w = builder(app, "launcher", "launcher")
        .title("miniBot · AI 聊天")
        .inner_size(1060.0, 740.0)
        .min_inner_size(760.0, 520.0)
        .decorations(false)
        .shadow(true)
        .resizable(true)
        .maximizable(false)
        .center()
        .visible(false)
        .build()?;
    prevent_destroy_on_close(&w);
    Ok(w)
}

/// 取常驻浮窗；若尚未创建（或预创建失败）则立即创建
/// 注意：WebviewWindowBuilder::build() 严禁在主线程回调（同步命令 / run_on_main_thread）内调用，否则 WebView2 会死锁
pub fn get_or_build(app: &AppHandle, label: &str) -> Option<WebviewWindow> {
    if let Some(w) = app.get_webview_window(label) {
        return Some(w);
    }
    let r = match label {
        "tip" => build_tip(app),
        "recorder" => build_recorder(app),
        "recorder_border" => build_recorder_border(app),
        "result" => build_result(app),
        "capture" => build_capture(app),
        "launcher" => build_launcher(app),
        _ => return None,
    };
    r.map_err(|e| crate::log_msg(app, format!("[wins] 创建 {label} 失败：{e}"))).ok()
}

/// 启动时预创建常用浮窗（隐藏），按热键或触发动作时只需显示，速度最快且避免运行时建窗死锁
pub fn precreate(app: &AppHandle) -> tauri::Result<()> {
    for (name, f) in [
        ("launcher", build_launcher as fn(&AppHandle) -> tauri::Result<WebviewWindow>),
        ("capture", build_capture),
        ("result", build_result),
        ("tip", build_tip),
    ] {
        match f(app) {
            Ok(_) => crate::log_msg(app, format!("[setup] 预创建 {name} 成功")),
            Err(e) => crate::log_msg(app, format!("[setup] 预创建 {name} 失败：{e}")),
        }
    }
    Ok(())
}

/// 把窗口放到光标附近并限制在当前显示器内（兼容 Windows 物理像素与 macOS Retina 逻辑点坐标）
fn place_near_cursor(w: &WebviewWindow, dx: i32, dy: i32) {
    let (cx, cy) = crate::winutil::cursor_pos();
    let size = w.outer_size().unwrap_or_default();
    let mon = w
        .monitor_from_point(cx as f64, cy as f64)
        .ok()
        .flatten()
        .or_else(|| w.current_monitor().ok().flatten())
        .or_else(|| w.primary_monitor().ok().flatten());

    #[cfg(target_os = "macos")]
    {
        let scale = mon
            .as_ref()
            .map(|m| m.scale_factor())
            .or_else(|| w.scale_factor().ok())
            .unwrap_or(1.0)
            .max(1.0);
        let sw = ((size.width as f64 / scale).round() as i32).max(260);
        let sh = ((size.height as f64 / scale).round() as i32).max(48);
        let mut x = cx + dx;
        let mut y = cy + dy;
        if let Some(m) = mon {
            let mp = m.position();
            let ms = m.size();
            let mp_x = (mp.x as f64 / scale).round() as i32;
            let mp_y = (mp.y as f64 / scale).round() as i32;
            let ms_w = (ms.width as f64 / scale).round() as i32;
            let ms_h = (ms.height as f64 / scale).round() as i32;
            let right = mp_x + ms_w;
            let bottom = mp_y + ms_h;
            if x + sw > right - 8 {
                x = right - sw - 8;
            }
            if x < mp_x + 8 {
                x = mp_x + 8;
            }
            if y < mp_y + 8 {
                y = cy + 22;
            }
            if y + sh > bottom - 8 {
                y = cy - sh - 14;
            }
            x = x.max(mp_x + 8);
            y = y.max(mp_y + 8);
        }
        let _ = w.set_position(tauri::LogicalPosition::new(x as f64, y as f64));
    }

    #[cfg(not(target_os = "macos"))]
    {
        let sw = (size.width as i32).max(260);
        let sh = (size.height as i32).max(48);
        let mut x = cx + dx;
        let mut y = cy + dy;
        if let Some(m) = mon {
            let mp = m.position();
            let ms = m.size();
            let right = mp.x + ms.width as i32;
            let bottom = mp.y + ms.height as i32;
            if x + sw > right - 8 {
                x = right - sw - 8;
            }
            if x < mp.x + 8 {
                x = mp.x + 8;
            }
            if y < mp.y + 8 {
                // 上方空间不足时自动放到光标下方
                y = cy + 22;
            }
            if y + sh > bottom - 8 {
                y = cy - sh - 14;
            }
            x = x.max(mp.x + 8);
            y = y.max(mp.y + 8);
        }
        let _ = w.set_position(PhysicalPosition::new(x, y));
    }
}

/// 跨平台窗口操作线程调度：
/// - Windows：必须在独立后台线程执行（WebviewWindowBuilder::build 在主线程回调内会引发 WebView2 死锁）
/// - macOS：AppKit / NSWindow / WKWebView 强制要求所有窗口创建、显示、隐藏、坐标与尺寸修改必须在主线程（Main Thread）执行，否则会触发 NSInternalInconsistencyException 崩溃或 WindowServer 死锁
fn run_win_task<F>(app: &AppHandle, f: F)
where
    F: FnOnce(AppHandle) + Send + 'static,
{
    let app2 = app.clone();
    #[cfg(target_os = "macos")]
    {
        let _ = app.run_on_main_thread(move || {
            f(app2);
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::thread::spawn(move || {
            f(app2);
        });
    }
}

pub fn show_tip(app: &AppHandle, p: Payload) {
    run_win_task(app, move |app2| {
        let Some(w) = get_or_build(&app2, "tip") else { return };
        // 参考千问桌面端划词栏：默认悬浮在选中文本正上方偏左位置
        place_near_cursor(&w, -110, -68);
        deliver(&app2, "tip", p);
        let _ = w.show();
        #[cfg(windows)]
        {
            let h = hwnd_of(&w);
            crate::winutil::show_noactivate(h);
            let _ = app2.run_on_main_thread(move || {
                crate::winutil::show_noactivate(h);
            });
            std::thread::sleep(std::time::Duration::from_millis(25));
            crate::winutil::show_noactivate(h);
        }
    });
}

/// 打开极简录屏控制胶囊条与鼠标穿透防穿帮选区高亮框
pub fn open_recorder_hud(app: &AppHandle, args: &crate::recorder::RecordStartArgs) {
    let args = args.clone();
    run_win_task(app, move |app2| {
        // 1. 定位并展示录制区域边框指示窗（完全鼠标穿透 + 视频画面隐身）
        if let Some(wb) = get_or_build(&app2, "recorder_border") {
            let _ = wb.set_ignore_cursor_events(true);
            exclude_from_capture(&wb, true);

            #[cfg(target_os = "macos")]
            {
                let s = args.scale.max(1.0);
                let mon_w = (args.monitor_w as f32 / s).round() as i32;
                let mon_h = (args.monitor_h as f32 / s).round() as i32;
                let rx = args.monitor_x + (args.crop_x as f32 / s).round() as i32;
                let ry = args.monitor_y + (args.crop_y as f32 / s).round() as i32;
                let rw = (args.crop_w as f32 / s).round() as i32;
                let rh = (args.crop_h as f32 / s).round() as i32;
                let is_full = rw >= mon_w - 4 && rh >= mon_h - 4;
                let pad = if is_full { 0 } else { 3 };
                let bx = (rx - pad).max(args.monitor_x);
                let by = (ry - pad).max(args.monitor_y);
                let bw = (rw + pad * 2).min(args.monitor_x + mon_w - bx).max(24);
                let bh = (rh + pad * 2).min(args.monitor_y + mon_h - by).max(24);
                let _ = wb.set_position(tauri::LogicalPosition::new(bx as f64, by as f64));
                let _ = wb.set_size(tauri::LogicalSize::new(bw as f64, bh as f64));
                let _ = wb.show();
                let _ = wb.set_ignore_cursor_events(true);
                exclude_from_capture(&wb, true);
            }

            #[cfg(not(target_os = "macos"))]
            {
                let mon_w = args.monitor_w as i32;
                let mon_h = args.monitor_h as i32;
                let rx = args.monitor_x + args.crop_x as i32;
                let ry = args.monitor_y + args.crop_y as i32;
                let rw = args.crop_w as i32;
                let rh = args.crop_h as i32;
                let is_full = rw >= mon_w - 4 && rh >= mon_h - 4;
                let pad = if is_full { 0 } else { 3 };
                let bx = (rx - pad).max(args.monitor_x);
                let by = (ry - pad).max(args.monitor_y);
                let bw = (rw + pad * 2).min(args.monitor_x + mon_w - bx).max(24);
                let bh = (rh + pad * 2).min(args.monitor_y + mon_h - by).max(24);
                let _ = wb.set_position(PhysicalPosition::new(bx, by));
                let _ = wb.set_size(tauri::PhysicalSize::new(bw as u32, bh as u32));
                let _ = wb.show();
                let _ = wb.set_ignore_cursor_events(true);
                exclude_from_capture(&wb, true);
                #[cfg(windows)]
                {
                    let hb = hwnd_of(&wb);
                    crate::winutil::show_click_through_rect(hb, bx, by, bw, bh);
                    let wb_main = wb.clone();
                    let _ = app2.run_on_main_thread(move || {
                        let _ = wb_main.set_ignore_cursor_events(true);
                        crate::winutil::show_click_through_rect(hb, bx, by, bw, bh);
                    });
                }
            }
        }

        // 2. 定位并展示极简录屏悬浮控制胶囊条（优先选区正上方 -> 选区正下方 -> 全屏时屏幕顶部居中）
        let Some(w) = get_or_build(&app2, "recorder") else { return };
        exclude_from_capture(&w, true);

        #[cfg(target_os = "macos")]
        {
            let s = args.scale.max(1.0);
            let mon_w = (args.monitor_w as f32 / s).round() as i32;
            let mon_h = (args.monitor_h as f32 / s).round() as i32;
            let cx = args.monitor_x + (args.crop_x as f32 / s).round() as i32;
            let cy = args.monitor_y + (args.crop_y as f32 / s).round() as i32;
            let cw = (args.crop_w as f32 / s).round() as i32;
            let ch = (args.crop_h as f32 / s).round() as i32;
            let hud_w = 260;
            let hud_h = 46;
            let mon_right = args.monitor_x + mon_w;
            let mon_bottom = args.monitor_y + mon_h;
            let x = (cx + (cw - hud_w) / 2).clamp(args.monitor_x + 12, (mon_right - 380).max(args.monitor_x + 12));
            let y = if cy - args.monitor_y >= hud_h + 14 {
                cy - hud_h - 10
            } else if cy + ch + hud_h + 14 <= mon_bottom {
                cy + ch + 10
            } else {
                args.monitor_y + 28
            };
            let _ = w.set_size(tauri::LogicalSize::new(480.0, 56.0));
            let _ = w.set_position(tauri::LogicalPosition::new(x as f64, y as f64));
        }

        #[cfg(not(target_os = "macos"))]
        {
            let s = args.scale.max(1.0);
            let hud_w = (260.0 * s).round() as i32;
            let hud_max_w = (440.0 * s).round() as i32;
            let hud_h = (48.0 * s).round() as i32;
            let cx = args.monitor_x + args.crop_x as i32;
            let cy = args.monitor_y + args.crop_y as i32;
            let cw = args.crop_w as i32;
            let ch = args.crop_h as i32;
            let mon_right = args.monitor_x + args.monitor_w as i32;
            let mon_bottom = args.monitor_y + args.monitor_h as i32;
            let x = (cx + (cw - hud_w) / 2).clamp(
                args.monitor_x + 12,
                (mon_right - hud_max_w - 12).max(args.monitor_x + 12),
            );
            let y = if cy - args.monitor_y >= hud_h + 14 {
                cy - hud_h - 10
            } else if cy + ch + hud_h + 14 <= mon_bottom {
                cy + ch + 10
            } else {
                args.monitor_y + 18
            };
            let _ = w.set_size(tauri::LogicalSize::new(480.0, 56.0));
            let _ = w.set_position(PhysicalPosition::new(x, y));
        }

        let _ = w.show();
        exclude_from_capture(&w, true);
        #[cfg(windows)]
        {
            let h = hwnd_of(&w);
            crate::winutil::show_noactivate(h);
            crate::winutil::set_exclude_from_capture(h, true);
            let _ = app2.run_on_main_thread(move || {
                crate::winutil::show_noactivate(h);
                crate::winutil::set_exclude_from_capture(h, true);
            });
        }
    });
}

pub fn hide(app: &AppHandle, label: &str) {
    #[cfg(target_os = "macos")]
    {
        let label = label.to_string();
        run_win_task(app, move |app2| {
            if let Some(w) = app2.get_webview_window(&label) {
                let _ = w.hide();
            }
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Some(w) = app.get_webview_window(label) {
            let _ = w.hide();
            crate::winutil::hide_hwnd(hwnd_of(&w));
        }
    }
}

/// 打开结果浮窗（执行智能体/本地动作）。payload.meta.action 指定动作 id
pub fn open_result(app: &AppHandle, p: Payload) {
    run_win_task(app, move |app2| {
        // 先隐藏全屏 capture 遮罩与 tip 胶囊，避免其隐藏时抢走新弹出的 result 浮窗焦点
        crate::capture::close(&app2);
        hide(&app2, "tip");
        let Some(w) = get_or_build(&app2, "result") else { return };
        let was_visible = w.is_visible().unwrap_or(false);
        let is_img_search = p.meta.get("action").and_then(|v| v.as_str()) == Some("image_search");
        if !was_visible || is_img_search {
            let ui = app2.state::<crate::AppState>().settings.get().ui;
            let mut rw = ui.result_width.clamp(360.0, 1400.0);
            let mut rh = ui.result_height.clamp(240.0, 1200.0);
            if is_img_search {
                rw = rw.max(660.0);
                rh = rh.max(580.0);
            }
            let _ = w.set_size(tauri::LogicalSize::new(rw, rh));
            if !was_visible {
                place_near_cursor(&w, 16, 20);
            }
        }
        deliver(&app2, "result", p);
        let _ = w.show();
        let _ = w.set_focus();
        let h = hwnd_of(&w);
        crate::winutil::focus_hwnd(h);
        #[cfg(windows)]
        {
            let _ = app2.run_on_main_thread(move || {
                crate::winutil::focus_hwnd(h);
            });
        }
    });
}

pub fn open_launcher(app: &AppHandle) {
    run_win_task(app, move |app2| {
        let Some(w) = get_or_build(&app2, "launcher") else { return };
        let hwnd = crate::winutil::foreground_hwnd();
        let src = crate::winutil::foreground_process();
        let p = Payload::text(String::new(), Origin::Launcher, src, hwnd);
        deliver(&app2, "launcher", p);
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        let h = hwnd_of(&w);
        crate::winutil::focus_hwnd(h);
        #[cfg(windows)]
        {
            let _ = app2.run_on_main_thread(move || {
                crate::winutil::focus_hwnd(h);
            });
        }
    });
}

static PIN_SEQ: AtomicU32 = AtomicU32::new(1);

/// 钉图：每张图一个置顶小窗
pub fn open_pin(app: &AppHandle, image: String, x: i32, y: i32, w: f64, h: f64) {
    let label = format!("pin-{}", PIN_SEQ.fetch_add(1, Ordering::SeqCst));
    run_win_task(app, move |app2| {
        let Ok(win) = builder(&app2, &label, "pin")
            .title("miniBot 钉图")
            .inner_size(w, h)
            .decorations(false)
            .transparent(true)
            .shadow(true)
            .resizable(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible(false)
            .build()
        else {
            return;
        };
        let mut p = Payload::text(String::new(), Origin::Capture, None, 0);
        p.kind = crate::payload::PayloadKind::Image;
        p.image = Some(image);
        deliver(&app2, &label, p);
        let _ = win.set_position(PhysicalPosition::new(x, y));
        let _ = win.show();
    });
}

pub fn open_qr(app: &AppHandle, text: String) {
    run_win_task(app, move |app2| {
        let w = match app2.get_webview_window("qr") {
            Some(w) => w,
            None => match builder(&app2, "qr", "qr")
                .title("生成二维码")
                .inner_size(440.0, 580.0)
                .decorations(false)
                .shadow(true)
                .resizable(false)
                .always_on_top(true)
                .visible(false)
                .build()
            {
                Ok(w) => w,
                Err(_) => return,
            },
        };
        let p = Payload::text(text, Origin::Launcher, None, 0);
        deliver(&app2, "qr", p);
        let _ = w.show();
        let _ = w.set_focus();
    });
}

/// 打开 LocalSend 局域网文件/文本/截图快传浮窗
pub fn open_localsend(app: &AppHandle, p: Payload) {
    run_win_task(app, move |app2| {
        let w = match app2.get_webview_window("localsend") {
            Some(w) => w,
            None => match builder(&app2, "localsend", "localsend")
                .title("miniBot · LocalSend 局域网快传")
                .inner_size(560.0, 620.0)
                .min_inner_size(460.0, 480.0)
                .decorations(false)
                .shadow(true)
                .resizable(true)
                .maximizable(false)
                .always_on_top(true)
                .center()
                .visible(false)
                .build()
            {
                Ok(w) => w,
                Err(e) => {
                    crate::log_msg(&app2, format!("[wins] 创建 localsend 窗口失败：{e}"));
                    return;
                }
            },
        };
        deliver(&app2, "localsend", p);
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        let h = hwnd_of(&w);
        crate::winutil::focus_hwnd(h);
        #[cfg(windows)]
        {
            let _ = app2.run_on_main_thread(move || {
                crate::winutil::focus_hwnd(h);
            });
        }
        crate::capture::close(&app2);
        hide(&app2, "tip");
    });
}

/// 普通页面窗口（设置 / 历史）：无原生标题栏极简风格；若从 AI 聊天框跳转打开则先隐藏聊天框，关闭页面时自动恢复聊天框
pub fn open_page(app: &AppHandle, route: &str, title: &str) {
    let route = route.to_string();
    let title = title.to_string();
    run_win_task(app, move |app2| {
        if let Some(chat_win) = app2.get_webview_window("launcher") {
            if chat_win.is_visible().unwrap_or(false) {
                RETURN_TO_CHAT.store(true, Ordering::SeqCst);
                let _ = chat_win.hide();
            }
        }
        if let Some(w) = app2.get_webview_window(&route) {
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
            let h = hwnd_of(&w);
            crate::winutil::focus_hwnd(h);
            return;
        }
        match builder(&app2, &route, &route)
            .title(format!("miniBot · {title}"))
            .inner_size(940.0, 680.0)
            .min_inner_size(680.0, 480.0)
            .decorations(false)
            .shadow(true)
            .center()
            .build()
        {
            Ok(w) => {
                let app_evt = app2.clone();
                w.on_window_event(move |evt| {
                    if matches!(evt, tauri::WindowEvent::Destroyed) {
                        if RETURN_TO_CHAT.swap(false, Ordering::SeqCst) {
                            open_launcher(&app_evt);
                        }
                    }
                });
                let _ = w.show();
                let _ = w.set_focus();
                let h = hwnd_of(&w);
                crate::winutil::focus_hwnd(h);
            }
            Err(e) => crate::log_msg(&app2, format!("[wins] 打开页面 {route} 失败：{e}")),
        }
    });
}

/// 关闭设置/历史页面并在需要时恢复 AI 聊天主窗口
pub fn close_page(app: &AppHandle, label: &str) {
    let label = label.to_string();
    run_win_task(app, move |app2| {
        if let Some(w) = app2.get_webview_window(&label) {
            let _ = w.hide();
            let _ = w.close();
        }
        if RETURN_TO_CHAT.swap(false, Ordering::SeqCst) {
            open_launcher(&app2);
        }
    });
}

/// 打开独立高性能 Markdown 文档查看与编辑器窗口
pub fn open_doc_editor(app: &AppHandle, path: Option<String>, query: Option<String>) {
    let p_clone = path.clone();
    let q_clone = query.clone();
    let st = app.state::<crate::AppState>();
    if let Some(p) = &path {
        *st.pending_doc.lock().unwrap() = Some((p.clone(), query.clone()));
    }
    run_win_task(app, move |app2| {
        let payload_json = serde_json::json!({
            "path": p_clone,
            "query": q_clone,
        });
        if let Some(w) = app2.get_webview_window("editor") {
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
            let _ = app2.emit_to("editor", "minibot://open-doc", payload_json);
            let h = hwnd_of(&w);
            crate::winutil::focus_hwnd(h);
            return;
        }
        match builder(&app2, "editor", "editor")
            .title("miniBot · Markdown 编辑器")
            .inner_size(1080.0, 760.0)
            .min_inner_size(680.0, 480.0)
            .decorations(false)
            .shadow(true)
            .center()
            .build()
        {
            Ok(w) => {
                let _ = w.show();
                let _ = w.set_focus();
                let _ = app2.emit_to("editor", "minibot://open-doc", payload_json);
                let h = hwnd_of(&w);
                crate::winutil::focus_hwnd(h);
            }
            Err(e) => crate::log_msg(&app2, format!("[wins] 打开 Markdown 编辑器失败：{e}")),
        }
    });
}
