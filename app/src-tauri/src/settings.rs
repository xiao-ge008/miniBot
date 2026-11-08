//! 设置：JSON 文件持久化，位于 %APPDATA%\com.minibot.app\settings.json
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub capture: String,
    pub launcher: String,
    pub selection: String,
    pub color: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            capture: "Alt+Q".into(),
            launcher: "Alt+Space".into(),
            selection: "Alt+W".into(),
            color: "Alt+C".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmSettings {
    /// 本地推理引擎可执行文件路径
    pub server_path: String,
    /// 模型统一存放目录（默认安装包/程序同级 llama/models 目录）
    pub models_dir: String,
    pub model_path: String,
    pub mmproj_path: String,
    /// 模型下载源：hf-mirror（国内镜像默认） | huggingface
    pub mirror: String,
    pub port: u16,
    pub ctx: u32,
    /// 闲置多少秒后释放模型
    pub idle_secs: u64,
    /// 预估所需显存（MB），低于该值触发让道检查
    pub required_vram_mb: u64,
    pub extra_args: String,
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
            mirror: "hf-mirror".into(),
            port: 18080,
            ctx: 32768,
            idle_secs: 180,
            required_vram_mb: 2400,
            extra_args: String::new(),
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
        if inner.llm.models_dir.trim().is_empty() {
            inner.llm.models_dir = def_llm.models_dir;
        }
        if inner.llm.mirror.trim().is_empty() {
            inner.llm.mirror = "hf-mirror".into();
        }
        if inner.llm.ctx == 20480 || inner.llm.ctx == 8192 || inner.llm.ctx == 0 {
            inner.llm.ctx = 32768;
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
