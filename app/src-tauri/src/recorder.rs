//! 跨平台极简录屏引擎（移植并精简自 Recordly 双端原生捕获层）
//! - Windows：基于 Windows Graphics Capture (WGC) + Direct3D11 GPU 选区裁剪 + Media Foundation (H.264 + AAC 硬件编码) + WASAPI (系统内录 + 麦克风混音)
//! - macOS：基于 ScreenCaptureKit + AVAssetWriter (H.264 + AAC 硬件编码)
#![allow(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

pub const EVT_RECORDER_STATE: &str = "minibot://recorder-state";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordStartArgs {
    /// 显示器左上角坐标（Windows 物理像素 / macOS 逻辑点）
    pub monitor_x: i32,
    pub monitor_y: i32,
    pub monitor_w: u32,
    pub monitor_h: u32,
    pub scale: f32,
    /// 录制选区相对于显示器左上角的物理像素坐标
    pub crop_x: u32,
    pub crop_y: u32,
    pub crop_w: u32,
    pub crop_h: u32,
    /// 帧率：30 或 60
    pub fps: u32,
    /// 是否录制系统声音（内录）
    pub system_audio: bool,
    /// 是否录制麦克风声音
    pub mic_audio: bool,
    /// 是否在视频中显示鼠标光标
    pub capture_cursor: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecorderStateInfo {
    /// "idle" | "recording" | "paused" | "saving" | "done" | "error"
    pub status: String,
    pub elapsed_ms: u64,
    pub fps: u32,
    pub width: u32,
    pub height: u32,
    pub system_audio: bool,
    pub mic_audio: bool,
    pub output_path: String,
    pub file_size_bytes: u64,
    pub error: Option<String>,
}

impl Default for RecorderStateInfo {
    fn default() -> Self {
        Self {
            status: "idle".into(),
            elapsed_ms: 0,
            fps: 60,
            width: 0,
            height: 0,
            system_audio: true,
            mic_audio: false,
            output_path: String::new(),
            file_size_bytes: 0,
            error: None,
        }
    }
}

struct ActiveSession {
    stop_flag: Arc<AtomicBool>,
    cancel_flag: Arc<AtomicBool>,
    pause_flag: Arc<AtomicBool>,
    elapsed_ms: Arc<AtomicU64>,
    info: RecorderStateInfo,
    #[cfg(target_os = "macos")]
    child_stdin: Option<std::process::ChildStdin>,
}

static SESSION: OnceLock<Mutex<Option<ActiveSession>>> = OnceLock::new();
static LAST_INFO: OnceLock<Mutex<RecorderStateInfo>> = OnceLock::new();

fn session_lock() -> &'static Mutex<Option<ActiveSession>> {
    SESSION.get_or_init(|| Mutex::new(None))
}

fn last_info_lock() -> &'static Mutex<RecorderStateInfo> {
    LAST_INFO.get_or_init(|| Mutex::new(RecorderStateInfo::default()))
}

pub fn emit_state(app: &AppHandle, info: &RecorderStateInfo) {
    *last_info_lock().lock().unwrap() = info.clone();
    if info.status == "done" || info.status == "error" || info.status == "saving" {
        crate::wins::hide(app, "recorder_border");
        if let Some(w) = app.get_webview_window("recorder") {
            let _ = w.set_size(tauri::LogicalSize::new(480.0, 56.0));
        }
    }
    let _ = app.emit(EVT_RECORDER_STATE, info);
}

pub fn get_status() -> RecorderStateInfo {
    if let Some(sess) = session_lock().lock().unwrap().as_ref() {
        let mut info = sess.info.clone();
        info.elapsed_ms = sess.elapsed_ms.load(Ordering::Relaxed);
        info.status = if sess.pause_flag.load(Ordering::Relaxed) {
            "paused".into()
        } else {
            "recording".into()
        };
        return info;
    }
    last_info_lock().lock().unwrap().clone()
}

pub fn default_video_dir(app: &AppHandle) -> PathBuf {
    let st = app.state::<crate::AppState>();
    let custom = st.settings.get().save_dir;
    let base = dirs::video_dir()
        .or_else(dirs::picture_dir)
        .or_else(dirs::desktop_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = if !custom.trim().is_empty() {
        PathBuf::from(custom.trim()).join("Recordings")
    } else {
        base.join("miniBot")
    };
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// 启动双端原生录屏并弹出防穿帮悬浮胶囊条
pub fn start_recording(app: &AppHandle, args: RecordStartArgs) -> Result<RecorderStateInfo, String> {
    {
        let guard = session_lock().lock().unwrap();
        if guard.is_some() {
            return Err("当前已有正在进行的录屏任务".into());
        }
    }

    // 先隐藏选区遮罩窗口，等待一帧让合成器刷新桌面背景
    crate::capture::close(app);

    let fps = if args.fps >= 45 { 60 } else { 30 };
    let out_w = (args.crop_w.max(32) & !1).min(args.monitor_w.max(32) & !1);
    let out_h = (args.crop_h.max(32) & !1).min(args.monitor_h.max(32) & !1);

    let dir = default_video_dir(app);
    let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let output_path = dir.join(format!("miniBot_Rec_{ts}.mp4"));
    let output_path_str = output_path.to_string_lossy().to_string();

    let stop_flag = Arc::new(AtomicBool::new(false));
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let pause_flag = Arc::new(AtomicBool::new(false));
    let elapsed_ms = Arc::new(AtomicU64::new(0));

    let info = RecorderStateInfo {
        status: "recording".into(),
        elapsed_ms: 0,
        fps,
        width: out_w,
        height: out_h,
        system_audio: args.system_audio,
        mic_audio: args.mic_audio,
        output_path: output_path_str.clone(),
        file_size_bytes: 0,
        error: None,
    };

    // 先更新全局状态，确保 HUD 和选区框打开时立即读取到最新的 recording 状态
    emit_state(app, &info);

    #[cfg(windows)]
    {
        let sess = ActiveSession {
            stop_flag: stop_flag.clone(),
            cancel_flag: cancel_flag.clone(),
            pause_flag: pause_flag.clone(),
            elapsed_ms: elapsed_ms.clone(),
            info: info.clone(),
        };
        *session_lock().lock().unwrap() = Some(sess);
        emit_state(app, &info);
        crate::wins::open_recorder_hud(app, &args);

        let app_bg = app.clone();
        let args_bg = args.clone();
        let info_bg = info.clone();
        std::thread::spawn(move || {
            // 留出 120ms 让截图选区窗口彻底消失
            std::thread::sleep(Duration::from_millis(120));
            let res = win_imp::run_windows_capture(
                &app_bg,
                args_bg,
                PathBuf::from(&info_bg.output_path),
                stop_flag.clone(),
                cancel_flag.clone(),
                pause_flag.clone(),
                elapsed_ms.clone(),
            );
            let was_cancelled = cancel_flag.load(Ordering::Relaxed);
            let final_elapsed = elapsed_ms.load(Ordering::Relaxed);
            *session_lock().lock().unwrap() = None;
            crate::wins::hide(&app_bg, "recorder_border");

            if was_cancelled {
                let _ = std::fs::remove_file(&info_bg.output_path);
                let mut idle = RecorderStateInfo::default();
                idle.status = "idle".into();
                emit_state(&app_bg, &idle);
                crate::wins::hide(&app_bg, "recorder");
                return;
            }

            match res {
                Ok(()) => {
                    let size = std::fs::metadata(&info_bg.output_path).map(|m| m.len()).unwrap_or(0);
                    let mut done = info_bg.clone();
                    done.status = "done".into();
                    done.elapsed_ms = final_elapsed;
                    done.file_size_bytes = size;
                    emit_state(&app_bg, &done);
                    crate::log_msg(&app_bg, format!("[recorder] 录制完成：{} ({} bytes)", done.output_path, size));
                }
                Err(err) => {
                    let _ = std::fs::remove_file(&info_bg.output_path);
                    let mut failed = info_bg.clone();
                    failed.status = "error".into();
                    failed.error = Some(err.clone());
                    emit_state(&app_bg, &failed);
                    crate::log_msg(&app_bg, format!("[recorder] 录制失败：{err}"));
                }
            }
        });
    }

    #[cfg(target_os = "macos")]
    {
        mac_imp::spawn_macos_capture(
            app,
            args.clone(),
            info.clone(),
            stop_flag,
            cancel_flag,
            pause_flag,
            elapsed_ms,
        )?;
        crate::wins::open_recorder_hud(app, &args);
    }

    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        let _ = (stop_flag, cancel_flag, pause_flag, elapsed_ms);
        return Err("当前操作系统暂不支持原生录屏".into());
    }

    Ok(info)
}

pub fn pause_recording(app: &AppHandle) -> Result<RecorderStateInfo, String> {
    let mut guard = session_lock().lock().unwrap();
    let Some(sess) = guard.as_mut() else {
        return Err("没有正在进行的录制".into());
    };
    sess.pause_flag.store(true, Ordering::SeqCst);
    #[cfg(target_os = "macos")]
    if let Some(stdin) = sess.child_stdin.as_mut() {
        use std::io::Write;
        let _ = writeln!(stdin, "pause");
        let _ = stdin.flush();
    }
    let mut info = sess.info.clone();
    info.status = "paused".into();
    info.elapsed_ms = sess.elapsed_ms.load(Ordering::Relaxed);
    drop(guard);
    emit_state(app, &info);
    Ok(info)
}

pub fn resume_recording(app: &AppHandle) -> Result<RecorderStateInfo, String> {
    let mut guard = session_lock().lock().unwrap();
    let Some(sess) = guard.as_mut() else {
        return Err("没有正在进行的录制".into());
    };
    sess.pause_flag.store(false, Ordering::SeqCst);
    #[cfg(target_os = "macos")]
    if let Some(stdin) = sess.child_stdin.as_mut() {
        use std::io::Write;
        let _ = writeln!(stdin, "resume");
        let _ = stdin.flush();
    }
    let mut info = sess.info.clone();
    info.status = "recording".into();
    info.elapsed_ms = sess.elapsed_ms.load(Ordering::Relaxed);
    drop(guard);
    emit_state(app, &info);
    Ok(info)
}

pub fn stop_recording(app: &AppHandle) -> Result<RecorderStateInfo, String> {
    let mut guard = session_lock().lock().unwrap();
    let Some(sess) = guard.as_mut() else {
        return Ok(last_info_lock().lock().unwrap().clone());
    };
    sess.stop_flag.store(true, Ordering::SeqCst);
    #[cfg(target_os = "macos")]
    if let Some(mut stdin) = sess.child_stdin.take() {
        use std::io::Write;
        let _ = writeln!(stdin, "stop");
        let _ = stdin.flush();
    }
    let mut info = sess.info.clone();
    info.status = "saving".into();
    info.elapsed_ms = sess.elapsed_ms.load(Ordering::Relaxed);
    drop(guard);
    crate::wins::hide(app, "recorder_border");
    emit_state(app, &info);
    Ok(info)
}

pub fn cancel_recording(app: &AppHandle) -> Result<(), String> {
    let mut guard = session_lock().lock().unwrap();
    if let Some(sess) = guard.as_mut() {
        sess.cancel_flag.store(true, Ordering::SeqCst);
        sess.stop_flag.store(true, Ordering::SeqCst);
        #[cfg(target_os = "macos")]
        if let Some(mut stdin) = sess.child_stdin.take() {
            use std::io::Write;
            let _ = writeln!(stdin, "stop");
            let _ = stdin.flush();
        }
    }
    drop(guard);
    crate::wins::hide(app, "recorder_border");
    crate::wins::hide(app, "recorder");
    Ok(())
}

/// 在资源管理器 / Finder 中定位生成的视频文件
pub fn reveal_video_in_folder(path: &str) -> Result<(), String> {
    let p = PathBuf::from(path);
    if !p.exists() {
        return Err("视频文件不存在".into());
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer.exe")
            .arg(format!("/select,{}", p.to_string_lossy()))
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .args(["-R", &p.to_string_lossy()])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 将录制的 MP4 文件作为系统文件对象复制到剪贴板（支持直接粘贴到微信/飞书/资源管理器）
pub fn copy_video_file_to_clipboard(app: &AppHandle, path: &str) -> Result<(), String> {
    let p = PathBuf::from(path);
    if !p.exists() {
        return Err("视频文件不存在".into());
    }
    let st = app.state::<crate::AppState>();
    st.clip.suppress(1500);

    #[cfg(windows)]
    {
        win_imp::set_clipboard_file_drop(&p)?;
        st.clip.mark_self_write();
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "set the clipboard to (POSIX file {:?})",
            p.to_string_lossy().to_string()
        );
        let status = std::process::Command::new("osascript")
            .args(["-e", &script])
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("写入 macOS 剪贴板失败".into());
        }
        st.clip.mark_self_write();
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err("不支持的平台".into())
}

// ============================================================================
// Windows 原生录屏实现：WGC + Direct3D11 + Media Foundation (H.264/AAC) + WASAPI
// ============================================================================
#[cfg(windows)]
mod win_imp {
    use super::RecordStartArgs;
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use tauri::AppHandle;

    use windows::core::{Interface, HSTRING, PCWSTR};
    use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
    use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
    use windows::Graphics::DirectX::DirectXPixelFormat;
    use windows::Win32::Foundation::{HANDLE, HWND, POINT, RECT};
    use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1};
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_BOX,
        D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE,
        D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    };
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
    use windows::Win32::Graphics::Dxgi::IDXGIDevice;
    use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MonitorFromRect, HMONITOR, MONITOR_DEFAULTTOPRIMARY, MONITOR_DEFAULTTONULL};
    use windows::Win32::Media::Audio::{
        eCapture, eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator,
        MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
        AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_LOOPBACK,
        AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, WAVEFORMATEX, WAVE_FORMAT_PCM,
    };
    use windows::Win32::Media::MediaFoundation::{
        IMFAttributes, IMFMediaBuffer, IMFMediaType, IMFSample, IMFSinkWriter,
        MFCreateAttributes, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample,
        MFCreateSinkWriterFromURL, MFShutdown, MFStartup, MFAudioFormat_AAC, MFAudioFormat_PCM,
        MFMediaType_Audio, MFMediaType_Video, MFNominalRange_16_235, MFVideoFormat_H264,
        MFVideoFormat_NV12, MFVideoInterlace_Progressive, MFVideoPrimaries_BT709,
        MFVideoTransFunc_709, MFVideoTransferMatrix_BT709, MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
        MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_BLOCK_ALIGNMENT, MF_MT_AUDIO_NUM_CHANNELS,
        MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
        MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE,
        MF_MT_TRANSFER_FUNCTION, MF_MT_VIDEO_NOMINAL_RANGE, MF_MT_VIDEO_PRIMARIES,
        MF_MT_YUV_MATRIX, MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_VERSION, MFSTARTUP_FULL,
    };
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED};
    use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
    use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
    use windows::Win32::System::WinRT::{RoGetActivationFactory, RoInitialize, RO_INIT_MULTITHREADED};

    /// 将文件路径写入 Windows 剪贴板 CF_HDROP (15) 格式
    pub fn set_clipboard_file_drop(path: &Path) -> Result<(), String> {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GlobalAlloc(uFlags: u32, dwBytes: usize) -> *mut core::ffi::c_void;
            fn GlobalLock(hMem: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
            fn GlobalUnlock(hMem: *mut core::ffi::c_void) -> i32;
        }
        const GMEM_MOVEABLE: u32 = 0x0002;
        #[repr(C)]
        struct DropFiles {
            p_files: u32,
            pt_x: i32,
            pt_y: i32,
            f_nc: i32,
            f_wide: i32,
        }
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .chain(std::iter::once(0))
            .collect();
        let header_size = std::mem::size_of::<DropFiles>();
        let total_bytes = header_size + wide.len() * 2;
        unsafe {
            let hglobal = GlobalAlloc(GMEM_MOVEABLE, total_bytes);
            if hglobal.is_null() {
                return Err("GlobalAlloc 失败".into());
            }
            let ptr = GlobalLock(hglobal);
            if ptr.is_null() {
                return Err("GlobalLock 失败".into());
            }
            let df = ptr as *mut DropFiles;
            (*df).p_files = header_size as u32;
            (*df).pt_x = 0;
            (*df).pt_y = 0;
            (*df).f_nc = 0;
            (*df).f_wide = 1;
            let dst = (ptr as *mut u8).add(header_size) as *mut u16;
            std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
            let _ = GlobalUnlock(hglobal);

            OpenClipboard(Some(HWND::default())).map_err(|e| e.to_string())?;
            let _ = EmptyClipboard();
            let res = SetClipboardData(15u32, Some(HANDLE(hglobal)));
            let _ = CloseClipboard();
            res.map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn pack_u64(hi: u32, lo: u32) -> u64 {
        ((hi as u64) << 32) | (lo as u64)
    }

    fn calc_bitrate(w: u32, h: u32, fps: u32) -> u32 {
        let pixels = (w as u64) * (h as u64);
        let base: u32 = if pixels >= 3840 * 2160 {
            36_000_000
        } else if pixels >= 2560 * 1440 {
            22_000_000
        } else if pixels >= 1920 * 1080 {
            14_000_000
        } else {
            9_000_000
        };
        if fps >= 60 {
            ((base as f64) * 1.3) as u32
        } else {
            base
        }
    }

    /// 移植自 Recordly bt709_video.h：BGRA 转 BT.709 Limited Range NV12
    fn convert_bgra_to_bt709_nv12(
        bgra: &[u8],
        pitch: usize,
        width: usize,
        height: usize,
        nv12: &mut [u8],
    ) {
        let y_plane_len = width * height;
        let (y_plane, uv_plane) = nv12.split_at_mut(y_plane_len);

        for y in 0..height {
            let src_row = &bgra[y * pitch..y * pitch + width * 4];
            let dst_row = &mut y_plane[y * width..(y + 1) * width];
            for x in 0..width {
                let b = src_row[x * 4] as i32;
                let g = src_row[x * 4 + 1] as i32;
                let r = src_row[x * 4 + 2] as i32;
                let luma = ((47 * r + 157 * g + 16 * b + 128) >> 8) + 16;
                dst_row[x] = luma.clamp(16, 235) as u8;
            }
        }

        let half_h = height / 2;
        let half_w = width / 2;
        for cy in 0..half_h {
            let y0 = cy * 2;
            let y1 = y0 + 1;
            let row0 = &bgra[y0 * pitch..y0 * pitch + width * 4];
            let row1 = &bgra[y1 * pitch..y1 * pitch + width * 4];
            let uv_row = &mut uv_plane[cy * width..(cy + 1) * width];

            for cx in 0..half_w {
                let x0 = cx * 8;
                let b = (row0[x0] as i32 + row0[x0 + 4] as i32 + row1[x0] as i32 + row1[x0 + 4] as i32 + 2) >> 2;
                let g = (row0[x0 + 1] as i32 + row0[x0 + 5] as i32 + row1[x0 + 1] as i32 + row1[x0 + 5] as i32 + 2) >> 2;
                let r = (row0[x0 + 2] as i32 + row0[x0 + 6] as i32 + row1[x0 + 2] as i32 + row1[x0 + 6] as i32 + 2) >> 2;

                let cb = ((-26 * r - 87 * g + 113 * b + 128) >> 8) + 128;
                let cr = ((112 * r - 102 * g - 10 * b + 128) >> 8) + 128;
                uv_row[cx * 2] = cb.clamp(16, 240) as u8;
                uv_row[cx * 2 + 1] = cr.clamp(16, 240) as u8;
            }
        }
    }

    /// 启动 WASAPI 音频采集线程（支持系统声音 Loopback 或麦克风，由 Windows 内核自动重采样为 48kHz Stereo 16-bit PCM）
    fn spawn_wasapi_capture(
        is_loopback: bool,
        ring: Arc<Mutex<VecDeque<i16>>>,
        stop_flag: Arc<AtomicBool>,
        pause_flag: Arc<AtomicBool>,
    ) {
        std::thread::spawn(move || unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let run = || -> windows::core::Result<()> {
                let enumerator: IMMDeviceEnumerator =
                    CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
                let flow = if is_loopback { eRender } else { eCapture };
                let device = enumerator.GetDefaultAudioEndpoint(flow, eConsole)?;
                let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;

                let target_fmt = WAVEFORMATEX {
                    wFormatTag: WAVE_FORMAT_PCM as u16,
                    nChannels: 2,
                    nSamplesPerSec: 48000,
                    nAvgBytesPerSec: 48000 * 4,
                    nBlockAlign: 4,
                    wBitsPerSample: 16,
                    cbSize: 0,
                };
                let mut flags = AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
                if is_loopback {
                    flags |= AUDCLNT_STREAMFLAGS_LOOPBACK;
                }
                client.Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    flags,
                    1_000_000, // 100ms 缓冲
                    0,
                    &target_fmt,
                    None,
                )?;

                let capture: IAudioCaptureClient = client.GetService()?;
                client.Start()?;

                while !stop_flag.load(Ordering::Relaxed) {
                    let mut packet_len = capture.GetNextPacketSize()?;
                    if packet_len == 0 {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    while packet_len > 0 && !stop_flag.load(Ordering::Relaxed) {
                        let mut data_ptr: *mut u8 = std::ptr::null_mut();
                        let mut frames_read: u32 = 0;
                        let mut buf_flags: u32 = 0;
                        capture.GetBuffer(
                            &mut data_ptr,
                            &mut frames_read,
                            &mut buf_flags,
                            None,
                            None,
                        )?;

                        if !pause_flag.load(Ordering::Relaxed) && frames_read > 0 {
                            let sample_count = (frames_read as usize) * 2;
                            let mut q = ring.lock().unwrap();
                            // 限制最大缓冲 1.5 秒，防止异常积压
                            if q.len() < 48000 * 3 {
                                if (buf_flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)) != 0 || data_ptr.is_null() {
                                    q.extend(std::iter::repeat(0i16).take(sample_count));
                                } else {
                                    let slice = std::slice::from_raw_parts(data_ptr as *const i16, sample_count);
                                    q.extend(slice.iter().copied());
                                }
                            }
                        }

                        capture.ReleaseBuffer(frames_read)?;
                        packet_len = capture.GetNextPacketSize()?;
                    }
                }

                let _ = client.Stop();
                Ok(())
            };
            let _ = run();
            CoUninitialize();
        });
    }

    fn write_mf_sample(
        writer: &IMFSinkWriter,
        stream_idx: u32,
        bytes: &[u8],
        pts_hns: i64,
        dur_hns: i64,
    ) -> Result<(), String> {
        unsafe {
            let buf: IMFMediaBuffer =
                MFCreateMemoryBuffer(bytes.len() as u32).map_err(|e| format!("MFCreateMemoryBuffer: {e}"))?;
            let mut ptr: *mut u8 = std::ptr::null_mut();
            buf.Lock(&mut ptr, None, None).map_err(|e| format!("Buffer Lock: {e}"))?;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
            let _ = buf.Unlock();
            buf.SetCurrentLength(bytes.len() as u32).map_err(|e| e.to_string())?;

            let sample: IMFSample = MFCreateSample().map_err(|e| format!("MFCreateSample: {e}"))?;
            sample.AddBuffer(&buf).map_err(|e| e.to_string())?;
            sample.SetSampleTime(pts_hns).map_err(|e| e.to_string())?;
            sample.SetSampleDuration(dur_hns).map_err(|e| e.to_string())?;
            writer.WriteSample(stream_idx, &sample).map_err(|e| format!("WriteSample: {e}"))?;
            Ok(())
        }
    }

    pub fn run_windows_capture(
        app: &AppHandle,
        args: RecordStartArgs,
        output_path: PathBuf,
        stop_flag: Arc<AtomicBool>,
        cancel_flag: Arc<AtomicBool>,
        pause_flag: Arc<AtomicBool>,
        elapsed_ms: Arc<AtomicU64>,
    ) -> Result<(), String> {
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(|e| format!("MFStartup 失败: {e}"))?;
        }

        let res = (|| unsafe {
            // 1. 定位目标显示器 HMONITOR
            let mon_rect = RECT {
                left: args.monitor_x,
                top: args.monitor_y,
                right: args.monitor_x + args.monitor_w as i32,
                bottom: args.monitor_y + args.monitor_h as i32,
            };
            let mut hmon: HMONITOR = MonitorFromRect(&mon_rect, MONITOR_DEFAULTTONULL);
            if hmon.is_invalid() {
                hmon = MonitorFromPoint(
                    POINT {
                        x: args.monitor_x + 16,
                        y: args.monitor_y + 16,
                    },
                    MONITOR_DEFAULTTOPRIMARY,
                );
            }

            // 2. 创建 Direct3D11 硬件设备与 WinRT GraphicsCaptureItem
            let feature_levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
            let mut d3d_device: Option<ID3D11Device> = None;
            let mut d3d_ctx: Option<ID3D11DeviceContext> = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                 Default::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&feature_levels),
                D3D11_SDK_VERSION,
                Some(&mut d3d_device),
                None,
                Some(&mut d3d_ctx),
            )
            .map_err(|e| format!("创建 D3D11 设备失败: {e}"))?;
            let d3d_device = d3d_device.ok_or("D3D11Device 为空")?;
            let d3d_ctx = d3d_ctx.ok_or("D3D11DeviceContext 为空")?;

            let dxgi_device: IDXGIDevice = d3d_device.cast().map_err(|e| format!("Cast IDXGIDevice: {e}"))?;
            let inspectable = CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device)
                .map_err(|e| format!("CreateDirect3D11DeviceFromDXGIDevice: {e}"))?;
            let winrt_device: IDirect3DDevice = inspectable.cast().map_err(|e| format!("Cast IDirect3DDevice: {e}"))?;

            let interop: IGraphicsCaptureItemInterop =
                RoGetActivationFactory(&HSTRING::from("Windows.Graphics.Capture.GraphicsCaptureItem"))
                    .map_err(|e| format!("获取 WGC Interop 失败: {e}"))?;
            let item: GraphicsCaptureItem = interop
                .CreateForMonitor(hmon)
                .map_err(|e| format!("CreateForMonitor 失败: {e}"))?;

            let item_size = item.Size().map_err(|e| format!("获取屏幕尺寸失败: {e}"))?;
            let mon_w = (item_size.Width.max(32) as u32) & !1;
            let mon_h = (item_size.Height.max(32) as u32) & !1;

            let cw = (args.crop_w.max(32) & !1).min(mon_w);
            let ch = (args.crop_h.max(32) & !1).min(mon_h);
            let cx = args.crop_x.min(mon_w.saturating_sub(cw));
            let cy = args.crop_y.min(mon_h.saturating_sub(ch));

            // 3. 创建 Staging Texture（仅分配裁剪区域大小，GPU 直接 CopySubresourceRegion 裁剪）
            let staging_desc = D3D11_TEXTURE2D_DESC {
                Width: cw,
                Height: ch,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            let mut staging_tex: Option<ID3D11Texture2D> = None;
            d3d_device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging_tex))
                .map_err(|e| format!("创建 Staging 纹理失败: {e}"))?;
            let staging_tex = staging_tex.ok_or("Staging 纹理为空")?;

            // 4. 初始化 Media Foundation SinkWriter（H.264 视频轨 + 可选 AAC 音频轨）
            let fps = if args.fps >= 45 { 60 } else { 30 };
            let mut attrs: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut attrs, 2).map_err(|e| e.to_string())?;
            let attrs = attrs.unwrap();
            let _ = attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1);

            let out_wstr = HSTRING::from(output_path.as_os_str());
            let writer: IMFSinkWriter =
                MFCreateSinkWriterFromURL(PCWSTR(out_wstr.as_ptr()), None, &attrs)
                    .map_err(|e| format!("创建 MP4 编码器失败: {e}"))?;

            // 视频输出流 (H.264)
            let v_out: IMFMediaType = MFCreateMediaType().map_err(|e| e.to_string())?;
            v_out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(|e| e.to_string())?;
            v_out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264).map_err(|e| e.to_string())?;
            v_out.SetUINT32(&MF_MT_AVG_BITRATE, calc_bitrate(cw, ch, fps)).map_err(|e| e.to_string())?;
            v_out.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32).map_err(|e| e.to_string())?;
            v_out.SetUINT64(&MF_MT_FRAME_SIZE, pack_u64(cw, ch)).map_err(|e| e.to_string())?;
            v_out.SetUINT64(&MF_MT_FRAME_RATE, pack_u64(fps, 1)).map_err(|e| e.to_string())?;
            v_out.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack_u64(1, 1)).map_err(|e| e.to_string())?;
            let _ = v_out.SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32);
            let _ = v_out.SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32);
            let _ = v_out.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32);
            let _ = v_out.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32);

            let video_stream_idx = writer.AddStream(&v_out).map_err(|e| format!("AddStream(Video): {e}"))?;

            // 视频输入流 (NV12)
            let v_in: IMFMediaType = MFCreateMediaType().map_err(|e| e.to_string())?;
            v_in.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(|e| e.to_string())?;
            v_in.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12).map_err(|e| e.to_string())?;
            v_in.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32).map_err(|e| e.to_string())?;
            v_in.SetUINT64(&MF_MT_FRAME_SIZE, pack_u64(cw, ch)).map_err(|e| e.to_string())?;
            v_in.SetUINT64(&MF_MT_FRAME_RATE, pack_u64(fps, 1)).map_err(|e| e.to_string())?;
            v_in.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack_u64(1, 1)).map_err(|e| e.to_string())?;
            let _ = v_in.SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32);
            let _ = v_in.SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32);
            let _ = v_in.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32);
            let _ = v_in.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32);

            writer
                .SetInputMediaType(video_stream_idx, &v_in, None)
                .map_err(|e| format!("SetInputMediaType(Video): {e}"))?;

            // 音频流（若开启系统内录或麦克风，添加 48kHz Stereo AAC 音轨）
            let has_audio = args.system_audio || args.mic_audio;
            let mut audio_stream_idx: Option<u32> = None;
            if has_audio {
                let a_out: IMFMediaType = MFCreateMediaType().map_err(|e| e.to_string())?;
                let _ = a_out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio);
                let _ = a_out.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC);
                let _ = a_out.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, 48000);
                let _ = a_out.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2);
                let _ = a_out.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16);
                let _ = a_out.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 24000); // 192 kbps

                if let Ok(a_idx) = writer.AddStream(&a_out) {
                    let a_in: IMFMediaType = MFCreateMediaType().map_err(|e| e.to_string())?;
                    let _ = a_in.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio);
                    let _ = a_in.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM);
                    let _ = a_in.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, 48000);
                    let _ = a_in.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2);
                    let _ = a_in.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16);
                    let _ = a_in.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 4);
                    let _ = a_in.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 48000 * 4);
                    if writer.SetInputMediaType(a_idx, &a_in, None).is_ok() {
                        audio_stream_idx = Some(a_idx);
                    }
                }
            }

            writer.BeginWriting().map_err(|e| format!("BeginWriting 失败: {e}"))?;

            // 5. 启动 WGC 捕获会话
            let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                &winrt_device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
                item_size,
            )
            .map_err(|e| format!("创建 FramePool 失败: {e}"))?;

            let session = frame_pool
                .CreateCaptureSession(&item)
                .map_err(|e| format!("创建 CaptureSession 失败: {e}"))?;
            let _ = session.SetIsCursorCaptureEnabled(args.capture_cursor);
            let _ = session.SetIsBorderRequired(false);
            session.StartCapture().map_err(|e| format!("启动 WGC 捕获失败: {e}"))?;

            // 6. 启动 WASAPI 音频采集线程
            let sys_ring: Arc<Mutex<VecDeque<i16>>> = Arc::new(Mutex::new(VecDeque::with_capacity(48000 * 2)));
            let mic_ring: Arc<Mutex<VecDeque<i16>>> = Arc::new(Mutex::new(VecDeque::with_capacity(48000 * 2)));
            if audio_stream_idx.is_some() {
                if args.system_audio {
                    spawn_wasapi_capture(true, sys_ring.clone(), stop_flag.clone(), pause_flag.clone());
                }
                if args.mic_audio {
                    spawn_wasapi_capture(false, mic_ring.clone(), stop_flag.clone(), pause_flag.clone());
                }
            }

            // 7. 实时编码主循环（视频 + 音频按统一时间轴交错写入 SinkWriter）
            let frame_dur_hns: i64 = 10_000_000 / (fps as i64);
            let step_duration = Duration::from_nanos((1_000_000_000 / fps) as u64);
            let mut nv12_buf = vec![16u8; (cw as usize) * (ch as usize) * 3 / 2];
            // 初始化 UV 平面为中性灰 128
            for b in &mut nv12_buf[(cw as usize) * (ch as usize)..] {
                *b = 128;
            }
            let mut has_first_frame = false;
            let mut start_instant = Instant::now();
            let mut pause_started: Option<Instant> = None;
            let mut total_paused = Duration::ZERO;
            let mut next_video_pts_hns: i64 = 0;
            let mut audio_frames_written: u64 = 0;
            let mut mix_pcm_bytes = vec![0u8; 960 * 4]; // 20ms @ 48kHz Stereo i16

            crate::log_msg(app, format!("[recorder] Windows WGC 录制已启动：{}x{} @ {}fps", cw, ch, fps));

            while !stop_flag.load(Ordering::Relaxed) {
                if pause_flag.load(Ordering::Relaxed) {
                    if pause_started.is_none() {
                        pause_started = Some(Instant::now());
                    }
                    // 暂停期间丢弃缓冲帧
                    while let Ok(f) = frame_pool.TryGetNextFrame() {
                        let _ = f.Close();
                    }
                    sys_ring.lock().unwrap().clear();
                    mic_ring.lock().unwrap().clear();
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                } else if let Some(ps) = pause_started.take() {
                    total_paused += ps.elapsed();
                }

                // 拉取最新可用帧（若有积压则取最新一帧）
                let mut latest_frame = None;
                while let Ok(f) = frame_pool.TryGetNextFrame() {
                    if let Some(old) = latest_frame.replace(f) {
                        let _ = old.Close();
                    }
                }

                if let Some(frame) = latest_frame {
                    if let Ok(surface) = frame.Surface() {
                        if let Ok(access) = surface.cast::<IDirect3DDxgiInterfaceAccess>() {
                            if let Ok(src_tex) = access.GetInterface::<ID3D11Texture2D>() {
                                let src_box = D3D11_BOX {
                                    left: cx,
                                    top: cy,
                                    front: 0,
                                    right: cx + cw,
                                    bottom: cy + ch,
                                    back: 1,
                                };
                                d3d_ctx.CopySubresourceRegion(
                                    &staging_tex,
                                    0,
                                    0,
                                    0,
                                    0,
                                    &src_tex,
                                    0,
                                    Some(&src_box),
                                );
                                let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                                if d3d_ctx.Map(&staging_tex, 0, D3D11_MAP_READ, 0, Some(&mut mapped)).is_ok() {
                                    let total_bytes = (mapped.RowPitch as usize) * (ch as usize);
                                    let bgra = std::slice::from_raw_parts(mapped.pData as *const u8, total_bytes);
                                    convert_bgra_to_bt709_nv12(
                                        bgra,
                                        mapped.RowPitch as usize,
                                        cw as usize,
                                        ch as usize,
                                        &mut nv12_buf,
                                    );
                                    d3d_ctx.Unmap(&staging_tex, 0);
                                    if !has_first_frame {
                                        has_first_frame = true;
                                        start_instant = Instant::now();
                                        total_paused = Duration::ZERO;
                                        sys_ring.lock().unwrap().clear();
                                        mic_ring.lock().unwrap().clear();
                                    }
                                }
                            }
                        }
                    }
                    let _ = frame.Close();
                }

                if !has_first_frame {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }

                let active_dur = start_instant.elapsed().saturating_sub(total_paused);
                elapsed_ms.store(active_dur.as_millis() as u64, Ordering::Relaxed);
                let target_pts_hns = (active_dur.as_nanos() / 100) as i64;

                // 写入视频帧（即便屏幕画面静止未产生新 WGC 帧，也复用上一帧填补时间轴）
                while next_video_pts_hns <= target_pts_hns {
                    let _ = write_mf_sample(
                        &writer,
                        video_stream_idx,
                        &nv12_buf,
                        next_video_pts_hns,
                        frame_dur_hns,
                    );
                    next_video_pts_hns += frame_dur_hns;
                }

                // 写入混音后的 20ms 音频块 (960 frames @ 48kHz Stereo)
                if let Some(a_idx) = audio_stream_idx {
                    let target_audio_frames = ((target_pts_hns.max(0) as u64) * 48000) / 10_000_000;
                    while audio_frames_written + 960 <= target_audio_frames {
                        {
                            let mut sq = sys_ring.lock().unwrap();
                            let mut mq = mic_ring.lock().unwrap();
                            for i in 0..(960 * 2) {
                                let s = sq.pop_front().unwrap_or(0) as i32;
                                let m = mq.pop_front().unwrap_or(0) as i32;
                                let mixed = (s + m).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                                let le = mixed.to_le_bytes();
                                mix_pcm_bytes[i * 2] = le[0];
                                mix_pcm_bytes[i * 2 + 1] = le[1];
                            }
                        }
                        let a_pts_hns = ((audio_frames_written * 10_000_000) / 48000) as i64;
                        let _ = write_mf_sample(&writer, a_idx, &mix_pcm_bytes, a_pts_hns, 200_000);
                        audio_frames_written += 960;
                    }
                }

                std::thread::sleep(step_duration / 2);
            }

            let _ = session.Close();
            let _ = frame_pool.Close();

            if !cancel_flag.load(Ordering::Relaxed) && has_first_frame {
                writer.Finalize().map_err(|e| format!("Finalize MP4 失败: {e}"))?;
            }
            Ok(())
        })();

        unsafe {
            let _ = MFShutdown();
            CoUninitialize();
        }
        res
    }
}

// ============================================================================
// macOS 原生录屏实现：ScreenCaptureKit + AVAssetWriter (H.264/AAC)
// ============================================================================
#[cfg(target_os = "macos")]
mod mac_imp {
    use super::{emit_state, session_lock, ActiveSession, RecordStartArgs, RecorderStateInfo};
    use std::io::{BufRead, BufReader};
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tauri::{AppHandle, Manager};

    const SWIFT_SRC: &str = include_str!("recorder_macos.swift");

    #[cfg(has_prebuilt_mac_recorder)]
    const PREBUILT_BIN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/minibot-mac-recorder"));

    fn ensure_mac_helper(app: &AppHandle) -> Result<PathBuf, String> {
        let dir = app
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| std::env::temp_dir())
            .join("bin");
        let _ = std::fs::create_dir_all(&dir);
        let bin_path = dir.join("minibot-mac-recorder-v1");

        #[cfg(has_prebuilt_mac_recorder)]
        if !PREBUILT_BIN.is_empty() && !bin_path.exists() {
            std::fs::write(&bin_path, PREBUILT_BIN).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&bin_path, std::fs::Permissions::from_mode(0o755));
            }
        }

        if bin_path.exists() {
            return Ok(bin_path);
        }

        // 若未嵌入预编译产物，使用系统自带 swiftc 即时编译并缓存
        let src_path = dir.join("recorder_macos.swift");
        std::fs::write(&src_path, SWIFT_SRC).map_err(|e| e.to_string())?;
        let out = Command::new("/usr/bin/swiftc")
            .args([
                "-O",
                &src_path.to_string_lossy(),
                "-framework",
                "ScreenCaptureKit",
                "-framework",
                "AVFoundation",
                "-framework",
                "CoreGraphics",
                "-framework",
                "CoreImage",
                "-framework",
                "CoreMedia",
                "-framework",
                "Foundation",
                "-o",
                &bin_path.to_string_lossy(),
            ])
            .output()
            .map_err(|e| format!("调用 swiftc 编译录屏组件失败: {e}"))?;

        if !out.status.success() {
            return Err(format!(
                "编译 macOS ScreenCaptureKit 组件失败: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        Ok(bin_path)
    }

    pub fn spawn_macos_capture(
        app: &AppHandle,
        args: RecordStartArgs,
        info: RecorderStateInfo,
        stop_flag: Arc<AtomicBool>,
        cancel_flag: Arc<AtomicBool>,
        pause_flag: Arc<AtomicBool>,
        elapsed_ms: Arc<AtomicU64>,
    ) -> Result<(), String> {
        let helper = ensure_mac_helper(app)?;
        let s = args.scale.max(1.0);
        let region_x = (args.monitor_x as f32) + (args.crop_x as f32) / s;
        let region_y = (args.monitor_y as f32) + (args.crop_y as f32) / s;
        let region_w = (args.crop_w as f32) / s;
        let region_h = (args.crop_h as f32) / s;

        let cfg_json = serde_json::json!({
            "fps": info.fps,
            "displayX": args.monitor_x,
            "displayY": args.monitor_y,
            "regionX": region_x,
            "regionY": region_y,
            "regionWidth": region_w,
            "regionHeight": region_h,
            "outputPath": info.output_path,
            "capturesSystemAudio": args.system_audio,
            "capturesMicrophone": args.mic_audio,
            "capturesCursor": args.capture_cursor,
            "excludedProcessIds": []
        })
        .to_string();

        let mut child = Command::new(&helper)
            .arg(&cfg_json)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("启动 macOS 录屏进程失败: {e}"))?;

        let stdin = child.stdin.take();
        let stdout = child.stdout.take();

        let sess = ActiveSession {
            stop_flag: stop_flag.clone(),
            cancel_flag: cancel_flag.clone(),
            pause_flag: pause_flag.clone(),
            elapsed_ms: elapsed_ms.clone(),
            info: info.clone(),
            child_stdin: stdin,
        };
        *session_lock().lock().unwrap() = Some(sess);
        emit_state(app, &info);

        // 计时线程
        let stop_t = stop_flag.clone();
        let pause_t = pause_flag.clone();
        let elapsed_t = elapsed_ms.clone();
        std::thread::spawn(move || {
            let start = Instant::now();
            let mut pause_start: Option<Instant> = None;
            let mut total_paused = Duration::ZERO;
            while !stop_t.load(Ordering::Relaxed) {
                if pause_t.load(Ordering::Relaxed) {
                    if pause_start.is_none() {
                        pause_start = Some(Instant::now());
                    }
                } else if let Some(ps) = pause_start.take() {
                    total_paused += ps.elapsed();
                }
                if !pause_t.load(Ordering::Relaxed) {
                    let d = start.elapsed().saturating_sub(total_paused);
                    elapsed_t.store(d.as_millis() as u64, Ordering::Relaxed);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });

        // 监控进程结束
        let app_bg = app.clone();
        let info_bg = info.clone();
        std::thread::spawn(move || {
            if let Some(out) = stdout {
                let reader = BufReader::new(out);
                for line in reader.lines().map_while(Result::ok) {
                    crate::log_msg(&app_bg, format!("[mac-recorder] {line}"));
                }
            }
            let status = child.wait();
            stop_flag.store(true, Ordering::SeqCst);
            let was_cancelled = cancel_flag.load(Ordering::Relaxed);
            let final_elapsed = elapsed_ms.load(Ordering::Relaxed);
            *session_lock().lock().unwrap() = None;
            crate::wins::hide(&app_bg, "recorder_border");

            if was_cancelled {
                let _ = std::fs::remove_file(&info_bg.output_path);
                let mut idle = RecorderStateInfo::default();
                idle.status = "idle".into();
                emit_state(&app_bg, &idle);
                crate::wins::hide(&app_bg, "recorder");
                return;
            }

            match status {
                Ok(s) if s.success() && PathBuf::from(&info_bg.output_path).exists() => {
                    let size = std::fs::metadata(&info_bg.output_path).map(|m| m.len()).unwrap_or(0);
                    let mut done = info_bg.clone();
                    done.status = "done".into();
                    done.elapsed_ms = final_elapsed;
                    done.file_size_bytes = size;
                    emit_state(&app_bg, &done);
                }
                _ => {
                    let mut failed = info_bg.clone();
                    failed.status = "error".into();
                    failed.error = Some("macOS 屏幕录制进程异常退出，请检查是否已授予「屏幕录制」权限".into());
                    emit_state(&app_bg, &failed);
                }
            }
        });

        Ok(())
    }
}
