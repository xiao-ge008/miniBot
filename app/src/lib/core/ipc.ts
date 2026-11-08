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
  CloudTestResult,
  DownloadProgress,
  EngineOverview,
  GpuSnapshot,
  HistoryEntry,
  LocalSendItem,
  LocalSendPeer,
  LocalSendProgress,
  LocalSendSelfInfo,
  OcrResult,
  Payload,
  QrResult,
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
  engineOverview: () => invoke<EngineOverview>('engine_overview'),
  engineActivateModel: (modelId: string) => invoke<EngineOverview>('engine_activate_model', { modelId }),
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
  saveImage: (data: string) => invoke<string>('save_image', { data }),
  ocrImage: (data: string) => invoke<OcrResult>('ocr_image', { data }),
  qrDecode: (data: string) => invoke<QrResult[]>('qr_decode', { data }),
  qrEncode: (text: string, ec: string, dark: string, light: string) => invoke<string>('qr_encode', { text, ec, dark, light }),
  pinImage: (data: string, x: number, y: number, w: number, h: number) => invoke<void>('pin_image', { data, x, y, w, h }),

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
  chatMessageAdd: (sessionId: string, role: string, content: string, attachments: ChatAttachment[] = []) =>
    invoke<number>('chat_message_add', { sessionId, role, content, attachments }),

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

export function onTtsEvent(cb: (data: { action: string; id?: string; token?: string }) => void) {
  return listen<{ action: string; id?: string; token?: string }>('minibot://tts', (e) => cb(e.payload));
}

/** 智能判断用户提问是否需要触发联网搜索 */
export function shouldAutoWebSearch(text: string): boolean {
  const t = text.trim();
  if (!t || t.length < 2) return false;
  return /联网|搜索|搜一下|查一下|查询|最新|今天|昨日|昨天|今年|现在|目前|近期|最近|2024|2025|2026|新闻|天气|股价|汇率|价格|比分|官网|网址|链接|是谁|是什么|为什么|怎么|如何|哪里|哪个|多少|介绍|百科|评价|对比|news|search|latest|today|price/i.test(
    t,
  );
}

/** 将联网搜索结果格式化为注入给模型的上下文文本 */
export function formatSearchContext(resp: SearchResponse): string {
  if (!resp.hits || resp.hits.length === 0) return '';
  const lines = resp.hits.map(
    (h: SearchHit, idx: number) => `[${idx + 1}] 标题：${h.title}\n摘要：${h.snippet}\n链接：${h.url}`,
  );
  return `\n\n【实时联网搜索参考资料（当前时间：${resp.date}，关键词：${resp.query}）】\n${lines.join('\n\n')}\n\n请优先依据上述实时联网搜索资料准确、简洁地回答用户问题。`;
}

let seq = 0;
export const uid = (prefix = 'r') => `${prefix}${Date.now().toString(36)}${(seq++).toString(36)}`;
