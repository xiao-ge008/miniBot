//! 剪贴板监听：轮询序号（极低开销），识别文字/图片 → 弹出 Tip
use crate::payload::{Origin, Payload, PayloadKind};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
#[cfg(windows)]
use tauri::Emitter;
use tauri::{AppHandle, Manager};

#[derive(Default)]
pub struct ClipWatcher {
    /// 用户手动暂停到某个 UNIX 秒
    paused_until: AtomicU64,
    /// 程序内部临时抑制（模拟复制/粘贴期间），UNIX 毫秒
    suppress_until: AtomicU64,
    /// 自己写入剪贴板后需忽略的序号
    ignore_seq: AtomicU32,
    /// 最近一次触发的文本与毫秒时间戳（仅用于 500ms 内的单次按键重复去抖）
    last_text: Mutex<(String, u64)>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

/// 带重试读取剪贴板文本（Windows 下 OpenClipboard 在 get_text 内部调用，需重试以避开向日葵/FDM/浏览器瞬时锁）
pub fn read_text_retry() -> Option<String> {
    let mut cb = arboard::Clipboard::new().ok()?;
    for i in 0..12 {
        match cb.get_text() {
            Ok(t) => return Some(t),
            Err(_) => {
                if i < 11 {
                    std::thread::sleep(Duration::from_millis(28));
                }
            }
        }
    }
    None
}

/// 带重试读取剪贴板图片
pub fn read_image_retry() -> Option<arboard::ImageData<'static>> {
    let mut cb = arboard::Clipboard::new().ok()?;
    for i in 0..8 {
        match cb.get_image() {
            Ok(img) => return Some(img),
            Err(_) => {
                if i < 7 {
                    std::thread::sleep(Duration::from_millis(28));
                }
            }
        }
    }
    None
}

/// 带重试写入剪贴板文本
pub fn set_text_retry(text: String) -> bool {
    let Ok(mut cb) = arboard::Clipboard::new() else { return false };
    for i in 0..8 {
        if cb.set_text(text.clone()).is_ok() {
            return true;
        }
        if i < 7 {
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    false
}

fn is_self_or_blacklisted(fg: &str, blacklist: &[String]) -> bool {
    let f = fg.trim().to_lowercase();
    let f_stem = f.trim_end_matches(".exe").trim_end_matches(".app");
    if f_stem == "minibot" {
        return true;
    }
    blacklist.iter().any(|b| {
        let bl = b.trim().to_lowercase();
        let bl_stem = bl.trim_end_matches(".exe").trim_end_matches(".app");
        !bl_stem.is_empty() && (bl == f || bl_stem == f_stem)
    })
}

impl ClipWatcher {
    pub fn pause_for(&self, secs: u64) {
        self.paused_until.store(if secs == 0 { 0 } else { now() + secs }, Ordering::SeqCst);
    }

    pub fn is_paused(&self) -> bool {
        self.paused_until.load(Ordering::SeqCst) > now() || self.suppress_until.load(Ordering::SeqCst) > now_ms()
    }

    /// 临时抑制 Tip（ms=0 解除）
    pub fn suppress(&self, ms: u64) {
        self.suppress_until.store(if ms == 0 { 0 } else { now_ms() + ms }, Ordering::SeqCst);
    }

    /// 自己即将写剪贴板：写完后调用 mark_self_write
    pub fn mark_self_write(&self) {
        self.ignore_seq.store(crate::winutil::clipboard_seq(), Ordering::SeqCst);
    }

    pub fn spawn(app: AppHandle) {
        #[cfg(windows)]
        Self::spawn_mouse_selection(app.clone());
        std::thread::spawn(move || {
            let mut last_seq = crate::winutil::clipboard_seq();
            loop {
                std::thread::sleep(Duration::from_millis(120));
                let seq = crate::winutil::clipboard_seq();
                if seq == last_seq {
                    continue;
                }
                last_seq = seq;
                let st = app.state::<crate::AppState>();
                let w = &st.clip;
                if w.ignore_seq.load(Ordering::SeqCst) == seq {
                    continue;
                }
                let tip = st.settings.get().tip;
                if !tip.enabled || w.is_paused() {
                    continue;
                }
                let fg = crate::winutil::foreground_process().unwrap_or_default().to_lowercase();
                if is_self_or_blacklisted(&fg, &tip.blacklist) {
                    continue;
                }
                // 稍等浏览器/编辑器完成多格式写入，并带重试读取
                std::thread::sleep(Duration::from_millis(45));
                if let Some(text) = read_text_retry() {
                    // 同步最新 seq，避免同一轮多步写入重复触发
                    last_seq = crate::winutil::clipboard_seq();
                    let t = text.trim();
                    let n = t.chars().count();
                    if n < tip.min_chars || n > tip.max_chars {
                        continue;
                    }
                    {
                        let ts = now_ms();
                        let mut last = w.last_text.lock().unwrap();
                        if last.0 == t && ts.saturating_sub(last.1) < 500 {
                            continue;
                        }
                        *last = (t.to_string(), ts);
                    }
                    crate::log_msg(&app, format!("[clip] 检测到复制文本 ({n} chars) from {fg} -> show_tip"));
                    let p = Payload::text(t.to_string(), Origin::Clipboard, Some(fg), crate::winutil::foreground_hwnd());
                    crate::wins::show_tip(&app, p);
                } else if let Some(img) = read_image_retry() {
                    last_seq = crate::winutil::clipboard_seq();
                    if img.width < 8 || img.height < 8 {
                        continue;
                    }
                    let Some(rgba) = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned()) else { continue };
                    let png = crate::imgutil::encode_png(&rgba);
                    crate::log_msg(&app, format!("[clip] 检测到复制图片 ({}x{}) from {fg} -> show_tip", img.width, img.height));
                    let mut p = Payload::image(png, Origin::Clipboard, Some(fg), crate::winutil::foreground_hwnd());
                    // 后台顺手检测二维码，决定 Tip 是否显示「识别二维码」
                    let codes = crate::qr::decode_rgba(&rgba);
                    if !codes.is_empty() {
                        p.meta = serde_json::json!({ "qr": codes });
                    }
                    crate::wins::show_tip(&app, p);
                }
            }
        });
    }

    /// 千问桌面端同款「鼠标划词即出 Tip」+「点击外部自动收起 Tip」（仅 Windows 原生 Win32 下启用，避免 macOS 后台线程轮询 WindowServer/NSWindow 引发死锁）
    #[cfg(windows)]
    fn spawn_mouse_selection(app: AppHandle) {
        std::thread::spawn(move || {
            let mut was_down = false;
            let mut down_pos = (0i32, 0i32);
            let mut down_ms = 0u64;
            let mut saw_ibeam = false;
            let mut last_up_ms = 0u64;
            let mut last_up_pos = (0i32, 0i32);

            loop {
                std::thread::sleep(Duration::from_millis(25));
                let is_down = crate::winutil::is_mouse_left_down();
                let pos = crate::winutil::cursor_pos();

                if is_down && !was_down {
                    // 鼠标左键刚按下：若点击在 Tip 窗口外，立即隐藏 Tip
                    if let Some(tip_win) = app.get_webview_window("tip") {
                        if tip_win.is_visible().unwrap_or(false) {
                            if !crate::wins::point_in_win(&tip_win, pos.0, pos.1) {
                                let _ = tip_win.hide();
                            }
                        }
                    }
                    // 若点击在聊天主窗口（launcher）或结果浮窗（result）矩形外，通知前端按未置顶规则自动收起
                    let capture_visible = app
                        .get_webview_window("capture")
                        .and_then(|w| w.is_visible().ok())
                        .unwrap_or(false);
                    if !capture_visible {
                        for label in ["launcher", "result"] {
                            if let Some(win) = app.get_webview_window(label) {
                                if win.is_visible().unwrap_or(false) {
                                    if !crate::wins::point_in_win(&win, pos.0, pos.1) {
                                        let _ = app.emit_to(label, "minibot://outside-click", ());
                                    }
                                }
                            }
                        }
                    }
                    was_down = true;
                    down_pos = pos;
                    down_ms = now_ms();
                    saw_ibeam = crate::winutil::is_cursor_ibeam();
                } else if is_down && was_down {
                    if !saw_ibeam && crate::winutil::is_cursor_ibeam() {
                        saw_ibeam = true;
                    }
                } else if !is_down && was_down {
                    was_down = false;
                    let up_ms = now_ms();
                    let up_ibeam = crate::winutil::is_cursor_ibeam();
                    let dx = (pos.0 - down_pos.0) as f64;
                    let dy = (pos.1 - down_pos.1) as f64;
                    let dist = (dx * dx + dy * dy).sqrt();
                    let dur = up_ms.saturating_sub(down_ms);

                    let d_last = (((pos.0 - last_up_pos.0).pow(2) + (pos.1 - last_up_pos.1).pow(2)) as f64).sqrt();
                    let is_dblclick = dist < 8.0 && dur < 320 && up_ms.saturating_sub(last_up_ms) < 380 && d_last < 10.0 && up_ibeam;
                    let is_drag_sel = dist >= 16.0 && (65..=10000).contains(&dur) && (saw_ibeam || up_ibeam);

                    last_up_ms = up_ms;
                    last_up_pos = pos;

                    if !(is_drag_sel || is_dblclick) {
                        continue;
                    }

                    let st = app.state::<crate::AppState>();
                    let tip = st.settings.get().tip;
                    if !tip.enabled || st.clip.is_paused() {
                        continue;
                    }
                    let fg = crate::winutil::foreground_process().unwrap_or_default().to_lowercase();
                    if fg.is_empty() || is_self_or_blacklisted(&fg, &tip.blacklist) {
                        continue;
                    }

                    // 稍作等待，确保浏览器完成选区状态更新且用户未继续点击
                    std::thread::sleep(Duration::from_millis(65));
                    if crate::winutil::is_mouse_left_down() {
                        continue;
                    }
                    let hwnd = crate::winutil::foreground_hwnd();
                    if let Some(text) = crate::actions::grab_selection_clean(&app) {
                        let t = text.trim();
                        let n = t.chars().count();
                        if n < tip.min_chars || n > tip.max_chars {
                            continue;
                        }
                        crate::log_msg(&app, format!("[selection-mouse] 划词获取文本 ({n} chars) from {fg} -> show_tip"));
                        let p = Payload::text(t.to_string(), Origin::Selection, Some(fg), hwnd);
                        crate::wins::show_tip(&app, p);
                    }
                }
            }
        });
    }
}

pub fn kind_of_text(t: &str) -> PayloadKind {
    let s = t.trim();
    let lower = s.to_lowercase();
    if !s.contains(char::is_whitespace) && (lower.starts_with("http://") || lower.starts_with("https://")) {
        let video_ext = [".mp4", ".mkv", ".mov", ".webm", ".avi", ".flv", ".m3u8"];
        if video_ext.iter().any(|e| lower.split('?').next().unwrap_or("").ends_with(e)) {
            return PayloadKind::Video;
        }
        return PayloadKind::Url;
    }
    let is_hex = s.starts_with('#') && matches!(s.len(), 4 | 7 | 9) && s[1..].chars().all(|c| c.is_ascii_hexdigit());
    if is_hex || lower.starts_with("rgb(") || lower.starts_with("rgba(") || lower.starts_with("hsl(") {
        return PayloadKind::Color;
    }
    PayloadKind::Text
}
