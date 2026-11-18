//! 剪贴板监听：轮询序号（极低开销），识别文字/图片 → 弹出 Tip
use crate::payload::{Origin, Payload, PayloadKind};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
#[cfg(windows)]
use tauri::Emitter;
use tauri::{AppHandle, Manager};

pub const ZERO_WIDTH_FINGERPRINT: &str = "\u{200B}\u{200C}\u{200B}";

#[derive(Default)]
pub struct ClipWatcher {
    /// 用户手动暂停到某个 UNIX 秒
    paused_until: AtomicU64,
    /// 程序内部临时抑制（模拟复制/粘贴期间），UNIX 毫秒
    suppress_until: AtomicU64,
    /// 网页插件触发的跨端防冲突抑制，UNIX 毫秒（独立于内部模拟复制，防止被重置）
    ext_suppress_until: AtomicU64,
    /// 自己写入剪贴板后需忽略的序号
    ignore_seq: AtomicU32,
    /// 最近一次触发的文本与毫秒时间戳（用于短时间重复去抖与插件清洗去重）
    last_text: Mutex<(String, u64)>,
    /// 最近一次写入或捕获的图片特征 (宽, 高, 字节大小, 时间戳)，防止截屏复制等自身写入触发二次 Tip
    last_image_sig: Mutex<Option<(usize, usize, usize, u64)>>,
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
        let n_ms = now_ms();
        self.paused_until.load(Ordering::SeqCst) > now()
            || self.suppress_until.load(Ordering::SeqCst) > n_ms
            || self.ext_suppress_until.load(Ordering::SeqCst) > n_ms
    }

    /// 临时抑制 Tip（ms=0 解除内部模拟复制抑制，不影响插件外部抑制）
    pub fn suppress(&self, ms: u64) {
        self.suppress_until.store(if ms == 0 { 0 } else { now_ms() + ms }, Ordering::SeqCst);
    }

    /// 来自网页插件或零宽指纹的外部抑制（只增不减，不会被内部 grab_selection 重置）
    pub fn suppress_from_ext(&self, ms: u64) {
        let target = now_ms() + ms;
        let cur = self.ext_suppress_until.load(Ordering::SeqCst);
        if target > cur {
            self.ext_suppress_until.store(target, Ordering::SeqCst);
        }
    }

    /// 检查当前是否处于网页插件抑制窗口期
    pub fn is_ext_suppressed(&self) -> bool {
        self.ext_suppress_until.load(Ordering::SeqCst) > now_ms()
    }

    /// 记录最近一次处理的文本，避免插件清洗写回或短时间重复触发二次弹窗
    pub fn record_last_text(&self, text: &str) {
        let mut last = self.last_text.lock().unwrap();
        *last = (text.trim().to_string(), now_ms());
    }

    /// 自己即将写剪贴板：写完后调用 mark_self_write
    pub fn mark_self_write(&self) {
        self.ignore_seq.store(crate::winutil::clipboard_seq(), Ordering::SeqCst);
    }

    /// 登记自身即将写入或刚写入的图片指纹，防止剪贴板监听二次弹 Tip
    pub fn record_self_image(&self, width: usize, height: usize, byte_len: usize) {
        let mut last_sig = self.last_image_sig.lock().unwrap();
        *last_sig = Some((width, height, byte_len, now_ms()));
    }

    /// 检查图片是否与最近写入的指纹匹配或短时间内重复
    pub fn should_ignore_image(&self, width: usize, height: usize, byte_len: usize) -> bool {
        let ts = now_ms();
        let mut last_sig = self.last_image_sig.lock().unwrap();
        if let Some((lw, lh, llen, lts)) = *last_sig {
            if lw == width && lh == height && llen == byte_len && ts.saturating_sub(lts) < 2500 {
                return true;
            }
        }
        *last_sig = Some((width, height, byte_len, ts));
        false
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
                // 稍等浏览器/编辑器完成多格式写入，并带重试读取
                std::thread::sleep(Duration::from_millis(45));
                if let Some(text) = read_text_retry() {
                    // 方案 A：优先检测是否来自网页插件的零宽防冲突指纹（即使已处于 paused/suppress 态也必须清洗零宽字符）
                    if text.contains(ZERO_WIDTH_FINGERPRINT) {
                        crate::log_msg(&app, "[clip] 检测到网页插件专属标记，抑制桌面 Tip 并清洗剪贴板".to_string());
                        let cleaned = text.replace(ZERO_WIDTH_FINGERPRINT, "");
                        w.record_last_text(&cleaned);
                        w.suppress_from_ext(1800);
                        if set_text_retry(cleaned) {
                            w.mark_self_write();
                        }
                        last_seq = crate::winutil::clipboard_seq();
                        crate::wins::hide(&app, "tip");
                        continue;
                    }

                    // 同步最新 seq，避免同一轮多步写入重复触发
                    last_seq = crate::winutil::clipboard_seq();
                    if !tip.enabled || w.is_paused() {
                        continue;
                    }
                    let fg = crate::winutil::foreground_process().unwrap_or_default().to_lowercase();
                    if is_self_or_blacklisted(&fg, &tip.blacklist) {
                        continue;
                    }
                    let t = text.trim();
                    let n = t.chars().count();
                    if n < tip.min_chars || n > tip.max_chars {
                        continue;
                    }
                    {
                        let ts = now_ms();
                        let mut last = w.last_text.lock().unwrap();
                        if last.0 == t && ts.saturating_sub(last.1) < 1500 {
                            continue;
                        }
                        *last = (t.to_string(), ts);
                    }
                    crate::log_msg(&app, format!("[clip] 检测到复制文本 ({n} chars) from {fg} -> show_tip"));
                    let p = Payload::text(t.to_string(), Origin::Clipboard, Some(fg), crate::winutil::foreground_hwnd());
                    crate::wins::show_tip(&app, p);
                } else {
                    if !tip.enabled || w.is_paused() {
                        last_seq = crate::winutil::clipboard_seq();
                        continue;
                    }
                    let fg = crate::winutil::foreground_process().unwrap_or_default().to_lowercase();
                    if is_self_or_blacklisted(&fg, &tip.blacklist) {
                        last_seq = crate::winutil::clipboard_seq();
                        continue;
                    }
                    if let Some(img) = read_image_retry() {
                        last_seq = crate::winutil::clipboard_seq();
                        if img.width < 8 || img.height < 8 {
                            continue;
                        }
                        if w.should_ignore_image(img.width, img.height, img.bytes.len()) {
                            crate::log_msg(&app, format!("[clip] 忽略自身写入或短时间内重复图片 ({}x{})", img.width, img.height));
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
            }
        });
    }

    /// 千问桌面端同款「鼠标划词即出 Tip」+「点击外部自动收起 Tip」（仅 Windows 原生 Win32 下启用，避免 macOS 后台线程轮询 WindowServer/NSWindow 引发死锁）
    #[cfg(windows)]
    fn spawn_mouse_selection(app: AppHandle) {
        std::thread::spawn(move || {
            crate::winutil::set_thread_dpi_aware();
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

                    // 稍作等待，确保浏览器完成选区状态更新（及插件 mouseup 预抑制信号送达）且用户未继续点击
                    std::thread::sleep(Duration::from_millis(65));
                    if crate::winutil::is_mouse_left_down() || st.clip.is_paused() {
                        continue;
                    }
                    let hwnd = crate::winutil::foreground_hwnd();
                    if let Some(text) = crate::actions::grab_selection_clean(&app) {
                        if st.clip.is_ext_suppressed() || text.contains(ZERO_WIDTH_FINGERPRINT) {
                            continue;
                        }
                        let t = text.trim();
                        let n = t.chars().count();
                        if n < tip.min_chars || n > tip.max_chars {
                            continue;
                        }
                        st.clip.record_last_text(t);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ext_suppress_isolated_from_internal_suppress() {
        let w = ClipWatcher::default();
        assert!(!w.is_paused());
        assert!(!w.is_ext_suppressed());

        // 网页插件发起 2000ms 外部抑制
        w.suppress_from_ext(2000);
        assert!(w.is_ext_suppressed());
        assert!(w.is_paused());

        // 即使内部模拟 Ctrl+C 结束时调用 suppress(0) 或 suppress(350)，也不得清除插件外部抑制
        w.suppress(0);
        assert!(w.is_ext_suppressed());
        assert!(w.is_paused());

        w.suppress(350);
        assert!(w.is_ext_suppressed());
        assert!(w.is_paused());

        // 验证清洗后文本登记
        let dirty = format!("测试跨端防冲突文本{}", ZERO_WIDTH_FINGERPRINT);
        let cleaned = dirty.replace(ZERO_WIDTH_FINGERPRINT, "");
        w.record_last_text(&cleaned);
        let last = w.last_text.lock().unwrap();
        assert_eq!(last.0, "测试跨端防冲突文本");
        assert!(last.1 > 0);
    }
}
