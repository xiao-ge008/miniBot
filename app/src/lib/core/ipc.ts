// 类型化的 IPC 封装：前端只通过这里调用 Rust
import { invoke, Channel } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type {
  AppInfo,
  ChatAttachment,
  ChatEvent,
  ChatMessage,
  ChatMessageRecord,
  ChatSession,
  ChatStats,
  CloudTestResult,
  CustomLocalModel,
  DocFileInfo,
  DocWriteResult,
  DownloadProgress,
  EngineOverview,
  GpuSnapshot,
  HistoryEntry,
  ImageSearchResponse,
  KbSearchHit,
  KbSearchResponse,
  KbStats,
  KbSyncSummary,
  LayaStatus,
  LocalSendItem,
  LocalSendPeer,
  LocalSendProgress,
  LocalSendSelfInfo,
  ModelParams,
  OcrResult,
  Payload,
  QrResult,
  RecorderStateInfo,
  RecordStartArgs,
  ScannedFolderResult,
  SearchHit,
  SearchResponse,
  Settings,
  StatusInfo,
  UpdateDownloadProgress,
  UpdateInfo,
  VoiceOption,
} from './types';

export const ipc = {
  takePayload: () => invoke<Payload | null>('take_payload'),
  hideSelf: () => invoke<void>('hide_self'),
  minimizeSelf: () => invoke<void>('minimize_self'),
  openResult: (payload: Payload) => invoke<void>('open_result', { payload }),
  openPage: (route: string, title: string) => invoke<void>('open_page', { route, title }),
  closePage: () => invoke<void>('close_page'),
  openQr: (text: string) => invoke<void>('open_qr', { text }),
  openLocalSend: (payload?: Payload | null) => invoke<void>('open_localsend', { payload: payload ?? null }),
  trigger: (name: string) => invoke<void>('trigger', { name }),
  replaceText: (hwnd: number, text: string) => invoke<void>('replace_text', { hwnd, text }),

  getSettings: () => invoke<Settings>('get_settings'),
  saveSettings: (settings: Settings) => invoke<string[]>('save_settings', { settings }),
  hotkeysSuspend: (suspend: boolean) => invoke<void>('hotkeys_suspend', { suspend }),
  appInfo: () => invoke<AppInfo>('app_info'),

  // 知识库 API
  kbSaveClip: (text: string, sourceApp?: string, url?: string) =>
    invoke<string>('kb_save_clip', { text, sourceApp: sourceApp ?? null, url: url ?? null }),
  kbSaveImage: (imgData: string, ocrText?: string, visionDesc?: string, source?: string) =>
    invoke<string>('kb_save_image', {
      imgData,
      ocrText: ocrText ?? null,
      visionDesc: visionDesc ?? null,
      source: source ?? null,
    }),
  kbSaveWeb: (url: string, content: string, title?: string) =>
    invoke<string>('kb_save_web', { url, content, title: title ?? null }),
  kbGetStats: () => invoke<KbStats>('kb_get_stats'),
  kbOpenFolder: () => invoke<void>('kb_open_folder'),
  kbPickFolder: () => invoke<KbSyncSummary | null>('kb_pick_folder'),
  kbSyncIndex: () => invoke<KbSyncSummary>('kb_sync_index'),
  kbLayaStatus: () => invoke<LayaStatus>('kb_laya_status'),
  kbDownloadLaya: (onProgress: (p: DownloadProgress) => void) => {
    const ch = new Channel<DownloadProgress>();
    ch.onmessage = onProgress;
    return invoke<LayaStatus>('kb_download_laya', { onProgress: ch });
  },
  kbDeleteLaya: () => invoke<void>('kb_delete_laya'),
  kbOpenFile: (path: string) => invoke<void>('kb_open_file', { path }),
  kbRevealFile: (path: string) => invoke<void>('kb_reveal_file', { path }),
  kbReadImageBase64: (path: string) => invoke<string>('kb_read_image_base64', { path }),
  docRead: (path: string) => invoke<DocFileInfo>('doc_read_file', { path }),
  docWrite: (path: string, content: string) =>
    invoke<DocWriteResult>('doc_write_file', { path, content }),
  docOpenEditor: (path?: string | null, query?: string | null) =>
    invoke<void>('doc_open_editor', { path: path ?? null, query: query ?? null }),
  docGetPending: () => invoke<[string, string | null] | null>('doc_get_pending'),
  docSetContextMenu: (enable: boolean) =>
    invoke<boolean>('doc_set_context_menu', { enable }),
  docIsContextMenuRegistered: () => invoke<boolean>('doc_is_context_menu_registered'),
  kbSearch: (query: string, limit = 20) =>
    invoke<KbSearchResponse>('kb_search', { query, limit }),
  saveImageToKnowledgeWithVision: async (
    imageData: string,
    sourceApp?: string | null,
    existingOcr?: string | null,
    existingVision?: string | null,
  ): Promise<{ path: string; ocr: string; vision: string }> => {
    // 1. OCR 提取文字
    let ocrText = existingOcr ?? '';
    if (!ocrText) {
      try {
        const ocrRes = await ipc.ocrImage(imageData);
        ocrText = ocrRes.text ?? '';
      } catch {
        /* OCR 容错 */
      }
    }

    // 2. 视觉大模型深度结构化解析（面向知识库以文搜图的穷尽式多维细节分析）
    let aiVisionAnalysis = '';
    const hintContext = existingVision?.trim() ? `【已知前置线索/问答主题】：\n${existingVision.trim()}\n\n` : '';
    const ocrContext = ocrText.trim() ? `【画面中识别到的文字】：\n${ocrText.slice(0, 500).trim()}\n\n` : '';

    try {
      const reqId = uid('vis_kb');
      const detailedPrompt = `你是一位顶尖的图像资产分析与视觉知识库索引专家。请对当前图片进行【深度、详尽、穷尽一切细节】的结构化视觉解析。
你的解析将直接用于个人知识库以文搜图（自然语言问答与关键词检索）。请务必巨细靡遗地描摹画面中出现的每一个元素与特征，绝对不要概括省略、不要写“等等”或套话，必须确保用户日后搜查任何细节（如“穿着什么衣服”、“什么发型颜色”、“屏幕上的具体文字”、“界面中的某个功能按钮或报错”、“场景里的具体物品道具”）时都能 100% 准确命中！

${hintContext}${ocrContext}
请严格按照以下 5 个维度展开详尽分析（使用具体丰富的名词与描述性词汇）：

### 1. 画面类型与核心主题
明确说明画面类型（如：角色立绘/插画、现实人物写真、软件系统UI界面截图、聊天记录、网页截图、图表数据文档、风景建筑等），并详细说明画面的核心内容、主要事件与主题。

### 2. 人物形象与服饰穿搭（若画面含人物/角色，必须极尽详尽，若无则明确注明无）
- **外貌神态**：性别、大致年龄/体态、发型（长发/短发/双马尾/发饰造型）、发色、眼睛特征/瞳色、五官妆容、面部表情与情绪神态；
- **全身服饰穿搭（重中之重）**：
  * 上衣：详细款式（如衬衫、T恤、连帽衫、吊带背心、针织毛衣、西服制服等）、具体颜色、领口形状、袖长款式、印花/纹理/面料材质；
  * 下装：具体款式（如百褶裙、超短裙、牛仔长裤、西装裤、短裤、工装裤等）、颜色、裙长/裤长、褶皱或特色装饰；
  * 外套与配饰：外套大衣、领带/领结/领花、帽子、眼镜、腰带、首饰项链、手链手套、发卡发带、包包、手持道具等每一个细节；
  * 腿部与鞋袜：丝袜/连裤袜/过膝袜/短袜的具体颜色、透明度与长度，鞋子类型（高跟鞋、皮鞋、运动鞋、长筒靴、凉鞋等）；
- **姿态与动作**：站姿/坐姿/走动姿态、双手摆放位置与微动作、身体朝向与视线注视方向。

### 3. UI 界面、功能模块与文字信息（若为软件截图/网页/聊天记录，若无则跳过）
- 界面所属软件或系统名称（如 VSCode、微信、Chrome 浏览器、业务后台管理系统等）；
- 整体窗口布局（顶部菜单导航、左侧边栏树、中央主体展示区、右侧面板、底部状态栏等具体分区）；
- 界面中出现的关键文字、标题、菜单项、按钮文字、输入框提示、代码片段、错误或警告提示文本（尽可能完整保留原词原文）；
- 处于激活/选中状态的选项卡、高亮颜色与相关状态指示。

### 4. 场景环境、道具陈设与光影色彩
- 场景背景：具体环境（室内卧室、现代化办公室、咖啡厅、校园教室、户外自然风景、纯色虚化背景等）；
- 背景物品：陈列的家具、电脑数码设备、植物花卉、桌上小物件、墙面挂饰等具体道具；
- 光影与色彩：主色调搭配、光线明暗、色温与整体视觉氛围。

### 5. 检索关键词库 (Search Keywords)
请在最后列出 20-40 个逗号分隔的高频检索关键词（全面涵盖：角色名/主题名、别名、所有服饰穿搭词、发型发色、具体动作姿势、场景环境词、界面功能词、主色调、同义词与英文对照词等），直接赋能全文检索精准命中。`;

      const messages: ChatMessage[] = [
        {
          role: 'user',
          content: [
            {
              type: 'image_url',
              image_url: { url: imageData },
            },
            {
              type: 'text',
              text: detailedPrompt,
            },
          ],
        },
      ];
      let collected = '';
      await ipc.llmChat(
        reqId,
        messages,
        (ev) => {
          if (ev.type === 'delta') collected += ev.text;
        },
        {
          params: {
            max_tokens: 2048,
            temperature: 0.2,
          },
        },
      );
      aiVisionAnalysis = collected.trim();
    } catch (e) {
      console.warn('视觉深度解析调用异常，降级保存:', e);
    }

    // 组合最终存储的视觉描述（保留前置线索，融合大模型穷尽式细节）
    let finalVisionDesc = '';
    if (existingVision?.trim()) {
      finalVisionDesc += `### 📌 问答/主题摘要\n${existingVision.trim()}\n\n`;
    }
    if (aiVisionAnalysis) {
      finalVisionDesc += aiVisionAnalysis;
    } else if (existingVision?.trim()) {
      finalVisionDesc = existingVision.trim();
    }

    // 3. 后端保存原图与伴生同名 Markdown 文件
    const savedPath = await ipc.kbSaveImage(
      imageData,
      ocrText || undefined,
      finalVisionDesc || undefined,
      sourceApp ?? undefined,
    );

    // 4. 自动增量构建索引
    try {
      await ipc.kbSyncIndex();
    } catch {
      /* 忽略索引异常 */
    }

    return { path: savedPath, ocr: ocrText, vision: finalVisionDesc };
  },
  saveToKnowledge: async (p: Payload): Promise<string> => {
    if (p.kind === 'image' && p.image) {
      const res = await ipc.saveImageToKnowledgeWithVision(
        p.image,
        p.origin,
        p.text ?? undefined,
        p.meta?.vision_desc as string | undefined,
      );
      return res.path;
    } else if (p.kind === 'url' && p.text) {
      return await ipc.kbSaveWeb(p.text, p.text, p.meta?.title as string | undefined);
    } else if (p.text) {
      return await ipc.kbSaveClip(p.text, p.origin, (p.meta?.url as string) ?? undefined);
    }
    throw new Error('不支持的内容类型');
  },

  llmChat: (
    id: string,
    messages: ChatMessage[],
    onEvent: (e: ChatEvent) => void,
    opts: { params?: Record<string, unknown>; force?: boolean } = {},
  ) => {
    const ch = new Channel<ChatEvent>();
    ch.onmessage = onEvent;
    return invoke<string>('llm_chat', { id, messages, params: opts.params ?? null, force: opts.force ?? false, onEvent: ch });
  },
  llmCancel: (id: string) => invoke<void>('llm_cancel', { id }),
  llmStatus: () => invoke<StatusInfo>('llm_status'),
  llmTestApi: (baseUrl: string, apiKey: string, model: string) =>
    invoke<CloudTestResult>('llm_test_api', { baseUrl, apiKey, model }),
  llmFetchModels: (baseUrl: string, apiKey: string) =>
    invoke<string[]>('llm_fetch_models', { baseUrl, apiKey }),
  llmRelease: () => invoke<void>('llm_release'),
  llmStart: () => invoke<void>('llm_start'),
  llmCheckPort: (port: number) =>
    invoke<{ available: boolean; port: number; suggested_port: number }>('llm_check_port', { port }),
  engineOverview: () => invoke<EngineOverview>('engine_overview'),
  engineActivateModel: (modelId: string) => invoke<EngineOverview>('engine_activate_model', { modelId }),
  engineUpdateModelParams: (modelId: string, params: ModelParams) =>
    invoke<EngineOverview>('engine_update_model_params', { modelId, params }),
  engineScanCustomFolder: () => invoke<ScannedFolderResult | null>('engine_scan_custom_folder'),
  engineSaveCustomModel: (model: CustomLocalModel) =>
    invoke<EngineOverview>('engine_save_custom_model', { model }),
  engineDeleteCustomModel: (modelId: string) =>
    invoke<EngineOverview>('engine_delete_custom_model', { modelId }),
  enginePickModelsDir: () => invoke<[EngineOverview, number] | null>('engine_pick_models_dir'),
  engineDownloadModel: (modelId: string, mirror: string, onProgress: (p: DownloadProgress) => void) => {
    const ch = new Channel<DownloadProgress>();
    ch.onmessage = onProgress;
    return invoke<EngineOverview>('engine_download_model', { modelId, mirror, onProgress: ch });
  },
  engineDownloadRuntime: (backend: string, onProgress: (p: DownloadProgress) => void) => {
    const ch = new Channel<DownloadProgress>();
    ch.onmessage = onProgress;
    return invoke<EngineOverview>('engine_download_runtime', { backend, onProgress: ch });
  },
  engineCancelDownload: () => invoke<void>('engine_cancel_download'),
  gpuSnapshot: () => invoke<GpuSnapshot>('gpu_snapshot'),

  captureReady: () => invoke<void>('capture_ready'),
  captureClose: () => invoke<void>('capture_close'),
  sendToChat: (image: string, autoSend = true) => invoke<void>('send_to_chat', { image, autoSend }),
  webSearch: (query: string, maxResults = 5) => invoke<SearchResponse>('web_search', { query, maxResults }),
  copyText: (text: string) => invoke<void>('copy_text', { text }),
  copyImage: (data: string) => invoke<void>('copy_image', { data }),
  saveImage: async (data: string) => {
    if (typeof window !== 'undefined') (window as any).__mb_dialog_open = true;
    try {
      return await invoke<string>('save_image', { data });
    } finally {
      if (typeof window !== 'undefined') {
        setTimeout(() => {
          (window as any).__mb_dialog_open = false;
        }, 300);
      }
    }
  },
  ocrImage: (data: string) => invoke<OcrResult>('ocr_image', { data }),
  qrDecode: (data: string) => invoke<QrResult[]>('qr_decode', { data }),
  qrEncode: (text: string, ec: string, dark: string, light: string) => invoke<string>('qr_encode', { text, ec, dark, light }),
  pinImage: (data: string, x: number, y: number, w: number, h: number) => invoke<void>('pin_image', { data, x, y, w, h }),
  imageSearch: (data: string, engine = 'baidu') => invoke<ImageSearchResponse>('image_search', { data, engine }),
  fetchRemoteImage: (url: string) => invoke<string>('fetch_remote_image', { url }),

  historyAdd: (entry: HistoryEntry, image?: string | null) => invoke<number>('history_add', { entry, image: image ?? null }),
  historyList: (query: string, limit = 200) => invoke<HistoryEntry[]>('history_list', { query, limit }),
  historyDelete: (id: number) => invoke<void>('history_delete', { id }),
  historyClear: () => invoke<void>('history_clear'),
  readImageFile: (path: string) => invoke<string>('read_image_file', { path }),

  chatSessionList: () => invoke<ChatSession[]>('chat_session_list'),
  chatSessionCreate: (title = '新对话') => invoke<ChatSession>('chat_session_create', { title }),
  chatSessionRename: (id: string, title: string) => invoke<void>('chat_session_rename', { id, title }),
  chatSessionDelete: (id: string) => invoke<void>('chat_session_delete', { id }),
  chatMessageList: (sessionId: string) => invoke<ChatMessageRecord[]>('chat_message_list', { sessionId }),
  chatMessageAdd: (
    sessionId: string,
    role: string,
    content: string,
    attachments: ChatAttachment[] = [],
    stats?: ChatStats | null,
    kbHits?: KbSearchHit[] | null,
  ) =>
    invoke<number>('chat_message_add', {
      sessionId,
      role,
      content,
      attachments,
      stats: stats ?? null,
      kbHits: kbHits ?? null,
    }),
  chatMessageDeleteFrom: (sessionId: string, fromId: number) =>
    invoke<void>('chat_message_delete_from', { sessionId, fromId }),

  listUserAgents: () => invoke<unknown[]>('list_user_agents'),
  saveUserAgent: (agent: unknown) => invoke<void>('save_user_agent', { agent }),
  deleteUserAgent: (id: string) => invoke<void>('delete_user_agent', { id }),
  openAgentsDir: () => invoke<void>('open_agents_dir'),

  ttsVoices: () => invoke<VoiceOption[]>('tts_voices'),
  ttsSpeak: (text: string, voice?: string | null, rate?: string | null) =>
    invoke<string>('tts_speak', { text, voice: voice ?? null, rate: rate ?? null }),
  ttsBroadcast: (payload: Record<string, unknown>) => invoke<void>('tts_broadcast', { payload }),

  localSendInfo: () => invoke<LocalSendSelfInfo>('localsend_info'),
  localSendScan: (extraIps: string[] = []) => invoke<LocalSendPeer[]>('localsend_scan', { extraIps }),
  localSendProbe: (ip: string, port?: number) => invoke<LocalSendPeer>('localsend_probe', { ip, port: port ?? null }),
  localSendPickFiles: () => invoke<LocalSendItem[]>('localsend_pick_files'),
  localSendPickFolder: () => invoke<LocalSendItem[]>('localsend_pick_folder'),
  localSendResolvePaths: (paths: string[]) => invoke<LocalSendItem[]>('localsend_resolve_paths', { paths }),
  localSendSend: (
    peer: LocalSendPeer,
    items: LocalSendItem[],
    pin: string | null,
    onProgress: (p: LocalSendProgress) => void,
  ) => {
    const ch = new Channel<LocalSendProgress>();
    ch.onmessage = onProgress;
    return invoke<void>('localsend_send', { peer, items, pin: pin ?? null, onProgress: ch });
  },
  localSendCancel: () => invoke<void>('localsend_cancel'),
  localSendSetContextMenu: (enable: boolean) => invoke<boolean>('localsend_set_context_menu', { enable }),

  appCheckUpdate: () => invoke<UpdateInfo>('app_check_update'),
  appDownloadUpdate: (
    downloadUrl: string,
    assetName: string,
    useMirror: boolean,
    onProgress: (p: UpdateDownloadProgress) => void,
  ) => {
    const ch = new Channel<UpdateDownloadProgress>();
    ch.onmessage = onProgress;
    return invoke<string>('app_download_update', {
      downloadUrl,
      assetName,
      useMirror,
      onProgress: ch,
    });
  },
  appCancelUpdateDownload: () => invoke<void>('app_cancel_update_download'),
  appInstallUpdate: (installerPath: string) => invoke<void>('app_install_update', { installerPath }),
  openExternalUrl: (url: string) => invoke<void>('open_external_url', { url }),

  recorderStart: (args: RecordStartArgs) => invoke<RecorderStateInfo>('recorder_start', { args }),
  recorderPause: () => invoke<RecorderStateInfo>('recorder_pause'),
  recorderResume: () => invoke<RecorderStateInfo>('recorder_resume'),
  recorderStop: () => invoke<RecorderStateInfo>('recorder_stop'),
  recorderCancel: () => invoke<void>('recorder_cancel'),
  recorderStatus: () => invoke<RecorderStateInfo>('recorder_status'),
  recorderReveal: (path: string) => invoke<void>('recorder_reveal', { path }),
  recorderCopyFile: (path: string) => invoke<void>('recorder_copy_file', { path }),
};

/** 订阅本窗口的 Payload 投递：挂载时取一次，之后每次投递再取 */
export async function onPayload(cb: (p: Payload) => void): Promise<() => void> {
  const first = await ipc.takePayload();
  if (first) cb(first);
  return listen('minibot://payload', async () => {
    const p = await ipc.takePayload();
    if (p) cb(p);
  });
}

export function onLlmStatus(cb: (s: StatusInfo) => void) {
  return listen<StatusInfo>('llm://status', (e) => cb(e.payload));
}

export function onSettings(cb: (s: Settings) => void) {
  return listen<Settings>('minibot://settings', (e) => cb(e.payload));
}

export function onChatCapture(cb: (data: { image: string; auto_send: boolean }) => void) {
  return listen<{ image: string; auto_send: boolean }>('minibot://chat-capture', (e) => cb(e.payload));
}

export function onOutsideClick(cb: () => void) {
  return listen('minibot://outside-click', () => cb());
}

export function onDocOpen(cb: (payload: { path?: string | null; query?: string | null }) => void) {
  return listen<{ path?: string | null; query?: string | null }>('minibot://open-doc', (ev) => cb(ev.payload));
}

export function onTtsEvent(cb: (data: { action: string; id?: string; token?: string }) => void) {
  return listen<{ action: string; id?: string; token?: string }>('minibot://tts', (e) => cb(e.payload));
}

export function onRecorderState(cb: (s: RecorderStateInfo) => void) {
  return listen<RecorderStateInfo>('minibot://recorder-state', (e) => cb(e.payload));
}

/** 默认关闭自动联网搜索 */
export function shouldAutoWebSearch(_text: string): boolean {
  return false;
}

/** 将联网搜索结果格式化为注入给模型的上下文文本 */
export function formatSearchContext(resp: SearchResponse): string {
  if (!resp.hits || resp.hits.length === 0) return '';
  const lines = resp.hits.map(
    (h: SearchHit, idx: number) => `[${idx + 1}] 标题：${h.title}\n摘要：${h.snippet}\n链接：${h.url}`,
  );
  return `\n\n【实时联网搜索参考资料（当前时间：${resp.date}，关键词：${resp.query}）】\n${lines.join('\n\n')}\n\n请优先依据上述实时联网搜索资料准确、简洁地回答用户问题。`;
}

/** 格式化 AI 回复的 token 速度与消耗时长 */
export function formatChatStats(stats?: ChatStats | null): string {
  if (!stats || (stats.tokens <= 0 && stats.duration_ms <= 0)) return '';
  const sec = (stats.duration_ms / 1000).toFixed(stats.duration_ms < 10000 ? 2 : 1);
  const tps = stats.tokens_per_sec > 0 ? stats.tokens_per_sec.toFixed(1) : '0.0';
  return `${tps} tokens/s · 耗时 ${sec}s (${stats.tokens} tokens)`;
}

let seq = 0;
export const uid = (prefix = 'r') => `${prefix}${Date.now().toString(36)}${(seq++).toString(36)}`;
