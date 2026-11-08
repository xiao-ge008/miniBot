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

fn build_tip(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    builder(app, "tip", "tip")
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
        .build()
}

fn build_result(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let ui = app.state::<crate::AppState>().settings.get().ui;
    let rw = ui.result_width.clamp(360.0, 1400.0);
    let rh = ui.result_height.clamp(240.0, 1200.0);
    builder(app, "result", "result")
        .title("miniBot")
        .inner_size(rw, rh)
        .min_inner_size(340.0, 220.0)
        .decorations(false)
        .shadow(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()
}

fn build_capture(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    builder(app, "capture", "capture")
        .title("miniBot 截图")
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()
}

fn build_launcher(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    builder(app, "launcher", "launcher")
        .title("miniBot · AI 聊天")
        .inner_size(1060.0, 740.0)
        .min_inner_size(760.0, 520.0)
        .decorations(false)
        .shadow(true)
        .resizable(true)
        .maximizable(false)
        .center()
        .visible(false)
        .build()
}

/// 取常驻浮窗；若尚未创建（或预创建失败）则立即创建
/// 注意：WebviewWindowBuilder::build() 严禁在主线程回调（同步命令 / run_on_main_thread）内调用，否则 WebView2 会死锁
pub fn get_or_build(app: &AppHandle, label: &str) -> Option<WebviewWindow> {
    if let Some(w) = app.get_webview_window(label) {
        return Some(w);
    }
    let r = match label {
        "tip" => build_tip(app),
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
        ("tip", build_tip as fn(&AppHandle) -> tauri::Result<WebviewWindow>),
        ("result", build_result),
        ("capture", build_capture),
        ("launcher", build_launcher),
    ] {
        match f(app) {
            Ok(_) => crate::log_msg(app, format!("[setup] 预创建 {name} 成功")),
            Err(e) => crate::log_msg(app, format!("[setup] 预创建 {name} 失败：{e}")),
        }
    }
    Ok(())
}

/// 把窗口放到光标附近并限制在当前显示器内
fn place_near_cursor(w: &WebviewWindow, dx: i32, dy: i32) {
    let (cx, cy) = crate::winutil::cursor_pos();
    let size = w.outer_size().unwrap_or_default();
    let sw = (size.width as i32).max(260);
    let sh = (size.height as i32).max(48);
    let mut x = cx + dx;
    let mut y = cy + dy;
    let mon = w
        .monitor_from_point(cx as f64, cy as f64)
        .ok()
        .flatten()
        .or_else(|| w.current_monitor().ok().flatten())
        .or_else(|| w.primary_monitor().ok().flatten());
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

pub fn show_tip(app: &AppHandle, p: Payload) {
    let app2 = app.clone();
    std::thread::spawn(move || {
        let Some(w) = get_or_build(&app2, "tip") else { return };
        // 参考千问桌面端划词栏：默认悬浮在选中文本正上方偏左位置
        place_near_cursor(&w, -110, -68);
        deliver(&app2, "tip", p);
        let _ = w.show();
        let h = hwnd_of(&w);
        crate::winutil::show_noactivate(h);
        let _ = app2.run_on_main_thread(move || {
            crate::winutil::show_noactivate(h);
        });
        std::thread::sleep(std::time::Duration::from_millis(25));
        crate::winutil::show_noactivate(h);
    });
}

pub fn hide(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.hide();
        crate::winutil::hide_hwnd(hwnd_of(&w));
    }
}

/// 打开结果浮窗（执行智能体/本地动作）。payload.meta.action 指定动作 id
pub fn open_result(app: &AppHandle, p: Payload) {
    let app2 = app.clone();
    std::thread::spawn(move || {
        // 先隐藏全屏 capture 遮罩与 tip 胶囊，避免其隐藏时抢走新弹出的 result 浮窗焦点
        crate::capture::close(&app2);
        hide(&app2, "tip");
        let Some(w) = get_or_build(&app2, "result") else { return };
        let was_visible = w.is_visible().unwrap_or(false);
        if !was_visible {
            let ui = app2.state::<crate::AppState>().settings.get().ui;
            let rw = ui.result_width.clamp(360.0, 1400.0);
            let rh = ui.result_height.clamp(240.0, 1200.0);
            let _ = w.set_size(tauri::LogicalSize::new(rw, rh));
            place_near_cursor(&w, 16, 20);
        }
        deliver(&app2, "result", p);
        let _ = w.show();
        let _ = w.set_focus();
        let h = hwnd_of(&w);
        crate::winutil::focus_hwnd(h);
        let _ = app2.run_on_main_thread(move || {
            crate::winutil::focus_hwnd(h);
        });
    });
}

pub fn open_launcher(app: &AppHandle) {
    let app2 = app.clone();
    std::thread::spawn(move || {
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
        let _ = app2.run_on_main_thread(move || {
            crate::winutil::focus_hwnd(h);
        });
    });
}

static PIN_SEQ: AtomicU32 = AtomicU32::new(1);

/// 钉图：每张图一个置顶小窗
pub fn open_pin(app: &AppHandle, image: String, x: i32, y: i32, w: f64, h: f64) {
    let label = format!("pin-{}", PIN_SEQ.fetch_add(1, Ordering::SeqCst));
    let app2 = app.clone();
    std::thread::spawn(move || {
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
    let app2 = app.clone();
    std::thread::spawn(move || {
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
    let app2 = app.clone();
    std::thread::spawn(move || {
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
        let _ = app2.run_on_main_thread(move || {
            crate::winutil::focus_hwnd(h);
        });
        crate::capture::close(&app2);
        hide(&app2, "tip");
    });
}

/// 普通页面窗口（设置 / 历史）：无原生标题栏极简风格；若从 AI 聊天框跳转打开则先隐藏聊天框，关闭页面时自动恢复聊天框
pub fn open_page(app: &AppHandle, route: &str, title: &str) {
    let app2 = app.clone();
    let route = route.to_string();
    let title = title.to_string();
    std::thread::spawn(move || {
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
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.hide();
        let _ = w.close();
    }
    if RETURN_TO_CHAT.swap(false, Ordering::SeqCst) {
        open_launcher(app);
    }
}
