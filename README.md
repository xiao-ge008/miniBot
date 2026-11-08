# miniBot

**miniBot** 是一款基于 **Tauri v2 + Rust + Svelte 5** 构建的轻量级跨平台桌面 AI 效率工具（支持 Windows 与 macOS）。集成了多模态 AI 聊天、鼠标划词悬浮工具栏、智能区域截图与钉图、离线 OCR 文字识别、二维码识别与生成、屏幕取色器、Edge TTS 自然语音朗读，以及兼容 LocalSend 协议的局域网极速文件快传。

---

## ✨ 核心特性

- **🤖 本地 + 云端双模 AI 引擎**
  - **本地多模态推理**：支持按需拉起 `llama-server` 运行 `LFM3-VL-2B` 等 GGUF 视觉多模态模型，内置硬件自动检测（NVIDIA CUDA / AMD & Intel Vulkan / Apple Metal / CPU）、闲置自动释放及显存超限（≥15GB）智能让道机制。
  - **云端 OpenAI 兼容 API**：一键切换硅基流动（SiliconFlow）、DeepSeek、通义千问、智谱 GLM、豆包、Kimi、OpenAI、Ollama 等接口；针对纯文本模型上传图片时自动通过本地离线 OCR 提取图像文字后再交由模型分析。
- **🪄 全局划词与复制悬浮条（Tip）**
  - 鼠标拖拽划词或复制文本/图片时，智能弹出不抢焦点的轻量工具条，支持一键解释、翻译、总结、润色、代码检查与直接替换原文。
- **📸 智能截图、钉图与离线 OCR**
  - 支持多显示器区域截图、窗口自动吸附框选、标注马赛克、桌面置顶钉图（Pin）、二维码解析及系统级毫秒级离线 OCR。
- **🔊 自然语音朗读（Edge TTS）**
  - 内置多语种神经网络语音合成（中/英/日/韩等自动语种识别），支持 AI 回复流式朗读与划词即点即读。
- **📡 LocalSend 局域网文件/文本快传**
  - 原生实现 LocalSend v2 协议（UDP 多播发现 + HTTPS 自签名加密传输），支持与手机/电脑端 LocalSend 互传文件、文件夹、剪贴板文本与截图，并支持系统右键菜单一键发送。
- **🔌 本地 OpenAI 兼容 API 网关**
  - 内置轻量级 HTTP 网关（默认 `127.0.0.1:18089/v1`），方便外部脚本与应用共享调用。

---

## ⌨️ 默认全局快捷键

| 功能 | 默认快捷键 | 说明 |
| :--- | :--- | :--- |
| **呼出 AI 聊天框** | `Alt + Space` | 打开多轮多模态 AI 聊天主窗口（内置联网搜索与智能体） |
| **区域截图识别** | `Alt + Q` | 框选屏幕区域进行识图问答、OCR、二维码识别、钉图或保存 |
| **处理当前选中文字** | `Alt + W` | 抓取当前光标选区或剪贴板内容并弹出 AI 快捷动作条 |
| **屏幕取色器** | `Alt + C` | 放大镜像素级取色，支持 HEX / RGB / HSL 调色板转换 |

> 所有快捷键均可在「设置」面板中自由修改，支持 2~4 键组合及 F1~F12 功能键。

---

## 🛠️ 本地开发与构建

### 环境要求

- **Node.js** >= 22 & **pnpm** >= 10
- **Rust** >= 1.85（支持 Rust 2024 Edition）
- 平台系统依赖：
  - **Windows**：Microsoft Visual Studio C++ 生成工具、WebView2 Runtime
  - **macOS**：Xcode Command Line Tools

### 启动开发模式

```bash
cd app
pnpm install
pnpm tauri dev
```

### 打包生产版本

```bash
cd app
pnpm tauri build
```

- **Windows 安装包**：`app/src-tauri/target/release/bundle/nsis/miniBot_*_x64-setup.exe`
- **macOS 安装包**：`app/src-tauri/target/release/bundle/dmg/miniBot_*.dmg`

---

## 📦 GitHub Actions 自动打包

本项目已配置 `.github/workflows/build.yml`，每次推送代码或手动触发 Workflow 时，GitHub Actions 会自动在 `windows-latest` 与 `macos-latest` 环境中并行编译并生成 Windows (`.exe`) 和 macOS (`.dmg`) 安装包，产物可在 **Actions -> Artifacts** 或 **Releases** 页面直接下载。

---

## 📄 开源协议

本项目基于 [MIT License](./LICENSE) 开源。
