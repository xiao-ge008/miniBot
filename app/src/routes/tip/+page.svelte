<script lang="ts">
  // 极简划词 / 复制悬浮胶囊栏：翻译 | 解释 | 总结 | 润色 | 问答 | 朗读 | 复制
  import { onMount, tick } from 'svelte';
  import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { ipc, onPayload } from '$lib/core/ipc';
  import { execute, loadActions } from '$lib/core/actions';
  import { stopSpeak, tts } from '$lib/core/tts.svelte';
  import SpeakButton from '$lib/components/SpeakButton.svelte';
  import type { Payload } from '$lib/core/types';

  let payload = $state<Payload | null>(null);
  let copied = $state(false);
  let savedKb = $state(false);
  let savingKb = $state(false);
  let hovering = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let hideMs = 5000;
  let bar: HTMLDivElement;
  const win = getCurrentWindow();

  const tipSpeaking = $derived(tts.activeId === 'tip-speak' && tts.status !== 'idle');

  $effect(() => {
    // 朗读状态切换（朗读 <-> 合成中 <-> 停止）时自适应胶囊宽度并保持窗口可见
    void tts.status;
    void tts.activeId;
    if (tipSpeaking) {
      clearTimeout(timer);
    }
    fit();
  });

  function schedule() {
    clearTimeout(timer);
    if (tipSpeaking) return;
    const ms = payload?.meta?.sticky ? 8000 : Math.max(hideMs, 4500);
    timer = setTimeout(() => {
      if (!hovering && !tipSpeaking) close();
    }, ms);
  }

  function close() {
    clearTimeout(timer);
    copied = false;
    if (tts.activeId === 'tip-speak') stopSpeak();
    ipc.hideSelf();
  }

  async function fit() {
    await tick();
    if (!bar) return;
    const r = bar.getBoundingClientRect();
    const w = Math.max(Math.ceil(r.width) + 10, 160);
    const h = Math.max(Math.ceil(r.height) + 10, 42);
    try {
      await win.setSize(new LogicalSize(w, h));
    } catch {
      /* 忽略尺寸同步异常 */
    }
  }

  async function show(p: Payload) {
    if (tts.activeId === 'tip-speak') stopSpeak();
    await loadActions();
    payload = p;
    copied = false;
    await fit();
    schedule();
  }

  async function runId(id: string) {
    if (!payload) return;
    const p = payload;
    clearTimeout(timer);
    if (tts.activeId === 'tip-speak') stopSpeak();
    await execute(id, p);
    ipc.hideSelf();
  }

  async function doCopy() {
    if (!payload) return;
    clearTimeout(timer);
    if (tts.activeId === 'tip-speak') stopSpeak();
    if (payload.text) {
      await ipc.copyText(payload.text);
    } else if (payload.image) {
      await ipc.copyImage(payload.image);
    }
    copied = true;
    setTimeout(() => {
      copied = false;
      ipc.hideSelf();
    }, 360);
  }

  async function doOpenUrl() {
    if (!payload?.text) return;
    clearTimeout(timer);
    if (tts.activeId === 'tip-speak') stopSpeak();
    await openUrl(payload.text);
    ipc.hideSelf();
  }

  async function doLocalSend() {
    if (!payload) return;
    const p = payload;
    clearTimeout(timer);
    if (tts.activeId === 'tip-speak') stopSpeak();
    await ipc.openLocalSend(p);
    ipc.hideSelf();
  }

  async function doSaveKb() {
    if (!payload || savingKb) return;
    clearTimeout(timer);
    if (tts.activeId === 'tip-speak') stopSpeak();
    savingKb = true;
    try {
      await ipc.saveToKnowledge(payload);
      savedKb = true;
      setTimeout(() => {
        savedKb = false;
        ipc.hideSelf();
      }, 800);
    } catch (e) {
      console.error('存入知识库失败:', e);
    } finally {
      savingKb = false;
    }
  }

  onMount(() => {
    ipc.getSettings().then((s) => (hideMs = Math.max(s.tip.hide_ms, 4500)));
    const un = onPayload(show);
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && close();
    window.addEventListener('keydown', onKey);
    return () => {
      un.then((f) => f());
      window.removeEventListener('keydown', onKey);
    };
  });
</script>

<div class="wrap">
  <div
    class="bar"
    bind:this={bar}
    role="toolbar"
    tabindex="-1"
    onmouseenter={() => {
      hovering = true;
      clearTimeout(timer);
    }}
    onmouseleave={() => {
      hovering = false;
      schedule();
    }}
  >
    {#if payload}
      <div class="row">
        <div class="grip" data-tauri-drag-region title="按住拖动">
          <svg width="6" height="12" viewBox="0 0 6 12" fill="currentColor" style="pointer-events:none">
            <circle cx="1.5" cy="2" r="1" /><circle cx="4.5" cy="2" r="1" />
            <circle cx="1.5" cy="6" r="1" /><circle cx="4.5" cy="6" r="1" />
            <circle cx="1.5" cy="10" r="1" /><circle cx="4.5" cy="10" r="1" />
          </svg>
        </div>

        {#if payload.kind === 'image' && payload.image}
          <img class="thumb" src={payload.image} alt="" />
          <button class="item" onclick={() => runId('auto')} title="智能识别图片内容">
            <span>识图</span>
          </button>
          <button class="item" onclick={() => runId('ocr')} title="提取图片文字 (OCR)">
            <span>文字</span>
          </button>
          <button class="item" onclick={() => runId('translate')} title="图片翻译">
            <span>翻译</span>
          </button>
          <button class="item" onclick={() => runId('qa')} title="针对图片问答">
            <span>问答</span>
          </button>
          {#if (payload.meta?.qr?.length ?? 0) > 0}
            <button class="item" onclick={() => runId('qr')} title="识别二维码">
              <span>二维码</span>
            </button>
          {/if}
          <span class="sep"></span>
          <button class="item" onclick={doLocalSend} title="通过 LocalSend 发送图片到局域网在线设备">
            <span>发送</span>
          </button>
          <button class="item" class:ok={savedKb} onclick={doSaveKb} disabled={savingKb} title="存入本地知识库（自动提取 OCR 与视觉智能整理）">
            <span>{savingKb ? '整理中…' : savedKb ? '已存入' : '存知识库'}</span>
          </button>
          <button class="item" class:ok={copied} onclick={doCopy} title="复制图片">
            <span>{copied ? '已复制' : '复制'}</span>
          </button>
        {:else if payload.kind === 'color'}
          <span class="swatch" style="background:{payload.text}"></span>
          <button class="item" onclick={() => runId('color')} title="颜色转换与对比度">
            <span>调色板</span>
          </button>
          <span class="sep"></span>
          <button class="item" class:ok={copied} onclick={doCopy} title="复制色值">
            <span>{copied ? '已复制' : '复制'}</span>
          </button>
        {:else}
          <button class="item" onclick={() => runId('translate')} title="智能中英互译">
            <span>翻译</span>
          </button>
          <button class="item" onclick={() => runId('explain')} title="解释与分析">
            <span>解释</span>
          </button>
          <button class="item" onclick={() => runId('summarize')} title="提炼核心要点总结">
            <span>总结</span>
          </button>
          <button class="item" onclick={() => runId('polish')} title="润色与改写">
            <span>润色</span>
          </button>
          <button class="item" onclick={() => runId('qa')} title="基于选中内容问答">
            <span>问答</span>
          </button>
          <span class="sep"></span>
          <SpeakButton
            id="tip-speak"
            text={payload.text ?? ''}
            label="朗读"
            variant="tip"
            title="使用 Edge TTS 朗读选中文字"
            onFinish={schedule}
          />
          <button class="item" onclick={doLocalSend} title="通过 LocalSend 发送内容到局域网在线设备">
            <span>发送</span>
          </button>
          <button class="item" class:ok={savedKb} onclick={doSaveKb} disabled={savingKb} title="存入本地知识库 (按日期合并)">
            <span>{savingKb ? '保存中…' : savedKb ? '已存入' : '存知识库'}</span>
          </button>
          <button class="item" class:ok={copied} onclick={doCopy} title="复制到剪贴板">
            <span>{copied ? '已复制' : '复制'}</span>
          </button>
          {#if payload.kind === 'url' || payload.kind === 'video'}
            <button class="item" onclick={doOpenUrl} title="在浏览器打开链接">
              <span>打开</span>
            </button>
          {/if}
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  .wrap {
    padding: 4px;
    display: inline-block;
  }
  .bar {
    display: inline-flex;
    align-items: center;
    padding: 4px 6px;
    background: var(--bg-solid);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: var(--shadow);
    animation: pop 0.1s cubic-bezier(0.2, 0.9, 0.3, 1);
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(2px) scale(0.98);
    }
  }
  .row {
    display: flex;
    align-items: center;
    gap: 1px;
  }
  .grip {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 12px;
    height: 24px;
    color: var(--fg-dim);
    cursor: grab;
    opacity: 0.45;
  }
  .grip:hover {
    opacity: 0.85;
  }
  .sep {
    width: 1px;
    height: 13px;
    background: var(--border);
    margin: 0 3px;
  }
  .item {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0 8px;
    height: 28px;
    border: none;
    background: transparent;
    color: var(--fg);
    font-size: 12.5px;
    font-weight: 500;
    line-height: 1;
    border-radius: 6px;
    transition: background 0.12s ease, color 0.12s ease;
  }
  .item:hover {
    background: var(--hover);
  }
  .item.ok {
    color: var(--ok);
  }
  .thumb {
    height: 20px;
    max-width: 30px;
    object-fit: cover;
    border-radius: 3px;
    border: 1px solid var(--border);
    margin: 0 3px;
  }
  .swatch {
    width: 14px;
    height: 14px;
    border-radius: 3px;
    border: 1px solid var(--border);
    margin: 0 4px;
  }
</style>
