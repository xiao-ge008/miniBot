<script lang="ts">
  // 极简纯粹 AI 聊天/结果视图：单一职责（不跨功能切换）+ 选中内容卡片 + 思考强度控制 + 流式输出
  import { onMount, tick, untrack } from 'svelte';
  import { formatChatStats, ipc, onLlmStatus, uid } from '$lib/core/ipc';
  import {
    buildMessages,
    buildReasoningParams,
    defaultOptions,
    normalizeReasoningEffort,
    REASONING_LEVELS,
    type Agent,
    type ReasoningEffort,
  } from '$lib/core/agents';
  import { renderMarkdown } from '$lib/core/markdown';
  import { stopSpeak } from '$lib/core/tts.svelte';
  import GpuBusy from '$lib/components/GpuBusy.svelte';
  import SpeakButton from '$lib/components/SpeakButton.svelte';
  import type { ChatMessage, ChatStats, GpuSnapshot, LlmError, Payload } from '$lib/core/types';

  let { payload, agent: initialAgent, onbusy }: { payload: Payload; agent: Agent; onbusy: (b: boolean) => void } = $props();

  type Stage = 'idle' | 'ask' | 'ocr' | 'loading' | 'streaming' | 'done' | 'error' | 'gpu';
  let agent = $state<Agent>(untrack(() => initialAgent));
  let opts = $state<Record<string, string>>(untrack(() => defaultOptions(initialAgent)));
  let reasoningEffort = $state<ReasoningEffort>(
    untrack(() => initialAgent.reasoning_effort ?? 'off'),
  );
  let userChangedReasoning = false;
  let effortSynced = false;
  let stage = $state<Stage>('idle');
  let output = $state('');
  let errorMsg = $state('');
  let gpu = $state<{ snapshot: GpuSnapshot; required: number } | null>(null);
  let convo: ChatMessage[] = [];
  let done = $state<{ q: string; a: string; stats?: ChatStats | null; convoSnap: ChatMessage[] }[]>([]);
  let currentStats = $state<ChatStats | null>(null);
  let curQ = $state('');
  let question = $state('');
  let inputText = $state(untrack(() => payload.text ?? ''));
  let ctxExpanded = $state(false);
  let thinkOpen = $state(false);
  let reqId = '';
  let scroller: HTMLDivElement;
  let askBox = $state<HTMLTextAreaElement>();
  let copied = $state(false);

  const isPolish = $derived(agent.id === 'polish');
  const isImageQa = $derived(!!payload.image && !inputText);
  const canReplace = $derived(
    !!agent.replace && payload.source_hwnd !== 0 && (payload.origin === 'selection' || payload.origin === 'clipboard'),
  );
  const busy = $derived(stage === 'ocr' || stage === 'loading' || stage === 'streaming');
  $effect(() => onbusy(busy));

  const QUICK_POLISH: { label: string; prompt: string }[] = [
    { label: '✨ 流畅润色', prompt: '润色词句使其通顺自然、表达地道流畅，保持原意不变' },
    { label: '🏛️ 更正式专业', prompt: '改写为更正式、得体、专业的书面语或商务表达风格' },
    { label: '💬 更口语自然', prompt: '改写为亲切自然的日常口语化表达，通俗易懂' },
    { label: '✂️ 精简凝练', prompt: '删繁就简，去除冗余啰嗦词句，保留核心意思并压缩篇幅' },
    { label: '📝 扩充细节', prompt: '适当丰富细节与层次过渡，使表达更饱满有力' },
    { label: '🔧 纠错改病句', prompt: '仅修正错别字、语法语病与标点符号，最大限度保留原文原貌' },
  ];

  const QUICK_TEXT_QA: { label: string; prompt: string }[] = [
    { label: '💡 解释核心意思', prompt: '请用通俗简练的话解释这段内容的核心意思' },
    { label: '📋 提炼关键要点', prompt: '请分条提炼这段内容的关键要点' },
    { label: '🔍 分析逻辑与问题', prompt: '请深入分析这段内容的逻辑结构、背景或潜在问题' },
    { label: '📝 一句话总结', prompt: '请用一句话精准概括这段内容的核心结论' },
    { label: '🌐 翻译并解读术语', prompt: '请翻译这段内容，并解释其中的重点词汇或专业术语' },
  ];

  const QUICK_IMAGE_QA: { label: string; prompt: string }[] = [
    { label: '🖼️ 解读图片内容', prompt: '请详细描述并解读这张截图中的核心内容与关键信息' },
    { label: '📝 提取并整理文字', prompt: '请提取图片中的全部文字，并按清晰的层级排版整理' },
    { label: '💡 解答图中问题/报错', prompt: '请分析图片中的问题、报错或题目，并给出详细解答与解决步骤' },
    { label: '📊 总结图表/数据要点', prompt: '请提炼图片中图表、表格或界面的关键数据与核心结论' },
    { label: '🌐 翻译图中文字', prompt: '请将图片中的外文内容准确翻译为简体中文' },
  ];

  const quickOptions = $derived(isPolish ? QUICK_POLISH : isImageQa ? QUICK_IMAGE_QA : QUICK_TEXT_QA);
  const askGuideTitle = $derived(
    isPolish
      ? '✍️ 请选择润色优化方向，或在下方输入您的修改意见：'
      : isImageQa
        ? '💬 请选择针对当前截图的提问方向，或在下方输入您的问题：'
        : '💬 请选择针对当前段落的提问方向，或在下方输入您的问题：',
  );
  const askPlaceholder = $derived(
    stage === 'ask'
      ? isPolish
        ? '输入您的具体润色意见（如：语气委婉些 / 改成邮件格式），回车或点发送…'
        : isImageQa
          ? '输入针对当前截图想问的问题，回车或点击发送…'
          : '输入针对当前段落想问的问题，回车或点击发送…'
      : isPolish
        ? '继续输入修改意见，回车或点击发送…'
        : '继续追问，回车或点击发送…',
  );

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

  const currentParsed = $derived(splitThink(output));
  const cleanOutput = $derived(currentParsed.answer || output.replace(/<think>[\s\S]*?(<\/think>|$)/g, '').trim());

  async function doCopy() {
    if (!cleanOutput) return;
    await ipc.copyText(cleanOutput);
    copied = true;
    setTimeout(() => (copied = false), 1200);
  }

  async function prepareText(): Promise<{ text: string; image: string | null } | null> {
    let text = inputText;
    let image = payload.image ?? null;
    if (image && !agent.vision) {
      stage = 'ocr';
      try {
        const r = await ipc.ocrImage(image);
        text = r.text.trim();
        inputText = text;
        if (text) {
          image = null;
        } else {
          text = '（请直接识别图片中的文字内容并按要求处理）';
        }
      } catch {
        text = '（请直接识别图片中的文字内容并按要求处理）';
      }
    }
    return { text, image };
  }

  async function ensureModelDefaultEffort() {
    if (effortSynced) return;
    effortSynced = true;
    try {
      const cfg = await ipc.getSettings();
      if (!userChangedReasoning) {
        reasoningEffort = normalizeReasoningEffort(cfg.llm.reasoning_effort, 'off');
      }
    } catch {
      /* ignore */
    }
  }

  async function start(force = false) {
    await ensureModelDefaultEffort();
    const qText = question.trim() || curQ;
    if (agent.chat && !qText) {
      stage = 'ask';
      output = '';
      done = [];
      await tick();
      askBox?.focus();
      return;
    }
    const input = await prepareText();
    if (!input) return;
    done = [];
    curQ = agent.chat ? qText : '';
    convo = buildMessages(agent, payload, { ...input, question: curQ, opts });
    question = '';
    await stream(force);
  }

  /**
   * 重新生成当前最后一轮 AI 回复：
   * 清空当前已生成的内容（当前轮次尚未推入 convo，因此 convo 上下文天然不包含本次旧回复），
   * 并传入 regenerate=true（cache_prompt: false）彻底避免旧回复污染上下文与 KV 缓存。
   */
  async function regenerateCurrent() {
    if (busy || stage === 'ask' || convo.length === 0) return;
    stopSpeak();
    await stream(false, true);
  }

  /**
   * 重新生成历史第 index 轮 AI 回复：
   * 删除该轮及之后的所有问答历史，将 convo 回滚至该轮提问时的干净快照，避免旧回复污染上下文。
   */
  async function regenerateTurn(index: number) {
    if (busy) return;
    const target = done[index];
    if (!target) return;
    stopSpeak();
    convo = target.convoSnap.map((m) => ({ ...m }));
    curQ = target.q;
    done = done.slice(0, index);
    await tick();
    await stream(false, true);
  }

  async function stream(force = false, regenerate = false) {
    output = '';
    errorMsg = '';
    gpu = null;
    thinkOpen = false;
    currentStats = null;
    stage = 'loading';
    reqId = uid('c');

    const activeConvo: ChatMessage[] = convo.map((m) => ({ ...m }));

    try {
      const full = await ipc.llmChat(
        reqId,
        activeConvo,
        (e) => {
          if (e.type === 'started') stage = 'streaming';
          if (e.type === 'delta') {
            output += e.text;
            scrollDown();
          }
          if (e.type === 'stats') {
            currentStats = {
              tokens: e.tokens,
              tokens_per_sec: e.tokens_per_sec,
              duration_ms: e.duration_ms,
            };
          }
        },
        {
          force,
          params: {
            temperature: agent.temperature ?? 0.7,
            top_p: 0.95,
            top_k: 40,
            min_p: 0.05,
            max_tokens: agent.max_tokens ?? -1,
            cache_prompt: !regenerate,
            ...buildReasoningParams(reasoningEffort),
          },
        },
      );
      output = full || output;
      stage = 'done';
      saveHistory();
      await tick();
      askBox?.focus();
    } catch (err) {
      const e = err as LlmError;
      if (e?.kind === 'gpu_busy') {
        gpu = { snapshot: e.snapshot, required: e.required_mb };
        stage = 'gpu';
      } else if (e?.kind === 'cancelled') {
        stage = 'done';
      } else if (e?.kind === 'port_busy') {
        stage = 'error';
        errorMsg = `内部推理端口 ${e.port} 被占用，请在「设置 → 模型」中修改端口或点击检测自动分配空闲端口。`;
      } else {
        stage = 'error';
        errorMsg = (e as { message?: string })?.message ?? String(err);
      }
    }
  }

  function scrollDown() {
    requestAnimationFrame(() => scroller && (scroller.scrollTop = scroller.scrollHeight));
  }

  function saveHistory() {
    ipc
      .historyAdd(
        {
          action: agent.id,
          kind: payload.kind,
          input: [curQ, inputText || (payload.image ? '[图片]' : '')].filter(Boolean).join('\n\n'),
          output: cleanOutput,
          source_app: payload.source_app ?? null,
        },
        payload.origin === 'capture' ? payload.image : null,
      )
      .catch(() => {});
  }

  async function submitQuestion(preset?: string) {
    const q = (preset ?? question).trim();
    if (!q || busy) return;
    question = q;
    if (stage === 'ask') {
      await start();
      return;
    }
    done = [
      ...done,
      {
        q: curQ,
        a: output,
        stats: currentStats ? { ...currentStats } : null,
        convoSnap: convo.map((m) => ({ ...m })),
      },
    ];
    const prevAssistant: ChatMessage = {
      role: 'assistant',
      content: cleanOutput,
    };
    if (currentParsed.think) {
      prevAssistant.reasoning_content = currentParsed.think;
    }
    convo = [...convo, prevAssistant, { role: 'user', content: q }];
    curQ = q;
    question = '';
    await stream();
  }

  function cancel() {
    if (reqId) ipc.llmCancel(reqId);
  }

  function onReasoningChange() {
    userChangedReasoning = true;
    if (stage === 'done') regenerateCurrent();
  }

  onMount(() => {
    start();
    const unS = onLlmStatus((s) => {
      if (!userChangedReasoning && s.reasoning_effort) {
        reasoningEffort = normalizeReasoningEffort(s.reasoning_effort, 'off');
      }
    });
    return () => {
      unS.then((f) => f());
      cancel();
      stopSpeak();
    };
  });
</script>

<div class="chat">
  <!-- 主对话滚动区 -->
  <div class="scroll" bind:this={scroller}>
    <!-- 选中内容卡片（融合当前模式选项与思考开关） -->
    <div class="ctx-card">
      <div class="ctx-head">
        <span class="ctx-tag">{payload.image && !inputText ? '选中截图' : '选中内容'}</span>
        {#if inputText}
          <span class="ctx-len">{inputText.length} 字</span>
          {#if inputText.length > 160}
            <button class="ctx-more" onclick={() => (ctxExpanded = !ctxExpanded)}>
              {ctxExpanded ? '收起' : '展开'}
            </button>
          {/if}
          <SpeakButton id="chat-ctx" text={inputText} title="使用 Edge TTS 朗读选中内容" />
        {/if}
        <span class="spacer"></span>
        {#each agent.options ?? [] as o (o.key)}
          <select class="opt-sel" bind:value={opts[o.key]} onchange={() => start()} disabled={busy} title={o.label}>
            {#each o.choices as c (c)}<option value={c}>{c}</option>{/each}
          </select>
        {/each}
        <select
          class="opt-sel think-sel"
          class:on={reasoningEffort !== 'off'}
          bind:value={reasoningEffort}
          onchange={onReasoningChange}
          disabled={busy}
          title="调节思考强度（对齐 llama.cpp 聊天框）"
        >
          {#each REASONING_LEVELS as lv (lv.value)}
            <option value={lv.value}>💭 思考：{lv.shortLabel}</option>
          {/each}
        </select>
      </div>

      {#if payload.image}
        <div class="img-wrap">
          <img class="src-img" src={payload.image} alt="" />
        </div>
      {/if}
      {#if inputText}
        <div class="ctx-body selectable" class:clamped={!ctxExpanded && inputText.length > 160}>
          {inputText}
        </div>
      {/if}
    </div>

    <!-- 润色 / 问答模式初始方向与快捷选项 -->
    {#if stage === 'ask' && done.length === 0}
      <div class="ask-guide">
        <div class="ask-guide-title">{askGuideTitle}</div>
        <div class="quick-prompts">
          {#each quickOptions as opt (opt.label)}
            <button class="qp" onclick={() => submitQuestion(opt.prompt)} disabled={busy}>
              {opt.label}
            </button>
          {/each}
        </div>
      </div>
    {:else if isPolish && stage !== 'ask'}
      <!-- 润色已出结果后，仍可一键切换其他润色方向（自动重置原文上下文，不污染） -->
      <div class="quick-prompts compact">
        {#each QUICK_POLISH as opt (opt.label)}
          <button
            class="qp sm"
            class:active={curQ === opt.prompt}
            onclick={() => {
              if (busy) return;
              question = opt.prompt;
              curQ = opt.prompt;
              done = [];
              start();
            }}
            disabled={busy}
          >
            {opt.label}
          </button>
        {/each}
      </div>
    {/if}

    <!-- 历史问答轮次 -->
    {#each done as t, i (i)}
      {@const p = splitThink(t.a)}
      <div class="turn">
        {#if t.q}
          <div class="q-row">
            <span class="q-bubble selectable">{t.q}</span>
          </div>
        {/if}
        {#if p.think}
          <details class="think-box">
            <summary class="think-sum">
              <span class="think-arrow">▸</span>
              <span>思考过程</span>
              <span class="think-meta">{p.think.length} 字</span>
            </summary>
            <div class="think-body selectable">{p.think}</div>
          </details>
        {/if}
        {#if p.answer}
          <div class="md out">{@html renderMarkdown(p.answer)}</div>
          <div class="turn-tools">
            <SpeakButton id={`turn-${i}`} text={p.answer} label="朗读" title="朗读此条 AI 回复" />
            <button
              class="turn-regen"
              onclick={() => regenerateTurn(i)}
              disabled={busy}
              title="删除此轮及后续问答上下文并重新生成"
            >
              重新生成
            </button>
            {#if t.stats && formatChatStats(t.stats)}
              <span class="turn-stats">⚡ {formatChatStats(t.stats)}</span>
            {/if}
          </div>
        {/if}
      </div>
    {/each}

    <!-- 当前轮次问题 -->
    {#if curQ && stage !== 'ask'}
      <div class="q-row">
        <span class="q-bubble selectable">{curQ}</span>
      </div>
    {/if}

    {#if stage === 'ocr'}
      <div class="skeleton-wrap">
        <div class="hint"><span class="pulse-dot"></span>正在提取图中文字…</div>
      </div>
    {:else if stage === 'loading'}
      <div class="skeleton-wrap">
        <div class="hint">
          <span class="pulse-dot"></span>正在思考与生成…
        </div>
      </div>
    {:else if stage === 'gpu' && gpu}
      <GpuBusy
        snapshot={gpu.snapshot}
        required={gpu.required}
        interrupted={!!output}
        onforce={() => stream(true)}
        oncancel={() => ipc.hideSelf()}
      />
    {:else if stage === 'error'}
      <div class="err">{errorMsg}</div>
    {/if}

    {#if output && stage !== 'ask'}
      <!-- 思考折叠面板 -->
      {#if currentParsed.think || currentParsed.thinking}
        <details class="think-box" bind:open={thinkOpen}>
          <summary class="think-sum">
            <span class="think-arrow">▸</span>
            {#if currentParsed.thinking && stage === 'streaming'}
              <span class="pulse-dot"></span>
              <span>思考中…</span>
            {:else}
              <span>思考过程</span>
              <span class="think-meta">{currentParsed.think.length} 字</span>
            {/if}
          </summary>
          <div class="think-body selectable">
            {currentParsed.think}{#if currentParsed.thinking && stage === 'streaming'}<span class="caret">▍</span>{/if}
          </div>
        </details>
      {/if}

      <!-- 流式正文输出 -->
      {#if currentParsed.answer || (!currentParsed.thinking && !currentParsed.think)}
        <div class="md out">
          {@html renderMarkdown(currentParsed.answer)}{#if stage === 'streaming' && !currentParsed.thinking}<span class="caret">▍</span>{/if}
        </div>
      {/if}

      {#if currentStats && formatChatStats(currentStats)}
        <div class="turn-stats-row">
          <span class="turn-stats" class:live={stage === 'streaming'}>⚡ {formatChatStats(currentStats)}</span>
        </div>
      {/if}
    {/if}
  </div>

  <!-- 底部输入与操作融合栏 -->
  <div class="bottom-bar">
    {#if busy || (cleanOutput && stage === 'done') || stage === 'error'}
      <div class="actions">
        {#if busy}
          <button class="act-btn" onclick={cancel}>停止</button>
        {:else}
          {#if cleanOutput}
            <SpeakButton id="chat-out" text={cleanOutput} label="朗读" variant="pill" title="使用 Edge TTS 朗读 AI 回复" />
            <button class="act-btn" class:copied onclick={doCopy}>
              {copied ? '已复制' : '复制'}
            </button>
            {#if canReplace}
              <button class="act-btn primary" onclick={() => ipc.replaceText(payload.source_hwnd, cleanOutput)} title="Ctrl+Enter">替换原文</button>
            {/if}
          {/if}
          <button
            class="act-btn ghost"
            onclick={regenerateCurrent}
            title="删除当前回复并重新生成，避免旧回复污染上下文"
          >
            重新生成
          </button>
        {/if}
      </div>
    {/if}

    <div class="ask-bar">
      <textarea
        bind:this={askBox}
        bind:value={question}
        rows="1"
        placeholder={askPlaceholder}
        onkeydown={(e) => {
          if (e.key === 'Enter' && !e.shiftKey && !e.ctrlKey) {
            e.preventDefault();
            submitQuestion();
          }
        }}
        disabled={busy}
      ></textarea>
      <button
        class="send-btn"
        onclick={() => submitQuestion()}
        disabled={busy || !question.trim()}
        title="发送 (Enter)"
      >
        ↑
      </button>
    </div>
  </div>
</div>

<svelte:window
  onkeydown={(e) => {
    if (e.key === 'Enter' && e.ctrlKey && canReplace && cleanOutput && !busy) ipc.replaceText(payload.source_hwnd, cleanOutput);
  }}
/>

<style>
  .chat {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  /* 主滚动区 */
  .scroll {
    flex: 1;
    overflow: auto;
    padding: 10px 14px 12px;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  /* 选中内容卡片 */
  .ctx-card {
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 6px 10px;
    flex-shrink: 0;
  }
  .ctx-head {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11.5px;
    color: var(--fg-dim);
  }
  .ctx-tag {
    font-weight: 550;
  }
  .ctx-len {
    opacity: 0.7;
  }
  .ctx-more {
    border: none;
    background: transparent;
    color: var(--accent-blue);
    font-size: 11.5px;
    padding: 0 2px;
  }
  .spacer {
    flex: 1;
  }
  .opt-sel {
    padding: 1px 6px;
    font-size: 11.5px;
    border-radius: 4px;
    height: 22px;
    background: var(--panel-elevated);
  }
  .think-sel.on {
    color: var(--accent-blue);
    border-color: color-mix(in srgb, var(--accent-blue) 45%, transparent);
    background: color-mix(in srgb, var(--accent-blue) 8%, transparent);
    font-weight: 500;
  }
  .img-wrap {
    margin-top: 6px;
    display: flex;
    justify-content: center;
  }
  .src-img {
    max-width: 100%;
    max-height: 110px;
    object-fit: contain;
    border-radius: 4px;
    display: block;
  }
  .ctx-body {
    margin-top: 5px;
    font-size: 0.92em;
    line-height: 1.55;
    color: var(--fg);
    opacity: 0.88;
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 150px;
    overflow: auto;
  }
  .ctx-body.clamped {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  /* 润色与问答快捷建议 */
  .ask-guide {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px 10px;
    background: var(--panel);
    border: 1px dashed var(--border);
    border-radius: 6px;
  }
  .ask-guide-title {
    font-size: 12px;
    color: var(--fg-dim);
    font-weight: 500;
  }
  .quick-prompts {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .quick-prompts.compact {
    gap: 5px;
  }
  .qp {
    font-size: 12px;
    padding: 4px 11px;
    border-radius: 14px;
    background: var(--bg-solid);
    color: var(--fg);
    border: 1px solid var(--border);
    transition: all 0.14s ease;
  }
  .qp.sm {
    font-size: 11px;
    padding: 2px 8px;
    color: var(--fg-dim);
  }
  .qp.active {
    color: var(--accent-blue);
    border-color: color-mix(in srgb, var(--accent-blue) 50%, transparent);
    background: color-mix(in srgb, var(--accent-blue) 10%, transparent);
    font-weight: 500;
  }
  .qp:hover:not(:disabled) {
    color: var(--accent-blue);
    border-color: color-mix(in srgb, var(--accent-blue) 50%, var(--fg-dim));
    background: var(--hover);
  }

  /* 问答气泡 */
  .turn {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-bottom: 8px;
    border-bottom: 1px dashed var(--border);
  }
  .turn-tools {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .turn-regen {
    padding: 1px 7px;
    font-size: 11px;
    border-radius: 4px;
    background: transparent;
    color: var(--fg-dim);
    border: 1px solid var(--border);
  }
  .turn-regen:hover:not(:disabled) {
    color: var(--fg);
    border-color: var(--fg-dim);
    background: var(--hover);
  }
  .turn-stats-row {
    display: flex;
    align-items: center;
    padding-top: 2px;
  }
  .turn-stats {
    font-size: 11px;
    color: var(--fg-dim);
    opacity: 0.78;
    font-variant-numeric: tabular-nums;
    user-select: none;
  }
  .turn-stats.live {
    color: var(--accent-blue);
    opacity: 0.92;
  }
  .q-row {
    display: flex;
    justify-content: flex-end;
  }
  .q-bubble {
    background: var(--panel);
    border: 1px solid var(--border);
    color: var(--fg);
    padding: 5px 10px;
    border-radius: 8px 8px 2px 8px;
    font-size: 0.95em;
    line-height: 1.5;
    max-width: 85%;
    white-space: pre-wrap;
    word-break: break-word;
  }

  /* 可折叠思考面板 */
  .think-box {
    border-left: 2px solid var(--border);
    padding-left: 9px;
    margin: 2px 0;
  }
  .think-sum {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
    color: var(--fg-dim);
    cursor: pointer;
    user-select: none;
    list-style: none;
    padding: 2px 0;
  }
  .think-sum::-webkit-details-marker {
    display: none;
  }
  .think-arrow {
    display: inline-block;
    font-size: 10px;
    transition: transform 0.15s ease;
  }
  .think-box[open] .think-arrow {
    transform: rotate(90deg);
  }
  .think-meta {
    font-size: 11px;
    opacity: 0.65;
  }
  .think-body {
    margin-top: 4px;
    font-size: 0.88em;
    line-height: 1.6;
    color: var(--fg-dim);
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 180px;
    overflow: auto;
  }

  /* 状态与流式输出 */
  .skeleton-wrap {
    padding: 4px 0;
  }
  .hint {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--fg-dim);
    font-size: 12px;
  }
  .pulse-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent-blue);
    animation: pulse 1s infinite ease-in-out;
  }
  .err {
    color: var(--danger);
    white-space: pre-wrap;
    font-size: 12.5px;
    padding: 8px 10px;
    background: rgba(239, 68, 68, 0.06);
    border: 1px solid rgba(239, 68, 68, 0.2);
    border-radius: 6px;
  }
  .caret {
    display: inline-block;
    margin-left: 2px;
    animation: pulse 0.8s infinite;
    color: var(--accent-blue);
    font-size: 12px;
  }
  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.25; }
  }

  /* 底部融合区 */
  .bottom-bar {
    border-top: 1px solid var(--border);
    flex-shrink: 0;
    background: var(--bg);
  }
  .actions {
    display: flex;
    gap: 5px;
    padding: 5px 12px 0;
    align-items: center;
  }
  .act-btn {
    padding: 2px 9px;
    font-size: 11.5px;
    border-radius: 4px;
  }
  button.copied {
    color: var(--ok);
    border-color: var(--ok);
  }
  .ask-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px 8px;
  }
  .ask-bar textarea {
    flex: 1;
    resize: none;
    height: 30px;
    max-height: 76px;
    padding: 5px 9px;
    font-size: 12.5px;
    line-height: 1.4;
    border-radius: 6px;
  }
  .send-btn {
    width: 30px;
    height: 30px;
    padding: 0;
    border-radius: 6px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 13px;
    font-weight: 600;
    background: var(--accent);
    color: var(--accent-fg);
    border: none;
    flex-shrink: 0;
  }
  .send-btn:disabled {
    opacity: 0.3;
  }
</style>
