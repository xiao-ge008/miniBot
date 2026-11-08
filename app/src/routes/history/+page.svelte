<script lang="ts">
  // 历史记录（保留天数见设置，默认 30 天自动清理）
  import { onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { ipc } from '$lib/core/ipc';
  import { BUILTIN_AGENTS } from '$lib/core/agents';
  import SpeakButton from '$lib/components/SpeakButton.svelte';
  import type { HistoryEntry } from '$lib/core/types';

  const win = getCurrentWindow();

  let q = $state('');
  let list = $state<HistoryEntry[]>([]);
  let current = $state<HistoryEntry | null>(null);
  let img = $state('');
  let timer: ReturnType<typeof setTimeout> | undefined;

  const names: Record<string, string> = Object.fromEntries([
    ...BUILTIN_AGENTS.map((a) => [a.id, `${a.icon} ${a.name}`]),
    ['ocr', '🔤 识别文字'],
  ]);

  async function search() {
    list = await ipc.historyList(q.trim(), 300);
    if (current && !list.find((x) => x.id === current!.id)) current = null;
  }

  async function select(e: HistoryEntry) {
    current = e;
    img = e.image ? await ipc.readImageFile(e.image).catch(() => '') : '';
  }

  async function del(e: HistoryEntry) {
    await ipc.historyDelete(e.id!);
    current = null;
    search();
  }

  const fmt = (ts?: number) => (ts ? new Date(ts * 1000).toLocaleString('zh-CN', { hour12: false }) : '');

  onMount(search);
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && ipc.closePage()} />

<div class="page">
  <aside>
    <div class="side-head" data-tauri-drag-region>
      <b data-tauri-drag-region>划词与识图历史</b>
      <span class="ttl-hint" data-tauri-drag-region>30天自动清理</span>
    </div>
    <input
      class="search"
      bind:value={q}
      placeholder="搜索输入或结果…"
      oninput={() => {
        clearTimeout(timer);
        timer = setTimeout(search, 250);
      }}
    />
    <div class="list">
      {#each list as e (e.id)}
        <button class="item" class:sel={current?.id === e.id} onclick={() => select(e)}>
          <div class="top"><span>{names[e.action] ?? e.action}</span><span class="dim">{fmt(e.ts)}</span></div>
          <div class="preview">{e.output.slice(0, 80) || e.input.slice(0, 80)}</div>
        </button>
      {:else}
        <div class="dim pad">暂无记录</div>
      {/each}
    </div>
  </aside>
  <main>
    <div class="top-bar" data-tauri-drag-region>
      {#if current}
        <b data-tauri-drag-region>{names[current.action] ?? current.action}</b>
        <span class="dim" data-tauri-drag-region>{fmt(current.ts)} {current.source_app ? `· ${current.source_app}` : ''}</span>
        <span class="sp" data-tauri-drag-region></span>
        <SpeakButton id={`hist-${current.id}`} text={current.output} label="朗读结果" variant="pill" />
        <button onclick={() => ipc.copyText(current!.output)}>复制结果</button>
        <button class="ghost" onclick={() => del(current!)}>删除</button>
      {:else}
        <span class="dim" data-tauri-drag-region>选择左侧记录查看详情</span>
        <span class="sp" data-tauri-drag-region></span>
      {/if}
      <button class="win-ctrl" onclick={() => ipc.minimizeSelf()} title="最小化">─</button>
      <button class="win-ctrl close" onclick={() => ipc.closePage()} title="关闭 (Esc)">✕</button>
    </div>

    <div class="detail-body">
      {#if current}
        {#if img}<img src={img} alt="" />{/if}
        <h4>输入内容</h4>
        <pre class="selectable">{current.input}</pre>
        <h4>识别/处理结果</h4>
        <pre class="selectable">{current.output}</pre>
      {/if}
    </div>
  </main>
</div>

<style>
  :global(body) {
    background: var(--bg-solid) !important;
  }
  .page {
    display: flex;
    height: 100vh;
  }
  aside {
    width: 300px;
    border-right: 1px solid var(--border);
    background: var(--panel);
    display: flex;
    flex-direction: column;
  }
  .side-head {
    height: 46px;
    padding: 0 14px;
    border-bottom: 1px solid var(--border);
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 13.5px;
  }
  .search {
    margin: 10px;
    background: var(--bg-solid);
  }
  .ttl-hint {
    font-size: 11px;
    color: var(--fg-dim);
  }
  .list {
    flex: 1;
    overflow: auto;
  }
  .item {
    display: block;
    width: 100%;
    text-align: left;
    border: none;
    border-bottom: 1px solid var(--border);
    border-radius: 0;
    background: transparent;
    padding: 8px 12px;
  }
  .item.sel {
    background: var(--hover);
  }
  .top {
    display: flex;
    justify-content: space-between;
    font-size: 13px;
  }
  .preview {
    font-size: 12px;
    color: var(--fg-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    margin-top: 2px;
  }
  main {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .top-bar {
    height: 46px;
    padding: 0 12px 0 20px;
    border-bottom: 1px solid var(--border);
    display: flex;
    gap: 10px;
    align-items: center;
    flex-shrink: 0;
  }
  .detail-body {
    flex: 1;
    overflow: auto;
    padding: 14px 20px;
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
  }
  .win-ctrl:hover {
    background: var(--hover);
    color: var(--fg);
  }
  .win-ctrl.close:hover {
    background: rgba(239, 68, 68, 0.14);
    color: var(--danger);
  }
  .sp {
    flex: 1;
    align-self: stretch;
  }
  img {
    max-width: 100%;
    max-height: 260px;
    border-radius: 8px;
    border: 1px solid var(--border);
    margin-top: 10px;
  }
  pre {
    white-space: pre-wrap;
    word-break: break-word;
    background: var(--hover);
    padding: 10px;
    border-radius: 8px;
    font-family: inherit;
  }
  .dim {
    color: var(--fg-dim);
    font-size: 12px;
  }
  .pad {
    padding: 16px;
  }
</style>
