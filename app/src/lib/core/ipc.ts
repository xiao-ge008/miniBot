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

    // 2. 视觉大模型深度整理画面主体与细节
    let visionDesc = existingVision ?? '';
    if (!visionDesc) {
      try {
        const reqId = uid('vis_kb');
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
                text: '请对这张图片进行结构化视觉内容整理（用于个人知识库沉淀与以文搜图）：\n1. 画面主体与场景背景；\n2. 核心视觉要素、人物状态、界面布局与关键细节；\n3. 图片所传达的核心信息与主题。\n请条理清晰，简明扼要。',
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
              max_tokens: 512,
              temperature: 0.3,
            },
          },
        );
        visionDesc = collected.trim();
      } catch (e) {
        console.warn('视觉整理调用异常，降级保存:', e);
      }
    }

    // 3. 后端保存原图与伴生 vision.md
    const savedPath = await ipc.kbSaveImage(
      imageData,
      ocrText || undefined,
      visionDesc || undefined,
      sourceApp ?? undefined,
    );

    // 4. 自动增量构建索引
    try {
      await ipc.kbSyncIndex();
    } catch {
      /* 忽略索引异常 */
    }

    return { path: savedPath, ocr: ocrText, vision: visionDesc };
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
