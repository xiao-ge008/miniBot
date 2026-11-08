//! 版本管理与在线更新：
//! 1. 从 GitHub Releases (xiao-ge008/miniBot) 检测最新版本与平台安装包
//! 2. SemVer 语义化版本比对（判断是否有新版本）
//! 3. 支持国内镜像加速 (ghfast / gh-proxy) 与 GitHub 官方源流式下载更新包（实时进度 + 速度 + 取消）
//! 4. 一键拉起新版本安装程序（Windows NSIS .exe / macOS .dmg）并退出当前实例完成平滑覆盖升级

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager};

pub const GITHUB_REPO: &str = "xiao-ge008/miniBot";
pub const GITHUB_RELEASES_URL: &str = "https://github.com/xiao-ge008/miniBot/releases";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    /// 当前运行版本（如 "0.1.0"）
    pub current_version: String,
    /// 远端最新发布版本（如 "0.2.0"）
    pub latest_version: String,
    /// 是否存在比当前版本更新的正式版本
    pub has_update: bool,
    /// Release 标题
    pub release_name: String,
    /// Release 更新日志 (Markdown)
    pub release_notes: String,
    /// 发布时间
    pub published_at: String,
    /// GitHub Release 详情页链接
    pub html_url: String,
    /// 匹配当前操作系统的安装包文件名
    pub asset_name: String,
    /// 匹配当前操作系统的安装包直链
    pub download_url: String,
    /// 安装包大小（MB，保留 1 位小数）
    pub asset_size_mb: f64,
    /// 当前操作系统标识："windows" | "macos" | "linux"
    pub platform: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateDownloadProgress {
    /// "downloading" | "done" | "cancelled" | "error"
    pub stage: String,
    pub file_name: String,
    pub downloaded_mb: f64,
    pub total_mb: f64,
    pub speed_mbps: f64,
    pub percent: u32,
    pub saved_path: String,
    pub message: String,
}

static UPDATE_CANCEL: OnceLock<Mutex<Option<Arc<AtomicBool>>>> = OnceLock::new();

fn cancel_slot() -> &'static Mutex<Option<Arc<AtomicBool>>> {
    UPDATE_CANCEL.get_or_init(|| Mutex::new(None))
}

pub fn cancel_update_download() {
    if let Some(flag) = cancel_slot().lock().unwrap().as_ref() {
        flag.store(true, Ordering::SeqCst);
    }
}

/// 清理版本号前缀（如 "v0.1.0" -> "0.1.0"）
pub fn clean_version(v: &str) -> String {
    v.trim()
        .trim_start_matches(|c: char| c == 'v' || c == 'V')
        .trim()
        .to_string()
}

/// 解析语义化版本号 (major, minor, patch)
fn parse_semver(v: &str) -> (u32, u32, u32) {
    let cleaned = clean_version(v);
    let core = cleaned.split('-').next().unwrap_or(&cleaned);
    let mut parts = core.split('.').map(|s| s.trim().parse::<u32>().unwrap_or(0));
    let major = parts.next().unwrap_or(0);
    let minor = parts.next().unwrap_or(0);
    let patch = parts.next().unwrap_or(0);
    (major, minor, patch)
}

/// 比较两个语义化版本号：a > b 返回 Ordering::Greater
pub fn compare_semver(a: &str, b: &str) -> std::cmp::Ordering {
    parse_semver(a).cmp(&parse_semver(b))
}

pub fn current_platform() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

/// 从 GitHub Release assets 数组中挑选匹配当前操作系统的安装包
fn pick_platform_asset(assets: &[serde_json::Value]) -> Option<(String, String, f64)> {
    let platform = current_platform();
    let mut best: Option<(i32, String, String, f64)> = None;

    for asset in assets {
        let name = asset.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let url = asset
            .get("browser_download_url")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let size_bytes = asset.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
        if name.is_empty() || url.is_empty() {
            continue;
        }
        let low = name.to_lowercase();
        let score = match platform {
            "windows" => {
                if low.ends_with("-setup.exe") || low.ends_with("_setup.exe") {
                    100
                } else if low.ends_with(".exe") {
                    90
                } else if low.ends_with(".msi") {
                    80
                } else {
                    0
                }
            }
            "macos" => {
                if low.ends_with(".dmg") {
                    100
                } else if low.ends_with(".app.tar.gz") {
                    85
                } else {
                    0
                }
            }
            _ => {
                if low.ends_with(".appimage") || low.ends_with(".deb") {
                    90
                } else {
                    0
                }
            }
        };

        if score > 0 {
            let size_mb = ((size_bytes as f64 / (1024.0 * 1024.0)) * 10.0).round() / 10.0;
            if best.as_ref().map(|b| score > b.0).unwrap_or(true) {
                best = Some((score, name.to_string(), url.to_string(), size_mb));
            }
        }
    }

    best.map(|(_, name, url, size_mb)| (name, url, size_mb))
}

/// 格式化 ISO8601 时间字符串（如 "2026-11-08T01:20:00Z" -> "2026-11-08 09:20"）
fn format_published_at(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw) {
        return dt
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M")
            .to_string();
    }
    raw.chars().take(16).collect::<String>().replace('T', " ")
}

/// 检查 GitHub Releases 最新版本
pub async fn check_update(current_ver: &str) -> Result<UpdateInfo, String> {
    let cur = clean_version(current_ver);
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(15))
        .user_agent(format!("miniBot-Updater/{cur}"))
        .build()
        .map_err(|e| format!("初始化网络客户端失败：{e}"))?;

    let latest_api = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");
    let list_api = format!("https://api.github.com/repos/{GITHUB_REPO}/releases?per_page=5");

    // 1. 优先请求 /releases/latest
    let mut release_json: Option<serde_json::Value> = None;
    if let Ok(resp) = client
        .get(&latest_api)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
    {
        if resp.status().is_success() {
            if let Ok(v) = resp.json::<serde_json::Value>().await {
                release_json = Some(v);
            }
        }
    }

    // 2. 若无 latest（例如只有 pre-release 或首次正在构建），回退查询 /releases 列表
    if release_json.is_none() {
        if let Ok(resp) = client
            .get(&list_api)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
        {
            if resp.status().is_success() {
                if let Ok(arr) = resp.json::<Vec<serde_json::Value>>().await {
                    release_json = arr.into_iter().next();
                }
            } else if resp.status().as_u16() == 403 {
                return Err("GitHub API 访问频率受限，请稍后再试或直接前往 GitHub Releases 页面查看".into());
            }
        }
    }

    // 3. 若仓库尚无任何已发布的 Release（例如 GitHub Actions 首次打包尚未结束）
    let Some(rel) = release_json else {
        return Ok(UpdateInfo {
            current_version: cur.clone(),
            latest_version: cur,
            has_update: false,
            release_name: "miniBot v0.1.0".into(),
            release_notes: "当前已是最新版本（GitHub Actions 云端构建完成后将自动在此展示安装包与更新日志）。".into(),
            published_at: String::new(),
            html_url: GITHUB_RELEASES_URL.into(),
            asset_name: String::new(),
            download_url: String::new(),
            asset_size_mb: 0.0,
            platform: current_platform().into(),
        });
    };

    let tag_name = rel
        .get("tag_name")
        .and_then(|v| v.as_str())
        .unwrap_or(&cur);
    let latest_ver = clean_version(tag_name);
    let has_update = compare_semver(&latest_ver, &cur) == std::cmp::Ordering::Greater;

    let release_name = rel
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(tag_name)
        .to_string();
    let release_notes = rel
        .get("body")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("暂无详细更新说明。")
        .to_string();
    let published_at = format_published_at(
        rel.get("published_at")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
    );
    let html_url = rel
        .get("html_url")
        .and_then(|v| v.as_str())
        .unwrap_or(GITHUB_RELEASES_URL)
        .to_string();

    let assets = rel
        .get("assets")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let (asset_name, download_url, asset_size_mb) =
        pick_platform_asset(&assets).unwrap_or_default();

    Ok(UpdateInfo {
        current_version: cur,
        latest_version: latest_ver,
        has_update,
        release_name,
        release_notes,
        published_at,
        html_url,
        asset_name,
        download_url,
        asset_size_mb,
        platform: current_platform().into(),
    })
}

/// 构造候选下载 URL 列表（开启国内加速时优先尝试高速镜像，失败自动回退官方源）
fn candidate_download_urls(raw_url: &str, use_mirror: bool) -> Vec<String> {
    let u = raw_url.trim();
    if !use_mirror || !u.starts_with("https://github.com/") {
        return vec![u.to_string()];
    }
    vec![
        format!("https://ghfast.top/{u}"),
        format!("https://gh-proxy.com/{u}"),
        u.to_string(),
    ]
}

/// 获取更新包存放目录（%TEMP%\miniBot-updates）
pub fn updates_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("miniBot-updates");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// 流式下载更新安装包并推送实时进度
pub async fn download_update(
    download_url: String,
    asset_name: String,
    use_mirror: bool,
    on_progress: Channel<UpdateDownloadProgress>,
) -> Result<String, String> {
    if download_url.trim().is_empty() {
        return Err("未找到适合当前操作系统的安装包下载链接，请前往 GitHub Releases 页面手动下载".into());
    }

    let file_name = if asset_name.trim().is_empty() {
        download_url
            .rsplit('/')
            .next()
            .unwrap_or("miniBot-update-installer.exe")
            .to_string()
    } else {
        asset_name.trim().to_string()
    };

    let cancel_flag = Arc::new(AtomicBool::new(false));
    {
        let mut slot = cancel_slot().lock().unwrap();
        if let Some(old) = slot.take() {
            old.store(true, Ordering::SeqCst);
        }
        *slot = Some(cancel_flag.clone());
    }

    let dir = updates_dir();
    let final_path = dir.join(&file_name);
    let part_path = dir.join(format!("{file_name}.part"));
    let _ = std::fs::remove_file(&part_path);

    let urls = candidate_download_urls(&download_url, use_mirror);
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(12))
        .user_agent("miniBot-Updater/0.1.0")
        .build()
        .map_err(|e| format!("创建下载客户端失败：{e}"))?;

    let _ = on_progress.send(UpdateDownloadProgress {
        stage: "downloading".into(),
        file_name: file_name.clone(),
        downloaded_mb: 0.0,
        total_mb: 0.0,
        speed_mbps: 0.0,
        percent: 1,
        saved_path: String::new(),
        message: if use_mirror {
            "正在连接国内高速镜像源…".into()
        } else {
            "正在连接 GitHub 官方下载源…".into()
        },
    });

    let mut resp_opt = None;
    let mut last_err = String::new();
    for url in &urls {
        if cancel_flag.load(Ordering::SeqCst) {
            return Err("下载已取消".into());
        }
        match client.get(url).send().await {
            Ok(r) if r.status().is_success() => {
                resp_opt = Some(r);
                break;
            }
            Ok(r) => {
                last_err = format!("HTTP {}", r.status());
            }
            Err(e) => {
                last_err = e.to_string();
            }
        }
    }

    let Some(resp) = resp_opt else {
        return Err(format!("连接更新包下载源失败：{last_err}"));
    };

    let total_bytes = resp.content_length().unwrap_or(0);
    let total_mb = ((total_bytes as f64 / (1024.0 * 1024.0)) * 10.0).round() / 10.0;

    let mut file = tokio::fs::File::create(&part_path)
        .await
        .map_err(|e| format!("创建临时安装包文件失败：{e}"))?;

    let mut downloaded: u64 = 0;
    let start_ts = Instant::now();
    let mut last_emit = Instant::now();
    let mut stream = resp.bytes_stream();

    while let Some(chunk_res) = stream.next().await {
        if cancel_flag.load(Ordering::SeqCst) {
            drop(file);
            let _ = tokio::fs::remove_file(&part_path).await;
            let _ = on_progress.send(UpdateDownloadProgress {
                stage: "cancelled".into(),
                file_name: file_name.clone(),
                downloaded_mb: 0.0,
                total_mb,
                speed_mbps: 0.0,
                percent: 0,
                saved_path: String::new(),
                message: "已取消更新包下载".into(),
            });
            return Err("下载已取消".into());
        }

        let chunk = chunk_res.map_err(|e| format!("下载更新包中断：{e}"))?;
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .map_err(|e| format!("写入更新包失败：{e}"))?;
        downloaded += chunk.len() as u64;

        if last_emit.elapsed() >= Duration::from_millis(120) {
            last_emit = Instant::now();
            let dl_mb = ((downloaded as f64 / (1024.0 * 1024.0)) * 10.0).round() / 10.0;
            let elapsed_s = start_ts.elapsed().as_secs_f64().max(0.1);
            let speed = (((downloaded as f64 / (1024.0 * 1024.0)) / elapsed_s) * 10.0).round() / 10.0;
            let pct = if total_bytes > 0 {
                ((downloaded as f64 / total_bytes as f64) * 100.0)
                    .round()
                    .clamp(1.0, 99.0) as u32
            } else {
                50
            };
            let _ = on_progress.send(UpdateDownloadProgress {
                stage: "downloading".into(),
                file_name: file_name.clone(),
                downloaded_mb: dl_mb,
                total_mb,
                speed_mbps: speed,
                percent: pct,
                saved_path: String::new(),
                message: format!("正在下载 {file_name} ({dl_mb:.1} / {total_mb:.1} MB · {speed:.1} MB/s)"),
            });
        }
    }

    tokio::io::AsyncWriteExt::flush(&mut file)
        .await
        .map_err(|e| format!("刷新安装包缓存失败：{e}"))?;
    drop(file);

    if final_path.exists() {
        let _ = tokio::fs::remove_file(&final_path).await;
    }
    tokio::fs::rename(&part_path, &final_path)
        .await
        .map_err(|e| format!("保存安装包文件失败：{e}"))?;

    let saved_str = final_path.to_string_lossy().to_string();
    let final_mb = ((downloaded as f64 / (1024.0 * 1024.0)) * 10.0).round() / 10.0;

    let _ = on_progress.send(UpdateDownloadProgress {
        stage: "done".into(),
        file_name,
        downloaded_mb: final_mb,
        total_mb: if total_mb > 0.0 { total_mb } else { final_mb },
        speed_mbps: 0.0,
        percent: 100,
        saved_path: saved_str.clone(),
        message: "✓ 更新包下载完成，点击「立即安装重启」即可完成升级".into(),
    });

    Ok(saved_str)
}

/// 启动已下载的安装包并退出当前进程，完成平滑升级
pub async fn install_update(app: &AppHandle, installer_path: String) -> Result<(), String> {
    let path = PathBuf::from(installer_path.trim());
    if !path.exists() {
        return Err("安装包文件不存在或已被清理，请重新下载更新".into());
    }

    // 先释放正在运行的本地模型进程，防止残留锁
    let st = app.state::<crate::AppState>();
    st.llm
        .release(app, crate::llm::Status::Released, "准备安装新版本更新")
        .await;

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

        // 优先尝试脱离当前进程作业组独立启动安装向导，失败则回退普通 spawn
        let spawned = std::process::Command::new(&path)
            .creation_flags(DETACHED_PROCESS | CREATE_BREAKAWAY_FROM_JOB)
            .spawn()
            .or_else(|_| std::process::Command::new(&path).spawn());

        spawned.map_err(|e| format!("启动 Windows 安装程序失败：{e}"))?;

        // 稍作等待确保安装向导进程已独立拉起，然后退出当前 miniBot 释放文件占用
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(450));
            std::process::exit(0);
        });
    }

    #[cfg(target_os = "macos")]
    {
        let dmg_str = path.to_string_lossy().replace('\'', "'\\''");
        let upgrade_script = format!(
            r#"sleep 0.5
DMG='{dmg_str}'
MNT=$(mktemp -d /tmp/minibot_dmg.XXXXXX)
if hdiutil attach "$DMG" -mountpoint "$MNT" -nobrowse -readonly -quiet; then
    APP_SRC=$(find "$MNT" -maxdepth 2 -name "*.app" | head -n 1)
    if [ -n "$APP_SRC" ] && [ -w "/Applications" ]; then
        rm -rf "/Applications/miniBot.app"
        cp -R "$APP_SRC" "/Applications/miniBot.app"
        xattr -cr "/Applications/miniBot.app" 2>/dev/null || true
        hdiutil detach "$MNT" -quiet 2>/dev/null || true
        rmdir "$MNT" 2>/dev/null || true
        open -n "/Applications/miniBot.app"
        exit 0
    fi
    hdiutil detach "$MNT" -quiet 2>/dev/null || true
fi
rmdir "$MNT" 2>/dev/null || true
open "$DMG"
"#
        );
        std::process::Command::new("sh")
            .args(["-c", &upgrade_script])
            .spawn()
            .map_err(|e| format!("启动 macOS 更新安装脚本失败：{e}"))?;

        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(350));
            std::process::exit(0);
        });
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        use tauri_plugin_opener::OpenerExt;
        app.opener()
            .open_path(path.to_string_lossy(), None::<&str>)
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_semver_compare() {
        assert_eq!(compare_semver("v0.2.0", "0.1.0"), std::cmp::Ordering::Greater);
        assert_eq!(compare_semver("0.1.1", "v0.1.0"), std::cmp::Ordering::Greater);
        assert_eq!(compare_semver("v0.1.0", "0.1.0"), std::cmp::Ordering::Equal);
        assert_eq!(compare_semver("0.1.0", "0.1.2"), std::cmp::Ordering::Less);
        assert_eq!(compare_semver("1.0.0", "0.9.9"), std::cmp::Ordering::Greater);
    }
}
