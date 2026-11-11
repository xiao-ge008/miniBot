//! 知识库后台异步串行处理队列：
//! 点击「存入知识库」后窗口可立即消失，识别、提纯与入库由后端 FIFO 队列逐一完成。
//! 上限 10 项（含当前处理中 + 排队等待中），超出时友好提示用户稍等片刻。

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

pub const MAX_KB_QUEUE: usize = 10;
pub const KB_QUEUE_EVENT: &str = "minibot://kb-queue";
const MAX_RECENT_HISTORY: usize = 8;

#[derive(Debug, Clone, Deserialize)]
pub struct KbQueueInput {
    /// "image" | "clip" | "web"
    pub kind: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub image_data: Option<String>,
    #[serde(default)]
    pub existing_ocr: Option<String>,
    #[serde(default)]
    pub existing_vision: Option<String>,
    #[serde(default)]
    pub source_app: Option<String>,
    #[serde(default)]
    pub force_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KbQueueTaskInfo {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub source_app: Option<String>,
    /// "queued" | "ocr" | "classify" | "extract" | "distill" | "saving" | "done" | "error"
    pub stage: String,
    pub stage_label: String,
    pub detected_type: Option<String>,
    pub saved_path: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KbQueueStatus {
    pub max_size: usize,
    pub active: Option<KbQueueTaskInfo>,
    pub pending: Vec<KbQueueTaskInfo>,
    pub active_count: usize,
    pub pending_count: usize,
    pub total_in_flight: usize,
    pub completed_count: u64,
    pub failed_count: u64,
    pub recent: Vec<KbQueueTaskInfo>,
    pub last_notice: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct KbEnqueueResult {
    pub accepted: bool,
    pub task_id: Option<String>,
    pub position: usize,
    pub in_flight: usize,
    pub max_size: usize,
    pub message: String,
    pub status: KbQueueStatus,
}

#[derive(Debug, Clone)]
struct QueuedJob {
    info: KbQueueTaskInfo,
    input: KbQueueInput,
}

#[derive(Default)]
struct QueueInner {
    next_seq: u64,
    active: Option<KbQueueTaskInfo>,
    queue: VecDeque<QueuedJob>,
    recent: VecDeque<KbQueueTaskInfo>,
    completed_count: u64,
    failed_count: u64,
    last_notice: Option<String>,
}

impl QueueInner {
    fn in_flight(&self) -> usize {
        usize::from(self.active.is_some()) + self.queue.len()
    }

    fn snapshot(&self) -> KbQueueStatus {
        let active_count = usize::from(self.active.is_some());
        let pending_count = self.queue.len();
        KbQueueStatus {
            max_size: MAX_KB_QUEUE,
            active: self.active.clone(),
            pending: self.queue.iter().map(|j| j.info.clone()).collect(),
            active_count,
            pending_count,
            total_in_flight: active_count + pending_count,
            completed_count: self.completed_count,
            failed_count: self.failed_count,
            recent: self.recent.iter().cloned().collect(),
            last_notice: self.last_notice.clone(),
        }
    }

    fn push_recent(&mut self, item: KbQueueTaskInfo) {
        self.recent.push_front(item);
        while self.recent.len() > MAX_RECENT_HISTORY {
            self.recent.pop_back();
        }
    }
}

pub struct KbQueueManager {
    inner: Mutex<QueueInner>,
    notify: Notify,
}

impl Default for KbQueueManager {
    fn default() -> Self {
        Self {
            inner: Mutex::new(QueueInner::default()),
            notify: Notify::new(),
        }
    }
}

impl KbQueueManager {
    pub fn status(&self) -> KbQueueStatus {
        self.inner.lock().unwrap().snapshot()
    }

    /// 尝试将一个保存任务放入后端 FIFO 队列；若当前进行中+排队数 >= MAX_KB_QUEUE 则友好拒绝。
    pub fn enqueue_raw(&self, input: KbQueueInput) -> KbEnqueueResult {
        let now = chrono::Local::now().timestamp_millis();
        let kind_norm = normalize_kind(&input.kind);
        let title = build_job_title(&kind_norm, &input);

        let mut g = self.inner.lock().unwrap();
        let in_flight = g.in_flight();
        if in_flight >= MAX_KB_QUEUE {
            let msg = format!(
                "后台知识库处理较忙（已有 {in_flight}/{MAX_KB_QUEUE} 项正在排队），请稍等片刻后再继续保存"
            );
            g.last_notice = Some(msg.clone());
            let status = g.snapshot();
            drop(g);
            return KbEnqueueResult {
                accepted: false,
                task_id: None,
                position: 0,
                in_flight,
                max_size: MAX_KB_QUEUE,
                message: msg,
                status,
            };
        }

        g.next_seq += 1;
        let id = format!("kbq_{}_{}", now, g.next_seq);
        let info = KbQueueTaskInfo {
            id: id.clone(),
            kind: kind_norm,
            title: title.clone(),
            source_app: input.source_app.clone(),
            stage: "queued".into(),
            stage_label: if in_flight == 0 {
                "即将开始处理…".into()
            } else {
                format!("排队中（前方还有 {} 项）", in_flight)
            },
            detected_type: None,
            saved_path: None,
            error: None,
            created_at: now,
            finished_at: None,
        };
        g.queue.push_back(QueuedJob {
            info,
            input,
        });
        let new_in_flight = g.in_flight();
        let msg = if new_in_flight == 1 {
            format!("已转入后台识别并保存：{title}")
        } else {
            format!("已加入后台队列（第 {new_in_flight}/{MAX_KB_QUEUE} 位）：{title}")
        };
        g.last_notice = Some(msg.clone());
        let status = g.snapshot();
        drop(g);

        self.notify.notify_one();

        KbEnqueueResult {
            accepted: true,
            task_id: Some(id),
            position: new_in_flight,
            in_flight: new_in_flight,
            max_size: MAX_KB_QUEUE,
            message: msg,
            status,
        }
    }

    pub fn enqueue(&self, app: &AppHandle, input: KbQueueInput) -> KbEnqueueResult {
        let res = self.enqueue_raw(input);
        emit_and_refresh(app, &res.status);
        res
    }

    pub fn spawn_worker(self: &Arc<Self>, app: AppHandle) {
        let mgr = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            loop {
                let next_job = {
                    let mut g = mgr.inner.lock().unwrap();
                    match g.queue.pop_front() {
                        Some(mut job) => {
                            job.info.stage = "starting".into();
                            job.info.stage_label = "正在准备处理…".into();
                            g.active = Some(job.info.clone());
                            // 更新后面排队项的提示文案
                            for (idx, pending) in g.queue.iter_mut().enumerate() {
                                pending.info.stage_label =
                                    format!("排队中（前方还有 {} 项）", idx + 1);
                            }
                            let st = g.snapshot();
                            drop(g);
                            emit_and_refresh(&app, &st);
                            Some(job)
                        }
                        None => None,
                    }
                };

                let Some(job) = next_job else {
                    mgr.notify.notified().await;
                    continue;
                };

                let outcome = process_single_job(&app, &mgr, &job).await;

                {
                    let mut g = mgr.inner.lock().unwrap();
                    let finished = chrono::Local::now().timestamp_millis();
                    let mut done_info = g.active.take().unwrap_or(job.info.clone());
                    done_info.finished_at = Some(finished);
                    match outcome {
                        Ok(res) => {
                            done_info.stage = "done".into();
                            done_info.detected_type = res.detected_type.or(done_info.detected_type);
                            done_info.saved_path = Some(res.saved_path);
                            done_info.stage_label = res.note.unwrap_or_else(|| "已存入知识库".into());
                            g.completed_count += 1;
                            g.last_notice = Some(format!("✓ {} — {}", done_info.title, done_info.stage_label));
                        }
                        Err(err_msg) => {
                            done_info.stage = "error".into();
                            done_info.error = Some(err_msg.clone());
                            done_info.stage_label = format!("处理失败：{err_msg}");
                            g.failed_count += 1;
                            g.last_notice = Some(format!("⚠ {} 入库失败：{err_msg}", done_info.title));
                        }
                    }
                    g.push_recent(done_info);
                    let st = g.snapshot();
                    drop(g);
                    emit_and_refresh(&app, &st);
                }
            }
        });
    }

    fn update_active_stage(
        &self,
        app: &AppHandle,
        stage: &str,
        stage_label: impl Into<String>,
        detected_type: Option<String>,
    ) {
        let mut g = self.inner.lock().unwrap();
        if let Some(active) = g.active.as_mut() {
            active.stage = stage.to_string();
            active.stage_label = stage_label.into();
            if detected_type.is_some() {
                active.detected_type = detected_type;
            }
        }
        let st = g.snapshot();
        drop(g);
        emit_and_refresh(app, &st);
    }
}

struct JobSuccess {
    saved_path: String,
    detected_type: Option<String>,
    note: Option<String>,
}

fn emit_and_refresh(app: &AppHandle, status: &KbQueueStatus) {
    let _ = app.emit(KB_QUEUE_EVENT, status);
    crate::tray::refresh(app);
}

fn normalize_kind(raw: &str) -> String {
    match raw.trim().to_lowercase().as_str() {
        "image" => "image".into(),
        "url" | "web" => "web".into(),
        _ => "clip".into(),
    }
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    let clean = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = clean.chars().take(max_chars).collect();
    if clean.chars().count() > max_chars {
        out.push('…');
    }
    out
}

fn build_job_title(kind: &str, input: &KbQueueInput) -> String {
    if let Some(t) = input.title.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        return truncate_chars(t, 28);
    }
    match kind {
        "image" => {
            if let Some(hint) = input
                .existing_vision
                .as_deref()
                .map(clean_think_tags)
                .filter(|s| !s.is_empty())
            {
                return format!("🖼️ {}", truncate_chars(&hint, 22));
            }
            if let Some(app) = input.source_app.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                return format!("🖼️ 图片 ({})", truncate_chars(app, 14));
            }
            "🖼️ 图片识别入库".into()
        }
        "web" => {
            let u = input.url.as_deref().or(input.text.as_deref()).unwrap_or("网页");
            format!("🌐 {}", truncate_chars(u.trim(), 24))
        }
        _ => {
            let txt = input.text.as_deref().unwrap_or("文本片段");
            format!("📝 {}", truncate_chars(txt.trim(), 24))
        }
    }
}

async fn process_single_job(
    app: &AppHandle,
    mgr: &KbQueueManager,
    job: &QueuedJob,
) -> Result<JobSuccess, String> {
    match job.info.kind.as_str() {
        "image" => process_image_job(app, mgr, &job.input).await,
        "web" => process_text_or_web_job(app, mgr, &job.input, true).await,
        _ => process_text_or_web_job(app, mgr, &job.input, false).await,
    }
}

fn format_llm_error(err: &crate::llm::LlmError) -> String {
    match err {
        crate::llm::LlmError::GpuBusy { required_mb, .. } => {
            format!("显存不足（需 {required_mb} MB）")
        }
        crate::llm::LlmError::PortBusy { port } => format!("本地模型端口 {port} 被占用"),
        crate::llm::LlmError::Cancelled => "请求已取消".into(),
        crate::llm::LlmError::Other { message } => message.clone(),
    }
}

async fn process_image_job(
    app: &AppHandle,
    mgr: &KbQueueManager,
    input: &KbQueueInput,
) -> Result<JobSuccess, String> {
    let image_data = input
        .image_data
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "缺少图片数据".to_string())?;

    // 1. OCR 文字提取（若尚未提供 existing_ocr）
    let mut ocr_text = input.existing_ocr.clone().unwrap_or_default();
    if ocr_text.trim().is_empty() {
        mgr.update_active_stage(app, "ocr", "正在提取图片文字 (OCR)…", None);
        let img_owned = image_data.to_string();
        if let Ok(Ok(res)) = tokio::task::spawn_blocking(move || -> Result<String, String> {
            let bytes = crate::imgutil::decode_b64(&img_owned).map_err(|e| e.to_string())?;
            let rgba = crate::imgutil::decode_rgba(&bytes).map_err(|e| e.to_string())?;
            let ocr_res = crate::ocr::recognize(&rgba).map_err(|e| e.to_string())?;
            Ok(ocr_res.text)
        })
        .await
        {
            ocr_text = res;
        }
    }

    let pure_existing_vision = input
        .existing_vision
        .as_deref()
        .map(clean_think_tags)
        .unwrap_or_default();

    // 2. 阶段一：加载 vision/classifier.md 快筛图片类型（人物统一归入唯一的 character，其余风景/界面/静物/图表按各自模板解析）
    let mut detected_type = input
        .force_type
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "auto")
        .unwrap_or("")
        .to_string();

    let st = app.state::<crate::AppState>();
    let mut ai_warning: Option<String> = None;

    if detected_type.is_empty() {
        detected_type = "general".into();
        mgr.update_active_stage(app, "classify", "正在判定图片类别…", None);
        let custom_ids: Vec<String> = crate::kb::list_prompt_templates(app)
            .unwrap_or_default()
            .into_iter()
            .filter(|p| p.category == "vision" && !p.is_builtin)
            .map(|p| p.id)
            .collect();

        if let Ok(cls_tpl) = crate::kb::get_prompt_template(app, "vision", "classifier") {
            let messages = serde_json::json!([
                {
                    "role": "user",
                    "content": [
                        { "type": "image_url", "image_url": { "url": image_data } },
                        { "type": "text", "text": cls_tpl.body }
                    ]
                }
            ]);
            // 快筛只需极简分类标签，强制禁用长思考防止输出耗尽导致误判
            let params = serde_json::json!({
                "max_tokens": cls_tpl.max_tokens.max(128),
                "temperature": cls_tpl.temperature,
                "reasoning_effort": "off",
                "chat_template_kwargs": { "enable_thinking": false }
            });
            match st.llm.chat_headless(app, messages, params).await {
                Ok(raw) => {
                    detected_type = resolve_vision_category(&raw, &custom_ids);
                    crate::log_msg(
                        app,
                        format!(
                            "[kb-queue] 图像快筛: tpl={} | raw={:?} -> category={}",
                            cls_tpl.path,
                            truncate_chars(&raw, 120),
                            detected_type
                        ),
                    );
                }
                Err(e) => {
                    let msg = format_llm_error(&e);
                    crate::log_msg(app, format!("[kb-queue] 图像快筛失败({msg})，归类 general"));
                    ai_warning = Some(format!("分类跳过（{msg}），已按通用模板解析"));
                }
            }
        }
    }

    // 3. 阶段二：实时从磁盘加载对应类别的 {detected_type}.md 提取特征（不做多模板交叉兜底替换）
    let target_tpl = crate::kb::get_prompt_template(app, "vision", &detected_type);

    let mut ai_vision_analysis = String::new();
    let mut category_title = "综合视觉特征解析".to_string();
    let mut used_tpl_rel = String::new();

    match target_tpl {
        Ok(tpl) => {
            category_title = tpl.title.clone();
            used_tpl_rel = format!("vision/{}.md", tpl.id);
            crate::log_msg(
                app,
                format!(
                    "[kb-queue] 注入视觉提示词: id={} | path={} | body_chars={} | mtime={}",
                    tpl.id,
                    tpl.path,
                    tpl.body.chars().count(),
                    tpl.modified_ms
                ),
            );
            mgr.update_active_stage(
                app,
                "extract",
                format!("正在深度识别「{}」…", tpl.title),
                Some(detected_type.clone()),
            );
            let specialized_prompt = render_vision_prompt(&tpl.body, &pure_existing_vision, &ocr_text);
            let messages = serde_json::json!([
                {
                    "role": "user",
                    "content": [
                        { "type": "image_url", "image_url": { "url": image_data } },
                        { "type": "text", "text": specialized_prompt }
                    ]
                }
            ]);
            // 结构化视觉提取禁用长思考，将全部 max_tokens 预算留给完整结构与关键词输出，防止写到一半被截断
            let params = serde_json::json!({
                "max_tokens": if tpl.max_tokens > 0 { tpl.max_tokens.max(3800) } else { 3800 },
                "temperature": tpl.temperature,
                "reasoning_effort": "off",
                "chat_template_kwargs": { "enable_thinking": false }
            });
            match st.llm.chat_headless(app, messages, params).await {
                Ok(raw) => {
                    ai_vision_analysis = clean_think_tags(&raw);
                    if ai_vision_analysis.is_empty() {
                        ai_vision_analysis = raw.trim().to_string();
                    }
                }
                Err(e) => {
                    let msg = format_llm_error(&e);
                    crate::log_msg(app, format!("[kb-queue] 图像解析异常: {msg}"));
                    ai_warning = Some(format!("AI 解析跳过（{msg}），已保留原图与 OCR"));
                }
            }
        }
        Err(e) => {
            crate::log_msg(app, format!("[kb-queue] 读取视觉提示词 vision/{detected_type}.md 失败: {e}"));
            ai_warning = Some(format!("视觉提示词 vision/{detected_type}.md 读取失败（{e}），未执行 AI 解析"));
        }
    }

    let mut final_vision_desc = format!("## {category_title}\n\n");
    if !pure_existing_vision.is_empty() {
        final_vision_desc.push_str(&format!("### 📌 问答/主题摘要\n{pure_existing_vision}\n\n"));
    }
    if !ai_vision_analysis.is_empty() {
        final_vision_desc.push_str(&ai_vision_analysis);
    } else if !pure_existing_vision.is_empty() {
        final_vision_desc.push_str(&pure_existing_vision);
    }
    if !used_tpl_rel.is_empty() {
        final_vision_desc.push_str(&format!("\n\n<!-- prompt_template: {used_tpl_rel} -->"));
    }

    let vision_opt = if ai_vision_analysis.is_empty() && pure_existing_vision.is_empty() {
        None
    } else {
        Some(final_vision_desc.as_str())
    };
    let ocr_opt = if ocr_text.trim().is_empty() {
        None
    } else {
        Some(ocr_text.as_str())
    };

    mgr.update_active_stage(app, "saving", "正在写入原图、解析与索引…", Some(detected_type.clone()));
    let saved_path = crate::kb::save_image(
        app,
        image_data,
        ocr_opt,
        vision_opt,
        input.source_app.as_deref(),
    )
    .await?;
    let kb_root = crate::kb::get_kb_root(app);
    let _ = tokio::task::spawn_blocking(move || crate::kb::sync_kb_index(&kb_root)).await;

    Ok(JobSuccess {
        saved_path: saved_path.to_string_lossy().to_string(),
        detected_type: Some(detected_type),
        note: Some(
            ai_warning.unwrap_or_else(|| format!("已完成「{category_title}」识别并入库")),
        ),
    })
}

async fn process_text_or_web_job(
    app: &AppHandle,
    mgr: &KbQueueManager,
    input: &KbQueueInput,
    is_web: bool,
) -> Result<JobSuccess, String> {
    let raw_text = input.text.as_deref().unwrap_or("").trim().to_string();
    let url_str = input
        .url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let effective_text = if raw_text.is_empty() {
        url_str.clone().unwrap_or_default()
    } else {
        raw_text
    };
    if effective_text.is_empty() {
        return Err("内容为空，无法存入知识库".into());
    }

    let st = app.state::<crate::AppState>();
    let auto_distill = st.settings.get().knowledge.clip_auto_distill;
    let mut distilled_summary: Option<String> = None;
    let mut distill_note: Option<String> = None;

    let clip_tpl_id = if is_web { "web_distill" } else { "clip_distill" };
    if auto_distill && effective_text.chars().count() >= 20 {
        match crate::kb::get_prompt_template(app, "clip", clip_tpl_id) {
            Ok(tpl) => {
                mgr.update_active_stage(
                    app,
                    "distill",
                    format!("正在执行「{}」…", tpl.title),
                    Some(tpl.id.clone()),
                );
                let prompt_text = render_clip_prompt(
                    &tpl.body,
                    &effective_text,
                    input.source_app.as_deref(),
                    url_str.as_deref(),
                );
                let messages = serde_json::json!([
                    { "role": "user", "content": prompt_text }
                ]);
                let params = serde_json::json!({
                    "max_tokens": if tpl.max_tokens > 0 { tpl.max_tokens } else { 1536 },
                    "temperature": tpl.temperature,
                    "reasoning_effort": "off",
                    "chat_template_kwargs": { "enable_thinking": false }
                });
                match st.llm.chat_headless(app, messages, params).await {
                    Ok(raw) => {
                        let cleaned = clean_think_tags(&raw);
                        if !cleaned.is_empty() {
                            distilled_summary = Some(cleaned);
                        }
                    }
                    Err(e) => {
                        distill_note = Some(format!("已存原文（提纯跳过：{}）", format_llm_error(&e)));
                    }
                }
            }
            Err(e) => {
                crate::log_msg(app, format!("[kb-queue] 读取片段提示词 clip/{clip_tpl_id}.md 失败: {e}"));
                distill_note = Some(format!("已存原文（提示词读取失败：{e}）"));
            }
        }
    }

    mgr.update_active_stage(app, "saving", "正在写入知识库并同步索引…", None);
    let saved_path = if is_web {
        let u = url_str
            .as_deref()
            .unwrap_or_else(|| effective_text.as_str());
        crate::kb::save_web(
            app,
            u,
            input.title.as_deref(),
            &effective_text,
            distilled_summary.as_deref(),
        )
        .await?
    } else {
        crate::kb::save_clip(
            app,
            &effective_text,
            input.source_app.as_deref(),
            url_str.as_deref(),
            distilled_summary.as_deref(),
        )
        .await?
    };

    Ok(JobSuccess {
        saved_path: saved_path.to_string_lossy().to_string(),
        detected_type: Some(clip_tpl_id.into()),
        note: Some(distill_note.unwrap_or_else(|| {
            if distilled_summary.is_some() {
                "已提纯要点并存入知识库".into()
            } else {
                "已快速存入知识库".into()
            }
        })),
    })
}

/// 清理推理模型输出中的 <think>...</think> 思考段（包含未闭合段）
pub fn clean_think_tags(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let lower = raw.to_lowercase();
    let mut pos = 0usize;
    while pos < raw.len() {
        if let Some(rel_start) = lower[pos..].find("<think>") {
            let start = pos + rel_start;
            out.push_str(&raw[pos..start]);
            let after_open = start + 7;
            if let Some(rel_end) = lower[after_open..].find("</think>") {
                pos = after_open + rel_end + 8;
            } else {
                break;
            }
        } else {
            out.push_str(&raw[pos..]);
            break;
        }
    }
    out.trim().to_string()
}

fn match_vision_category_text(text: &str, custom_vision_ids: &[String]) -> Option<String> {
    let low_text = text.to_lowercase();
    let clean = low_text.trim();
    if clean.is_empty() {
        return None;
    }

    for cid in custom_vision_ids {
        let low = cid.to_lowercase();
        if !low.is_empty() && low != "classifier" && clean.contains(&low) {
            return Some(cid.clone());
        }
    }

    // 人物类统一归入唯一的 character（涵盖单人、双人、情侣、亲密、人像摄影、立绘与裸露交互）
    if clean.contains("character")
        || clean.contains("couple_intimate")
        || clean.contains("couple")
        || clean.contains("intimate")
        || clean.contains("人物")
        || clean.contains("角色")
        || clean.contains("立绘")
        || clean.contains("人像")
        || clean.contains("女性")
        || clean.contains("女生")
        || clean.contains("少女")
        || clean.contains("美女")
        || clean.contains("女子")
        || clean.contains("男子")
        || clean.contains("情侣")
        || clean.contains("亲密")
        || clean.contains("缠绵")
        || clean.contains("裸体")
        || clean.contains("半裸")
        || clean.contains("写真")
    {
        return Some("character".into());
    }
    if clean.contains("scenery")
        || clean.contains("风景")
        || clean.contains("建筑")
        || clean.contains("街景")
    {
        return Some("scenery".into());
    }
    if clean.contains("ui_software")
        || clean.contains("界面")
        || clean.contains("软件")
        || clean.contains("聊天记录")
    {
        return Some("ui_software".into());
    }
    if clean.contains("object_food")
        || clean.contains("美食")
        || clean.contains("静物")
        || clean.contains("物体")
        || clean.contains("食物")
    {
        return Some("object_food".into());
    }
    if clean.contains("document_chart")
        || clean.contains("图表")
        || clean.contains("文档")
        || clean.contains("架构")
    {
        return Some("document_chart".into());
    }

    None
}

pub fn resolve_vision_category(raw_output: &str, custom_vision_ids: &[String]) -> String {
    let clean = clean_think_tags(raw_output);
    if let Some(hit) = match_vision_category_text(&clean, custom_vision_ids) {
        return hit;
    }
    if let Some(hit) = match_vision_category_text(raw_output, custom_vision_ids) {
        return hit;
    }
    "general".into()
}

fn collapse_blank_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut newlines = 0usize;
    for ch in s.chars() {
        if ch == '\n' {
            newlines += 1;
            if newlines <= 2 {
                out.push('\n');
            }
        } else {
            newlines = 0;
            out.push(ch);
        }
    }
    out.trim().to_string()
}

pub fn render_vision_prompt(body: &str, hint_text: &str, ocr_text: &str) -> String {
    let hint_block = if hint_text.trim().is_empty() {
        String::new()
    } else {
        format!("【已知前置线索/问答主题】：\n{}\n", hint_text.trim())
    };
    let ocr_snippet: String = ocr_text.trim().chars().take(500).collect();
    let ocr_block = if ocr_snippet.is_empty() {
        String::new()
    } else {
        format!("【画面中提取到的文字原词】：\n{ocr_snippet}\n")
    };

    let mut rendered = body.to_string();
    if rendered.contains("{{HINT_CONTEXT}}") {
        rendered = rendered.replace("{{HINT_CONTEXT}}", &hint_block);
    } else if !hint_block.is_empty() {
        rendered = format!("{hint_block}\n{rendered}");
    }

    if rendered.contains("{{OCR_CONTEXT}}") {
        rendered = rendered.replace("{{OCR_CONTEXT}}", &ocr_block);
    } else if !ocr_block.is_empty() {
        rendered = format!("{ocr_block}\n{rendered}");
    }

    collapse_blank_lines(&rendered)
}

pub fn render_clip_prompt(
    body: &str,
    clip_text: &str,
    source_app: Option<&str>,
    url: Option<&str>,
) -> String {
    let mut meta_lines = Vec::new();
    if let Some(app) = source_app.map(str::trim).filter(|s| !s.is_empty()) {
        meta_lines.push(format!("- 来源应用：{app}"));
    }
    if let Some(u) = url.map(str::trim).filter(|s| !s.is_empty()) {
        meta_lines.push(format!("- 关联网址：{u}"));
    }
    let source_block = if meta_lines.is_empty() {
        String::new()
    } else {
        format!("【来源上下文】：\n{}\n", meta_lines.join("\n"))
    };

    let mut rendered = body.to_string();
    if rendered.contains("{{SOURCE_CONTEXT}}") {
        rendered = rendered.replace("{{SOURCE_CONTEXT}}", &source_block);
    } else if !source_block.is_empty() {
        rendered = format!("{source_block}\n{rendered}");
    }

    if rendered.contains("{{CLIP_TEXT}}") {
        rendered = rendered.replace("{{CLIP_TEXT}}", clip_text);
    } else {
        rendered = format!("{rendered}\n\n{clip_text}");
    }

    collapse_blank_lines(&rendered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_clip(i: usize) -> KbQueueInput {
        KbQueueInput {
            kind: "clip".into(),
            title: Some(format!("片段-{i}")),
            text: Some(format!("这是一段测试知识片段 #{i}，长度大于20字符以便验证。")),
            url: None,
            image_data: None,
            existing_ocr: None,
            existing_vision: None,
            source_app: Some("test".into()),
            force_type: None,
        }
    }

    #[test]
    fn test_queue_cap_10_and_fifo() {
        let mgr = KbQueueManager::default();
        for i in 1..=MAX_KB_QUEUE {
            let res = mgr.enqueue_raw(sample_clip(i));
            assert!(res.accepted, "第 {i} 项应成功入队");
            assert_eq!(res.in_flight, i);
            assert_eq!(res.position, i);
        }

        // 第 11 项必须被拦截并给出友好繁忙提示
        let overflow = mgr.enqueue_raw(sample_clip(11));
        assert!(!overflow.accepted, "超过 10 项上限时应拒绝入队");
        assert_eq!(overflow.in_flight, MAX_KB_QUEUE);
        assert_eq!(overflow.max_size, MAX_KB_QUEUE);
        assert!(overflow.message.contains("10/10"));
        assert!(overflow.message.contains("稍等"));

        // 模拟一条正在处理，此时排队剩 9 条，共 10 条仍满；完成后变 9 条可继续入队
        {
            let mut g = mgr.inner.lock().unwrap();
            let first = g.queue.pop_front().unwrap();
            assert_eq!(first.info.title, "片段-1");
            g.active = Some(first.info);
            assert_eq!(g.in_flight(), 10);
        }
        assert!(!mgr.enqueue_raw(sample_clip(12)).accepted);

        {
            let mut g = mgr.inner.lock().unwrap();
            g.active = None;
            assert_eq!(g.in_flight(), 9);
        }
        let ok_again = mgr.enqueue_raw(sample_clip(13));
        assert!(ok_again.accepted);
        assert_eq!(ok_again.in_flight, 10);
    }

    #[test]
    fn test_clean_think_and_resolve_category() {
        assert_eq!(
            clean_think_tags("<think>正在推理\n分类过程</think>\n\ncharacter"),
            "character"
        );
        // 所有人物/亲密/人像输出均统一归一化为唯一的 character，不再存在 couple_intimate
        assert_eq!(
            resolve_vision_category("<think>思考...</think>couple_intimate", &[]),
            "character"
        );
        assert_eq!(
            resolve_vision_category("scenery", &[]),
            "scenery"
        );
        assert_eq!(
            resolve_vision_category("anime_cos", &["anime_cos".into()]),
            "anime_cos"
        );
        assert_eq!(
            resolve_vision_category("<think>图片中是一位夜间户外的女性人像", &[]),
            "character"
        );
    }
}
