//! 全局热键注册（支持 2 键、3 键如 Ctrl+Alt+A / Ctrl+Shift+X、4 键如 Ctrl+Alt+Shift+A 及单功能键 F1~F12）
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

/// 规范化快捷键字符串，兼容多修饰键（Ctrl+Alt+A、Ctrl+Shift+A、Alt+Shift+Q 等）及各种别名写法
pub fn normalize_shortcut(raw: &str) -> String {
    let cleaned = raw.replace('＋', "+").replace(' ', "");
    if cleaned.is_empty() {
        return String::new();
    }
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut super_key = false;
    let mut main_key = String::new();

    for part in cleaned.split('+').filter(|s| !s.is_empty()) {
        let lower = part.to_lowercase();
        match lower.as_str() {
            "ctrl" | "control" | "ctl" => ctrl = true,
            "alt" | "option" | "opt" => alt = true,
            "shift" => shift = true,
            "win" | "super" | "meta" | "cmd" | "command" => super_key = true,
            "space" | "空格" => main_key = "Space".into(),
            "esc" | "escape" => main_key = "Escape".into(),
            "enter" | "return" => main_key = "Enter".into(),
            "tab" => main_key = "Tab".into(),
            "backspace" => main_key = "Backspace".into(),
            "delete" | "del" => main_key = "Delete".into(),
            "insert" | "ins" => main_key = "Insert".into(),
            "home" => main_key = "Home".into(),
            "end" => main_key = "End".into(),
            "pageup" | "pgup" => main_key = "PageUp".into(),
            "pagedown" | "pgdn" => main_key = "PageDown".into(),
            "up" | "arrowup" => main_key = "ArrowUp".into(),
            "down" | "arrowdown" => main_key = "ArrowDown".into(),
            "left" | "arrowleft" => main_key = "ArrowLeft".into(),
            "right" | "arrowright" => main_key = "ArrowRight".into(),
            "printscreen" | "prtsc" => main_key = "PrintScreen".into(),
            "backquote" | "`" | "~" => main_key = "Backquote".into(),
            "minus" | "-" => main_key = "Minus".into(),
            "equal" | "=" => main_key = "Equal".into(),
            "bracketleft" | "[" => main_key = "BracketLeft".into(),
            "bracketright" | "]" => main_key = "BracketRight".into(),
            "backslash" | "\\" => main_key = "Backslash".into(),
            "semicolon" | ";" => main_key = "Semicolon".into(),
            "quote" | "'" => main_key = "Quote".into(),
            "comma" | "," => main_key = "Comma".into(),
            "period" | "." => main_key = "Period".into(),
            "slash" | "/" => main_key = "Slash".into(),
            _ => {
                if part.len() == 1 {
                    main_key = part.to_uppercase();
                } else if lower.starts_with("key") && part.len() == 4 {
                    main_key = part[3..].to_uppercase();
                } else if lower.starts_with("digit") && part.len() == 6 {
                    main_key = part[5..].to_string();
                } else {
                    // 如 F1~F12 等保持首字母大写
                    let mut chars = part.chars();
                    if let Some(first) = chars.next() {
                        main_key = format!("{}{}", first.to_uppercase(), chars.as_str().to_lowercase());
                    }
                }
            }
        }
    }

    if main_key.is_empty() {
        return raw.trim().to_string();
    }

    let mut out = Vec::new();
    if ctrl {
        out.push("Ctrl".to_string());
    }
    if alt {
        out.push("Alt".to_string());
    }
    if shift {
        out.push("Shift".to_string());
    }
    if super_key {
        out.push("Super".to_string());
    }
    out.push(main_key);
    out.join("+")
}

/// 按设置重新注册所有热键，返回失败列表（如被其他软件占用）
pub fn register(app: &AppHandle) -> Vec<String> {
    let st = app.state::<crate::AppState>();
    let hk = st.settings.get().hotkeys;
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let mut map = st.hotkey_map.lock().unwrap();
    map.clear();
    let mut errors = vec![];
    for (action, key, label) in [
        ("capture", &hk.capture, "区域截图识别"),
        ("launcher", &hk.launcher, "呼出 AI 聊天框"),
        ("selection", &hk.selection, "处理当前选中文字"),
        ("color", &hk.color, "屏幕取色器"),
    ] {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            continue;
        }
        let norm = normalize_shortcut(trimmed);
        match norm.parse::<Shortcut>().or_else(|_| trimmed.parse::<Shortcut>()) {
            Ok(sc) => match gs.register(sc) {
                Ok(()) => {
                    map.insert(sc.id(), action.to_string());
                }
                Err(e) => errors.push(format!("{label}（{norm}）注册失败，可能被其他软件占用：{e}")),
            },
            Err(e) => errors.push(format!("{label}（{trimmed}）格式无效：{e}")),
        }
    }
    *st.hotkey_errors.lock().unwrap() = errors.clone();
    errors
}

pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, sc, ev| {
            if ev.state() != ShortcutState::Pressed {
                return;
            }
            let action = app.state::<crate::AppState>().hotkey_map.lock().unwrap().get(&sc.id()).cloned();
            if let Some(a) = action {
                crate::actions::trigger(app, &a);
            }
        })
        .build()
}
