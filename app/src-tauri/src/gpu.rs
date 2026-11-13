//! 显卡管家：NVML 直接读取显存与占用进程，零配置自动检测显存超限（>= 15GB）并让道
use nvml_wrapper::{enums::device::UsedGpuMemory, Nvml};
use serde::Serialize;
use std::sync::OnceLock;

/// 计算动态让道阈值（MB）：
/// - 若显卡总显存 <= 16GB，阈值为 15GB（留 1GB 给系统/桌面）；
/// - 若显卡为 24GB / 32GB / 48GB 大显卡（用户运行 27B 等大模型正常需要 16GB~20GB 显存），
///   动态放宽至 total_mb - 1536MB，避免 27B 大模型在生成过程中因达到 15GB 被自身监控强杀截断。
pub fn yield_threshold_mb(total_mb: u64) -> u64 {
    if total_mb <= 16384 {
        15 * 1024
    } else {
        total_mb.saturating_sub(1536).max(15 * 1024)
    }
}

#[allow(dead_code)]
pub const YIELD_USED_MB: u64 = 15 * 1024;

static NVML: OnceLock<Option<Nvml>> = OnceLock::new();

fn nvml() -> Option<&'static Nvml> {
    NVML.get_or_init(|| Nvml::init().ok()).as_ref()
}

#[derive(Debug, Clone, Serialize)]
pub struct GpuProcess {
    pub pid: u32,
    pub name: String,
    /// 单进程显存（MB），WDDM 下可能为 None
    pub used_mb: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GpuSnapshot {
    pub available: bool,
    pub name: String,
    pub total_mb: u64,
    pub used_mb: u64,
    pub free_mb: u64,
    pub util: u32,
    pub processes: Vec<GpuProcess>,
}

pub fn snapshot() -> GpuSnapshot {
    let mut snap = GpuSnapshot {
        available: false,
        name: String::new(),
        total_mb: 0,
        used_mb: 0,
        free_mb: 0,
        util: 0,
        processes: vec![],
    };
    let Some(nvml) = nvml() else { return snap };
    let Ok(dev) = nvml.device_by_index(0) else { return snap };
    snap.available = true;
    snap.name = dev.name().unwrap_or_default();
    if let Ok(m) = dev.memory_info() {
        snap.total_mb = m.total / 1024 / 1024;
        snap.used_mb = m.used / 1024 / 1024;
        snap.free_mb = m.free / 1024 / 1024;
    }
    snap.util = dev.utilization_rates().map(|u| u.gpu).unwrap_or(0);

    let mut list = dev.running_compute_processes().unwrap_or_default();
    list.extend(dev.running_graphics_processes().unwrap_or_default());
    let me = std::process::id();
    let mut seen = std::collections::HashSet::new();
    for p in list {
        if p.pid == me || !seen.insert(p.pid) {
            continue;
        }
        let name = crate::winutil::process_name(p.pid).unwrap_or_else(|| format!("pid {}", p.pid));
        let lname = name.to_lowercase();
        // 跳过系统常驻进程与自身推理进程，减少噪音
        if matches!(
            lname.as_str(),
            "dwm.exe"
                | "explorer.exe"
                | "searchhost.exe"
                | "textinputhost.exe"
                | "shellexperiencehost.exe"
                | "startmenuexperiencehost.exe"
                | "app.exe"
                | "minibot.exe"
                | "msedgewebview2.exe"
                | "llama-server.exe"
        ) {
            continue;
        }
        let used_mb = match p.used_gpu_memory {
            UsedGpuMemory::Used(b) => Some(b / 1024 / 1024),
            UsedGpuMemory::Unavailable => None,
        };
        snap.processes.push(GpuProcess { pid: p.pid, name, used_mb });
    }
    snap.processes.sort_by(|a, b| b.used_mb.cmp(&a.used_mb));
    snap
}
