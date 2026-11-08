<script lang="ts">
  // 结果浮窗：按动作的 view 渲染对应视图（视图注册表可扩展）
  import { onMount, type Component } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { ipc, onPayload, onLlmStatus, onOutsideClick } from '$lib/core/ipc';
  import { getAction, loadActions, type Action } from '$lib/core/actions';
  import type { Payload, StatusInfo } from '$lib/core/types';
  import ChatView from '$lib/views/ChatView.svelte';
  import OcrView from '$lib/views/OcrView.svelte';
  import QrView from '$lib/views/QrView.svelte';
  import PaletteView from '$lib/views/PaletteView.svelte';
  import ColorView from '$lib/views/ColorView.svelte';

  /** 视图注册表：新增视图（如视频抽帧）在这里登记 */
  const VIEWS: Record<string, Component<any>> = {
    chat: ChatView,
    ocr: OcrView,
    qr: QrView,
    palette: PaletteView,
    color: ColorView,
  };

  let payload = $state<Payload | null>(null);
  let action = $state<Action | null>(null);
  let pinned = $state(false);
  let busy = $state(false);
  let status = $state<StatusInfo | null>(null);
  let openedAt = 0;
  const win = getCurrentWindow();

  function close() {
    ipc.hideSelf();
  }

  async function togglePin() {
    pinned = !pinned;
    await win.setAlwaysOnTop(true).catch(() => {});
  }

  onMount(() => {
    const unP = onPayload(async (p) => {
      openedAt = Date.now();
      await loadActions();
      action = getAction(p.meta?.action) ?? null;
      busy = action?.view === 'chat';
      payload = p;
    });
    const unS = onLlmStatus((s) => (status = s));
    ipc.llmStatus().then((s) => (status = s));
    const unO = onOutsideClick(() => {
      if (!pinned && Date.now() - openedAt > 200) {
        close();
      }
    });
    // 失去焦点自动隐藏（未固定且打开超过 280ms）
    const unF = win.onFocusChanged(({ payload: focused }) => {
      if (focused) {
        openedAt = Date.now();
        return;
      }
      if (!pinned && Date.now() - openedAt > 280) {
        setTimeout(async () => {
          try {
            const stillFocused = await win.isFocused();
            if (!stillFocused && !pinned) close();
          } catch {
            if (!pinned) close();
          }
        }, 120);
      }
    });
    return () => {
      unP.then((f) => f());
      unS.then((f) => f());
      unO.then((f) => f());
      unF.then((f) => f());
    };
  });

  const View = $derived(action?.view ? VIEWS[action.view] : null);
  const dot = $derived(
    status?.api_enabled || status?.status === 'ready'
      ? 'ok'
      : status?.status === 'loading'
        ? 'load'
        : status?.status === 'yielded'
          ? 'warn'
          : 'off',
  );
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && close()} />

<div class="frame">
  <div class="title" data-tauri-drag-region>
    <span class="name" data-tauri-drag-region>
      {#if action}{action.label}{:else}miniBot{/if}
    </span>
    <span
      class="dot {dot}"
      title={status?.api_enabled ? `☁️ 云端模型 API：${status.api_model}` : status ? `本地模型：${status.status}` : ''}
    ></span>
    <span class="spacer" data-tauri-drag-region></span>
    <button class="ghost icon" class:on={pinned} onclick={togglePin} title={pinned ? '取消置顶固定' : '固定窗口（失焦不关闭）'}>固定</button>
    <button class="ghost icon" onclick={close} title="关闭 (Esc)">✕</button>
  </div>
  <div class="body">
    {#if payload && View}
      {#key payload.id}
        <View {payload} agent={action?.agent} onbusy={(b: boolean) => (busy = b)} />
      {/key}
    {:else if payload}
      <div class="empty">未知动作：{payload.meta?.action}</div>
    {/if}
  </div>
</div>

<style>
  :global(html, body) {
    height: 100%;
  }
  .frame {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-solid);
    overflow: hidden;
  }
  .title {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px 4px 12px;
    border-bottom: 1px solid var(--border);
    height: 36px;
    flex-shrink: 0;
  }
  .name {
    font-weight: 600;
    font-size: 13px;
    letter-spacing: -0.01em;
  }
  .spacer {
    flex: 1;
    align-self: stretch;
  }
  .icon {
    padding: 3px 7px;
    font-size: 12.5px;
    opacity: 0.65;
    border-radius: 6px;
  }
  .icon:hover {
    opacity: 1;
  }
  .icon.on {
    opacity: 1;
    background: var(--panel);
    border-color: var(--border);
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--fg-dim);
    opacity: 0.5;
  }
  .dot.ok {
    background: var(--ok);
    opacity: 1;
  }
  .dot.load {
    background: var(--accent-blue);
    opacity: 1;
  }
  .dot.warn {
    background: var(--warn);
    opacity: 1;
  }
  .body {
    flex: 1;
    min-height: 0;
  }
  .empty {
    padding: 16px;
    color: var(--fg-dim);
  }
</style>
