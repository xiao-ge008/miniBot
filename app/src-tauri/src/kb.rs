//! 本地知识库核心管理：处理多源数据沉淀（划词按日合并、图片资产与伴生描述、网页快照）
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KbStats {
    pub base_path: String,
    pub notes_count: usize,
    pub images_count: usize,
    pub web_count: usize,
    pub chunks_count: usize,
    pub total_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KbSyncSummary {
    pub folder_path: String,
    pub scanned_docs: usize,
    pub total_chunks: usize,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveResult {
    pub success: bool,
    pub path: String,
    pub message: String,
}

fn nanoid_simple(len: usize) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let s = format!("{:08x}", nanos);
    s[..len.min(s.len())].to_string()
}

/// 获取用户配置的知识库根路径，未配置时回退至系统默认路径
pub fn get_kb_root(app: &AppHandle) -> PathBuf {
    let st = app.state::<crate::AppState>();
    let p = st.settings.get().knowledge.base_path;
    if p.trim().is_empty() {
        crate::settings::default_knowledge_dir()
    } else {
        PathBuf::from(p)
    }
}

/// 确保知识库三大核心分类目录存在
pub fn ensure_kb_dirs(root: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(root.join("notes"))?;
    std::fs::create_dir_all(root.join("images"))?;
    std::fs::create_dir_all(root.join("web"))?;
    Ok(())
}

/// 1. 划词保存：按日期自动追加合并写入 notes/YYYY-MM-DD.md
pub async fn save_clip(
    app: &AppHandle,
    text: &str,
    source_app: Option<&str>,
    url: Option<&str>,
    distilled_summary: Option<&str>,
) -> Result<PathBuf, String> {
    if text.trim().is_empty() {
        return Err("划词内容为空".into());
    }

    let root = get_kb_root(app);
    ensure_kb_dirs(&root).map_err(|e| format!("创建知识库目录失败: {e}"))?;

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let note_path = root.join("notes").join(format!("{today}.md"));

    let time_str = chrono::Local::now().format("%H:%M:%S").to_string();
    let mut block = String::new();

    // 如果当天文件尚不存在，先写入头部标题
    if !note_path.exists() {
        block.push_str(&format!("# 每日知识摘录 - {today}\n\n"));
    }

    block.push_str(&format!("### 📌 摘录于 {time_str}\n"));
    if let Some(src) = source_app.filter(|s| !s.trim().is_empty()) {
        block.push_str(&format!("- **来源应用**：`{src}`\n"));
    }
    if let Some(u) = url.filter(|s| !s.trim().is_empty()) {
        block.push_str(&format!("- **关联链接**：{u}\n"));
    }

    if let Some(summary) = distilled_summary.map(str::trim).filter(|s| !s.is_empty()) {
        block.push_str("\n");
        block.push_str(summary);
        block.push_str("\n\n<details>\n<summary>📄 原始摘录片段（点击展开）</summary>\n\n```text\n");
        block.push_str(text.trim());
        block.push_str("\n```\n\n</details>\n\n---\n\n");
    } else {
        block.push_str("\n```text\n");
        block.push_str(text.trim());
        block.push_str("\n```\n\n---\n\n");
    }

    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&note_path)
        .map_err(|e| format!("打开笔记文件失败: {e}"))?;

    f.write_all(block.as_bytes())
        .map_err(|e| format!("写入笔记内容失败: {e}"))?;

    // 自动异步增量更新索引
    let root_clone = root.clone();
    tokio::task::spawn_blocking(move || {
        let _ = sync_kb_index(&root_clone);
    });

    Ok(note_path)
}

/// 2. 图片资产保存：原图保存为 images/{base_name}.png，识别与伴生描述保存为同名的 images/{base_name}.md
pub async fn save_image(
    app: &AppHandle,
    img_data_base64: &str,
    ocr_text: Option<&str>,
    vision_desc: Option<&str>,
    source: Option<&str>,
) -> Result<PathBuf, String> {
    let root = get_kb_root(app);
    ensure_kb_dirs(&root).map_err(|e| format!("创建知识库目录失败: {e}"))?;

    let now = chrono::Local::now();
    let base_name = format!("{}_{}", now.format("%Y%m%d_%H%M%S"), nanoid_simple(6));
    let images_dir = root.join("images");
    std::fs::create_dir_all(&images_dir).map_err(|e| format!("创建图片资产目录失败: {e}"))?;

    // 去除 data:image/png;base64, 前缀
    let clean_b64 = if let Some(idx) = img_data_base64.find(',') {
        &img_data_base64[idx + 1..]
    } else {
        img_data_base64
    };

    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(clean_b64.trim())
        .map_err(|e| format!("图片 Base64 解码失败: {e}"))?;

    let img_path = images_dir.join(format!("{base_name}.png"));
    std::fs::write(&img_path, &bytes).map_err(|e| format!("保存原图失败: {e}"))?;

    // 伴生生成同名 Markdown 结构化描述文件（用于全文以文搜图与同名对应）
    let md_path = images_dir.join(format!("{base_name}.md"));
    let mut md_content = String::new();
    md_content.push_str(&format!(
        "---\nasset_type: image\nasset_name: {base_name}.png\ncreated_at: {}\norigin_image: {base_name}.png\n---\n\n",
        now.to_rfc3339()
    ));

    md_content.push_str(&format!("# 图片资产 - {base_name}\n\n"));
    md_content.push_str(&format!("![原图]({base_name}.png)\n\n"));

    if let Some(src) = source.filter(|s| !s.trim().is_empty()) {
        md_content.push_str(&format!("**来源**: {}\n\n", src));
    }

    if let Some(desc) = vision_desc.filter(|s| !s.trim().is_empty()) {
        let trimmed = desc.trim();
        if trimmed.starts_with('#') {
            md_content.push_str(&format!("{trimmed}\n\n"));
        } else {
            md_content.push_str(&format!("## 🖼️ 深度视觉特征与内容解析\n\n{trimmed}\n\n"));
        }
    }

    if let Some(ocr) = ocr_text.filter(|s| !s.trim().is_empty()) {
        md_content.push_str(&format!("## 🔤 OCR 提取文字\n```text\n{}\n```\n\n", ocr.trim()));
    }

    std::fs::write(&md_path, md_content.as_bytes()).map_err(|e| format!("保存伴生描述失败: {e}"))?;

    // 自动异步增量更新索引
    let root_clone = root.clone();
    tokio::task::spawn_blocking(move || {
        let _ = sync_kb_index(&root_clone);
    });

    Ok(img_path)
}

/// 3. 网页内容保存：存入 web/{date}_{title}.md
pub async fn save_web(
    app: &AppHandle,
    url: &str,
    title: Option<&str>,
    content: &str,
    distilled_summary: Option<&str>,
) -> Result<PathBuf, String> {
    let root = get_kb_root(app);
    ensure_kb_dirs(&root).map_err(|e| format!("创建知识库目录失败: {e}"))?;

    let now = chrono::Local::now();
    let safe_title: String = title
        .unwrap_or("web_page")
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || (c >= '\u{4e00}' && c <= '\u{9fa5}') {
                c
            } else {
                '_'
            }
        })
        .take(50)
        .collect();

    let file_name = format!("{}_{}.md", now.format("%Y%m%d_%H%M%S"), safe_title);
    let web_path = root.join("web").join(file_name);

    let mut body = String::new();
    if let Some(summary) = distilled_summary.map(str::trim).filter(|s| !s.is_empty()) {
        body.push_str("## ✨ AI 提纯与总结\n\n");
        body.push_str(summary);
        body.push_str("\n\n---\n\n## 📄 原始内容\n\n");
    }
    body.push_str(content);

    let md = format!(
        "---\nurl: {}\ntitle: {}\ncreated_at: {}\n---\n\n# {}\n\n- **原始网址**：{}\n- **抓取时间**：{}\n\n---\n\n{}\n",
        url,
        title.unwrap_or(url),
        now.to_rfc3339(),
        title.unwrap_or(url),
        url,
        now.format("%Y-%m-%d %H:%M:%S"),
        body
    );

    std::fs::write(&web_path, md.as_bytes()).map_err(|e| format!("保存网页内容失败: {e}"))?;

    // 自动异步增量更新索引
    let root_clone = root.clone();
    tokio::task::spawn_blocking(move || {
        let _ = sync_kb_index(&root_clone);
    });

    Ok(web_path)
}

/// 4. 获取知识库数据统计：递归全量统计当前根目录下的所有文档、图片与已建索引切片数
pub fn get_kb_stats(app: &AppHandle) -> Result<KbStats, String> {
    let root = get_kb_root(app);
    let _ = ensure_kb_dirs(&root);

    let mut doc_count = 0;
    let mut image_count = 0;
    let mut web_count = 0;
    let mut total_size_bytes = 0u64;

    scan_dir_stats(&root, &mut doc_count, &mut image_count, &mut web_count, &mut total_size_bytes);

    // 查询 FTS5 表中的已建切片总数
    let chunks_count = if let Ok(conn) = open_db(&root) {
        conn.query_row("SELECT count(*) FROM kb_fts", [], |r| r.get::<_, i64>(0))
            .map(|v| v as usize)
            .unwrap_or(0)
    } else {
        0
    };

    Ok(KbStats {
        base_path: root.to_string_lossy().to_string(),
        notes_count: doc_count,
        images_count: image_count,
        web_count,
        chunks_count,
        total_size_bytes,
    })
}

fn scan_dir_stats(
    dir: &Path,
    doc_count: &mut usize,
    image_count: &mut usize,
    web_count: &mut usize,
    total_size: &mut u64,
) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "node_modules" || name == "target" || name == "dist" || name == "build" {
                continue;
            }
            if p.is_dir() {
                scan_dir_stats(&p, doc_count, image_count, web_count, total_size);
            } else if p.is_file() {
                if let Ok(m) = e.metadata() {
                    *total_size += m.len();
                }
                if let Some(ext) = p.extension() {
                    let ext_str = ext.to_string_lossy().to_lowercase();
                    match ext_str.as_str() {
                        "md" | "markdown" | "txt" | "text" | "json" | "csv" | "log" | "yaml" | "yml" => {
                            *doc_count += 1;
                            if p.to_string_lossy().contains("web") {
                                *web_count += 1;
                            }
                        }
                        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" => {
                            *image_count += 1;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

/// 5. 在操作系统的文件管理器中打开知识库根目录
pub fn open_kb_folder(app: &AppHandle) -> Result<(), String> {
    let root = get_kb_root(app);
    let _ = ensure_kb_dirs(&root);

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&root)
            .spawn()
            .map_err(|e| format!("打开知识库文件夹失败: {e}"))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&root)
            .spawn()
            .map_err(|e| format!("打开知识库文件夹失败: {e}"))?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&root)
            .spawn()
            .map_err(|e| format!("打开知识库文件夹失败: {e}"))?;
    }

    Ok(())
}

// ==================== SQLite FTS5 本地极速索引与检索内核 ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KbSearchHit {
    pub path: String,
    pub relative_path: String,
    pub title: String,
    pub heading: String,
    pub content: String,
    pub line_start: usize,
    pub line_end: usize,
    pub score: f64,
    pub origin_image: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KbSearchResponse {
    pub query: String,
    pub hits: Vec<KbSearchHit>,
    pub context_text: String,
}

fn open_db(root: &Path) -> rusqlite::Result<rusqlite::Connection> {
    let sag_dir = root.join(".sag");
    let _ = std::fs::create_dir_all(&sag_dir);
    let db_path = sag_dir.join("kb.sqlite");
    let conn = rusqlite::Connection::open(db_path)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS kb_files (
            path TEXT PRIMARY KEY,
            mtime INTEGER
        );
        CREATE VIRTUAL TABLE IF NOT EXISTS kb_fts USING fts5(
            doc_path,
            heading,
            content,
            line_start UNINDEXED,
            line_end UNINDEXED,
            tokenize='unicode61'
        );",
    )?;
    Ok(conn)
}

/// 增量扫描并同步知识库文件的 FTS5 索引
pub fn sync_kb_index(root: &Path) -> Result<KbSyncSummary, String> {
    let mut conn = open_db(root).map_err(|e| format!("打开数据库失败: {e}"))?;
    let _ = ensure_kb_dirs(root);

    let mut doc_files = Vec::new();
    collect_doc_files(root, &mut doc_files);

    let tx = conn.transaction().map_err(|e| format!("开启事务失败: {e}"))?;

    // 1. 清理已在磁盘被删除或移动的旧文件索引
    let mut stale_paths = Vec::new();
    {
        if let Ok(mut stmt) = tx.prepare("SELECT path FROM kb_files") {
            if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
                for p in rows.flatten() {
                    if !Path::new(&p).exists() {
                        stale_paths.push(p);
                    }
                }
            }
        }
    }
    for sp in stale_paths {
        let _ = tx.execute("DELETE FROM kb_fts WHERE doc_path = ?1", rusqlite::params![&sp]);
        let _ = tx.execute("DELETE FROM kb_files WHERE path = ?1", rusqlite::params![&sp]);
    }

    // 2. 增量扫描并切片新文件或已修改的文件
    for file_path in &doc_files {
        let meta = match std::fs::metadata(file_path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let p_str = file_path.to_string_lossy().to_string();

        let cached_mtime: Option<i64> = tx
            .query_row(
                "SELECT mtime FROM kb_files WHERE path = ?1",
                rusqlite::params![&p_str],
                |r| r.get(0),
            )
            .ok();

        if cached_mtime == Some(mtime) {
            continue; // 未发生变更，跳过
        }

        // 文件发生改动或为新增文件：先清理旧条目
        let _ = tx.execute("DELETE FROM kb_fts WHERE doc_path = ?1", rusqlite::params![&p_str]);

        // 读取正文进行分块切片
        if let Ok(content) = std::fs::read_to_string(file_path) {
            let chunks = chunk_markdown(&content);
            for (heading, chunk_text, l_start, l_end) in chunks {
                let _ = tx.execute(
                    "INSERT INTO kb_fts (doc_path, heading, content, line_start, line_end) VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![&p_str, &heading, &chunk_text, l_start as i64, l_end as i64],
                );
            }
        }

        // 更新文件时间戳
        let _ = tx.execute(
            "INSERT INTO kb_files (path, mtime) VALUES (?1, ?2) ON CONFLICT(path) DO UPDATE SET mtime = excluded.mtime",
            rusqlite::params![&p_str, mtime],
        );
    }

    tx.commit().map_err(|e| format!("提交索引事务失败: {e}"))?;

    let total_chunks: usize = conn
        .query_row("SELECT count(*) FROM kb_fts", [], |r| r.get::<_, i64>(0))
        .map(|v| v as usize)
        .unwrap_or(0);

    Ok(KbSyncSummary {
        folder_path: root.to_string_lossy().to_string(),
        scanned_docs: doc_files.len(),
        total_chunks,
    })
}

fn collect_doc_files(dir: &Path, list: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "node_modules" || name == "target" || name == "dist" || name == "build" {
                continue;
            }
            if p.is_dir() {
                collect_doc_files(&p, list);
            } else if p.is_file() {
                if let Some(ext) = p.extension() {
                    let ext_str = ext.to_string_lossy().to_lowercase();
                    match ext_str.as_str() {
                        "md" | "markdown" | "txt" | "text" | "json" | "csv" | "log" | "yaml" | "yml" => {
                            list.push(p);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

/// 将 Markdown 或纯文本文档按标题和自然段落切片（保留行号）
fn chunk_markdown(text: &str) -> Vec<(String, String, usize, usize)> {
    let mut chunks = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return chunks;
    }

    let mut current_heading = String::from("正文");
    let mut current_buf = Vec::new();
    let mut chunk_start_line = 1;
    let mut current_chars = 0;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let line_num = i + 1;

        let is_heading = trimmed.starts_with('#');
        let is_empty_break = trimmed.is_empty() && current_buf.len() >= 5;
        let is_length_overflow = current_buf.len() >= 35 || current_chars >= 1000;

        if is_heading || is_empty_break || is_length_overflow {
            if !current_buf.is_empty() {
                let chunk_text = current_buf.join("\n").trim().to_string();
                if !chunk_text.is_empty() {
                    chunks.push((current_heading.clone(), chunk_text, chunk_start_line, line_num.saturating_sub(1)));
                }
                current_buf.clear();
                current_chars = 0;
            }

            if is_heading {
                current_heading = trimmed.trim_start_matches('#').trim().to_string();
            }
            chunk_start_line = line_num;
        }

        current_buf.push(*line);
        current_chars += line.len();
    }

    if !current_buf.is_empty() {
        let chunk_text = current_buf.join("\n").trim().to_string();
        if !chunk_text.is_empty() {
            chunks.push((current_heading, chunk_text, chunk_start_line, lines.len()));
        }
    }

    chunks
}

/// 智能提取中文与英文自然语言提问中的检索关键词
pub fn extract_search_keywords(raw: &str) -> Vec<String> {
    let clean = raw
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || (c >= '\u{4e00}' && c <= '\u{9fa5}') {
                c
            } else {
                ' '
            }
        })
        .collect::<String>();

    // 常见问答疑问词、动作前缀与虚词停用词（按长度倒序替换，防止子串误伤）
    let stopwords = [
        "告诉我有关于", "帮我查一下", "帮我查查", "帮我查", "我想知道", "问一下", "请问一下", "请问",
        "知道吗", "你知道", "分别是谁", "干什么的", "是干嘛的", "干嘛的", "扮演谁", "是谁呀",
        "是谁啊", "是谁呢", "是谁的", "是谁", "谁是", "谁啊", "谁呀", "谁呢", "有谁", "是哪位",
        "是哪个", "哪一个", "是啥呀", "是啥啊", "是啥", "啥是", "介绍一下", "介绍下", "介绍",
        "讲讲", "说说", "聊聊", "解释一下", "解释下", "解释", "查询", "搜索", "了解", "什么是",
        "什么叫做", "什么叫", "什么样", "什么时候", "为什么", "怎么回事", "怎么", "怎样",
        "有哪些", "有什么", "有啥", "具体是", "通常", "平时", "经常", "一般", "往往",
        "主要", "比如", "一下", "一个", "有的", "并且", "以及", "还是", "或者", "因为",
        "所以", "如果", "但是", "可以", "能不能", "是否", "什么", "谁", "啥",
        "的", "了", "吗", "呢", "吧", "啊", "呀", "与", "和", "在", "中", "上", "下",
        "里", "到", "把", "被", "它", "他", "她", "你", "我", "们", "其", "这", "那", "是",
    ];

    let mut filtered = clean;
    for sw in stopwords {
        filtered = filtered.replace(sw, " ");
    }

    let mut keywords = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for part in filtered.split_whitespace() {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }

        // 英文或数字单词直接保留
        if trimmed.is_ascii() {
            if trimmed.len() >= 2 {
                let lower = trimmed.to_lowercase();
                if seen.insert(lower.clone()) {
                    keywords.push(lower);
                }
            }
            continue;
        }

        let char_vec: Vec<char> = trimmed.chars().collect();
        if char_vec.is_empty() {
            continue;
        }

        // 无论长度长短，都提取全词以及 2-gram、3-gram
        let full_str: String = char_vec.iter().collect();
        if full_str.chars().count() >= 2 && seen.insert(full_str.clone()) {
            keywords.push(full_str);
        }

        if char_vec.len() >= 3 {
            for w in 2..=3 {
                for i in 0..=char_vec.len().saturating_sub(w) {
                    let sub: String = char_vec[i..i + w].iter().collect();
                    if seen.insert(sub.clone()) {
                        keywords.push(sub);
                    }
                }
            }
        }
    }

    // 兜底回退：如果由于极端提问导致过滤完为空，直接提取原句中的所有 2-gram 字符对
    if keywords.is_empty() {
        let raw_clean: Vec<char> = raw
            .chars()
            .filter(|c| c.is_alphanumeric() || (*c >= '\u{4e00}' && *c <= '\u{9fa5}'))
            .collect();
        if raw_clean.len() >= 2 {
            for i in 0..raw_clean.len() - 1 {
                let pair: String = raw_clean[i..i + 2].iter().collect();
                if seen.insert(pair.clone()) {
                    keywords.push(pair);
                }
            }
        } else if !raw_clean.is_empty() {
            keywords.push(raw_clean.into_iter().collect());
        }
    }

    keywords
}

/// 智能寻找文档切片所关联的图片文件（同名图片优先 > 正文引用图片 > 旧版 origin.png）
fn resolve_associated_image(doc_path: &Path, content: &str) -> Option<String> {
    // 1. 同名图片优先（如 20261111_021530.md 对应 20261111_021530.png / jpg / webp）
    for ext in ["png", "jpg", "jpeg", "webp", "gif", "bmp"] {
        let same_name_img = doc_path.with_extension(ext);
        if same_name_img.exists() {
            return Some(same_name_img.to_string_lossy().to_string());
        }
    }

    // 2. 检查正文中的 Markdown 图片引用：![...](xxx.png) 或 相对路径
    if let Some(img_ref) = extract_image_ref_from_markdown(content) {
        if let Some(parent) = doc_path.parent() {
            let direct = parent.join(&img_ref);
            if direct.exists() {
                return Some(direct.to_string_lossy().to_string());
            }
        }
        let abs = Path::new(&img_ref);
        if abs.exists() {
            return Some(abs.to_string_lossy().to_string());
        }
    }

    // 3. 兼容旧版资产结构（同目录下 origin.png）
    if let Some(parent) = doc_path.parent() {
        let old_origin = parent.join("origin.png");
        if old_origin.exists() {
            return Some(old_origin.to_string_lossy().to_string());
        }
    }

    None
}

/// 从 Markdown 片段中解析提取引用的第一个图片文件路径
fn extract_image_ref_from_markdown(content: &str) -> Option<String> {
    for line in content.lines() {
        if let Some(start_idx) = line.find("![") {
            if let Some(open_paren) = line[start_idx..].find('(') {
                let after = &line[start_idx + open_paren + 1..];
                if let Some(close_paren) = after.find(')') {
                    let path = after[..close_paren].trim();
                    let clean = path.split_whitespace().next().unwrap_or("");
                    let lower = clean.to_lowercase();
                    if lower.ends_with(".png")
                        || lower.ends_with(".jpg")
                        || lower.ends_with(".jpeg")
                        || lower.ends_with(".webp")
                        || lower.ends_with(".gif")
                    {
                        return Some(clean.to_string());
                    }
                }
            }
        }
    }
    None
}

/// 执行知识库多阶混合检索（FTS5 MATCH + LIKE 兜底 + 关键词加权重排）
pub fn search_kb(
    app: &AppHandle,
    raw_query: &str,
    limit: usize,
) -> Result<KbSearchResponse, String> {
    let root = get_kb_root(app);
    let _ = sync_kb_index(&root);

    let conn = open_db(&root).map_err(|e| format!("打开知识库数据库失败: {e}"))?;
    let root_str = root.to_string_lossy().to_string();

    let keywords = extract_search_keywords(raw_query);
    if keywords.is_empty() {
        return Ok(KbSearchResponse {
            query: raw_query.to_string(),
            hits: vec![],
            context_text: String::new(),
        });
    }

    let mut candidate_map: std::collections::HashMap<String, KbSearchHit> = std::collections::HashMap::new();

    // 阶段 1：FTS5 MATCH 查询
    let match_terms: Vec<String> = keywords
        .iter()
        .map(|k| {
            if k.is_ascii() {
                format!("{k}*")
            } else {
                format!("\"{k}\"*")
            }
        })
        .collect();
    let match_expr = match_terms.join(" OR ");

    if let Ok(mut stmt) = conn.prepare(
        "SELECT doc_path, heading, content, line_start, line_end, bm25(kb_fts) as rank
         FROM kb_fts
         WHERE kb_fts MATCH ?1
         ORDER BY rank ASC
         LIMIT 40",
    ) {
        if let Ok(rows) = stmt.query_map(rusqlite::params![&match_expr], |r| {
            let path: String = r.get(0)?;
            let heading: String = r.get(1)?;
            let content: String = r.get(2)?;
            let line_start = r.get::<_, i64>(3)? as usize;
            let line_end = r.get::<_, i64>(4)? as usize;
            let score: f64 = r.get(5)?;
            Ok((path, heading, content, line_start, line_end, score))
        }) {
            for item in rows.flatten() {
                let (path_str, heading, content, l_start, l_end, score) = item;
                let key = format!("{path_str}:{l_start}");
                let p = Path::new(&path_str);
                let rel_path = if path_str.starts_with(&root_str) {
                    path_str[root_str.len()..].trim_start_matches(['\\', '/']).to_string()
                } else {
                    path_str.clone()
                };
                let origin_img = resolve_associated_image(p, &content);
                let raw_stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "未知文档".to_string());
                let title = if origin_img.is_some() || path_str.contains("images") {
                    if raw_stem == "vision" {
                        if let Some(parent_name) = p.parent().and_then(|pr| pr.file_name()).map(|f| f.to_string_lossy().to_string()) {
                            format!("🖼️ 图片资产 ({parent_name})")
                        } else {
                            "🖼️ 图片资产".to_string()
                        }
                    } else {
                        format!("🖼️ {raw_stem}")
                    }
                } else {
                    raw_stem
                };

                candidate_map.insert(key, KbSearchHit {
                    path: path_str,
                    relative_path: rel_path,
                    title,
                    heading,
                    content,
                    line_start: l_start,
                    line_end: l_end,
                    score: 100.0 - score.min(99.0),
                    origin_image: origin_img,
                });
            }
        }
    }

    // 阶段 2：SQL LIKE 兜底（针对中文多字无空格短语，保证 100% 召回，不遗漏任何相关笔记）
    for kw in keywords.iter().take(4) {
        if kw.chars().count() < 2 {
            continue;
        }
        let pattern = format!("%{kw}%");
        if let Ok(mut stmt) = conn.prepare(
            "SELECT doc_path, heading, content, line_start, line_end
             FROM kb_fts
             WHERE content LIKE ?1 OR heading LIKE ?1 OR doc_path LIKE ?1
             LIMIT 25",
        ) {
            if let Ok(rows) = stmt.query_map(rusqlite::params![&pattern], |r| {
                let path: String = r.get(0)?;
                let heading: String = r.get(1)?;
                let content: String = r.get(2)?;
                let line_start = r.get::<_, i64>(3)? as usize;
                let line_end = r.get::<_, i64>(4)? as usize;
                Ok((path, heading, content, line_start, line_end))
            }) {
                for item in rows.flatten() {
                    let (path_str, heading, content, l_start, l_end) = item;
                    let key = format!("{path_str}:{l_start}");
                    if !candidate_map.contains_key(&key) {
                        let p = Path::new(&path_str);
                        let rel_path = if path_str.starts_with(&root_str) {
                            path_str[root_str.len()..].trim_start_matches(['\\', '/']).to_string()
                        } else {
                            path_str.clone()
                        };
                        let origin_img = resolve_associated_image(p, &content);
                        let raw_stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "未知文档".to_string());
                        let title = if origin_img.is_some() || path_str.contains("images") {
                            if raw_stem == "vision" {
                                if let Some(parent_name) = p.parent().and_then(|pr| pr.file_name()).map(|f| f.to_string_lossy().to_string()) {
                                    format!("🖼️ 图片资产 ({parent_name})")
                                } else {
                                    "🖼️ 图片资产".to_string()
                                }
                            } else {
                                format!("🖼️ {raw_stem}")
                            }
                        } else {
                            raw_stem
                        };
                        candidate_map.insert(key, KbSearchHit {
                            path: path_str,
                            relative_path: rel_path,
                            title,
                            heading,
                            content,
                            line_start: l_start,
                            line_end: l_end,
                            score: 50.0,
                            origin_image: origin_img,
                        });
                    }
                }
            }
        }
    }

    // 阶段 3：多因子综合加权与精准重排序（Rerank）
    let mut hits: Vec<KbSearchHit> = candidate_map.into_values().collect();
    let clean_raw = raw_query.trim().to_lowercase();

    for hit in &mut hits {
        let content_lower = hit.content.to_lowercase();
        let heading_lower = hit.heading.to_lowercase();
        let path_lower = hit.path.to_lowercase();

        let mut matched_keywords_count = 0;
        let mut score = hit.score;

        for kw in &keywords {
            let kw_lower = kw.to_lowercase();
            let in_content = content_lower.contains(&kw_lower);
            let in_heading = heading_lower.contains(&kw_lower);
            let in_path = path_lower.contains(&kw_lower);

            if in_content || in_heading || in_path {
                matched_keywords_count += 1;
            }
            if in_heading {
                score += 35.0; // 标题命中高权重
            }
            if in_path {
                score += 25.0; // 文件名命中高权重
            }
            if in_content {
                score += 15.0;
            }
        }

        // 关键词重合度加权：命中的关键词越多，分数显著提升
        score += (matched_keywords_count as f64) * 25.0;

        // 如果包含用户的完整自然提问短语，大幅加分
        if clean_raw.chars().count() >= 3 && content_lower.contains(&clean_raw) {
            score += 80.0;
        }

        hit.score = score;
    }

    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    // 阶段 4：若已安装并启用 Laya GGUF 加速模型（laya_multilingual_q8_0.gguf），对候选片段执行神经语义重排
    let laya_enabled = app
        .state::<crate::AppState>()
        .settings
        .get()
        .knowledge
        .laya_enabled;
    if laya_enabled && hits.len() > 1 {
        if let Some((scored, ms)) = laya_rerank_hits(raw_query, &mut hits) {
            crate::log_msg(
                app,
                format!("[kb][laya] GGUF 神经语义重排完成: 候选={scored} 耗时={ms}ms"),
            );
        }
    }

    hits.truncate(limit.max(12));

    let mut context_text = String::new();
    if !hits.is_empty() {
        context_text.push_str("\n\n【本地个人知识库检索参考资料（出处可核对）】：\n");
        context_text.push_str("以下内容检索自用户的本地知识库，具有最高参考价值。请仔细阅读并优先根据以下事实回答用户：\n\n");
        for (idx, hit) in hits.iter().enumerate() {
            context_text.push_str(&format!(
                "[{}] 来源：《{}》- {} (第 {}-{} 行",
                idx + 1,
                hit.title,
                hit.relative_path,
                hit.line_start,
                hit.line_end
            ));
            if !hit.heading.is_empty() && hit.heading != "正文" {
                context_text.push_str(&format!("，章节：{}", hit.heading));
            }
            if let Some(img) = &hit.origin_image {
                context_text.push_str(&format!("，对应原图资产：{}", img));
            }
            context_text.push_str(")\n内容：\n");
            context_text.push_str(&hit.content);
            context_text.push_str("\n\n");
        }
        context_text.push_str(
            "【知识库问答指导要求】：\n\
            1. 请严格优先依据上述知识库资料回答用户提问，保证回答准确、详实、逻辑清晰；\n\
            2. 必须在所有引用事实、人物设定或具体结论的句末标注对应的引用角标（如 [1]、[2]、[3]）；\n\
            3. 如果资料中已有明确记载，请直接肯定地给出答案；若未完全涵盖用户提出的所有方面，请客观指明已记载部分并简要补充。\n\n"
        );
    }

    Ok(KbSearchResponse {
        query: raw_query.to_string(),
        hits,
        context_text,
    })
}

// ==================== Laya GGUF 加速模型管理、推理守护进程与下载 ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayaStatus {
    pub installed: bool,
    pub model_ready: bool,
    pub engine_ready: bool,
    pub needs_redownload: bool,
    pub size_mb: f64,
    pub path: String,
    pub enabled: bool,
}

pub fn get_laya_dir() -> PathBuf {
    crate::settings::default_engine_dir().join("models").join("laya")
}

/// 校验文件是否为合法的 GGUF 模型文件（文件大小 >= 50MB 且头部魔数为 "GGUF"）
pub fn is_valid_gguf(path: &Path) -> bool {
    use std::io::Read;
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() || meta.len() < 50 * 1024 * 1024 {
        return false;
    }
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 4];
    if f.read_exact(&mut magic).is_err() {
        return false;
    }
    &magic == b"GGUF"
}

/// 扫描并清理老版本遗留的错误模型文件（如 model.safetensors、半成品 .part 或损坏的非 GGUF 文件）
/// 返回是否发现并清理了旧版遗留模型
fn cleanup_legacy_laya_files(dir: &Path) -> bool {
    let mut found_legacy = false;
    let legacy_safetensors = dir.join("model.safetensors");
    if legacy_safetensors.exists() {
        let _ = std::fs::remove_file(&legacy_safetensors);
        found_legacy = true;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            let fname = p
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            if fname.ends_with(".safetensors") || fname.ends_with(".safetensors.part") {
                let _ = std::fs::remove_file(&p);
                found_legacy = true;
            } else if fname.ends_with(".gguf") && !is_valid_gguf(&p) {
                let _ = std::fs::remove_file(&p);
                found_legacy = true;
            }
        }
    }
    found_legacy
}

pub fn get_laya_file_path() -> PathBuf {
    let dir = get_laya_dir();
    let preferred = dir.join("laya_multilingual_q8_0.gguf");
    if is_valid_gguf(&preferred) {
        return preferred;
    }
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("gguf"))
                    .unwrap_or(false)
                && is_valid_gguf(&p)
            {
                return p;
            }
        }
    }
    preferred
}

pub fn get_laya_bin_path() -> PathBuf {
    let bin_name = if cfg!(windows) { "laya.exe" } else { "laya" };
    get_laya_dir().join(bin_name)
}

struct LayaDaemon {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: std::io::BufReader<std::process::ChildStdout>,
    model_path: PathBuf,
}

static LAYA_DAEMON: std::sync::OnceLock<std::sync::Mutex<Option<LayaDaemon>>> =
    std::sync::OnceLock::new();

fn laya_daemon_slot() -> &'static std::sync::Mutex<Option<LayaDaemon>> {
    LAYA_DAEMON.get_or_init(|| std::sync::Mutex::new(None))
}

pub fn stop_laya_daemon() {
    if let Ok(mut guard) = laya_daemon_slot().lock() {
        if let Some(mut d) = guard.take() {
            let _ = d.child.kill();
            let _ = d.child.wait();
        }
    }
}

impl LayaDaemon {
    fn spawn(bin_path: &Path, model_path: &Path) -> Result<Self, String> {
        use std::io::BufRead;
        use std::process::{Command, Stdio};

        let mut cmd = Command::new(bin_path);
        cmd.args([
            "daemon",
            &model_path.to_string_lossy(),
            "--device",
            "auto",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("启动 Laya GGUF 引擎失败: {e}"))?;
        crate::jobobj::attach(child.id());

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "无法打开 Laya stdin".to_string())?;
        let stdout_raw = child
            .stdout
            .take()
            .ok_or_else(|| "无法打开 Laya stdout".to_string())?;
        let mut stdout = std::io::BufReader::new(stdout_raw);

        let mut line = String::new();
        loop {
            line.clear();
            let n = stdout
                .read_line(&mut line)
                .map_err(|e| format!("读取 Laya 就绪状态失败: {e}"))?;
            if n == 0 {
                let _ = child.kill();
                return Err("Laya 守护进程在就绪前退出".to_string());
            }
            if line.contains("\"status\"") && line.contains("\"ready\"") {
                break;
            }
        }

        Ok(Self {
            child,
            stdin,
            stdout,
            model_path: model_path.to_path_buf(),
        })
    }

    fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn score_relevance(&mut self, query: &str, snippet: &str) -> Option<f64> {
        use std::io::{BufRead, Write};

        let req = serde_json::json!({
            "questions": {
                "q": {
                    "type": "noul",
                    "instructions": format!("文档内容是否与问题相关：{query}")
                }
            },
            "state": {
                "document": snippet
            }
        });
        let mut payload = serde_json::to_string(&req).ok()?;
        payload.push('\n');
        self.stdin.write_all(payload.as_bytes()).ok()?;
        self.stdin.flush().ok()?;

        let mut resp_line = String::new();
        let n = self.stdout.read_line(&mut resp_line).ok()?;
        if n == 0 {
            return None;
        }
        let val: serde_json::Value = serde_json::from_str(resp_line.trim()).ok()?;
        val.get("answers")?
            .get("q")?
            .get("noul")?
            .as_f64()
    }
}

/// 使用 Laya GGUF 模型（laya_multilingual_q8_0.gguf）对前 N 条候选片段做神经语义重排
pub fn laya_rerank_hits(raw_query: &str, hits: &mut [KbSearchHit]) -> Option<(usize, u128)> {
    let query = raw_query.trim();
    if query.is_empty() || hits.len() <= 1 {
        return None;
    }
    let model_path = get_laya_file_path();
    let bin_path = get_laya_bin_path();
    if !model_path.exists() || !bin_path.exists() {
        return None;
    }

    let t0 = std::time::Instant::now();
    let mut guard = laya_daemon_slot().lock().ok()?;
    let needs_respawn = match guard.as_mut() {
        Some(d) => !d.is_alive() || d.model_path != model_path,
        None => true,
    };
    if needs_respawn {
        if let Some(mut old) = guard.take() {
            let _ = old.child.kill();
            let _ = old.child.wait();
        }
        match LayaDaemon::spawn(&bin_path, &model_path) {
            Ok(d) => *guard = Some(d),
            Err(_) => return None,
        }
    }

    let daemon = guard.as_mut()?;
    let top_n = hits.len().min(8);
    let mut scored_count = 0usize;

    for hit in hits.iter_mut().take(top_n) {
        let combined = if hit.heading.is_empty() || hit.heading == "正文" {
            hit.content.clone()
        } else {
            format!("{}：{}", hit.heading, hit.content)
        };
        let snippet: String = combined
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(240)
            .collect();
        if snippet.is_empty() {
            continue;
        }
        if let Some(prob) = daemon.score_relevance(query, &snippet) {
            let p = prob.clamp(0.0, 1.0);
            // 将 Laya 的 noul 概率映射为显著重排加分（0.001 -> +3.8，0.05 -> +30.8，0.36 -> +100.8，0.95 -> +193.0）
            let boost = (p.sqrt() * 120.0) + (p * 80.0);
            hit.score += boost;
            scored_count += 1;
        } else {
            // 若管道异常断开，立即终止本轮重排并清理守护进程，不阻塞主检索结果
            if let Some(mut broken) = guard.take() {
                let _ = broken.child.kill();
            }
            break;
        }
    }

    if scored_count > 0 {
        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        Some((scored_count, t0.elapsed().as_millis()))
    } else {
        None
    }
}

pub fn check_laya_status(app: &AppHandle) -> LayaStatus {
    let dir = get_laya_dir();
    let _ = std::fs::create_dir_all(&dir);
    let redownload_marker = dir.join(".needs_gguf_redownload");
    let auto_marker = dir.join(".gguf_auto_enabled");

    // 1. 扫描并清理老用户遗留的错误模型文件（如 model.safetensors 或非 GGUF 损坏文件）
    if cleanup_legacy_laya_files(&dir) {
        let _ = std::fs::write(&redownload_marker, b"1");
    }

    // 2. 严格校验 GGUF 模型与原生推理执行器是否同时就绪
    let target = get_laya_file_path();
    let model_ready = is_valid_gguf(&target);
    let engine_ready = get_laya_bin_path().exists();
    let installed = model_ready && engine_ready;

    let size_bytes = if model_ready {
        std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };
    let size_mb = (size_bytes as f64 / (1024.0 * 1024.0) * 10.0).round() / 10.0;

    let st = app.state::<crate::AppState>();
    let mut s = st.settings.get();

    if installed {
        // 模型与推理引擎均已就绪：清除老用户重下标记，并在首次识别时自动开启加速
        if redownload_marker.exists() {
            let _ = std::fs::remove_file(&redownload_marker);
        }
        if !auto_marker.exists() {
            let _ = std::fs::write(&auto_marker, b"1");
            if !s.knowledge.laya_enabled {
                s.knowledge.laya_enabled = true;
                let _ = st.settings.set(s.clone());
            }
        }
    } else if s.knowledge.laya_enabled {
        // 老用户之前开启过 laya_enabled（当时下载的是错误的 model.safetensors），
        // 记录需要重新下载 GGUF 的标记，并将配置重置为未开启，防止虚假开启状态。
        let _ = std::fs::write(&redownload_marker, b"1");
        s.knowledge.laya_enabled = false;
        let _ = st.settings.set(s.clone());
    }

    let needs_redownload =
        !installed && (redownload_marker.exists() || (model_ready && !engine_ready));
    let enabled = installed && s.knowledge.laya_enabled;

    LayaStatus {
        installed,
        model_ready,
        engine_ready,
        needs_redownload,
        size_mb,
        path: target.to_string_lossy().to_string(),
        enabled,
    }
}

/// 递归查找解压目录中的指定可执行文件
fn find_extracted_file(root: &Path, target_name: &str) -> Option<PathBuf> {
    let direct = root.join(target_name);
    if direct.is_file() {
        return Some(direct);
    }
    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if let Some(found) = find_extracted_file(&p, target_name) {
                return Some(found);
            }
        }
    }
    None
}

pub async fn download_laya_model(
    app: &AppHandle,
    on_prog: tauri::ipc::Channel<crate::engine::DownloadProgress>,
) -> Result<LayaStatus, String> {
    use futures_util::StreamExt;
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;

    stop_laya_daemon();
    let dir = get_laya_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = cleanup_legacy_laya_files(&dir);
    let target = dir.join("laya_multilingual_q8_0.gguf");

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(12))
        .user_agent("Mozilla/5.0 miniBot/0.1.0")
        .build()
        .map_err(|e| e.to_string())?;

    // 1. 若缺少 laya 原生推理执行器（Windows: laya.exe, macOS/Linux: laya），先行自动下载并解压
    let bin_path = get_laya_bin_path();
    let bin_name = bin_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("laya")
        .to_string();
    if !bin_path.exists() {
        let _ = on_prog.send(crate::engine::DownloadProgress {
            target: "laya".into(),
            stage: "downloading".into(),
            file: bin_name.clone(),
            downloaded_mb: 0.0,
            total_mb: if cfg!(target_os = "macos") { 1.2 } else { 17.5 },
            percent: 2,
            message: "正在下载 Laya GGUF 原生推理引擎…".into(),
        });

        let asset_name = if cfg!(windows) {
            "laya-windows-x86_64-vulkan.zip"
        } else if cfg!(target_os = "macos") {
            "laya-macos-arm64-metal.tar.gz"
        } else {
            "laya-linux-x86_64-vulkan.tar.gz"
        };
        let rt_urls = [
            format!("https://ghfast.top/https://github.com/monatis/ggmlc/releases/latest/download/{asset_name}"),
            format!("https://gh-proxy.org/https://github.com/monatis/ggmlc/releases/latest/download/{asset_name}"),
            format!("https://github.com/monatis/ggmlc/releases/latest/download/{asset_name}"),
        ];
        let archive_path = dir.join(asset_name);
        let tmp_extract = dir.join("laya_extract_tmp");

        for u in &rt_urls {
            if let Ok(resp) = client.get(u).send().await {
                if resp.status().is_success() {
                    if let Ok(bytes) = resp.bytes().await {
                        if std::fs::write(&archive_path, &bytes).is_ok() {
                            let _ = std::fs::remove_dir_all(&tmp_extract);
                            let _ = std::fs::create_dir_all(&tmp_extract);
                            let _ = std::process::Command::new("tar")
                                .args([
                                    "-xf",
                                    &archive_path.to_string_lossy(),
                                    "-C",
                                    &tmp_extract.to_string_lossy(),
                                ])
                                .output();
                            if let Some(extracted_bin) = find_extracted_file(&tmp_extract, &bin_name)
                            {
                                let _ = std::fs::copy(&extracted_bin, &bin_path);
                                #[cfg(not(windows))]
                                {
                                    use std::os::unix::fs::PermissionsExt;
                                    if let Ok(meta) = std::fs::metadata(&bin_path) {
                                        let mut perms = meta.permissions();
                                        perms.set_mode(0o755);
                                        let _ = std::fs::set_permissions(&bin_path, perms);
                                    }
                                    #[cfg(target_os = "macos")]
                                    {
                                        let _ = std::process::Command::new("xattr")
                                            .args(["-cr", &bin_path.to_string_lossy()])
                                            .output();
                                    }
                                }
                            }
                            let _ = std::fs::remove_dir_all(&tmp_extract);
                            let _ = std::fs::remove_file(&archive_path);
                            if bin_path.exists() {
                                break;
                            }
                        }
                    }
                }
            }
        }
        if !bin_path.exists() {
            return Err("下载 Laya GGUF 原生推理引擎失败，请检查网络连接后重试".into());
        }
    }

    // 2. 若本地尚无合法的 laya_multilingual_q8_0.gguf，则从 mys/laya-multilingual-GGUF 下载（约 345MB）
    if !is_valid_gguf(&target) {
        let urls = [
            "https://hf-mirror.com/mys/laya-multilingual-GGUF/resolve/main/laya_multilingual_q8_0.gguf",
            "https://huggingface.co/mys/laya-multilingual-GGUF/resolve/main/laya_multilingual_q8_0.gguf",
        ];

        let mut success = false;
        let mut last_err = String::new();

        for url in urls {
            let resp = match client.get(url).send().await {
                Ok(r) if r.status().is_success() => r,
                Ok(r) => {
                    last_err = format!("HTTP {}", r.status());
                    continue;
                }
                Err(e) => {
                    last_err = e.to_string();
                    continue;
                }
            };

            let total_bytes = resp.content_length().unwrap_or(361_712_736);
            let total_mb = (total_bytes as f64 / (1024.0 * 1024.0) * 10.0).round() / 10.0;

            let part_path = target.with_extension("gguf.part");
            let mut file = match tokio::fs::File::create(&part_path).await {
                Ok(f) => f,
                Err(e) => return Err(format!("创建临时文件失败: {e}")),
            };

            let mut stream = resp.bytes_stream();
            let mut downloaded: u64 = 0;
            let start_time = std::time::Instant::now();
            let mut last_emit = std::time::Instant::now();
            let mut failed = false;

            while let Some(chunk_result) = stream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(e) => {
                        failed = true;
                        last_err = e.to_string();
                        break;
                    }
                };

                if let Err(e) = file.write_all(&chunk).await {
                    failed = true;
                    last_err = e.to_string();
                    break;
                }

                downloaded += chunk.len() as u64;

                if last_emit.elapsed() >= Duration::from_millis(180) || downloaded == total_bytes {
                    let downloaded_mb =
                        (downloaded as f64 / (1024.0 * 1024.0) * 10.0).round() / 10.0;
                    let percent = if total_bytes > 0 {
                        ((downloaded as f64 / total_bytes as f64) * 100.0).clamp(0.0, 100.0)
                    } else {
                        0.0
                    };
                    let elapsed_sec = start_time.elapsed().as_secs_f64().max(0.1);
                    let speed_mbps = (downloaded_mb / elapsed_sec * 10.0).round() / 10.0;

                    let _ = on_prog.send(crate::engine::DownloadProgress {
                        target: "laya".into(),
                        stage: "downloading".into(),
                        file: "laya_multilingual_q8_0.gguf".into(),
                        downloaded_mb,
                        total_mb,
                        percent: percent as u32,
                        message: format!(
                            "正在下载 laya_multilingual_q8_0.gguf… {speed_mbps} MB/s"
                        ),
                    });
                    last_emit = std::time::Instant::now();
                }
            }

            let _ = file.flush().await;
            drop(file);

            if !failed && is_valid_gguf(&part_path) {
                let _ = tokio::fs::rename(&part_path, &target).await;
                success = true;
                break;
            } else {
                if !failed {
                    last_err = "下载的文件未通过 GGUF 格式头校验".to_string();
                }
                let _ = tokio::fs::remove_file(&part_path).await;
            }
        }

        if !success {
            return Err(format!("下载 GGUF 加速模型失败: {last_err}"));
        }
    }

    // 3. 下载并校验通过后，清除重下标记并自动启用极速模式
    let _ = std::fs::remove_file(dir.join(".needs_gguf_redownload"));
    let _ = std::fs::write(dir.join(".gguf_auto_enabled"), b"1");
    let st = app.state::<crate::AppState>();
    let mut s = st.settings.get();
    s.knowledge.laya_enabled = true;
    let _ = st.settings.set(s);

    let _ = on_prog.send(crate::engine::DownloadProgress {
        target: "laya".into(),
        stage: "done".into(),
        file: "laya_multilingual_q8_0.gguf".into(),
        downloaded_mb: 345.0,
        total_mb: 345.0,
        percent: 100,
        message: "Laya GGUF 加速模型已下载就绪并自动启用".into(),
    });

    Ok(check_laya_status(app))
}

pub fn delete_laya_model(app: &AppHandle) -> Result<(), String> {
    stop_laya_daemon();
    let dir = get_laya_dir();
    let target = get_laya_file_path();
    if target.exists() {
        std::fs::remove_file(&target).map_err(|e| format!("删除模型文件失败: {e}"))?;
    }
    let _ = cleanup_legacy_laya_files(&dir);
    let _ = std::fs::remove_file(dir.join(".gguf_auto_enabled"));
    let _ = std::fs::remove_file(dir.join(".needs_gguf_redownload"));
    let st = app.state::<crate::AppState>();
    let mut s = st.settings.get();
    s.knowledge.laya_enabled = false;
    let _ = st.settings.set(s);
    Ok(())
}

/// 用系统默认关联程序打开知识库文档或原图
pub fn open_kb_file(path: &str) -> Result<(), String> {
    let p = Path::new(path);
    if !p.exists() {
        return Err(format!("文件不存在: {path}"));
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", path])
            .spawn()
            .map_err(|e| format!("打开文件失败: {e}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| format!("打开文件失败: {e}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| format!("打开文件失败: {e}"))?;
    }
    Ok(())
}

/// 在系统文件资源管理器中定位并选中该文件
pub fn reveal_kb_file(path: &str) -> Result<(), String> {
    let p = Path::new(path);
    if !p.exists() {
        return Err(format!("文件不存在: {path}"));
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer.exe")
            .arg(format!("/select,{}", p.to_string_lossy()))
            .spawn()
            .map_err(|e| format!("定位文件失败: {e}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", &p.to_string_lossy()])
            .spawn()
            .map_err(|e| format!("定位文件失败: {e}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(parent) = p.parent() {
            std::process::Command::new("xdg-open")
                .arg(parent)
                .spawn()
                .map_err(|e| format!("定位文件失败: {e}"))?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocFileInfo {
    pub path: String,
    pub name: String,
    pub content: String,
    pub size: u64,
    pub modified_ms: u64,
    pub is_kb_file: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocWriteResult {
    pub path: String,
    pub size: u64,
    pub modified_ms: u64,
    pub kb_reindexed: bool,
}

/// 判断路径是否属于参与检索索引的知识库可见文档（排除 .prompts / .sag 等隐藏目录）
fn is_visible_kb_file(app: &AppHandle, p: &Path) -> bool {
    let kb_root = get_kb_root(app);
    if !p.starts_with(&kb_root) {
        return false;
    }
    if let Ok(rel) = p.strip_prefix(&kb_root) {
        for comp in rel.components() {
            if comp.as_os_str().to_string_lossy().starts_with('.') {
                return false;
            }
        }
    }
    true
}

/// 读取本地 Markdown / 文本文件内容与元数据
pub fn read_doc_file(app: &AppHandle, path: &str) -> Result<DocFileInfo, String> {
    let p = Path::new(path);
    if !p.exists() {
        return Err(format!("文件不存在: {path}"));
    }
    if !p.is_file() {
        return Err(format!("目标路径不是文件: {path}"));
    }
    let metadata = std::fs::metadata(p).map_err(|e| format!("获取文件元数据失败: {e}"))?;
    let size = metadata.len();
    if size > 15 * 1024 * 1024 {
        return Err(format!("文件过大 ({} MB)，超出轻量编辑器限制", size / 1024 / 1024));
    }
    let content = std::fs::read_to_string(p).map_err(|e| format!("读取文件内容失败 (需为有效 UTF-8 文本): {e}"))?;
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let name = p
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "未命名.md".to_string());

    let is_kb_file = is_visible_kb_file(app, p);

    Ok(DocFileInfo {
        path: path.to_string(),
        name,
        content,
        size,
        modified_ms,
        is_kb_file,
    })
}

/// 安全保存文档内容；如果是知识库目录下的文件，则自动增量更新 SQLite 索引
pub fn write_doc_file(app: &AppHandle, path: &str, content: &str) -> Result<DocWriteResult, String> {
    let p = Path::new(path);
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp_path = format!("{}.mb_tmp", path);
    if let Err(e) = std::fs::write(&tmp_path, content.as_bytes()) {
        return Err(format!("写入临时文件失败: {e}"));
    }
    if let Err(e) = std::fs::rename(&tmp_path, p) {
        let _ = std::fs::remove_file(&tmp_path);
        std::fs::write(p, content.as_bytes()).map_err(|e2| format!("保存文件失败: {e2} (rename: {e})"))?;
    }

    let metadata = std::fs::metadata(p).map_err(|e| format!("读取更新后元数据失败: {e}"))?;
    let size = metadata.len();
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let is_kb_file = is_visible_kb_file(app, p);
    let mut kb_reindexed = false;

    if is_kb_file {
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            let root = get_kb_root(&app_clone);
            let _ = tokio::task::spawn_blocking(move || sync_kb_index(&root)).await;
        });
        kb_reindexed = true;
    }

    Ok(DocWriteResult {
        path: path.to_string(),
        size,
        modified_ms,
        kb_reindexed,
    })
}

pub fn is_md_context_menu_registered() -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let out = std::process::Command::new("reg")
            .args(["query", r"HKCU\Software\Classes\SystemFileAssociations\.md\shell\miniBotMarkdownEditor"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        if let Ok(o) = out {
            return o.status.success();
        }
    }
    false
}

pub fn set_md_context_menu(enable: bool) -> Result<bool, String> {
    let exe = std::env::current_exe().map_err(|e| format!("无法获取当前程序路径: {e}"))?;
    let exe_str = exe.to_string_lossy().to_string();

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        let exts = [".md", ".markdown", ".mdown", ".mdx", ".txt"];
        let cmd_val = format!("\"{exe_str}\" --edit \"%1\"");
        let icon_val = format!("\"{exe_str}\",0");

        for ext in exts {
            let key = format!(r"HKCU\Software\Classes\SystemFileAssociations\{ext}\shell\miniBotMarkdownEditor");
            if enable {
                let _ = std::process::Command::new("reg")
                    .args(["add", &key, "/ve", "/d", "📝 使用 miniBot 编辑 Markdown", "/f"])
                    .creation_flags(CREATE_NO_WINDOW)
                    .output();
                let _ = std::process::Command::new("reg")
                    .args(["add", &key, "/v", "Icon", "/d", &icon_val, "/f"])
                    .creation_flags(CREATE_NO_WINDOW)
                    .output();
                let _ = std::process::Command::new("reg")
                    .args(["add", &format!(r"{key}\command"), "/ve", "/d", &cmd_val, "/f"])
                    .creation_flags(CREATE_NO_WINDOW)
                    .output();
            } else {
                let _ = std::process::Command::new("reg")
                    .args(["delete", &key, "/f"])
                    .creation_flags(CREATE_NO_WINDOW)
                    .output();
            }
        }
        return Ok(is_md_context_menu_registered());
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (enable, exe_str);
        Ok(false)
    }
}

/// 读取本地图片文件的 Base64 数据 URL（用于前端安全免跨域展示缩略图）
pub fn read_image_base64(path: &str) -> Result<String, String> {
    let p = Path::new(path);
    if !p.exists() {
        return Err(format!("图片文件不存在: {path}"));
    }
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_else(|| "png".to_string());

    let mime = match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        _ => "image/png",
    };

    let bytes = std::fs::read(p).map_err(|e| format!("读取图片文件失败: {e}"))?;
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{mime};base64,{b64}"))
}

// ==================== 知识库提示词模板（视觉提取 + 片段提纯与总结） ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTemplate {
    pub category: String,
    pub id: String,
    pub name: String,
    pub title: String,
    pub description: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub path: String,
    pub is_builtin: bool,
    pub modified_ms: u64,
    pub content: String,
    pub body: String,
}

const BUILTIN_PROMPTS_README: &str = include_str!("../prompts/README.md");

/// 内置提示词模板清单（人物类仅保留唯一模板 vision/character，其余场景/界面/静物/图表各自独立）
const BUILTIN_PROMPTS: &[(&str, &str, &str)] = &[
    ("vision", "classifier", include_str!("../prompts/vision/classifier.md")),
    ("vision", "character", include_str!("../prompts/vision/character.md")),
    ("vision", "scenery", include_str!("../prompts/vision/scenery.md")),
    ("vision", "ui_software", include_str!("../prompts/vision/ui_software.md")),
    ("vision", "object_food", include_str!("../prompts/vision/object_food.md")),
    ("vision", "document_chart", include_str!("../prompts/vision/document_chart.md")),
    ("vision", "general", include_str!("../prompts/vision/general.md")),
    ("clip", "clip_distill", include_str!("../prompts/clip/clip_distill.md")),
    ("clip", "web_distill", include_str!("../prompts/clip/web_distill.md")),
];

/// 获取可编辑提示词工作根目录（位于 {kb_root}/.prompts，自动豁免 FTS5 知识库检索污染）
pub fn get_prompts_root(app: &AppHandle) -> PathBuf {
    get_kb_root(app).join(".prompts")
}

/// 优先从安装包资源目录 prompts/{category}/{id}.md 读取出厂模板，回退到编译期嵌入内容
fn resolve_default_prompt_content(app: &AppHandle, category: &str, id: &str) -> Option<String> {
    if let Ok(res_dir) = app.path().resource_dir() {
        let candidate = res_dir.join("prompts").join(category).join(format!("{id}.md"));
        if let Ok(text) = std::fs::read_to_string(&candidate) {
            return Some(text);
        }
    }
    BUILTIN_PROMPTS
        .iter()
        .find(|(c, i, _)| *c == category && *i == id)
        .map(|(_, _, body)| (*body).to_string())
}

fn is_builtin_prompt(category: &str, id: &str) -> bool {
    BUILTIN_PROMPTS.iter().any(|(c, i, _)| *c == category && *i == id)
}

fn file_mtime_secs(p: &Path) -> u64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)))
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 确保工作区提示词目录存在，初始化唯一默认视觉模板 vision/character.md 与文本模板 clip/clip_distill.md；
/// 用户修改后的工作区提示词永远优先保留，绝不被内置版本覆盖。
pub fn ensure_prompts_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let prompts_root = get_prompts_root(app);
    let vision_dir = prompts_root.join("vision");
    let clip_dir = prompts_root.join("clip");
    std::fs::create_dir_all(&vision_dir).map_err(|e| format!("创建视觉提示词目录失败: {e}"))?;
    std::fs::create_dir_all(&clip_dir).map_err(|e| format!("创建片段提示词目录失败: {e}"))?;

    let readme_path = prompts_root.join("README.md");
    if !readme_path.exists() {
        let readme_text = app
            .path()
            .resource_dir()
            .ok()
            .and_then(|d| std::fs::read_to_string(d.join("prompts").join("README.md")).ok())
            .unwrap_or_else(|| BUILTIN_PROMPTS_README.to_string());
        let _ = std::fs::write(&readme_path, readme_text.as_bytes());
    }

    let init_marker = prompts_root.join(".initialized");
    let first_init = !init_marker.exists();
    if first_init {
        for (cat, id, _) in BUILTIN_PROMPTS {
            let target = prompts_root.join(cat).join(format!("{id}.md"));
            if !target.exists() {
                if let Some(content) = resolve_default_prompt_content(app, cat, id) {
                    let _ = std::fs::write(&target, content.as_bytes());
                }
            }
        }
        let _ = std::fs::write(&init_marker, b"1");
    }

    // 若用户在安装目录 prompts/ 内放置或更新了文件（mtime 晚于 exe 10 秒以上且比工作区更新），热同步到工作区
    let exe_mtime = std::env::current_exe()
        .ok()
        .map(|p| file_mtime_secs(&p))
        .unwrap_or(0);
    if let Ok(res_dir) = app.path().resource_dir() {
        let src_root = res_dir.join("prompts");
        for cat in ["vision", "clip"] {
            let src_cat = src_root.join(cat);
            let Ok(entries) = std::fs::read_dir(&src_cat) else {
                continue;
            };
            for e in entries.flatten() {
                let src_file = e.path();
                if !src_file.is_file() {
                    continue;
                }
                let is_md = src_file
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(|s| s.eq_ignore_ascii_case("md"))
                    .unwrap_or(false);
                if !is_md {
                    continue;
                }
                let Some(fname) = src_file.file_name() else {
                    continue;
                };
                let dst_file = prompts_root.join(cat).join(fname);
                let should_copy = if !dst_file.exists() {
                    first_init
                } else {
                    let src_m = file_mtime_secs(&src_file);
                    let dst_m = file_mtime_secs(&dst_file);
                    exe_mtime > 0 && src_m > exe_mtime.saturating_add(10) && src_m > dst_m
                };
                if should_copy {
                    if let Ok(content) = std::fs::read(&src_file) {
                        let _ = std::fs::write(&dst_file, content);
                    }
                }
            }
        }
    }

    Ok(prompts_root)
}

/// 解析提示词 Markdown 文件的 YAML Frontmatter 与正文
fn parse_prompt_markdown(
    category: &str,
    stem: &str,
    path: &Path,
    raw: &str,
    modified_ms: u64,
) -> PromptTemplate {
    let mut id = stem.to_string();
    let mut name = stem.to_string();
    let mut title = format!("🖼️ {stem}");
    let mut description = String::new();
    let mut max_tokens: u32 = if category == "vision" && stem == "classifier" {
        128
    } else if category == "clip" {
        1536
    } else {
        4096
    };
    let mut temperature: f32 = if category == "vision" && stem == "classifier" {
        0.1
    } else {
        0.2
    };
    let mut body = raw.trim().to_string();

    let normalized = raw.replace("\r\n", "\n");
    if let Some(after_open) = normalized.strip_prefix("---\n") {
        if let Some(close_idx) = after_open.find("\n---\n").or_else(|| after_open.find("\n---")) {
            let fm = &after_open[..close_idx];
            let rest = &after_open[close_idx + 4..];
            body = rest.trim_start_matches('\n').trim().to_string();

            for line in fm.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                if let Some((k, v)) = trimmed.split_once(':') {
                    let key = k.trim();
                    let val = v.trim().trim_matches('"').trim_matches('\'').trim();
                    if val.is_empty() {
                        continue;
                    }
                    match key {
                        "id" => id = val.to_string(),
                        "name" => name = val.to_string(),
                        "title" => title = val.to_string(),
                        "description" => description = val.to_string(),
                        "max_tokens" => {
                            if let Ok(n) = val.parse::<u32>() {
                                max_tokens = n.clamp(16, 16384);
                            }
                        }
                        "temperature" => {
                            if let Ok(t) = val.parse::<f32>() {
                                temperature = t.clamp(0.0, 2.0);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    PromptTemplate {
        category: category.to_string(),
        id,
        name,
        title,
        description,
        max_tokens,
        temperature,
        path: path.to_string_lossy().to_string(),
        is_builtin: is_builtin_prompt(category, stem),
        modified_ms,
        content: raw.to_string(),
        body,
    }
}

fn read_prompt_file_as_template(category: &str, path: &Path) -> Result<PromptTemplate, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("读取提示词文件失败: {e}"))?;
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "custom".to_string());
    let modified_ms = std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Ok(parse_prompt_markdown(category, &stem, path, &raw, modified_ms))
}

/// 按类别和 ID 按需读取单个提示词模板（不同类别彼此隔离，按需加载）
pub fn get_prompt_template(app: &AppHandle, category: &str, id: &str) -> Result<PromptTemplate, String> {
    let prompts_root = ensure_prompts_dir(app)?;
    let safe_cat = match category {
        "vision" | "clip" => category,
        _ => return Err(format!("不支持的提示词分类: {category}")),
    };
    let clean_id: String = id
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if clean_id.is_empty() {
        return Err("提示词 ID 无效".into());
    }
    let file_path = prompts_root.join(safe_cat).join(format!("{clean_id}.md"));
    if !file_path.exists() {
        return Err(format!("提示词文件不存在: {safe_cat}/{clean_id}.md"));
    }
    read_prompt_file_as_template(safe_cat, &file_path)
}

/// 列出所有视觉提取与片段提纯提示词模板（内置序优先 + 用户自定义 .md）
pub fn list_prompt_templates(app: &AppHandle) -> Result<Vec<PromptTemplate>, String> {
    let prompts_root = ensure_prompts_dir(app)?;
    let mut out = Vec::new();

    for cat in ["vision", "clip"] {
        let dir = prompts_root.join(cat);
        let mut custom_files = Vec::new();

        // 先按内置顺序加载，保证 UI 展示与路由稳定有序
        let mut seen_ids = std::collections::HashSet::new();
        for (b_cat, b_id, _) in BUILTIN_PROMPTS {
            if *b_cat != cat {
                continue;
            }
            let p = dir.join(format!("{b_id}.md"));
            if p.exists() {
                if let Ok(tpl) = read_prompt_file_as_template(cat, &p) {
                    seen_ids.insert(format!("{b_id}.md").to_lowercase());
                    out.push(tpl);
                }
            }
        }

        // 再扫描用户自行添加的自定义 .md 模板
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let p = e.path();
                if !p.is_file() {
                    continue;
                }
                let is_md = p
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(|s| s.eq_ignore_ascii_case("md"))
                    .unwrap_or(false);
                if !is_md {
                    continue;
                }
                let fname = e.file_name().to_string_lossy().to_string();
                if fname.eq_ignore_ascii_case("readme.md") || seen_ids.contains(&fname.to_lowercase()) {
                    continue;
                }
                custom_files.push(p);
            }
        }
        custom_files.sort();
        for p in custom_files {
            if let Ok(tpl) = read_prompt_file_as_template(cat, &p) {
                out.push(tpl);
            }
        }
    }

    Ok(out)
}

/// 恢复指定内置提示词（或全部内置提示词）为出厂默认模板
pub fn reset_prompt_template(
    app: &AppHandle,
    category: Option<&str>,
    id: Option<&str>,
) -> Result<Vec<PromptTemplate>, String> {
    let prompts_root = ensure_prompts_dir(app)?;
    match (category, id) {
        (Some(cat), Some(target_id)) => {
            let content = resolve_default_prompt_content(app, cat, target_id)
                .ok_or_else(|| format!("未找到内置默认模板: {cat}/{target_id}.md"))?;
            let path = prompts_root.join(cat).join(format!("{target_id}.md"));
            std::fs::write(&path, content.as_bytes())
                .map_err(|e| format!("重置模板文件失败: {e}"))?;
        }
        _ => {
            for (cat, target_id, _) in BUILTIN_PROMPTS {
                if let Some(filter_cat) = category {
                    if *cat != filter_cat {
                        continue;
                    }
                }
                if let Some(content) = resolve_default_prompt_content(app, cat, target_id) {
                    let path = prompts_root.join(cat).join(format!("{target_id}.md"));
                    std::fs::write(&path, content.as_bytes())
                        .map_err(|e| format!("重置模板文件失败: {e}"))?;
                }
            }
        }
    }
    list_prompt_templates(app)
}

/// 新建一个自定义提示词模板 .md 文件，方便扩展新图像分类或新提纯规则
pub fn create_prompt_template(
    app: &AppHandle,
    category: &str,
    id: &str,
    name: Option<&str>,
) -> Result<PromptTemplate, String> {
    let prompts_root = ensure_prompts_dir(app)?;
    let safe_cat = match category {
        "vision" | "clip" => category,
        _ => return Err("分类必须为 vision 或 clip".into()),
    };
    let clean_id: String = id
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    let clean_id = clean_id.trim_matches('_').to_string();
    if clean_id.is_empty() {
        return Err("模板英文标识 ID 不能为空（建议使用英文字母、数字或下划线，如 anime_cg）".into());
    }

    let path = prompts_root.join(safe_cat).join(format!("{clean_id}.md"));
    if path.exists() {
        return Err(format!("模板 {safe_cat}/{clean_id}.md 已存在"));
    }

    let display_name = name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&clean_id);

    let skeleton = if safe_cat == "vision" {
        format!(
            "---\nid: {clean_id}\nname: {display_name}\ntitle: 🖼️ {display_name}深度解析\ndescription: 自定义视觉提取分类模板\nmax_tokens: 2048\ntemperature: 0.2\n---\n你是一位资深视觉资产分析与知识库索引专家。当前图片属于【{display_name}】类别。\n请对画面进行详尽、结构化、无遗漏的描摹，确保后续全文检索精准命中。\n\n{{{{HINT_CONTEXT}}}}\n{{{{OCR_CONTEXT}}}}\n### 1. 核心主体与场景情境\n\n### 2. 关键细节特征提取\n\n### 3. 全文检索高频关键词库 (Search Keywords)\n请列出 25~45 个逗号分隔的高频检索关键词。\n"
        )
    } else {
        format!(
            "---\nid: {clean_id}\nname: {display_name}\ntitle: ✂️ {display_name}\ndescription: 自定义文本片段提纯与总结模板\nmax_tokens: 1536\ntemperature: 0.2\n---\n你是一位知识提纯与总结专家。请使用【{display_name}】视角提炼以下片段的核心知识：\n\n{{{{SOURCE_CONTEXT}}}}\n```text\n{{{{CLIP_TEXT}}}}\n```\n\n#### 💡 核心结论\n\n#### 🧩 关键要点\n\n#### 🏷️ 检索关键词库\n"
        )
    };

    std::fs::write(&path, skeleton.as_bytes()).map_err(|e| format!("创建模板文件失败: {e}"))?;
    read_prompt_file_as_template(safe_cat, &path)
}

/// 在系统文件资源管理器中打开提示词文件夹（work=可编辑目录，builtin=安装资源目录）
pub fn open_prompts_folder(app: &AppHandle, target: Option<&str>) -> Result<String, String> {
    let dir = if target == Some("builtin") {
        app.path()
            .resource_dir()
            .map(|d| d.join("prompts"))
            .unwrap_or_else(|_| get_prompts_root(app))
    } else {
        ensure_prompts_dir(app)?
    };
    if !dir.exists() {
        let _ = std::fs::create_dir_all(&dir);
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&dir)
            .spawn()
            .map_err(|e| format!("打开提示词文件夹失败: {e}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&dir)
            .spawn()
            .map_err(|e| format!("打开提示词文件夹失败: {e}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&dir)
            .spawn()
            .map_err(|e| format!("打开提示词文件夹失败: {e}"))?;
    }

    Ok(dir.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_keywords() {
        let kw1 = extract_search_keywords("小雪是谁");
        assert!(kw1.contains(&"小雪".to_string()), "必须包含'小雪'，实际提取到: {:?}", kw1);

        let kw2 = extract_search_keywords("小雨的经常穿什么衣服");
        assert!(kw2.contains(&"小雨".to_string()), "必须包含'小雨'，实际提取到: {:?}", kw2);
    }

    #[test]
    fn test_builtin_prompts_frontmatter() {
        assert_eq!(
            BUILTIN_PROMPTS.len(),
            9,
            "内置提示词模板应包含 7 个视觉模板（人物仅 character）与 2 个片段模板"
        );
        assert!(
            !BUILTIN_PROMPTS.iter().any(|(_, id, _)| *id == "couple_intimate"),
            "人物类仅保留唯一的 character，不应包含 couple_intimate"
        );
        for (cat, id, content) in BUILTIN_PROMPTS {
            let tpl = parse_prompt_markdown(cat, id, Path::new("dummy.md"), content, 0);
            assert_eq!(tpl.id, *id, "模板 id 不匹配: {cat}/{id}");
            assert!(!tpl.name.is_empty(), "模板 name 为空: {cat}/{id}");
            assert!(!tpl.body.is_empty(), "模板正文为空: {cat}/{id}");
            assert!(tpl.max_tokens >= 16);
        }

        // 无 Frontmatter 时优雅降级
        let plain = parse_prompt_markdown("vision", "custom", Path::new("custom.md"), "直接描述图片细节", 123);
        assert_eq!(plain.id, "custom");
        assert_eq!(plain.body, "直接描述图片细节");
    }

    #[test]
    fn test_laya_gguf_validation_and_legacy_cleanup() {
        let tmp_dir = std::env::temp_dir().join(format!("minibot_laya_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp_dir);
        std::fs::create_dir_all(&tmp_dir).unwrap();

        let fake_safetensors = tmp_dir.join("model.safetensors");
        let broken_gguf = tmp_dir.join("broken.gguf");
        std::fs::write(&fake_safetensors, b"not a gguf file").unwrap();
        std::fs::write(&broken_gguf, b"HTML error page").unwrap();

        assert!(!is_valid_gguf(&fake_safetensors));
        assert!(!is_valid_gguf(&broken_gguf));
        assert!(
            cleanup_legacy_laya_files(&tmp_dir),
            "应当识别并清理旧版 model.safetensors 与损坏的 .gguf"
        );
        assert!(!fake_safetensors.exists());
        assert!(!broken_gguf.exists());
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_laya_gguf_rerank_integration() {
        let gguf = get_laya_file_path();
        let bin = get_laya_bin_path();
        assert!(
            gguf.to_string_lossy().ends_with("laya_multilingual_q8_0.gguf"),
            "Laya 模型路径应指向 laya_multilingual_q8_0.gguf，实际为: {:?}",
            gguf
        );
        if gguf.exists() && bin.exists() {
            assert!(
                is_valid_gguf(&gguf),
                "本地 laya_multilingual_q8_0.gguf 必须通过 GGUF 魔数与体积校验"
            );
            let mut hits = vec![
                KbSearchHit {
                    path: "doc1.md".into(),
                    relative_path: "doc1.md".into(),
                    title: "办公时间".into(),
                    heading: "考勤规定".into(),
                    content: "我们的上班时间是周一至周五早上9点到下午18点。".into(),
                    line_start: 1,
                    line_end: 5,
                    score: 100.0,
                    origin_image: None,
                },
                KbSearchHit {
                    path: "doc2.md".into(),
                    relative_path: "doc2.md".into(),
                    title: "公司地址".into(),
                    heading: "联系方式".into(),
                    content: "公司总部地址位于北京市海淀区中关村软件园。".into(),
                    line_start: 1,
                    line_end: 5,
                    score: 95.0,
                    origin_image: None,
                },
            ];
            let res = laya_rerank_hits("公司地址在哪里？", &mut hits);
            stop_laya_daemon();
            assert!(res.is_some(), "Laya GGUF 重排应当成功返回评分数量与耗时");
            assert_eq!(
                hits[0].path, "doc2.md",
                "经过 Laya GGUF 神经语义重排后，原本分数较低的 doc2.md（公司地址）应排在第一位"
            );
        }
    }
}
