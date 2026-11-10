<script lang="ts">
  // 极简多轮 AI 智能聊天主窗口（点击右下角托盘图标 / 双击启动 / Alt+Space 呼出）
  // 特性：
  // 1. 左侧：永久保存的多轮聊天会话列表（与 30 天自动清理的划词/识图记录完全隔离）+ 底部设置图标
  // 2. 右侧：多轮流式对话显示区（支持 <think> 思考过程折叠/展开）+ 底部极简输入框
  // 3. 多模态：支持图片与视频上传/粘贴/拖拽识别（视频自动均匀抽取 6 帧时序故事板供视觉模型识别）
  import { onMount, tick } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import {
    formatChatStats,
    formatSearchContext,
    ipc,
    onChatCapture,
    onLlmStatus,
    onPayload,
    uid,
  } from '$lib/core/ipc';
  import {
    buildReasoningParams,
    normalizeReasoningEffort,
    REASONING_LEVELS,
    type ReasoningEffort,
  } from '$lib/core/agents';
  import { renderMarkdown } from '$lib/core/markdown';
  import { stopSpeak, tts } from '$lib/core/tts.svelte';
  import GpuBusy from '$lib/components/GpuBusy.svelte';
  import SpeakButton from '$lib/components/SpeakButton.svelte';
  import type {
    ChatAttachment,
    ChatMessage,
    ChatMessageRecord,
    ChatSession,
    ChatStats,
    GpuSnapshot,
    KbSearchHit,
    LlmError,
    SearchHit,
    StatusInfo,
    UpdateDownloadProgress,
    UpdateInfo,
  } from '$lib/core/types';

  const win = getCurrentWindow();

  // 会话与消息状态
  let sessions = $state<ChatSession[]>([]);
  let currentSessionId = $state<string>('');
  let messages = $state<ChatMessageRecord[]>([]);

  // 输入与附件状态（默认思考等级跟随当前模型在模型设置中的配置，也可在聊天框随时切换）
  let inputText = $state('');
  let pendingAttachments = $state<ChatAttachment[]>([]);
  let parsingMedia = $state(false);
  let dragOver = $state(false);
  let modelDefaultEffort = $state<ReasoningEffort>('off');
  let reasoningEffort = $state<ReasoningEffort>('off');
  let enableWebSearch = $state(false);
  let searchingWeb = $state(false);
  let lastSearchHits = $state<SearchHit[]>([]);
  let enableKnowledgeSearch = $state(false);
  let searchingKnowledge = $state(false);
  let lastKnowledgeHits = $state<KbSearchHit[]>([]);

  function setReasoningEffort(level: ReasoningEffort) {
    reasoningEffort = level;
  }

  function toggleReasoningQuick() {
    const targetOn = modelDefaultEffort !== 'off' ? modelDefaultEffort : 'medium';
    setReasoningEffort(reasoningEffort === 'off' ? targetOn : 'off');
  }

  // 流式推理状态
  type Stage = 'idle' | 'loading' | 'streaming' | 'error' | 'gpu';
  let stage = $state<Stage>('idle');
  let streamingOutput = $state('');
  let streamingStats = $state<ChatStats | null>(null);
  let errorMsg = $state('');
  let gpu = $state<{ snapshot: GpuSnapshot; required: number } | null>(null);
  let reqId = '';

  let kbActionTip = $state('');
  let kbActionTimer: ReturnType<typeof setTimeout> | undefined;

  function flashMsg(msg: string, ms = 3000) {
    kbActionTip = msg;
    clearTimeout(kbActionTimer);
    kbActionTimer = setTimeout(() => {
      kbActionTip = '';
    }, ms);
  }

  // UI 交互状态
  let status = $state<StatusInfo | null>(null);
  let pinned = $state(false);
  let previewAttachment = $state<ChatAttachment | null>(null);
  let openThinks = $state<Record<string, boolean>>({});
  let streamingThinkOpen = $state(false);
  let copiedId = $state<string | number>('');

  // 在线版本更新提示状态
  let updateInfo = $state<UpdateInfo | null>(null);
  let updateDismissed = $state(false);
  let downloadingUpdate = $state(false);
  let updateDlProgress = $state<UpdateDownloadProgress | null>(null);
  let updateError = $state('');
  let lastUpdateCheckAt = 0;

  async function checkUpdateSilent(force = false) {
    if (downloadingUpdate) return;
    if (!force && Date.now() - lastUpdateCheckAt < 60_000 && updateInfo) return;
    lastUpdateCheckAt = Date.now();
    try {
      const res = await ipc.appCheckUpdate();
      updateInfo = res;
    } catch {
      /* 静默检查忽略网络异常 */
    }
  }

  async function handleOneClickUpdate() {
    if (downloadingUpdate || !updateInfo?.download_url) return;
    downloadingUpdate = true;
    updateError = '';
    updateDlProgress = {
      stage: 'downloading',
      file_name: updateInfo.asset_name || `miniBot_${updateInfo.latest_version}_x64-setup.exe`,
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
      if (savedPath) {
        await ipc.appInstallUpdate(savedPath);
      }
    } catch (e) {
      const errStr = String(e);
      if (!errStr.includes('已取消')) {
        updateError = errStr;
      }
    } finally {
      downloadingUpdate = false;
    }
  }

  let scroller: HTMLDivElement;
  let textareaEl: HTMLTextAreaElement;
  let fileInputEl: HTMLInputElement;
  let pickingFile = false;
  let openedAt = Date.now();

  function openFilePicker() {
    pickingFile = true;
    fileInputEl?.click();
  }

  const busy = $derived(stage === 'loading' || stage === 'streaming');
  const currentSession = $derived(sessions.find((s) => s.id === currentSessionId) ?? null);

  /** 拆分 <think>...</think> 思考过程与正式回答 */
  function splitThink(raw: string): { think: string; answer: string; thinking: boolean } {
    const startIdx = raw.indexOf('<think>');
    if (startIdx === -1) {
      return { think: '', answer: raw, thinking: false };
    }
    const before = raw.slice(0, startIdx);
    const afterStart = raw.slice(startIdx + 7);
    const endIdx = afterStart.indexOf('</think>');
    if (endIdx === -1) {
      return { think: afterStart.trim(), answer: before.trim(), thinking: true };
    }
    const think = afterStart.slice(0, endIdx).trim();
    const answer = (before + afterStart.slice(endIdx + 8)).trim();
    return { think, answer, thinking: false };
  }

  const streamParsed = $derived(splitThink(streamingOutput));

  function scrollDown() {
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  }

  function autoResizeTextarea() {
    if (!textareaEl) return;
    textareaEl.style.height = 'auto';
    textareaEl.style.height = `${Math.max(72, Math.min(textareaEl.scrollHeight, 220))}px`;
  }

  // ---------- 会话管理 ----------

  async function loadSessions(selectFirst = false) {
    let all = await ipc.chatSessionList();
    // 清理多余的空白「新对话」会话，始终只保留一个空白新对话
    const emptyNewSessions: ChatSession[] = [];
    for (const s of all) {
      if (s.title === '新对话') {
        const msgs = await ipc.chatMessageList(s.id);
        if (msgs.length === 0) emptyNewSessions.push(s);
      }
    }
    if (emptyNewSessions.length > 1) {
      for (const extra of emptyNewSessions.slice(1)) {
        await ipc.chatSessionDelete(extra.id);
      }
      all = await ipc.chatSessionList();
    }
    sessions = all;
    if (sessions.length === 0) {
      const created = await ipc.chatSessionCreate('新对话');
      sessions = [created];
      await selectSession(created.id);
    } else if (selectFirst || !currentSessionId || !sessions.some((s) => s.id === currentSessionId)) {
      await selectSession(sessions[0].id);
    }
  }

  async function selectSession(id: string) {
    if (busy) return;
    currentSessionId = id;
    errorMsg = '';
    gpu = null;
    messages = await ipc.chatMessageList(id);
    await tick();
    scrollDown();
    textareaEl?.focus();
  }

  async function createNewSession() {
    if (busy) return;
    // 如果当前会话已经是空的「新对话」，直接聚焦即可，避免产生大量空白会话
    if (currentSession && currentSession.title === '新对话' && messages.length === 0) {
      textareaEl?.focus();
      return;
    }
    const created = await ipc.chatSessionCreate('新对话');
    sessions = [created, ...sessions];
    await selectSession(created.id);
  }

  async function deleteSession(id: string, e: MouseEvent) {
    e.stopPropagation();
    if (busy && id === currentSessionId) {
      ipc.llmCancel(reqId);
    }
    await ipc.chatSessionDelete(id);
    sessions = sessions.filter((s) => s.id !== id);
    if (sessions.length === 0) {
      const created = await ipc.chatSessionCreate('新对话');
      sessions = [created];
      await selectSession(created.id);
    } else if (currentSessionId === id) {
      await selectSession(sessions[0].id);
    }
  }

  // ---------- 图片与视频多模态解析 ----------

  function fmtDuration(sec: number): string {
    if (!Number.isFinite(sec) || sec <= 0) return '00:00';
    const m = Math.floor(sec / 60);
    const s = Math.floor(sec % 60);
    return `${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
  }

  async function processImageFile(file: File): Promise<ChatAttachment> {
    const url = URL.createObjectURL(file);
    try {
      const img = await new Promise<HTMLImageElement>((resolve, reject) => {
        const el = new Image();
        el.onload = () => resolve(el);
        el.onerror = () => reject(new Error('图片加载失败'));
        el.src = url;
      });
      const maxSide = 1280;
      let w = img.naturalWidth || 800;
      let h = img.naturalHeight || 600;
      if (w > maxSide || h > maxSide) {
        const ratio = Math.min(maxSide / w, maxSide / h);
        w = Math.round(w * ratio);
        h = Math.round(h * ratio);
      }
      const canvas = document.createElement('canvas');
      canvas.width = w;
      canvas.height = h;
      const ctx = canvas.getContext('2d')!;
      ctx.drawImage(img, 0, 0, w, h);
      const dataUrl = canvas.toDataURL('image/jpeg', 0.86);
      return {
        kind: 'image',
        name: file.name || 'image.jpg',
        data_url: dataUrl,
        info: `${img.naturalWidth}×${img.naturalHeight}`,
      };
    } finally {
      URL.revokeObjectURL(url);
    }
  }

  async function processVideoFile(file: File): Promise<ChatAttachment> {
    const url = URL.createObjectURL(file);
    try {
      const video = document.createElement('video');
      video.muted = true;
      video.playsInline = true;
      video.preload = 'auto';
      video.src = url;

      await new Promise<void>((resolve, reject) => {
        const t = setTimeout(() => reject(new Error('视频解码超时')), 12000);
        video.onloadedmetadata = () => {
          clearTimeout(t);
          resolve();
        };
        video.onerror = () => {
          clearTimeout(t);
          reject(new Error('不支持该视频编码格式'));
        };
      });

      const duration = Number.isFinite(video.duration) && video.duration > 0 ? video.duration : 1;
      const vw = video.videoWidth || 640;
      const vh = video.videoHeight || 360;

      // 均匀抽取 6 帧合成 3×2 高清时序故事板，每格左上角标注时间戳
      const cols = 3;
      const rows = 2;
      const totalFrames = cols * rows;
      const cellW = 420;
      const cellH = Math.max(220, Math.min(360, Math.round((cellW * vh) / vw)));

      const board = document.createElement('canvas');
      board.width = cellW * cols;
      board.height = cellH * rows;
      const bctx = board.getContext('2d')!;
      bctx.fillStyle = '#0f172a';
      bctx.fillRect(0, 0, board.width, board.height);

      let thumbUrl = '';

      for (let i = 0; i < totalFrames; i++) {
        const ratio = totalFrames === 1 ? 0.5 : 0.06 + (i / (totalFrames - 1)) * 0.88;
        const targetTime = Math.min(Math.max(0, duration * ratio), Math.max(0, duration - 0.05));

        await new Promise<void>((resolve) => {
          const timeout = setTimeout(resolve, 1800);
          video.onseeked = () => {
            clearTimeout(timeout);
            resolve();
          };
          video.currentTime = targetTime;
        });

        const col = i % cols;
        const row = Math.floor(i / cols);
        const x = col * cellW;
        const y = row * cellH;

        bctx.drawImage(video, x + 2, y + 2, cellW - 4, cellH - 4);

        // 左上角时间戳标签
        const label = `#${i + 1} ${fmtDuration(targetTime)}`;
        bctx.font = 'bold 13px system-ui, sans-serif';
        const tw = bctx.measureText(label).width;
        bctx.fillStyle = 'rgba(0, 0, 0, 0.68)';
        bctx.fillRect(x + 8, y + 8, tw + 14, 22);
        bctx.fillStyle = '#ffffff';
        bctx.fillText(label, x + 15, y + 24);

        // 保存第一帧作为缩略图封面
        if (i === 0) {
          const tc = document.createElement('canvas');
          tc.width = 320;
          tc.height = Math.round((320 * vh) / vw);
          tc.getContext('2d')!.drawImage(video, 0, 0, tc.width, tc.height);
          thumbUrl = tc.toDataURL('image/jpeg', 0.82);
        }
      }

      const boardDataUrl = board.toDataURL('image/jpeg', 0.86);
      return {
        kind: 'video',
        name: file.name || 'video.mp4',
        data_url: boardDataUrl,
        thumb_url: thumbUrl || boardDataUrl,
        info: `${fmtDuration(duration)} · ${vw}×${vh} · 6帧时序采样`,
      };
    } finally {
      URL.revokeObjectURL(url);
    }
  }

  async function handleFiles(files: FileList | File[]) {
    const arr = Array.from(files);
    if (arr.length === 0) return;
    parsingMedia = true;
    errorMsg = '';
    try {
      for (const f of arr) {
        const isVideo =
          f.type.startsWith('video/') || /\.(mp4|webm|mov|mkv|avi|m4v)$/i.test(f.name);
        const isImage =
          f.type.startsWith('image/') || /\.(png|jpe?g|webp|gif|bmp)$/i.test(f.name);
        if (isVideo) {
          const att = await processVideoFile(f);
          pendingAttachments = [...pendingAttachments, att];
        } else if (isImage) {
          const att = await processImageFile(f);
          pendingAttachments = [...pendingAttachments, att];
        }
      }
    } catch (err) {
      errorMsg = (err as Error)?.message ?? '媒体文件解析失败';
    } finally {
      parsingMedia = false;
      if (fileInputEl) fileInputEl.value = '';
      textareaEl?.focus();
    }
  }

  function removePending(idx: number) {
    pendingAttachments = pendingAttachments.filter((_, i) => i !== idx);
  }

  function onPaste(e: ClipboardEvent) {
    const items = e.clipboardData?.items;
    if (!items) return;
    const mediaFiles: File[] = [];
    for (const item of Array.from(items)) {
      if (item.kind === 'file') {
        const f = item.getAsFile();
        if (f && (f.type.startsWith('image/') || f.type.startsWith('video/'))) {
          mediaFiles.push(f);
        }
      }
    }
    if (mediaFiles.length > 0) {
      e.preventDefault();
      handleFiles(mediaFiles);
    }
  }

  function onDrop(e: DragEvent) {
    e.preventDefault();
    dragOver = false;
    if (e.dataTransfer?.files?.length) {
      handleFiles(e.dataTransfer.files);
    }
  }

  async function handleCaptureToChat(payload: { image: string; auto_send: boolean }) {
    if (!payload?.image) return;
    try {
      const img = await new Promise<HTMLImageElement>((resolve, reject) => {
        const el = new Image();
        el.onload = () => resolve(el);
        el.onerror = () => reject(new Error('截图解码失败'));
        el.src = payload.image;
      });
      const maxSide = 1280;
      let w = img.naturalWidth || 800;
      let h = img.naturalHeight || 600;
      if (w > maxSide || h > maxSide) {
        const ratio = Math.min(maxSide / w, maxSide / h);
        w = Math.round(w * ratio);
        h = Math.round(h * ratio);
      }
      const canvas = document.createElement('canvas');
      canvas.width = w;
      canvas.height = h;
      const ctx = canvas.getContext('2d')!;
      ctx.drawImage(img, 0, 0, w, h);
      const dataUrl = canvas.toDataURL('image/jpeg', 0.88);
      const att: ChatAttachment = {
        kind: 'image',
        name: '屏幕截图.jpg',
        data_url: dataUrl,
        info: `${img.naturalWidth}×${img.naturalHeight}`,
      };
      pendingAttachments = [...pendingAttachments, att];
      await tick();
      if (payload.auto_send) {
        await sendMessage(false);
      } else {
        textareaEl?.focus();
      }
    } catch {
      /* 忽略异常截图 */
    }
  }

  // ---------- 发送与流式多轮对话（完全对齐 llama.cpp 内置 WebUI） ----------

  function buildOpenAiMessages(
    historyRecords: ChatMessageRecord[],
    searchCtx = '',
    isKbEnabled = false,
  ): ChatMessage[] {
    const out: ChatMessage[] = [];

    // 当用户点亮【🧠 知识库】时，在消息最顶端注入专属系统指令，引导 AI 建立「先查阅知识库」的思考思维
    if (isKbEnabled) {
      out.push({
        role: 'system',
        content: `【miniBot 专属知识库智能体模式已激活】
当前用户已主动开启【🧠 个人知识库】。你正在作为用户的专属私人知识库助手进行深度对话。
用户的知识库包含用户私人的工作笔记、角色扮演世界观设定、NPC人物档案、小说剧本与专属资料。

【核心思维链与回答天条（必须严格遵守）】：
1. 【知识库优先思维】：用户提问中涉及的所有人名（如“小雪”、“小雨”）、角色、称呼（如“主人”、“大管家”）、地点或设定，均为你所接入的个人私有知识库中的专属私密设定！严禁将其当作现实世界的公众人物、明星或常识去脑补！
2. 【先查阅证据再作答】：用户提问后，必须首先在深度思考中查阅与调阅系统检索出来的知识库资料。若资料中已有明确记载，直接以确凿、肯定的口吻进行回答；
3. 【引用标注】：引述任何事实、设定或细节时，必须在句末标注对应的引用角标（如 [1]、[2]）；
4. 【未直接记录时的引导】：若检索资料中未包含用户所问的特定细节，请在思考中明确，并客观说明“在当前知识库现有资料中暂未找到关于XX的直接记载”，可结合已有角色关系给出分析，切勿混淆为现实明星或无关百科！`,
      });
    }

    // 只保留最近 2 轮带图消息的原始 base64 图像，防止超长多轮对话撑满视觉上下文
    const mediaMsgIndices = historyRecords
      .map((m, idx) => (m.attachments?.length ? idx : -1))
      .filter((idx) => idx !== -1);
    const recentMediaSet = new Set(mediaMsgIndices.slice(-2));
    const lastIdx = historyRecords.length - 1;

    historyRecords.forEach((m, idx) => {
      const extraSearch = idx === lastIdx && m.role === 'user' ? searchCtx : '';
      if (m.role === 'user' && m.attachments?.length && recentMediaSet.has(idx)) {
        const parts: ({ type: 'text'; text: string } | { type: 'image_url'; image_url: { url: string } })[] = [];
        const videoNotes: string[] = [];
        for (const att of m.attachments) {
          parts.push({ type: 'image_url', image_url: { url: att.data_url } });
          if (att.kind === 'video') {
            videoNotes.push(
              `[视频附件《${att.name}》（${att.info ?? ''}）：上方图像是按时间顺序均匀抽取的 6 帧关键画面，每格左上角标有对应时间点 #1~#6]`,
            );
          }
        }
        const combinedText = [
          ...videoNotes,
          (m.content.trim() || '请详细识别并分析我上传的媒体内容。') + extraSearch,
        ].join('\n');
        parts.push({ type: 'text', text: combinedText });
        out.push({ role: 'user', content: parts });
      } else if (m.role === 'assistant') {
        const parsed = splitThink(m.content);
        const msgObj: ChatMessage = {
          role: 'assistant',
          content: parsed.answer,
        };
        if (parsed.think) {
          msgObj.reasoning_content = parsed.think;
        }
        out.push(msgObj);
      } else {
        out.push({
          role: m.role as 'user' | 'assistant',
          content: (m.content.trim() || '（已上传媒体文件）') + extraSearch,
        });
      }
    });

    return out;
  }

  async function sendMessage(force = false) {
    if (busy || parsingMedia) return;
    const text = inputText.trim();
    const atts = [...pendingAttachments];

    // 如果是显卡让道后的强制重试，直接对当前最后一轮用户消息重新发起流式推理
    if (!force) {
      if (!text && atts.length === 0) return;
      if (!currentSessionId) {
        const created = await ipc.chatSessionCreate('新对话');
        sessions = [created, ...sessions];
        currentSessionId = created.id;
      }

      inputText = '';
      pendingAttachments = [];
      if (textareaEl) textareaEl.style.height = '72px';

      const displayContent =
        text || (atts.some((a) => a.kind === 'video') ? '请识别并分析这个视频的内容' : '请识别并分析这张图片的内容');

      const msgId = await ipc.chatMessageAdd(currentSessionId, 'user', displayContent, atts);
      const userRecord: ChatMessageRecord = {
        id: msgId,
        session_id: currentSessionId,
        role: 'user',
        content: displayContent,
        attachments: atts,
        ts: Math.floor(Date.now() / 1000),
      };
      messages = [...messages, userRecord];
      // 刷新左侧会话列表标题与排序
      sessions = await ipc.chatSessionList();
      await tick();
      scrollDown();
    }

    await runStream(force);
  }

  /**
   * 重新生成指定索引处的 AI 回复：
   * 先从数据库和前端 messages 状态中彻底删除该条 AI 回复（及其后续消息），
   * 确保旧回复不会残留在上下文里污染模型，随后禁用 KV Cache 缓存重新发起推理。
   */
  async function regenerateMessage(index: number) {
    if (busy || parsingMedia) return;
    const target = messages[index];
    if (!target) return;

    // 确定需要截断删除的起始位置（如果点击的是 assistant 消息，从该条开始删；如果是 user 消息，从下一条开始删）
    const cutIdx = target.role === 'assistant' ? index : index + 1;
    const firstRemoved = messages[cutIdx];
    if (firstRemoved && currentSessionId && firstRemoved.id != null) {
      try {
        await ipc.chatMessageDeleteFrom(currentSessionId, firstRemoved.id);
      } catch {
        /* 忽略清理异常 */
      }
    }

    // 清理被删除消息关联的思考折叠状态，并截断前端上下文数组
    for (let k = cutIdx; k < messages.length; k++) {
      const mKey = String(messages[k]?.id ?? k);
      delete openThinks[mKey];
    }
    messages = messages.slice(0, cutIdx);
    stopSpeak();

    await tick();
    scrollDown();
    await runStream(false, true);
  }

  async function runStream(force = false, regenerate = false) {
    streamingOutput = '';
    streamingStats = null;
    errorMsg = '';
    gpu = null;
    streamingThinkOpen = false;
    lastSearchHits = [];
    stage = 'loading';
    reqId = uid('chat');

    // 仅当用户手动开启「🌐 联网」开关时才执行联网检索（默认关闭，不自动触发）
    let webSearchCtx = '';
    let kbSearchCtx = '';
    const lastUser = [...messages].reverse().find((m) => m.role === 'user');
    const userQuery = lastUser?.content?.trim() ?? '';
    const isDefaultMediaPrompt =
      userQuery === '请识别并分析这张图片的内容' || userQuery === '请识别并分析这个视频的内容';

    if (enableKnowledgeSearch && userQuery && !isDefaultMediaPrompt) {
      searchingKnowledge = true;
      try {
        const kbres = await ipc.kbSearch(userQuery, 15);
        lastKnowledgeHits = kbres.hits ?? [];
        kbSearchCtx = kbres.context_text ?? '';
      } catch (e) {
        console.error('知识库检索异常:', e);
      } finally {
        searchingKnowledge = false;
      }
    }

    if (enableWebSearch && userQuery && !isDefaultMediaPrompt) {
      searchingWeb = true;
      try {
        const effectiveQuery = /你能联网|可以联网|能搜索吗|会联网吗/.test(userQuery)
          ? '今日最新科技热点资讯'
          : userQuery;
        const sres = await ipc.webSearch(effectiveQuery, 5);
        lastSearchHits = sres.hits ?? [];
        webSearchCtx = formatSearchContext(sres);
      } catch {
        /* 联网异常时静默降级为离线回答 */
      } finally {
        searchingWeb = false;
      }
    }

    const openAiMsgs = buildOpenAiMessages(
      messages,
      `${kbSearchCtx}${webSearchCtx}`,
      enableKnowledgeSearch,
    );
    let latestStats: ChatStats | null = null;

    try {
      const full = await ipc.llmChat(
        reqId,
        openAiMsgs,
        (e) => {
          if (e.type === 'started') stage = 'streaming';
          if (e.type === 'delta') {
            streamingOutput += e.text;
            scrollDown();
          }
          if (e.type === 'stats') {
            const s: ChatStats = {
              tokens: e.tokens,
              tokens_per_sec: e.tokens_per_sec,
              duration_ms: e.duration_ms,
            };
            latestStats = s;
            streamingStats = s;
          }
        },
        {
          force,
          params: {
            temperature: 0.7,
            top_p: 0.95,
            top_k: 40,
            min_p: 0.05,
            max_tokens: -1,
            cache_prompt: !regenerate,
            ...buildReasoningParams(reasoningEffort),
          },
        },
      );
      const finalOut = (full || streamingOutput).trim();
      const finalStats = latestStats;
      stage = 'idle';
      streamingOutput = '';
      streamingStats = null;
      if (finalOut) {
        const hitsToSave = lastKnowledgeHits.length > 0 ? [...lastKnowledgeHits] : null;
        const aid = await ipc.chatMessageAdd(
          currentSessionId,
          'assistant',
          finalOut,
          [],
          finalStats,
          hitsToSave,
        );
        messages = [
          ...messages,
          {
            id: aid,
            session_id: currentSessionId,
            role: 'assistant',
            content: finalOut,
            attachments: [],
            stats: finalStats,
            kb_hits: hitsToSave,
            ts: Math.floor(Date.now() / 1000),
          },
        ];
        sessions = await ipc.chatSessionList();
      }
      await tick();
      scrollDown();
      textareaEl?.focus();
    } catch (err) {
      const e = err as LlmError;
      if (e?.kind === 'gpu_busy') {
        gpu = { snapshot: e.snapshot, required: e.required_mb };
        stage = 'gpu';
      } else if (e?.kind === 'cancelled') {
        const finalStats = latestStats;
        if (streamingOutput.trim()) {
          const aid = await ipc.chatMessageAdd(
            currentSessionId,
            'assistant',
            streamingOutput.trim(),
            [],
            finalStats,
          );
          messages = [
            ...messages,
            {
              id: aid,
              session_id: currentSessionId,
              role: 'assistant',
              content: streamingOutput.trim(),
              attachments: [],
              stats: finalStats,
              ts: Math.floor(Date.now() / 1000),
            },
          ];
        }
        streamingOutput = '';
        streamingStats = null;
        stage = 'idle';
      } else if (e?.kind === 'port_busy') {
        stage = 'error';
        errorMsg = `内部推理端口 ${e.port} 被占用，请在「设置 → 模型」中修改端口或点击检测自动分配空闲端口。`;
      } else {
        stage = 'error';
        errorMsg = (e as { message?: string })?.message ?? String(err);
      }
    }
  }

  function stopStream() {
    if (reqId) ipc.llmCancel(reqId);
  }

  function handleContentClick(e: MouseEvent, hits?: KbSearchHit[] | null) {
    const target = (e.target as HTMLElement)?.closest('.kb-cite-link');
    if (target && hits && hits.length > 0) {
      e.preventDefault();
      e.stopPropagation();
      const citeIdx = Number(target.getAttribute('data-cite-idx'));
      const hit = hits[citeIdx - 1];
      if (hit) {
        ipc.docOpenEditor(hit.path, hit.heading || hit.title);
        flashMsg(`📝 正在内置编辑器打开原文：《${hit.title}》`, 3500);
      }
    }
  }

  function getCitedHits(content: string, hits?: KbSearchHit[] | null): { n: number; hit: KbSearchHit }[] {
    if (!hits || hits.length === 0 || !content) return [];
    const citedSet = new Set<number>();
    const matches = content.matchAll(/\[(\d+)\]/g);
    for (const m of matches) {
      const n = parseInt(m[1], 10);
      if (n >= 1 && n <= hits.length) {
        citedSet.add(n);
      }
    }
    const out: { n: number; hit: KbSearchHit }[] = [];
    for (const n of Array.from(citedSet).sort((a, b) => a - b)) {
      out.push({ n, hit: hits[n - 1] });
    }
    return out;
  }

  // 知识库出处关联图片 Base64 缓存与全屏灯箱预览
  let kbImgCache = $state<Record<string, string>>({});
  let lightboxImg = $state<{ src: string; title: string; path: string } | null>(null);

  function loadKbImage(imgPhysPath: string) {
    if (!imgPhysPath || kbImgCache[imgPhysPath]) return;
    ipc
      .kbReadImageBase64(imgPhysPath)
      .then((dataUri) => {
        kbImgCache = { ...kbImgCache, [imgPhysPath]: dataUri };
      })
      .catch(() => {});
  }

  function preloadHitsImages(hits?: KbSearchHit[] | null) {
    if (!hits) return;
    for (const h of hits) {
      if (h.origin_image && !kbImgCache[h.origin_image]) {
        loadKbImage(h.origin_image);
      }
    }
  }

  $effect(() => {
    if (lastKnowledgeHits?.length) {
      preloadHitsImages(lastKnowledgeHits);
    }
  });

  $effect(() => {
    if (messages?.length) {
      for (const m of messages) {
        if (m.kb_hits?.length) {
          preloadHitsImages(m.kb_hits);
        }
      }
    }
  });

  function getCitedImageHits(content: string, hits?: KbSearchHit[] | null): { n: number; hit: KbSearchHit }[] {
    const cited = getCitedHits(content, hits);
    let list: { n: number; hit: KbSearchHit }[] = [];
    if (cited.some((c) => c.hit.origin_image)) {
      list = cited.filter((c) => !!c.hit.origin_image);
    } else if (hits) {
      list = hits
        .slice(0, 8)
        .map((h, i) => ({ n: i + 1, hit: h }))
        .filter((c) => !!c.hit.origin_image);
    }
    // 按 origin_image 路径去重，避免相同图片的多个切片在画廊重复出现
    const seen = new Set<string>();
    const unique: { n: number; hit: KbSearchHit }[] = [];
    for (const item of list) {
      if (item.hit.origin_image && !seen.has(item.hit.origin_image)) {
        seen.add(item.hit.origin_image);
        unique.push(item);
      }
    }
    return unique;
  }

  async function copyReply(id: string | number, raw: string) {
    const clean = splitThink(raw).answer || raw.replace(/<think>[\s\S]*?<\/think>/g, '').trim();
    await ipc.copyText(clean);
    copiedId = id;
    setTimeout(() => {
      if (copiedId === id) copiedId = '';
    }, 1200);
  }

  function toggleThink(key: string) {
    openThinks[key] = !openThinks[key];
  }

  async function togglePin() {
    pinned = !pinned;
    await win.setAlwaysOnTop(pinned).catch(() => {});
  }

  function minimizeWindow() {
    ipc.minimizeSelf();
    win.minimize().catch(() => {});
  }

  function closeWindow() {
    if (previewAttachment) {
      previewAttachment = null;
    }
    stopSpeak();
    ipc.hideSelf();
    win.hide().catch(() => {});
  }

  function fmtDate(ts: number): string {
    const d = new Date(ts * 1000);
    const now = new Date();
    if (d.toDateString() === now.toDateString()) {
      return d.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit', hour12: false });
    }
    return `${d.getMonth() + 1}/${d.getDate()}`;
  }

  async function syncModelReasoningEffort() {
    try {
      const cfg = await ipc.getSettings();
      const eff = normalizeReasoningEffort(cfg.llm.reasoning_effort, 'off');
      modelDefaultEffort = eff;
      reasoningEffort = eff;
    } catch {
      /* ignore */
    }
  }

  onMount(() => {
    openedAt = Date.now();
    syncModelReasoningEffort();
    loadSessions(true);
    checkUpdateSilent(true);
    ipc.getSettings().then((s) => {
      enableKnowledgeSearch = s.knowledge?.auto_search ?? false;
    }).catch(() => {});
    const unP = onPayload(() => {
      openedAt = Date.now();
      win.setAlwaysOnTop(pinned).catch(() => {});
      syncModelReasoningEffort();
      loadSessions(false);
      checkUpdateSilent(false);
      textareaEl?.focus();
    });
    const unC = onChatCapture((data) => {
      openedAt = Date.now();
      win.setAlwaysOnTop(pinned).catch(() => {});
      syncModelReasoningEffort();
      handleCaptureToChat(data);
    });
    const unS = onLlmStatus((s) => {
      status = s;
      if (s.reasoning_effort) {
        const eff = normalizeReasoningEffort(s.reasoning_effort, 'off');
        if (eff !== modelDefaultEffort || s.reason === '切换模型' || s.reason === '模型参数已更新') {
          modelDefaultEffort = eff;
          reasoningEffort = eff;
        }
      }
    });
    // AI 聊天主窗口在切换其他应用窗口时保持可见，不再因失焦或外部点击自动隐藏（仅通过右上角 ✕ / 最小化 / Esc 关闭）
    const unF = win.onFocusChanged(({ payload: focused }) => {
      if (focused) {
        openedAt = Date.now();
        setTimeout(() => {
          pickingFile = false;
        }, 250);
      }
    });
    ipc.llmStatus().then((s) => (status = s));
    return () => {
      unP.then((f) => f());
      unC.then((f) => f());
      unS.then((f) => f());
      unF.then((f) => f());
    };
  });
</script>

<svelte:window
  onkeydown={(e) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      if (previewAttachment) {
        previewAttachment = null;
      } else {
        closeWindow();
      }
    }
  }}
/>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="window"
  ondragover={(e) => {
    e.preventDefault();
    dragOver = true;
  }}
  ondragleave={() => (dragOver = false)}
  ondrop={onDrop}
>
  {#if dragOver}
    <div class="drop-mask">
      <div class="drop-box">
        <span class="drop-icon">🖼 🎬</span>
        <div>松开鼠标上传图片或视频进行 AI 识别</div>
      </div>
    </div>
  {/if}

  <!-- 左侧：永久聊天记录 + 底部设置图标 -->
  <aside class="sidebar">
    <div class="side-top" data-tauri-drag-region>
      <button class="new-chat-btn" onclick={createNewSession} disabled={busy}>
        <span class="plus">＋</span>
        <span>新建对话</span>
      </button>
    </div>

    <div class="session-list">
      {#each sessions as s (s.id)}
        <div
          class="session-item"
          class:active={s.id === currentSessionId}
          onclick={() => selectSession(s.id)}
          onkeydown={(e) => e.key === 'Enter' && selectSession(s.id)}
          role="button"
          tabindex="0"
        >
          <span class="s-title" title={s.title}>{s.title}</span>
          <span class="s-time">{fmtDate(s.updated_at)}</span>
          <button
            class="s-del"
            title="删除此对话"
            onclick={(e) => deleteSession(s.id, e)}
          >×</button>
        </div>
      {/each}
    </div>

    <div class="side-bottom">
      <button
        class="bottom-icon-btn"
        title="打开设置"
        onclick={() => ipc.openPage('settings', '设置')}
      >
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="3" />
          <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
        </svg>
        <span>设置</span>
        {#if updateInfo?.has_update}
          <span class="side-up-dot" title="发现新版本 v{updateInfo.latest_version}">NEW</span>
        {/if}
      </button>

      <button
        class="bottom-icon-btn ghost-sub"
        title="划词与截图识别历史（保存30天自动清理）"
        onclick={() => ipc.openPage('history', '划词与识图历史')}
      >
        <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="9" />
          <polyline points="12 7 12 12 15 15" />
        </svg>
        <span>临时记录</span>
      </button>

      <button
        class="bottom-icon-btn ghost-sub"
        title="LocalSend 局域网文件/文本快传"
        onclick={() => ipc.openLocalSend()}
      >
        <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
          <line x1="22" y1="2" x2="11" y2="13" />
          <polygon points="22 2 15 22 11 13 2 9 22 2" />
        </svg>
        <span>快传</span>
      </button>

      <span class="spacer"></span>
      <span
        class="status-dot {status?.api_enabled ? 'ready' : (status?.status ?? 'released')}"
        title={status?.api_enabled
          ? `☁️ 云端模型 API 已就绪：${status.api_model}`
          : `模型状态：${status?.status === 'ready' ? '已就绪' : status?.status === 'loading' ? '加载中' : '空闲已释放'}`}
      ></span>
    </div>
  </aside>

  <!-- 右侧：聊天显示区域 + 下方极简聊天框 -->
  <main class="chat-main">
    <header class="chat-header">
      <div class="header-drag" data-tauri-drag-region>
        <div class="chat-title" data-tauri-drag-region>
          {currentSession?.title ?? '新对话'}
        </div>
      </div>
      <div class="win-actions no-drag" role="toolbar" tabindex="-1" onmousedown={(e) => e.stopPropagation()}>
        <button
          type="button"
          class="win-btn"
          class:active={pinned}
          onclick={togglePin}
          title={pinned ? '已开启窗口置顶（点击取消置顶）' : '窗口置顶（始终悬浮在其他窗口最上方）'}
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill={pinned ? 'currentColor' : 'none'} stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="pointer-events:none">
            <line x1="12" y1="17" x2="12" y2="22" />
            <path d="M5 17h14v-1.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V6h1a2 2 0 0 0 0-4H8a2 2 0 0 0 0 4h1v4.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24Z" />
          </svg>
        </button>
        <button
          type="button"
          class="win-btn"
          onclick={minimizeWindow}
          title="最小化"
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" style="pointer-events:none">
            <line x1="5" y1="12" x2="19" y2="12" />
          </svg>
        </button>
        <button
          type="button"
          class="win-btn close"
          onclick={closeWindow}
          title="关闭窗口 (Esc)"
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" style="pointer-events:none">
            <line x1="18" y1="6" x2="6" y2="18" />
            <line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      </div>
    </header>

    {#if updateInfo?.has_update && !updateDismissed}
      <div class="update-banner">
        <div class="ub-main">
          {#if downloadingUpdate && updateDlProgress}
            <span class="ub-text">
              🚀 {updateDlProgress.message} ({updateDlProgress.percent}%)
            </span>
          {:else}
            <span class="ub-badge">新版本</span>
            <span class="ub-text">
              发现 miniBot <b>v{updateInfo.latest_version}</b>（当前 v{updateInfo.current_version}）
              {#if updateError}
                <span class="ub-err">· {updateError}</span>
              {/if}
            </span>
          {/if}
        </div>
        <div class="ub-actions">
          {#if downloadingUpdate}
            <button class="ub-btn ghost" onclick={() => ipc.appCancelUpdateDownload()}>取消</button>
          {:else}
            <button class="ub-btn primary" onclick={handleOneClickUpdate}>立即更新</button>
            <button class="ub-btn close" onclick={() => (updateDismissed = true)} title="忽略本次提示">✕</button>
          {/if}
        </div>
      </div>
    {/if}

    {#if kbActionTip}
      <div class="kb-toast">
        {kbActionTip}
      </div>
    {/if}

    <div class="chat-body" bind:this={scroller}>
      {#if messages.length === 0 && !busy && !errorMsg && !gpu}
        <div class="empty-state">
          <img class="empty-logo" src="/logo.png" alt="miniBot" />
          <div class="empty-title">有什么可以帮您？</div>
          <div class="empty-sub">支持多轮对话、深度思考，可直接粘贴或拖入图片与视频进行识别</div>
          <div class="quick-chips">
            <button class="chip-btn" onclick={openFilePicker}>🖼 上传图片 / 视频识别</button>
            <button
              class="chip-btn"
              onclick={() => {
                inputText = '帮我写一段简洁优雅的 Python 异步爬虫示例';
                textareaEl?.focus();
              }}>💻 代码编写</button
            >
            <button
              class="chip-btn"
              onclick={() => {
                inputText = '用通俗易懂的语言解释一下什么是量子纠缠';
                textareaEl?.focus();
              }}>💡 概念解析</button
            >
          </div>
        </div>
      {/if}

      {#each messages as msg, i (msg.id ?? i)}
        {#if msg.role === 'user'}
          <div class="msg-row user">
            <div class="user-bubble">
              {#if msg.attachments && msg.attachments.length > 0}
                <div class="att-grid">
                  {#each msg.attachments as att}
                    <button
                      class="att-card"
                      onclick={() => (previewAttachment = att)}
                      title="点击放大查看识别画面"
                    >
                      <img src={att.thumb_url || att.data_url} alt={att.name} />
                      {#if att.kind === 'video'}
                        <span class="att-badge">🎬 视频 · 6帧采样</span>
                      {/if}
                    </button>
                  {/each}
                </div>
              {/if}
              <div class="user-text selectable">{msg.content}</div>
            </div>
          </div>
        {:else if msg.role === 'assistant'}
          {@const parsed = splitThink(msg.content)}
          {@const key = String(msg.id ?? i)}
          <div class="msg-row assistant">
            <div class="ai-card">
              {#if parsed.think}
                <div class="think-box">
                  <button class="think-toggle" onclick={() => toggleThink(key)}>
                    <span class="think-arrow" class:open={!!openThinks[key]}>▸</span>
                    <span>已深度思考</span>
                    <span class="think-len">{parsed.think.length} 字</span>
                  </button>
                  {#if openThinks[key]}
                    <div class="think-content selectable">{parsed.think}</div>
                  {/if}
                </div>
              {/if}
              <div
                class="md selectable"
                onclick={(e) => handleContentClick(e, msg.kb_hits)}
              >
                {@html renderMarkdown(parsed.answer, msg.kb_hits)}
              </div>

              {#if getCitedImageHits(parsed.answer, msg.kb_hits).length > 0}
                <div class="kb-gallery-box">
                  <div class="kb-gallery-head">
                    <span>🖼️ 命中关联知识库图片 ({getCitedImageHits(parsed.answer, msg.kb_hits).length})</span>
                    <span class="gallery-hint">点击可全屏查看原图</span>
                  </div>
                  <div class="kb-gallery-grid">
                    {#each getCitedImageHits(parsed.answer, msg.kb_hits) as { n, hit }}
                      {#if hit.origin_image}
                        <div
                          class="kb-img-card"
                          role="button"
                          tabindex="0"
                          onclick={() => {
                            if (kbImgCache[hit.origin_image!]) {
                              lightboxImg = {
                                src: kbImgCache[hit.origin_image!],
                                title: hit.title,
                                path: hit.origin_image!,
                              };
                            }
                          }}
                          onkeydown={(e) => {
                            if (e.key === 'Enter' && kbImgCache[hit.origin_image!]) {
                              lightboxImg = {
                                src: kbImgCache[hit.origin_image!],
                                title: hit.title,
                                path: hit.origin_image!,
                              };
                            }
                          }}
                        >
                          <div class="kb-img-thumb-wrap">
                            {#if kbImgCache[hit.origin_image]}
                              <img
                                src={kbImgCache[hit.origin_image]}
                                alt={hit.title}
                                class="kb-img-thumb"
                                loading="lazy"
                              />
                            {:else}
                              <div class="kb-img-loading">
                                <span class="spinner small"></span>
                                <span>载入图片…</span>
                              </div>
                            {/if}
                            <span class="kb-img-badge">[{n}]</span>
                          </div>
                          <div class="kb-img-meta">
                            <span class="kb-img-title" title="{hit.title}">{hit.title}</span>
                            <div class="kb-img-actions" onclick={(e) => e.stopPropagation()} role="presentation">
                              <button
                                type="button"
                                class="src-act-btn"
                                title="在内置编辑器查看笔记"
                                onclick={() => {
                                  ipc.docOpenEditor(hit.path, hit.heading || hit.title);
                                  flashMsg(`📝 正在内置编辑器打开：《${hit.title}》`);
                                }}
                              >
                                📝 查看
                              </button>
                              <button
                                type="button"
                                class="src-act-btn"
                                title="在资源管理器中定位原图"
                                onclick={() => {
                                  ipc.kbRevealFile(hit.origin_image!);
                                  flashMsg(`📂 已定位原图：《${hit.title}》`);
                                }}
                              >
                                📂 原图
                              </button>
                            </div>
                          </div>
                        </div>
                      {/if}
                    {/each}
                  </div>
                </div>
              {/if}

              {#if getCitedHits(parsed.answer, msg.kb_hits).length > 0}
                <div class="kb-sources-box">
                  <div class="kb-sources-head">
                    <span>📚 知识库参考出处 ({getCitedHits(parsed.answer, msg.kb_hits).length})</span>
                  </div>
                  <div class="kb-sources-pills">
                    {#each getCitedHits(parsed.answer, msg.kb_hits) as { n, hit }}
                      <div class="kb-source-card" title="{hit.content.slice(0, 160)}…">
                        <span class="source-tag">[{n}]</span>
                        {#if hit.origin_image && kbImgCache[hit.origin_image]}
                          <button
                            type="button"
                            class="source-mini-thumb-btn"
                            title="点击放大查看图片"
                            onclick={(e) => {
                              e.stopPropagation();
                              lightboxImg = {
                                src: kbImgCache[hit.origin_image!],
                                title: hit.title,
                                path: hit.origin_image!,
                              };
                            }}
                          >
                            <img
                              src={kbImgCache[hit.origin_image]}
                              alt=""
                              class="source-mini-thumb"
                            />
                          </button>
                        {/if}
                        <span class="source-title" title="{hit.relative_path}">{hit.title}</span>
                        <span class="source-lines">L{hit.line_start}-{hit.line_end}</span>
                        <div class="source-btns">
                          <button
                            type="button"
                            class="src-act-btn"
                            onclick={() => {
                              ipc.docOpenEditor(hit.path, hit.heading || hit.title);
                              flashMsg(`📝 正在内置编辑器打开：《${hit.title}》`);
                            }}
                            title="在内置高性能 Markdown 编辑器中打开并编辑"
                          >
                            📝 查看/编辑
                          </button>
                          <button
                            type="button"
                            class="src-act-btn"
                            onclick={() => {
                              ipc.kbRevealFile(hit.path);
                              flashMsg(`📂 已在文件管理器中定位：《${hit.title}》`);
                            }}
                            title="在资源管理器中定位并选中该文件"
                          >
                            📂 定位
                          </button>
                        </div>
                      </div>
                    {/each}
                  </div>
                </div>
              {/if}
              <div class="ai-toolbar" class:visible={tts.activeId === `msg-${key}`}>
                <SpeakButton
                  id={`msg-${key}`}
                  text={parsed.answer || msg.content}
                  label="朗读"
                  title="使用 Edge TTS 朗读此条 AI 回复"
                />
                <button class="tool-btn" onclick={() => copyReply(key, msg.content)}>
                  {copiedId === key ? '✓ 已复制' : '复制'}
                </button>
                <button
                  class="tool-btn"
                  onclick={() => regenerateMessage(i)}
                  disabled={busy}
                  title="删除当前回复及后续上下文并重新生成，避免旧回复污染上下文"
                >
                  重新生成
                </button>
                {#if msg.stats && formatChatStats(msg.stats)}
                  <span class="msg-stats" title="生成速度与消耗时长">⚡ {formatChatStats(msg.stats)}</span>
                {/if}
              </div>
            </div>
          </div>
        {/if}
      {/each}

      <!-- 正在流式生成中的消息（或生成中途被显存超限让道中断的半截内容） -->
      {#if stage === 'loading' || stage === 'streaming' || (stage === 'gpu' && streamingOutput)}
        <div class="msg-row assistant">
          <div class="ai-card">
            {#if searchingWeb}
              <div class="state-line">
                <span class="spinner"></span>
                <span>🌐 正在搜索网络实时信息…</span>
              </div>
            {:else if stage === 'loading' && !streamingOutput}
              <div class="state-line">
                <span class="spinner"></span>
                <span>正在唤醒模型并思考…</span>
              </div>
            {/if}

            {#if lastSearchHits.length > 0}
              <div class="web-hits-pill">
                🌐 已检索 {lastSearchHits.length} 条网页来源：{lastSearchHits
                  .slice(0, 3)
                  .map((h) => h.title)
                  .join(' · ')}
              </div>
            {/if}

            {#if streamParsed.think || streamParsed.thinking}
              <div class="think-box streaming">
                <button
                  class="think-toggle"
                  onclick={() => (streamingThinkOpen = !streamingThinkOpen)}
                >
                  <span class="think-arrow" class:open={streamingThinkOpen}>▸</span>
                  {#if streamParsed.thinking}
                    <span class="pulse-dot"></span>
                    <span>正在深度思考…</span>
                    {#if streamParsed.think}
                      <span class="think-len">{streamParsed.think.length} 字</span>
                    {/if}
                  {:else}
                    <span>已深度思考</span>
                    <span class="think-len">{streamParsed.think.length} 字</span>
                  {/if}
                </button>
                {#if streamingThinkOpen && streamParsed.think}
                  <div class="think-content selectable">{streamParsed.think}</div>
                {/if}
              </div>
            {/if}

            {#if streamParsed.answer}
              <div
                class="md selectable"
                onclick={(e) => handleContentClick(e, lastKnowledgeHits)}
              >
                {@html renderMarkdown(streamParsed.answer, lastKnowledgeHits)}
              </div>

              {#if getCitedImageHits(streamParsed.answer, lastKnowledgeHits).length > 0}
                <div class="kb-gallery-box">
                  <div class="kb-gallery-head">
                    <span>🖼️ 命中关联知识库图片 ({getCitedImageHits(streamParsed.answer, lastKnowledgeHits).length})</span>
                    <span class="gallery-hint">点击可全屏查看原图</span>
                  </div>
                  <div class="kb-gallery-grid">
                    {#each getCitedImageHits(streamParsed.answer, lastKnowledgeHits) as { n, hit }}
                      {#if hit.origin_image}
                        <div
                          class="kb-img-card"
                          role="button"
                          tabindex="0"
                          onclick={() => {
                            if (kbImgCache[hit.origin_image!]) {
                              lightboxImg = {
                                src: kbImgCache[hit.origin_image!],
                                title: hit.title,
                                path: hit.origin_image!,
                              };
                            }
                          }}
                          onkeydown={(e) => {
                            if (e.key === 'Enter' && kbImgCache[hit.origin_image!]) {
                              lightboxImg = {
                                src: kbImgCache[hit.origin_image!],
                                title: hit.title,
                                path: hit.origin_image!,
                              };
                            }
                          }}
                        >
                          <div class="kb-img-thumb-wrap">
                            {#if kbImgCache[hit.origin_image]}
                              <img
                                src={kbImgCache[hit.origin_image]}
                                alt={hit.title}
                                class="kb-img-thumb"
                                loading="lazy"
                              />
                            {:else}
                              <div class="kb-img-loading">
                                <span class="spinner small"></span>
                                <span>载入图片…</span>
                              </div>
                            {/if}
                            <span class="kb-img-badge">[{n}]</span>
                          </div>
                          <div class="kb-img-meta">
                            <span class="kb-img-title" title="{hit.title}">{hit.title}</span>
                            <div class="kb-img-actions" onclick={(e) => e.stopPropagation()} role="presentation">
                              <button
                                type="button"
                                class="src-act-btn"
                                title="在内置编辑器查看笔记"
                                onclick={() => {
                                  ipc.docOpenEditor(hit.path, hit.heading || hit.title);
                                  flashMsg(`📝 正在内置编辑器打开：《${hit.title}》`);
                                }}
                              >
                                📝 查看
                              </button>
                              <button
                                type="button"
                                class="src-act-btn"
                                title="在资源管理器中定位原图"
                                onclick={() => {
                                  ipc.kbRevealFile(hit.origin_image!);
                                  flashMsg(`📂 已定位原图：《${hit.title}》`);
                                }}
                              >
                                📂 原图
                              </button>
                            </div>
                          </div>
                        </div>
                      {/if}
                    {/each}
                  </div>
                </div>
              {/if}

              {#if getCitedHits(streamParsed.answer, lastKnowledgeHits).length > 0}
                <div class="kb-sources-box">
                  <div class="kb-sources-head">
                    <span>📚 知识库参考出处 ({getCitedHits(streamParsed.answer, lastKnowledgeHits).length})</span>
                  </div>
                  <div class="kb-sources-pills">
                    {#each getCitedHits(streamParsed.answer, lastKnowledgeHits) as { n, hit }}
                      <div class="kb-source-card" title="{hit.content.slice(0, 160)}…">
                        <span class="source-tag">[{n}]</span>
                        {#if hit.origin_image && kbImgCache[hit.origin_image]}
                          <button
                            type="button"
                            class="source-mini-thumb-btn"
                            title="点击放大查看图片"
                            onclick={(e) => {
                              e.stopPropagation();
                              lightboxImg = {
                                src: kbImgCache[hit.origin_image!],
                                title: hit.title,
                                path: hit.origin_image!,
                              };
                            }}
                          >
                            <img
                              src={kbImgCache[hit.origin_image]}
                              alt=""
                              class="source-mini-thumb"
                            />
                          </button>
                        {/if}
                        <span class="source-title" title="{hit.relative_path}">{hit.title}</span>
                        <span class="source-lines">L{hit.line_start}-{hit.line_end}</span>
                        <div class="source-btns">
                          <button
                            type="button"
                            class="src-act-btn"
                            onclick={() => {
                              ipc.docOpenEditor(hit.path, hit.heading || hit.title);
                              flashMsg(`📝 正在内置编辑器打开：《${hit.title}》`);
                            }}
                            title="在内置高性能 Markdown 编辑器中打开并编辑"
                          >
                            📝 查看/编辑
                          </button>
                          <button
                            type="button"
                            class="src-act-btn"
                            onclick={() => {
                              ipc.kbRevealFile(hit.path);
                              flashMsg(`📂 已在文件管理器中定位：《${hit.title}》`);
                            }}
                            title="在资源管理器中定位并选中该文件"
                          >
                            📂 定位
                          </button>
                        </div>
                      </div>
                    {/each}
                  </div>
                </div>
              {/if}
            {:else if stage === 'streaming' && !streamParsed.thinking}
              <div class="state-line">
                <span class="pulse-dot"></span>
                <span>正在生成回复…</span>
              </div>
            {/if}

            {#if streamingStats && formatChatStats(streamingStats)}
              <div class="ai-toolbar visible">
                <span class="msg-stats live">⚡ {formatChatStats(streamingStats)}</span>
              </div>
            {/if}
          </div>
        </div>
      {/if}

      {#if stage === 'gpu' && gpu}
        <div class="msg-row assistant">
          <div class="ai-card">
            <GpuBusy
              snapshot={gpu.snapshot}
              required={gpu.required}
              interrupted={!!streamingOutput}
              onforce={() => runStream(true)}
              oncancel={() => {
                gpu = null;
                streamingOutput = '';
                stage = 'idle';
              }}
            />
          </div>
        </div>
      {/if}

      {#if errorMsg}
        <div class="err-banner selectable">
          <span>⚠ {errorMsg}</span>
          {#if stage === 'error'}
            <button class="retry-btn" onclick={() => sendMessage(true)}>重试</button>
          {/if}
        </div>
      {/if}
    </div>

    <!-- 底部：极简输入区 -->
    <div class="composer-wrap">
      <div class="composer">
        {#if pendingAttachments.length > 0 || parsingMedia}
          <div class="pending-bar">
            {#each pendingAttachments as att, idx}
              <div class="pending-item">
                <button
                  class="pending-thumb"
                  onclick={() => (previewAttachment = att)}
                  title={att.info ? `${att.name} (${att.info})` : att.name}
                >
                  <img src={att.thumb_url || att.data_url} alt={att.name} />
                  {#if att.kind === 'video'}
                    <span class="v-tag">🎬 6帧</span>
                  {/if}
                </button>
                <div class="pending-meta">
                  <span class="p-name">{att.name}</span>
                  {#if att.info}<span class="p-info">{att.info}</span>{/if}
                </div>
                <button class="p-remove" onclick={() => removePending(idx)} title="移除">×</button>
              </div>
            {/each}
            {#if parsingMedia}
              <div class="parsing-pill">
                <span class="spinner"></span>
                <span>正在提取媒体画面…</span>
              </div>
            {/if}
          </div>
        {/if}

        <textarea
          bind:this={textareaEl}
          bind:value={inputText}
          rows="3"
          placeholder="输入消息，Enter 发送，Shift+Enter 换行（支持直接粘贴或拖拽图片/视频）…"
          oninput={autoResizeTextarea}
          onpaste={onPaste}
          onkeydown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey) {
              e.preventDefault();
              sendMessage(false);
            } else if (e.key === 'Escape') {
              e.preventDefault();
              if (previewAttachment) previewAttachment = null;
              else if (lightboxImg) lightboxImg = null;
              else closeWindow();
            }
          }}
        ></textarea>

        <div class="composer-bar">
          <input
            bind:this={fileInputEl}
            type="file"
            accept="image/*,video/*,.mp4,.webm,.mov,.mkv,.avi,.m4v"
            multiple
            hidden
            onchange={(e) => {
              pickingFile = false;
              const files = (e.currentTarget as HTMLInputElement).files;
              if (files) handleFiles(files);
            }}
            oncancel={() => {
              pickingFile = false;
            }}
          />

          <button
            class="c-tool"
            onclick={openFilePicker}
            disabled={busy || parsingMedia}
            title="上传图片或视频（支持 MP4/WebM/MOV/PNG/JPG/WebP）"
          >
            <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48" />
            </svg>
            <span>图片/视频</span>
          </button>

          <div class="think-effort-group" class:active={reasoningEffort !== 'off'}>
            <button
              type="button"
              class="think-main-btn"
              onclick={toggleReasoningQuick}
              title={reasoningEffort === 'off' ? '点击开启高强度深度思考' : '点击关闭思考'}
            >
              <span>💭 思考强度</span>
            </button>
            <div class="think-levels" role="radiogroup" aria-label="思考强度">
              {#each REASONING_LEVELS as lv (lv.value)}
                <button
                  type="button"
                  class="think-lv-btn"
                  class:selected={reasoningEffort === lv.value}
                  onclick={() => setReasoningEffort(lv.value)}
                  title={lv.label}
                >
                  {lv.shortLabel}
                </button>
              {/each}
            </div>
          </div>

          <button
            class="c-tool"
            onclick={() => ipc.trigger('capture_chat')}
            disabled={busy || parsingMedia}
            title="框选屏幕区域截图并发送到聊天"
          >
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="6" cy="6" r="3" />
              <circle cx="6" cy="18" r="3" />
              <line x1="20" y1="4" x2="8.12" y2="15.88" />
              <line x1="14.47" y1="14.48" x2="20" y2="20" />
              <line x1="8.12" y1="8.12" x2="12" y2="12" />
            </svg>
            <span>截图</span>
          </button>

          <button
            class="c-tool"
            class:active={enableWebSearch}
            onclick={() => (enableWebSearch = !enableWebSearch)}
            title={enableWebSearch ? '已开启实时联网搜索（点击关闭）' : '默认关闭联网，点击可开启实时联网搜索'}
          >
            <span>🌐 联网</span>
          </button>

          <button
            class="c-tool"
            class:active={enableKnowledgeSearch}
            onclick={() => (enableKnowledgeSearch = !enableKnowledgeSearch)}
            title={enableKnowledgeSearch ? '已开启本地知识库检索（点击关闭）' : '默认关闭知识库检索，点击开启（将从本地笔记与图片中检索事实）'}
          >
            <span>🧠 知识库</span>
          </button>

          <span class="spacer"></span>

          {#if busy}
            <button class="send-btn stop" onclick={stopStream} title="停止生成">■</button>
          {:else}
            <button
              class="send-btn"
              onclick={() => sendMessage(false)}
              disabled={(!inputText.trim() && pendingAttachments.length === 0) || parsingMedia}
              title="发送 (Enter)"
            >↑</button>
          {/if}
        </div>
      </div>
    </div>
  </main>

  <!-- 附件大图 / 视频 6 帧故事板预览弹窗 -->
  {#if previewAttachment}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="lightbox" onclick={() => (previewAttachment = null)} role="button" tabindex="0">
      <div class="lightbox-inner" onclick={(e) => e.stopPropagation()} role="presentation">
        <div class="lightbox-head">
          <b>{previewAttachment.name}</b>
          {#if previewAttachment.info}
            <span class="lightbox-info">{previewAttachment.info}</span>
          {/if}
          <span class="spacer"></span>
          <button class="win-btn close" onclick={() => (previewAttachment = null)}>✕</button>
        </div>
        <img src={previewAttachment.data_url} alt={previewAttachment.name} />
      </div>
    </div>
  {/if}

  <!-- 知识库关联图片高清全屏预览灯箱 -->
  {#if lightboxImg}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="lightbox" onclick={() => (lightboxImg = null)} role="button" tabindex="0">
      <div class="lightbox-inner" onclick={(e) => e.stopPropagation()} role="presentation">
        <div class="lightbox-head">
          <b>🖼️ {lightboxImg.title}</b>
          <span class="lightbox-info" title={lightboxImg.path}>{lightboxImg.path}</span>
          <span class="spacer"></span>
          <button
            type="button"
            class="src-act-btn"
            style="padding: 3px 10px; font-size: 11px;"
            onclick={() => {
              ipc.kbRevealFile(lightboxImg!.path);
              flashMsg('📂 已在资源管理器中定位原图');
            }}
            title="在文件夹中定位此图片"
          >
            📂 定位原图
          </button>
          <button class="win-btn close" onclick={() => (lightboxImg = null)}>✕</button>
        </div>
        <img src={lightboxImg.src} alt={lightboxImg.title} />
      </div>
    </div>
  {/if}
</div>

<style>
  :global(html, body) {
    margin: 0;
    padding: 0;
    height: 100%;
    background: var(--bg-solid) !important;
  }
  .window {
    position: relative;
    display: flex;
    width: 100vw;
    height: 100vh;
    margin: 0;
    background: var(--bg-solid);
    border: none;
    border-radius: 0;
    box-shadow: none;
    overflow: hidden;
  }

  /* 拖拽上传遮罩 */
  .drop-mask {
    position: absolute;
    inset: 0;
    z-index: 90;
    background: rgba(15, 23, 42, 0.55);
    backdrop-filter: blur(4px);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .drop-box {
    padding: 28px 40px;
    border-radius: 16px;
    border: 2px dashed rgba(255, 255, 255, 0.5);
    background: rgba(15, 23, 42, 0.75);
    color: #fff;
    text-align: center;
    font-size: 14.5px;
    font-weight: 500;
  }
  .drop-icon {
    display: block;
    font-size: 28px;
    margin-bottom: 8px;
  }

  /* 左侧会话栏 */
  .sidebar {
    width: 236px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    border-right: 1px solid var(--border);
    background: var(--panel);
  }
  .side-top {
    padding: 14px 12px 10px;
  }
  .new-chat-btn {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    height: 36px;
    border-radius: 9px;
    border: 1px solid var(--border);
    background: var(--bg-solid);
    font-size: 13px;
    font-weight: 500;
    color: var(--fg);
    transition: background 0.12s;
  }
  .new-chat-btn:hover:not(:disabled) {
    background: var(--hover);
  }
  .plus {
    font-size: 15px;
    line-height: 1;
  }

  .session-list {
    flex: 1;
    overflow-y: auto;
    padding: 4px 8px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .session-item {
    position: relative;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 10px;
    border-radius: 8px;
    cursor: pointer;
    font-size: 12.5px;
    color: var(--fg-dim);
    transition: background 0.12s, color 0.12s;
  }
  .session-item:hover {
    background: var(--hover);
    color: var(--fg);
  }
  .session-item.active {
    background: var(--hover);
    color: var(--fg);
    font-weight: 500;
  }
  .s-title {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .s-time {
    font-size: 11px;
    opacity: 0.65;
    flex-shrink: 0;
  }
  .session-item:hover .s-time {
    display: none;
  }
  .s-del {
    display: none;
    border: none;
    background: transparent;
    color: var(--fg-dim);
    padding: 0 4px;
    font-size: 14px;
    line-height: 1;
    border-radius: 4px;
  }
  .session-item:hover .s-del {
    display: inline-flex;
  }
  .s-del:hover {
    color: var(--err);
    background: rgba(239, 68, 68, 0.12);
  }

  .side-bottom {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 8px 10px;
    border-top: 1px solid var(--border);
  }
  .bottom-icon-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 5px 8px;
    border-radius: 7px;
    border: none;
    background: transparent;
    color: var(--fg-dim);
    font-size: 12px;
  }
  .bottom-icon-btn:hover {
    background: var(--hover);
    color: var(--fg);
  }
  .status-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--fg-dim);
    opacity: 0.4;
    margin-right: 4px;
  }
  .status-dot.ready {
    background: var(--ok);
    opacity: 1;
  }
  .status-dot.loading {
    background: var(--accent-blue);
    opacity: 1;
  }
  .status-dot.yielded {
    background: var(--warn);
    opacity: 1;
  }

  /* 右侧主聊天区 */
  .chat-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .chat-header {
    height: 42px;
    padding: 0 10px 0 16px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .header-drag {
    flex: 1;
    min-width: 0;
    height: 100%;
    display: flex;
    align-items: center;
  }
  .chat-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--fg);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .win-actions {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
    -webkit-app-region: no-drag;
  }
  .win-btn {
    width: 28px;
    height: 28px;
    padding: 0;
    border: 1px solid transparent;
    border-radius: 6px;
    background: transparent;
    color: var(--fg-dim);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    -webkit-app-region: no-drag;
  }
  .win-btn:hover {
    background: var(--hover);
    color: var(--fg);
  }
  .win-btn.active {
    color: var(--fg);
    background: var(--panel);
    border-color: var(--border);
  }
  .win-btn.close:hover {
    background: rgba(239, 68, 68, 0.14);
    color: var(--err);
  }

  /* 消息列表 */
  .chat-body {
    flex: 1;
    overflow-y: auto;
    padding: 16px 20px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .empty-state {
    margin: auto;
    text-align: center;
    max-width: 420px;
    padding: 24px 12px;
  }
  .empty-logo {
    width: 58px;
    height: 58px;
    object-fit: contain;
    margin-bottom: 10px;
  }
  .empty-title {
    font-size: 16px;
    font-weight: 600;
    margin-bottom: 6px;
  }
  .empty-sub {
    font-size: 12.5px;
    color: var(--fg-dim);
    margin-bottom: 16px;
  }
  .quick-chips {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
  }
  .chip-btn {
    font-size: 12px;
    padding: 6px 12px;
    border-radius: 999px;
    background: var(--panel);
    border: 1px solid var(--border);
    color: var(--fg-dim);
  }
  .chip-btn:hover {
    color: var(--fg);
    background: var(--hover);
  }

  .msg-row {
    display: flex;
    width: 100%;
  }
  .msg-row.user {
    justify-content: flex-end;
  }
  .msg-row.assistant {
    justify-content: flex-start;
  }

  .user-bubble {
    max-width: 78%;
    padding: 10px 14px;
    border-radius: 14px 14px 4px 14px;
    background: var(--user-bubble-bg, var(--fg));
    color: var(--user-bubble-fg, var(--bg-solid));
    border: 1px solid var(--border);
    font-size: var(--app-font-size, 14px);
    line-height: 1.58;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .user-text {
    white-space: pre-wrap;
    word-break: break-word;
  }

  .att-grid {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .att-card {
    position: relative;
    padding: 0;
    border: 1px solid rgba(255, 255, 255, 0.2);
    border-radius: 8px;
    overflow: hidden;
    background: #000;
    max-width: 220px;
  }
  .att-card img {
    display: block;
    max-width: 220px;
    max-height: 140px;
    object-fit: cover;
  }
  .att-badge {
    position: absolute;
    left: 6px;
    bottom: 6px;
    padding: 2px 6px;
    border-radius: 4px;
    background: rgba(0, 0, 0, 0.72);
    color: #fff;
    font-size: 10.5px;
  }

  .ai-card {
    max-width: 92%;
    min-width: 120px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .ai-toolbar {
    display: flex;
    align-items: center;
    gap: 6px;
    opacity: 0.82;
    transition: opacity 0.15s;
  }
  .ai-card:hover .ai-toolbar,
  .ai-toolbar.visible {
    opacity: 1;
  }
  .tool-btn {
    padding: 2px 7px;
    font-size: 11px;
    border-radius: 5px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--fg-dim);
  }
  .tool-btn:hover {
    color: var(--fg);
    background: var(--hover);
  }
  .msg-stats {
    font-size: 11px;
    color: var(--fg-dim);
    opacity: 0.85;
    margin-left: 4px;
    font-variant-numeric: tabular-nums;
    user-select: none;
  }
  .msg-stats.live {
    margin-left: 0;
    color: var(--accent-blue);
  }

  /* 思考折叠框 */
  .web-hits-pill {
    font-size: 11.5px;
    color: var(--fg-dim);
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 7px;
    padding: 4px 10px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .think-box {
    border-radius: 9px;
    background: rgba(128, 128, 128, 0.05);
    border: 1px solid var(--border);
    overflow: hidden;
  }
  .think-len {
    font-size: 11px;
    opacity: 0.65;
    margin-left: 2px;
  }
  .think-toggle {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 10px;
    border: none;
    background: transparent;
    color: var(--fg-dim);
    font-size: 12px;
    text-align: left;
  }
  .think-toggle:hover {
    background: var(--hover);
    color: var(--fg);
  }
  .think-arrow {
    display: inline-block;
    transition: transform 0.15s ease;
    font-size: 11px;
  }
  .think-arrow.open {
    transform: rotate(90deg);
  }
  .think-content {
    padding: 6px 11px 9px;
    border-top: 1px dashed var(--border);
    font-size: 12.5px;
    line-height: 1.55;
    color: var(--fg-dim);
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 220px;
    overflow-y: auto;
  }

  .state-line {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--fg-dim);
    font-size: 12.5px;
    padding: 4px 0;
  }
  .spinner {
    width: 13px;
    height: 13px;
    border: 2px solid var(--border);
    border-top-color: var(--fg);
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }
  .pulse-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--fg);
    animation: pulse 1.1s ease-in-out infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @keyframes pulse {
    0%,
    100% {
      opacity: 0.25;
    }
    50% {
      opacity: 0.9;
    }
  }

  .err-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 8px 12px;
    border-radius: 9px;
    background: rgba(239, 68, 68, 0.08);
    border: 1px solid rgba(239, 68, 68, 0.25);
    color: var(--err);
    font-size: 12.5px;
  }
  .retry-btn {
    padding: 3px 9px;
    font-size: 11.5px;
    border-radius: 6px;
  }

  /* 底部极简输入框 */
  .composer-wrap {
    padding: 10px 24px 18px;
    flex-shrink: 0;
  }
  .composer {
    border: 1px solid var(--border);
    border-radius: 14px;
    background: var(--panel);
    padding: 12px 14px 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    box-shadow: 0 2px 10px rgba(0, 0, 0, 0.03);
    transition: border-color 0.15s, box-shadow 0.15s;
  }
  .composer:focus-within {
    border-color: var(--fg-dim);
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.06);
  }

  .pending-bar {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    padding-bottom: 4px;
    border-bottom: 1px dashed var(--border);
  }
  .pending-item {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 4px 8px 4px 4px;
    border-radius: 8px;
    background: var(--bg);
    border: 1px solid var(--border);
    max-width: 240px;
  }
  .pending-thumb {
    position: relative;
    padding: 0;
    border: none;
    border-radius: 5px;
    overflow: hidden;
    width: 42px;
    height: 32px;
    flex-shrink: 0;
    background: #000;
  }
  .pending-thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .v-tag {
    position: absolute;
    inset: auto 0 0 0;
    background: rgba(0, 0, 0, 0.72);
    color: #fff;
    font-size: 9px;
    text-align: center;
    line-height: 1.3;
  }
  .pending-meta {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .p-name {
    font-size: 11.5px;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .p-info {
    font-size: 10px;
    color: var(--fg-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .p-remove {
    border: none;
    background: transparent;
    color: var(--fg-dim);
    padding: 0 3px;
    font-size: 14px;
    line-height: 1;
  }
  .p-remove:hover {
    color: var(--err);
  }
  .parsing-pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--fg-dim);
    padding: 4px 8px;
  }

  textarea {
    width: 100%;
    border: none;
    background: transparent;
    padding: 2px 2px;
    font-size: 14.5px;
    line-height: 1.6;
    resize: none;
    outline: none;
    min-height: 72px;
    max-height: 220px;
  }

  .composer-bar {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .c-tool {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 3px 9px;
    height: 26px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--fg-dim);
    font-size: 11.5px;
  }
  .c-tool:hover:not(:disabled) {
    color: var(--fg);
    background: var(--hover);
  }
  .c-tool.active {
    color: var(--fg);
    background: var(--hover);
    border-color: var(--fg-dim);
    font-weight: 500;
  }
  .think-effort-group {
    display: inline-flex;
    align-items: center;
    height: 26px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: transparent;
    padding: 0 3px 0 8px;
    gap: 4px;
    transition: border-color 0.15s, background 0.15s;
  }
  .think-effort-group.active {
    border-color: color-mix(in srgb, var(--accent-blue) 45%, var(--border));
    background: color-mix(in srgb, var(--accent-blue) 6%, transparent);
  }
  .think-main-btn {
    border: none;
    background: transparent;
    color: var(--fg-dim);
    font-size: 11.5px;
    padding: 0 2px;
    height: 22px;
    display: inline-flex;
    align-items: center;
    cursor: pointer;
  }
  .think-effort-group.active .think-main-btn {
    color: var(--fg);
    font-weight: 500;
  }
  .think-levels {
    display: inline-flex;
    align-items: center;
    gap: 1px;
    background: var(--bg);
    border-radius: 999px;
    padding: 1px;
    border: 1px solid var(--border);
  }
  .think-lv-btn {
    border: none;
    background: transparent;
    color: var(--fg-dim);
    font-size: 11px;
    height: 18px;
    min-width: 20px;
    padding: 0 5px;
    border-radius: 999px;
    cursor: pointer;
    transition: all 0.12s;
  }
  .think-lv-btn:hover {
    color: var(--fg);
    background: var(--hover);
  }
  .think-lv-btn.selected {
    background: var(--fg);
    color: var(--bg-solid);
    font-weight: 600;
  }
  .spacer {
    flex: 1;
  }
  .send-btn {
    width: 28px;
    height: 28px;
    padding: 0;
    border-radius: 50%;
    border: none;
    background: var(--fg);
    color: var(--bg-solid);
    font-size: 14px;
    font-weight: 600;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .send-btn:disabled {
    opacity: 0.25;
    cursor: default;
  }
  .send-btn.stop {
    background: var(--err);
    color: #fff;
    font-size: 11px;
  }

  /* 附件大图预览弹窗 */
  .lightbox {
    position: fixed;
    inset: 0;
    z-index: 100;
    background: rgba(0, 0, 0, 0.72);
    backdrop-filter: blur(4px);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
  }
  .lightbox-inner {
    max-width: 92%;
    max-height: 92%;
    background: var(--bg-solid);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    box-shadow: var(--shadow);
  }
  .lightbox-head {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
  }
  .lightbox-info {
    font-size: 11.5px;
    color: var(--fg-dim);
  }
  .lightbox-inner img {
    max-width: 100%;
    max-height: calc(88vh - 50px);
    object-fit: contain;
    border-radius: 8px;
  }

  /* 新版本更新提醒横幅 */
  .side-up-dot {
    font-size: 9px;
    font-weight: 700;
    padding: 1px 4px;
    border-radius: 99px;
    background: var(--accent-blue);
    color: #fff;
    line-height: 1.2;
  }
  .update-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 6px 14px;
    background: color-mix(in srgb, var(--accent-blue) 10%, var(--panel));
    border-bottom: 1px solid color-mix(in srgb, var(--accent-blue) 30%, var(--border));
    font-size: 12px;
    flex-shrink: 0;
  }
  .ub-main {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    flex: 1;
  }
  .ub-badge {
    font-size: 10.5px;
    font-weight: 600;
    padding: 1px 6px;
    border-radius: 99px;
    background: var(--accent-blue);
    color: #fff;
    flex-shrink: 0;
  }
  .ub-text {
    color: var(--fg);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ub-err {
    color: var(--danger);
  }
  .ub-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }
  .ub-btn {
    padding: 2px 10px;
    font-size: 11.5px;
    border-radius: 5px;
    height: 24px;
  }
  .ub-btn.primary {
    background: var(--accent-blue);
    color: #fff;
    border-color: var(--accent-blue);
    font-weight: 500;
  }
  .ub-btn.close {
    padding: 0 5px;
    border: none;
    background: transparent;
    color: var(--fg-dim);
  }

  /* 知识库引用角标与来源卡片 */
  :global(.kb-cite-link) {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 11px;
    line-height: 1;
    font-weight: 600;
    color: var(--accent-blue);
    background: color-mix(in srgb, var(--accent-blue) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--accent-blue) 35%, transparent);
    border-radius: 4px;
    padding: 1px 5px;
    margin: 0 2px;
    cursor: pointer;
    text-decoration: none;
    vertical-align: 1px;
    transition: all 0.15s ease;
  }
  :global(.kb-cite-link:hover) {
    background: var(--accent-blue);
    color: #fff;
    transform: translateY(-1px);
    box-shadow: 0 2px 5px rgba(0, 0, 0, 0.15);
  }

  .kb-sources-box {
    margin-top: 10px;
    padding: 8px 12px;
    background: color-mix(in srgb, var(--accent-blue) 4%, var(--bg-solid));
    border: 1px solid color-mix(in srgb, var(--accent-blue) 18%, var(--border));
    border-radius: 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .kb-sources-head {
    font-size: 11.5px;
    font-weight: 600;
    color: var(--accent-blue);
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .kb-sources-pills {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .kb-source-card {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 8px;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 6px;
    font-size: 11px;
    transition: border-color 0.15s ease;
  }
  .kb-source-card:hover {
    border-color: color-mix(in srgb, var(--accent-blue) 45%, var(--border));
  }
  .source-tag {
    font-weight: 700;
    color: var(--accent-blue);
    font-size: 10.5px;
  }
  .source-title {
    max-width: 140px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--fg);
    font-weight: 500;
  }
  .source-lines {
    color: var(--fg-dim);
    font-size: 10px;
    font-family: Consolas, monospace;
  }
  .source-btns {
    display: flex;
    align-items: center;
    gap: 4px;
    margin-left: 2px;
  }
  .src-act-btn {
    font-size: 10.5px;
    padding: 1px 6px;
    border-radius: 4px;
    border: 1px solid var(--border);
    background: var(--bg-solid);
    color: var(--fg);
    cursor: pointer;
    transition: all 0.12s ease;
  }
  .src-act-btn:hover {
    background: var(--accent-blue);
    color: #fff;
    border-color: var(--accent-blue);
  }

  .source-mini-thumb-btn {
    border: none;
    background: transparent;
    padding: 0;
    margin: 0;
    cursor: zoom-in;
    display: inline-flex;
    align-items: center;
  }
  .source-mini-thumb {
    width: 18px;
    height: 18px;
    border-radius: 3px;
    object-fit: cover;
    border: 1px solid var(--border);
    cursor: zoom-in;
    transition: transform 0.12s ease;
  }
  .source-mini-thumb:hover {
    transform: scale(1.2);
  }

  .kb-gallery-box {
    margin-top: 10px;
    padding: 10px 12px;
    background: color-mix(in srgb, var(--accent-blue) 6%, var(--bg-solid));
    border: 1px solid color-mix(in srgb, var(--accent-blue) 25%, var(--border));
    border-radius: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .kb-gallery-head {
    font-size: 12px;
    font-weight: 600;
    color: var(--accent-blue);
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .gallery-hint {
    font-size: 10.5px;
    color: var(--fg-dim);
    font-weight: 400;
  }
  .kb-gallery-grid {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
  }
  .kb-img-card {
    display: flex;
    flex-direction: column;
    width: 130px;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 8px;
    overflow: hidden;
    cursor: zoom-in;
    transition: all 0.16s ease;
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.05);
  }
  .kb-img-card:hover {
    border-color: var(--accent-blue);
    transform: translateY(-2px);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.12);
  }
  .kb-img-thumb-wrap {
    position: relative;
    width: 100%;
    height: 90px;
    background: rgba(0, 0, 0, 0.04);
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
  }
  .kb-img-thumb {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
    transition: transform 0.2s ease;
  }
  .kb-img-card:hover .kb-img-thumb {
    transform: scale(1.05);
  }
  .kb-img-loading {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    font-size: 10px;
    color: var(--fg-dim);
  }
  .kb-img-badge {
    position: absolute;
    top: 4px;
    left: 4px;
    background: rgba(0, 0, 0, 0.65);
    color: #fff;
    font-size: 9.5px;
    font-weight: 700;
    padding: 1px 5px;
    border-radius: 4px;
    backdrop-filter: blur(4px);
  }
  .kb-img-meta {
    padding: 5px 6px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 4px;
    background: var(--panel);
    border-top: 1px solid var(--border);
  }
  .kb-img-title {
    font-size: 10.5px;
    font-weight: 500;
    color: var(--fg);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }
  .kb-img-actions {
    display: flex;
    align-items: center;
    gap: 3px;
  }

  .kb-toast {
    position: fixed;
    top: 50px;
    right: 20px;
    background: var(--panel);
    border: 1px solid var(--accent-blue);
    color: var(--fg);
    padding: 7px 15px;
    border-radius: 8px;
    font-size: 12px;
    font-weight: 500;
    box-shadow: 0 4px 18px rgba(0, 0, 0, 0.22);
    z-index: 999;
    animation: kbToastIn 0.2s cubic-bezier(0.16, 1, 0.3, 1);
  }
  @keyframes kbToastIn {
    from {
      opacity: 0;
      transform: translateY(-8px) scale(0.96);
    }
    to {
      opacity: 1;
      transform: translateY(0) scale(1);
    }
  }
</style>
