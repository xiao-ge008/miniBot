<script lang="ts">
  // 钉图（参考 eSearch 贴图 ding/item.ts）：悬停滑出顶栏、拖动/滚轮调节透明度与缩放、原位 OCR 翻译覆盖、镜像翻转、1:1 归位
  import { onMount } from 'svelte';
  import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
  import { ipc, onPayload, uid } from '$lib/core/ipc';
  import { translateTarget } from '$lib/core/agents';

  let src = $state('');
  let natW = 0;
  let natH = 0;
  let zoom = $state(1);
  let opacity = $state(1);
  let mirror = $state(false);
  let showBar = $state(false);
  let showTrans = $state(false);
  let transBusy = $state(false);
  let transText = $state('');
  let menu = $state<{ x: number; y: number } | null>(null);
  let toast = $state('');
  const win = getCurrentWindow();

  onMount(() => {
    const un = onPayload(async (p) => {
      src = p.image ?? '';
      showTrans = false;
      transText = '';
      const im = new Image();
      im.src = src;
      try {
        await im.decode();
      } catch {}
      const dpr = window.devicePixelRatio || 1;
      natW = (im.naturalWidth || 300) / dpr;
      natH = (im.naturalHeight || 200) / dpr;
    });
    return () => un.then((f) => f());
  });

  function applyZoom(z: number) {
    zoom = Math.max(0.2, Math.min(5, z));
    win.setSize(new LogicalSize(Math.max(80, natW * zoom), Math.max(50, natH * zoom)));
  }

  function onWheel(e: WheelEvent) {
    e.preventDefault();
    if (e.ctrlKey) {
      opacity = Math.max(0.15, Math.min(1, opacity + (e.deltaY < 0 ? 0.08 : -0.08)));
      return;
    }
    applyZoom(zoom * (e.deltaY < 0 ? 1.1 : 0.9));
  }

  function flash(t: string) {
    toast = t;
    setTimeout(() => (toast = ''), 1300);
  }

  async function toggleTranslate() {
    if (showTrans) {
      showTrans = false;
      return;
    }
    showTrans = true;
    if (transText || transBusy) return;
    transBusy = true;
    try {
      const ocr = await ipc.ocrImage(src);
      const raw = ocr.text.trim();
      if (!raw) {
        transText = '（未识别到文字）';
        return;
      }
      const target = translateTarget(raw);
      transText = '';
      await ipc.llmChat(
        uid('pin'),
        [
          { role: 'system', content: '你是一名专业翻译。只输出译文，保留段落换行，不要添加任何解释。' },
          { role: 'user', content: `请将以下内容翻译为${target}：\n\n${raw}` },
        ],
        (ev) => {
          if (ev.type === 'delta') transText += ev.text;
        },
      );
    } catch (e) {
      transText = `翻译失败：${e}`;
    } finally {
      transBusy = false;
    }
  }

  async function doMenu(id: string) {
    menu = null;
    if (id === 'copy') {
      await ipc.copyImage(src);
      flash('已复制图片');
    } else if (id === 'save') {
      const p = await ipc.saveImage(src);
      flash(`已保存 ${p}`);
    } else if (id === 'mirror') {
      mirror = !mirror;
    } else if (id === 'reset') {
      opacity = 1;
      mirror = false;
      applyZoom(1);
    } else if (id === 'close') win.close();
  }
</script>

<svelte:window
  onkeydown={(e) => {
    if (e.key === 'Escape') win.close();
    else if (e.key.toLowerCase() === 'r') doMenu('reset');
    else if (e.key.toLowerCase() === 'm') doMenu('mirror');
  }}
  onclick={() => (menu = null)}
/>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="pin"
  role="application"
  onmouseenter={() => (showBar = true)}
  onmouseleave={() => (showBar = false)}
  onwheel={onWheel}
  onmousedown={(e) => e.button === 0 && !menu && !(e.target as HTMLElement).closest('.top-bar') && win.startDragging()}
  ondblclick={(e) => !(e.target as HTMLElement).closest('.top-bar') && win.close()}
  oncontextmenu={(e) => {
    e.preventDefault();
    menu = { x: e.clientX, y: e.clientY };
  }}
>
  <div class="top-bar" class:show={showBar}>
    <span class="pill" title="Ctrl+滚轮调节透明度">◑ {Math.round(opacity * 100)}%</span>
    <span class="pill" title="滚轮调节缩放">🔍 {Math.round(zoom * 100)}%</span>
    <button class="tb" class:on={showTrans} onclick={toggleTranslate} title="原位翻译覆盖">译</button>
    <button class="tb" class:on={mirror} onclick={() => doMenu('mirror')} title="水平镜像 (M)">⇄</button>
    <button class="tb" onclick={() => doMenu('reset')} title="1:1 归位 (R)">1:1</button>
    <button class="tb" onclick={() => doMenu('copy')} title="复制">📋</button>
    <button class="tb" onclick={() => doMenu('save')} title="保存">💾</button>
    <button class="tb close" onclick={() => doMenu('close')} title="关闭 (Esc)">✕</button>
  </div>

  <div class="stage" style="opacity:{opacity}">
    {#if src}<img {src} alt="" draggable="false" class:mirror />{/if}
    {#if showTrans}
      <div class="trans-layer selectable" role="region" onmousedown={(e) => e.stopPropagation()}>
        {#if transBusy && !transText}
          <span class="dim">⏳ 正在识别并翻译图中文字…</span>
        {:else}
          {transText}
        {/if}
      </div>
    {/if}
  </div>
</div>

{#if menu}
  <div class="menu" style="left:{menu.x}px; top:{menu.y}px">
    <button class="ghost" onclick={() => doMenu('copy')}>📋 复制图片</button>
    <button class="ghost" onclick={() => doMenu('save')}>💾 保存图片</button>
    <button class="ghost" onclick={toggleTranslate}>🌐 {showTrans ? '隐藏译文' : '图中翻译'}</button>
    <button class="ghost" onclick={() => doMenu('mirror')}>⇄ 水平镜像 (M)</button>
    <button class="ghost" onclick={() => doMenu('reset')}>↺ 原始大小 (R)</button>
    <button class="ghost" onclick={() => doMenu('close')}>✕ 关闭 (Esc)</button>
  </div>
{/if}
{#if toast}<div class="toast">{toast}</div>{/if}

<style>
  .pin {
    position: fixed;
    inset: 0;
    cursor: move;
    border: 1px solid rgba(59, 130, 246, 0.55);
    border-radius: 6px;
    overflow: hidden;
  }
  .stage {
    width: 100%;
    height: 100%;
    position: relative;
  }
  img {
    width: 100%;
    height: 100%;
    display: block;
    pointer-events: none;
    transition: transform 0.18s ease;
  }
  img.mirror {
    transform: scaleX(-1);
  }
  .top-bar {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    z-index: 4;
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 3px 5px;
    background: rgba(18, 18, 21, 0.86);
    backdrop-filter: blur(12px);
    border-bottom: 1px solid rgba(255, 255, 255, 0.12);
    transform: translateY(-105%);
    transition: transform 0.2s cubic-bezier(0.25, 1, 0.5, 1);
    cursor: default;
  }
  .top-bar.show {
    transform: translateY(0);
  }
  .pill {
    font-size: 11px;
    color: #d4d4d8;
    padding: 1px 4px;
    font-family: Consolas, monospace;
  }
  .tb {
    border: none;
    background: transparent;
    color: #fafafa;
    font-size: 11.5px;
    padding: 2px 5px;
    border-radius: 4px;
  }
  .tb:hover {
    background: rgba(255, 255, 255, 0.15);
  }
  .tb.on {
    background: #2563eb;
    color: #fff;
  }
  .tb.close {
    margin-left: auto;
  }
  .trans-layer {
    position: absolute;
    inset: 0;
    background: rgba(18, 18, 21, 0.88);
    backdrop-filter: blur(8px);
    color: #fafafa;
    padding: 32px 10px 10px;
    font-size: 13px;
    line-height: 1.55;
    white-space: pre-wrap;
    overflow: auto;
    cursor: text;
  }
  .dim {
    color: #a1a1aa;
  }
  .menu {
    position: fixed;
    display: flex;
    flex-direction: column;
    background: var(--bg-solid);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 3px;
    box-shadow: var(--shadow);
    z-index: 6;
  }
  .menu button {
    text-align: left;
    font-size: 12.5px;
    padding: 4px 10px;
  }
  .toast {
    position: fixed;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
    background: rgba(18, 18, 21, 0.85);
    color: #fff;
    padding: 4px 10px;
    border-radius: 6px;
    font-size: 12px;
    max-width: 90%;
    word-break: break-all;
    z-index: 7;
  }
</style>
