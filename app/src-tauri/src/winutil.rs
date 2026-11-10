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
        GetCursorPos, GetForegroundWindow, GetWindowThreadProcessId,
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
        use windows::Win32::UI::WindowsAndMessaging::{
            BringWindowToTop, SetForegroundWindow, SetWindowPos, HWND_TOPMOST,
            SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
        };
        unsafe {
            let h = HWND(hwnd as *mut _);
            // 确保窗口置顶并成为前台窗口，避免使用 AttachThreadInput 导致非 UI 线程输入队列死锁与光标高频抖动
            let _ = SetWindowPos(h, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
            let _ = BringWindowToTop(h);
            let _ = SetForegroundWindow(h);
        }
    }

    /// Windows 原生极速枚举可见顶层应用窗口矩形（微秒级，避免 xcap::Window::all() 卡顿）
    pub fn fast_window_rects(fx: i32, fy: i32, fw: u32, fh: u32) -> Vec<[i32; 4]> {
        use windows::core::BOOL;
        use windows::Win32::Foundation::{LPARAM, RECT};
        use windows::Win32::UI::WindowsAndMessaging::{
            EnumWindows, GetWindowLongW, GetWindowRect, GetWindowTextLengthW, IsIconic, IsWindowVisible,
            GWL_EXSTYLE, WS_EX_TOOLWINDOW,
        };

        struct EnumCtx {
            fx: i32,
            fy: i32,
            fw: i32,
            fh: i32,
            rects: Vec<[i32; 4]>,
        }

        unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
            unsafe {
                let ctx = &mut *(lparam.0 as *mut EnumCtx);
                if ctx.rects.len() >= 25 {
                    return BOOL(0);
                }
                if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
                    return BOOL(1);
                }
                let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
                if (ex_style & WS_EX_TOOLWINDOW.0) != 0 {
                    return BOOL(1);
                }
                if GetWindowTextLengthW(hwnd) == 0 {
                    return BOOL(1);
                }
                let mut r = RECT::default();
                if GetWindowRect(hwnd, &mut r).is_ok() {
                    let w = r.right - r.left;
                    let h = r.bottom - r.top;
                    if w >= 60 && h >= 60 {
                        let rx0 = r.left - ctx.fx;
                        let ry0 = r.top - ctx.fy;
                        let x0 = rx0.max(0);
                        let y0 = ry0.max(0);
                        let x1 = (rx0 + w).min(ctx.fw);
                        let y1 = (ry0 + h).min(ctx.fh);
                        if x1 - x0 >= 60 && y1 - y0 >= 60 {
                            ctx.rects.push([x0, y0, x1 - x0, y1 - y0]);
                        }
                    }
                }
                BOOL(1)
            }
        }

        let mut ctx = EnumCtx {
            fx,
            fy,
            fw: fw as i32,
            fh: fh as i32,
            rects: Vec::with_capacity(25),
        };

        unsafe {
            let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut _ as isize));
        }
        ctx.rects
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

    /// 设置窗口是否在屏幕捕获/录屏中隐身（WDA_EXCLUDEFROMCAPTURE = 0x11），防止录屏控制条被录进视频
    pub fn set_exclude_from_capture(hwnd: isize, exclude: bool) {
        if hwnd == 0 {
            return;
        }
        use windows::Win32::UI::WindowsAndMessaging::{
            SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
        };
        unsafe {
            let h = HWND(hwnd as *mut _);
            let affinity = if exclude { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
            let _ = SetWindowDisplayAffinity(h, affinity);
        }
    }

    /// 将窗口设置为鼠标事件完全穿透 + 不抢焦点 + 工具窗口 + 置顶（用于录屏选区高亮边框）
    pub fn make_click_through(hwnd: isize) {
        if hwnd == 0 {
            return;
        }
        use windows::Win32::UI::WindowsAndMessaging::{
            GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_LAYERED, WS_EX_NOACTIVATE,
            WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
        };
        unsafe {
            let h = HWND(hwnd as *mut _);
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
            SetWindowLongPtrW(
                h,
                GWL_EXSTYLE,
                ex | (WS_EX_TRANSPARENT.0
                    | WS_EX_LAYERED.0
                    | WS_EX_NOACTIVATE.0
                    | WS_EX_TOOLWINDOW.0
                    | WS_EX_TOPMOST.0) as isize,
            );
        }
    }

    /// 精准定位、调整尺寸并显示鼠标穿透防穿帮窗口（用于录屏选区高亮边框）
    pub fn show_click_through_rect(hwnd: isize, x: i32, y: i32, w: i32, h: i32) {
        if hwnd == 0 {
            return;
        }
        use windows::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, ShowWindow, HWND_TOPMOST, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_SHOWNOACTIVATE,
        };
        make_click_through(hwnd);
        set_exclude_from_capture(hwnd, true);
        unsafe {
            let hw = HWND(hwnd as *mut _);
            let _ = SetWindowPos(
                hw,
                Some(HWND_TOPMOST),
                x,
                y,
                w.max(16),
                h.max(16),
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            let _ = ShowWindow(hw, SW_SHOWNOACTIVATE);
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

    #[allow(dead_code)]
    pub fn check_screen_capture_permission() -> bool {
        true
    }

    #[allow(dead_code)]
    pub fn request_screen_capture_permission() -> bool {
        true
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use core::ffi::{c_char, c_void};

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventCreate(source: *const c_void) -> *mut c_void;
        fn CGEventGetLocation(event: *const c_void) -> CGPoint;
        fn CGEventSourceButtonState(state_id: i32, button: u32) -> bool;
        fn CGEventSourceFlagsState(state_id: i32) -> u64;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: *const c_void);
    }

    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {}

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> *mut c_void;
        fn sel_registerName(name: *const c_char) -> *mut c_void;
        fn objc_msgSend();
        fn objc_autoreleasePoolPush() -> *mut c_void;
        fn objc_autoreleasePoolPop(pool: *mut c_void);
    }

    /// 在非主线程调用 Objective-C / AppKit 方法时必须包裹 AutoreleasePool，防止临时对象泄漏导致内存暴涨或崩溃
    fn with_autorelease_pool<T, F: FnOnce() -> T>(f: F) -> T {
        unsafe {
            let pool = objc_autoreleasePoolPush();
            let res = f();
            objc_autoreleasePoolPop(pool);
            res
        }
    }

    unsafe fn msg_send_id(receiver: *mut c_void, sel: *mut c_void) -> *mut c_void {
        if receiver.is_null() || sel.is_null() {
            return std::ptr::null_mut();
        }
        unsafe {
            let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
                std::mem::transmute(objc_msgSend as *const ());
            f(receiver, sel)
        }
    }

    unsafe fn msg_send_isize(receiver: *mut c_void, sel: *mut c_void) -> isize {
        if receiver.is_null() || sel.is_null() {
            return 0;
        }
        unsafe {
            let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> isize =
                std::mem::transmute(objc_msgSend as *const ());
            f(receiver, sel)
        }
    }

    unsafe fn msg_send_i32(receiver: *mut c_void, sel: *mut c_void) -> i32 {
        if receiver.is_null() || sel.is_null() {
            return 0;
        }
        unsafe {
            let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
                std::mem::transmute(objc_msgSend as *const ());
            f(receiver, sel)
        }
    }

    unsafe fn frontmost_app() -> *mut c_void {
        unsafe {
            let cls = objc_getClass(c"NSWorkspace".as_ptr());
            let ws = msg_send_id(cls, sel_registerName(c"sharedWorkspace".as_ptr()));
            msg_send_id(ws, sel_registerName(c"frontmostApplication".as_ptr()))
        }
    }

    pub fn process_name(pid: u32) -> Option<String> {
        let out = std::process::Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "comm="])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let full = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if full.is_empty() {
            return None;
        }
        full.rsplit('/').next().map(|s| s.to_string())
    }

    /// 在 macOS 上返回当前最前端应用 PID（作为跨线程标识句柄，供后续 focus_hwnd 切回焦点）
    pub fn foreground_hwnd() -> isize {
        with_autorelease_pool(|| unsafe {
            let app = frontmost_app();
            if app.is_null() {
                return 0;
            }
            msg_send_i32(app, sel_registerName(c"processIdentifier".as_ptr())) as isize
        })
    }

    /// 获取当前 macOS 前台应用名称（如 "Safari"、"Google Chrome"、"miniBot"）
    pub fn foreground_process() -> Option<String> {
        with_autorelease_pool(|| unsafe {
            let app = frontmost_app();
            if app.is_null() {
                return None;
            }
            let ns_str = msg_send_id(app, sel_registerName(c"localizedName".as_ptr()));
            if !ns_str.is_null() {
                let c_ptr = msg_send_id(ns_str, sel_registerName(c"UTF8String".as_ptr())) as *const c_char;
                if !c_ptr.is_null() {
                    let s = std::ffi::CStr::from_ptr(c_ptr).to_string_lossy().trim().to_string();
                    if !s.is_empty() {
                        return Some(s);
                    }
                }
            }
            let pid = msg_send_i32(app, sel_registerName(c"processIdentifier".as_ptr()));
            if pid > 0 {
                return process_name(pid as u32);
            }
            None
        })
    }

    /// 激活指定 PID 对应的 macOS 应用窗口（用于「替换原文」切回来源应用粘贴）
    pub fn focus_hwnd(hwnd: isize) {
        if hwnd <= 0 {
            return;
        }
        with_autorelease_pool(|| unsafe {
            let cls = objc_getClass(c"NSRunningApplication".as_ptr());
            let sel_find = sel_registerName(c"runningApplicationWithProcessIdentifier:".as_ptr());
            if cls.is_null() || sel_find.is_null() {
                return;
            }
            let find_fn: unsafe extern "C" fn(*mut c_void, *mut c_void, i32) -> *mut c_void =
                std::mem::transmute(objc_msgSend as *const ());
            let app = find_fn(cls, sel_find, hwnd as i32);
            if app.is_null() {
                return;
            }
            let sel_act = sel_registerName(c"activateWithOptions:".as_ptr());
            let act_fn: unsafe extern "C" fn(*mut c_void, *mut c_void, usize) -> bool =
                std::mem::transmute(objc_msgSend as *const ());
            // NSApplicationActivateIgnoringOtherApps = 1 << 1 (2)
            let _ = act_fn(app, sel_act, 2);
        });
    }

    /// 获取当前鼠标全局屏幕坐标（以主屏幕左上角为 (0,0) 的逻辑点坐标）
    pub fn cursor_pos() -> (i32, i32) {
        with_autorelease_pool(|| unsafe {
            let ev = CGEventCreate(std::ptr::null());
            if ev.is_null() {
                return (320, 240);
            }
            let pt = CGEventGetLocation(ev);
            CFRelease(ev);
            (pt.x.round() as i32, pt.y.round() as i32)
        })
    }

    pub fn is_cursor_ibeam() -> bool {
        false
    }

    /// 鼠标左键当前是否按下（CoreGraphics CombinedSessionState = 0, LeftButton = 0）
    pub fn is_mouse_left_down() -> bool {
        unsafe { CGEventSourceButtonState(0, 0) }
    }

    /// 是否有任何修饰键（Shift / Ctrl / Option / Command）正被按住
    pub fn any_modifier_down() -> bool {
        const MOD_MASK: u64 = 0x0002_0000 | 0x0004_0000 | 0x0008_0000 | 0x0010_0000;
        unsafe { (CGEventSourceFlagsState(0) & MOD_MASK) != 0 }
    }

    pub fn point_in_hwnd(_hwnd: isize, _x: i32, _y: i32) -> bool {
        true
    }

    /// 读取 macOS 系统剪贴板变更序号（NSPasteboard.generalPasteboard.changeCount）
    pub fn clipboard_seq() -> u32 {
        with_autorelease_pool(|| unsafe {
            let cls = objc_getClass(c"NSPasteboard".as_ptr());
            let pb = msg_send_id(cls, sel_registerName(c"generalPasteboard".as_ptr()));
            msg_send_isize(pb, sel_registerName(c"changeCount".as_ptr())) as u32
        })
    }

    pub fn send_ctrl(letter: char) {
        let ch = letter.to_ascii_lowercase();
        // 参考 eSearch darwin 实现：使用物理虚拟键码（c=8, v=9）并延迟 0.08s 避开快捷键修饰键冲突与中文输入法干扰
        let key_cmd = match ch {
            'c' => "key code 8 using command down".to_string(),
            'v' => "key code 9 using command down".to_string(),
            _ => format!("keystroke \"{ch}\" using command down"),
        };
        let _ = std::process::Command::new("osascript")
            .args([
                "-e",
                "tell application \"System Events\"",
                "-e",
                "delay 0.08",
                "-e",
                &key_cmd,
                "-e",
                "end tell",
            ])
            .output();
    }

    pub fn send_ctrl_c_clean() {
        let _ = std::process::Command::new("osascript")
            .args([
                "-e",
                "tell application \"System Events\" to key code 8 using command down",
            ])
            .output();
    }

    pub fn make_noactivate(_hwnd: isize) {}

    pub fn show_noactivate(_hwnd: isize) {}

    pub fn hide_hwnd(_hwnd: isize) {}

    pub fn minimize_hwnd(_hwnd: isize) {}

    pub fn set_exclude_from_capture(ns_win_ptr: isize, exclude: bool) {
        if ns_win_ptr == 0 {
            return;
        }
        with_autorelease_pool(|| unsafe {
            let sel = sel_registerName(c"setSharingType:".as_ptr());
            if sel.is_null() {
                return;
            }
            let f: unsafe extern "C" fn(*mut c_void, *mut c_void, usize) =
                std::mem::transmute(objc_msgSend as *const ());
            // NSWindowSharingNone = 0, NSWindowSharingReadOnly = 1
            let sharing_type: usize = if exclude { 0 } else { 1 };
            f(ns_win_ptr as *mut c_void, sel, sharing_type);
        });
    }

    pub fn make_click_through(_hwnd: isize) {}

    pub fn show_click_through_rect(_hwnd: isize, _x: i32, _y: i32, _w: i32, _h: i32) {}

    pub fn refresh_shell_icon_cache() {}

    pub fn fast_window_rects(_fx: i32, _fy: i32, _fw: u32, _fh: u32) -> Vec<[i32; 4]> {
        vec![]
    }

    extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
    }

    pub fn check_screen_capture_permission() -> bool {
        unsafe { CGPreflightScreenCaptureAccess() }
    }

    pub fn request_screen_capture_permission() -> bool {
        unsafe { CGRequestScreenCaptureAccess() }
    }
}

#[cfg(all(not(windows), not(target_os = "macos")))]
mod imp {
    #[allow(dead_code)]
    pub fn check_screen_capture_permission() -> bool {
        true
    }

    #[allow(dead_code)]
    pub fn request_screen_capture_permission() -> bool {
        true
    }

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

    pub fn fast_window_rects(_fx: i32, _fy: i32, _fw: u32, _fh: u32) -> Vec<[i32; 4]> {
        vec![]
    }

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

    pub fn send_ctrl(_letter: char) {}

    pub fn send_ctrl_c_clean() {}

    pub fn make_noactivate(_hwnd: isize) {}

    pub fn show_noactivate(_hwnd: isize) {}

    pub fn hide_hwnd(_hwnd: isize) {}

    pub fn minimize_hwnd(_hwnd: isize) {}

    pub fn set_exclude_from_capture(_hwnd: isize, _exclude: bool) {}

    pub fn make_click_through(_hwnd: isize) {}

    pub fn show_click_through_rect(_hwnd: isize, _x: i32, _y: i32, _w: i32, _h: i32) {}

    pub fn refresh_shell_icon_cache() {}
}

pub use imp::*;
