//! 设置：JSON 文件持久化，位于 %APPDATA%\com.minibot.app\settings.json
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub capture: String,
    pub record: String,
    pub launcher: String,
    pub selection: String,
    pub color: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            capture: "Alt+Q".into(),
            record: "Alt+R".into(),
            launcher: "Alt+Space".into(),
            selection: "Alt+W".into(),
            color: "Alt+C".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelParams {
    /// 上下文窗口长度（默认 32768 = 32K）
    pub ctx: u32,
    /// GPU 卸载层数（默认 99 表示全部加载至显卡，0 表示纯 CPU）
    pub ngl: i32,
    /// KV Cache 量化类型："q8_0"（默认） | "f16" | "q4_0"
    pub kv_quant: String,
    /// 默认思考等级："off"（默认） | "low" | "medium" | "high" | "max"
    pub reasoning_effort: String,
    /// 闲置多少秒后自动释放显存（默认 180 秒）
    pub idle_secs: u64,
    /// 额外启动参数（如 --threads 8）
    pub extra_args: String,
}

pub fn normalize_reasoning_effort(v: &str) -> String {
    match v.trim().to_lowercase().as_str() {
        "off" | "low" | "medium" | "high" | "max" => v.trim().to_lowercase(),
        _ => "off".into(),
    }
}

impl Default for ModelParams {
    fn default() -> Self {
        Self {
            ctx: 32768,
            ngl: 99,
            kv_quant: "q8_0".into(),
            reasoning_effort: "off".into(),
            idle_secs: 180,
            extra_args: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomLocalModel {
    pub id: String,
    pub name: String,
    pub desc: String,
    pub folder_path: String,
    pub model_path: String,
    pub mmproj_path: String,
    pub vision: bool,
    pub quant_tag: String,
    pub size_label: String,
    pub params: ModelParams,
}

impl Default for CustomLocalModel {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            desc: String::new(),
            folder_path: String::new(),
            model_path: String::new(),
            mmproj_path: String::new(),
            vision: false,
            quant_tag: "GGUF".into(),
            size_label: String::new(),
            params: ModelParams::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmSettings {
    /// 本地推理引擎可执行文件路径
    pub server_path: String,
    /// 默认模型统一存放目录（固定为 llama/models 目录）
    pub models_dir: String,
    pub model_path: String,
    pub mmproj_path: String,
    /// 模型下载源：modelscope（默认） | hf-mirror | huggingface
    pub mirror: String,
    pub port: u16,
    pub ctx: u32,
    /// GPU 卸载层数（默认 99）
    pub ngl: i32,
    /// KV Cache 量化精度（默认 q8_0）
    pub kv_quant: String,
    /// 当前激活模型的默认思考等级："off" | "low" | "medium" | "high" | "max"
    pub reasoning_effort: String,
    /// 闲置多少秒后释放模型
    pub idle_secs: u64,
    /// 预估所需显存（MB），低于该值触发让道检查
    pub required_vram_mb: u64,
    pub extra_args: String,
    /// 每个模型（如 q4m / q2s / custom_xxx）各自独立的参数配置
    pub model_params: std::collections::HashMap<String, ModelParams>,
    /// 用户自定义的多个本地模型列表
    pub custom_models: Vec<CustomLocalModel>,
    /// 是否开启云端 OpenAI 兼容模型 API（开启后所有对话/划词/识图均走云端 API，无需本地显卡与高配 CPU）
    pub api_enabled: bool,
    /// 云端服务商标识：siliconflow | deepseek | dashscope | zhipu | doubao | moonshot | openai | ollama | custom
    pub api_provider: String,
    /// 云端 OpenAI 兼容接口地址（如 https://api.siliconflow.cn/v1）
    pub api_base_url: String,
    /// 云端 API Key
    pub api_key: String,
    /// 云端模型名称（如 Qwen/Qwen2.5-VL-72B-Instruct、deepseek-chat）
    pub api_model: String,
    /// 云端模型是否支持视觉多模态（Vision）；若为 false，识图/传图时自动使用本地离线 OCR 提取文字后发给云端模型
    pub api_vision: bool,
}

pub fn default_server_bin_name() -> &'static str {
    if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    }
}

impl Default for LlmSettings {
    fn default() -> Self {
        let base = default_engine_dir();
        let models_dir = base.join("models");
        // 若存在默认内置 LFM3 子目录则优先指向已有模型，否则指向默认 Q4_K_M 路径
        let (default_model, default_mmproj) = if models_dir.join("LFM3/LFM3-VL-2B-heretic-Q4_K_M.gguf").exists() {
            (
                models_dir.join("LFM3/LFM3-VL-2B-heretic-Q4_K_M.gguf"),
                models_dir.join("LFM3/LFM3-VL-2B-heretic-mmproj-F16.gguf"),
            )
        } else {
            (
                models_dir.join("LFM3-VL-2B-Q4_K_M.gguf"),
                models_dir.join("mmproj-LFM3-VL-2B-f16.gguf"),
            )
        };
        Self {
            server_path: base.join(default_server_bin_name()).to_string_lossy().into(),
            models_dir: models_dir.to_string_lossy().into(),
            model_path: default_model.to_string_lossy().into(),
            mmproj_path: default_mmproj.to_string_lossy().into(),
            mirror: "modelscope".into(),
            port: 18080,
            ctx: 32768,
            ngl: 99,
            kv_quant: "q8_0".into(),
            reasoning_effort: "off".into(),
            idle_secs: 180,
            required_vram_mb: 2400,
            extra_args: String::new(),
            model_params: std::collections::HashMap::new(),
            custom_models: Vec::new(),
            api_enabled: false,
            api_provider: "siliconflow".into(),
            api_base_url: "https://api.siliconflow.cn/v1".into(),
            api_key: String::new(),
            api_model: "Qwen/Qwen2.5-VL-72B-Instruct".into(),
            api_vision: true,
        }
    }
}

/// 定位默认引擎目录（优先可执行文件所在目录及上级目录，macOS .app 下回退 Application Support 目录）
pub fn default_engine_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(p) = find_llama_dir(&exe) {
            return p;
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(p) = find_llama_dir(&cwd) {
            return p;
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(data_dir) = dirs::data_local_dir() {
            return data_dir.join("com.minibot.app").join("llama");
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            return parent.join("llama");
        }
    }
    PathBuf::from("llama")
}

/// 从指定路径向上查找 llama 目录
fn find_llama_dir(start: &std::path::Path) -> Option<PathBuf> {
    let mut cur = Some(start);
    while let Some(dir) = cur {
        let cand = dir.join("llama");
        if cand.join("llama-server.exe").exists()
            || cand.join("llama-server").exists()
            || cand.join("models").exists()
        {
            return Some(cand);
        }
        cur = dir.parent();
    }
    None
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TipSettings {
    pub enabled: bool,
    pub hide_ms: u64,
    pub min_chars: usize,
    pub max_chars: usize,
    /// 进程名黑名单（小写，如 keepass.exe）
    pub blacklist: Vec<String>,
}

impl Default for TipSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            hide_ms: 3000,
            min_chars: 2,
            max_chars: 20000,
            blacklist: vec![
                "keepass.exe".into(),
                "keepassxc.exe".into(),
                "1password.exe".into(),
                "bitwarden.exe".into(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TtsSettings {
    /// 默认 Edge TTS 发音人（如 zh-CN-XiaoxiaoNeural）
    pub voice: String,
    /// 语速调节（如 "+0%", "+25%", "-15%"）
    pub rate: String,
    /// 音量调节（如 "+0%"）
    pub volume: String,
    /// 是否对纯外文（英/日/韩）智能匹配母语发音人
    pub auto_lang: bool,
}

impl Default for TtsSettings {
    fn default() -> Self {
        Self {
            voice: "zh-CN-XiaoxiaoNeural".into(),
            rate: "+0%".into(),
            volume: "+0%".into(),
            auto_lang: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiSettings {
    /// 是否开启对外 OpenAI 兼容网关
    pub enabled: bool,
    pub port: u16,
    /// 允许局域网访问（开启时强制要求 api_key）
    pub lan: bool,
    pub api_key: String,
    /// 显卡忙时外部请求策略：reject | force
    pub busy_policy: String,
    /// 调用方未指定时是否启用思考链
    pub thinking: bool,
}

impl Default for ApiSettings {
    fn default() -> Self {
        Self { enabled: true, port: 18181, lan: false, api_key: String::new(), busy_policy: "reject".into(), thinking: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSettings {
    /// 主题模式：system | light | dark
    pub theme: String,
    /// 自定义字体名称（空字符串表示使用默认字体栈）
    pub font_family: String,
    /// 基础字号（px，默认 14）
    pub font_size: u32,
    /// 正文行高（默认 1.68）
    pub line_height: f32,
    /// AI 聊天/结果窗默认宽度（默认 520）
    pub result_width: f64,
    /// AI 聊天/结果窗默认高度（默认 460）
    pub result_height: f64,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            font_family: String::new(),
            font_size: 14,
            line_height: 1.68,
            result_width: 520.0,
            result_height: 460.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KnowledgeSettings {
    /// 知识库根目录路径（空则默认使用 %APPDATA%/miniBot/Knowledge）
    pub base_path: String,
    /// 是否开启 Laya 极速决策引擎加速（若本地已存在模型）
    pub laya_enabled: bool,
    /// 聊天框是否默认开启知识库搜索（默认 false）
    pub auto_search: bool,
    /// 检索候选数量（默认 20）
    pub candidate_count: usize,
    /// 最终向模型提供的证据片段数量（默认 5）
    pub top_k: usize,
}

pub fn default_knowledge_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(appdata) = dirs::data_dir() {
            return appdata.join("miniBot").join("Knowledge");
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(appdata) = dirs::data_local_dir() {
            return appdata.join("com.minibot.app").join("Knowledge");
        }
    }
    PathBuf::from("Knowledge")
}

impl Default for KnowledgeSettings {
    fn default() -> Self {
        Self {
            base_path: default_knowledge_dir().to_string_lossy().into(),
            laya_enabled: false,
            auto_search: false,
            candidate_count: 20,
            top_k: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub autostart: bool,
    pub target_lang: String,
    pub history_days: u32,
    pub history_keep_images: bool,
    pub save_dir: String,
    pub ui: UiSettings,
    pub hotkeys: Hotkeys,
    pub llm: LlmSettings,
    pub tip: TipSettings,
    pub tts: TtsSettings,
    pub api: ApiSettings,
    pub knowledge: KnowledgeSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            autostart: false,
            target_lang: "zh-en".into(),
            history_days: 30,
            history_keep_images: true,
            save_dir: dirs::picture_dir()
                .map(|p| p.join("miniBot").to_string_lossy().into())
                .unwrap_or_default(),
            ui: UiSettings::default(),
            hotkeys: Hotkeys::default(),
            llm: LlmSettings::default(),
            tip: TipSettings::default(),
            tts: TtsSettings::default(),
            api: ApiSettings::default(),
            knowledge: KnowledgeSettings::default(),
        }
    }
}

pub struct SettingsStore {
    path: PathBuf,
    inner: RwLock<Settings>,
}

impl SettingsStore {
    pub fn load(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("settings.json");
        let mut inner: Settings = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let def_llm = LlmSettings::default();
        if inner.llm.server_path.trim().is_empty()
            || (!cfg!(windows) && inner.llm.server_path.to_lowercase().ends_with(".exe"))
        {
            inner.llm.server_path = def_llm.server_path;
        }
        // 默认模型目录固定在 llama/models 下不可修改
        inner.llm.models_dir = def_llm.models_dir;
        if inner.llm.mirror.trim().is_empty() {
            inner.llm.mirror = "modelscope".into();
        }
        if inner.llm.ctx == 20480 || inner.llm.ctx == 8192 || inner.llm.ctx == 0 {
            inner.llm.ctx = 32768;
        }
        if inner.llm.kv_quant.trim().is_empty() {
            inner.llm.kv_quant = "q8_0".into();
        }
        inner.llm.reasoning_effort = normalize_reasoning_effort(&inner.llm.reasoning_effort);
        if inner.llm.ngl < 0 {
            inner.llm.ngl = 99;
        }
        if inner.llm.api_provider.trim().is_empty() {
            inner.llm.api_provider = "siliconflow".into();
        }
        if inner.llm.api_base_url.trim().is_empty() {
            inner.llm.api_base_url = "https://api.siliconflow.cn/v1".into();
        }
        if inner.llm.api_model.trim().is_empty() {
            inner.llm.api_model = "Qwen/Qwen2.5-VL-72B-Instruct".into();
        }
        if inner.tts.voice.trim().is_empty() {
            inner.tts.voice = "zh-CN-XiaoxiaoNeural".into();
        }
        if inner.tts.rate.trim().is_empty() {
            inner.tts.rate = "+0%".into();
        }
        if inner.tts.volume.trim().is_empty() {
            inner.tts.volume = "+0%".into();
        }
        let store = Self { path, inner: RwLock::new(inner) };
        let _ = store.persist();
        store
    }

    pub fn get(&self) -> Settings {
        self.inner.read().unwrap().clone()
    }

    pub fn set(&self, s: Settings) -> anyhow::Result<()> {
        *self.inner.write().unwrap() = s;
        self.persist()
    }

    fn persist(&self) -> anyhow::Result<()> {
        let s = serde_json::to_string_pretty(&*self.inner.read().unwrap())?;
        std::fs::write(&self.path, s)?;
        Ok(())
    }
}
