<script lang="ts">
  // 极简无边框设置面板：
  // 1. 本地模型页：本机配置检查（GPU/显存/内存）+ 本地引擎管理（N卡CUDA/A卡&Intel Vulkan/Mac Metal/CPU）+ 智能模型下载（Q4M/Q2S，支持 hf-mirror）+ 修改目录自动迁移模型 + 折叠高级设置（默认 8K 上下文）
  // 2. 窗口互斥：从聊天窗打开设置时聊天窗自动隐藏，关闭设置窗时自动恢复聊天窗
  import { onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { ipc, onLlmStatus } from '$lib/core/ipc';
  import SpeakButton from '$lib/components/SpeakButton.svelte';
  import type {
    AppInfo,
    CloudTestResult,
    DownloadProgress,
    EngineOverview,
    Settings,
    StatusInfo,
    VoiceOption,
  } from '$lib/core/types';

  const win = getCurrentWindow();

  const TABS = ['模型', '外观与常规', '全局热键', '划词与复制', 'OpenAI 网关', '历史清理'] as const;

  interface ApiPresetModel {
    id: string;
    label: string;
    vision: boolean;
  }

  interface ApiProviderPreset {
    id: string;
    name: string;
    baseUrl: string;
    defaultModel: string;
    defaultVision: boolean;
    hint: string;
    models: ApiPresetModel[];
  }

  const API_PROVIDERS: ApiProviderPreset[] = [
    {
      id: 'siliconflow',
      name: '硅基流动 (SiliconFlow)',
      baseUrl: 'https://api.siliconflow.cn/v1',
      defaultModel: 'Qwen/Qwen2.5-VL-72B-Instruct',
      defaultVision: true,
      hint: '国内高速直连，支持 Qwen-VL 视觉与 DeepSeek V3/R1 全系列模型',
      models: [
        { id: 'Qwen/Qwen2.5-VL-72B-Instruct', label: 'Qwen2.5-VL-72B (高清识图·推荐)', vision: true },
        { id: 'Pro/Qwen/Qwen2.5-VL-7B-Instruct', label: 'Qwen2.5-VL-7B (轻量极速识图)', vision: true },
        { id: 'Pro/deepseek-ai/DeepSeek-V3', label: 'DeepSeek-V3 (旗舰对话)', vision: false },
        { id: 'deepseek-ai/DeepSeek-R1', label: 'DeepSeek-R1 (深度思考)', vision: false },
        { id: 'Qwen/Qwen2.5-7B-Instruct', label: 'Qwen2.5-7B (免费极速文本)', vision: false },
      ],
    },
    {
      id: 'deepseek',
      name: 'DeepSeek 官方',
      baseUrl: 'https://api.deepseek.com/v1',
      defaultModel: 'deepseek-chat',
      defaultVision: false,
      hint: 'DeepSeek 官方开放平台（识图时将自动调用本地离线 OCR 提取图中文字）',
      models: [
        { id: 'deepseek-chat', label: 'deepseek-chat (DeepSeek-V3 极速对话)', vision: false },
        { id: 'deepseek-reasoner', label: 'deepseek-reasoner (DeepSeek-R1 深度推理)', vision: false },
      ],
    },
    {
      id: 'dashscope',
      name: '阿里云百炼 (通义千问)',
      baseUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1',
      defaultModel: 'qwen-vl-max',
      defaultVision: true,
      hint: '阿里云 DashScope OpenAI 兼容接口，支持通义千问视觉与文本模型',
      models: [
        { id: 'qwen-vl-max', label: 'qwen-vl-max (旗舰视觉多模态)', vision: true },
        { id: 'qwen-vl-plus', label: 'qwen-vl-plus (高性价比识图)', vision: true },
        { id: 'qwen-plus', label: 'qwen-plus (均衡通用对话)', vision: false },
        { id: 'qwen-max', label: 'qwen-max (千问旗舰文本)', vision: false },
        { id: 'qwen-turbo', label: 'qwen-turbo (极速轻量)', vision: false },
      ],
    },
    {
      id: 'zhipu',
      name: '智谱 AI (BigModel)',
      baseUrl: 'https://open.bigmodel.cn/api/paas/v4',
      defaultModel: 'glm-4v-flash',
      defaultVision: true,
      hint: '智谱开放平台，提供免费版 GLM-4V-Flash 视觉模型与 GLM-4-Flash 文本模型',
      models: [
        { id: 'glm-4v-flash', label: 'glm-4v-flash (免费视觉识图)', vision: true },
        { id: 'glm-4-flash', label: 'glm-4-flash (免费极速对话)', vision: false },
        { id: 'glm-4v-plus', label: 'glm-4v-plus (增强视觉多模态)', vision: true },
        { id: 'glm-4-plus', label: 'glm-4-plus (旗舰通用对话)', vision: false },
      ],
    },
    {
      id: 'doubao',
      name: '火山引擎 (豆包 Doubao)',
      baseUrl: 'https://ark.cn-beijing.volces.com/api/v3',
      defaultModel: 'doubao-1-5-vision-pro-32k-250115',
      defaultVision: true,
      hint: '字节跳动火山方舟平台，模型名称也可填写推理接入点 ID (ep-xxxx)',
      models: [
        { id: 'doubao-1-5-vision-pro-32k-250115', label: 'Doubao-1.5-Vision-Pro (视觉识图)', vision: true },
        { id: 'doubao-1-5-pro-32k-250115', label: 'Doubao-1.5-Pro (通用旗舰)', vision: false },
        { id: 'deepseek-v3-250324', label: '火山 DeepSeek-V3', vision: false },
      ],
    },
    {
      id: 'moonshot',
      name: '月之暗面 (Kimi)',
      baseUrl: 'https://api.moonshot.cn/v1',
      defaultModel: 'moonshot-v1-8k-vision-preview',
      defaultVision: true,
      hint: 'Kimi 开放平台 OpenAI 兼容接口',
      models: [
        { id: 'moonshot-v1-8k-vision-preview', label: 'moonshot-v1-8k-vision (视觉识图)', vision: true },
        { id: 'moonshot-v1-8k', label: 'moonshot-v1-8k (标准对话)', vision: false },
        { id: 'moonshot-v1-32k', label: 'moonshot-v1-32k (长文本)', vision: false },
      ],
    },
    {
      id: 'openai',
      name: 'OpenAI / 兼容中转站',
      baseUrl: 'https://api.openai.com/v1',
      defaultModel: 'gpt-4o-mini',
      defaultVision: true,
      hint: '支持 OpenAI 官方及各类 OneAPI / NewAPI 第三方兼容中转地址',
      models: [
        { id: 'gpt-4o-mini', label: 'gpt-4o-mini (极速视觉多模态)', vision: true },
        { id: 'gpt-4o', label: 'gpt-4o (旗舰视觉多模态)', vision: true },
        { id: 'claude-3-5-sonnet-latest', label: 'claude-3-5-sonnet (中转多模态)', vision: true },
        { id: 'gemini-2.0-flash', label: 'gemini-2.0-flash (中转多模态)', vision: true },
      ],
    },
    {
      id: 'ollama',
      name: 'Ollama / LM Studio (外部服务)',
      baseUrl: 'http://127.0.0.1:11434/v1',
      defaultModel: 'qwen2.5vl:7b',
      defaultVision: true,
      hint: '连接本机或局域网其他机器上运行的 Ollama / LM Studio / vLLM 服务（无需 API Key）',
      models: [
        { id: 'qwen2.5vl:7b', label: 'qwen2.5vl:7b (视觉)', vision: true },
        { id: 'qwen2.5:7b', label: 'qwen2.5:7b (文本)', vision: false },
        { id: 'llama3.2-vision', label: 'llama3.2-vision (视觉)', vision: true },
      ],
    },
    {
      id: 'custom',
      name: '自定义 OpenAI 格式接口',
      baseUrl: '',
      defaultModel: '',
      defaultVision: true,
      hint: '任意兼容 OpenAI /v1/chat/completions 规范的云端或私有化部署 API',
      models: [],
    },
  ];

  const FONT_PRESETS: { label: string; value: string }[] = [
    { label: '系统默认 (Inter / 微软雅黑)', value: '' },
    { label: '微软雅黑 (Microsoft YaHei UI)', value: "'Microsoft YaHei UI', 'Microsoft YaHei'" },
    { label: '苹方 / 思源黑体 (PingFang / Noto Sans)', value: "'PingFang SC', 'Noto Sans SC', 'Source Han Sans SC'" },
    { label: '霞鹜文楷 (LXGW WenKai)', value: "'LXGW WenKai', 'LXGW WenKai Screen'" },
    { label: '鸿蒙黑体 (HarmonyOS Sans SC)', value: "'HarmonyOS Sans SC'" },
    { label: '思源宋体 / 衬线 (Source Han Serif)', value: "'Source Han Serif SC', 'Noto Serif SC', 'Songti SC', SimSun" },
    { label: '现代西文无衬线 (Inter / Segoe UI)', value: "'Inter', 'Segoe UI Variable Display', 'Segoe UI'" },
    { label: '编程等宽 (Cascadia Code / JetBrains Mono)', value: "'Cascadia Code', 'JetBrains Mono', Consolas, monospace" },
  ];

  let tab = $state<(typeof TABS)[number]>('模型');
  let modelSubTab = $state<'local' | 'api'>('local');
  let s = $state<Settings | null>(null);
  let info = $state<AppInfo | null>(null);
  let status = $state<StatusInfo | null>(null);
  let overview = $state<EngineOverview | null>(null);
  let dlProgress = $state<DownloadProgress | null>(null);
  let dlError = $state('');
  let downloading = $state(false);
  let startingEngine = $state(false);
  let engineBackendChoice = $state('auto');
  let advOpen = $state(false);

  // 云端 API 交互状态
  let showApiKey = $state(false);
  let testingApi = $state(false);
  let apiTestResult = $state<CloudTestResult | null>(null);
  let apiTestError = $state('');
  let fetchingModels = $state(false);
  let fetchedModels = $state<string[]>([]);
  let customModelInputMode = $state(false);
  let autoFetchTimer: ReturnType<typeof setTimeout> | undefined;
  let ttsVoices = $state<VoiceOption[]>([]);

  const currentProviderPreset = $derived(
    API_PROVIDERS.find((p) => p.id === (s?.llm.api_provider || 'siliconflow')) ?? API_PROVIDERS[0],
  );

  const availableApiModels = $derived.by(() => {
    const out: { id: string; label: string }[] = [];
    const seen = new Set<string>();
    const cur = s?.llm.api_model?.trim() || '';
    if (fetchedModels.length > 0) {
      if (cur && !fetchedModels.includes(cur)) {
        seen.add(cur);
        out.push({ id: cur, label: cur });
      }
      for (const m of fetchedModels) {
        if (!seen.has(m)) {
          seen.add(m);
          out.push({ id: m, label: m });
        }
      }
    } else {
      for (const pm of currentProviderPreset.models) {
        seen.add(pm.id);
        out.push({ id: pm.id, label: pm.label });
      }
      if (cur && !seen.has(cur)) {
        seen.add(cur);
        out.push({ id: cur, label: cur });
      }
    }
    return out;
  });

  let msg = $state('');
  let errors = $state<string[]>([]);
  let blacklistText = $state('');
  let saveTimer: ReturnType<typeof setTimeout> | undefined;

  type HotkeyField = 'launcher' | 'capture' | 'selection' | 'color';
  const HOTKEY_ITEMS: { key: HotkeyField; label: string; hint: string }[] = [
    { key: 'launcher', label: '呼出 AI 智能聊天框', hint: '示例：Alt+Space、Ctrl+Alt+Space' },
    { key: 'capture', label: '区域截图识别', hint: '示例：Alt+A、Ctrl+Alt+A、Ctrl+Shift+A' },
    { key: 'selection', label: '处理当前选中文字', hint: '示例：Alt+W、Ctrl+Alt+W' },
    { key: 'color', label: '屏幕取色器', hint: '示例：Alt+C、Ctrl+Alt+C' },
  ];
  let recordingField = $state<HotkeyField | null>(null);
  let recordingPreview = $state('');
  const heldMods = new Set<'Ctrl' | 'Alt' | 'Shift' | 'Super'>();
  let modResetTimer: ReturnType<typeof setTimeout> | undefined;

  function startRecording(field: HotkeyField) {
    recordingField = field;
    heldMods.clear();
    recordingPreview = '请直接按键（支持 2~4 键，如 Ctrl+Alt+A）…';
    ipc.hotkeysSuspend(true);
  }

  function stopRecording() {
    if (!recordingField) return;
    recordingField = null;
    heldMods.clear();
    recordingPreview = '';
    clearTimeout(modResetTimer);
    ipc.hotkeysSuspend(false);
  }

  function clearHotkey(field: HotkeyField) {
    if (!s) return;
    s.hotkeys[field] = '';
    stopRecording();
    save();
  }

  function extractMainKey(e: KeyboardEvent): string | null {
    const code = e.code || '';
    if (
      code === 'ControlLeft' ||
      code === 'ControlRight' ||
      code === 'AltLeft' ||
      code === 'AltRight' ||
      code === 'ShiftLeft' ||
      code === 'ShiftRight' ||
      code === 'MetaLeft' ||
      code === 'MetaRight' ||
      e.key === 'Control' ||
      e.key === 'Alt' ||
      e.key === 'Shift' ||
      e.key === 'Meta'
    ) {
      return null;
    }
    if (code.startsWith('Key') && code.length === 4) return code.slice(3).toUpperCase();
    if (code.startsWith('Digit') && code.length === 6) return code.slice(5);
    if (code.startsWith('Numpad') && code.length === 7 && /\d/.test(code[6])) return code.slice(6);
    if (/^F([1-9]|1[0-2])$/i.test(code)) return code.toUpperCase();
    const map: Record<string, string> = {
      Space: 'Space',
      Enter: 'Enter',
      NumpadEnter: 'Enter',
      Tab: 'Tab',
      Backquote: '`',
      Minus: '-',
      Equal: '=',
      BracketLeft: '[',
      BracketRight: ']',
      Backslash: '\\',
      Semicolon: ';',
      Quote: "'",
      Comma: ',',
      Period: '.',
      Slash: '/',
      ArrowUp: 'Up',
      ArrowDown: 'Down',
      ArrowLeft: 'Left',
      ArrowRight: 'Right',
      Home: 'Home',
      End: 'End',
      PageUp: 'PageUp',
      PageDown: 'PageDown',
      Insert: 'Insert',
      PrintScreen: 'PrintScreen',
    };
    if (map[code]) return map[code];
    if (e.key && e.key.length === 1 && /[a-zA-Z0-9]/.test(e.key)) {
      return e.key.toUpperCase();
    }
    return null;
  }

  function orderedMods(set: Set<'Ctrl' | 'Alt' | 'Shift' | 'Super'>): string[] {
    const order: ('Ctrl' | 'Alt' | 'Shift' | 'Super')[] = ['Ctrl', 'Alt', 'Shift', 'Super'];
    return order.filter((m) => set.has(m));
  }

  function handleHotkeyKeydown(field: HotkeyField, e: KeyboardEvent) {
    if (!s) return;
    e.preventDefault();
    e.stopPropagation();

    if (e.key === 'Escape') {
      stopRecording();
      (e.currentTarget as HTMLElement)?.blur();
      return;
    }

    if (
      (e.key === 'Backspace' || e.key === 'Delete') &&
      !e.ctrlKey &&
      !e.altKey &&
      !e.shiftKey &&
      !e.metaKey &&
      heldMods.size === 0
    ) {
      clearHotkey(field);
      (e.currentTarget as HTMLElement)?.blur();
      return;
    }

    if (e.ctrlKey || e.key === 'Control') heldMods.add('Ctrl');
    if (e.altKey || e.key === 'Alt') heldMods.add('Alt');
    if (e.shiftKey || e.key === 'Shift') heldMods.add('Shift');
    if (e.metaKey || e.key === 'Meta') heldMods.add('Super');

    clearTimeout(modResetTimer);
    modResetTimer = setTimeout(() => {
      heldMods.clear();
      if (recordingField === field) {
        recordingPreview = '请直接按键（支持 2~4 键，如 Ctrl+Alt+A）…';
      }
    }, 1800);

    const mods = orderedMods(heldMods);
    const mainKey = extractMainKey(e);

    if (!mainKey) {
      recordingPreview = mods.length
        ? `${mods.join(' + ')} + 请继续按下一键…`
        : '请直接按键（支持 2~4 键，如 Ctrl+Alt+A）…';
      return;
    }

    const isFnKey = /^F([1-9]|1[0-2])$/.test(mainKey) || mainKey === 'PrintScreen';
    if (mods.length === 0 && !isFnKey) {
      recordingPreview = `请配合 Ctrl / Alt / Shift 一起按（如 Ctrl+Alt+${mainKey}）`;
      return;
    }

    const combo = [...mods, mainKey].join('+');
    s.hotkeys[field] = combo;
    stopRecording();
    (e.currentTarget as HTMLElement)?.blur();
    save();
  }

  const DEFAULT_FONT = "'Inter', 'Segoe UI Variable', 'Segoe UI', 'PingFang SC', 'Microsoft YaHei UI', system-ui, sans-serif";

  function flashMsg(text: string, ms = 2500) {
    msg = text;
    setTimeout(() => {
      if (msg === text) msg = '';
    }, ms);
  }

  function previewUi() {
    if (!s?.ui || typeof document === 'undefined') return;
    const root = document.documentElement;
    root.dataset.theme = s.ui.theme || 'system';
    const fam = s.ui.font_family?.trim();
    root.style.setProperty('--app-font', fam ? `${fam}, ${DEFAULT_FONT}` : DEFAULT_FONT);
    root.style.setProperty('--app-font-size', `${Math.max(12, Math.min(22, s.ui.font_size || 14))}px`);
    root.style.setProperty('--app-line-height', String(Math.max(1.3, Math.min(2.4, s.ui.line_height || 1.68))));
  }

  function scheduleAutoSave() {
    previewUi();
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      save();
    }, 180);
  }

  async function load() {
    const loaded = await ipc.getSettings();
    loaded.ui = {
      theme: loaded.ui?.theme || 'system',
      font_family: loaded.ui?.font_family ?? '',
      font_size: loaded.ui?.font_size || 14,
      line_height: loaded.ui?.line_height || 1.68,
      result_width: loaded.ui?.result_width || 520,
      result_height: loaded.ui?.result_height || 460,
    };
    if (!loaded.llm.ctx || loaded.llm.ctx === 20480) loaded.llm.ctx = 8192;
    if (!loaded.llm.mirror) loaded.llm.mirror = 'hf-mirror';
    if (!loaded.llm.api_provider) loaded.llm.api_provider = 'siliconflow';
    if (!loaded.llm.api_base_url) loaded.llm.api_base_url = 'https://api.siliconflow.cn/v1';
    if (!loaded.llm.api_model) loaded.llm.api_model = 'Qwen/Qwen2.5-VL-72B-Instruct';
    if (typeof loaded.llm.api_vision !== 'boolean') loaded.llm.api_vision = true;
    loaded.tts = {
      voice: loaded.tts?.voice || 'zh-CN-XiaoxiaoNeural',
      rate: loaded.tts?.rate || '+0%',
      volume: loaded.tts?.volume || '+0%',
      auto_lang: typeof loaded.tts?.auto_lang === 'boolean' ? loaded.tts.auto_lang : true,
    };
    s = loaded;
    info = await ipc.appInfo();
    ttsVoices = await ipc.ttsVoices().catch(() => []);
    blacklistText = s.tip.blacklist.join('\n');
    errors = info.hotkey_errors;
    previewUi();
    overview = await ipc.engineOverview();
    triggerAutoFetchModels(120);
  }

  async function save() {
    if (!s) return;
    s.tip.blacklist = blacklistText
      .split(/\r?\n/)
      .map((x) => x.trim().toLowerCase())
      .filter(Boolean);
    s.ui.result_width = Math.max(360, Math.min(1400, Number(s.ui.result_width) || 520));
    s.ui.result_height = Math.max(240, Math.min(1200, Number(s.ui.result_height) || 460));
    s.ui.font_size = Math.max(12, Math.min(22, Number(s.ui.font_size) || 14));
    s.ui.line_height = Math.max(1.3, Math.min(2.4, Number(s.ui.line_height) || 1.68));
    previewUi();
    try {
      errors = await ipc.saveSettings($state.snapshot(s) as Settings);
      info = await ipc.appInfo();
      flashMsg(errors.length ? '已保存（部分热键冲突）' : '✓ 已同步保存');
    } catch (e) {
      flashMsg(`保存失败：${e}`);
    }
  }

  function setSizePreset(w: number, h: number) {
    if (!s) return;
    s.ui.result_width = w;
    s.ui.result_height = h;
    save();
  }

  // ---------- 本地引擎与模型操作 ----------

  async function toggleEngineRun() {
    if (status?.status === 'ready' || status?.status === 'loading') {
      await ipc.llmRelease();
      flashMsg('✓ 已关闭本地引擎并释放显存');
    } else {
      startingEngine = true;
      try {
        await ipc.llmStart();
        flashMsg('✓ 本地引擎已启动就绪');
      } catch (e) {
        flashMsg(`启动失败：${e}`, 4000);
      } finally {
        startingEngine = false;
      }
    }
  }

  async function handleDownloadEngine() {
    if (downloading) return;
    downloading = true;
    dlError = '';
    dlProgress = {
      target: 'engine',
      stage: 'downloading',
      file: '准备下载引擎…',
      downloaded_mb: 0,
      total_mb: 0,
      percent: 1,
      message: '正在连接引擎仓库…',
    };
    try {
      overview = await ipc.engineDownloadRuntime(engineBackendChoice, (p) => {
        dlProgress = p;
      });
      s = await ipc.getSettings();
      flashMsg('✓ 本地推理引擎已安装/更新完成');
    } catch (e) {
      const errStr = String(e);
      if (!errStr.includes('已取消')) {
        dlError = `引擎下载未完成：${errStr}`;
      }
      flashMsg(`引擎下载未完成：${errStr}`, 4000);
    } finally {
      downloading = false;
      dlProgress = null;
    }
  }

  async function handleDownloadModel(modelId: string) {
    if (downloading || !s) return;
    downloading = true;
    dlError = '';
    dlProgress = {
      target: modelId,
      stage: 'downloading',
      file: modelId.toUpperCase(),
      downloaded_mb: 0,
      total_mb: 0,
      percent: 1,
      message: '正在连接模型镜像源…',
    };
    try {
      overview = await ipc.engineDownloadModel(modelId, s.llm.mirror || 'hf-mirror', (p) => {
        dlProgress = p;
      });
      s = await ipc.getSettings();
      flashMsg('✓ 模型下载完成并已自动启用');
    } catch (e) {
      const errStr = String(e);
      if (!errStr.includes('已取消')) {
        dlError = `模型下载未完成：${errStr}`;
      }
      flashMsg(`模型下载未完成：${errStr}`, 4000);
    } finally {
      downloading = false;
      dlProgress = null;
    }
  }

  async function handleActivateModel(modelId: string) {
    try {
      overview = await ipc.engineActivateModel(modelId);
      s = await ipc.getSettings();
      flashMsg('✓ 已切换当前使用的模型');
    } catch (e) {
      flashMsg(`切换失败：${e}`);
    }
  }

  async function handlePickModelsDir() {
    if (downloading) return;
    try {
      const res = await ipc.enginePickModelsDir();
      if (!res) return;
      const [newOv, movedCount] = res;
      overview = newOv;
      s = await ipc.getSettings();
      const installedCount = newOv.models.filter((m) => m.installed).length;
      if (movedCount > 0) {
        flashMsg(`✓ 已更换位置并自动迁移 ${movedCount} 个模型文件`, 3500);
      } else if (installedCount > 0) {
        flashMsg(`✓ 已切换存储位置，成功识别 ${installedCount} 个本地模型`, 3500);
      } else {
        flashMsg('✓ 模型存储位置已更新', 3500);
      }
    } catch (e) {
      flashMsg(`修改目录失败：${e}`, 3500);
    }
  }

  // ---------- 云端 OpenAI 兼容模型 API 操作 ----------

  function selectApiProvider(providerId: string) {
    if (!s) return;
    s.llm.api_provider = providerId;
    apiTestResult = null;
    apiTestError = '';
    fetchedModels = [];
    customModelInputMode = false;
    const preset = API_PROVIDERS.find((p) => p.id === providerId);
    if (preset && providerId !== 'custom') {
      s.llm.api_base_url = preset.baseUrl;
      s.llm.api_model = preset.defaultModel;
      s.llm.api_vision = preset.defaultVision;
    }
    save();
    triggerAutoFetchModels(100);
  }

  function inferVisionFromModelId(modelId: string): boolean {
    const low = modelId.toLowerCase();
    if (
      low.includes('vl') ||
      low.includes('vision') ||
      low.includes('4o') ||
      low.includes('4v') ||
      low.includes('claude-3') ||
      low.includes('gemini') ||
      low.includes('pixtral') ||
      low.includes('qvq')
    ) {
      return true;
    }
    if (low.includes('deepseek-chat') || low.includes('deepseek-reasoner') || low.includes('qwq')) {
      return false;
    }
    return s?.llm.api_vision ?? true;
  }

  async function handleTestCloudApi() {
    if (!s || testingApi) return;
    testingApi = true;
    apiTestResult = null;
    apiTestError = '';
    try {
      await save();
      const res = await ipc.llmTestApi(s.llm.api_base_url, s.llm.api_key, s.llm.api_model);
      apiTestResult = res;
      flashMsg(`✓ 连接成功（${res.latency_ms} ms）`);
    } catch (e) {
      apiTestError = String(e);
    } finally {
      testingApi = false;
    }
  }

  async function handleFetchCloudModels(silent = false) {
    if (!s || fetchingModels) return;
    const baseUrl = s.llm.api_base_url.trim();
    const apiKey = s.llm.api_key.trim();
    const isLocalEndpoint =
      s.llm.api_provider === 'ollama' ||
      baseUrl.includes('127.0.0.1') ||
      baseUrl.includes('localhost');
    if (!baseUrl || (silent && !apiKey && !isLocalEndpoint)) {
      return;
    }
    fetchingModels = true;
    if (!silent) apiTestError = '';
    try {
      const list = await ipc.llmFetchModels(baseUrl, apiKey);
      fetchedModels = list;
      if (list.length > 0 && (!s.llm.api_model.trim() || !list.includes(s.llm.api_model))) {
        const matched = list.find((m) => m === s?.llm.api_model) || list[0];
        s.llm.api_model = matched;
        s.llm.api_vision = inferVisionFromModelId(matched);
        save();
      }
      if (!silent) {
        flashMsg(`✓ 已获取到 ${list.length} 个模型`);
      }
    } catch (e) {
      if (!silent) {
        apiTestError = String(e);
      }
    } finally {
      fetchingModels = false;
    }
  }

  function triggerAutoFetchModels(delayMs = 350) {
    clearTimeout(autoFetchTimer);
    autoFetchTimer = setTimeout(() => {
      handleFetchCloudModels(true);
    }, delayMs);
  }

  onMount(() => {
    load();
    ipc.llmStatus().then((x) => (status = x));
    const un = onLlmStatus((x) => (status = x));
    return () => un.then((f) => f());
  });
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && !recordingField && ipc.closePage()} />

<div class="page">
  <!-- 左侧极简导航栏 -->
  <aside class="sidebar">
    <div class="brand" data-tauri-drag-region>
      <img src="/logo.png" alt="miniBot" class="brand-logo" />
      <span class="brand-name" data-tauri-drag-region>miniBot</span>
      <small data-tauri-drag-region>{info?.version ?? ''}</small>
    </div>

    <div class="nav-list">
      {#each TABS as t (t)}
        <button class="nav-btn" class:active={tab === t} onclick={() => (tab = t)}>{t}</button>
      {/each}
    </div>
  </aside>

  <!-- 右侧主设置区 -->
  <div class="content-col">
    <!-- 顶部无边框拖拽栏 + 右上角独立关闭按钮（关闭后自动回到 AI 聊天框） -->
    <header class="top-bar" data-tauri-drag-region>
      <h1 class="tab-title" data-tauri-drag-region>{tab}</h1>
      <div class="top-right no-drag" role="toolbar" tabindex="-1" onmousedown={(e) => e.stopPropagation()}>
        {#if msg}<span class="save-msg">{msg}</span>{/if}
        <button class="primary save-btn" onclick={save}>保存设置</button>
        <div class="win-sep"></div>
        <button class="win-ctrl" onclick={() => ipc.minimizeSelf()} title="最小化">─</button>
        <button class="win-ctrl close" onclick={() => ipc.closePage()} title="关闭设置并返回聊天 (Esc)">✕</button>
      </div>
    </header>

    {#if s}
      <main class="main-scroll">
        {#if info?.gateway_error}<div class="alert">{info.gateway_error}</div>{/if}
        {#each errors as e (e)}<div class="alert">{e}</div>{/each}

        {#if tab === '模型'}
          <!-- 顶部二级 Tab 切换：默认「本地模型」，第二个「模型 API」 -->
          <div class="subtab-bar">
            <div class="subtab-group">
              <button
                type="button"
                class="subtab-btn"
                class:active={modelSubTab === 'local'}
                onclick={() => (modelSubTab = 'local')}
              >
                <span>⚡ 本地模型</span>
                {#if !s.llm.api_enabled}
                  <span class="subtab-pill ok">当前生效</span>
                {/if}
              </button>
              <button
                type="button"
                class="subtab-btn"
                class:active={modelSubTab === 'api'}
                onclick={() => {
                  modelSubTab = 'api';
                  triggerAutoFetchModels(60);
                }}
              >
                <span>☁️ 模型 API</span>
                {#if s.llm.api_enabled}
                  <span class="subtab-pill ok">当前生效</span>
                {:else}
                  <span class="subtab-pill">支持低配/无显卡机器</span>
                {/if}
              </button>
            </div>

            <div class="subtab-hint">
              {#if s.llm.api_enabled}
                <span class="mode-indicator cloud">☁️ 已启用云端 API：{s.llm.api_model || '未指定模型'}</span>
              {:else}
                <span class="mode-indicator local">🖥 当前使用：本地离线引擎</span>
              {/if}
            </div>
          </div>

          {#if modelSubTab === 'local'}
            <!-- 若本机无独立显卡且 CPU/内存较低，提醒可一键切换至「模型 API」使用云端模型 -->
            {#if overview?.hardware.low_end_device && !s.llm.api_enabled}
              <div class="hw-recommend-banner">
                <div class="hwr-main">
                  <div class="hwr-title">💡 检测到本机无独立显卡或处理器配置偏低</div>
                  <div class="hwr-desc">{overview.hardware.low_end_reason}</div>
                </div>
                <button
                  type="button"
                  class="act-btn light"
                  onclick={() => (modelSubTab = 'api')}
                >
                  去开启模型 API →
                </button>
              </div>
            {:else if s.llm.api_enabled}
              <div class="cloud-active-banner">
                <div class="hwr-main">
                  <div class="hwr-title">☁️ 当前正在使用「模型 API」云端模型（{s.llm.api_model}）</div>
                  <div class="hwr-desc">
                    本地推理引擎已自动待命，不占用任何本机显存与 CPU。如需改回本地离线模型，可点击右侧按钮切换。
                  </div>
                </div>
                <div class="engine-dl-group">
                  <button
                    type="button"
                    class="act-btn"
                    onclick={() => {
                      s!.llm.api_enabled = false;
                      save();
                    }}
                  >
                    切回本地模型
                  </button>
                  <button
                    type="button"
                    class="act-btn light"
                    onclick={() => (modelSubTab = 'api')}
                  >
                    配置模型 API →
                  </button>
                </div>
              </div>
            {/if}

            <!-- 1. 本地运行时 / 推理引擎 -->
            <section class="rt-section">
              <div class="rt-head">
                <div class="rt-head-left">
                  <span class="rt-icon">⚡</span>
                  <span class="rt-title">本地运行时</span>
                  <span class="ver-pill">{overview?.engine_version ?? '检测中'}</span>
                  {#if overview?.engine_installed}
                    <span class="ver-pill backend">{overview.engine_backend} 加速</span>
                  {/if}
                </div>
                <span
                  class="run-badge"
                  class:running={status?.status === 'ready' || status?.status === 'loading'}
                >
                  {#if status?.status === 'ready'}
                    运行中 · {Math.floor(status.remaining_secs / 60)}:{String(status.remaining_secs % 60).padStart(2, '0')}
                  {:else if status?.status === 'loading' || startingEngine}
                    唤醒中…
                  {:else if overview?.engine_installed}
                    待命（按需唤醒）
                  {:else}
                    未安装
                  {/if}
                </span>
              </div>

              <div class="rt-block">
                <div class="rt-row">
                  <div class="rt-info">
                    <div class="rt-name">
                      {overview?.engine_installed ? '已安装本地推理引擎运行时' : '尚未安装本地推理引擎'}
                    </div>
                    <div class="rt-desc">
                      {#if status?.status === 'ready'}
                        本地推理服务正在运行。关闭后将立即释放全部显存，新对话会自动重新唤醒引擎。
                      {:else}
                        本地推理服务处于空闲省电状态（0 显存占用），发起对话或识图时自动秒级加载。
                      {/if}
                    </div>
                  </div>
                  {#if overview?.engine_installed}
                    <button
                      class="act-btn"
                      onclick={toggleEngineRun}
                      disabled={startingEngine || downloading}
                    >
                      {#if status?.status === 'ready' || status?.status === 'loading'}
                        ■ 关闭
                      {:else if startingEngine}
                        启动中…
                      {:else}
                        ▶ 启动
                      {/if}
                    </button>
                  {/if}
                </div>

                <div class="rt-divider"></div>

                <div class="rt-row">
                  <div class="rt-info">
                    <div class="rt-name">引擎硬件加速构建</div>
                    <div class="rt-desc">
                      已为本机自动匹配
                      <b>{overview?.hardware.recommended_backend_label ?? '最佳硬件加速'}</b>
                      构建（全面兼容 NVIDIA N卡、AMD A卡、Intel 显卡及 Mac）。
                    </div>
                  </div>
                  <div class="engine-dl-group">
                    <select class="backend-sel" bind:value={engineBackendChoice} disabled={downloading}>
                      <option value="auto">自动匹配 ({overview?.hardware.recommended_backend.toUpperCase() ?? 'AUTO'})</option>
                      <option value="cuda">NVIDIA 显卡 (CUDA)</option>
                      <option value="vulkan">AMD / Intel 显卡 (Vulkan)</option>
                      <option value="cpu">通用处理器 (CPU AVX2)</option>
                      <option value="metal">macOS (Apple Metal)</option>
                    </select>
                    <button
                      class="act-btn light"
                      onclick={handleDownloadEngine}
                      disabled={downloading}
                    >
                      ⬇ {overview?.engine_installed ? '更新引擎' : '安装引擎'}
                    </button>
                  </div>
                </div>
              </div>
            </section>

            <!-- 2. 本机配置检查 -->
            <section class="rt-section">
              <div class="rt-head">
                <div class="rt-head-left">
                  <span class="rt-icon">🖥</span>
                  <span class="rt-title">本机配置</span>
                </div>
              </div>

              <div class="hw-bar">
                <div class="hw-item">
                  <span class="hw-ic">⚡</span>
                  <span class="hw-val">{overview?.hardware.gpu_name || '正在检测处理器与显卡…'}</span>
                </div>
                <div class="hw-item">
                  <span class="hw-ic">❖</span>
                  <span class="hw-val">
                    {overview?.hardware.vram_gb ? `${overview.hardware.vram_gb.toFixed(1)} GB 显存` : '共享系统内存'}
                  </span>
                </div>
                {#if overview?.hardware.cpu_name}
                  <div class="hw-item">
                    <span class="hw-ic">⚙</span>
                    <span class="hw-val">{overview.hardware.cpu_name} ({overview.hardware.cpu_cores}线程)</span>
                  </div>
                {/if}
                <div class="hw-item">
                  <span class="hw-ic">⬢</span>
                  <span class="hw-val">{overview?.hardware.ram_gb.toFixed(1) ?? '16.0'} GB 内存</span>
                </div>
              </div>
            </section>

            <!-- 实时下载进度横幅与错误提示 -->
            {#if downloading && dlProgress}
              <div class="dl-banner">
                <div class="dl-top">
                  <span class="dl-msg">{dlProgress.message}</span>
                  <span class="dl-pct">{dlProgress.percent}%</span>
                  <button class="ghost sm cancel-dl" onclick={() => ipc.engineCancelDownload()}>取消</button>
                </div>
                <div class="dl-track">
                  <div class="dl-fill" style="width: {dlProgress.percent}%"></div>
                </div>
              </div>
            {:else if dlError}
              <div class="api-test-banner err" style="display:flex;align-items:center;justify-content:space-between;gap:12px;">
                <span>✕ {dlError}</span>
                <button type="button" class="ghost sm" onclick={() => (dlError = '')}>关闭</button>
              </div>
            {/if}

            <!-- 3. 精选视觉模型下载与管理 -->
            <section class="rt-section">
              <div class="rt-head">
                <div class="rt-head-left">
                  <span class="rt-icon">⬇</span>
                  <span class="rt-title">本地模型库</span>
                  <span class="ver-pill">{overview?.models.length ?? 2}</span>
                </div>

                <div class="model-head-tools">
                  <!-- 国内镜像 / ModelScope 备用 / 官方源切换 -->
                  <div class="mirror-switch" title="模型下载源（若当前源较慢将自动切换至 ModelScope 魔搭社区备用源）">
                    <button
                      type="button"
                      class="m-btn"
                      class:active={s.llm.mirror !== 'huggingface' && s.llm.mirror !== 'modelscope'}
                      onclick={() => {
                        s!.llm.mirror = 'hf-mirror';
                        save();
                      }}>国内镜像 (hf-mirror)</button
                    >
                    <button
                      type="button"
                      class="m-btn"
                      class:active={s.llm.mirror === 'modelscope'}
                      onclick={() => {
                        s!.llm.mirror = 'modelscope';
                        save();
                      }}>ModelScope (魔搭)</button
                    >
                    <button
                      type="button"
                      class="m-btn"
                      class:active={s.llm.mirror === 'huggingface'}
                      onclick={() => {
                        s!.llm.mirror = 'huggingface';
                        save();
                      }}>HuggingFace</button
                    >
                  </div>

                  <!-- 小图标修改模型存放目录（修改后自动迁移已有模型） -->
                  <button
                    type="button"
                    class="dir-icon-btn"
                    onclick={handlePickModelsDir}
                    title={`当前模型目录：${overview?.models_dir ?? '默认安装包目录'}\n点击修改存储位置（已有模型将自动转移至新目录）`}
                  >
                    <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
                    </svg>
                    <span>存储位置</span>
                  </button>
                </div>
              </div>

              <div class="model-list">
                {#if overview}
                  {#each overview.models as m (m.id)}
                    <div class="model-card" class:active-card={m.active}>
                      <div class="mc-main">
                        <div class="mc-title-row">
                          <span class="mc-name">{m.name}</span>
                          {#if m.recommended}
                            <span class="mc-rec-badge">★ 本机推荐（{m.recommend_reason}）</span>
                          {/if}
                        </div>
                        <div class="mc-desc">{m.desc}</div>
                        <div class="mc-tags">
                          <span class="tag gold">
                            {#if m.id === 'q4m'}
                              ⚡ 显卡 ≥ 4GB 高清加速
                            {:else if m.id === 'q2s'}
                              💡 &lt; 4GB 显存 / CPU 极速
                            {:else}
                              📦 本地已识别模型
                            {/if}
                          </span>
                          {#if m.installed}
                            <span class="tag">{m.size_label} · 已就绪</span>
                          {/if}
                          <span class="tag">默认 8K 上下文</span>
                          <span class="tag">识图 / 视频</span>
                          <span class="tag">{m.quant_tag}</span>
                        </div>
                      </div>

                      <div class="mc-action">
                        {#if m.installed && m.active}
                          <span class="in-use-pill">✓ 使用中</span>
                        {:else if m.installed}
                          <button class="act-btn light" onclick={() => handleActivateModel(m.id)} disabled={downloading}>
                            启用此模型
                          </button>
                        {:else}
                          <button
                            class="act-btn"
                            onclick={() => handleDownloadModel(m.id)}
                            disabled={downloading}
                          >
                            ⬇ 下载 · {m.size_label}
                          </button>
                        {/if}
                      </div>
                    </div>
                  {/each}
                {/if}
              </div>
            </section>

            <!-- 4. 高级设置（默认折叠，小白无需关心，默认 8K 上下文） -->
            <section class="rt-section">
              <button class="adv-toggle" type="button" onclick={() => (advOpen = !advOpen)}>
                <span class="adv-arrow" class:open={advOpen}>▸</span>
                <span>高级设置（默认 8K 上下文，普通用户无需修改）</span>
              </button>

              {#if advOpen}
                <div class="card adv-card">
                  <div class="grid-2">
                    <label class="field">
                      <span>上下文窗口长度（默认 8192 = 8K）</span>
                      <select
                        value={String(s.llm.ctx)}
                        onchange={(e) => {
                          s!.llm.ctx = Number((e.currentTarget as HTMLSelectElement).value) || 8192;
                          save();
                        }}
                      >
                        <option value="4096">4K (4096 · 极省显存)</option>
                        <option value="8192">8K (8192 · 默认推荐)</option>
                        <option value="16384">16K (16384 · 长文档)</option>
                        <option value="32768">32K (32768 · 大显存适用)</option>
                      </select>
                    </label>
                    <label class="field">
                      <span>闲置自动释放显存（秒）</span>
                      <input type="number" min="30" bind:value={s.llm.idle_secs} onchange={save} />
                    </label>
                    <label class="field">
                      <span>内部推理端口</span>
                      <input type="number" bind:value={s.llm.port} onchange={save} />
                    </label>
                  </div>

                  <label class="field">
                    <span>额外启动参数（可选）</span>
                    <input bind:value={s.llm.extra_args} onchange={save} placeholder="例如 --threads 8" />
                  </label>

                  <div class="dim">
                    当前引擎目录：{overview?.engine_dir}<br />
                    当前模型目录：{overview?.models_dir}
                  </div>
                </div>
              {/if}
            </section>

          {:else}
            <!-- 子 Tab 2：模型 API（极简配置 + 自动获取模型下拉选择） -->
            <section class="rt-section">
              <div class="card">
                <div class="row-item">
                  <div class="row-meta">
                    <div class="row-label">启用云端 API 模型</div>
                  </div>
                  <div class="engine-dl-group">
                    <button
                      type="button"
                      class="act-btn"
                      onclick={handleTestCloudApi}
                      disabled={testingApi}
                    >
                      {testingApi ? '测试中…' : '⚡ 测试连接'}
                    </button>
                    <input
                      type="checkbox"
                      class="switch"
                      bind:checked={s.llm.api_enabled}
                      onchange={() => {
                        save();
                        flashMsg(s!.llm.api_enabled ? '✓ 已开启云端 API 模型' : '✓ 已切换回本地模型');
                      }}
                    />
                  </div>
                </div>

                {#if apiTestResult}
                  <div class="api-test-banner ok">
                    <span>✓ 连接成功（{apiTestResult.latency_ms} ms）· {apiTestResult.model}</span>
                  </div>
                {/if}
                {#if apiTestError}
                  <div class="api-test-banner err">
                    <span>✕ {apiTestError}</span>
                  </div>
                {/if}

                <div class="divider"></div>

                <!-- 服务商快捷选择 -->
                <div class="field">
                  <span>服务商</span>
                  <div class="provider-grid">
                    {#each API_PROVIDERS as p (p.id)}
                      <button
                        type="button"
                        class="prov-chip"
                        class:active={s.llm.api_provider === p.id}
                        onclick={() => selectApiProvider(p.id)}
                      >
                        {p.name}
                      </button>
                    {/each}
                  </div>
                </div>

                <div class="grid-2">
                  <label class="field">
                    <span>API 地址 (Base URL)</span>
                    <input
                      bind:value={s.llm.api_base_url}
                      oninput={() => triggerAutoFetchModels(450)}
                      onchange={save}
                      placeholder="https://api.siliconflow.cn/v1"
                    />
                  </label>

                  <div class="field">
                    <span>API Key</span>
                    <div class="key-input-wrap">
                      <input
                        type={showApiKey ? 'text' : 'password'}
                        bind:value={s.llm.api_key}
                        oninput={() => triggerAutoFetchModels(450)}
                        onchange={save}
                        placeholder="sk-..."
                      />
                      <button
                        type="button"
                        class="key-eye"
                        onclick={() => (showApiKey = !showApiKey)}
                      >
                        {showApiKey ? '隐藏' : '显示'}
                      </button>
                    </div>
                  </div>
                </div>

                <!-- 模型下拉选择（自动从接口获取模型列表） -->
                <div class="field">
                  <div class="field-head">
                    <span>
                      选择模型
                      {#if fetchingModels}
                        <small class="dim">（正在自动获取模型列表…）</small>
                      {:else if fetchedModels.length > 0}
                        <small class="dim">（已自动获取 {fetchedModels.length} 个模型）</small>
                      {/if}
                    </span>
                    <label class="vision-inline-chk">
                      <input
                        type="checkbox"
                        bind:checked={s.llm.api_vision}
                        onchange={save}
                      />
                      <span>支持视觉识图</span>
                    </label>
                  </div>

                  <div class="model-select-row">
                    {#if !customModelInputMode && availableApiModels.length > 0}
                      <select
                        class="api-model-sel"
                        value={s.llm.api_model}
                        onchange={(e) => {
                          const v = (e.currentTarget as HTMLSelectElement).value;
                          if (v === '__custom__') {
                            customModelInputMode = true;
                            return;
                          }
                          if (v) {
                            s!.llm.api_model = v;
                            s!.llm.api_vision = inferVisionFromModelId(v);
                            save();
                          }
                        }}
                      >
                        {#each availableApiModels as item (item.id)}
                          <option value={item.id}>{item.label}</option>
                        {/each}
                        <option value="__custom__">手动输入模型名称…</option>
                      </select>
                    {:else}
                      <input
                        class="api-model-sel"
                        bind:value={s.llm.api_model}
                        onchange={() => {
                          s!.llm.api_vision = inferVisionFromModelId(s!.llm.api_model);
                          save();
                        }}
                        placeholder="输入模型 ID（如 Qwen/Qwen2.5-VL-72B-Instruct）"
                      />
                      {#if availableApiModels.length > 0}
                        <button
                          type="button"
                          class="act-btn"
                          onclick={() => (customModelInputMode = false)}
                        >
                          返回下拉选择
                        </button>
                      {/if}
                    {/if}

                    <button
                      type="button"
                      class="act-btn"
                      onclick={() => handleFetchCloudModels(false)}
                      disabled={fetchingModels}
                      title="重新从接口获取模型列表"
                    >
                      {fetchingModels ? '获取中…' : '🔄 刷新'}
                    </button>
                  </div>
                </div>
              </div>
            </section>
          {/if}

        {:else if tab === '外观与常规'}
          <section class="group">
            <div class="group-title">主题与窗口</div>
            <div class="card">
              <div class="row-item">
                <div class="row-meta">
                  <div class="row-label">界面主题</div>
                  <div class="row-desc">切换白天、黑夜或跟随系统，所有聊天框与悬浮条立即同步生效</div>
                </div>
                <div class="seg-group">
                  {#each [
                    { id: 'system', label: '跟随系统' },
                    { id: 'light', label: '白天' },
                    { id: 'dark', label: '黑夜' },
                  ] as item (item.id)}
                    <button
                      type="button"
                      class="seg-btn"
                      class:active={s.ui.theme === item.id}
                      onclick={() => {
                        s!.ui.theme = item.id;
                        save();
                      }}
                    >
                      {item.label}
                    </button>
                  {/each}
                </div>
              </div>

              <div class="divider"></div>

              <div class="row-item">
                <div class="row-meta">
                  <div class="row-label">划词/识图结果窗默认大小</div>
                  <div class="row-desc">设置弹出结果窗的初始宽×高（px），也可点击快捷尺寸</div>
                </div>
                <div class="size-controls">
                  <div class="presets">
                    <button type="button" class="ghost sm" onclick={() => setSizePreset(440, 380)}>紧凑</button>
                    <button type="button" class="ghost sm" onclick={() => setSizePreset(520, 460)}>标准</button>
                    <button type="button" class="ghost sm" onclick={() => setSizePreset(640, 560)}>宽敞</button>
                  </div>
                  <div class="wh-inputs">
                    <input type="number" min="360" max="1400" step="10" bind:value={s.ui.result_width} onchange={save} />
                    <span class="dim">×</span>
                    <input type="number" min="240" max="1200" step="10" bind:value={s.ui.result_height} onchange={save} />
                  </div>
                </div>
              </div>
            </div>
          </section>

          <section class="group">
            <div class="group-title">文字与排版</div>
            <div class="card">
              <div class="grid-2">
                <label class="field">
                  <span>预设字体</span>
                  <select
                    value={FONT_PRESETS.some((p) => p.value === s?.ui.font_family) ? s.ui.font_family : '__custom__'}
                    onchange={(e) => {
                      const v = (e.currentTarget as HTMLSelectElement).value;
                      if (v !== '__custom__') {
                        s!.ui.font_family = v;
                        save();
                      }
                    }}
                  >
                    {#each FONT_PRESETS as p (p.label)}
                      <option value={p.value}>{p.label}</option>
                    {/each}
                    <option value="__custom__">自定义字体名称…</option>
                  </select>
                </label>

                <label class="field">
                  <span>自定义字体名称（支持本机任意已安装字体）</span>
                  <input
                    bind:value={s.ui.font_family}
                    oninput={scheduleAutoSave}
                    placeholder="例如：LXGW WenKai, MiSans"
                  />
                </label>
              </div>

              <div class="divider"></div>

              <div class="grid-2">
                <label class="field">
                  <div class="field-head">
                    <span>正文字号</span>
                    <b>{s.ui.font_size} px</b>
                  </div>
                  <input
                    type="range"
                    min="12"
                    max="20"
                    step="1"
                    bind:value={s.ui.font_size}
                    oninput={scheduleAutoSave}
                  />
                </label>

                <label class="field">
                  <div class="field-head">
                    <span>正文行高</span>
                    <b>{s.ui.line_height.toFixed(2)}</b>
                  </div>
                  <input
                    type="range"
                    min="1.4"
                    max="2.1"
                    step="0.04"
                    bind:value={s.ui.line_height}
                    oninput={scheduleAutoSave}
                  />
                </label>
              </div>

              <div class="preview-box">
                <div class="preview-tag">实时排版预览</div>
                <div class="md">
                  极简不是少，而是恰到好处。The quick brown fox jumps over the lazy dog. 0123456789
                </div>
              </div>
            </div>
          </section>

          <section class="group">
            <div class="group-title">系统与常规</div>
            <div class="card">
              <div class="row-item">
                <div class="row-meta">
                  <div class="row-label">开机自动启动</div>
                  <div class="row-desc">登录系统后静默驻留右下角托盘</div>
                </div>
                <input type="checkbox" class="switch" bind:checked={s.autostart} onchange={save} />
              </div>

              <div class="divider"></div>

              <div class="grid-2">
                <label class="field">
                  <span>截图保存目录</span>
                  <input bind:value={s.save_dir} onchange={save} />
                </label>
                <label class="field">
                  <span>默认翻译方向</span>
                  <select bind:value={s.target_lang} onchange={save}>
                    <option value="zh-en">中英互译（自动判断）</option>
                  </select>
                </label>
              </div>
            </div>
          </section>

        {:else if tab === '全局热键'}
          <section class="group">
            <div class="group-title">
              快捷键绑定（点击输入框后直接按下键盘组合键，支持 2~4 个按键如 Ctrl+Alt+A、Ctrl+Shift+A、Alt+Space）
            </div>
            <div class="card">
              <div class="grid-2">
                {#each HOTKEY_ITEMS as item (item.key)}
                  <div class="field">
                    <span>{item.label}</span>
                    <div class="hk-box" class:recording={recordingField === item.key}>
                      <input
                        class="hk-input"
                        readonly
                        value={recordingField === item.key ? recordingPreview : s.hotkeys[item.key]}
                        placeholder="点击此处直接按键录制（留空为禁用）"
                        title={item.hint}
                        onfocus={() => startRecording(item.key)}
                        onblur={stopRecording}
                        onkeydown={(e) => handleHotkeyKeydown(item.key, e)}
                      />
                      {#if s.hotkeys[item.key]}
                        <button
                          type="button"
                          class="hk-clear"
                          title="清空禁用此热键"
                          onclick={() => clearHotkey(item.key)}>×</button
                        >
                      {/if}
                    </div>
                  </div>
                {/each}
              </div>
              <div class="dim">
                提示：点击输入框后，可同时按下组合键（如 <b>Ctrl+Alt+A</b>、<b>Ctrl+Shift+X</b>），也可依次点按修饰键与字母键；按 Backspace 或点击右侧 × 可清空禁用。
              </div>
            </div>
          </section>

        {:else if tab === '划词与复制'}
          <section class="group">
            <div class="group-title">悬浮工具条设置</div>
            <div class="card">
              <div class="row-item">
                <div class="row-meta">
                  <div class="row-label">启用划词与复制悬浮条</div>
                  <div class="row-desc">鼠标选中文本或复制时在光标旁弹出极简操作条</div>
                </div>
                <input type="checkbox" class="switch" bind:checked={s.tip.enabled} onchange={save} />
              </div>

              <div class="divider"></div>

              <div class="grid-3">
                <label class="field">
                  <span>自动消失（毫秒）</span>
                  <input type="number" min="1000" step="500" bind:value={s.tip.hide_ms} onchange={save} />
                </label>
                <label class="field">
                  <span>触发最少字数</span>
                  <input type="number" min="1" bind:value={s.tip.min_chars} onchange={save} />
                </label>
                <label class="field">
                  <span>触发最多字数</span>
                  <input type="number" min="100" bind:value={s.tip.max_chars} onchange={save} />
                </label>
              </div>

              <div class="divider"></div>

              <label class="field">
                <span>屏蔽进程黑名单（每行一个进程名，例如 keepass.exe）</span>
                <textarea rows="5" bind:value={blacklistText} onchange={save}></textarea>
              </label>
            </div>
          </section>

          <section class="group">
            <div class="group-title">Edge TTS 神经网络语音朗读</div>
            <div class="card">
              <div class="grid-2">
                <label class="field">
                  <span>默认发音人（微软 Edge Neural 自然音色）</span>
                  <select bind:value={s.tts.voice} onchange={save}>
                    {#each ttsVoices as v (v.id)}
                      <option value={v.id}>{v.name} · {v.gender}（{v.desc}）</option>
                    {:else}
                      <option value="zh-CN-XiaoxiaoNeural">晓晓 (Xiaoxiao) · 女声（温暖自然·默认）</option>
                    {/each}
                  </select>
                </label>

                <label class="field">
                  <span>朗读语速</span>
                  <select bind:value={s.tts.rate} onchange={save}>
                    <option value="-30%">慢速 (-30%)</option>
                    <option value="-15%">稍慢 (-15%)</option>
                    <option value="+0%">标准语速 (+0%)</option>
                    <option value="+15%">稍快 (+15%)</option>
                    <option value="+30%">快速 (+30%)</option>
                    <option value="+50%">倍速 (+50%)</option>
                  </select>
                </label>
              </div>

              <div class="divider"></div>

              <div class="row-item">
                <div class="row-meta">
                  <div class="row-label">外语文本智能匹配母语发音人</div>
                  <div class="row-desc">当划词或 AI 输出为纯英文、纯日文或纯韩文时，自动选用对应母语高自然度音色朗读</div>
                </div>
                <input type="checkbox" class="switch" bind:checked={s.tts.auto_lang} onchange={save} />
              </div>

              <div class="divider"></div>

              <div class="row-item">
                <div class="row-meta">
                  <div class="row-label">音色效果试听</div>
                  <div class="row-desc">在划词复制胶囊栏和聊天界面的每条 AI 输出下方均可直接点击「朗读」按钮发声</div>
                </div>
                <SpeakButton
                  id="settings-tts-preview"
                  text="你好，我是 miniBot 桌面 AI 助手，现在为您演示微软 Edge TTS 神经网络自然语音朗读效果。"
                  label="试听当前音色"
                  variant="pill"
                />
              </div>
            </div>
          </section>

        {:else if tab === 'OpenAI 网关'}
          <section class="group">
            <div class="group-title">对外兼容网关</div>
            <div class="card">
              <div class="row-item">
                <div class="row-meta">
                  <div class="row-label">启用 OpenAI 兼容 API 服务</div>
                  <div class="row-desc">供第三方工具调用本地模型与 OCR/二维码接口（重启生效）</div>
                </div>
                <input type="checkbox" class="switch" bind:checked={s.api.enabled} onchange={save} />
              </div>

              <div class="divider"></div>

              <div class="grid-2">
                <label class="field">
                  <span>监听端口</span>
                  <input type="number" bind:value={s.api.port} onchange={save} />
                </label>
                <label class="field">
                  <span>显卡繁忙策略</span>
                  <select bind:value={s.api.busy_policy} onchange={save}>
                    <option value="reject">返回 503（推荐）</option>
                    <option value="force">强制加载</option>
                  </select>
                </label>
              </div>

              <label class="field">
                <span>API Key（留空则免密访问）</span>
                <input bind:value={s.api.api_key} onchange={save} placeholder="sk-..." />
              </label>

              <div class="divider"></div>

              <div class="row-item">
                <div class="row-meta">
                  <div class="row-label">允许局域网访问</div>
                  <div class="row-desc">开启时必须配置 API Key</div>
                </div>
                <input type="checkbox" class="switch" bind:checked={s.api.lan} onchange={save} />
              </div>

              <div class="preview-box mono selectable">
                Base URL：{info?.api_url}<br />
                模型名称：minibot-2b
              </div>
            </div>
          </section>

        {:else if tab === '历史清理'}
          <section class="group">
            <div class="group-title">数据存储与自动清理策略</div>
            <div class="card">
              <div class="row-item">
                <div class="row-meta">
                  <div class="row-label">主 AI 聊天窗多轮记录</div>
                  <div class="row-desc">存储在聊天窗左侧列表中，除非手动点击删除，否则永久保留</div>
                </div>
                <span class="badge-ok">永久保留</span>
              </div>

              <div class="divider"></div>

              <div class="row-item">
                <div class="row-meta">
                  <div class="row-label">划词翻译与截图识图临时记录保留天数</div>
                  <div class="row-desc">默认保留 30 天（一个月），超期自动清理数据与缓存图片</div>
                </div>
                <input
                  type="number"
                  min="1"
                  max="30"
                  style="width: 84px"
                  bind:value={s.history_days}
                  onchange={save}
                />
              </div>

              <div class="divider"></div>

              <div class="row-item">
                <div class="row-meta">
                  <div class="row-label">保存截图识别的缩略图 (WebP)</div>
                  <div class="row-desc">随 30 天到期记录一同自动清理</div>
                </div>
                <input type="checkbox" class="switch" bind:checked={s.history_keep_images} onchange={save} />
              </div>

              <div class="divider"></div>

              <div class="gpu-actions">
                <button onclick={() => ipc.openPage('history', '划词与识图历史')}>查看划词/识图临时记录</button>
                <button
                  onclick={async () => {
                    if (confirm('确定清空全部划词与识图临时历史？（不影响主聊天窗的永久对话）')) {
                      await ipc.historyClear();
                      flashMsg('✓ 已清空临时历史');
                    }
                  }}>清空临时记录</button
                >
              </div>
            </div>
          </section>
        {/if}
      </main>
    {/if}
  </div>
</div>

<style>
  :global(html, body) {
    margin: 0;
    padding: 0;
    height: 100%;
    background: var(--bg-solid) !important;
    user-select: none;
  }
  .page {
    display: flex;
    width: 100vw;
    height: 100vh;
    background: var(--bg-solid);
    overflow: hidden;
  }

  /* 左侧导航栏 */
  .sidebar {
    width: 176px;
    flex-shrink: 0;
    background: var(--panel);
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
  }
  .brand {
    height: 48px;
    padding: 0 16px;
    display: flex;
    align-items: center;
    gap: 7px;
    border-bottom: 1px solid var(--border);
  }
  .brand-logo {
    width: 22px;
    height: 22px;
    object-fit: contain;
    flex-shrink: 0;
  }
  .brand-name {
    font-weight: 650;
    font-size: 14px;
    letter-spacing: -0.01em;
  }
  .brand small {
    color: var(--fg-dim);
    font-size: 11px;
    margin-left: auto;
  }
  .nav-list {
    padding: 10px 8px;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .nav-btn {
    text-align: left;
    font-size: 13px;
    padding: 7px 12px;
    border-radius: 7px;
    border: none;
    background: transparent;
    color: var(--fg-dim);
  }
  .nav-btn:hover {
    color: var(--fg);
    background: var(--hover);
  }
  .nav-btn.active {
    background: var(--bg-solid);
    color: var(--fg);
    font-weight: 550;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.05);
  }

  /* 右侧内容列 */
  .content-col {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .top-bar {
    height: 48px;
    padding: 0 12px 0 24px;
    border-bottom: 1px solid var(--border);
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-shrink: 0;
  }
  .tab-title {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  .top-right {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .save-msg {
    font-size: 12px;
    color: var(--ok);
  }
  .save-btn {
    padding: 4px 12px;
    font-size: 12.5px;
    border-radius: 7px;
  }
  .win-sep {
    width: 1px;
    height: 16px;
    background: var(--border);
    margin: 0 2px;
  }
  .win-ctrl {
    width: 28px;
    height: 28px;
    padding: 0;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--fg-dim);
    font-size: 12px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .win-ctrl:hover {
    background: var(--hover);
    color: var(--fg);
  }
  .win-ctrl.close:hover {
    background: rgba(239, 68, 68, 0.14);
    color: var(--danger);
  }

  /* 主滚动区 */
  .main-scroll {
    flex: 1;
    overflow-y: auto;
    padding: 20px 28px 36px;
    display: flex;
    flex-direction: column;
    gap: 22px;
  }

  /* 本地模型页：参考图极简风格 */
  .rt-section {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .rt-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  .rt-head-left {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .rt-icon {
    font-size: 14px;
    color: var(--fg-dim);
  }
  .rt-title {
    font-size: 13.5px;
    font-weight: 600;
    color: var(--fg);
  }
  .ver-pill {
    font-size: 11.5px;
    padding: 2px 8px;
    border-radius: 5px;
    background: var(--panel);
    border: 1px solid var(--border);
    color: var(--fg-dim);
    font-family: Consolas, monospace;
  }
  .ver-pill.backend {
    color: var(--fg);
  }
  .run-badge {
    font-size: 11.5px;
    padding: 3px 10px;
    border-radius: 6px;
    background: var(--panel);
    border: 1px solid var(--border);
    color: var(--fg-dim);
  }
  .run-badge.running {
    color: var(--ok);
    border-color: color-mix(in srgb, var(--ok) 35%, var(--border));
    background: color-mix(in srgb, var(--ok) 10%, transparent);
  }

  .rt-block {
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 11px;
    padding: 14px 18px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .rt-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }
  .rt-info {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .rt-name {
    font-size: 13.5px;
    font-weight: 600;
    color: var(--fg);
  }
  .rt-desc {
    font-size: 12px;
    color: var(--fg-dim);
    line-height: 1.5;
  }
  .rt-divider {
    height: 1px;
    background: var(--border);
  }

  .engine-dl-group {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }
  .backend-sel {
    font-size: 12px;
    padding: 5px 8px;
    background: var(--bg-solid);
    border-radius: 7px;
  }
  .act-btn {
    padding: 5px 13px;
    font-size: 12.5px;
    font-weight: 500;
    border-radius: 7px;
    background: var(--bg-solid);
    border: 1px solid var(--border);
    color: var(--fg);
    flex-shrink: 0;
  }
  .act-btn:hover:not(:disabled) {
    background: var(--hover);
  }
  .act-btn.light {
    background: var(--fg);
    color: var(--bg-solid);
    border-color: transparent;
  }
  .act-btn.light:hover:not(:disabled) {
    opacity: 0.9;
  }

  /* 本机配置条 */
  .hw-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 22px;
    padding: 12px 18px;
    border-radius: 11px;
    background: var(--panel);
    border: 1px solid var(--border);
    font-size: 12.5px;
    color: var(--fg-dim);
  }
  .hw-item {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .hw-ic {
    opacity: 0.75;
  }
  .hw-val {
    color: var(--fg);
    font-weight: 500;
  }

  /* 下载进度条 */
  .dl-banner {
    background: var(--panel);
    border: 1px solid var(--accent-blue);
    border-radius: 10px;
    padding: 10px 14px;
    display: flex;
    flex-direction: column;
    gap: 7px;
  }
  .dl-top {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 12px;
  }
  .dl-msg {
    flex: 1;
    color: var(--fg);
    font-weight: 500;
  }
  .dl-pct {
    font-family: Consolas, monospace;
    color: var(--accent-blue);
    font-weight: 600;
  }
  .cancel-dl {
    color: var(--danger);
  }
  .dl-track {
    height: 6px;
    border-radius: 999px;
    background: var(--bg-solid);
    overflow: hidden;
  }
  .dl-fill {
    height: 100%;
    background: var(--accent-blue);
    transition: width 0.15s ease;
  }

  /* 模型列表头部工具 */
  .model-head-tools {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .mirror-switch {
    display: inline-flex;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 7px;
    padding: 2px;
    gap: 2px;
  }
  .m-btn {
    border: none;
    background: transparent;
    color: var(--fg-dim);
    font-size: 11.5px;
    padding: 3px 9px;
    border-radius: 5px;
  }
  .m-btn.active {
    background: var(--bg-solid);
    color: var(--fg);
    font-weight: 500;
  }
  .dir-icon-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 4px 10px;
    font-size: 12px;
    border-radius: 7px;
    background: var(--panel);
    border: 1px solid var(--border);
    color: var(--fg-dim);
  }
  .dir-icon-btn:hover {
    color: var(--fg);
    background: var(--hover);
  }

  /* 模型卡片 */
  .model-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .model-card {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 14px 18px;
    border-radius: 11px;
    background: var(--panel);
    border: 1px solid var(--border);
    transition: border-color 0.15s;
  }
  .model-card.active-card {
    border-color: color-mix(in srgb, var(--ok) 45%, var(--border));
  }
  .mc-main {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .mc-title-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .mc-name {
    font-size: 14px;
    font-weight: 650;
    color: var(--fg);
  }
  .mc-rec-badge {
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 5px;
    background: color-mix(in srgb, var(--warn) 16%, transparent);
    color: var(--warn);
    font-weight: 550;
  }
  .mc-desc {
    font-size: 12px;
    color: var(--fg-dim);
  }
  .mc-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 3px;
  }
  .tag {
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 5px;
    background: var(--bg-solid);
    border: 1px solid var(--border);
    color: var(--fg-dim);
  }
  .tag.gold {
    background: color-mix(in srgb, var(--warn) 14%, transparent);
    border-color: color-mix(in srgb, var(--warn) 30%, transparent);
    color: var(--warn);
  }
  .in-use-pill {
    font-size: 12px;
    padding: 5px 12px;
    border-radius: 7px;
    background: color-mix(in srgb, var(--ok) 14%, transparent);
    color: var(--ok);
    font-weight: 600;
  }

  /* 折叠高级设置 */
  .adv-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    border: none;
    background: transparent;
    color: var(--fg-dim);
    font-size: 12.5px;
    padding: 4px 2px;
    text-align: left;
  }
  .adv-toggle:hover {
    color: var(--fg);
  }
  .adv-arrow {
    display: inline-block;
    transition: transform 0.15s ease;
  }
  .adv-arrow.open {
    transform: rotate(90deg);
  }
  .adv-card {
    margin-top: 2px;
  }

  /* 通用分组卡片 */
  .group {
    display: flex;
    flex-direction: column;
    gap: 7px;
  }
  .group-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--fg-dim);
    padding-left: 2px;
  }
  .card {
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 11px;
    padding: 14px 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .divider {
    height: 1px;
    background: var(--border);
    margin: 2px 0;
  }

  .row-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }
  .row-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .row-label {
    font-size: 13.5px;
    font-weight: 500;
    color: var(--fg);
  }
  .row-desc {
    font-size: 12px;
    color: var(--fg-dim);
  }

  .grid-2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px 16px;
  }
  .grid-3 {
    display: grid;
    grid-template-columns: 1fr 1fr 1fr;
    gap: 12px 14px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 12.5px;
    color: var(--fg-dim);
  }
  .field-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .field-head b {
    color: var(--fg);
    font-weight: 550;
  }
  .field input,
  .field select,
  .field textarea {
    color: var(--fg);
    background: var(--bg-solid);
  }

  .seg-group {
    display: inline-flex;
    background: var(--bg-solid);
    border: 1px solid var(--border);
    padding: 2px;
    border-radius: 8px;
    gap: 2px;
    flex-shrink: 0;
  }
  .seg-btn {
    border: none;
    background: transparent;
    color: var(--fg-dim);
    padding: 4px 12px;
    font-size: 12.5px;
    border-radius: 6px;
  }
  .seg-btn:hover {
    color: var(--fg);
  }
  .seg-btn.active {
    background: var(--panel-elevated);
    color: var(--fg);
    font-weight: 550;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.08);
  }

  .size-controls {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-shrink: 0;
  }
  .presets {
    display: flex;
    gap: 4px;
  }
  .wh-inputs {
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .wh-inputs input {
    width: 72px;
    background: var(--bg-solid);
    text-align: center;
  }
  .sm {
    font-size: 11.5px;
    padding: 3px 8px;
  }

  .preview-box {
    background: var(--bg-solid);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 10px 14px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .preview-tag {
    font-size: 11px;
    color: var(--fg-dim);
  }

  .switch {
    width: 18px;
    height: 18px;
    accent-color: var(--fg);
    cursor: pointer;
  }

  .gpu-actions {
    display: flex;
    gap: 8px;
    margin-top: 4px;
  }

  .hk-box {
    position: relative;
    display: flex;
    align-items: center;
  }
  .hk-input {
    width: 100%;
    padding-right: 30px;
    cursor: pointer;
    font-weight: 500;
  }
  .hk-box.recording .hk-input {
    border-color: var(--accent, #3b82f6);
    box-shadow: 0 0 0 2px rgba(59, 130, 246, 0.18);
    color: var(--accent, #3b82f6);
  }
  .hk-clear {
    position: absolute;
    right: 6px;
    width: 20px;
    height: 20px;
    padding: 0;
    border: none;
    border-radius: 5px;
    background: transparent;
    color: var(--fg-dim);
    font-size: 14px;
    line-height: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .hk-clear:hover {
    background: var(--hover);
    color: var(--fg);
  }

  .badge-ok {
    font-size: 11.5px;
    padding: 3px 9px;
    border-radius: 999px;
    background: rgba(16, 185, 129, 0.12);
    color: var(--ok);
    font-weight: 500;
  }
  .alert {
    background: color-mix(in srgb, var(--warn) 12%, transparent);
    border: 1px solid var(--warn);
    border-radius: 8px;
    padding: 8px 12px;
    font-size: 12.5px;
    white-space: pre-wrap;
  }
  .dim {
    color: var(--fg-dim);
    font-size: 12px;
  }
  .mono {
    font-family: 'Cascadia Code', Consolas, monospace;
    font-size: 12px;
  }

  /* 模型页二级 Tab 栏 */
  .subtab-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding-bottom: 4px;
    border-bottom: 1px solid var(--border);
  }
  .subtab-group {
    display: inline-flex;
    background: var(--panel);
    border: 1px solid var(--border);
    padding: 3px;
    border-radius: 9px;
    gap: 4px;
  }
  .subtab-btn {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    border: none;
    background: transparent;
    color: var(--fg-dim);
    padding: 6px 14px;
    font-size: 13px;
    font-weight: 500;
    border-radius: 6px;
  }
  .subtab-btn:hover {
    color: var(--fg);
  }
  .subtab-btn.active {
    background: var(--bg-solid);
    color: var(--fg);
    font-weight: 600;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.06);
  }
  .subtab-pill {
    font-size: 10.5px;
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--panel);
    border: 1px solid var(--border);
    color: var(--fg-dim);
    font-weight: 500;
  }
  .subtab-pill.ok {
    background: color-mix(in srgb, var(--ok) 14%, transparent);
    border-color: color-mix(in srgb, var(--ok) 35%, transparent);
    color: var(--ok);
  }
  .mode-indicator {
    font-size: 12px;
    padding: 4px 10px;
    border-radius: 6px;
    background: var(--panel);
    border: 1px solid var(--border);
    color: var(--fg-dim);
  }
  .mode-indicator.cloud {
    color: var(--accent-blue);
    border-color: color-mix(in srgb, var(--accent-blue) 35%, var(--border));
    background: color-mix(in srgb, var(--accent-blue) 8%, transparent);
    font-weight: 500;
  }

  /* 硬件建议横幅与云端启用横幅 */
  .hw-recommend-banner,
  .cloud-active-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 12px 16px;
    border-radius: 10px;
    background: color-mix(in srgb, var(--warn) 10%, var(--panel));
    border: 1px solid color-mix(in srgb, var(--warn) 40%, var(--border));
  }
  .cloud-active-banner {
    background: color-mix(in srgb, var(--accent-blue) 8%, var(--panel));
    border-color: color-mix(in srgb, var(--accent-blue) 35%, var(--border));
  }
  .hwr-main {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .hwr-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--fg);
  }
  .hwr-desc {
    font-size: 12px;
    color: var(--fg-dim);
    line-height: 1.45;
  }

  /* 云端 API 服务商与模型卡片 */
  .provider-grid {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 2px;
  }
  .prov-chip {
    padding: 5px 11px;
    font-size: 12px;
    border-radius: 7px;
    background: var(--bg-solid);
    border: 1px solid var(--border);
    color: var(--fg-dim);
  }
  .prov-chip:hover {
    color: var(--fg);
    background: var(--hover);
  }
  .prov-chip.active {
    background: var(--fg);
    color: var(--bg-solid);
    border-color: transparent;
    font-weight: 550;
  }

  .key-input-wrap {
    position: relative;
    display: flex;
    align-items: center;
  }
  .key-input-wrap input {
    width: 100%;
    padding-right: 48px;
  }
  .key-eye {
    position: absolute;
    right: 6px;
    padding: 2px 7px;
    font-size: 11px;
    border-radius: 5px;
    border: 1px solid var(--border);
    background: var(--panel);
    color: var(--fg-dim);
  }
  .key-eye:hover {
    color: var(--fg);
  }

  .model-select-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .api-model-sel {
    flex: 1;
    min-width: 0;
  }
  .vision-inline-chk {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
    color: var(--fg);
    cursor: pointer;
    user-select: none;
  }
  .vision-inline-chk input {
    width: 14px;
    height: 14px;
    accent-color: var(--fg);
    cursor: pointer;
  }

  .api-test-banner {
    padding: 9px 14px;
    border-radius: 9px;
    font-size: 12.5px;
    line-height: 1.45;
  }
  .api-test-banner.ok {
    background: color-mix(in srgb, var(--ok) 12%, var(--panel));
    border: 1px solid color-mix(in srgb, var(--ok) 40%, var(--border));
    color: var(--ok);
  }
  .api-test-banner.err {
    background: rgba(239, 68, 68, 0.08);
    border: 1px solid rgba(239, 68, 68, 0.3);
    color: var(--danger);
  }
</style>
