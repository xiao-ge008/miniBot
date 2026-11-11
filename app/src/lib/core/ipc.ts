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
  ClipDistillResult,
  CloudTestResult,
  CustomLocalModel,
  DocFileInfo,
  DocWriteResult,
  DownloadProgress,
  EngineOverview,
  GpuSnapshot,
  HistoryEntry,
  ImageSearchResponse,
  KbEnqueueResult,
  KbQueueInput,
  KbQueueStatus,
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
  PromptTemplate,
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
  VisionAnalysisResult,
  VoiceOption,
} from './types';

/** 清理模型输出中残留的思考链标签 */
function cleanThinkTags(str?: string | null): string {
  if (!str) return '';
  return str.replace(/<think>[\s\S]*?(<\/think>|$)/gi, '').trim();
}

/** 将分类器原始返回归一化为视觉提示词模板 ID（人物统一归入唯一的 character，其余风景/界面/静物/图表按各自类别） */
function resolveVisionCategory(rawOutput: string, knownCustomIds: string[] = []): string {
  const clean = cleanThinkTags(rawOutput).toLowerCase().trim();
  if (!clean) return 'general';

  for (const cid of knownCustomIds) {
    const low = cid.toLowerCase();
    if (low && low !== 'classifier' && clean.includes(low)) {
      return cid;
    }
  }
  if (
    clean.includes('character') ||
    clean.includes('couple_intimate') ||
    clean.includes('couple') ||
    clean.includes('intimate') ||
    clean.includes('人物') ||
    clean.includes('角色') ||
    clean.includes('立绘') ||
    clean.includes('人像') ||
    clean.includes('亲密') ||
    clean.includes('缠绵') ||
    clean.includes('情侣') ||
    clean.includes('写真')
  ) {
    return 'character';
  }
  if (clean.includes('scenery') || clean.includes('风景') || clean.includes('建筑') || clean.includes('街景')) {
    return 'scenery';
  }
  if (clean.includes('ui_software') || clean.includes('ui') || clean.includes('界面') || clean.includes('软件') || clean.includes('聊天')) {
    return 'ui_software';
  }
  if (clean.includes('object_food') || clean.includes('美食') || clean.includes('静物') || clean.includes('物体') || clean.includes('食物')) {
    return 'object_food';
  }
  if (clean.includes('document_chart') || clean.includes('图表') || clean.includes('文档') || clean.includes('架构')) {
    return 'document_chart';
  }
  return 'general';
}

/** 渲染视觉解析提示词模板中的动态占位符（{{HINT_CONTEXT}} / {{OCR_CONTEXT}}） */
function renderVisionPrompt(body: string, hintText: string, ocrText: string): string {
  const hintBlock = hintText.trim() ? `【已知前置线索/问答主题】：\n${hintText.trim()}\n` : '';
  const ocrBlock = ocrText.trim() ? `【画面中提取到的文字原词】：\n${ocrText.slice(0, 500).trim()}\n` : '';

  let rendered = body;
  const hasHintSlot = rendered.includes('{{HINT_CONTEXT}}');
  const hasOcrSlot = rendered.includes('{{OCR_CONTEXT}}');

  if (hasHintSlot) {
    rendered = rendered.replaceAll('{{HINT_CONTEXT}}', hintBlock);
  } else if (hintBlock) {
    rendered = `${hintBlock}\n${rendered}`;
  }

  if (hasOcrSlot) {
    rendered = rendered.replaceAll('{{OCR_CONTEXT}}', ocrBlock);
  } else if (ocrBlock) {
    rendered = `${ocrBlock}\n${rendered}`;
  }

  return rendered.replace(/\n{3,}/g, '\n\n').trim();
}

/** 渲染片段提纯/总结提示词模板中的动态占位符（{{SOURCE_CONTEXT}} / {{CLIP_TEXT}}） */
function renderClipPrompt(body: string, clipText: string, sourceApp?: string | null, url?: string | null): string {
  const metaLines: string[] = [];
  if (sourceApp?.trim()) metaLines.push(`- 来源应用：${sourceApp.trim()}`);
  if (url?.trim()) metaLines.push(`- 关联网址：${url.trim()}`);
  const sourceBlock = metaLines.length ? `【来源上下文】：\n${metaLines.join('\n')}\n` : '';

  let rendered = body;
  if (rendered.includes('{{SOURCE_CONTEXT}}')) {
    rendered = rendered.replaceAll('{{SOURCE_CONTEXT}}', sourceBlock);
  } else if (sourceBlock) {
    rendered = `${sourceBlock}\n${rendered}`;
  }

  if (rendered.includes('{{CLIP_TEXT}}')) {
    rendered = rendered.replaceAll('{{CLIP_TEXT}}', clipText.trim());
  } else {
    rendered = `${rendered}\n\n【待提纯原文】：\n\`\`\`text\n${clipText.trim()}\n\`\`\``;
  }

  return rendered.replace(/\n{3,}/g, '\n\n').trim();
}

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
  kbSaveClip: (text: string, sourceApp?: string, url?: string, distilledSummary?: string) =>
    invoke<string>('kb_save_clip', {
      text,
      sourceApp: sourceApp ?? null,
      url: url ?? null,
      distilledSummary: distilledSummary ?? null,
    }),
  kbSaveImage: (imgData: string, ocrText?: string, visionDesc?: string, source?: string) =>
    invoke<string>('kb_save_image', {
      imgData,
      ocrText: ocrText ?? null,
      visionDesc: visionDesc ?? null,
      source: source ?? null,
    }),
  kbSaveWeb: (url: string, content: string, title?: string, distilledSummary?: string) =>
    invoke<string>('kb_save_web', {
      url,
      content,
      title: title ?? null,
      distilledSummary: distilledSummary ?? null,
    }),
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
  kbGetPrompt: (category: 'vision' | 'clip', id: string) =>
    invoke<PromptTemplate>('kb_get_prompt', { category, id }),
  kbListPrompts: () => invoke<PromptTemplate[]>('kb_list_prompts'),
  kbResetPrompt: (category?: string | null, id?: string | null) =>
    invoke<PromptTemplate[]>('kb_reset_prompt', { category: category ?? null, id: id ?? null }),
  kbCreatePrompt: (category: 'vision' | 'clip', id: string, name?: string | null) =>
    invoke<PromptTemplate>('kb_create_prompt', { category, id, name: name ?? null }),
  kbOpenPromptsFolder: (target?: 'work' | 'builtin') =>
    invoke<string>('kb_open_prompts_folder', { target: target ?? null }),
  /**
   * 按类别独立加载磁盘 .md 提示词模板并运行图片视觉解析（供知识库入库与设置页调试台共享）
   */
  analyzeImageWithVisionPrompts: async (
    imageData: string,
    opts: {
      existingOcr?: string | null;
      existingVision?: string | null;
      forceType?: string | null;
      skipOcr?: boolean;
      onStage?: (info: { stage: 'ocr' | 'classify' | 'extract'; detectedType?: string; template?: PromptTemplate }) => void;
      onDelta?: (delta: string, accumulated: string) => void;
    } = {},
  ): Promise<VisionAnalysisResult> => {
    const startTs = performance.now();

    // 1. OCR 提取文字
    let ocrText = opts.existingOcr ?? '';
    if (!ocrText && !opts.skipOcr) {
      opts.onStage?.({ stage: 'ocr' });
      try {
        const ocrRes = await ipc.ocrImage(imageData);
        ocrText = ocrRes.text ?? '';
      } catch {
        /* OCR 容错 */
      }
    }

    const pureExistingVision = cleanThinkTags(opts.existingVision);

    // 2. 阶段一：加载 vision/classifier.md 进行快筛判定（人物主体一律归为唯一的 character，非人物按各自类别加载）
    let detectedType = (opts.forceType ?? '').trim();
    if (!detectedType || detectedType === 'auto') {
      detectedType = 'general';
      opts.onStage?.({ stage: 'classify' });
      try {
        const allPrompts = await ipc.kbListPrompts().catch(() => [] as PromptTemplate[]);
        const customVisionIds = allPrompts
          .filter((p) => p.category === 'vision' && !p.is_builtin)
          .map((p) => p.id);

        const clsTpl = await ipc.kbGetPrompt('vision', 'classifier');
        const clsId = uid('vis_cls');
        const clsMessages: ChatMessage[] = [
          {
            role: 'user',
            content: [
              { type: 'image_url', image_url: { url: imageData } },
              { type: 'text', text: clsTpl.body },
            ],
          },
        ];
        let clsRaw = '';
        await ipc.llmChat(
          clsId,
          clsMessages,
          (ev) => {
            if (ev.type === 'delta') clsRaw += ev.text;
          },
          {
            params: {
              max_tokens: clsTpl.max_tokens || 128,
              temperature: clsTpl.temperature ?? 0.1,
              reasoning_effort: 'off',
              chat_template_kwargs: { enable_thinking: false },
            } as Record<string, unknown>,
          },
        );
        detectedType = resolveVisionCategory(clsRaw, customVisionIds);
      } catch (e) {
        console.warn('图像类型快筛失败，归类 general:', e);
      }
    }

    // 3. 阶段二：直接读取对应的 {detectedType}.md 文件，不做跨类别替补兜底
    let targetTpl: PromptTemplate | null = null;
    try {
      targetTpl = await ipc.kbGetPrompt('vision', detectedType);
    } catch (e) {
      console.warn(`读取视觉提示词 vision/${detectedType}.md 失败:`, e);
      targetTpl = null;
    }

    opts.onStage?.({ stage: 'extract', detectedType, template: targetTpl ?? undefined });

    let aiVisionAnalysis = '';
    if (targetTpl) {
      const specializedPrompt = renderVisionPrompt(targetTpl.body, pureExistingVision, ocrText);
      try {
        const reqId = uid('vis_kb');
        const messages: ChatMessage[] = [
          {
            role: 'user',
            content: [
              { type: 'image_url', image_url: { url: imageData } },
              { type: 'text', text: specializedPrompt },
            ],
          },
        ];
        let collected = '';
        await ipc.llmChat(
          reqId,
          messages,
          (ev) => {
            if (ev.type === 'delta') {
              collected += ev.text;
              opts.onDelta?.(ev.text, cleanThinkTags(collected));
            }
          },
          {
            params: {
              max_tokens: Math.max(targetTpl.max_tokens || 3800, 3800),
              temperature: targetTpl.temperature ?? 0.2,
              reasoning_effort: 'off',
              chat_template_kwargs: { enable_thinking: false },
            } as Record<string, unknown>,
          },
        );
        aiVisionAnalysis = cleanThinkTags(collected);
      } catch (e) {
        console.warn('视觉特化深度解析调用异常，降级保存:', e);
      }
    }

    const categoryTitle = targetTpl?.title || '视觉特征深度解析';
    let finalVisionDesc = `## ${categoryTitle}\n\n`;
    if (pureExistingVision) {
      finalVisionDesc += `### 📌 问答/主题摘要\n${pureExistingVision}\n\n`;
    }
    if (aiVisionAnalysis) {
      finalVisionDesc += aiVisionAnalysis;
    } else if (pureExistingVision) {
      finalVisionDesc += pureExistingVision;
    }

    return {
      detectedType,
      template: targetTpl,
      ocrText,
      rawAnalysis: aiVisionAnalysis,
      finalVisionDesc,
      elapsedMs: Math.round(performance.now() - startTs),
    };
  },

  /**
   * 按 clip/*.md 模板对文本/网页片段进行提纯与总结（供入库与调试台共享）
   */
  distillClipWithPrompt: async (
    text: string,
    opts: {
      templateId?: string;
      sourceApp?: string | null;
      url?: string | null;
      onDelta?: (delta: string, accumulated: string) => void;
    } = {},
  ): Promise<ClipDistillResult> => {
    const startTs = performance.now();
    const tplId = opts.templateId || 'clip_distill';
    let tpl: PromptTemplate | null = null;
    try {
      tpl = await ipc.kbGetPrompt('clip', tplId);
    } catch (e) {
      console.warn(`读取片段提示词 clip/${tplId}.md 失败:`, e);
      tpl = null;
    }

    if (!tpl) {
      return { template: null, summary: '', elapsedMs: Math.round(performance.now() - startTs) };
    }

    const promptText = renderClipPrompt(tpl.body, text, opts.sourceApp, opts.url);
    const reqId = uid('clip_distill');
    let collected = '';
    await ipc.llmChat(
      reqId,
      [{ role: 'user', content: promptText }],
      (ev) => {
        if (ev.type === 'delta') {
          collected += ev.text;
          opts.onDelta?.(ev.text, cleanThinkTags(collected));
        }
      },
      {
        params: {
          max_tokens: tpl.max_tokens || 1536,
          temperature: tpl.temperature ?? 0.2,
          reasoning_effort: 'off',
          chat_template_kwargs: { enable_thinking: false },
        } as Record<string, unknown>,
      },
    );

    return {
      template: tpl,
      summary: cleanThinkTags(collected),
      elapsedMs: Math.round(performance.now() - startTs),
    };
  },

  saveImageToKnowledgeWithVision: async (
    imageData: string,
    sourceApp?: string | null,
    existingOcr?: string | null,
    existingVision?: string | null,
  ): Promise<{ path: string; ocr: string; vision: string }> => {
    const res = await ipc.analyzeImageWithVisionPrompts(imageData, {
      existingOcr,
      existingVision,
    });

    const savedPath = await ipc.kbSaveImage(
      imageData,
      res.ocrText || undefined,
      res.finalVisionDesc || undefined,
      sourceApp ?? undefined,
    );

    try {
      await ipc.kbSyncIndex();
    } catch {
      /* 忽略索引异常 */
    }

    return { path: savedPath, ocr: res.ocrText, vision: res.finalVisionDesc };
  },

  kbEnqueueSave: (input: KbQueueInput) => invoke<KbEnqueueResult>('kb_enqueue_save', { input }),
  kbQueueStatus: () => invoke<KbQueueStatus>('kb_queue_status'),

  enqueueImageToKnowledge: async (
    imageData: string,
    sourceApp?: string | null,
    existingOcr?: string | null,
    existingVision?: string | null,
    title?: string | null,
  ): Promise<KbEnqueueResult> => {
    return await ipc.kbEnqueueSave({
      kind: 'image',
      title: title ?? null,
      image_data: imageData,
      source_app: sourceApp ?? null,
      existing_ocr: existingOcr ?? null,
      existing_vision: existingVision ?? null,
    });
  },

  enqueuePayloadToKnowledge: async (p: Payload): Promise<KbEnqueueResult> => {
    if (p.kind === 'image' && p.image) {
      return await ipc.kbEnqueueSave({
        kind: 'image',
        image_data: p.image,
        source_app: p.origin ?? null,
        existing_ocr: p.text ?? null,
        existing_vision: (p.meta?.vision_desc as string | undefined) ?? null,
      });
    }
    if (p.kind === 'url' && p.text) {
      return await ipc.kbEnqueueSave({
        kind: 'web',
        title: (p.meta?.title as string | undefined) ?? null,
        text: p.text,
        url: p.text,
        source_app: p.origin ?? null,
      });
    }
    if (p.text) {
      return await ipc.kbEnqueueSave({
        kind: 'clip',
        text: p.text,
        url: (p.meta?.url as string | undefined) ?? null,
        source_app: p.origin ?? null,
      });
    }
    throw new Error('不支持的内容类型');
  },

  saveToKnowledge: async (p: Payload): Promise<string> => {
    const res = await ipc.enqueuePayloadToKnowledge(p);
    if (!res.accepted) {
      throw new Error(res.message);
    }
    return res.task_id ?? 'queued';
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

export function onKbQueueUpdate(cb: (s: KbQueueStatus) => void) {
  return listen<KbQueueStatus>('minibot://kb-queue', (e) => cb(e.payload));
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
