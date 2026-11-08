<script lang="ts">
  // 识别文字视图（参考 eSearch + Snippai）：系统 OCR 瞬时识别并自动复制、支持合并换行、分句点击复制、一键转交翻译/AI
  import { onMount } from 'svelte';
  import { ipc } from '$lib/core/ipc';
  import { execute } from '$lib/core/actions';
  import { stopSpeak } from '$lib/core/tts.svelte';
  import SpeakButton from '$lib/components/SpeakButton.svelte';
  import type { OcrLine, Payload } from '$lib/core/types';

  let { payload }: { payload: Payload } = $props();
  let text = $state('');
  let lines = $state<OcrLine[]>([]);
  let loading = $state(true);
  let err = $state('');
  let copied = $state(false);
  let autoCopied = $state(false);
  let lineMode = $state(false);
  let ms = $state(0);

  onMount(async () => {
    const t0 = performance.now();
    try {
      const r = await ipc.ocrImage(payload.image!);
      text = r.text;
      lines = r.lines ?? [];
      ms = Math.round(performance.now() - t0);
      if (text.trim()) {
        await ipc.copyText(text);
        autoCopied = true;
      }
      ipc
        .historyAdd({ action: 'ocr', kind: 'image', input: '[图片]', output: text, source_app: payload.source_app ?? null }, payload.image)
        .catch(() => {});
    } catch (e) {
      err = String(e);
    } finally {
      loading = false;
    }
  });

  async function copy(val = text) {
    if (!val) return;
    await ipc.copyText(val);
    copied = true;
    setTimeout(() => (copied = false), 1200);
  }

  /** 参考 eSearch：合并多余换行（中英文智能拼接段落） */
  function mergeLines() {
    text = text
      .replace(/([^\n])\n([^\n])/g, (_, a, b) => {
        const isAscii = /[a-zA-Z0-9,.;:?!]/.test(a) && /[a-zA-Z0-9]/.test(b);
        return isAscii ? `${a} ${b}` : `${a}${b}`;
      })
      .trim();
    copy(text);
  }

  function send(action: string) {
    execute(action, { ...payload, kind: 'text', image: null, text, meta: {} });
  }
</script>

<div class="ocr">
  {#if payload.image}
    <div class="top-bar">
      <img class="thumb" src={payload.image} alt="截图预览" />
      <div class="top-meta">
        {#if loading}
          <span class="badge">🔤 正在识别图中文字…</span>
        {:else if err}
          <span class="badge danger">识别失败</span>
        {:else}
          <span class="badge ok">{autoCopied ? '✓ 已识别并自动复制' : '✓ 识别完成'}</span>
          <span class="sub">{text.length} 字 · {lines.length} 行 · {ms} ms</span>
        {/if}
      </div>
      {#if !loading && !err && text}
        <button class="ghost sm" onclick={mergeLines} title="去除多余换行并重新复制">合并换行</button>
        {#if lines.length > 1}
          <button class="ghost sm" class:on={lineMode} onclick={() => (lineMode = !lineMode)} title="按行查看与单行复制">逐行</button>
        {/if}
      {/if}
    </div>
  {/if}

  {#if loading}
    <div class="sk-box">
      <div class="sk w85"></div>
      <div class="sk w65"></div>
      <div class="sk w75"></div>
    </div>
  {:else if err}
    <div class="err">
      ❌ OCR 失败：{err}<br />
      <span class="sub">可点击下方「🔍 精准识别」改用本地视觉模型提取文字。</span>
    </div>
  {:else if lineMode && lines.length}
    <div class="lines">
      {#each lines as ln, idx (idx)}
        <button class="line-item" onclick={() => copy(ln.text)} title="点击复制该行">
          <span class="ln-no">{idx + 1}</span>
          <span class="ln-txt selectable">{ln.text}</span>
        </button>
      {/each}
    </div>
  {:else}
    <textarea bind:value={text} placeholder="未识别到文字，可点击下方「🔍 精准识别」使用视觉模型提取"></textarea>
  {/if}

  <div class="actions">
    <button class="primary" onclick={() => copy(text)} disabled={!text}>{copied ? '已复制' : '复制全文'}</button>
    <SpeakButton id="ocr-out" {text} label="朗读" variant="pill" title="使用 Edge TTS 朗读识别出的文字" />
    <button onclick={() => send('translate')} disabled={!text}>翻译</button>
    <button onclick={() => send('explain')} disabled={!text}>解释</button>
    <button onclick={() => send('summarize')} disabled={!text}>总结</button>
    <button onclick={() => send('qa')} disabled={!text}>问答</button>
    <button onclick={() => execute('ocr_precise', payload)} title="使用本地视觉大模型识别复杂排版/手写/表格">精准识别</button>
  </div>
</div>

<style>
  .ocr {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: 8px 10px;
    gap: 8px;
  }
  .top-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 8px;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  .thumb {
    height: 32px;
    max-width: 64px;
    object-fit: cover;
    border-radius: 4px;
    border: 1px solid var(--border);
  }
  .top-meta {
    display: flex;
    flex-direction: column;
    gap: 1px;
    flex: 1;
    min-width: 0;
  }
  .badge {
    font-size: 12px;
    font-weight: 600;
  }
  .badge.ok {
    color: var(--ok);
  }
  .badge.danger {
    color: var(--danger);
  }
  .sub {
    color: var(--fg-dim);
    font-size: 11.5px;
  }
  .sm {
    font-size: 12px;
    padding: 3px 7px;
  }
  .sm.on {
    background: var(--hover);
    border-color: var(--border);
  }
  textarea {
    flex: 1;
    width: 100%;
    resize: none;
    line-height: 1.65;
    font-size: 13.5px;
  }
  .lines {
    flex: 1;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .line-item {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    text-align: left;
    white-space: normal;
    padding: 5px 8px;
    border-radius: 6px;
  }
  .ln-no {
    color: var(--fg-dim);
    font-size: 11px;
    font-family: Consolas, monospace;
    min-width: 18px;
  }
  .ln-txt {
    flex: 1;
    word-break: break-word;
  }
  .sk-box {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 4px;
  }
  .sk {
    height: 12px;
    border-radius: 6px;
    background: var(--panel);
  }
  .w85 { width: 85%; }
  .w75 { width: 75%; }
  .w65 { width: 65%; }
  .err {
    color: var(--danger);
    font-size: 13px;
    flex: 1;
    padding: 8px 4px;
  }
  .actions {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
</style>
