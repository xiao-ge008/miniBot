//! 跨平台系统原生小工具（Windows Win32 原生实现 + macOS/非 Windows 兼容层）
#![allow(unsafe_code)]

#[cfg(windows)]
mod imp {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, HWND, POINT};
    use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL, VK_LMENU, VK_LWIN, VK_MENU, VK_RMENU, VK_RWIN,
        VK_SHIFT,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
    };

    /// 根据 PID 取进程可执行文件名（如 python.exe）
    pub fn process_name(pid: u32) -> Option<String> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
            let _ = CloseHandle(h);
            ok.ok()?;
            let full = String::from_utf16_lossy(&buf[..len as usize]);
            full.rsplit('\\').next().map(|s| s.to_string())
        }
    }

    /// 当前前台窗口句柄（原始值，便于跨线程传递）
    pub fn foreground_hwnd() -> isize {
        unsafe { GetForegroundWindow().0 as isize }
    }

    /// 前台窗口所属进程名
    pub fn foreground_process() -> Option<String> {
        unsafe {
            let hwnd = GetForegroundWindow();
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == 0 {
                return None;
            }
            process_name(pid)
        }
    }

    pub fn focus_hwnd(hwnd: isize) {
        if hwnd == 0 {
            return;
        }
        use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
        use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
        use windows::Win32::UI::WindowsAndMessaging::{
            BringWindowToTop, GetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, HWND_NOTOPMOST,
            HWND_TOPMOST, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, WS_EX_TOPMOST,
        };
        unsafe {
            let h = HWND(hwnd as *mut _);
            let was_topmost = (GetWindowLongPtrW(h, GWL_EXSTYLE) & (WS_EX_TOPMOST.0 as isize)) != 0;
            let fg = GetForegroundWindow();
            let fg_tid = GetWindowThreadProcessId(fg, None);
            let cur_tid = GetCurrentThreadId();
            let attached = if fg_tid != 0 && fg_tid != cur_tid {
                AttachThreadInput(cur_tid, fg_tid, true).as_bool()
            } else {
                false
            };
            let _ = SetWindowPos(h, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
            let _ = BringWindowToTop(h);
            let _ = SetForegroundWindow(h);
            let _ = SetFocus(Some(h));
            if !was_topmost {
                let _ = SetWindowPos(h, Some(HWND_NOTOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
            }
            if attached {
                let _ = AttachThreadInput(cur_tid, fg_tid, false);
            }
        }
    }

    pub fn cursor_pos() -> (i32, i32) {
        let mut p = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut p);
        }
        (p.x, p.y)
    }

    /// 判断当前鼠标光标是否为文本选择光标（IDC_IBEAM），用于精准识别「鼠标划词」而不误触普通拖拽
    pub fn is_cursor_ibeam() -> bool {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetCursorInfo, LoadCursorW, CURSORINFO, CURSOR_SHOWING, IDC_IBEAM,
        };
        unsafe {
            let Ok(ibeam) = LoadCursorW(None, IDC_IBEAM) else {
                return false;
            };
            let mut ci = CURSORINFO {
                cbSize: std::mem::size_of::<CURSORINFO>() as u32,
                ..Default::default()
            };
            if GetCursorInfo(&mut ci).is_ok() && (ci.flags.0 & CURSOR_SHOWING.0) != 0 {
                return ci.hCursor == ibeam;
            }
            false
        }
    }

    /// 鼠标左键当前是否按下
    pub fn is_mouse_left_down() -> bool {
        use windows::Win32::UI::Input::KeyboardAndMouse::VK_LBUTTON;
        (unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } as u16 & 0x8000) != 0
    }

    /// 是否有任何修饰键正被按住
    pub fn any_modifier_down() -> bool {
        for k in [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN] {
            if (unsafe { GetAsyncKeyState(k.0 as i32) } as u16 & 0x8000) != 0 {
                return true;
            }
        }
        false
    }

    /// 判断屏幕坐标 (x, y) 是否在指定窗口矩形内
    pub fn point_in_hwnd(hwnd: isize, x: i32, y: i32) -> bool {
        if hwnd == 0 {
            return false;
        }
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsWindowVisible};
        unsafe {
            let h = HWND(hwnd as *mut _);
            if !IsWindowVisible(h).as_bool() {
                return false;
            }
            let mut r = RECT::default();
            if GetWindowRect(h, &mut r).is_ok() {
                return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
            }
            false
        }
    }

    pub fn clipboard_seq() -> u32 {
        unsafe { GetClipboardSequenceNumber() }
    }

    fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    /// 松开用户可能仍按着的修饰键和常用触发键（热键触发时 Alt/W/Space 等往往还按着）
    fn release_modifiers() {
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_LCONTROL, VK_LSHIFT, VK_RCONTROL, VK_RSHIFT, VK_SPACE};
        let keys = [
            VK_MENU,
            VK_LMENU,
            VK_RMENU,
            VK_SHIFT,
            VK_LSHIFT,
            VK_RSHIFT,
            VK_LWIN,
            VK_RWIN,
            VK_CONTROL,
            VK_LCONTROL,
            VK_RCONTROL,
            VK_SPACE,
            VIRTUAL_KEY(b'W' as u16),
            VIRTUAL_KEY(b'Q' as u16),
            VIRTUAL_KEY(b'C' as u16),
        ];
        let mut inputs = vec![];
        for k in keys {
            if unsafe { GetAsyncKeyState(k.0 as i32) } as u16 & 0x8000 != 0 {
                inputs.push(key(k, true));
            }
        }
        if !inputs.is_empty() {
            unsafe {
                SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
            }
            std::thread::sleep(std::time::Duration::from_millis(35));
        }
    }

    /// 模拟 Ctrl+<字母>（用于热键触发，先松开当前按住的修饰键）
    pub fn send_ctrl(letter: char) {
        release_modifiers();
        let vk = VIRTUAL_KEY(letter.to_ascii_uppercase() as u16);
        let inputs = [key(VK_CONTROL, false), key(vk, false), key(vk, true), key(VK_CONTROL, true)];
        unsafe {
            SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        }
    }

    /// 轻量模拟 Ctrl+C（用于鼠标划词松开后取词，不发送额外释放键）
    pub fn send_ctrl_c_clean() {
        let vk = VIRTUAL_KEY(b'C' as u16);
        let inputs = [key(VK_CONTROL, false), key(vk, false), key(vk, true), key(VK_CONTROL, true)];
        unsafe {
            SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        }
    }

    /// 设置窗口为「不抢焦点」的工具窗口（Tip 用）
    pub fn make_noactivate(hwnd: isize) {
        if hwnd == 0 {
            return;
        }
        use windows::Win32::UI::WindowsAndMessaging::{
            GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
        };
        unsafe {
            let h = HWND(hwnd as *mut _);
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
            SetWindowLongPtrW(h, GWL_EXSTYLE, ex | (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0) as isize);
        }
    }

    /// 显示窗口但不激活（不抢走用户当前输入焦点，且强制置顶于浏览器之上）
    pub fn show_noactivate(hwnd: isize) {
        if hwnd == 0 {
            return;
        }
        use windows::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, ShowWindow, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_SHOWNOACTIVATE,
        };
        make_noactivate(hwnd);
        unsafe {
            let h = HWND(hwnd as *mut _);
            let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
            let _ = SetWindowPos(h, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
        }
    }

    /// 原生强制隐藏指定句柄窗口（配合 Tauri hide 双重兜底）
    pub fn hide_hwnd(hwnd: isize) {
        if hwnd == 0 {
            return;
        }
        use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
        unsafe {
            let h = HWND(hwnd as *mut _);
            let _ = ShowWindow(h, SW_HIDE);
        }
    }

    /// 原生最小化指定句柄窗口
    pub fn minimize_hwnd(hwnd: isize) {
        if hwnd == 0 {
            return;
        }
        use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_MINIMIZE};
        unsafe {
            let h = HWND(hwnd as *mut _);
            let _ = ShowWindow(h, SW_MINIMIZE);
        }
    }

    /// 通知 Windows Explorer 刷新当前可执行文件及快捷方式的图标缓存（解决覆盖安装后仍显示旧图标缓存问题）
    pub fn refresh_shell_icon_cache() {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "shell32")]
        unsafe extern "system" {
            fn SHChangeNotify(
                wEventId: i32,
                uFlags: u32,
                dwItem1: *const std::ffi::c_void,
                dwItem2: *const std::ffi::c_void,
            );
        }
        const SHCNE_UPDATEITEM: i32 = 0x0000_2000;
        const SHCNE_ASSOCCHANGED: i32 = 0x0800_0000;
        const SHCNF_IDLIST: u32 = 0x0000;
        const SHCNF_PATHW: u32 = 0x0005;
        const SHCNF_FLUSHNOWAIT: u32 = 0x2000;

        let notify_path = |p: &std::path::Path| {
            let wide: Vec<u16> = p.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
            unsafe {
                SHChangeNotify(
                    SHCNE_UPDATEITEM,
                    SHCNF_PATHW | SHCNF_FLUSHNOWAIT,
                    wide.as_ptr() as *const std::ffi::c_void,
                    std::ptr::null(),
                );
            }
        };

        if let Ok(exe) = std::env::current_exe() {
            notify_path(&exe);
        }
        if let Ok(profile) = std::env::var("USERPROFILE") {
            let desktop_lnk = std::path::PathBuf::from(profile).join("Desktop").join("miniBot.lnk");
            if desktop_lnk.exists() {
                notify_path(&desktop_lnk);
            }
        }
        if let Ok(appdata) = std::env::var("APPDATA") {
            let start_lnk = std::path::PathBuf::from(appdata)
                .join(r"Microsoft\Windows\Start Menu\Programs\miniBot.lnk");
            if start_lnk.exists() {
                notify_path(&start_lnk);
            }
        }
        unsafe {
            SHChangeNotify(
                SHCNE_ASSOCCHANGED,
                SHCNF_IDLIST | SHCNF_FLUSHNOWAIT,
                std::ptr::null(),
                std::ptr::null(),
            );
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn process_name(_pid: u32) -> Option<String> {
        None
    }

    pub fn foreground_hwnd() -> isize {
        0
    }

    pub fn foreground_process() -> Option<String> {
        None
    }

    pub fn focus_hwnd(_hwnd: isize) {}

    pub fn cursor_pos() -> (i32, i32) {
        (320, 240)
    }

    pub fn is_cursor_ibeam() -> bool {
        false
    }

    pub fn is_mouse_left_down() -> bool {
        false
    }

    pub fn any_modifier_down() -> bool {
        false
    }

    pub fn point_in_hwnd(_hwnd: isize, _x: i32, _y: i32) -> bool {
        true
    }

    pub fn clipboard_seq() -> u32 {
        0
    }

    pub fn send_ctrl(letter: char) {
        let ch = letter.to_ascii_lowercase();
        let script = format!(
            "tell application \"System Events\" to keystroke \"{ch}\" using command down"
        );
        let _ = std::process::Command::new("osascript")
            .args(["-e", &script])
            .output();
    }

    pub fn send_ctrl_c_clean() {
        send_ctrl('c');
    }

    pub fn make_noactivate(_hwnd: isize) {}

    pub fn show_noactivate(_hwnd: isize) {}

    pub fn hide_hwnd(_hwnd: isize) {}

    pub fn minimize_hwnd(_hwnd: isize) {}

    pub fn refresh_shell_icon_cache() {}
}

pub use imp::*;
