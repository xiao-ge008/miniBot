//! 历史记录：SQLite
//! - `history` 表：划词翻译/解释/总结/润色、截图识别等一次性记录，默认保留 30 天自动清理
//! - `chat_sessions` / `chat_messages` 表：主 AI 聊天窗多轮对话记录，永久保存（仅用户手动删除时移除）
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

pub struct History {
    conn: Mutex<Connection>,
    img_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub ts: i64,
    pub action: String,
    pub kind: String,
    pub input: String,
    pub output: String,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub source_app: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSession {
    pub id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatAttachment {
    /// "image" | "video"
    pub kind: String,
    pub name: String,
    /// 供视觉模型识别与预览的 Data URI（视频为带时间戳的关键帧故事板拼图）
    pub data_url: String,
    /// 可选单帧封面 Data URI
    #[serde(default)]
    pub thumb_url: Option<String>,
    /// 媒体元信息描述（如时长、分辨率、抽帧数）
    #[serde(default)]
    pub info: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessageRecord {
    #[serde(default)]
    pub id: i64,
    pub session_id: String,
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub attachments: Vec<ChatAttachment>,
    #[serde(default)]
    pub stats: Option<crate::llm::ChatStats>,
    #[serde(default)]
    pub ts: i64,
}

impl History {
    pub fn open(dir: PathBuf) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        let img_dir = dir.join("history_images");
        std::fs::create_dir_all(&img_dir)?;
        let conn = Connection::open(dir.join("history.db"))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS history(
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               ts INTEGER NOT NULL,
               action TEXT NOT NULL,
               kind TEXT NOT NULL,
               input TEXT NOT NULL,
               output TEXT NOT NULL,
               image TEXT,
               source_app TEXT);
             CREATE INDEX IF NOT EXISTS idx_history_ts ON history(ts);

             CREATE TABLE IF NOT EXISTS chat_sessions(
               id TEXT PRIMARY KEY,
               title TEXT NOT NULL,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL);
             CREATE INDEX IF NOT EXISTS idx_chat_sessions_updated ON chat_sessions(updated_at DESC);

             CREATE TABLE IF NOT EXISTS chat_messages(
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               session_id TEXT NOT NULL,
               role TEXT NOT NULL,
               content TEXT NOT NULL,
               attachments_json TEXT NOT NULL DEFAULT '[]',
               stats_json TEXT,
               ts INTEGER NOT NULL,
               FOREIGN KEY(session_id) REFERENCES chat_sessions(id) ON DELETE CASCADE);
             CREATE INDEX IF NOT EXISTS idx_chat_messages_session ON chat_messages(session_id, id ASC);",
        )?;
        // 兼容旧版数据库平滑新增 stats_json 列
        let _ = conn.execute("ALTER TABLE chat_messages ADD COLUMN stats_json TEXT", []);
        Ok(Self { conn: Mutex::new(conn), img_dir })
    }

    /// 写入划词/截图识别等一次性历史记录，并顺带清理超过 30 天的旧数据
    pub fn add(&self, mut e: Entry, image_data: Option<String>, keep_images: bool) -> anyhow::Result<i64> {
        e.ts = chrono::Utc::now().timestamp();
        if let (Some(data), true) = (image_data, keep_images) {
            if let Ok(bytes) = crate::imgutil::decode_b64(&data) {
                if let Ok(img) = crate::imgutil::decode_rgba(&bytes) {
                    let path = self.img_dir.join(format!("{}_{}.webp", e.ts, crate::payload::new_id()));
                    let rgb = image::DynamicImage::ImageRgba8(img);
                    if rgb.save_with_format(&path, image::ImageFormat::WebP).is_ok() {
                        e.image = Some(path.to_string_lossy().into());
                    }
                }
            }
        }
        let row_id = {
            let c = self.conn.lock().unwrap();
            c.execute(
                "INSERT INTO history(ts,action,kind,input,output,image,source_app) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![e.ts, e.action, e.kind, e.input, e.output, e.image, e.source_app],
            )?;
            c.last_insert_rowid()
        };
        // 每次新增一次性划词/识图记录时自动清理超过 30 天的数据
        let _ = self.cleanup(30);
        Ok(row_id)
    }

    pub fn list(&self, query: &str, limit: u32) -> anyhow::Result<Vec<Entry>> {
        let c = self.conn.lock().unwrap();
        let like = format!("%{}%", query);
        let mut stmt = c.prepare(
            "SELECT id,ts,action,kind,input,output,image,source_app FROM history
             WHERE (?1 = '%%' OR input LIKE ?1 OR output LIKE ?1)
             ORDER BY ts DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![like, limit], |r| {
            Ok(Entry {
                id: r.get(0)?,
                ts: r.get(1)?,
                action: r.get(2)?,
                kind: r.get(3)?,
                input: r.get(4)?,
                output: r.get(5)?,
                image: r.get(6)?,
                source_app: r.get(7)?,
            })
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn delete(&self, id: i64) -> anyhow::Result<()> {
        let c = self.conn.lock().unwrap();
        let img: Option<String> = c.query_row("SELECT image FROM history WHERE id=?1", [id], |r| r.get(0)).ok().flatten();
        if let Some(p) = img {
            let _ = std::fs::remove_file(p);
        }
        c.execute("DELETE FROM history WHERE id=?1", [id])?;
        Ok(())
    }

    pub fn clear(&self) -> anyhow::Result<()> {
        self.conn.lock().unwrap().execute("DELETE FROM history", [])?;
        let _ = std::fs::remove_dir_all(&self.img_dir);
        let _ = std::fs::create_dir_all(&self.img_dir);
        Ok(())
    }

    /// 删除超过 days 天的划词/识图临时记录及其图片（不影响 chat_sessions 永久聊天记录）
    pub fn cleanup(&self, days: u32) -> anyhow::Result<usize> {
        let cutoff = chrono::Utc::now().timestamp() - days as i64 * 86400;
        let c = self.conn.lock().unwrap();
        let mut stmt = c.prepare("SELECT image FROM history WHERE ts < ?1 AND image IS NOT NULL")?;
        let imgs: Vec<String> = stmt.query_map([cutoff], |r| r.get(0))?.filter_map(Result::ok).collect();
        for p in imgs {
            let _ = std::fs::remove_file(p);
        }
        Ok(c.execute("DELETE FROM history WHERE ts < ?1", [cutoff])?)
    }

    // ---------- 永久多轮 AI 聊天会话管理 ----------

    pub fn chat_session_list(&self) -> anyhow::Result<Vec<ChatSession>> {
        let c = self.conn.lock().unwrap();
        let mut stmt = c.prepare(
            "SELECT id, title, created_at, updated_at FROM chat_sessions ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ChatSession {
                id: r.get(0)?,
                title: r.get(1)?,
                created_at: r.get(2)?,
                updated_at: r.get(3)?,
            })
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn chat_session_create(&self, title: String) -> anyhow::Result<ChatSession> {
        let now = chrono::Utc::now().timestamp();
        let id = format!("s_{}_{}", now, crate::payload::new_id());
        let title = if title.trim().is_empty() { "新对话".to_string() } else { title };
        let c = self.conn.lock().unwrap();
        c.execute(
            "INSERT INTO chat_sessions(id, title, created_at, updated_at) VALUES(?1, ?2, ?3, ?4)",
            params![id, title, now, now],
        )?;
        Ok(ChatSession { id, title, created_at: now, updated_at: now })
    }

    pub fn chat_session_rename(&self, id: &str, title: &str) -> anyhow::Result<()> {
        let c = self.conn.lock().unwrap();
        c.execute("UPDATE chat_sessions SET title=?1 WHERE id=?2", params![title, id])?;
        Ok(())
    }

    pub fn chat_session_delete(&self, id: &str) -> anyhow::Result<()> {
        let c = self.conn.lock().unwrap();
        c.execute("DELETE FROM chat_messages WHERE session_id=?1", [id])?;
        c.execute("DELETE FROM chat_sessions WHERE id=?1", [id])?;
        Ok(())
    }

    pub fn chat_message_list(&self, session_id: &str) -> anyhow::Result<Vec<ChatMessageRecord>> {
        let c = self.conn.lock().unwrap();
        let mut stmt = c.prepare(
            "SELECT id, session_id, role, content, attachments_json, stats_json, ts
             FROM chat_messages
             WHERE session_id=?1
             ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([session_id], |r| {
            let att_str: String = r.get(4)?;
            let attachments: Vec<ChatAttachment> = serde_json::from_str(&att_str).unwrap_or_default();
            let stats_str: Option<String> = r.get(5).ok().flatten();
            let stats: Option<crate::llm::ChatStats> = stats_str
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok());
            Ok(ChatMessageRecord {
                id: r.get(0)?,
                session_id: r.get(1)?,
                role: r.get(2)?,
                content: r.get(3)?,
                attachments,
                stats,
                ts: r.get(6)?,
            })
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn chat_message_add(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        attachments: &[ChatAttachment],
        stats: Option<&crate::llm::ChatStats>,
    ) -> anyhow::Result<i64> {
        let now = chrono::Utc::now().timestamp();
        let att_json = serde_json::to_string(attachments).unwrap_or_else(|_| "[]".to_string());
        let stats_json = stats.and_then(|s| serde_json::to_string(s).ok());
        let c = self.conn.lock().unwrap();
        c.execute(
            "INSERT INTO chat_messages(session_id, role, content, attachments_json, stats_json, ts) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![session_id, role, content, att_json, stats_json, now],
        )?;
        let msg_id = c.last_insert_rowid();

        // 若当前会话标题仍是「新对话」且这是用户消息，自动用消息内容或附件名生成标题
        if role == "user" {
            let cur_title: Option<String> = c
                .query_row("SELECT title FROM chat_sessions WHERE id=?1", [session_id], |r| r.get(0))
                .ok();
            if cur_title.as_deref() == Some("新对话") {
                let raw = content.trim();
                let new_title = if !raw.is_empty() {
                    let s: String = raw.chars().take(22).collect();
                    if raw.chars().count() > 22 { format!("{s}…") } else { s }
                } else if let Some(first) = attachments.first() {
                    let prefix = if first.kind == "video" { "🎬 " } else { "🖼 " };
                    format!("{prefix}{}", first.name)
                } else {
                    "新对话".to_string()
                };
                let _ = c.execute(
                    "UPDATE chat_sessions SET title=?1, updated_at=?2 WHERE id=?3",
                    params![new_title, now, session_id],
                );
            } else {
                let _ = c.execute("UPDATE chat_sessions SET updated_at=?1 WHERE id=?2", params![now, session_id]);
            }
        } else {
            let _ = c.execute("UPDATE chat_sessions SET updated_at=?1 WHERE id=?2", params![now, session_id]);
        }

        Ok(msg_id)
    }

    /// 删除指定会话中从 `from_id`（含）开始的所有消息，用于重新生成时彻底清理被污染的上下文
    pub fn chat_message_delete_from(&self, session_id: &str, from_id: i64) -> anyhow::Result<()> {
        let c = self.conn.lock().unwrap();
        c.execute(
            "DELETE FROM chat_messages WHERE session_id=?1 AND id >= ?2",
            params![session_id, from_id],
        )?;
        Ok(())
    }
}
