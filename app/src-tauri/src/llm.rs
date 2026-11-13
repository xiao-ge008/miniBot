//! 模型管家：按需启动 llama-server、闲置自动释放、显存超 15GB 自动释放让道、流式对话
use crate::gpu::{self, GpuSnapshot};
use crate::settings::LlmSettings;
use futures_util::StreamExt;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager};
use tokio::process::{Child, Command};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Released,
    Loading,
    Ready,
    Yielded,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusInfo {
    pub status: Status,
    /// 距离自动释放的剩余秒数（仅本地引擎 Ready 时有意义）
    pub remaining_secs: u64,
    pub reason: String,
    /// 是否已启用云端 OpenAI 兼容模型 API
    pub api_enabled: bool,
    /// 当前配置的云端模型名称
    pub api_model: String,
    /// 当前生效模型的默认思考等级："off" | "low" | "medium" | "high" | "max"
    pub reasoning_effort: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CloudTestResult {
    pub ok: bool,
    pub latency_ms: u64,
    pub model: String,
    pub reply: String,
}

/// 将用户填写的 Base URL 智能规范化为 OpenAI 兼容的 `/chat/completions` 端点
pub fn resolve_chat_completions_url(base_url: &str) -> String {
    let u = base_url.trim().trim_end_matches('/');
    if u.is_empty() {
        return "https://api.siliconflow.cn/v1/chat/completions".into();
    }
    if u.ends_with("/chat/completions") {
        return u.to_string();
    }
    if u.ends_with("/v1") || u.ends_with("/v2") || u.ends_with("/v3") || u.ends_with("/v4") {
        format!("{u}/chat/completions")
    } else {
        format!("{u}/v1/chat/completions")
    }
}

/// 将用户填写的 Base URL 智能规范化为 OpenAI 兼容的 `/models` 端点
pub fn resolve_models_url(base_url: &str) -> String {
    let u = base_url.trim().trim_end_matches('/');
    if u.is_empty() {
        return "https://api.siliconflow.cn/v1/models".into();
    }
    if let Some(prefix) = u.strip_suffix("/chat/completions") {
        return format!("{prefix}/models");
    }
    if u.ends_with("/models") {
        return u.to_string();
    }
    if u.ends_with("/v1") || u.ends_with("/v2") || u.ends_with("/v3") || u.ends_with("/v4") {
        format!("{u}/models")
    } else {
        format!("{u}/v1/models")
    }
}

/// 根据目标 URL 智能构建 HTTP 客户端（本机地址绕过代理，公网地址允许使用系统代理）
pub fn build_http_client(url: &str, timeout_secs: u64) -> Result<reqwest::Client, reqwest::Error> {
    let mut b = reqwest::Client::builder().connect_timeout(Duration::from_secs(15));
    if timeout_secs > 0 {
        b = b.timeout(Duration::from_secs(timeout_secs));
    }
    if url.contains("://127.0.0.1") || url.contains("://localhost") {
        b = b.no_proxy();
    }
    b.build()
}

/// 检查 messages 中是否包含 image_url 多模态块
fn messages_have_images(messages: &serde_json::Value) -> bool {
    let Some(arr) = messages.as_array() else { return false };
    for m in arr {
        if let Some(parts) = m.get("content").and_then(|c| c.as_array()) {
            if parts.iter().any(|p| p.get("type").and_then(|t| t.as_str()) == Some("image_url")) {
                return true;
            }
        }
    }
    false
}

/// 当云端模型为纯文本模型（不支持 Vision）时，自动调用本地离线 OCR 提取图片文字并拼入纯文本消息
async fn convert_vision_messages_via_ocr(messages: serde_json::Value) -> serde_json::Value {
    let Some(arr) = messages.as_array() else { return messages };
    let mut out = Vec::with_capacity(arr.len());
    for m in arr {
        let mut msg_obj = m.clone();
        if let Some(parts) = m.get("content").and_then(|c| c.as_array()) {
            let mut text_blocks: Vec<String> = Vec::new();
            let mut img_idx = 0usize;
            for p in parts {
                let ptype = p.get("type").and_then(|t| t.as_str()).unwrap_or("");
                if ptype == "image_url" {
                    img_idx += 1;
                    if let Some(url) = p.pointer("/image_url/url").and_then(|u| u.as_str()) {
                        let data_str = url.to_string();
                        let ocr_txt = tokio::task::spawn_blocking(move || -> Option<String> {
                            let bytes = crate::imgutil::decode_b64(&data_str).ok()?;
                            let rgba = crate::imgutil::decode_rgba(&bytes).ok()?;
                            let res = crate::ocr::recognize(&rgba).ok()?;
                            let t = res.text.trim().to_string();
                            if t.is_empty() { None } else { Some(t) }
                        })
                        .await
                        .ok()
                        .flatten();
                        if let Some(t) = ocr_txt {
                            text_blocks.push(format!("【图片 #{img_idx} 本地 OCR 识别文字】：\n{t}"));
                        } else {
                            text_blocks.push(format!("【图片 #{img_idx}：未检测到清晰文字】"));
                        }
                    }
                } else if ptype == "text" {
                    if let Some(t) = p.get("text").and_then(|x| x.as_str()) {
                        if !t.trim().is_empty() {
                            text_blocks.push(t.to_string());
                        }
                    }
                }
            }
            msg_obj["content"] = serde_json::Value::String(text_blocks.join("\n\n"));
        }
        out.push(msg_obj);
    }
    serde_json::Value::Array(out)
}

/// 返回给前端的结构化错误
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LlmError {
    GpuBusy { snapshot: GpuSnapshot, required_mb: u64 },
    PortBusy { port: u16 },
    Cancelled,
    Other { message: String },
}

impl<E: std::fmt::Display> From<E> for LlmError {
    fn from(e: E) -> Self {
        LlmError::Other { message: e.to_string() }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, serde::Deserialize)]
pub struct ChatStats {
    pub tokens: u64,
    pub tokens_per_sec: f64,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatEvent {
    Loading,
    Started,
    Delta { text: String },
    Stats {
        tokens: u64,
        tokens_per_sec: f64,
        duration_ms: u64,
    },
}

pub struct ActiveGuard(Arc<LlmManager>);

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
        self.0.touch();
    }
}

#[derive(Default)]
pub struct LlmManager {
    child: tokio::sync::Mutex<Option<Child>>,
    load_lock: tokio::sync::Mutex<()>,
    status: Mutex<Option<Status>>,
    last_used: Mutex<Option<Instant>>,
    active: AtomicUsize,
    cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// 最近一次因显存超限（>=15GB）触发自动让道时的显卡快照，供被中断的流式请求返回 GpuBusy
    yielded_snapshot: Mutex<Option<GpuSnapshot>>,
}

/// 探测指定端口的本地推理服务是否已在健康运行
pub async fn probe_health(port: u16) -> bool {
    let client = match reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(600))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    let url = format!("http://127.0.0.1:{port}/health");
    if let Ok(r) = client.get(&url).send().await {
        return r.status().is_success();
    }
    false
}

/// 从 start 开始顺延探测第一个可绑定的 TCP 端口
pub fn find_available_port(start: u16, max_attempts: u16) -> Option<u16> {
    for p in start..start.saturating_add(max_attempts) {
        if std::net::TcpListener::bind(("127.0.0.1", p)).is_ok() {
            return Some(p);
        }
    }
    None
}

impl LlmManager {
    pub fn status(&self) -> Status {
        self.status.lock().unwrap().unwrap_or(Status::Released)
    }

    fn touch(&self) {
        *self.last_used.lock().unwrap() = Some(Instant::now());
    }

    pub fn status_info(&self, llm_cfg: &LlmSettings) -> StatusInfo {
        let status = self.status();
        let remaining = match (status, *self.last_used.lock().unwrap()) {
            (Status::Ready, Some(t)) => llm_cfg.idle_secs.saturating_sub(t.elapsed().as_secs()),
            _ => 0,
        };
        StatusInfo {
            status,
            remaining_secs: remaining,
            reason: String::new(),
            api_enabled: llm_cfg.api_enabled,
            api_model: llm_cfg.api_model.clone(),
            reasoning_effort: crate::settings::normalize_reasoning_effort(&llm_cfg.reasoning_effort),
        }
    }

    pub fn emit_status(&self, app: &AppHandle, reason: &str) {
        let llm_cfg = app.state::<crate::AppState>().settings.get().llm;
        let mut info = self.status_info(&llm_cfg);
        info.reason = reason.to_string();
        let _ = app.emit("llm://status", &info);
        crate::tray::refresh(app);
    }

    fn set_status(&self, app: &AppHandle, s: Status, reason: &str) {
        *self.status.lock().unwrap() = Some(s);
        self.emit_status(app, reason);
    }

    /// 若当前处于因显存超 15GB 触发的 Yielded 状态，构造 GpuBusy 错误以通知前端提示繁忙
    fn yield_error(&self, required_mb: u64) -> Option<LlmError> {
        if self.status() == Status::Yielded {
            let snapshot = self
                .yielded_snapshot
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(gpu::snapshot);
            Some(LlmError::GpuBusy { snapshot, required_mb })
        } else {
            None
        }
    }

    /// 预热：截图热键按下时调用，静默尝试加载（开启云端 API 或显卡忙则直接返回）
    pub async fn prewarm(&self, app: &AppHandle) {
        if app.state::<crate::AppState>().settings.get().llm.api_enabled {
            return;
        }
        let _ = self.ensure_ready(app, false).await;
    }

    /// 确保模型就绪；force=true 时跳过显卡检查
    pub async fn ensure_ready(&self, app: &AppHandle, force: bool) -> Result<(), LlmError> {
        if self.is_alive().await && self.status() == Status::Ready {
            self.touch();
            return Ok(());
        }
        let _guard = self.load_lock.lock().await;
        if self.is_alive().await && self.status() == Status::Ready {
            self.touch();
            return Ok(());
        }
        let st = app.state::<crate::AppState>();
        let settings = st.settings.get();
        let cfg = settings.llm.clone();

        if !force {
            let snap = gpu::snapshot();
            let limit = gpu::yield_threshold_mb(snap.total_mb);
            if snap.available && (snap.used_mb >= limit || snap.free_mb < cfg.required_vram_mb) {
                return Err(LlmError::GpuBusy { snapshot: snap, required_mb: cfg.required_vram_mb });
            }
        }

        let mut cfg = cfg;
        let port = cfg.port;
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_err() {
            // 1. 若当前端口已有健康运行的 llama-server，直接复用，秒级响应
            if probe_health(port).await {
                crate::log_msg(app, format!("[llm] 端口 {port} 已有健康推理服务运行，直接复用"));
                *self.yielded_snapshot.lock().unwrap() = None;
                self.touch();
                self.set_status(app, Status::Ready, "");
                return Ok(());
            }

            // 2. 否则说明端口被非健康/第三方进程占用，自动顺延探测空闲端口，避免报错拦截用户
            if let Some(avail) = find_available_port(port.saturating_add(1), 30) {
                crate::log_msg(app, format!("[llm] 默认端口 {port} 被占用，自动顺延切换至可用端口 {avail}"));
                let mut new_settings = settings.clone();
                new_settings.llm.port = avail;
                let _ = st.settings.set(new_settings);
                cfg.port = avail;
            } else {
                return Err(LlmError::PortBusy { port });
            }
        }

        *self.yielded_snapshot.lock().unwrap() = None;
        self.set_status(app, Status::Loading, "");
        match self.spawn(app, &cfg).await {
            Ok(()) => {
                self.touch();
                self.set_status(app, Status::Ready, "");
                Ok(())
            }
            Err(e) => {
                self.kill().await;
                self.set_status(app, Status::Released, "加载失败");
                Err(e)
            }
        }
    }

    async fn spawn(&self, app: &AppHandle, cfg: &LlmSettings) -> Result<(), LlmError> {
        let server = std::path::Path::new(&cfg.server_path);
        if !server.exists() {
            return Err(LlmError::Other {
                message: "尚未安装本地推理引擎，请在「设置 → 模型 → 本地模型」中下载引擎，或在「模型 API」中开启云端模型。".into(),
            });
        }
        if !std::path::Path::new(&cfg.model_path).exists() {
            return Err(LlmError::Other {
                message: "尚未下载本地模型，请在「设置 → 模型 → 本地模型」中下载推荐模型，或在「模型 API」中开启云端模型。".into(),
            });
        }

        let engine_dir = server.parent().unwrap_or(std::path::Path::new("."));
        let hw = crate::engine::detect_hardware();

        // 跨硬件保护：在 AMD / Intel / 纯 CPU 机器上自动屏蔽残留的 ggml-cuda.dll，防止因缺少 nvcuda.dll 导致引擎启动失败
        let cuda_dll = engine_dir.join("ggml-cuda.dll");
        let cuda_bak = engine_dir.join("ggml-cuda.dll.disabled");
        if hw.gpu_vendor != "nvidia" && cuda_dll.exists() {
            let _ = std::fs::rename(&cuda_dll, &cuda_bak);
        } else if hw.gpu_vendor == "nvidia" && !cuda_dll.exists() && cuda_bak.exists() {
            let _ = std::fs::rename(&cuda_bak, &cuda_dll);
        }

        let has_cuda = hw.gpu_vendor == "nvidia" && cuda_dll.exists() && hw.vram_gb >= 1.8;
        let has_vulkan = (hw.gpu_vendor == "amd" || hw.gpu_vendor == "intel")
            && engine_dir.join("ggml-vulkan.dll").exists()
            && hw.vram_gb >= 1.8;
        let has_metal = hw.gpu_vendor == "apple";

        let ngl = if has_cuda || has_vulkan || has_metal {
            cfg.ngl.clamp(0, 999).to_string()
        } else {
            "0".to_string()
        };

        let mut args: Vec<String> = vec![
            "-m".into(), cfg.model_path.clone(),
            "-ngl".into(), ngl,
            "-c".into(), cfg.ctx.max(2048).to_string(),
            "-np".into(), "1".into(),
            "--host".into(), "127.0.0.1".into(),
            "--port".into(), cfg.port.to_string(),
            "--no-webui".into(),
        ];

        // 根据该模型配置的 KV Cache 量化精度传参（默认 q8_0）
        let kv_q = cfg.kv_quant.trim().to_lowercase();
        if (has_cuda || has_vulkan || has_metal) && (kv_q == "q8_0" || kv_q == "q4_0") {
            args.extend(["-ctk".into(), kv_q.clone(), "-ctv".into(), kv_q]);
        }

        if !cfg.mmproj_path.is_empty() && std::path::Path::new(&cfg.mmproj_path).exists() {
            args.push("--mmproj".into());
            args.push(cfg.mmproj_path.clone());
        }
        args.extend(cfg.extra_args.split_whitespace().map(String::from));

        let log_path = app.path().app_log_dir().unwrap_or_default().join("engine.log");
        let _ = std::fs::create_dir_all(log_path.parent().unwrap());
        let log = std::fs::File::create(&log_path)?;

        let mut cmd = Command::new(server);
        cmd.args(&args)
            .current_dir(engine_dir)
            .stdin(std::process::Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .kill_on_drop(true);
        #[cfg(target_os = "macos")]
        {
            cmd.env("DYLD_LIBRARY_PATH", engine_dir)
                .env("GGML_METAL_PATH_RESOURCES", engine_dir);
        }
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

        let child = cmd.spawn().map_err(|e| format!("启动本地推理引擎失败：{e}"))?;
        if let Some(pid) = child.id() {
            crate::jobobj::attach(pid);
        }
        *self.child.lock().await = Some(child);

        // 等待健康检查通过
        let url = format!("http://127.0.0.1:{}/health", cfg.port);
        let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(2)).build()?;
        let start = Instant::now();
        loop {
            if !self.is_alive().await {
                let tail = std::fs::read_to_string(&log_path).unwrap_or_default();
                let tail: String = tail.lines().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
                return Err(LlmError::Other { message: format!("本地推理引擎意外退出：\n{tail}") });
            }
            if let Ok(r) = client.get(&url).send().await {
                if r.status().is_success() {
                    return Ok(());
                }
            }
            if start.elapsed() > Duration::from_secs(120) {
                return Err(LlmError::Other { message: "加载模型超时（120 秒）".into() });
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }

    async fn is_alive(&self) -> bool {
        let mut g = self.child.lock().await;
        match g.as_mut() {
            Some(c) => matches!(c.try_wait(), Ok(None)),
            None => false,
        }
    }

    async fn kill(&self) {
        if let Some(mut c) = self.child.lock().await.take() {
            let _ = c.kill().await;
        }
    }

    /// 释放模型（结束进程，显存全部归还）
    pub async fn release(&self, app: &AppHandle, status: Status, reason: &str) {
        // 取消进行中的请求
        for f in self.cancels.lock().unwrap().values() {
            f.store(true, Ordering::SeqCst);
        }
        self.kill().await;
        self.set_status(app, status, reason);
    }

    /// 后台守护：显存超 15GB 强制释放让道（即使正在使用中） + 闲置释放 + 进程异常检测
    pub fn spawn_watchdog(app: AppHandle) {
        tauri::async_runtime::spawn(async move {
            let mut tick = 0u64;
            loop {
                tokio::time::sleep(Duration::from_secs(2)).await;
                tick += 1;
                let st = app.state::<crate::AppState>();
                let mgr = &st.llm;
                if mgr.status() != Status::Ready {
                    continue;
                }
                let settings = st.settings.get();
                if !mgr.is_alive().await {
                    mgr.set_status(&app, Status::Released, "进程已退出");
                    continue;
                }
                // 检测显存是否达到显卡极限让道阈值（动态适配大显卡，避免 27B 正常占用 16G+ 时被误杀）
                let snap = gpu::snapshot();
                let limit = gpu::yield_threshold_mb(snap.total_mb);
                if snap.available && snap.used_mb >= limit {
                    *mgr.yielded_snapshot.lock().unwrap() = Some(snap);
                    mgr.release(&app, Status::Yielded, &format!("显存占用达到 {limit}MB 上限，已自动释放模型让道")).await;
                    continue;
                }
                let idle = mgr.last_used.lock().unwrap().map(|t| t.elapsed().as_secs()).unwrap_or(0);
                if mgr.active.load(Ordering::SeqCst) == 0 && idle >= settings.llm.idle_secs {
                    mgr.release(&app, Status::Released, "闲置自动释放").await;
                    continue;
                }
                if tick % 5 == 0 {
                    crate::tray::refresh(&app);
                }
            }
        });
    }

    pub fn cancel(&self, id: &str) {
        if let Some(f) = self.cancels.lock().unwrap().get(id) {
            f.store(true, Ordering::SeqCst);
        }
    }

    /// 外部调用（网关）占用模型：持有期间不会被闲置释放
    pub async fn acquire(self: &Arc<Self>, app: &AppHandle, force: bool) -> Result<ActiveGuard, LlmError> {
        self.ensure_ready(app, force).await?;
        self.active.fetch_add(1, Ordering::SeqCst);
        self.touch();
        Ok(ActiveGuard(self.clone()))
    }

    /// 流式对话。messages 为 OpenAI 格式（可含 image_url 的 data URI）
    /// 若设置中开启了 `api_enabled`，则直接请求云端 OpenAI 兼容 API（不启动本地引擎）
    #[inline]
    fn emit_ev(on: Option<&Channel<ChatEvent>>, ev: ChatEvent) {
        if let Some(ch) = on {
            let _ = ch.send(ev);
        }
    }

    /// 供后端静默调用大模型完成推理（如后台知识库队列），不绑定窗口流式通道
    pub async fn chat_headless(
        &self,
        app: &AppHandle,
        messages: serde_json::Value,
        params: serde_json::Value,
    ) -> Result<String, LlmError> {
        let id = format!(
            "headless_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let llm_cfg = app.state::<crate::AppState>().settings.get().llm;
        if llm_cfg.api_enabled {
            let cancel = Arc::new(AtomicBool::new(false));
            self.cancels.lock().unwrap().insert(id.clone(), cancel.clone());
            self.active.fetch_add(1, Ordering::SeqCst);
            let res = self.chat_cloud_inner(&llm_cfg, messages, params, &cancel, None).await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            self.cancels.lock().unwrap().remove(&id);
            return res;
        }

        self.ensure_ready(app, false).await?;
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancels.lock().unwrap().insert(id.clone(), cancel.clone());
        self.active.fetch_add(1, Ordering::SeqCst);
        let res = self.chat_inner(app, messages, params, &cancel, None).await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        self.cancels.lock().unwrap().remove(&id);
        self.touch();
        res
    }

    pub async fn chat(
        &self,
        app: &AppHandle,
        id: String,
        messages: serde_json::Value,
        params: serde_json::Value,
        force: bool,
        on: Channel<ChatEvent>,
    ) -> Result<String, LlmError> {
        let llm_cfg = app.state::<crate::AppState>().settings.get().llm;
        if llm_cfg.api_enabled {
            let _ = on.send(ChatEvent::Loading);
            let cancel = Arc::new(AtomicBool::new(false));
            self.cancels.lock().unwrap().insert(id.clone(), cancel.clone());
            self.active.fetch_add(1, Ordering::SeqCst);
            let res = self.chat_cloud_inner(&llm_cfg, messages, params, &cancel, Some(&on)).await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            self.cancels.lock().unwrap().remove(&id);
            return res;
        }

        if self.status() != Status::Ready {
            let _ = on.send(ChatEvent::Loading);
        }
        self.ensure_ready(app, force).await?;
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancels.lock().unwrap().insert(id.clone(), cancel.clone());
        self.active.fetch_add(1, Ordering::SeqCst);
        let res = self.chat_inner(app, messages, params, &cancel, Some(&on)).await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        self.cancels.lock().unwrap().remove(&id);
        self.touch();
        res
    }

    async fn chat_cloud_inner(
        &self,
        llm_cfg: &LlmSettings,
        messages: serde_json::Value,
        params: serde_json::Value,
        cancel: &AtomicBool,
        on: Option<&Channel<ChatEvent>>,
    ) -> Result<String, LlmError> {
        let base_url = llm_cfg.api_base_url.trim();
        if base_url.is_empty() {
            return Err(LlmError::Other {
                message: "尚未配置云端 API 地址（Base URL），请在「设置 → 模型 → 模型 API」中填写。".into(),
            });
        }
        let model = llm_cfg.api_model.trim();
        if model.is_empty() {
            return Err(LlmError::Other {
                message: "尚未配置云端模型名称，请在「设置 → 模型 → 模型 API」中填写或选择模型。".into(),
            });
        }

        let endpoint = resolve_chat_completions_url(base_url);
        let has_images = messages_have_images(&messages);
        let model_lower = model.to_lowercase();
        let likely_text_only = model_lower.contains("deepseek-chat")
            || model_lower.contains("deepseek-reasoner")
            || model_lower.contains("deepseek-v3")
            || model_lower.contains("deepseek-r1")
            || model_lower.contains("qwq");

        let final_messages = if has_images && (!llm_cfg.api_vision || likely_text_only) {
            convert_vision_messages_via_ocr(messages.clone()).await
        } else {
            messages.clone()
        };

        let temperature = params
            .get("temperature")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.7);
        let raw_max_tokens = params
            .get("max_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(4096);
        let max_tokens: u64 = if raw_max_tokens <= 0 { 4096 } else { raw_max_tokens as u64 };

        let mut body = serde_json::json!({
            "model": model,
            "messages": final_messages,
            "stream": true,
            "temperature": temperature,
            "max_tokens": max_tokens,
        });

        // 仅当目标平台支持 enable_thinking（如硅基流动/百炼 Qwen3）且用户主动开启思考时传递
        if let Some(enable_thinking) = params
            .pointer("/chat_template_kwargs/enable_thinking")
            .and_then(|v| v.as_bool())
        {
            if enable_thinking && (endpoint.contains("siliconflow") || endpoint.contains("dashscope")) {
                body["enable_thinking"] = serde_json::Value::Bool(true);
            }
        }

        let client = build_http_client(&endpoint, 120)?;
        let mut req = client
            .post(&endpoint)
            .header("Content-Type", "application/json");
        let api_key = llm_cfg.api_key.trim();
        if !api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {api_key}"));
        }

        let mut resp = req
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::Other {
                message: format!("连接云端 API 失败（{endpoint}）：{e}"),
            })?;

        // 若开启了视觉但所选云端模型实际不支持 image_url 返回 400，自动通过本地离线 OCR 提取图片文字重试一次
        if !resp.status().is_success() && has_images && llm_cfg.api_vision && !likely_text_only {
            let status_code = resp.status();
            let err_txt = resp.text().await.unwrap_or_default();
            let err_lower = err_txt.to_lowercase();
            if status_code.as_u16() == 400
                && (err_lower.contains("image")
                    || err_lower.contains("vision")
                    || err_lower.contains("multimodal")
                    || err_lower.contains("content")
                    || err_lower.contains("deserialize")
                    || err_lower.contains("invalid"))
            {
                let ocr_msgs = convert_vision_messages_via_ocr(messages).await;
                body["messages"] = ocr_msgs;
                let mut retry_req = client
                    .post(&endpoint)
                    .header("Content-Type", "application/json");
                if !api_key.is_empty() {
                    retry_req = retry_req.header("Authorization", format!("Bearer {api_key}"));
                }
                resp = retry_req.json(&body).send().await.map_err(|e| LlmError::Other {
                    message: format!("连接云端 API 失败：{e}"),
                })?;
            } else {
                return Err(LlmError::Other {
                    message: format_cloud_error(status_code.as_u16(), &err_txt),
                });
            }
        }

        if !resp.status().is_success() {
            let code = resp.status().as_u16();
            let err_txt = resp.text().await.unwrap_or_default();
            return Err(LlmError::Other {
                message: format_cloud_error(code, &err_txt),
            });
        }

        let req_start = Instant::now();
        let mut gen_start: Option<Instant> = None;
        let mut chunk_tokens = 0u64;
        let mut usage_tokens = 0u64;

        Self::emit_ev(on, ChatEvent::Started);
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        let mut full = String::new();
        let mut in_reasoning = false;
        while let Some(chunk) = stream.next().await {
            if cancel.load(Ordering::SeqCst) {
                return Err(LlmError::Cancelled);
            }
            let bytes = chunk.map_err(|e| LlmError::Other {
                message: format!("云端 API 流式读取中断：{e}"),
            })?;
            buf.push_str(&String::from_utf8_lossy(&bytes));
            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].trim().to_string();
                buf.drain(..=pos);
                if line.is_empty() {
                    continue;
                }
                let Some(data) = line.strip_prefix("data:") else {
                    // 兼容部分网关直接返回非 SSE JSON 的情况
                    if line.starts_with('{') && full.is_empty() {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                            if let Some(c) = v.pointer("/choices/0/message/content").and_then(|x| x.as_str()) {
                                full.push_str(c);
                                chunk_tokens += (c.chars().count() as u64).max(1);
                                Self::emit_ev(on, ChatEvent::Delta { text: c.to_string() });
                            }
                            if let Some(ut) = v.pointer("/usage/completion_tokens").and_then(|x| x.as_u64()) {
                                usage_tokens = ut;
                            }
                        }
                    }
                    continue;
                };
                let data = data.trim();
                if data == "[DONE]" {
                    if in_reasoning {
                        full.push_str("</think>\n\n");
                        Self::emit_ev(on, ChatEvent::Delta { text: "</think>\n\n".to_string() });
                    }
                    let duration_ms = req_start.elapsed().as_millis() as u64;
                    let gen_secs = gen_start
                        .map(|t| t.elapsed().as_secs_f64())
                        .unwrap_or_else(|| req_start.elapsed().as_secs_f64())
                        .max(0.05);
                    let tokens = if usage_tokens > 0 { usage_tokens } else { chunk_tokens.max(1) };
                    let tokens_per_sec = ((tokens as f64 / gen_secs) * 10.0).round() / 10.0;
                    Self::emit_ev(on, ChatEvent::Stats { tokens, tokens_per_sec, duration_ms });
                    return Ok(full);
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(ut) = v.pointer("/usage/completion_tokens").and_then(|x| x.as_u64()) {
                        if ut > 0 {
                            usage_tokens = ut;
                        }
                    }
                    let delta = &v["choices"][0]["delta"];
                    let mut got_token = false;
                    if let Some(r) = delta["reasoning_content"].as_str() {
                        if !r.is_empty() {
                            got_token = true;
                            if !in_reasoning {
                                in_reasoning = true;
                                full.push_str("<think>");
                                Self::emit_ev(on, ChatEvent::Delta { text: "<think>".to_string() });
                            }
                            full.push_str(r);
                            Self::emit_ev(on, ChatEvent::Delta { text: r.to_string() });
                        }
                    }
                    if let Some(t) = delta["content"].as_str() {
                        if !t.is_empty() {
                            got_token = true;
                            if in_reasoning {
                                in_reasoning = false;
                                full.push_str("</think>\n\n");
                                Self::emit_ev(on, ChatEvent::Delta { text: "</think>\n\n".to_string() });
                            }
                            full.push_str(t);
                            Self::emit_ev(on, ChatEvent::Delta { text: t.to_string() });
                        }
                    }
                    if got_token {
                        if gen_start.is_none() {
                            gen_start = Some(Instant::now());
                        }
                        chunk_tokens += 1;
                        let duration_ms = req_start.elapsed().as_millis() as u64;
                        let gen_secs = gen_start.unwrap().elapsed().as_secs_f64().max(0.05);
                        let tokens = if usage_tokens > 0 { usage_tokens } else { chunk_tokens };
                        let tokens_per_sec = ((tokens as f64 / gen_secs) * 10.0).round() / 10.0;
                        Self::emit_ev(on, ChatEvent::Stats { tokens, tokens_per_sec, duration_ms });
                    }
                }
            }
        }
        if in_reasoning {
            full.push_str("</think>\n\n");
            Self::emit_ev(on, ChatEvent::Delta { text: "</think>\n\n".to_string() });
        }
        let duration_ms = req_start.elapsed().as_millis() as u64;
        let gen_secs = gen_start
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or_else(|| req_start.elapsed().as_secs_f64())
            .max(0.05);
        let tokens = if usage_tokens > 0 { usage_tokens } else { chunk_tokens.max(1) };
        let tokens_per_sec = ((tokens as f64 / gen_secs) * 10.0).round() / 10.0;
        Self::emit_ev(on, ChatEvent::Stats { tokens, tokens_per_sec, duration_ms });
        Ok(full)
    }

    async fn chat_inner(
        &self,
        app: &AppHandle,
        messages: serde_json::Value,
        params: serde_json::Value,
        cancel: &AtomicBool,
        on: Option<&Channel<ChatEvent>>,
    ) -> Result<String, LlmError> {
        let llm_cfg = app.state::<crate::AppState>().settings.get().llm;
        let port = llm_cfg.port;
        let required_mb = llm_cfg.required_vram_mb;
        let req_start = Instant::now();
        let mut gen_start: Option<Instant> = None;
        let mut chunk_tokens = 0u64;
        let mut server_tokens = 0u64;
        let mut server_tps = 0.0f64;

        let has_mmproj = !llm_cfg.mmproj_path.trim().is_empty()
            && std::path::Path::new(&llm_cfg.mmproj_path).exists();
        let final_messages = if !has_mmproj && messages_have_images(&messages) {
            convert_vision_messages_via_ocr(messages).await
        } else {
            messages
        };

        let def_effort = crate::settings::normalize_reasoning_effort(&llm_cfg.reasoning_effort);
        let def_enable_thinking = def_effort != "off";

        let mut body = serde_json::json!({
            "messages": final_messages,
            "stream": true,
            "cache_prompt": true,
            "timings_per_token": true,
            "temperature": 0.7,
            "top_p": 0.95,
            "top_k": 40,
            "min_p": 0.05,
            "max_tokens": -1,
            "reasoning_format": "auto",
            "reasoning_control": true,
            "chat_template_kwargs": { "enable_thinking": def_enable_thinking },
        });
        if def_enable_thinking {
            body["reasoning_effort"] = serde_json::Value::String(def_effort);
        }
        if let (Some(b), Some(p)) = (body.as_object_mut(), params.as_object()) {
            for (k, v) in p {
                b.insert(k.clone(), v.clone());
            }
        }
        let endpoint = format!("http://127.0.0.1:{port}/v1/chat/completions");
        let client = reqwest::Client::builder().no_proxy().build()?;
        let mut resp = match client
            .post(&endpoint)
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                if let Some(ye) = self.yield_error(required_mb) {
                    return Err(ye);
                }
                return Err(e.into());
            }
        };

        // 兼容保护：若自定义模型的 Jinja 聊天模板不支持特定思考等级（如没有 high/max）或不支持思考参数返回 400/500，自动降级重试
        if !resp.status().is_success() {
            if let Some(ye) = self.yield_error(required_mb) {
                return Err(ye);
            }
            // 第一级回退：移除 reasoning_effort（保留 enable_thinking 与 budget）重试
            if body.get("reasoning_effort").is_some() {
                if let Some(obj) = body.as_object_mut() {
                    obj.remove("reasoning_effort");
                }
                if let Ok(r2) = client.post(&endpoint).json(&body).send().await {
                    resp = r2;
                }
            }
            // 第二级回退：若仍失败，移除所有思考模板扩展字段，以纯标准 OpenAI 请求重试
            if !resp.status().is_success() {
                if let Some(obj) = body.as_object_mut() {
                    obj.remove("reasoning_effort");
                    obj.remove("chat_template_kwargs");
                    obj.remove("thinking_budget_tokens");
                    obj.remove("reasoning_budget_tokens");
                    obj.remove("reasoning_control");
                    obj.remove("reasoning_format");
                }
                if let Ok(r3) = client.post(&endpoint).json(&body).send().await {
                    resp = r3;
                }
            }
        }

        if !resp.status().is_success() {
            if let Some(ye) = self.yield_error(required_mb) {
                return Err(ye);
            }
            let t = resp.text().await.unwrap_or_default();
            return Err(LlmError::Other { message: format!("推理服务返回错误：{t}") });
        }
        Self::emit_ev(on, ChatEvent::Started);
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        let mut full = String::new();
        let mut in_reasoning = false;
        while let Some(chunk) = stream.next().await {
            if cancel.load(Ordering::SeqCst) {
                if let Some(ye) = self.yield_error(required_mb) {
                    return Err(ye);
                }
                let duration_ms = req_start.elapsed().as_millis() as u64;
                let tokens = if server_tokens > 0 { server_tokens } else { chunk_tokens };
                if tokens > 0 {
                    let tokens_per_sec = if server_tps > 0.0 {
                        (server_tps * 10.0).round() / 10.0
                    } else {
                        let gen_secs = gen_start.map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.1).max(0.05);
                        ((tokens as f64 / gen_secs) * 10.0).round() / 10.0
                    };
                    Self::emit_ev(on, ChatEvent::Stats { tokens, tokens_per_sec, duration_ms });
                }
                return Err(LlmError::Cancelled);
            }
            let bytes = match chunk {
                Ok(b) => b,
                Err(e) => {
                    if let Some(ye) = self.yield_error(required_mb) {
                        return Err(ye);
                    }
                    return Err(e.into());
                }
            };
            buf.push_str(&String::from_utf8_lossy(&bytes));
            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].trim().to_string();
                buf.drain(..=pos);
                let Some(data) = line.strip_prefix("data:") else { continue };
                let data = data.trim();
                if data == "[DONE]" {
                    if in_reasoning {
                        full.push_str("</think>\n\n");
                        Self::emit_ev(on, ChatEvent::Delta { text: "</think>\n\n".to_string() });
                    }
                    let duration_ms = req_start.elapsed().as_millis() as u64;
                    let tokens = if server_tokens > 0 { server_tokens } else { chunk_tokens.max(1) };
                    let tokens_per_sec = if server_tps > 0.0 {
                        (server_tps * 10.0).round() / 10.0
                    } else {
                        let gen_secs = gen_start
                            .map(|t| t.elapsed().as_secs_f64())
                            .unwrap_or_else(|| req_start.elapsed().as_secs_f64())
                            .max(0.05);
                        ((tokens as f64 / gen_secs) * 10.0).round() / 10.0
                    };
                    Self::emit_ev(on, ChatEvent::Stats { tokens, tokens_per_sec, duration_ms });
                    return Ok(full);
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                    // 提取 llama-server 原生 timings 统计
                    if let Some(timings) = v.get("timings") {
                        if let Some(pn) = timings.get("predicted_n").and_then(|x| x.as_u64()) {
                            if pn > 0 {
                                server_tokens = pn;
                            }
                        }
                        if let Some(pps) = timings.get("predicted_per_second").and_then(|x| x.as_f64()) {
                            if pps > 0.0 {
                                server_tps = pps;
                            }
                        } else if let (Some(pn), Some(pms)) = (
                            timings.get("predicted_n").and_then(|x| x.as_f64()),
                            timings.get("predicted_ms").and_then(|x| x.as_f64()),
                        ) {
                            if pn > 0.0 && pms > 0.0 {
                                server_tps = pn * 1000.0 / pms;
                            }
                        }
                    } else if let Some(ut) = v.pointer("/usage/completion_tokens").and_then(|x| x.as_u64()) {
                        if ut > 0 {
                            server_tokens = ut;
                        }
                    }

                    let delta = &v["choices"][0]["delta"];
                    let mut got_token = false;
                    if let Some(r) = delta["reasoning_content"].as_str() {
                        if !r.is_empty() {
                            got_token = true;
                            if !in_reasoning {
                                in_reasoning = true;
                                full.push_str("<think>");
                                Self::emit_ev(on, ChatEvent::Delta { text: "<think>".to_string() });
                            }
                            full.push_str(r);
                            Self::emit_ev(on, ChatEvent::Delta { text: r.to_string() });
                        }
                    }
                    if let Some(t) = delta["content"].as_str() {
                        if !t.is_empty() {
                            got_token = true;
                            if in_reasoning {
                                in_reasoning = false;
                                full.push_str("</think>\n\n");
                                Self::emit_ev(on, ChatEvent::Delta { text: "</think>\n\n".to_string() });
                            }
                            full.push_str(t);
                            Self::emit_ev(on, ChatEvent::Delta { text: t.to_string() });
                        }
                    }
                    if got_token || server_tokens > 0 {
                        if got_token {
                            if gen_start.is_none() {
                                gen_start = Some(Instant::now());
                            }
                            chunk_tokens += 1;
                        }
                        let duration_ms = req_start.elapsed().as_millis() as u64;
                        let tokens = if server_tokens > 0 { server_tokens } else { chunk_tokens };
                        let tokens_per_sec = if server_tps > 0.0 {
                            (server_tps * 10.0).round() / 10.0
                        } else {
                            let gen_secs = gen_start
                                .map(|t| t.elapsed().as_secs_f64())
                                .unwrap_or_else(|| req_start.elapsed().as_secs_f64())
                                .max(0.05);
                            ((tokens as f64 / gen_secs) * 10.0).round() / 10.0
                        };
                        Self::emit_ev(on, ChatEvent::Stats { tokens, tokens_per_sec, duration_ms });
                    }
                }
            }
            self.touch();
        }
        // 流在未收到 [DONE] 时提前断开（如 llama-server 因显存超 15GB 被 kill）
        if let Some(ye) = self.yield_error(required_mb) {
            return Err(ye);
        }
        if in_reasoning {
            full.push_str("</think>\n\n");
            Self::emit_ev(on, ChatEvent::Delta { text: "</think>\n\n".to_string() });
        }
        let duration_ms = req_start.elapsed().as_millis() as u64;
        let tokens = if server_tokens > 0 { server_tokens } else { chunk_tokens.max(1) };
        let tokens_per_sec = if server_tps > 0.0 {
            (server_tps * 10.0).round() / 10.0
        } else {
            let gen_secs = gen_start
                .map(|t| t.elapsed().as_secs_f64())
                .unwrap_or_else(|| req_start.elapsed().as_secs_f64())
                .max(0.05);
            ((tokens as f64 / gen_secs) * 10.0).round() / 10.0
        };
        Self::emit_ev(on, ChatEvent::Stats { tokens, tokens_per_sec, duration_ms });
        Ok(full)
    }
}

fn format_cloud_error(status: u16, raw: &str) -> String {
    let msg = serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .or_else(|| v.get("message"))
                .or_else(|| v.get("detail"))
                .and_then(|x| x.as_str())
                .map(String::from)
        })
        .unwrap_or_else(|| raw.chars().take(240).collect());

    match status {
        401 => format!("云端 API 鉴权失败 (HTTP 401)：API Key 无效或未填写。详情：{msg}"),
        402 => format!("云端 API 账户余额不足 (HTTP 402)：{msg}"),
        403 => format!("云端 API 权限被拒绝 (HTTP 403)：{msg}"),
        404 => format!("云端 API 地址或模型名称不存在 (HTTP 404)：请检查 Base URL 与模型名称。详情：{msg}"),
        429 => format!("云端 API 请求过于频繁或超出额度限制 (HTTP 429)：{msg}"),
        _ => format!("云端 API 返回错误 (HTTP {status})：{msg}"),
    }
}

/// 测试云端 OpenAI 兼容接口连通性
pub async fn test_cloud_api(base_url: &str, api_key: &str, model: &str) -> Result<CloudTestResult, String> {
    let base_url = base_url.trim();
    if base_url.is_empty() {
        return Err("请先填写接口地址（Base URL）".into());
    }
    let model = model.trim();
    if model.is_empty() {
        return Err("请先填写或选择模型名称".into());
    }
    let endpoint = resolve_chat_completions_url(base_url);
    let client = build_http_client(&endpoint, 25).map_err(|e| e.to_string())?;
    let body = serde_json::json!({
        "model": model,
        "messages": [
            { "role": "user", "content": "请简短回复：连接成功" }
        ],
        "max_tokens": 24,
        "temperature": 0.1,
        "stream": false
    });
    let mut req = client.post(&endpoint).header("Content-Type", "application/json");
    let key = api_key.trim();
    if !key.is_empty() {
        req = req.header("Authorization", format!("Bearer {key}"));
    }
    let start = Instant::now();
    let resp = req
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("无法连接到 {endpoint}：{e}"))?;
    let latency_ms = start.elapsed().as_millis() as u64;
    let status = resp.status();
    let txt = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format_cloud_error(status.as_u16(), &txt));
    }
    let v: serde_json::Value = serde_json::from_str(&txt).unwrap_or_default();
    let reply = v
        .pointer("/choices/0/message/content")
        .and_then(|x| x.as_str())
        .or_else(|| v.pointer("/choices/0/message/reasoning_content").and_then(|x| x.as_str()))
        .unwrap_or("OK")
        .trim()
        .chars()
        .take(80)
        .collect::<String>();
    let actual_model = v
        .get("model")
        .and_then(|x| x.as_str())
        .unwrap_or(model)
        .to_string();
    Ok(CloudTestResult {
        ok: true,
        latency_ms,
        model: actual_model,
        reply: if reply.is_empty() { "OK".into() } else { reply },
    })
}

/// 从云端 OpenAI 兼容接口 `/models` 拉取可用模型 ID 列表
pub async fn fetch_cloud_models(base_url: &str, api_key: &str) -> Result<Vec<String>, String> {
    let base_url = base_url.trim();
    if base_url.is_empty() {
        return Err("请先填写接口地址（Base URL）".into());
    }
    let endpoint = resolve_models_url(base_url);
    let client = build_http_client(&endpoint, 15).map_err(|e| e.to_string())?;
    let mut req = client.get(&endpoint);
    let key = api_key.trim();
    if !key.is_empty() {
        req = req.header("Authorization", format!("Bearer {key}"));
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("请求模型列表失败（{endpoint}）：{e}"))?;
    let status = resp.status();
    let txt = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format_cloud_error(status.as_u16(), &txt));
    }
    let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| format!("解析模型列表 JSON 失败：{e}"))?;
    let mut list: Vec<String> = Vec::new();
    if let Some(arr) = v.get("data").and_then(|d| d.as_array()).or_else(|| v.as_array()) {
        for item in arr {
            if let Some(id) = item.get("id").and_then(|x| x.as_str()) {
                let id_trim = id.trim();
                if !id_trim.is_empty() {
                    list.push(id_trim.to_string());
                }
            }
        }
    }
    if list.is_empty() {
        return Err("接口未返回可用模型列表，请直接手动输入模型名称".into());
    }
    Ok(list)
}
