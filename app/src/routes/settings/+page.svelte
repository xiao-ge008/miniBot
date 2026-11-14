<script lang="ts">
  // 极简无边框设置面板：
  // 1. 本地模型页：本机配置检查（GPU/显存/内存）+ 本地引擎管理（N卡CUDA/A卡&Intel Vulkan/Mac Metal/CPU）+ 智能模型下载（Q4M/Q2S，支持 hf-mirror）+ 修改目录自动迁移模型 + 折叠高级设置（默认 8K 上下文）
  // 2. 窗口互斥：从聊天窗打开设置时聊天窗自动隐藏，关闭设置窗时自动恢复聊天窗
  import { onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { ipc, onLlmStatus } from '$lib/core/ipc';
  import SpeakButton from '$lib/components/SpeakButton.svelte';
  import KbPromptsPanel from '$lib/components/KbPromptsPanel.svelte';
  import type {
    AppInfo,
    CloudTestResult,
    CustomLocalModel,
    DownloadProgress,
    EngineOverview,
    KbStats,
    LayaStatus,
    ModelCard,
    ModelParams,
    ScannedFolderResult,
    Settings,
    StatusInfo,
    UpdateDownloadProgress,
    UpdateInfo,
    VoiceOption,
  } from '$lib/core/types';

  const win = getCurrentWindow();

  const TABS = ['模型', '个人知识库', '外观与常规', '全局热键', '划词与复制', 'OpenAI 网关', '历史清理', '版本更新'] as const;

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

  // 默认模型点击「下载」后展开线路选择（默认 modelscope 线路）
  let dlRouteOpenId = $state<string | null>(null);
  let dlRouteChoice = $state<'modelscope' | 'hf-mirror' | 'huggingface'>('modelscope');

  // 每个模型卡片右下角独立的「高级设置」展开状态
  let expandedAdvModelId = $state<string | null>(null);

  // 用户自定义本地模型（新增 / 编辑）状态
  let showCustomModelModal = $state(false);
  let scanningCustomFolder = $state(false);
  let savingCustomModel = $state(false);
  let scannedFolder = $state<ScannedFolderResult | null>(null);
  let customScanError = $state('');
  let customForm = $state<{
    id: string;
    name: string;
    desc: string;
    folder_path: string;
    model_path: string;
    mmproj_path: string;
    reasoning_effort: 'off' | 'low' | 'medium' | 'high' | 'max';
  }>({
    id: '',
    name: '',
    desc: '',
    folder_path: '',
    model_path: '',
    mmproj_path: '',
    reasoning_effort: 'off',
  });

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

  // 个人知识库状态
  let kbStats = $state<KbStats | null>(null);
  let layaStatus = $state<LayaStatus | null>(null);
  let layaDownloading = $state(false);
  let layaDlProgress = $state<DownloadProgress | null>(null);
  let layaDlError = $state('');
  let mdContextMenuRegistered = $state(false);

  async function handleToggleMdContextMenu() {
    try {
      const next = !mdContextMenuRegistered;
      const res = await ipc.docSetContextMenu(next);
      mdContextMenuRegistered = res;
      flashMsg(res ? '✓ 已注册系统右键菜单「使用 miniBot 编辑 Markdown」' : '✓ 已移除右键菜单');
    } catch (e: any) {
      flashMsg(`操作失败: ${e?.message ?? e}`);
    }
  }

  async function refreshKbStats() {
    try {
      kbStats = await ipc.kbGetStats();
    } catch (e) {
      console.error('获取知识库统计失败:', e);
    }
  }

  let layaAutoRedownloadTried = false;

  async function refreshLayaStatus() {
    try {
      layaStatus = await ipc.kbLayaStatus();
      if (s && layaStatus) {
        s.knowledge.laya_enabled = layaStatus.enabled;
      }
      if (layaStatus?.needs_redownload && !layaAutoRedownloadTried && !layaDownloading) {
        layaAutoRedownloadTried = true;
        flashMsg('🔄 检测到旧版 Laya 模型已升级为 GGUF，正在为您自动重新下载 (345MB)…', 5000);
        void handleDownloadLaya();
      }
    } catch (e) {
      console.error('获取 Laya 状态失败:', e);
    }
  }

  let kbSyncing = $state(false);

  async function handlePickKbFolder() {
    if (kbSyncing) return;
    try {
      kbSyncing = true;
      const res = await ipc.kbPickFolder();
      if (res && s) {
        s.knowledge.base_path = res.folder_path;
        await refreshKbStats();
        flashMsg(
          `✓ 已切换至新文件夹，自动扫描并索引了 ${res.scanned_docs} 篇文档 (共 ${res.total_chunks} 个知识切片)`,
          5000,
        );
      }
    } catch (e) {
      flashMsg(`切换目录失败: ${e}`, 4000);
    } finally {
      kbSyncing = false;
    }
  }

  async function handleSyncKbIndex() {
    if (kbSyncing) return;
    kbSyncing = true;
    try {
      flashMsg('🔍 正在全量扫描文本并建立索引…', 2500);
      const res = await ipc.kbSyncIndex();
      await refreshKbStats();
      flashMsg(
        `✓ 索引同步完成：已索引 ${res.scanned_docs} 篇文档 (共 ${res.total_chunks} 个知识切片)`,
        4500,
      );
    } catch (e) {
      flashMsg(`索引扫描失败: ${e}`, 4000);
    } finally {
      kbSyncing = false;
    }
  }

  async function handleDownloadLaya() {
    if (layaDownloading) return;
    layaDownloading = true;
    layaDlError = '';
    layaDlProgress = {
      target: 'laya',
      stage: 'downloading',
      file: 'laya_multilingual_q8_0.gguf',
      downloaded_mb: 0,
      total_mb: 345,
      percent: 1,
      message: '正在连接 mys/laya-multilingual-GGUF 下载源…',
    };
    try {
      layaStatus = await ipc.kbDownloadLaya((p) => {
        layaDlProgress = p;
      });
      if (s) s.knowledge.laya_enabled = true;
      flashMsg('✓ Laya GGUF 加速模型已下载就绪并自动启用');
    } catch (e) {
      layaDlError = `下载失败: ${e}`;
      flashMsg(`下载失败: ${e}`, 4000);
    } finally {
      layaDownloading = false;
      await refreshLayaStatus();
    }
  }

  async function handleDeleteLaya() {
    if (!confirm('确定要删除本地 Laya 加速模型以释放磁盘空间吗？')) return;
    try {
      await ipc.kbDeleteLaya();
      if (s) s.knowledge.laya_enabled = false;
      await refreshLayaStatus();
      flashMsg('✓ 已移除加速模型并释放空间');
    } catch (e) {
      flashMsg(`删除失败: ${e}`);
    }
  }

  // 版本检测与在线自动更新状态
  let updateInfo = $state<UpdateInfo | null>(null);
  let checkingUpdate = $state(false);
  let updateCheckError = $state('');
  let downloadingUpdate = $state(false);
  let updateDlProgress = $state<UpdateDownloadProgress | null>(null);
  let downloadedInstallerPath = $state('');
  let installingUpdate = $state(false);

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

  type HotkeyField = 'launcher' | 'capture' | 'record' | 'selection' | 'color';
  const HOTKEY_ITEMS: { key: HotkeyField; label: string; hint: string }[] = [
    { key: 'launcher', label: '呼出 AI 智能聊天框', hint: '示例：Alt+Space、Ctrl+Alt+Space' },
    { key: 'capture', label: '区域截图识别', hint: '示例：Alt+Q、Ctrl+Alt+A、Ctrl+Shift+A' },
    { key: 'record', label: '屏幕录制（再按一次完成保存）', hint: '示例：Alt+R、Ctrl+Alt+R' },
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

  let checkingPort = $state(false);
  let portTip = $state('');
  let portTipOk = $state(true);

  async function handleCheckPort() {
    if (!s) return;
    checkingPort = true;
    portTip = '';
    try {
      const res = await ipc.llmCheckPort(s.llm.port || 18080);
      if (res.available) {
        portTipOk = true;
        portTip = `✓ 端口 ${res.port} 可用`;
      } else {
        portTipOk = false;
        portTip = `⚠ 端口 ${res.port} 被占用，已自动建议并切换为 ${res.suggested_port}`;
        s.llm.port = res.suggested_port;
        await save();
      }
    } catch (e: any) {
      portTipOk = false;
      portTip = `检测失败：${e?.message ?? e}`;
    } finally {
      checkingPort = false;
      setTimeout(() => (portTip = ''), 6000);
    }
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
    if (!loaded.llm.ctx || loaded.llm.ctx === 20480 || loaded.llm.ctx === 8192) loaded.llm.ctx = 32768;
    if (!loaded.llm.mirror) loaded.llm.mirror = 'modelscope';
    dlRouteChoice =
      loaded.llm.mirror === 'hf-mirror' || loaded.llm.mirror === 'huggingface'
        ? loaded.llm.mirror
        : 'modelscope';
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
    if (!loaded.knowledge) {
      loaded.knowledge = {
        base_path: '',
        laya_enabled: false,
        auto_search: false,
        candidate_count: 20,
        top_k: 5,
        clip_auto_distill: true,
      };
    } else if (typeof loaded.knowledge.clip_auto_distill !== 'boolean') {
      loaded.knowledge.clip_auto_distill = true;
    }
    s = loaded;
    info = await ipc.appInfo();
    ttsVoices = await ipc.ttsVoices().catch(() => []);
    blacklistText = s.tip.blacklist.join('\n');
    errors = info.hotkey_errors;
    previewUi();
    overview = await ipc.engineOverview();
    triggerAutoFetchModels(120);
    handleCheckUpdate(false);
    refreshKbStats();
    refreshLayaStatus();
    ipc.docIsContextMenuRegistered().then((r) => (mdContextMenuRegistered = r));
  }

  async function handleCheckUpdate(manual = true) {
    if (checkingUpdate) return;
    checkingUpdate = true;
    updateCheckError = '';
    try {
      const res = await ipc.appCheckUpdate();
      updateInfo = res;
      if (manual) {
        if (res.has_update) {
          flashMsg(`🎉 发现新版本 v${res.latest_version}，可一键下载更新`, 4000);
        } else {
          flashMsg(`✓ 当前已是最新版本 (v${res.current_version})`, 3000);
        }
      }
    } catch (e) {
      updateCheckError = String(e);
      if (manual) {
        flashMsg(`检查更新失败：${e}`, 4000);
      }
    } finally {
      checkingUpdate = false;
    }
  }

  async function handleDownloadAndInstallUpdate() {
    if (downloadingUpdate || !updateInfo?.download_url) return;
    downloadingUpdate = true;
    updateCheckError = '';
    downloadedInstallerPath = '';
    updateDlProgress = {
      stage: 'downloading',
      file_name: updateInfo.asset_name || 'miniBot-setup',
      downloaded_mb: 0,
      total_mb: updateInfo.asset_size_mb || 0,
      speed_mbps: 0,
      percent: 1,
      saved_path: '',
      message: '正在准备下载新版本安装包…',
    };
    try {
      const savedPath = await ipc.appDownloadUpdate(
        updateInfo.download_url,
        updateInfo.asset_name,
        true,
        (p) => {
          updateDlProgress = p;
        },
      );
      downloadedInstallerPath = savedPath;
      flashMsg('✓ 更新包下载完成', 3500);
      if (savedPath) {
        await handleRunInstaller(savedPath);
      }
    } catch (e) {
      const errStr = String(e);
      if (!errStr.includes('已取消')) {
        updateCheckError = `下载更新失败：${errStr}`;
        flashMsg(`下载更新失败：${errStr}`, 4000);
      } else {
        flashMsg('已取消下载更新');
      }
    } finally {
      downloadingUpdate = false;
    }
  }

  async function handleRunInstaller(pathOverride?: string) {
    const targetPath = pathOverride || downloadedInstallerPath;
    if (!targetPath || installingUpdate) return;
    installingUpdate = true;
    try {
      flashMsg('🚀 正在启动安装程序并升级 miniBot…', 4000);
      await ipc.appInstallUpdate(targetPath);
    } catch (e) {
      updateCheckError = `启动安装程序失败：${e}`;
      flashMsg(`启动安装失败：${e}`, 4000);
    } finally {
      installingUpdate = false;
    }
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

  async function handleDownloadModel(modelId: string, mirrorOverride?: string) {
    if (downloading || !s) return;
    const chosenMirror = mirrorOverride || dlRouteChoice || 'modelscope';
    s.llm.mirror = chosenMirror;
    dlRouteOpenId = null;
    downloading = true;
    dlError = '';
    const routeLabel =
      chosenMirror === 'modelscope'
        ? 'ModelScope (魔搭)'
        : chosenMirror === 'huggingface'
          ? 'HuggingFace'
          : '国内镜像 (hf-mirror)';
    dlProgress = {
      target: modelId,
      stage: 'downloading',
      file: modelId.toUpperCase(),
      downloaded_mb: 0,
      total_mb: 0,
      percent: 1,
      message: `正在通过 ${routeLabel} 线路连接下载源…`,
    };
    try {
      overview = await ipc.engineDownloadModel(modelId, chosenMirror, (p) => {
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
      flashMsg('✓ 已切换当前使用的模型及专属参数');
    } catch (e) {
      flashMsg(`切换失败：${e}`);
    }
  }

  function formatCtxLabel(ctx?: number): string {
    const c = ctx || 32768;
    if (c === 100128) return '100K';
    return `${Math.round(c / 1024)}K`;
  }

  function formatReasoningShort(eff?: string): string {
    switch (eff) {
      case 'low':
        return '低';
      case 'medium':
        return '中';
      case 'high':
        return '高';
      case 'max':
        return '满';
      default:
        return '关';
    }
  }

  function buildModelTip(m: ModelCard): string {
    const lines: string[] = [];
    if (m.id === 'q4m') {
      lines.push('定位：默认均衡视觉模型（显存 ≥ 4GB 推荐）');
    } else if (m.id === 'q2s') {
      lines.push('定位：默认轻量视觉模型（< 4GB 显存 / 核显 / CPU 推荐）');
    } else {
      lines.push('定位：用户自定义本地 GGUF 模型');
    }
    if (m.model_path) {
      lines.push(`主模型：${m.model_path}`);
    }
    if (m.mmproj_path) {
      lines.push(`视觉组件：${m.mmproj_path.split(/[\\/]/).pop()}`);
    } else {
      lines.push('视觉组件：无（识图自动调用内置离线 OCR）');
    }
    return lines.join('\n');
  }

  async function handleSaveModelParams(m: ModelCard) {
    try {
      const cleanParams: ModelParams = {
        ctx: Number(m.params.ctx) || 32768,
        ngl: Number(m.params.ngl ?? 99),
        kv_quant: m.params.kv_quant || 'q8_0',
        reasoning_effort: (m.params.reasoning_effort || 'off') as ModelParams['reasoning_effort'],
        idle_secs: Math.max(30, Number(m.params.idle_secs) || 180),
        extra_args: (m.params.extra_args || '').trim(),
      };
      overview = await ipc.engineUpdateModelParams(m.id, cleanParams);
      s = await ipc.getSettings();
      flashMsg(`✓ 已保存「${m.name}」的独立参数`);
    } catch (e) {
      flashMsg(`保存模型参数失败：${e}`, 3500);
    }
  }

  function openAddCustomModel() {
    customScanError = '';
    scannedFolder = null;
    customForm = {
      id: '',
      name: '',
      desc: '',
      folder_path: '',
      model_path: '',
      mmproj_path: '',
      reasoning_effort: 'off',
    };
    showCustomModelModal = true;
  }

  function openEditCustomModel(m: ModelCard) {
    customScanError = '';
    scannedFolder = null;
    customForm = {
      id: m.id,
      name: m.name,
      desc: m.desc,
      folder_path: m.folder_path || '',
      model_path: m.model_path,
      mmproj_path: m.mmproj_path || '',
      reasoning_effort: (m.params?.reasoning_effort || 'off') as 'off' | 'low' | 'medium' | 'high' | 'max',
    };
    showCustomModelModal = true;
  }

  async function handleScanCustomFolder() {
    if (scanningCustomFolder) return;
    scanningCustomFolder = true;
    customScanError = '';
    try {
      const res = await ipc.engineScanCustomFolder();
      if (!res) return;
      scannedFolder = res;
      customForm.folder_path = res.folder_path;
      if (res.models.length > 0) {
        const first = res.models[0];
        customForm.model_path = first.path;
        if (!customForm.name.trim()) {
          customForm.name = first.suggested_name;
        }
      } else {
        customForm.model_path = '';
        customScanError = '所选文件夹下未检测到 .gguf 模型文件，请重新选择';
      }
      if (res.mmprojs.length > 0) {
        customForm.mmproj_path = res.mmprojs[0].path;
      } else {
        customForm.mmproj_path = '';
      }
    } catch (e) {
      customScanError = `扫描文件夹失败：${e}`;
    } finally {
      scanningCustomFolder = false;
    }
  }

  async function handleSaveCustomModel() {
    if (!customForm.model_path.trim()) {
      customScanError = '请先点击「选择文件夹」识别本地 .gguf 模型文件';
      return;
    }
    savingCustomModel = true;
    customScanError = '';
    try {
      const existingCard = overview?.models.find((x) => x.id === customForm.id);
      const payload: CustomLocalModel = {
        id: customForm.id,
        name: customForm.name.trim(),
        desc: customForm.desc.trim(),
        folder_path: customForm.folder_path.trim(),
        model_path: customForm.model_path.trim(),
        mmproj_path: customForm.mmproj_path.trim(),
        vision: Boolean(customForm.mmproj_path.trim()),
        quant_tag: '',
        size_label: '',
        params: {
          ctx: existingCard?.params?.ctx || 32768,
          ngl: existingCard?.params?.ngl ?? 99,
          kv_quant: existingCard?.params?.kv_quant || 'q8_0',
          reasoning_effort: customForm.reasoning_effort || 'off',
          idle_secs: existingCard?.params?.idle_secs || 180,
          extra_args: existingCard?.params?.extra_args || '',
        },
      };
      overview = await ipc.engineSaveCustomModel(payload);
      s = await ipc.getSettings();
      showCustomModelModal = false;
      flashMsg('✓ 已保存自定义本地模型');
    } catch (e) {
      customScanError = String(e);
    } finally {
      savingCustomModel = false;
    }
  }

  async function handleDeleteCustomModel(m: ModelCard) {
    try {
      overview = await ipc.engineDeleteCustomModel(m.id);
      s = await ipc.getSettings();
      if (expandedAdvModelId === m.id) expandedAdvModelId = null;
      flashMsg(`✓ 已从列表移除「${m.name}」`);
    } catch (e) {
      flashMsg(`移除失败：${e}`);
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
        <button class="nav-btn" class:active={tab === t} onclick={() => (tab = t)}>
          <span>{t}</span>
          {#if t === '版本更新' && updateInfo?.has_update}
            <span class="nav-update-dot" title="发现新版本 v{updateInfo.latest_version}">NEW</span>
          {:else if t === '个人知识库' && layaStatus?.needs_redownload}
            <span class="nav-update-dot" title="旧版 Laya 模型需升级为 GGUF">升级</span>
          {/if}
        </button>
      {/each}
    </div>

    <div class="sidebar-footer">
      <button type="button" class="ver-footer-btn" onclick={() => (tab = '版本更新')} title="点击检测版本更新">
        <span>当前版本 v{info?.version ?? '0.1.0'}</span>
        {#if updateInfo?.has_update}
          <span class="ver-up-pill">↑ v{updateInfo.latest_version}</span>
        {:else}
          <span class="ver-ok-pill">最新</span>
        {/if}
      </button>
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
          <!-- 顶部二级 Tab 切换：极简风格 -->
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
                  <span class="active-dot" title="当前生效"></span>
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
                  <span class="active-dot" title="当前生效"></span>
                {/if}
              </button>
            </div>

            <div class="subtab-hint">
              {#if s.llm.api_enabled}
                <span class="mode-indicator cloud">☁️ API · {s.llm.api_model || '未指定'}</span>
              {:else}
                <span class="mode-indicator local">🖥 本地离线引擎</span>
              {/if}
            </div>
          </div>

          {#if modelSubTab === 'local'}
            <!-- 低配提醒或云端启用状态：单行极简横幅 + 问号悬浮详情 -->
            {#if overview?.hardware.low_end_device && !s.llm.api_enabled}
              <div class="hw-recommend-banner compact">
                <div class="hwr-inline">
                  <span class="hwr-title">💡 检测到本机配置偏低，推荐使用云端 API</span>
                  <span class="help-tip" data-tip={overview.hardware.low_end_reason}>?</span>
                </div>
                <button type="button" class="act-btn light sm" onclick={() => (modelSubTab = 'api')}>
                  开启模型 API →
                </button>
              </div>
            {:else if s.llm.api_enabled}
              <div class="cloud-active-banner compact">
                <div class="hwr-inline">
                  <span class="hwr-title">☁️ 当前生效：模型 API（{s.llm.api_model}）</span>
                  <span
                    class="help-tip"
                    data-tip="本地推理引擎已自动待命（0 显存与 CPU 占用）。点击右侧按钮可随时切回本地离线模型。"
                    >?</span
                  >
                </div>
                <div class="engine-dl-group">
                  <button
                    type="button"
                    class="act-btn sm"
                    onclick={() => {
                      s!.llm.api_enabled = false;
                      save();
                    }}
                  >
                    切回本地
                  </button>
                  <button type="button" class="act-btn light sm" onclick={() => (modelSubTab = 'api')}>
                    API 设置 →
                  </button>
                </div>
              </div>
            {/if}

            <!-- 1. 本地运行时 / 推理引擎（极简双行 + 问号提示） -->
            <section class="rt-section">
              <div class="rt-head">
                <div class="rt-head-left">
                  <span class="rt-icon">⚡</span>
                  <span class="rt-title">本地运行时</span>
                  <span class="ver-pill">{overview?.engine_version ?? '检测中'}</span>
                  {#if overview?.engine_installed}
                    <span class="ver-pill backend">{overview.engine_backend}</span>
                  {/if}
                  <span
                    class="help-tip"
                    data-tip="空闲时 0 显存占用，发起对话或识图时自动秒级加载；手动关闭可立即释放全部显存。"
                    >?</span
                  >
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
                    待命
                  {:else}
                    未安装
                  {/if}
                </span>
              </div>

              <div class="rt-block compact">
                <div class="rt-row">
                  <div class="rt-label-group">
                    <span class="rt-name">推理服务</span>
                    <span class="rt-status-sub">
                      {status?.status === 'ready'
                        ? '运行中'
                        : overview?.engine_installed
                          ? '空闲省电（按需秒级唤醒）'
                          : '未安装引擎'}
                    </span>
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
                  <div class="rt-label-group">
                    <span class="rt-name">硬件加速</span>
                    <span class="rt-status-sub">
                      {overview?.hardware.recommended_backend_label ?? '自动匹配最佳后端'}
                    </span>
                    <span
                      class="help-tip"
                      data-tip="已根据本机显卡自动匹配最佳构建版本，全面兼容 NVIDIA CUDA、AMD/Intel Vulkan、macOS Metal 与 CPU AVX2。"
                      >?</span
                    >
                  </div>
                  <div class="engine-dl-group">
                    <select class="backend-sel" bind:value={engineBackendChoice} disabled={downloading}>
                      <option value="auto">自动 ({overview?.hardware.recommended_backend.toUpperCase() ?? 'AUTO'})</option>
                      <option value="cuda">NVIDIA (CUDA)</option>
                      <option value="vulkan">AMD / Intel (Vulkan)</option>
                      <option value="cpu">CPU (AVX2)</option>
                      <option value="metal">macOS (Metal)</option>
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

                <div class="rt-divider"></div>

                <div class="rt-row">
                  <div class="rt-label-group">
                    <span class="rt-name">本地推理服务端口</span>
                    <span class="rt-status-sub">
                      llama-server 本地监听端口（默认 18080）
                    </span>
                    <span
                      class="help-tip"
                      data-tip="内置推理引擎服务端口。若提示端口被占用，可在此修改为其他空闲端口（如 18088、28080）或点击检测自动分配。"
                      >?</span
                    >
                  </div>
                  <div class="port-cfg-group">
                    <input
                      type="number"
                      class="port-input"
                      min="1024"
                      max="65535"
                      bind:value={s.llm.port}
                      onchange={save}
                    />
                    <button
                      type="button"
                      class="act-btn light"
                      onclick={handleCheckPort}
                      disabled={checkingPort}
                    >
                      {checkingPort ? '检测中…' : '检测可用性'}
                    </button>
                    {#if portTip}
                      <span class="port-status-pill" class:ok={portTipOk} class:warn={!portTipOk}>
                        {portTip}
                      </span>
                    {/if}
                  </div>
                </div>
              </div>
            </section>

            <!-- 2. 本机配置 -->
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
                  <span class="hw-val">{overview?.hardware.gpu_name || '正在检测…'}</span>
                </div>
                <div class="hw-item">
                  <span class="hw-ic">❖</span>
                  <span class="hw-val">
                    {overview?.hardware.vram_gb ? `${overview.hardware.vram_gb.toFixed(1)} GB 显存` : '共享内存'}
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

            <!-- 3. 本地模型库 -->
            <section class="rt-section">
              <div class="rt-head">
                <div class="rt-head-left">
                  <span class="rt-icon">⬇</span>
                  <span class="rt-title">本地模型库</span>
                  <span class="ver-pill">{overview?.models.length ?? 2}</span>
                  <span
                    class="help-tip"
                    data-tip="默认 LFM3-VL 2B 模型固定存放在 llama/models 下不可修改路径；点击右侧「＋ 新增模型」可选择本地文件夹添加任意自定义 GGUF 模型。"
                    >?</span
                  >
                </div>

                <div class="model-head-tools">
                  <button
                    type="button"
                    class="act-btn light"
                    onclick={() => (showCustomModelModal ? (showCustomModelModal = false) : openAddCustomModel())}
                    disabled={downloading}
                  >
                    {showCustomModelModal ? '✕ 收起' : '＋ 新增模型'}
                  </button>
                </div>
              </div>

              <!-- 新增 / 编辑用户自定义本地模型面板 -->
              {#if showCustomModelModal}
                <div class="custom-model-panel">
                  <div class="cmp-head">
                    <div class="cmp-title">
                      <span>{customForm.id ? '编辑自定义模型' : '新增本地模型'}</span>
                      <span
                        class="help-tip"
                        data-tip="选择包含 .gguf 的文件夹，自动识别主模型与 mmproj 视觉组件；若无视觉组件，识图时将自动调用内置离线 OCR。"
                        >?</span
                      >
                    </div>
                    <button
                      type="button"
                      class="act-btn"
                      onclick={handleScanCustomFolder}
                      disabled={scanningCustomFolder}
                    >
                      {scanningCustomFolder ? '扫描中…' : '📁 选择文件夹'}
                    </button>
                  </div>

                  {#if customScanError}
                    <div class="cmp-err">⚠ {customScanError}</div>
                  {/if}

                  {#if customForm.folder_path || customForm.model_path}
                    <div class="cmp-detected">
                      <div class="cmp-grid">
                        <label class="cmp-field">
                          <span>主模型 (.gguf)</span>
                          {#if scannedFolder && scannedFolder.models.length > 1}
                            <select
                              value={customForm.model_path}
                              onchange={(e) => {
                                const p = (e.currentTarget as HTMLSelectElement).value;
                                customForm.model_path = p;
                                const found = scannedFolder?.models.find((x) => x.path === p);
                                if (found) customForm.name = found.suggested_name;
                              }}
                            >
                              {#each scannedFolder.models as mf (mf.path)}
                                <option value={mf.path}>{mf.file_name} ({mf.size_label})</option>
                              {/each}
                            </select>
                          {:else}
                            <input
                              readonly
                              title={customForm.model_path}
                              value={scannedFolder?.models[0]
                                ? `${scannedFolder.models[0].file_name} (${scannedFolder.models[0].size_label})`
                                : customForm.model_path.split(/[\\/]/).pop() || ''}
                            />
                          {/if}
                        </label>

                        <label class="cmp-field">
                          <div class="cmp-field-head">
                            <span>视觉组件 (mmproj)</span>
                            {#if customForm.mmproj_path}
                              <span class="cmp-vis-badge ok">👁 视觉</span>
                            {:else}
                              <span class="cmp-vis-badge">纯文本</span>
                            {/if}
                          </div>
                          {#if scannedFolder && scannedFolder.mmprojs.length > 0}
                            <select bind:value={customForm.mmproj_path}>
                              {#each scannedFolder.mmprojs as vp (vp.path)}
                                <option value={vp.path}>{vp.file_name} ({vp.size_label})</option>
                              {/each}
                              <option value="">不使用视觉组件</option>
                            </select>
                          {:else}
                            <input
                              readonly
                              value={customForm.mmproj_path
                                ? customForm.mmproj_path.split(/[\\/]/).pop()
                                : '无（自动使用内置离线 OCR）'}
                            />
                          {/if}
                        </label>
                      </div>

                      <div class="cmp-grid three">
                        <label class="cmp-field">
                          <span>模型名称</span>
                          <input
                            bind:value={customForm.name}
                            placeholder="如 Qwen2.5-VL-7B"
                          />
                        </label>
                        <label class="cmp-field">
                          <span>模型描述</span>
                          <input
                            bind:value={customForm.desc}
                            placeholder="如：适合 8GB 显存的视觉推理模型"
                          />
                        </label>
                        <label class="cmp-field">
                          <span class="mc-lbl-row">
                            默认思考
                            <span
                              class="help-tip tip-left"
                              data-tip="设置该模型的默认思考等级（截图、划词、AI 聊天默认同步此设置）。不支持高思考的自定义模型建议设为关闭或低/中。"
                              >?</span
                            >
                          </span>
                          <select bind:value={customForm.reasoning_effort}>
                            <option value="off">关闭思考 (默认)</option>
                            <option value="low">低强度 (512)</option>
                            <option value="medium">中强度 (2K)</option>
                            <option value="high">高强度 (8K)</option>
                            <option value="max">满血无限制 (Max)</option>
                          </select>
                        </label>
                      </div>

                      <div class="cmp-actions">
                        <button
                          type="button"
                          class="ghost sm"
                          onclick={() => (showCustomModelModal = false)}
                        >
                          取消
                        </button>
                        <button
                          type="button"
                          class="primary save-btn"
                          onclick={handleSaveCustomModel}
                          disabled={savingCustomModel || !customForm.model_path}
                        >
                          {savingCustomModel ? '保存中…' : '✓ 保存'}
                        </button>
                      </div>
                    </div>
                  {:else}
                    <div class="cmp-empty" onclick={handleScanCustomFolder} role="button" tabindex="0" onkeydown={(e) => e.key === 'Enter' && handleScanCustomFolder()}>
                      <span>📂 点击选择本地模型文件夹（自动识别模型与视觉）</span>
                    </div>
                  {/if}
                </div>
              {/if}

              <div class="model-list">
                {#if overview}
                  {#each overview.models as m (m.id)}
                    <div class="model-card-wrap" class:active-card={m.active}>
                      <div class="model-card">
                        <div class="mc-main">
                          <div class="mc-title-row">
                            <span class="mc-name">{m.name}</span>
                            {#if m.is_custom}
                              <span class="mc-custom-badge">自定义</span>
                            {/if}
                            {#if m.recommended}
                              <span class="mc-rec-badge" title={m.recommend_reason}>★ 推荐</span>
                            {/if}
                            <span class="help-tip" data-tip={buildModelTip(m)}>?</span>
                          </div>

                          <div class="mc-meta-row">
                            <span class="mc-desc">{m.desc}</span>
                            <div class="mc-tags">
                              <span class="tag">{m.size_label}</span>
                              <span class="tag">{m.quant_tag}</span>
                              <span class="tag">{formatCtxLabel(m.params?.ctx)}</span>
                              <span class="tag">思考:{formatReasoningShort(m.params?.reasoning_effort)}</span>
                              <span class="tag">{m.vision ? '视觉' : '文本'}</span>
                            </div>
                          </div>
                        </div>

                        <div class="mc-right-col">
                          <div class="mc-action">
                            {#if m.installed && m.active}
                              <span class="in-use-pill">✓ 使用中</span>
                            {:else if m.installed}
                              <button
                                class="act-btn light"
                                onclick={() => handleActivateModel(m.id)}
                                disabled={downloading}
                              >
                                启用
                              </button>
                            {:else}
                              <button
                                class="act-btn dl-highlight"
                                onclick={() => {
                                  if (dlRouteOpenId === m.id) {
                                    dlRouteOpenId = null;
                                  } else {
                                    dlRouteChoice = 'modelscope';
                                    dlRouteOpenId = m.id;
                                  }
                                }}
                                disabled={downloading}
                              >
                                ⬇ 下载 · {m.size_label}
                              </button>
                            {/if}

                            {#if m.is_custom}
                              <button
                                type="button"
                                class="mc-icon-btn"
                                title="编辑名称与描述"
                                onclick={() => openEditCustomModel(m)}
                              >
                                编辑
                              </button>
                              <button
                                type="button"
                                class="mc-icon-btn danger"
                                title="从列表移除（不删源文件）"
                                onclick={() => handleDeleteCustomModel(m)}
                              >
                                移除
                              </button>
                            {/if}
                          </div>

                          {#if m.installed}
                            <button
                              type="button"
                              class="mc-adv-btn"
                              class:open={expandedAdvModelId === m.id}
                              onclick={() =>
                                (expandedAdvModelId = expandedAdvModelId === m.id ? null : m.id)}
                            >
                              <span>⚙ 高级设置</span>
                              <span class="mc-adv-arrow">{expandedAdvModelId === m.id ? '▴' : '▾'}</span>
                            </button>
                          {/if}
                        </div>
                      </div>

                      <!-- 点击「下载」按钮后展开的线路选择栏（默认 ModelScope） -->
                      {#if !m.installed && dlRouteOpenId === m.id}
                        <div class="mc-route-bar">
                          <span class="mc-route-label">下载线路</span>
                          <div class="mc-route-opts">
                            <button
                              type="button"
                              class="route-chip"
                              class:active={dlRouteChoice === 'modelscope'}
                              onclick={() => (dlRouteChoice = 'modelscope')}
                            >
                              ModelScope (默认)
                            </button>
                            <button
                              type="button"
                              class="route-chip"
                              class:active={dlRouteChoice === 'hf-mirror'}
                              onclick={() => (dlRouteChoice = 'hf-mirror')}
                            >
                              hf-mirror
                            </button>
                            <button
                              type="button"
                              class="route-chip"
                              class:active={dlRouteChoice === 'huggingface'}
                              onclick={() => (dlRouteChoice = 'huggingface')}
                            >
                              HuggingFace
                            </button>
                          </div>
                          <div class="mc-route-go">
                            <button
                              type="button"
                              class="primary save-btn"
                              onclick={() => handleDownloadModel(m.id, dlRouteChoice)}
                              disabled={downloading}
                            >
                              开始下载
                            </button>
                            <button
                              type="button"
                              class="ghost sm"
                              onclick={() => (dlRouteOpenId = null)}
                            >
                              取消
                            </button>
                          </div>
                        </div>
                      {/if}

                      <!-- 点击卡片右下角「高级设置」后，卡片内紧凑展开当前模型专属参数 -->
                      {#if m.installed && expandedAdvModelId === m.id}
                        <div class="mc-adv-drawer">
                          <div class="mc-adv-grid">
                            <label class="mc-param-field">
                              <span class="mc-lbl-row">
                                默认思考
                                <span
                                  class="help-tip"
                                  data-tip="当前模型的默认思考等级（截图、划词、AI 聊天默认同步此设置，也可在对话界面随时切换）。不支持高思考的自定义模型建议设为关闭或低/中。"
                                  >?</span
                                >
                              </span>
                              <select
                                value={m.params.reasoning_effort || 'off'}
                                onchange={(e) => {
                                  m.params.reasoning_effort = (e.currentTarget as HTMLSelectElement).value as ModelParams['reasoning_effort'];
                                  handleSaveModelParams(m);
                                }}
                              >
                                <option value="off">关闭 (关)</option>
                                <option value="low">低 (512)</option>
                                <option value="medium">中 (2K)</option>
                                <option value="high">高 (8K)</option>
                                <option value="max">满 (Max)</option>
                              </select>
                            </label>

                            <label class="mc-param-field">
                              <span class="mc-lbl-row">
                                上下文
                                <span
                                  class="help-tip"
                                  data-tip="单次对话最大上下文窗口长度（ctx），默认 32K，最高支持 256K。"
                                  >?</span
                                >
                              </span>
                              <select
                                value={String(m.params.ctx || 32768)}
                                onchange={(e) => {
                                  m.params.ctx = Number((e.currentTarget as HTMLSelectElement).value) || 32768;
                                  handleSaveModelParams(m);
                                }}
                              >
                                <option value="4096">4K</option>
                                <option value="8192">8K</option>
                                <option value="16384">16K</option>
                                <option value="32768">32K (默认)</option>
                                <option value="65536">64K</option>
                                <option value="100128">100K</option>
                                <option value="131072">128K</option>
                                <option value="262144">256K</option>
                              </select>
                            </label>

                            <label class="mc-param-field">
                              <span class="mc-lbl-row">
                                GPU 层数
                                <span
                                  class="help-tip"
                                  data-tip="加载至显卡的层数 (-ngl)。99 代表全部交由 GPU 加速，0 代表纯 CPU 运行。"
                                  >?</span
                                >
                              </span>
                              <select
                                value={String(m.params.ngl ?? 99)}
                                onchange={(e) => {
                                  m.params.ngl = Number((e.currentTarget as HTMLSelectElement).value);
                                  handleSaveModelParams(m);
                                }}
                              >
                                <option value="99">全部 (99)</option>
                                <option value="48">48 层</option>
                                <option value="32">32 层</option>
                                <option value="20">20 层</option>
                                <option value="12">12 层</option>
                                <option value="0">纯 CPU (0)</option>
                              </select>
                            </label>

                            <label class="mc-param-field">
                              <span class="mc-lbl-row">
                                KV 量化
                                <span
                                  class="help-tip"
                                  data-tip="KV Cache 显存量化精度。Q8_0 可大幅降低长上下文显存占用且几乎无损。"
                                  >?</span
                                >
                              </span>
                              <select
                                value={m.params.kv_quant || 'q8_0'}
                                onchange={(e) => {
                                  m.params.kv_quant = (e.currentTarget as HTMLSelectElement).value;
                                  handleSaveModelParams(m);
                                }}
                              >
                                <option value="q8_0">Q8_0 (默认)</option>
                                <option value="f16">F16</option>
                                <option value="q4_0">Q4_0</option>
                              </select>
                            </label>

                            <label class="mc-param-field">
                              <span class="mc-lbl-row">
                                闲置释放 (秒)
                                <span
                                  class="help-tip tip-left"
                                  data-tip="停止对话超过设定秒数后，自动关闭推理进程并释放全部显存。"
                                  >?</span
                                >
                              </span>
                              <input
                                type="number"
                                min="30"
                                step="30"
                                bind:value={m.params.idle_secs}
                                onchange={() => handleSaveModelParams(m)}
                              />
                            </label>
                          </div>

                          <div class="mc-adv-row2">
                            <label class="mc-param-field grow">
                              <span class="mc-lbl-row">
                                启动参数
                                <span
                                  class="help-tip"
                                  data-tip="附加给 llama-server 的自定义命令行参数（如 --threads 8 --flash-attn），仅对当前模型生效。"
                                  >?</span
                                >
                              </span>
                              <input
                                bind:value={m.params.extra_args}
                                onchange={() => handleSaveModelParams(m)}
                                placeholder="可选，如 --threads 8"
                              />
                            </label>
                            <label class="mc-param-field port-col">
                              <span>端口</span>
                              <input type="number" bind:value={s.llm.port} onchange={save} />
                            </label>
                          </div>
                        </div>
                      {/if}
                    </div>
                  {/each}
                {/if}
              </div>
            </section>

          {:else}
            <!-- 子 Tab 2：模型 API（极简配置） -->
            <section class="rt-section">
              <div class="card">
                <div class="row-item">
                  <div class="rt-label-group">
                    <span class="row-label">启用云端 API 模型</span>
                    <span
                      class="help-tip"
                      data-tip="开启后所有对话与识图均通过云端 API 响应，不占用本机显卡与内存。若所选模型不支持视觉，识图时将自动调用本地离线 OCR。"
                      >?</span
                    >
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
                  <span class="mc-lbl-row">
                    服务商
                    {#if currentProviderPreset.hint}
                      <span class="help-tip" data-tip={currentProviderPreset.hint}>?</span>
                    {/if}
                  </span>
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

                <!-- 模型下拉选择 -->
                <div class="field">
                  <div class="field-head">
                    <span class="mc-lbl-row">
                      模型
                      {#if fetchingModels}
                        <small class="dim">（获取中…）</small>
                      {:else if fetchedModels.length > 0}
                        <small class="dim">（{fetchedModels.length}）</small>
                      {/if}
                    </span>
                    <label class="vision-inline-chk">
                      <input
                        type="checkbox"
                        bind:checked={s.llm.api_vision}
                        onchange={save}
                      />
                      <span>视觉识图</span>
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
                        <option value="__custom__">手动输入…</option>
                      </select>
                    {:else}
                      <input
                        class="api-model-sel"
                        bind:value={s.llm.api_model}
                        onchange={() => {
                          s!.llm.api_vision = inferVisionFromModelId(s!.llm.api_model);
                          save();
                        }}
                        placeholder="输入模型 ID"
                      />
                      {#if availableApiModels.length > 0}
                        <button
                          type="button"
                          class="act-btn"
                          onclick={() => (customModelInputMode = false)}
                        >
                          下拉选择
                        </button>
                      {/if}
                    {/if}

                    <button
                      type="button"
                      class="act-btn"
                      onclick={() => handleFetchCloudModels(false)}
                      disabled={fetchingModels}
                      title="刷新模型列表"
                    >
                      {fetchingModels ? '…' : '🔄 刷新'}
                    </button>
                  </div>
                </div>
              </div>
            </section>
          {/if}

        {:else if tab === '个人知识库'}
          <section class="group">
            <div class="group-title">存储目录</div>
            <div class="card">
              <div class="row-item-vertical">
                <div class="row-meta">
                  <div class="row-label">知识库根目录</div>
                  <div class="row-desc">划词笔记、图片资产与网页快照均保存在此目录，可直接切换为已有文档文件夹</div>
                </div>
                <div class="path-input-group">
                  <input
                    type="text"
                    class="txt path-input"
                    bind:value={s.knowledge.base_path}
                    placeholder="留空使用默认路径"
                  />
                  <button
                    type="button"
                    class="btn btn-action"
                    onclick={handlePickKbFolder}
                    disabled={kbSyncing}
                  >
                    {kbSyncing ? '⏳ 扫描中…' : '📁 更改'}
                  </button>
                  <button
                    type="button"
                    class="btn btn-action"
                    onclick={handleSyncKbIndex}
                    disabled={kbSyncing}
                    title="重新扫描文件夹并刷新检索索引"
                  >
                    {kbSyncing ? '⏳' : '🔄 重建索引'}
                  </button>
                  <button
                    type="button"
                    class="btn btn-action"
                    onclick={() => ipc.kbOpenFolder()}
                  >
                    📂 打开
                  </button>
                </div>
              </div>

              {#if kbStats}
                <div class="kb-stats-strip">
                  <span>📄 文档 <b>{kbStats.notes_count}</b></span>
                  <span class="kb-s-sep">·</span>
                  <span class="kb-s-hi">🧩 切片 <b>{kbStats.chunks_count}</b></span>
                  <span class="kb-s-sep">·</span>
                  <span>🖼️ 图片 <b>{kbStats.images_count}</b></span>
                  <span class="kb-s-sep">·</span>
                  <span>🌐 网页 <b>{kbStats.web_count}</b></span>
                  <span class="kb-s-sep">·</span>
                  <span>💾 <b>{(kbStats.total_size_bytes / 1024 / 1024).toFixed(1)} MB</b></span>
                </div>
              {/if}
            </div>
          </section>

          <section class="group">
            <div class="group-title">常用策略</div>
            <div class="card">
              <div class="row-item">
                <div class="row-meta">
                  <div class="row-label">聊天默认启用知识库</div>
                  <div class="row-desc">打开 AI 聊天窗时默认点亮知识库问答</div>
                </div>
                <label class="switch">
                  <input type="checkbox" bind:checked={s.knowledge.auto_search} />
                  <span class="slider"></span>
                </label>
              </div>

              <div class="row-item">
                <div class="row-meta">
                  <div class="row-label">保存片段时自动 AI 提纯</div>
                  <div class="row-desc">划词或收藏网页存入知识库时，自动提炼要点并保留原文</div>
                </div>
                <label class="switch">
                  <input type="checkbox" bind:checked={s.knowledge.clip_auto_distill} />
                  <span class="slider"></span>
                </label>
              </div>

              <div class="row-item">
                <div class="row-meta">
                  <div class="row-label">检索参考片段数</div>
                  <div class="row-desc">单次问答召回的候选片段上限（推荐 10 ~ 30）</div>
                </div>
                <input
                  type="number"
                  class="txt num-input"
                  min="5"
                  max="50"
                  bind:value={s.knowledge.candidate_count}
                />
              </div>

              <div class="row-item">
                <div class="row-meta">
                  <div class="row-label">
                    Laya 本地检索加速 (GGUF)
                    <span
                      class="laya-pill"
                      class:ok={layaStatus?.installed}
                      style={layaStatus?.needs_redownload ? 'background: rgba(245,158,11,0.16); color: #f59e0b; border-color: rgba(245,158,11,0.35);' : ''}
                    >
                      {#if layaStatus?.installed}
                        已就绪 {layaStatus.size_mb}MB
                      {:else if layaStatus?.needs_redownload}
                        ⚠ 需重下 GGUF · 345MB
                      {:else}
                        可选 345MB
                      {/if}
                    </span>
                  </div>
                  <div class="row-desc">
                    {#if layaStatus?.needs_redownload}
                      检测到旧版非 GGUF 模型已自动清理，需从 mys/laya-multilingual-GGUF 重新下载 laya_multilingual_q8_0.gguf
                    {:else}
                      基于 laya_multilingual_q8_0.gguf 的毫秒级多语言神经语义重排；未安装时使用关键词混合检索
                    {/if}
                  </div>
                </div>
                <div class="laya-compact-ctrl">
                  {#if layaDownloading}
                    <span class="laya-dl-mini">{layaDlProgress?.percent || 0}%</span>
                  {:else if layaStatus?.installed}
                    <label class="switch">
                      <input type="checkbox" bind:checked={s.knowledge.laya_enabled} />
                      <span class="slider"></span>
                    </label>
                    <button type="button" class="btn" onclick={handleDeleteLaya} title="删除加速模型释放空间">
                      🗑️
                    </button>
                  {:else}
                    <button type="button" class="btn" onclick={handleDownloadLaya}>
                      {layaStatus?.needs_redownload ? '🔄 重新下载 GGUF' : '⚡ 下载加速'}
                    </button>
                  {/if}
                </div>
              </div>
              {#if layaDownloading}
                <div class="laya-dl-box mini">
                  <div class="progress-bar">
                    <div class="progress-fill" style="width: {layaDlProgress?.percent || 0}%"></div>
                  </div>
                  <div class="laya-dl-foot">
                    <span>{layaDlProgress?.message || '正在下载…'} ({layaDlProgress?.downloaded_mb || 0}/{layaDlProgress?.total_mb || 345} MB)</span>
                    {#if layaDlError}<span class="dl-err">{layaDlError}</span>{/if}
                  </div>
                </div>
              {:else if layaDlError}
                <div class="laya-dl-foot"><span class="dl-err">{layaDlError}</span></div>
              {/if}
            </div>
          </section>

          <section class="group">
            <div class="group-title">扩展与文档工具</div>
            <div class="card">
              <KbPromptsPanel
                basePath={s.knowledge.base_path}
                onNotify={(msg, ms) => flashMsg(msg, ms)}
              />
              <div class="row-item">
                <div class="row-meta">
                  <div class="row-label">文件右键「使用 miniBot 编辑」</div>
                  <div class="row-desc">在系统文件右键菜单中添加 Markdown 快速编辑入口</div>
                </div>
                <label class="switch">
                  <input
                    type="checkbox"
                    checked={mdContextMenuRegistered}
                    onchange={handleToggleMdContextMenu}
                  />
                  <span class="slider"></span>
                </label>
              </div>
              <div class="row-item">
                <div class="row-meta">
                  <div class="row-label">独立 Markdown 编辑器</div>
                  <div class="row-desc">轻量分屏预览编辑器，支持直接查看与修改笔记</div>
                </div>
                <button
                  type="button"
                  class="btn"
                  onclick={() => ipc.docOpenEditor()}
                >
                  📝 打开编辑器
                </button>
              </div>
            </div>
          </section>

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

        {:else if tab === '版本更新'}
          <section class="group">
            <div class="card">
              <div class="upd-hero">
                <div class="upd-app-meta">
                  <img src="/logo.png" alt="miniBot" class="upd-logo" />
                  <div class="upd-titles">
                    <div class="upd-name-row">
                      <span class="upd-app-name">miniBot</span>
                      <span class="ver-pill">v{info?.version ?? '0.1.0'}</span>
                      {#if updateInfo?.has_update}
                        <span class="upd-status-badge new">发现新版本 v{updateInfo.latest_version}</span>
                      {:else if updateInfo}
                        <span class="upd-status-badge ok">已是最新版本</span>
                      {/if}
                    </div>
                  </div>
                </div>

                <div class="upd-hero-actions">
                  {#if updateInfo?.has_update && updateInfo.download_url}
                    <button
                      type="button"
                      class="primary"
                      onclick={handleDownloadAndInstallUpdate}
                      disabled={downloadingUpdate}
                    >
                      {downloadingUpdate
                        ? `正在下载 (${updateDlProgress?.percent ?? 0}%)…`
                        : `立即更新 v${updateInfo.latest_version}`}
                    </button>
                  {/if}
                  <button
                    type="button"
                    class="act-btn"
                    onclick={() => handleCheckUpdate(true)}
                    disabled={checkingUpdate || downloadingUpdate}
                  >
                    {checkingUpdate ? '检测中…' : '检测更新'}
                  </button>
                </div>
              </div>

              {#if updateCheckError}
                <div class="dl-err-banner" style="margin-top: 12px;">
                  <span>⚠ {updateCheckError}</span>
                  <button class="ghost sm" onclick={() => (updateCheckError = '')}>关闭</button>
                </div>
              {/if}

              {#if downloadingUpdate && updateDlProgress}
                <div class="dl-banner" style="margin-top: 12px;">
                  <div class="dl-top">
                    <span class="dl-msg">{updateDlProgress.message}</span>
                    <span class="dl-pct">{updateDlProgress.percent}%</span>
                    <button
                      type="button"
                      class="ghost sm cancel-dl"
                      onclick={() => ipc.appCancelUpdateDownload()}
                    >
                      取消
                    </button>
                  </div>
                  <div class="dl-track">
                    <div class="dl-fill" style="width: {updateDlProgress.percent}%"></div>
                  </div>
                </div>
              {/if}

              {#if downloadedInstallerPath && !downloadingUpdate}
                <div class="upd-ready-banner">
                  <div class="upd-ready-info">
                    <div class="upd-ready-title">✅ 安装包已就绪</div>
                    <div class="upd-ready-path mono selectable">{downloadedInstallerPath}</div>
                  </div>
                  <button
                    type="button"
                    class="act-btn light"
                    onclick={() => handleRunInstaller()}
                    disabled={installingUpdate}
                  >
                    {installingUpdate ? '正在启动安装…' : '立即安装'}
                  </button>
                </div>
              {/if}

              <div class="divider"></div>

              <div class="row-item">
                <div class="row-meta">
                  <div class="row-label">GitHub 地址</div>
                </div>
                <button
                  type="button"
                  class="link-inline"
                  onclick={() => ipc.openExternalUrl('https://github.com/xiao-ge008/miniBot')}
                >
                  https://github.com/xiao-ge008/miniBot
                </button>
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
    display: flex;
    align-items: center;
    justify-content: space-between;
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
  .nav-update-dot {
    font-size: 10px;
    font-weight: 700;
    padding: 1px 6px;
    border-radius: 999px;
    background: #10b981;
    color: #fff;
    letter-spacing: 0.02em;
  }
  .sidebar-footer {
    margin-top: auto;
    padding: 10px;
    border-top: 1px solid var(--border);
  }
  .ver-footer-btn {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 10px;
    border-radius: 7px;
    border: 1px solid var(--border);
    background: var(--bg-solid);
    color: var(--fg-dim);
    font-size: 11.5px;
    cursor: pointer;
  }
  .ver-footer-btn:hover {
    color: var(--fg);
    border-color: var(--accent);
  }
  .ver-up-pill {
    font-size: 10.5px;
    font-weight: 600;
    color: #10b981;
  }
  .ver-ok-pill {
    font-size: 10.5px;
    color: var(--fg-dim);
  }
  .upd-hero {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 14px;
    flex-wrap: wrap;
  }
  .upd-app-meta {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .upd-logo {
    width: 40px;
    height: 40px;
    border-radius: 10px;
    object-fit: contain;
    background: var(--panel);
    padding: 4px;
    border: 1px solid var(--border);
  }
  .upd-titles {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .upd-name-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .upd-app-name {
    font-size: 15px;
    font-weight: 650;
    color: var(--fg);
  }
  .upd-status-badge {
    font-size: 11.5px;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: 999px;
  }
  .upd-status-badge.new {
    background: rgba(16, 185, 129, 0.14);
    color: #10b981;
    border: 1px solid rgba(16, 185, 129, 0.35);
  }
  .upd-status-badge.ok {
    background: var(--panel);
    color: var(--fg-dim);
    border: 1px solid var(--border);
  }
  .link-inline {
    background: none;
    border: none;
    padding: 0;
    color: var(--accent);
    font-size: 12.5px;
    cursor: pointer;
    text-decoration: underline;
  }
  .upd-hero-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .upd-ready-banner {
    margin-top: 12px;
    padding: 10px 14px;
    border-radius: 9px;
    background: rgba(16, 185, 129, 0.08);
    border: 1px solid rgba(16, 185, 129, 0.3);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  .upd-ready-info {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  .upd-ready-title {
    font-size: 13px;
    font-weight: 600;
    color: #10b981;
  }
  .upd-ready-path {
    font-size: 11.5px;
    color: var(--fg-dim);
    word-break: break-all;
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

  /* 问号悬停气泡提示（遵循全局主题样式） */
  .help-tip {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 15px;
    height: 15px;
    border-radius: 50%;
    border: 1px solid var(--border);
    background: var(--bg-solid);
    color: var(--fg-dim);
    font-size: 10.5px;
    font-weight: 600;
    line-height: 1;
    cursor: help;
    user-select: none;
    flex-shrink: 0;
    transition: color 0.15s, border-color 0.15s, background 0.15s;
  }
  .help-tip:hover {
    color: var(--fg);
    border-color: var(--fg-dim);
    background: var(--hover);
  }
  .help-tip::after {
    content: attr(data-tip);
    position: absolute;
    bottom: calc(100% + 7px);
    left: 0;
    width: max-content;
    max-width: 280px;
    padding: 7px 10px;
    border-radius: 8px;
    background: var(--panel);
    border: 1px solid var(--border);
    color: var(--fg);
    font-size: 11.5px;
    font-weight: 400;
    line-height: 1.5;
    white-space: pre-line;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.16);
    pointer-events: none;
    opacity: 0;
    transform: translateY(3px);
    transition: opacity 0.14s ease, transform 0.14s ease;
    z-index: 90;
  }
  .help-tip.tip-left::after {
    left: auto;
    right: 0;
  }
  .help-tip:hover::after {
    opacity: 1;
    transform: translateY(0);
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
  .rt-block.compact {
    padding: 11px 16px;
    gap: 10px;
  }
  .rt-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }
  .rt-label-group {
    display: flex;
    align-items: center;
    gap: 7px;
    flex-wrap: wrap;
    min-width: 0;
  }
  .rt-status-sub {
    font-size: 12px;
    color: var(--fg-dim);
    margin-left: 4px;
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

  /* 模型列表头部工具与新增自定义模型面板 */
  .model-head-tools {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .custom-model-panel {
    background: var(--panel);
    border: 1px solid color-mix(in srgb, var(--accent, #3b82f6) 40%, var(--border));
    border-radius: 11px;
    padding: 12px 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .cmp-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  .cmp-title {
    font-size: 13.5px;
    font-weight: 600;
    color: var(--fg);
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .cmp-sub {
    font-size: 11.5px;
    font-weight: 400;
    color: var(--fg-dim);
  }
  .cmp-err {
    font-size: 12px;
    padding: 6px 10px;
    border-radius: 7px;
    background: rgba(239, 68, 68, 0.08);
    border: 1px solid rgba(239, 68, 68, 0.3);
    color: var(--danger);
  }
  .cmp-empty {
    padding: 16px;
    border-radius: 9px;
    border: 1px dashed var(--border);
    background: var(--bg-solid);
    text-align: center;
    font-size: 12.5px;
    color: var(--fg-dim);
    cursor: pointer;
    transition: border-color 0.15s, color 0.15s;
  }
  .cmp-empty:hover {
    border-color: var(--accent, #3b82f6);
    color: var(--fg);
  }
  .cmp-detected {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .cmp-det-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 10px;
    border-radius: 7px;
    background: var(--bg-solid);
    border: 1px solid var(--border);
    font-size: 11.5px;
  }
  .cmp-det-lbl {
    color: var(--fg-dim);
    flex-shrink: 0;
  }
  .cmp-det-val {
    color: var(--fg);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cmp-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px 12px;
  }
  .cmp-grid.three {
    grid-template-columns: 1.1fr 1.3fr 0.9fr;
  }
  .cmp-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: var(--fg-dim);
  }
  .cmp-field-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
  }
  .cmp-vis-badge {
    font-size: 10.5px;
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--bg-solid);
    border: 1px solid var(--border);
    color: var(--fg-dim);
  }
  .cmp-vis-badge.ok {
    background: color-mix(in srgb, var(--ok) 14%, transparent);
    border-color: color-mix(in srgb, var(--ok) 35%, transparent);
    color: var(--ok);
    font-weight: 550;
  }
  .cmp-field input,
  .cmp-field select {
    color: var(--fg);
    background: var(--bg-solid);
    font-size: 12px;
    padding: 5px 9px;
  }
  .cmp-actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    padding-top: 2px;
  }

  /* 模型卡片与卡片内紧凑展开的高级参数 */
  .model-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .model-card-wrap {
    border-radius: 11px;
    background: var(--panel);
    border: 1px solid var(--border);
    transition: border-color 0.15s;
    overflow: visible;
  }
  .model-card-wrap.active-card {
    border-color: color-mix(in srgb, var(--ok) 45%, var(--border));
  }
  .model-card {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 14px;
    padding: 11px 16px;
  }
  .mc-main {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
    flex: 1;
  }
  .mc-title-row {
    display: flex;
    align-items: center;
    gap: 7px;
    flex-wrap: wrap;
  }
  .mc-meta-row {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .mc-name {
    font-size: 14px;
    font-weight: 650;
    color: var(--fg);
  }
  .mc-default-badge {
    font-size: 10.5px;
    padding: 1px 6px;
    border-radius: 5px;
    background: var(--bg-solid);
    border: 1px solid var(--border);
    color: var(--fg-dim);
  }
  .mc-custom-badge {
    font-size: 10.5px;
    padding: 1px 6px;
    border-radius: 5px;
    background: color-mix(in srgb, var(--accent, #3b82f6) 14%, transparent);
    border: 1px solid color-mix(in srgb, var(--accent, #3b82f6) 35%, transparent);
    color: var(--accent, #3b82f6);
    font-weight: 550;
  }
  .mc-rec-badge {
    font-size: 10.5px;
    padding: 1px 7px;
    border-radius: 5px;
    background: color-mix(in srgb, var(--warn) 16%, transparent);
    color: var(--warn);
    font-weight: 550;
  }
  .mc-desc {
    font-size: 11.5px;
    color: var(--fg-dim);
    max-width: 340px;
  }
  .mc-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }
  .tag {
    font-size: 11px;
    padding: 2px 7px;
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

  .mc-right-col {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    justify-content: space-between;
    gap: 6px;
    flex-shrink: 0;
  }
  .mc-action {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .in-use-pill {
    font-size: 12px;
    padding: 4px 11px;
    border-radius: 7px;
    background: color-mix(in srgb, var(--ok) 14%, transparent);
    color: var(--ok);
    font-weight: 600;
  }
  /* 未检测到模型时，下载按钮高亮亮起 */
  .act-btn.dl-highlight {
    background: #10b981;
    color: #fff;
    border-color: #059669;
    font-weight: 600;
    box-shadow: 0 2px 8px rgba(16, 185, 129, 0.25);
  }
  .act-btn.dl-highlight:hover:not(:disabled) {
    background: #059669;
  }
  .mc-icon-btn {
    padding: 4px 8px;
    font-size: 11.5px;
    border-radius: 6px;
    background: var(--bg-solid);
    border: 1px solid var(--border);
    color: var(--fg-dim);
  }
  .mc-icon-btn:hover {
    color: var(--fg);
    background: var(--hover);
  }
  .mc-icon-btn.danger:hover {
    color: var(--danger);
    border-color: rgba(239, 68, 68, 0.35);
    background: rgba(239, 68, 68, 0.08);
  }

  /* 卡片右下角高级设置按钮 */
  .mc-adv-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    font-size: 11.5px;
    border-radius: 5px;
    border: 1px solid transparent;
    background: transparent;
    color: var(--fg-dim);
    margin-top: auto;
  }
  .mc-adv-btn:hover,
  .mc-adv-btn.open {
    color: var(--fg);
    background: var(--bg-solid);
    border-color: var(--border);
  }
  .mc-adv-arrow {
    font-size: 10px;
  }

  /* 点击下载后展开的线路选择栏 */
  .mc-route-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 8px;
    padding: 9px 16px;
    border-top: 1px solid var(--border);
    background: var(--bg-solid);
    border-radius: 0 0 11px 11px;
  }
  .mc-route-label {
    font-size: 11.5px;
    color: var(--fg-dim);
    font-weight: 500;
  }
  .mc-route-opts {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .route-chip {
    padding: 4px 10px;
    font-size: 11.5px;
    border-radius: 6px;
    background: var(--panel);
    border: 1px solid var(--border);
    color: var(--fg-dim);
  }
  .route-chip:hover {
    color: var(--fg);
  }
  .route-chip.active {
    background: var(--fg);
    color: var(--bg-solid);
    border-color: transparent;
    font-weight: 550;
  }
  .mc-route-go {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-left: auto;
  }

  /* 卡片内紧凑展开的模型专属高级设置面板 */
  .mc-adv-drawer {
    padding: 10px 16px 11px;
    border-top: 1px solid var(--border);
    background: var(--bg-solid);
    border-radius: 0 0 11px 11px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .mc-adv-grid {
    display: grid;
    grid-template-columns: repeat(5, minmax(0, 1fr));
    gap: 8px 10px;
  }
  .mc-adv-row2 {
    display: flex;
    align-items: flex-end;
    gap: 10px;
  }
  .mc-param-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 11.5px;
    color: var(--fg-dim);
    min-width: 0;
  }
  .mc-lbl-row {
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .mc-param-field.grow {
    flex: 1;
  }
  .mc-param-field.port-col {
    width: 110px;
    flex-shrink: 0;
  }
  .mc-param-field select,
  .mc-param-field input {
    font-size: 11.5px;
    padding: 4px 7px;
    height: 28px;
    border-radius: 6px;
    background: var(--panel);
    color: var(--fg);
    border: 1px solid var(--border);
  }
  .mc-adv-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    font-size: 11px;
    color: var(--fg-dim);
    padding-top: 2px;
  }
  .truncate {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
  .active-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--ok);
    flex-shrink: 0;
  }
  .mode-indicator {
    display: inline-flex;
    align-items: center;
    gap: 6px;
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
  .hw-recommend-banner.compact,
  .cloud-active-banner.compact {
    padding: 8px 14px;
  }
  .cloud-active-banner {
    background: color-mix(in srgb, var(--accent-blue) 8%, var(--panel));
    border-color: color-mix(in srgb, var(--accent-blue) 35%, var(--border));
  }
  .hwr-inline {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    font-size: 12.5px;
    color: var(--fg);
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

  .port-cfg-group {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .port-input {
    width: 90px;
    height: 30px;
    padding: 0 8px;
    font-size: 13px;
    font-family: Consolas, monospace;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--panel);
    color: var(--fg);
    text-align: center;
  }
  .port-status-pill {
    font-size: 12px;
    padding: 3px 8px;
    border-radius: 5px;
  }
  .port-status-pill.ok {
    background: color-mix(in srgb, var(--ok) 12%, transparent);
    color: var(--ok);
  }
  .port-status-pill.warn {
    background: rgba(239, 68, 68, 0.12);
    color: var(--danger);
  }

  /* 知识库专属样式 */
  .row-item-vertical {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 6px 0;
  }
  .path-input-group {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    flex-wrap: wrap;
  }
  .path-input {
    flex: 1;
    min-width: 260px;
    font-size: 12.5px;
    font-family: Consolas, monospace;
  }
  .btn-action {
    white-space: nowrap;
    font-size: 12px;
    padding: 6px 12px;
  }
  .kb-stats-strip {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    padding: 9px 16px;
    border-top: 1px dashed var(--border);
    font-size: 12px;
    color: var(--fg-dim);
  }
  .kb-stats-strip b {
    color: var(--fg);
    font-family: Consolas, monospace;
  }
  .kb-s-hi b {
    color: var(--accent-blue);
  }
  .kb-s-sep {
    opacity: 0.35;
  }
  .laya-pill {
    margin-left: 6px;
    font-size: 10.5px;
    padding: 1px 6px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--fg) 7%, transparent);
    color: var(--fg-dim);
    font-weight: 500;
  }
  .laya-pill.ok {
    background: color-mix(in srgb, var(--ok) 14%, transparent);
    color: var(--ok);
  }
  .laya-compact-ctrl {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }
  .laya-dl-mini {
    font-size: 12px;
    font-family: Consolas, monospace;
    color: var(--accent-blue);
    font-weight: 600;
  }
  .laya-dl-box.mini {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 16px 12px;
  }
  .laya-dl-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 16px 8px;
    font-size: 11.5px;
    color: var(--fg-dim);
  }
  .laya-dl-box.mini .laya-dl-foot {
    padding: 0;
  }
  .progress-bar {
    width: 100%;
    height: 5px;
    background: var(--border);
    border-radius: 3px;
    overflow: hidden;
  }
  .progress-fill {
    height: 100%;
    background: var(--accent-blue);
    transition: width 0.2s ease;
  }
  .dl-err {
    font-size: 12px;
    color: var(--danger);
  }
</style>
