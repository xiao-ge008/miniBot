//! 统一内容载体：所有来源（剪贴板/截图/选中/输入框/文件）都转成 Payload，
//! 前端根据 kind 匹配动作注册表。新增内容类型（如视频）只需扩展 PayloadKind。
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PayloadKind {
    Text,
    Url,
    Color,
    Image,
    /// 预留：视频文件 / 视频链接
    Video,
    /// 预留：文件
    File,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    Clipboard,
    Selection,
    Capture,
    Launcher,
    External,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Payload {
    pub id: String,
    pub kind: PayloadKind,
    pub origin: Origin,
    #[serde(default)]
    pub text: Option<String>,
    /// 图片：data:image/png;base64,...
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub source_app: Option<String>,
    /// 来源窗口句柄，用于「替换原文」回填
    #[serde(default)]
    pub source_hwnd: isize,
    /// 扩展字段：qr 结果、截图区域、智能体预设等
    #[serde(default)]
    pub meta: serde_json::Value,
}

static SEQ: AtomicU64 = AtomicU64::new(1);

pub fn new_id() -> String {
    format!("p{}", SEQ.fetch_add(1, Ordering::SeqCst))
}

impl Payload {
    pub fn text(text: String, origin: Origin, source_app: Option<String>, hwnd: isize) -> Self {
        let kind = crate::clip::kind_of_text(&text);
        Self { id: new_id(), kind, origin, text: Some(text), image: None, source_app, source_hwnd: hwnd, meta: serde_json::Value::Null }
    }

    pub fn image(png: Vec<u8>, origin: Origin, source_app: Option<String>, hwnd: isize) -> Self {
        let b64 = base64::engine::general_purpose::STANDARD.encode(png);
        Self {
            id: new_id(),
            kind: PayloadKind::Image,
            origin,
            text: None,
            image: Some(format!("data:image/png;base64,{b64}")),
            source_app,
            source_hwnd: hwnd,
            meta: serde_json::Value::Null,
        }
    }
}
