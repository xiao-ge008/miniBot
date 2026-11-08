//! 本地推理引擎与模型管家：
//! 1. 本机硬件自动检测（NVIDIA / AMD / Intel / Apple Silicon / CPU + 显存 + 物理内存）
//! 2. 本地引擎状态、版本检测与按硬件（N卡 CUDA / A卡 Vulkan / Mac Metal / CPU）在线下载更新
//! 3. 模型目录管理（修改目录自动迁移已有 .gguf 模型）
//! 4. LFM3-VL-2B 模型（Q4M / Q2S）一键下载（支持国内 hf-mirror 镜像与断点/取消）

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareInfo {
    /// 显卡或处理器名称
    pub gpu_name: String,
    /// "nvidia" | "amd" | "intel" | "apple" | "cpu"
    pub gpu_vendor: String,
    /// 显存大小（GB，保留 1 位小数）
    pub vram_gb: f64,
    /// 系统物理内存大小（GB，保留 1 位小数）
    pub ram_gb: f64,
    /// 处理器型号名称
    pub cpu_name: String,
    /// 处理器逻辑线程/核心数
    pub cpu_cores: u32,
    /// 是否为无独显/低配 CPU 设备（推荐优先使用云端模型 API）
    pub low_end_device: bool,
    /// 低配设备推荐开启云端模型 API 的具体原因说明
    pub low_end_reason: String,
    /// 推荐引擎加速后端："cuda" | "vulkan" | "metal" | "cpu"
    pub recommended_backend: String,
    /// 推荐引擎加速后端说明
    pub recommended_backend_label: String,
    /// 推荐模型量化规格："q4m"（≥4GB 显存） | "q2s"（<4GB 显存或 CPU）
    pub recommended_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCard {
    pub id: String,
    pub name: String,
    pub quant_tag: String,
    pub desc: String,
    pub size_label: String,
    pub recommended: bool,
    pub recommend_reason: String,
    pub installed: bool,
    pub active: bool,
    pub is_custom: bool,
    pub vision: bool,
    pub folder_path: String,
    pub model_path: String,
    pub mmproj_path: String,
    pub params: crate::settings::ModelParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScannedGgufFile {
    pub path: String,
    pub file_name: String,
    pub suggested_name: String,
    pub size_label: String,
    pub quant_tag: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScannedFolderResult {
    pub folder_path: String,
    pub models: Vec<ScannedGgufFile>,
    pub mmprojs: Vec<ScannedGgufFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineOverview {
    pub engine_installed: bool,
    pub engine_version: String,
    pub engine_backend: String,
    pub engine_dir: String,
    pub models_dir: String,
    pub mirror: String,
    pub hardware: HardwareInfo,
    pub models: Vec<ModelCard>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadProgress {
    /// "q4m" | "q2s" | "engine"
    pub target: String,
    /// "downloading" | "extracting" | "done" | "error"
    pub stage: String,
    pub file: String,
    pub downloaded_mb: f64,
    pub total_mb: f64,
    pub percent: u32,
    pub message: String,
}

static DOWNLOAD_CANCEL: OnceLock<Mutex<Option<Arc<AtomicBool>>>> = OnceLock::new();
static CACHED_ENGINE_VER: OnceLock<Mutex<Option<(String, String)>>> = OnceLock::new();
static CACHED_FALLBACK_GPU: OnceLock<(String, String, f64)> = OnceLock::new();

fn cancel_slot() -> &'static Mutex<Option<Arc<AtomicBool>>> {
    DOWNLOAD_CANCEL.get_or_init(|| Mutex::new(None))
}

pub fn cancel_download() {
    if let Some(flag) = cancel_slot().lock().unwrap().as_ref() {
        flag.store(true, Ordering::SeqCst);
    }
}

// ---------- 系统内存与硬件检测 ----------

#[cfg(windows)]
fn system_ram_gb() -> f64 {
    #[repr(C)]
    struct MEMORYSTATUSEX {
        dw_length: u32,
        dw_memory_load: u32,
        ull_total_phys: u64,
        ull_avail_phys: u64,
        ull_total_page_file: u64,
        ull_avail_page_file: u64,
        ull_total_virtual: u64,
        ull_avail_virtual: u64,
        ull_avail_extended_virtual: u64,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(lpBuffer: *mut MEMORYSTATUSEX) -> i32;
    }
    unsafe {
        let mut st: MEMORYSTATUSEX = std::mem::zeroed();
        st.dw_length = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut st) != 0 && st.ull_total_phys > 0 {
            let gb = st.ull_total_phys as f64 / (1024.0 * 1024.0 * 1024.0);
            return (gb * 10.0).round() / 10.0;
        }
    }
    16.0
}

#[cfg(not(windows))]
fn system_ram_gb() -> f64 {
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
        {
            if let Ok(bytes) = String::from_utf8_lossy(&out.stdout).trim().parse::<u64>() {
                if bytes > 0 {
                    let gb = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                    return (gb * 10.0).round() / 10.0;
                }
            }
        }
    }
    16.0
}

#[cfg(windows)]
fn detect_cpu_name() -> String {
    static CACHED_CPU: OnceLock<String> = OnceLock::new();
    CACHED_CPU
        .get_or_init(|| {
            use std::ffi::OsStr;
            use std::os::windows::ffi::{OsStrExt, OsStringExt};
            const HKEY_LOCAL_MACHINE: isize = -2147483646isize;
            const RRF_RT_REG_SZ: u32 = 0x0000_0002;
            #[link(name = "advapi32")]
            unsafe extern "system" {
                fn RegGetValueW(
                    hkey: isize,
                    lpSubKey: *const u16,
                    lpValue: *const u16,
                    dwFlags: u32,
                    pdwType: *mut u32,
                    pvData: *mut core::ffi::c_void,
                    pcbData: *mut u32,
                ) -> i32;
            }
            unsafe {
                let subkey: Vec<u16> = OsStr::new(r"HARDWARE\Description\System\CentralProcessor\0")
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
                let valname: Vec<u16> = OsStr::new("ProcessorNameString")
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
                let mut buf = [0u16; 256];
                let mut cb = (buf.len() * 2) as u32;
                let status = RegGetValueW(
                    HKEY_LOCAL_MACHINE,
                    subkey.as_ptr(),
                    valname.as_ptr(),
                    RRF_RT_REG_SZ,
                    std::ptr::null_mut(),
                    buf.as_mut_ptr() as *mut core::ffi::c_void,
                    &mut cb,
                );
                if status == 0 {
                    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                    let s = std::ffi::OsString::from_wide(&buf[..len])
                        .to_string_lossy()
                        .trim()
                        .to_string();
                    if !s.is_empty() {
                        return s;
                    }
                }
            }
            "x86_64 处理器".into()
        })
        .clone()
}

#[cfg(not(windows))]
fn detect_cpu_name() -> String {
    static CACHED_CPU: OnceLock<String> = OnceLock::new();
    CACHED_CPU
        .get_or_init(|| {
            #[cfg(target_os = "macos")]
            {
                if let Ok(out) = std::process::Command::new("sysctl")
                    .args(["-n", "machdep.cpu.brand_string"])
                    .output()
                {
                    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !s.is_empty() {
                        return s;
                    }
                }
            }
            if cfg!(target_arch = "aarch64") {
                "Apple Silicon 处理器".into()
            } else {
                "通用多核处理器".into()
            }
        })
        .clone()
}

/// 当 NVML 未检测到 NVIDIA 显卡时，通过注册表 64 位显存字段 + WMI 检测 AMD / Intel 显卡型号与真实显存（突破 WMI 4GB 溢出限制，并自动在核显+独显中选出最强显卡）
fn detect_non_nvidia_gpu() -> (String, String, f64) {
    CACHED_FALLBACK_GPU
        .get_or_init(|| {
            #[cfg(target_os = "macos")]
            {
                let cpu = detect_cpu_name();
                let ram = system_ram_gb();
                if cfg!(target_arch = "aarch64") || cpu.to_lowercase().contains("apple") {
                    // Apple Silicon 统一内存架构：Metal 可调度约 75% 系统内存作为显存
                    let metal_vram = ((ram * 0.75) * 10.0).round() / 10.0;
                    return (
                        format!("{cpu} GPU (统一内存)"),
                        "apple".into(),
                        metal_vram.max(4.0),
                    );
                }
                return ("macOS Metal 图形加速".into(), "apple".into(), 4.0);
            }
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                // 同时读取注册表 64 位真实显存 (qwMemorySize) 与 WMI Win32_VideoController，排除虚拟网卡/远程桌面驱动，选出显存最大的一块物理显卡
                let ps_script = r#"
$gpus = @()
$regPath = 'HKLM:\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\0*'
Get-ItemProperty -Path $regPath -ErrorAction SilentlyContinue | ForEach-Object {
    $name = $_.'DriverDesc'
    if ($name -and $name -notmatch 'Basic|Remote|Virtual|Parsec|Oray|GameViewer|Indirect') {
        $mem = $_.'HardwareInformation.qwMemorySize'
        if (-not $mem) { $mem = $_.'HardwareInformation.MemorySize' }
        $bytes = 0
        if ($mem -is [long] -or $mem -is [uint64] -or $mem -is [int]) { $bytes = [uint64]$mem }
        elseif ($mem -is [byte[]]) {
            if ($mem.Length -ge 8) { $bytes = [BitConverter]::ToUInt64($mem, 0) }
            elseif ($mem.Length -ge 4) { $bytes = [BitConverter]::ToUInt32($mem, 0) }
        }
        $gpus += [PSCustomObject]@{ Name = $name; Bytes = $bytes }
    }
}
if ($gpus.Count -eq 0) {
    Get-CimInstance Win32_VideoController -ErrorAction SilentlyContinue | ForEach-Object {
        if ($_.Name -and $_.Name -notmatch 'Basic|Remote|Virtual|Parsec|Oray') {
            $gpus += [PSCustomObject]@{ Name = $_.Name; Bytes = [uint64]($_.AdapterRAM) }
        }
    }
}
$best = $gpus | Sort-Object Bytes -Descending | Select-Object -First 1
if ($best) { $best | ConvertTo-Json -Compress }
"#;
                if let Ok(out) = std::process::Command::new("powershell")
                    .args(["-NoProfile", "-Command", ps_script])
                    .creation_flags(0x0800_0000)
                    .output()
                {
                    if let Ok(s) = String::from_utf8(out.stdout) {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(s.trim()) {
                            let name = v.get("Name").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
                            let ram_bytes = v.get("Bytes").and_then(|x| x.as_u64()).unwrap_or(0);
                            let mut vram_gb = ((ram_bytes as f64 / (1024.0 * 1024.0 * 1024.0)) * 10.0).round() / 10.0;
                            let lname = name.to_lowercase();
                            let vendor = if lname.contains("amd") || lname.contains("radeon") {
                                "amd"
                            } else if lname.contains("intel") || lname.contains("arc") || lname.contains("iris") || lname.contains("uhd") {
                                "intel"
                            } else if lname.contains("nvidia") || lname.contains("geforce") || lname.contains("rtx") || lname.contains("gtx") {
                                "nvidia"
                            } else {
                                "cpu"
                            };
                            // 对于 Intel Iris Xe / AMD Radeon 共享内存核显，若专用显存回报 < 1GB 且系统内存 >= 16GB，可共享约 4GB 显存供 Vulkan 调用
                            if (vendor == "amd" || vendor == "intel") && vram_gb < 1.0 {
                                let sys_ram = system_ram_gb();
                                if sys_ram >= 15.0 {
                                    vram_gb = 2.0;
                                }
                            }
                            if !name.is_empty() {
                                return (name, vendor.to_string(), vram_gb);
                            }
                        }
                    }
                }
            }
            ("CPU 通用计算模式".into(), "cpu".into(), 0.0)
        })
        .clone()
}

pub fn detect_hardware() -> HardwareInfo {
    let ram_gb = system_ram_gb();
    let cpu_name = detect_cpu_name();
    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(4);
    let snap = crate::gpu::snapshot();

    let (gpu_name, gpu_vendor, vram_gb) = if snap.available && !snap.name.is_empty() {
        let gb = ((snap.total_mb as f64 / 1024.0) * 10.0).round() / 10.0;
        (snap.name, "nvidia".to_string(), gb)
    } else {
        detect_non_nvidia_gpu()
    };

    let (recommended_backend, recommended_backend_label) = match gpu_vendor.as_str() {
        "nvidia" => ("cuda".to_string(), "NVIDIA CUDA 硬件加速".to_string()),
        "amd" => ("vulkan".to_string(), "AMD Vulkan 硬件加速".to_string()),
        "intel" => ("vulkan".to_string(), "Intel Vulkan 硬件加速".to_string()),
        "apple" => ("metal".to_string(), "Apple Metal 硬件加速".to_string()),
        _ => ("cpu".to_string(), "CPU AVX2 指令集加速".to_string()),
    };

    // 显存 >= 4GB（或 Apple Silicon）推荐 Q4M，低于 4GB 或纯 CPU 推荐 Q2S
    let recommended_model = if vram_gb >= 3.9 || gpu_vendor == "apple" {
        "q4m".to_string()
    } else {
        "q2s".to_string()
    };

    let cpu_lower = cpu_name.to_lowercase();
    let is_weak_cpu = cpu_cores <= 6
        || cpu_lower.contains("celeron")
        || cpu_lower.contains("pentium")
        || cpu_lower.contains("atom")
        || cpu_lower.contains("silver")
        || cpu_lower.contains("gold")
        || cpu_lower.contains("i3-");
    let no_dgpu = gpu_vendor == "cpu" || (gpu_vendor != "apple" && vram_gb < 2.0);
    let (low_end_device, low_end_reason) = if no_dgpu && (is_weak_cpu || ram_gb < 10.0) {
        (
            true,
            format!(
                "检测到本机无独立显卡且处理器（{} · {}线程）/ 内存（{:.1}GB）偏低，本地推理较慢，推荐在「模型 API」中开启云端模型",
                cpu_name, cpu_cores, ram_gb
            ),
        )
    } else if gpu_vendor == "cpu" && vram_gb < 0.5 {
        (
            true,
            format!(
                "本机未检测到可用显卡加速（当前仅靠 {} 纯 CPU 运算），可在「模型 API」中开启云端模型获得秒级响应",
                cpu_name
            ),
        )
    } else {
        (false, String::new())
    };

    HardwareInfo {
        gpu_name,
        gpu_vendor,
        vram_gb,
        ram_gb,
        cpu_name,
        cpu_cores,
        low_end_device,
        low_end_reason,
        recommended_backend,
        recommended_backend_label,
        recommended_model,
    }
}

// ---------- 本地引擎状态检测 ----------

fn detect_engine_version(server_path: &Path) -> String {
    if !server_path.exists() {
        return "未安装".into();
    }
    let key = server_path.to_string_lossy().to_string();
    let cache = CACHED_ENGINE_VER.get_or_init(|| Mutex::new(None));
    if let Some((k, ver)) = cache.lock().unwrap().as_ref() {
        if k == &key {
            return ver.clone();
        }
    }
    let mut ver_out = "已安装".to_string();
    let mut cmd = std::process::Command::new(server_path);
    cmd.arg("--version");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    if let Ok(out) = cmd.output() {
        let text = format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        // 匹配 "build 12189" 或 "b12189"
        if let Some(idx) = text.find("build ") {
            let digits: String = text[idx + 6..].chars().take_while(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() {
                ver_out = format!("b{digits}");
            }
        }
    }
    *cache.lock().unwrap() = Some((key, ver_out.clone()));
    ver_out
}

pub fn invalidate_engine_cache() {
    if let Some(m) = CACHED_ENGINE_VER.get() {
        *m.lock().unwrap() = None;
    }
}

fn detect_engine_backend(engine_dir: &Path) -> String {
    if engine_dir.join("ggml-cuda.dll").exists() {
        "CUDA".into()
    } else if engine_dir.join("ggml-vulkan.dll").exists() {
        "Vulkan".into()
    } else if cfg!(target_os = "macos") {
        "Metal".into()
    } else {
        "CPU".into()
    }
}

/// 递归扫描目录（最多 max_depth 层子目录）下的所有 .gguf 文件
fn scan_gguf_files(dir: &Path, max_depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fn walk(cur: &Path, depth: usize, max_depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(cur) else { return };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_file() {
                let is_gguf = p
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("gguf"))
                    .unwrap_or(false);
                if is_gguf {
                    out.push(p);
                }
            } else if p.is_dir() && depth < max_depth {
                walk(&p, depth + 1, max_depth, out);
            }
        }
    }
    walk(dir, 0, max_depth, &mut out);
    out.sort();
    out
}

fn file_size_gb_label(p: &Path, fallback: &str) -> String {
    if let Ok(meta) = std::fs::metadata(p) {
        let gb = meta.len() as f64 / (1024.0 * 1024.0 * 1024.0);
        if gb >= 0.1 {
            return format!("{gb:.1} GB");
        }
        let mb = meta.len() as f64 / (1024.0 * 1024.0);
        return format!("{mb:.0} MB");
    }
    fallback.to_string()
}

fn norm_path(p: &str) -> String {
    p.replace('\\', "/").trim().to_lowercase()
}

/// 从文件名智能提取 GGUF 量化标签（如 Q4_K_M / Q8_0 / IQ2_S / F16）
pub fn infer_quant_tag(filename: &str) -> String {
    let up = filename.to_uppercase();
    for tag in [
        "Q4_K_M", "Q4_K_S", "Q4_K_L", "Q4_0", "Q4_1", "IQ4_XS", "IQ4_NL",
        "Q5_K_M", "Q5_K_S", "Q5_0", "Q6_K", "Q8_0",
        "Q3_K_M", "Q3_K_S", "Q3_K_L", "IQ3_M", "IQ3_XS",
        "Q2_K_XL", "Q2_K", "IQ2_S", "IQ2_M", "IQ2_XS", "IQ2_XXS", "Q2_S",
        "BF16", "FP16", "F16", "F32",
    ] {
        if up.contains(tag) {
            return tag.to_string();
        }
    }
    "GGUF".into()
}

/// 从模型文件名生成简洁的推荐显示名称
pub fn clean_model_display_name(stem: &str) -> String {
    let mut s = stem.trim().to_string();
    for suffix in [
        "-Q4_K_M", "-Q4_K_S", "-Q4_K", "-Q4_0", "-Q5_K_M", "-Q5_K_S", "-Q6_K", "-Q8_0",
        "-Q3_K_M", "-Q2_K_XL", "-Q2_K", "-IQ2_S", "-UD-Q2_K_XL", "-F16", "-BF16",
        "_Q4_K_M", "_Q8_0", "_Q2_K",
    ] {
        if s.to_uppercase().ends_with(suffix) {
            let new_len = s.len().saturating_sub(suffix.len());
            s.truncate(new_len);
            break;
        }
    }
    if s.is_empty() {
        stem.to_string()
    } else {
        s
    }
}

fn apply_params_to_llm(llm: &mut crate::settings::LlmSettings, params: &crate::settings::ModelParams) {
    llm.ctx = params.ctx.max(2048);
    llm.ngl = params.ngl.clamp(0, 999);
    llm.kv_quant = if params.kv_quant.trim().is_empty() {
        "q8_0".into()
    } else {
        params.kv_quant.clone()
    };
    llm.reasoning_effort = crate::settings::normalize_reasoning_effort(&params.reasoning_effort);
    llm.idle_secs = params.idle_secs.max(30);
    llm.extra_args = params.extra_args.clone();
}

pub async fn get_overview(app: &AppHandle) -> EngineOverview {
    let st = app.state::<crate::AppState>();
    let mut s = st.settings.get();
    let hw = detect_hardware();

    let server_path = PathBuf::from(&s.llm.server_path);
    let engine_dir = server_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(crate::settings::default_engine_dir);
    // 默认模型路径强制固定在 llama/models 下不可修改
    let models_dir = engine_dir.join("models");
    let _ = std::fs::create_dir_all(&models_dir);

    let engine_installed = server_path.exists();
    let engine_version = detect_engine_version(&server_path);
    let engine_backend = if engine_installed {
        detect_engine_backend(&engine_dir)
    } else {
        "未安装".into()
    };

    // 扫描固定默认目录 llama/models（及 2 层子目录，如 llama/models/LFM3）中的所有 .gguf 文件
    let all_ggufs = scan_gguf_files(&models_dir, 2);

    // 收集默认目录下的所有 LFM 视觉投影文件 mmproj
    let all_mmprojs: Vec<PathBuf> = all_ggufs
        .iter()
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.to_lowercase().contains("mmproj"))
                .unwrap_or(false)
        })
        .cloned()
        .collect();

    let default_mmproj_path = all_mmprojs
        .first()
        .cloned()
        .unwrap_or_else(|| models_dir.join("mmproj-LFM3-VL-2B-f16.gguf"));

    let pick_mmproj_for = |model_p: &Path| -> String {
        let mname = model_p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_lowercase();
        if mname.contains("lfm2.5") {
            if let Some(found) = all_mmprojs.iter().find(|p| {
                let n = p.file_name().and_then(|x| x.to_str()).unwrap_or("").to_lowercase();
                n.contains("lfm2.5") || !n.contains("lfm3")
            }) {
                return found.to_string_lossy().into();
            }
        } else if mname.contains("lfm3") {
            if let Some(found) = all_mmprojs.iter().find(|p| {
                let n = p.file_name().and_then(|x| x.to_str()).unwrap_or("").to_lowercase();
                n.contains("lfm3")
            }) {
                return found.to_string_lossy().into();
            }
        }
        default_mmproj_path.to_string_lossy().into()
    };

    // 仅识别包含 "lfm" 的主模型文件作为内置 LFM3 2B 默认模型（绝不将其他自定义模型误识别为 LFM3）
    let mut q4m_found: Option<PathBuf> = None;
    let mut q2s_found: Option<PathBuf> = None;

    for p in &all_ggufs {
        let fname = p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_lowercase();
        if fname.contains("mmproj") || !fname.contains("lfm") {
            continue;
        }
        if q4m_found.is_none() && (fname.contains("q4_k_m") || fname.contains("q4_k") || fname.contains("q4m") || fname.contains("q4")) {
            q4m_found = Some(p.clone());
        } else if q2s_found.is_none()
            && (fname.contains("iq2_s") || fname.contains("q2_k") || fname.contains("q2_s") || fname.contains("q2s") || fname.contains("q2"))
        {
            q2s_found = Some(p.clone());
        }
    }

    let cur_model_norm = norm_path(&s.llm.model_path);

    // 1. 默认模型一：LFM3-VL 2B · Q4
    let q4m_path = q4m_found
        .clone()
        .unwrap_or_else(|| models_dir.join("LFM3-VL-2B-Q4_K_M.gguf"));
    let q4m_installed = q4m_found.is_some();
    let q4m_active = q4m_installed && norm_path(&q4m_path.to_string_lossy()) == cur_model_norm;
    let q4m_desc = "全能视觉多模态 · 支持图片与视频识别".to_string();
    let q4m_params = s
        .llm
        .model_params
        .get("q4m")
        .cloned()
        .unwrap_or_default();

    // 2. 默认模型二：LFM3-VL 2B · Q2
    let q2s_path = q2s_found
        .clone()
        .unwrap_or_else(|| models_dir.join("LFM3-VL-2B-Q2_K.gguf"));
    let q2s_installed = q2s_found.is_some();
    let q2s_active = q2s_installed && norm_path(&q2s_path.to_string_lossy()) == cur_model_norm;
    let q2s_desc = "轻量极速视觉版 · 专为低显存、核显与纯 CPU 优化".to_string();
    let q2s_params = s
        .llm
        .model_params
        .get("q2s")
        .cloned()
        .unwrap_or_default();

    let mut models: Vec<ModelCard> = vec![
        ModelCard {
            id: "q4m".into(),
            name: "LFM3-VL 2B · Q4".into(),
            quant_tag: "Q4_K_M".into(),
            desc: q4m_desc,
            size_label: file_size_gb_label(&q4m_path, "1.4 GB"),
            recommended: hw.recommended_model == "q4m",
            recommend_reason: "显存 ≥ 4GB 推荐".into(),
            installed: q4m_installed,
            active: q4m_active,
            is_custom: false,
            vision: true,
            folder_path: models_dir.to_string_lossy().into(),
            model_path: q4m_path.to_string_lossy().into(),
            mmproj_path: pick_mmproj_for(&q4m_path),
            params: q4m_params,
        },
        ModelCard {
            id: "q2s".into(),
            name: "LFM3-VL 2B · Q2".into(),
            quant_tag: "Q2_K".into(),
            desc: q2s_desc,
            size_label: file_size_gb_label(&q2s_path, "1.0 GB"),
            recommended: hw.recommended_model == "q2s",
            recommend_reason: "显存 < 4GB 或 CPU 推荐".into(),
            installed: q2s_installed,
            active: q2s_active,
            is_custom: false,
            vision: true,
            folder_path: models_dir.to_string_lossy().into(),
            model_path: q2s_path.to_string_lossy().into(),
            mmproj_path: pick_mmproj_for(&q2s_path),
            params: q2s_params,
        },
    ];

    // 3. 附加用户自定义的多个本地模型
    for cm in &s.llm.custom_models {
        let mp = PathBuf::from(&cm.model_path);
        let installed = mp.exists() && mp.is_file();
        let is_act = installed && norm_path(&cm.model_path) == cur_model_norm;
        let has_vision = !cm.mmproj_path.trim().is_empty() && Path::new(&cm.mmproj_path).exists();
        let params = s
            .llm
            .model_params
            .get(&cm.id)
            .cloned()
            .unwrap_or_else(|| cm.params.clone());
        let size_lbl = if installed {
            file_size_gb_label(&mp, &cm.size_label)
        } else if !cm.size_label.is_empty() {
            cm.size_label.clone()
        } else {
            "本地".into()
        };
        let quant = if cm.quant_tag.trim().is_empty() {
            infer_quant_tag(&cm.model_path)
        } else {
            cm.quant_tag.clone()
        };
        let desc = if cm.desc.trim().is_empty() {
            let fname = mp.file_name().and_then(|n| n.to_str()).unwrap_or("custom.gguf");
            format!("自定义本地模型：{fname}")
        } else {
            cm.desc.clone()
        };
        models.push(ModelCard {
            id: cm.id.clone(),
            name: cm.name.clone(),
            quant_tag: quant,
            desc,
            size_label: size_lbl,
            recommended: false,
            recommend_reason: String::new(),
            installed,
            active: is_act,
            is_custom: true,
            vision: has_vision,
            folder_path: cm.folder_path.clone(),
            model_path: cm.model_path.clone(),
            mmproj_path: if has_vision { cm.mmproj_path.clone() } else { String::new() },
            params,
        });
    }

    // 若当前没有任何已安装模型处于 active 状态，但列表中存在已就绪模型，则自动激活最佳已安装模型并同步其参数
    let any_active = models.iter().any(|m| m.installed && m.active);
    if !any_active {
        let best_idx = models
            .iter()
            .position(|m| m.installed && m.recommended)
            .or_else(|| models.iter().position(|m| m.installed));
        if let Some(i) = best_idx {
            models[i].active = true;
            s.llm.model_path = models[i].model_path.clone();
            s.llm.mmproj_path = models[i].mmproj_path.clone();
            apply_params_to_llm(&mut s.llm, &models[i].params);
            let _ = st.settings.set(s.clone());
        }
    } else if let Some(act_m) = models.iter().find(|m| m.installed && m.active) {
        let norm_eff = crate::settings::normalize_reasoning_effort(&act_m.params.reasoning_effort);
        if s.llm.reasoning_effort != norm_eff {
            s.llm.reasoning_effort = norm_eff;
            let _ = st.settings.set(s.clone());
        }
    }

    EngineOverview {
        engine_installed,
        engine_version,
        engine_backend,
        engine_dir: engine_dir.to_string_lossy().into(),
        models_dir: models_dir.to_string_lossy().into(),
        mirror: if s.llm.mirror.is_empty() { "modelscope".into() } else { s.llm.mirror },
        hardware: hw,
        models,
    }
}

// ---------- 切换激活模型（同时应用该模型绑定的独立参数） ----------

pub async fn activate_model(app: &AppHandle, model_id: &str) -> anyhow::Result<EngineOverview> {
    let overview = get_overview(app).await;
    let card = overview
        .models
        .iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| anyhow::anyhow!("未找到模型规格：{model_id}"))?;
    if !card.installed {
        anyhow::bail!("该模型文件尚未就绪，请先下载或检查模型路径");
    }
    let st = app.state::<crate::AppState>();
    // 若当前模型正在运行，先释放显存以便下次用新模型及新参数启动
    st.llm.release(app, crate::llm::Status::Released, "切换模型").await;
    let mut s = st.settings.get();
    s.llm.model_path = card.model_path.clone();
    s.llm.mmproj_path = card.mmproj_path.clone();
    s.llm.required_vram_mb = if model_id == "q2s" { 1500 } else { 2400 };
    apply_params_to_llm(&mut s.llm, &card.params);
    st.settings.set(s)?;
    st.llm.emit_status(app, "切换模型");
    Ok(get_overview(app).await)
}

// ---------- 保存单个模型的独立高级设置参数 ----------

pub async fn update_model_params(
    app: &AppHandle,
    model_id: String,
    params: crate::settings::ModelParams,
) -> anyhow::Result<EngineOverview> {
    let st = app.state::<crate::AppState>();
    let mut s = st.settings.get();
    let clean_params = crate::settings::ModelParams {
        ctx: params.ctx.max(2048),
        ngl: params.ngl.clamp(0, 999),
        kv_quant: if params.kv_quant.trim().is_empty() {
            "q8_0".into()
        } else {
            params.kv_quant.trim().to_lowercase()
        },
        reasoning_effort: crate::settings::normalize_reasoning_effort(&params.reasoning_effort),
        idle_secs: params.idle_secs.max(30),
        extra_args: params.extra_args.trim().to_string(),
    };
    s.llm.model_params.insert(model_id.clone(), clean_params.clone());
    if let Some(cm) = s.llm.custom_models.iter_mut().find(|m| m.id == model_id) {
        cm.params = clean_params.clone();
    }

    // 检查该模型是否为当前激活模型，若是则同步更新当前生效的引擎参数，并在运行中时自动释放以便新参数生效
    let overview_before = get_overview(app).await;
    let is_active = overview_before
        .models
        .iter()
        .any(|m| m.id == model_id && m.active);
    let mut need_restart = false;
    if is_active {
        need_restart = s.llm.ctx != clean_params.ctx
            || s.llm.ngl != clean_params.ngl
            || s.llm.kv_quant != clean_params.kv_quant
            || s.llm.extra_args != clean_params.extra_args;
        apply_params_to_llm(&mut s.llm, &clean_params);
    }
    st.settings.set(s)?;
    if is_active {
        st.llm.emit_status(app, "模型参数已更新");
        if need_restart && st.llm.status() == crate::llm::Status::Ready {
            st.llm
                .release(app, crate::llm::Status::Released, "模型参数已更新，下次请求按新参数加载")
                .await;
        }
    }
    Ok(get_overview(app).await)
}

// ---------- 用户自定义模型：选择文件夹识别主模型与视觉组件、保存与删除 ----------

pub fn scan_custom_folder_dialog(initial_dir: &str) -> Option<ScannedFolderResult> {
    let picked = pick_folder_dialog_with_title(
        initial_dir,
        "选择本地模型文件夹（自动识别 .gguf 主模型与 mmproj 视觉组件）",
    )?;
    let dir = PathBuf::from(&picked);
    let ggufs = scan_gguf_files(&dir, 2);
    let mut models = Vec::new();
    let mut mmprojs = Vec::new();

    for p in ggufs {
        let fname = p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let stem = p
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("自定义模型")
            .to_string();
        let lower = fname.to_lowercase();
        let item = ScannedGgufFile {
            path: p.to_string_lossy().into(),
            file_name: fname.clone(),
            suggested_name: clean_model_display_name(&stem),
            size_label: file_size_gb_label(&p, "本地"),
            quant_tag: infer_quant_tag(&fname),
        };
        if lower.contains("mmproj") {
            mmprojs.push(item);
        } else {
            models.push(item);
        }
    }

    Some(ScannedFolderResult {
        folder_path: picked,
        models,
        mmprojs,
    })
}

pub async fn save_custom_model(
    app: &AppHandle,
    mut custom: crate::settings::CustomLocalModel,
) -> anyhow::Result<EngineOverview> {
    let mp = PathBuf::from(custom.model_path.trim());
    if !mp.exists() || !mp.is_file() {
        anyhow::bail!("所选主模型文件不存在，请先选择包含 .gguf 模型的文件夹");
    }
    let fname = mp
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("model.gguf")
        .to_string();
    let stem = mp
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("自定义本地模型")
        .to_string();

    if custom.id.trim().is_empty() {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(1);
        custom.id = format!("custom_{ts}");
    }
    if custom.name.trim().is_empty() {
        custom.name = clean_model_display_name(&stem);
    } else {
        custom.name = custom.name.trim().to_string();
    }
    if custom.folder_path.trim().is_empty() {
        custom.folder_path = mp
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
    }
    custom.mmproj_path = custom.mmproj_path.trim().to_string();
    custom.vision = !custom.mmproj_path.is_empty() && Path::new(&custom.mmproj_path).exists();
    custom.size_label = file_size_gb_label(&mp, "本地");
    custom.quant_tag = infer_quant_tag(&fname);
    if custom.desc.trim().is_empty() {
        custom.desc = if custom.vision {
            format!("自定义多模态视觉模型：{fname}")
        } else {
            format!("自定义本地语言模型：{fname}")
        };
    } else {
        custom.desc = custom.desc.trim().to_string();
    }

    let st = app.state::<crate::AppState>();
    let mut s = st.settings.get();
    s.llm
        .model_params
        .entry(custom.id.clone())
        .or_insert_with(|| custom.params.clone());

    if let Some(existing) = s.llm.custom_models.iter_mut().find(|m| m.id == custom.id) {
        *existing = custom.clone();
    } else {
        s.llm.custom_models.push(custom.clone());
    }
    st.settings.set(s)?;
    Ok(get_overview(app).await)
}

pub async fn delete_custom_model(app: &AppHandle, model_id: String) -> anyhow::Result<EngineOverview> {
    let st = app.state::<crate::AppState>();
    let mut s = st.settings.get();
    let removed_path = s
        .llm
        .custom_models
        .iter()
        .find(|m| m.id == model_id)
        .map(|m| m.model_path.clone())
        .unwrap_or_default();
    s.llm.custom_models.retain(|m| m.id != model_id);
    s.llm.model_params.remove(&model_id);

    if !removed_path.is_empty() && norm_path(&s.llm.model_path) == norm_path(&removed_path) {
        st.llm
            .release(app, crate::llm::Status::Released, "移除当前自定义模型")
            .await;
        s.llm.model_path.clear();
        s.llm.mmproj_path.clear();
    }
    st.settings.set(s)?;
    Ok(get_overview(app).await)
}

// ---------- 选择新模型目录（原生 Win32 置顶文件夹对话框，绝不被设置窗口遮挡）并自动迁移/识别模型 ----------

pub fn pick_folder_dialog(initial_dir: &str) -> Option<String> {
    pick_folder_dialog_with_title(initial_dir, "选择模型存放文件夹（将自动识别或转移模型）")
}

pub fn pick_folder_dialog_with_title(initial_dir: &str, title: &str) -> Option<String> {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::{OsStrExt, OsStringExt};

        type HWND = isize;
        type LPITEMIDLIST = *mut core::ffi::c_void;
        type BFFCALLBACK = Option<unsafe extern "system" fn(HWND, u32, isize, isize) -> i32>;

        #[repr(C)]
        struct BROWSEINFOW {
            hwnd_owner: HWND,
            pidl_root: LPITEMIDLIST,
            psz_display_name: *mut u16,
            lpsz_title: *const u16,
            ul_flags: u32,
            lpfn: BFFCALLBACK,
            l_param: isize,
            i_image: i32,
        }

        const BIF_RETURNONLYFSDIRS: u32 = 0x0000_0001;
        const BIF_NEWDIALOGSTYLE: u32 = 0x0000_0040;
        const BFFM_INITIALIZED: u32 = 1;
        const BFFM_SETSELECTIONW: u32 = 0x0400 + 103;
        const HWND_TOPMOST: HWND = -1;
        const SWP_NOSIZE: u32 = 0x0001;
        const SWP_NOMOVE: u32 = 0x0002;
        const COINIT_APARTMENTTHREADED: u32 = 0x2;

        #[link(name = "shell32")]
        unsafe extern "system" {
            fn SHBrowseForFolderW(lpbi: *mut BROWSEINFOW) -> LPITEMIDLIST;
            fn SHGetPathFromIDListW(pidl: LPITEMIDLIST, pszPath: *mut u16) -> i32;
        }
        #[link(name = "ole32")]
        unsafe extern "system" {
            fn CoInitializeEx(pvReserved: *mut core::ffi::c_void, dwCoInit: u32) -> i32;
            fn CoTaskMemFree(pv: *mut core::ffi::c_void);
        }
        #[link(name = "user32")]
        unsafe extern "system" {
            fn GetForegroundWindow() -> HWND;
            fn SetForegroundWindow(hWnd: HWND) -> i32;
            fn SetWindowPos(
                hWnd: HWND,
                hWndInsertAfter: HWND,
                X: i32,
                Y: i32,
                cx: i32,
                cy: i32,
                uFlags: u32,
            ) -> i32;
            fn SendMessageW(hWnd: HWND, Msg: u32, wParam: usize, lParam: isize) -> isize;
        }

        unsafe extern "system" fn browse_cb(hwnd: HWND, msg: u32, _lparam: isize, lpdata: isize) -> i32 {
            if msg == BFFM_INITIALIZED {
                unsafe {
                    // 强制将文件夹选择框置于设置窗口最顶层并获取输入焦点
                    SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
                    SetForegroundWindow(hwnd);
                    if lpdata != 0 {
                        SendMessageW(hwnd, BFFM_SETSELECTIONW, 1, lpdata);
                    }
                }
            }
            0
        }

        unsafe {
            let _ = CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED);
            let owner = GetForegroundWindow();
            let mut display_buf = [0u16; 260];
            let title_wide: Vec<u16> = OsStr::new(title)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let init_wide: Vec<u16> = OsStr::new(initial_dir)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

            let mut bi = BROWSEINFOW {
                hwnd_owner: owner,
                pidl_root: std::ptr::null_mut(),
                psz_display_name: display_buf.as_mut_ptr(),
                lpsz_title: title_wide.as_ptr(),
                ul_flags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
                lpfn: Some(browse_cb),
                l_param: if initial_dir.is_empty() {
                    0
                } else {
                    init_wide.as_ptr() as isize
                },
                i_image: 0,
            };

            let pidl = SHBrowseForFolderW(&mut bi);
            if !pidl.is_null() {
                let mut path_buf = [0u16; 1024];
                let ok = SHGetPathFromIDListW(pidl, path_buf.as_mut_ptr());
                CoTaskMemFree(pidl);
                if ok != 0 {
                    let len = path_buf.iter().position(|&c| c == 0).unwrap_or(0);
                    let path = std::ffi::OsString::from_wide(&path_buf[..len])
                        .to_string_lossy()
                        .trim()
                        .to_string();
                    if !path.is_empty() {
                        return Some(path);
                    }
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = initial_dir;
        let safe_title = title.replace('"', "\\\"");
        if let Ok(out) = std::process::Command::new("osascript")
            .args([
                "-e",
                &format!("POSIX path of (choose folder with prompt \"{safe_title}\")"),
            ])
            .output()
        {
            if out.status.success() {
                let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !p.is_empty() {
                    return Some(p);
                }
            }
        }
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let _ = (initial_dir, title);
    None
}

/// 转移单个文件（同盘 rename，跨盘 copy + remove）
fn move_file_safe(src: &Path, dst: &Path) -> anyhow::Result<()> {
    if src == dst || dst.exists() {
        return Ok(());
    }
    if let Some(p) = dst.parent() {
        std::fs::create_dir_all(p)?;
    }
    if std::fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    std::fs::copy(src, dst)?;
    let _ = std::fs::remove_file(src);
    Ok(())
}

/// 修改模型存储目录：
/// 1. 若新目录已有模型，直接扫描识别并自动启用；
/// 2. 若旧目录与新目录互不包含且新目录尚无对应文件，自动将旧目录下的 .gguf 模型迁移至新目录。
pub async fn change_models_dir(app: &AppHandle, new_dir_str: String) -> anyhow::Result<(EngineOverview, usize)> {
    let new_dir = PathBuf::from(new_dir_str.trim());
    if new_dir.as_os_str().is_empty() {
        anyhow::bail!("目录路径不能为空");
    }
    std::fs::create_dir_all(&new_dir)?;

    let st = app.state::<crate::AppState>();
    st.llm.release(app, crate::llm::Status::Released, "更换模型目录").await;

    let mut s = st.settings.get();
    let old_dir = if s.llm.models_dir.trim().is_empty() {
        crate::settings::default_engine_dir().join("models")
    } else {
        PathBuf::from(&s.llm.models_dir)
    };

    let mut moved_count = 0usize;
    let same_hierarchy = new_dir.starts_with(&old_dir) || old_dir.starts_with(&new_dir);
    // 只有当新目录本身是空目录（尚无 .gguf 模型）且与旧目录无父子嵌套关系时，才把旧目录的模型迁移过去；
    // 如果用户选的是一个本身就装有模型的文件夹，直接识别新文件夹内的模型即可！
    let new_dir_existing = scan_gguf_files(&new_dir, 2);
    if new_dir_existing.is_empty() && old_dir.exists() && old_dir != new_dir && !same_hierarchy {
        let old_ggufs = scan_gguf_files(&old_dir, 2);
        for src in old_ggufs {
            if let Some(fname) = src.file_name() {
                let dst = new_dir.join(fname);
                if move_file_safe(&src, &dst).is_ok() {
                    moved_count += 1;
                    if norm_path(&s.llm.model_path) == norm_path(&src.to_string_lossy()) {
                        s.llm.model_path = dst.to_string_lossy().into();
                    }
                    if norm_path(&s.llm.mmproj_path) == norm_path(&src.to_string_lossy()) {
                        s.llm.mmproj_path = dst.to_string_lossy().into();
                    }
                }
            }
        }
    }

    s.llm.models_dir = new_dir.to_string_lossy().into();
    // 若原 model_path 不在新目录下，清空以便 get_overview 自动选取并激活新目录下识别到的模型
    if !norm_path(&s.llm.model_path).starts_with(&norm_path(&new_dir.to_string_lossy())) {
        s.llm.model_path.clear();
    }
    st.settings.set(s)?;
    let overview = get_overview(app).await;
    Ok((overview, moved_count))
}

// ---------- 流式文件下载工具（支持超时检测、慢速自动切换备用源） ----------

async fn download_url_to_path_inner(
    url: &str,
    dest: &Path,
    target_id: &str,
    display_name: &str,
    cancel: &AtomicBool,
    on_prog: &Channel<DownloadProgress>,
    abort_if_slow: bool,
) -> anyhow::Result<()> {
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p)?;
    }
    let part_name = format!(
        "{}.part",
        dest.file_name().and_then(|n| n.to_str()).unwrap_or("download.tmp")
    );
    let part_path = dest.with_file_name(part_name);
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) miniBot/0.1.0")
        .build()?;

    let resp = tokio::time::timeout(Duration::from_secs(12), client.get(url).send())
        .await
        .map_err(|_| anyhow::anyhow!("连接下载源超时：{url}"))??;
    if !resp.status().is_success() {
        anyhow::bail!("下载请求失败（HTTP {}）：{}", resp.status(), url);
    }

    let total_bytes = resp.content_length().unwrap_or(0);
    let total_mb = (total_bytes as f64 / (1024.0 * 1024.0) * 10.0).round() / 10.0;
    let mut file = tokio::fs::File::create(&part_path).await?;
    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;
    let dl_start = std::time::Instant::now();
    let mut last_emit = std::time::Instant::now();

    use tokio::io::AsyncWriteExt;
    loop {
        if cancel.load(Ordering::SeqCst) {
            drop(file);
            let _ = tokio::fs::remove_file(&part_path).await;
            anyhow::bail!("已取消下载");
        }
        let next_chunk = tokio::time::timeout(Duration::from_secs(18), stream.next()).await;
        let chunk_opt = match next_chunk {
            Ok(c) => c,
            Err(_) => {
                drop(file);
                let _ = tokio::fs::remove_file(&part_path).await;
                anyhow::bail!("下载数据流读取超时：{url}");
            }
        };
        let Some(chunk) = chunk_opt else {
            break;
        };
        let bytes = match chunk {
            Ok(b) => b,
            Err(e) => {
                drop(file);
                let _ = tokio::fs::remove_file(&part_path).await;
                return Err(e.into());
            }
        };
        file.write_all(&bytes).await?;
        downloaded += bytes.len() as u64;

        // 若启用了初始慢速检测：前 6 秒累计下载不足 384KB（约 <64KB/s），主动中断以切换至高速备用代理
        if abort_if_slow && dl_start.elapsed() >= Duration::from_secs(6) && downloaded < 384 * 1024 {
            drop(file);
            let _ = tokio::fs::remove_file(&part_path).await;
            anyhow::bail!("当前线路下载速度过慢，正在切换高速备用源");
        }

        if last_emit.elapsed() >= Duration::from_millis(120) {
            last_emit = std::time::Instant::now();
            let dl_mb = (downloaded as f64 / (1024.0 * 1024.0) * 10.0).round() / 10.0;
            let pct = if total_bytes > 0 {
                ((downloaded as f64 / total_bytes as f64) * 100.0).round() as u32
            } else {
                10
            };
            let _ = on_prog.send(DownloadProgress {
                target: target_id.to_string(),
                stage: "downloading".into(),
                file: display_name.to_string(),
                downloaded_mb: dl_mb,
                total_mb,
                percent: pct.clamp(1, 99),
                message: if total_mb > 0.0 {
                    format!("正在下载 {display_name} ({dl_mb:.1} / {total_mb:.1} MB)")
                } else {
                    format!("正在下载 {display_name} ({dl_mb:.1} MB)")
                },
            });
        }
    }
    if total_bytes > 0 && downloaded < total_bytes {
        drop(file);
        let _ = tokio::fs::remove_file(&part_path).await;
        anyhow::bail!("文件下载不完整（{downloaded} / {total_bytes} 字节）");
    }
    file.flush().await?;
    drop(file);
    if dest.exists() {
        let _ = tokio::fs::remove_file(dest).await;
    }
    tokio::fs::rename(&part_path, dest).await?;
    Ok(())
}

/// 下载 GitHub 资源：优先尝试直连（带慢速自动切换），若慢或失败自动切换至 https://gh-proxy.org/ (CF加速) 与 https://gh-proxy.com/
async fn download_github_with_proxy(
    raw_github_url: &str,
    dest: &Path,
    target_id: &str,
    display_name: &str,
    cancel: &AtomicBool,
    on_prog: &Channel<DownloadProgress>,
) -> anyhow::Result<()> {
    let raw = raw_github_url.trim();
    // 1. 先尝试直连（开启 6 秒慢速检测，慢则立即切 CF 代理）
    if download_url_to_path_inner(raw, dest, target_id, display_name, cancel, on_prog, true)
        .await
        .is_ok()
    {
        return Ok(());
    }
    if cancel.load(Ordering::SeqCst) {
        anyhow::bail!("已取消下载");
    }

    // 2. 切换至用户指定的 Cloudflare 加速代理 https://gh-proxy.org/
    let _ = on_prog.send(DownloadProgress {
        target: target_id.to_string(),
        stage: "downloading".into(),
        file: display_name.to_string(),
        downloaded_mb: 0.0,
        total_mb: 0.0,
        percent: 3,
        message: format!("正在通过 Cloudflare 加速代理 (gh-proxy.org) 下载 {display_name}…"),
    });
    let cf_proxy_url = format!("https://gh-proxy.org/{raw}");
    if download_url_to_path_inner(
        &cf_proxy_url,
        dest,
        target_id,
        display_name,
        cancel,
        on_prog,
        false,
    )
    .await
    .is_ok()
    {
        return Ok(());
    }
    if cancel.load(Ordering::SeqCst) {
        anyhow::bail!("已取消下载");
    }

    // 3. 第二备用代理 https://gh-proxy.com/
    let backup_proxy_url = format!("https://gh-proxy.com/{raw}");
    download_url_to_path_inner(
        &backup_proxy_url,
        dest,
        target_id,
        display_name,
        cancel,
        on_prog,
        false,
    )
    .await
}

// ---------- 一键下载视觉模型（支持 ModelScope 魔搭社区备用源 + HuggingFace / hf-mirror） ----------

async fn download_model_from_modelscope(
    models_dir: &Path,
    model_id: &str,
    cancel: &AtomicBool,
    on_prog: &Channel<DownloadProgress>,
) -> anyhow::Result<(PathBuf, PathBuf)> {
    let ms_base = "https://www.modelscope.cn/models/unsloth/LFM2.5-VL-3B-GGUF/resolve/master";
    let mmproj_dest = models_dir.join("mmproj-LFM2.5-VL-3B-F16.gguf");
    if !mmproj_dest.exists() {
        let mmproj_url = format!("{ms_base}/mmproj-F16.gguf");
        download_url_to_path_inner(
            &mmproj_url,
            &mmproj_dest,
            model_id,
            "视觉组件 mmproj-F16.gguf (ModelScope)",
            cancel,
            on_prog,
            false,
        )
        .await?;
    }

    let model_file = if model_id == "q2s" {
        "LFM2.5-VL-3B-UD-Q2_K_XL.gguf"
    } else {
        "LFM2.5-VL-3B-Q4_K_M.gguf"
    };
    let model_dest = models_dir.join(model_file);
    let model_url = format!("{ms_base}/{model_file}");
    download_url_to_path_inner(
        &model_url,
        &model_dest,
        model_id,
        &format!("{model_file} (ModelScope)"),
        cancel,
        on_prog,
        false,
    )
    .await?;
    Ok((model_dest, mmproj_dest))
}

async fn download_model_from_hf(
    models_dir: &Path,
    model_id: &str,
    base_host: &str,
    cancel: &AtomicBool,
    on_prog: &Channel<DownloadProgress>,
    abort_if_slow: bool,
) -> anyhow::Result<(PathBuf, PathBuf)> {
    let repo_base = format!("{base_host}/bartowski/LFM3-VL-2B-GGUF/resolve/main");
    let mmproj_dest = models_dir.join("mmproj-LFM3-VL-2B-f16.gguf");
    if !mmproj_dest.exists() {
        let url = format!("{repo_base}/mmproj-LFM3-VL-2B-f16.gguf?download=true");
        download_url_to_path_inner(
            &url,
            &mmproj_dest,
            model_id,
            "视觉组件 mmproj-LFM3-VL-2B-f16.gguf",
            cancel,
            on_prog,
            abort_if_slow,
        )
        .await?;
    }

    let primary_file = if model_id == "q2s" {
        "LFM3-VL-2B-Q2_K.gguf"
    } else {
        "LFM3-VL-2B-Q4_K_M.gguf"
    };
    let model_dest = models_dir.join(primary_file);
    let primary_url = format!("{repo_base}/{primary_file}?download=true");
    download_url_to_path_inner(
        &primary_url,
        &model_dest,
        model_id,
        primary_file,
        cancel,
        on_prog,
        abort_if_slow,
    )
    .await?;
    Ok((model_dest, mmproj_dest))
}

pub async fn download_model(
    app: &AppHandle,
    model_id: String,
    mirror: String,
    on_prog: Channel<DownloadProgress>,
) -> anyhow::Result<EngineOverview> {
    let cancel = Arc::new(AtomicBool::new(false));
    *cancel_slot().lock().unwrap() = Some(cancel.clone());

    let st = app.state::<crate::AppState>();
    let mut s = st.settings.get();
    if !mirror.trim().is_empty() {
        s.llm.mirror = mirror.clone();
        let _ = st.settings.set(s.clone());
    }

    let engine_dir = PathBuf::from(&s.llm.server_path)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(crate::settings::default_engine_dir);
    let models_dir = engine_dir.join("models");
    std::fs::create_dir_all(&models_dir)?;

    let chosen_mirror = if mirror.trim().is_empty() {
        "modelscope"
    } else {
        mirror.trim()
    };

    let (final_model_dest, mmproj_dest) = if chosen_mirror == "modelscope" {
        download_model_from_modelscope(&models_dir, &model_id, &cancel, &on_prog).await?
    } else {
        let base_host = if chosen_mirror == "huggingface" {
            "https://huggingface.co"
        } else {
            "https://hf-mirror.com"
        };
        match download_model_from_hf(&models_dir, &model_id, base_host, &cancel, &on_prog, true).await {
            Ok(pair) => pair,
            Err(_) if !cancel.load(Ordering::SeqCst) => {
                let _ = on_prog.send(DownloadProgress {
                    target: model_id.clone(),
                    stage: "downloading".into(),
                    file: "切换魔搭社区 (ModelScope) 备用源…".into(),
                    downloaded_mb: 0.0,
                    total_mb: 0.0,
                    percent: 2,
                    message: "当前源响应较慢，已自动切换至国内 ModelScope 魔搭社区备用源…".into(),
                });
                download_model_from_modelscope(&models_dir, &model_id, &cancel, &on_prog).await?
            }
            Err(e) => return Err(e),
        }
    };

    // 下载完成后自动切换启用该模型，并应用该模型的独立参数
    st.llm.release(app, crate::llm::Status::Released, "新模型已就绪").await;
    let mut s2 = st.settings.get();
    s2.llm.models_dir = models_dir.to_string_lossy().into();
    s2.llm.model_path = final_model_dest.to_string_lossy().into();
    s2.llm.mmproj_path = mmproj_dest.to_string_lossy().into();
    s2.llm.required_vram_mb = if model_id == "q2s" { 1500 } else { 2400 };
    let params = s2.llm.model_params.get(&model_id).cloned().unwrap_or_default();
    s2.llm.model_params.entry(model_id.clone()).or_insert_with(|| params.clone());
    apply_params_to_llm(&mut s2.llm, &params);
    st.settings.set(s2)?;

    let _ = on_prog.send(DownloadProgress {
        target: model_id,
        stage: "done".into(),
        file: String::new(),
        downloaded_mb: 0.0,
        total_mb: 0.0,
        percent: 100,
        message: "✓ 模型下载完成并已自动启用".into(),
    });

    Ok(get_overview(app).await)
}

// ---------- 按硬件架构（N卡 CUDA / A卡 Vulkan / Mac Metal / CPU）下载更新本地引擎 ----------

/// 确保引擎安装目录可写；若原路径无写入权限，自动回退到 %LOCALAPPDATA%\com.minibot.app\llama
fn resolve_writable_engine_dir(app: &AppHandle, current_server_path: &str) -> PathBuf {
    let candidate = PathBuf::from(current_server_path)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(crate::settings::default_engine_dir);

    if std::fs::create_dir_all(&candidate).is_ok() {
        let probe = candidate.join(".write_probe");
        if std::fs::write(&probe, b"ok").is_ok() {
            let _ = std::fs::remove_file(&probe);
            return candidate;
        }
    }
    if let Ok(local_data) = app.path().app_local_data_dir() {
        let fallback = local_data.join("llama");
        let _ = std::fs::create_dir_all(&fallback);
        return fallback;
    }
    candidate
}

fn match_engine_asset_name(name: &str, target_backend: &str) -> bool {
    let n = name.to_lowercase();
    match target_backend {
        "cuda" => {
            n.ends_with(".zip")
                && n.starts_with("llama-")
                && n.contains("bin-win-cuda-")
                && n.contains("x64")
                && !n.contains("cudart")
        }
        "vulkan" => n.ends_with(".zip") && n.contains("bin-win-vulkan-x64"),
        "metal" => {
            let arch_tag = if cfg!(target_arch = "x86_64") {
                "bin-macos-x64"
            } else {
                "bin-macos-arm64"
            };
            (n.ends_with(".zip") || n.ends_with(".tar.gz")) && n.contains(arch_tag)
        }
        _ => n.ends_with(".zip") && (n.contains("bin-win-cpu-x64") || n.contains("bin-win-avx2-x64")),
    }
}

/// 查询最新包含二进制包的 Release（遍历最近 10 个版本，跳过仅含 nightly-tag.txt 的标签；若 API 不可达则返回稳定版本 b12194 兜底）
async fn resolve_engine_release_urls(
    target_backend: &str,
) -> (String, String, Option<(String, String)>) {
    let fallback_tag = "b12194";
    let macos_arch = if cfg!(target_arch = "x86_64") { "x64" } else { "arm64" };
    let fallback = match target_backend {
        "cuda" => (
            format!("llama-{fallback_tag}-bin-win-cuda-12.4-x64.zip"),
            format!("https://github.com/ggml-org/llama.cpp/releases/download/{fallback_tag}/llama-{fallback_tag}-bin-win-cuda-12.4-x64.zip"),
            Some((
                "cudart-llama-bin-win-cuda-12.4-x64.zip".to_string(),
                format!("https://github.com/ggml-org/llama.cpp/releases/download/{fallback_tag}/cudart-llama-bin-win-cuda-12.4-x64.zip"),
            )),
        ),
        "vulkan" => (
            format!("llama-{fallback_tag}-bin-win-vulkan-x64.zip"),
            format!("https://github.com/ggml-org/llama.cpp/releases/download/{fallback_tag}/llama-{fallback_tag}-bin-win-vulkan-x64.zip"),
            None,
        ),
        "metal" => (
            format!("llama-{fallback_tag}-bin-macos-{macos_arch}.tar.gz"),
            format!("https://github.com/ggml-org/llama.cpp/releases/download/{fallback_tag}/llama-{fallback_tag}-bin-macos-{macos_arch}.tar.gz"),
            None,
        ),
        _ => (
            format!("llama-{fallback_tag}-bin-win-cpu-x64.zip"),
            format!("https://github.com/ggml-org/llama.cpp/releases/download/{fallback_tag}/llama-{fallback_tag}-bin-win-cpu-x64.zip"),
            None,
        ),
    };

    let Ok(client) = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(10))
        .user_agent("miniBot/0.1.0")
        .build()
    else {
        return fallback;
    };

    let api_urls = [
        "https://api.github.com/repos/ggml-org/llama.cpp/releases?per_page=10",
        "https://gh-proxy.org/https://api.github.com/repos/ggml-org/llama.cpp/releases?per_page=10",
    ];

    for api_url in api_urls {
        let Ok(resp) = client.get(api_url).send().await else {
            continue;
        };
        let Ok(releases) = resp.json::<Vec<serde_json::Value>>().await else {
            continue;
        };
        for rel in &releases {
            let Some(assets) = rel.get("assets").and_then(|a| a.as_array()) else {
                continue;
            };
            // 对 CUDA 优先匹配 cuda-12.4-x64，兼容性最好
            let chosen = if target_backend == "cuda" {
                assets
                    .iter()
                    .find(|a| {
                        let name = a.get("name").and_then(|x| x.as_str()).unwrap_or("");
                        match_engine_asset_name(name, "cuda") && name.contains("12.4")
                    })
                    .or_else(|| {
                        assets.iter().find(|a| {
                            let name = a.get("name").and_then(|x| x.as_str()).unwrap_or("");
                            match_engine_asset_name(name, "cuda")
                        })
                    })
            } else {
                assets.iter().find(|a| {
                    let name = a.get("name").and_then(|x| x.as_str()).unwrap_or("");
                    match_engine_asset_name(name, target_backend)
                })
            };

            if let Some(asset) = chosen {
                let name = asset
                    .get("name")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let url = asset
                    .get("browser_download_url")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                if !name.is_empty() && !url.is_empty() {
                    let cudart = if target_backend == "cuda" {
                        let want_12 = name.contains("12.");
                        assets
                            .iter()
                            .find(|a| {
                                let an = a
                                    .get("name")
                                    .and_then(|x| x.as_str())
                                    .unwrap_or("")
                                    .to_lowercase();
                                an.starts_with("cudart-llama-bin-win-cuda-")
                                    && an.ends_with("-x64.zip")
                                    && (!want_12 || an.contains("12."))
                            })
                            .and_then(|ca| {
                                let cn = ca.get("name")?.as_str()?.to_string();
                                let cu = ca.get("browser_download_url")?.as_str()?.to_string();
                                Some((cn, cu))
                            })
                    } else {
                        None
                    };
                    return (name, url, cudart);
                }
            }
        }
    }

    fallback
}

/// 解压压缩包到 engine_dir（优先调用系统内置高速 tar.exe，回退 PowerShell Expand-Archive）
fn extract_archive_to_dir(archive_path: &Path, engine_dir: &Path) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // 1. 优先使用 Windows 10/11 原生内置的 tar.exe（支持 .zip 与 .tar.gz，速度比 PowerShell 快 10 倍）
        let tar_ok = std::process::Command::new("tar.exe")
            .args([
                "-xf",
                &archive_path.to_string_lossy(),
                "-C",
                &engine_dir.to_string_lossy(),
            ])
            .creation_flags(0x0800_0000)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !tar_ok || !engine_dir.join("llama-server.exe").exists() {
            let ps_cmd = format!(
                "$ProgressPreference = 'SilentlyContinue'; Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
                archive_path.to_string_lossy().replace('\'', "''"),
                engine_dir.to_string_lossy().replace('\'', "''")
            );
            let out = std::process::Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", &ps_cmd])
                .creation_flags(0x0800_0000)
                .output()?;
            if !out.status.success() && !engine_dir.join("llama-server.exe").exists() {
                let err_msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
                anyhow::bail!("解压引擎安装包失败：{err_msg}");
            }
        }
    }
    #[cfg(not(windows))]
    {
        let name = archive_path.to_string_lossy().to_lowercase();
        if name.ends_with(".tar.gz") {
            let _ = std::process::Command::new("tar")
                .args([
                    "-xzf",
                    &archive_path.to_string_lossy(),
                    "-C",
                    &engine_dir.to_string_lossy(),
                ])
                .output();
        } else {
            let _ = std::process::Command::new("unzip")
                .args([
                    "-o",
                    &archive_path.to_string_lossy(),
                    "-d",
                    &engine_dir.to_string_lossy(),
                ])
                .output();
        }
    }

    // 若压缩包内部带有子目录（如 build/bin/llama-server 或 llama-bXXXX/bin/llama-server），自动将其内部文件及动态库软链接提升至 engine_dir 根目录
    let server_bin = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };
    if !engine_dir.join(server_bin).exists() {
        fn find_bin_parent(cur: &Path, bin_name: &str, depth: usize, max_depth: usize) -> Option<PathBuf> {
            if cur.join(bin_name).exists() {
                return Some(cur.to_path_buf());
            }
            if depth >= max_depth {
                return None;
            }
            let Ok(rd) = std::fs::read_dir(cur) else { return None };
            for entry in rd.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    if let Some(found) = find_bin_parent(&p, bin_name, depth + 1, max_depth) {
                        return Some(found);
                    }
                }
            }
            None
        }

        if let Some(sub_dir) = find_bin_parent(engine_dir, server_bin, 0, 3) {
            if sub_dir != engine_dir {
                if let Ok(rd) = std::fs::read_dir(&sub_dir) {
                    for entry in rd.flatten() {
                        let p = entry.path();
                        if let Some(fname) = p.file_name() {
                            let dst = engine_dir.join(fname);
                            let _ = std::fs::remove_file(&dst);
                            let _ = std::fs::rename(&p, &dst);
                        }
                    }
                }
            }
        }
    }

    // macOS / Linux 下赋予 llama-server 与动态库可执行权限，并清除 macOS Gatekeeper 隔离属性 (com.apple.quarantine)
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        let bin_path = engine_dir.join(server_bin);
        if let Ok(meta) = std::fs::metadata(&bin_path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(&bin_path, perms);
        }
        let _ = std::process::Command::new("chmod")
            .args(["-R", "+x", &engine_dir.to_string_lossy()])
            .output();
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("xattr")
                .args(["-cr", &engine_dir.to_string_lossy()])
                .output();
        }
    }
    Ok(())
}

pub async fn download_engine(
    app: &AppHandle,
    backend_hint: String,
    on_prog: Channel<DownloadProgress>,
) -> anyhow::Result<EngineOverview> {
    let cancel = Arc::new(AtomicBool::new(false));
    *cancel_slot().lock().unwrap() = Some(cancel.clone());

    let st = app.state::<crate::AppState>();
    st.llm.release(app, crate::llm::Status::Released, "更新本地引擎").await;
    let mut s = st.settings.get();
    let hw = detect_hardware();

    let target_backend = if backend_hint.trim().is_empty() || backend_hint == "auto" {
        hw.recommended_backend.clone()
    } else {
        backend_hint
    };

    let engine_dir = resolve_writable_engine_dir(app, &s.llm.server_path);
    std::fs::create_dir_all(&engine_dir)?;

    let _ = on_prog.send(DownloadProgress {
        target: "engine".into(),
        stage: "downloading".into(),
        file: "查询最新引擎版本…".into(),
        downloaded_mb: 0.0,
        total_mb: 0.0,
        percent: 2,
        message: "正在查询适配当前硬件的最新推理引擎构建…".into(),
    });

    let (asset_name, download_url, cudart_opt) = resolve_engine_release_urls(&target_backend).await;

    let ext = if asset_name.ends_with(".tar.gz") { "tar.gz" } else { "zip" };
    let archive_path = engine_dir.join(format!("engine_update.{ext}"));

    download_github_with_proxy(
        &download_url,
        &archive_path,
        "engine",
        &asset_name,
        &cancel,
        &on_prog,
    )
    .await?;

    // 若为 NVIDIA CUDA 后端，且 engine_dir 中尚无 cublas64_*.dll，则自动补充下载 cudart 运行时库，确保未装 CUDA Toolkit 的电脑也能开箱即用
    if target_backend == "cuda" {
        let has_cublas = std::fs::read_dir(&engine_dir)
            .ok()
            .map(|rd| {
                rd.flatten().any(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .to_lowercase()
                        .starts_with("cublas64_")
                })
            })
            .unwrap_or(false);
        if !has_cublas {
            if let Some((cudart_name, cudart_url)) = cudart_opt {
                let cudart_zip = engine_dir.join("cudart_update.zip");
                if download_github_with_proxy(
                    &cudart_url,
                    &cudart_zip,
                    "engine",
                    &cudart_name,
                    &cancel,
                    &on_prog,
                )
                .await
                .is_ok()
                {
                    let _ = extract_archive_to_dir(&cudart_zip, &engine_dir);
                    let _ = std::fs::remove_file(&cudart_zip);
                }
            }
        }
    }

    let _ = on_prog.send(DownloadProgress {
        target: "engine".into(),
        stage: "extracting".into(),
        file: asset_name.clone(),
        downloaded_mb: 0.0,
        total_mb: 0.0,
        percent: 96,
        message: "正在清理旧后端并解压部署新推理引擎…".into(),
    });

    // 切换不同硬件后端（如从 N卡 切换到 A卡/Intel Vulkan 或 CPU）时，清理可能冲突的旧 GPU 后端 DLL
    if target_backend != "cuda" {
        let _ = std::fs::remove_file(engine_dir.join("ggml-cuda.dll"));
        let _ = std::fs::remove_file(engine_dir.join("ggml-cuda.dll.disabled"));
    }
    if target_backend != "vulkan" {
        let _ = std::fs::remove_file(engine_dir.join("ggml-vulkan.dll"));
    }

    let extract_res = extract_archive_to_dir(&archive_path, &engine_dir);
    let _ = std::fs::remove_file(&archive_path);
    extract_res?;

    let server_bin = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };
    let new_server_path = engine_dir.join(server_bin);
    if !new_server_path.exists() {
        anyhow::bail!("引擎包解压后未找到 {server_bin}，请检查杀毒软件是否拦截或磁盘空间是否充足");
    }

    let new_server_str = new_server_path.to_string_lossy().into_owned();
    if s.llm.server_path != new_server_str {
        s.llm.server_path = new_server_str;
        let _ = st.settings.set(s);
    }

    invalidate_engine_cache();

    let _ = on_prog.send(DownloadProgress {
        target: "engine".into(),
        stage: "done".into(),
        file: String::new(),
        downloaded_mb: 0.0,
        total_mb: 0.0,
        percent: 100,
        message: "✓ 本地推理引擎安装/更新完成".into(),
    });

    Ok(get_overview(app).await)
}
