<script lang="ts">
  // 极简纯粹 AI 聊天/结果视图：单一职责（不跨功能切换）+ 选中内容卡片 + 思考折叠 + 流式输出
  import { onMount, tick, untrack } from 'svelte';
  import { formatSearchContext, ipc, shouldAutoWebSearch, uid } from '$lib/core/ipc';
  import { buildMessages, defaultOptions, type Agent } from '$lib/core/agents';
  import { renderMarkdown } from '$lib/core/markdown';
  import { stopSpeak } from '$lib/core/tts.svelte';
  import GpuBusy from '$lib/components/GpuBusy.svelte';
  import SpeakButton from '$lib/components/SpeakButton.svelte';
  import type { ChatMessage, GpuSnapshot, LlmError, Payload, SearchHit } from '$lib/core/types';

  let { payload, agent: initialAgent, onbusy }: { payload: Payload; agent: Agent; onbusy: (b: boolean) => void } = $props();

  type Stage = 'idle' | 'ask' | 'ocr' | 'loading' | 'streaming' | 'done' | 'error' | 'gpu';
  let agent = $state<Agent>(untrack(() => initialAgent));
  let opts = $state<Record<string, string>>(untrack(() => defaultOptions(initialAgent)));
  let enableThinking = $state<boolean>(untrack(() => !!initialAgent.thinking));
  let enableWebSearch = $state<boolean>(untrack(() => initialAgent.id === 'qa'));
  let searchingWeb = $state(false);
  let searchHits = $state<SearchHit[]>([]);
  let stage = $state<Stage>('idle');
  let output = $state('');
  let errorMsg = $state('');
  let gpu = $state<{ snapshot: GpuSnapshot; required: number } | null>(null);
  let convo: ChatMessage[] = [];
  let done = $state<{ q: string; a: string }[]>([]);
  let curQ = $state('');
  let question = $state('');
  let inputText = $state(untrack(() => payload.text ?? ''));
  let ctxExpanded = $state(false);
  let thinkOpen = $state(false);
  let reqId = '';
  let scroller: HTMLDivElement;
  let askBox = $state<HTMLTextAreaElement>();
  let copied = $state(false);

  const isLauncher = $derived(payload.origin === 'launcher');
  const canReplace = $derived(
    !!agent.replace && payload.source_hwnd !== 0 && (payload.origin === 'selection' || payload.origin === 'clipboard'),
  );
  const busy = $derived(stage === 'ocr' || stage === 'loading' || stage === 'streaming');
  $effect(() => onbusy(busy));

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

  async function start(force = false) {
    if (agent.chat && !isLauncher && !question.trim()) {
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
    curQ = agent.chat && !isLauncher ? question.trim() : '';
    if (agent.chat && isLauncher) {
      convo = [
        { role: 'system', content: agent.system },
        { role: 'user', content: input.text },
      ];
    } else {
      convo = buildMessages(agent, payload, { ...input, question: curQ, opts });
    }
    question = '';
    await stream(force);
  }

  async function stream(force = false) {
    output = '';
    errorMsg = '';
    gpu = null;
    thinkOpen = false;
    searchHits = [];
    stage = 'loading';
    reqId = uid('c');

    // 构建实际发送给模型的会话副本（支持注入实时联网搜索上下文）
    const activeConvo: ChatMessage[] = convo.map((m) => ({ ...m }));
    const queryBase = (curQ || inputText || '').trim();
    const shouldSearch =
      queryBase.length > 0 &&
      (enableWebSearch || (curQ && shouldAutoWebSearch(curQ))) &&
      (curQ.length > 0 || agent.id === 'qa' || enableWebSearch);

    if (shouldSearch) {
      let searchQuery = curQ.trim();
      if (/你能联网|可以联网|能搜索吗|会联网吗/.test(searchQuery)) {
        searchQuery = '今日最新科技热点资讯';
      } else if (!searchQuery || searchQuery.length < 6 || /这段|核心|意思|要点|分析|解释|什么/.test(searchQuery)) {
        const snippet = inputText.replace(/\s+/g, ' ').trim().slice(0, 90);
        searchQuery = [searchQuery, snippet].filter(Boolean).join(' ');
      }
      if (searchQuery) {
        searchingWeb = true;
        try {
          const sres = await ipc.webSearch(searchQuery, 5);
          if (sres && sres.hits && sres.hits.length > 0) {
            searchHits = sres.hits;
            const ctxBlock = formatSearchContext(sres);
            if (activeConvo.length > 0 && activeConvo[0].role === 'system' && typeof activeConvo[0].content === 'string') {
              activeConvo[0] = {
                role: 'system',
                content:
                  activeConvo[0].content +
                  '\n你已接入实时免费联网搜索引擎，当用户消息附带【实时联网搜索结果】时，请结合检索到的最新网页事实准确回答，切勿声称自己无法联网。',
              };
            }
            const lastIdx = activeConvo.length - 1;
            if (lastIdx >= 0 && activeConvo[lastIdx].role === 'user' && typeof activeConvo[lastIdx].content === 'string') {
              activeConvo[lastIdx] = {
                ...activeConvo[lastIdx],
                content: `${activeConvo[lastIdx].content}${ctxBlock}`,
              };
            }
          }
        } catch {
          // 联网检索失败时静默降级为本地推理
        } finally {
          searchingWeb = false;
        }
      }
    }

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
        },
        {
          force,
          params: {
            temperature: agent.temperature ?? 0.3,
            max_tokens: agent.max_tokens ?? 2048,
            chat_template_kwargs: { enable_thinking: enableThinking },
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
        errorMsg = `内部推理端口 ${e.port} 被占用，请关闭占用该端口的程序或在设置中修改端口。`;
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
    done = [...done, { q: curQ, a: output }];
    convo = [...convo, { role: 'assistant', content: cleanOutput }, { role: 'user', content: q }];
    curQ = q;
    question = '';
    await stream();
  }

  function cancel() {
    if (reqId) ipc.llmCancel(reqId);
  }

  function toggleThinking() {
    enableThinking = !enableThinking;
    if (stage === 'done') start();
  }

  function toggleWebSearch() {
    enableWebSearch = !enableWebSearch;
    if (stage === 'done') start();
  }

  onMount(() => {
    start();
    return () => {
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
        <button
          class="think-toggle"
          class:on={enableThinking}
          onclick={toggleThinking}
          disabled={busy}
          title={enableThinking ? '思考模式已开启（展示折叠思考过程）' : '点击开启深度思考'}
        >
          思考
        </button>
        <button
          class="think-toggle"
          class:on={enableWebSearch}
          onclick={toggleWebSearch}
          disabled={busy}
          title={enableWebSearch ? '联网搜索已开启（免费检索实时互联网信息）' : '点击开启免费联网搜索'}
        >
          🌐 联网
        </button>
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

    <!-- 问答模式初始快捷建议 -->
    {#if stage === 'ask' && done.length === 0}
      <div class="quick-prompts">
        <button class="qp" onclick={() => submitQuestion('请用通俗简练的话解释这段内容')}>解释核心意思</button>
        <button class="qp" onclick={() => submitQuestion('提炼这段内容的关键要点')}>提炼关键要点</button>
        <button class="qp" onclick={() => submitQuestion('分析其中的逻辑或潜在问题')}>深入分析</button>
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
          <span class="pulse-dot"></span>{searchingWeb ? '🌐 正在搜索网络实时信息…' : '正在思考与生成…'}
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

    {#if searchHits.length > 0 && stage !== 'ask'}
      <details class="search-sources">
        <summary class="search-sum">
          <span>🌐 已检索 {searchHits.length} 条网页来源</span>
        </summary>
        <div class="search-list">
          {#each searchHits as hit, hi (hi)}
            <a class="search-item" href={hit.url} target="_blank" rel="noopener noreferrer" title={hit.url}>
              <span class="search-idx">[{hi + 1}]</span>
              <span class="search-title">{hit.title}</span>
              <span class="search-engine">{hit.source}</span>
            </a>
          {/each}
        </div>
      </details>
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
    {/if}
  </div>

  <!-- 底部输入与操作融合栏 -->
  <div class="bottom-bar">
    {#if busy || (cleanOutput && stage === 'done')}
      <div class="actions">
        {#if busy}
          <button class="act-btn" onclick={cancel}>停止</button>
        {:else}
          <SpeakButton id="chat-out" text={cleanOutput} label="朗读" variant="pill" title="使用 Edge TTS 朗读 AI 回复" />
          <button class="act-btn" class:copied onclick={doCopy}>
            {copied ? '已复制' : '复制'}
          </button>
          {#if canReplace}
            <button class="act-btn primary" onclick={() => ipc.replaceText(payload.source_hwnd, cleanOutput)} title="Ctrl+Enter">替换原文</button>
          {/if}
          <button class="act-btn ghost" onclick={() => start()} title="重新生成">重试</button>
        {/if}
      </div>
    {/if}

    <div class="ask-bar">
      <textarea
        bind:this={askBox}
        bind:value={question}
        rows="1"
        placeholder={stage === 'ask' ? '输入问题，回车发送…' : '继续追问，回车发送…'}
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
  .think-toggle {
    padding: 0 7px;
    height: 22px;
    font-size: 11.5px;
    border-radius: 4px;
    background: transparent;
    color: var(--fg-dim);
    border: 1px solid var(--border);
  }
  .think-toggle.on {
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

  /* 问答快捷建议 */
  .quick-prompts {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .qp {
    font-size: 12px;
    padding: 3px 10px;
    border-radius: 12px;
    background: transparent;
    color: var(--fg-dim);
    border: 1px solid var(--border);
  }
  .qp:hover {
    color: var(--fg);
    border-color: var(--fg-dim);
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
    gap: 6px;
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

  /* 联网搜索来源面板 */
  .search-sources {
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 4px 9px;
    font-size: 11.5px;
    flex-shrink: 0;
  }
  .search-sum {
    cursor: pointer;
    color: var(--accent-blue);
    font-weight: 500;
    user-select: none;
  }
  .search-list {
    display: flex;
    flex-direction: column;
    gap: 3px;
    margin-top: 5px;
  }
  .search-item {
    display: flex;
    align-items: center;
    gap: 6px;
    text-align: left;
    padding: 3px 6px;
    border-radius: 4px;
    border: none;
    background: transparent;
    color: var(--fg);
    font-size: 11.5px;
    cursor: pointer;
    text-decoration: none;
  }
  .search-item:hover {
    background: var(--hover);
  }
  .search-idx {
    color: var(--accent-blue);
    font-weight: 600;
    flex-shrink: 0;
  }
  .search-title {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .search-engine {
    color: var(--fg-dim);
    font-size: 10.5px;
    flex-shrink: 0;
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
