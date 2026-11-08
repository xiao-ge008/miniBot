<script lang="ts">
  // 极简录屏组件：
  // 1. recorder 窗口：悬浮控制胶囊栏（计时 | 暂停/继续 | 完成保存 | 取消 | 录制完成后一键复制/文件夹/发送）
  // 2. recorder_border 窗口：鼠标完全穿透 + 视频防穿帮的录制区域高亮边框（让用户清晰感知录制范围与状态）
  import { onMount, tick } from 'svelte';
  import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
  import { ipc, onRecorderState, uid } from '$lib/core/ipc';
  import type { RecorderStateInfo } from '$lib/core/types';

  const win = getCurrentWindow();
  const isBorder = win.label === 'recorder_border';
  let bar = $state<HTMLDivElement | null>(null);

  let st = $state<RecorderStateInfo>({
    status: 'recording',
    elapsed_ms: 0,
    fps: 60,
    width: 0,
    height: 0,
    system_audio: true,
    mic_audio: false,
    output_path: '',
    file_size_bytes: 0,
    error: null,
  });

  let copied = $state(false);
  let busy = $state(false);
  let hovering = false;
  let pollTimer: ReturnType<typeof setInterval> | undefined;
  let autoCloseTimer: ReturnType<typeof setTimeout> | undefined;

  function fmtTime(ms: number): string {
    const totalSec = Math.max(0, Math.floor(ms / 1000));
    const m = Math.floor(totalSec / 60);
    const s = totalSec % 60;
    return `${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
  }

  function fmtSize(bytes: number): string {
    if (!bytes || bytes <= 0) return '';
    if (bytes < 1024 * 1024) return `${ Math.max(1, Math.round(bytes / 1024)) } KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }

  async function fit() {
    if (isBorder) return;
    await tick();
    if (!bar) return;
    const rect = bar.getBoundingClientRect();
    const contentW = Math.max(Math.ceil(rect.width), bar.offsetWidth, bar.scrollWidth);
    const contentH = Math.max(Math.ceil(rect.height), bar.offsetHeight);
    const w = Math.max(contentW + 16, 210);
    const h = Math.max(contentH + 12, 48);
    try {
      await win.setSize(new LogicalSize(w, h));
    } catch {
      /* 忽略尺寸同步异常 */
    }
  }

  function scheduleDoneClose() {
    if (isBorder) return;
    clearTimeout(autoCloseTimer);
    if (st.status !== 'done') return;
    autoCloseTimer = setTimeout(() => {
      if (!hovering && st.status === 'done') {
        closeHud();
      }
    }, 10000);
  }

  async function syncStatus() {
    try {
      const cur = await ipc.recorderStatus();
      const prevStatus = st.status;
      st = cur;
      if (isBorder) {
        return;
      }
      if (prevStatus !== cur.status) {
        copied = false;
        await fit();
        requestAnimationFrame(() => fit());
        if (cur.status === 'done') scheduleDoneClose();
      }
    } catch {
      /* ignore */
    }
  }

  async function togglePause() {
    if (busy) return;
    busy = true;
    try {
      if (st.status === 'paused') {
        st = await ipc.recorderResume();
      } else if (st.status === 'recording') {
        st = await ipc.recorderPause();
      }
      await fit();
    } finally {
      busy = false;
    }
  }

  async function stopAndSave() {
    if (busy) return;
    busy = true;
    try {
      try {
        await win.setSize(new LogicalSize(440, 56));
      } catch {
        /* ignore */
      }
      st = await ipc.recorderStop();
      await fit();
      requestAnimationFrame(() => fit());
    } finally {
      busy = false;
    }
  }

  async function cancelRec() {
    if (busy) return;
    busy = true;
    try {
      await ipc.recorderCancel();
    } finally {
      busy = false;
    }
  }

  async function copyFile() {
    if (!st.output_path) return;
    clearTimeout(autoCloseTimer);
    try {
      await ipc.recorderCopyFile(st.output_path);
      copied = true;
      await fit();
      setTimeout(() => {
        copied = false;
        closeHud();
      }, 650);
    } catch {
      /* ignore */
    }
  }

  async function revealFile() {
    if (!st.output_path) return;
    clearTimeout(autoCloseTimer);
    await ipc.recorderReveal(st.output_path);
    closeHud();
  }

  async function sendViaLocalSend() {
    if (!st.output_path) return;
    clearTimeout(autoCloseTimer);
    const p = {
      id: uid('rec'),
      kind: 'file' as const,
      origin: 'capture' as const,
      text: '',
      image: null,
      source_app: null,
      source_hwnd: 0,
      meta: { files: [st.output_path] },
    };
    await ipc.openLocalSend(p);
    closeHud();
  }

  function closeHud() {
    clearTimeout(autoCloseTimer);
    ipc.hideSelf();
  }

  onMount(() => {
    syncStatus();
    pollTimer = setInterval(syncStatus, 250);
    const un = onRecorderState(async (next) => {
      st = next;
      if (isBorder) {
        return;
      }
      copied = false;
      await fit();
      requestAnimationFrame(() => fit());
      if (next.status === 'done') scheduleDoneClose();
    });
    return () => {
      clearInterval(pollTimer);
      clearTimeout(autoCloseTimer);
      un.then((f) => f());
    };
  });
</script>

{#if isBorder}
  {#if st.status === 'recording' || st.status === 'paused'}
    <div class="rec-region-frame" class:paused={st.status === 'paused'}>
      <span class="corner tl"></span>
      <span class="corner tr"></span>
      <span class="corner bl"></span>
      <span class="corner br"></span>
      <div class="region-badge">
        <span class="badge-dot" class:paused={st.status === 'paused'}></span>
        <span class="badge-txt">{st.status === 'paused' ? '已暂停' : 'REC'}</span>
        {#if st.width > 0 && st.height > 0}
          <span class="badge-dim">{st.width}×{st.height}</span>
        {/if}
      </div>
    </div>
  {/if}
{:else}
  <div class="wrap">
    <div
      class="bar"
      bind:this={bar}
      role="toolbar"
      tabindex="-1"
      onmouseenter={() => {
        hovering = true;
        clearTimeout(autoCloseTimer);
      }}
      onmouseleave={() => {
        hovering = false;
        if (st.status === 'done') scheduleDoneClose();
      }}
    >
      <div class="row">
        <div class="grip" data-tauri-drag-region title="按住拖动（此悬浮条已开启系统级防穿帮，不会被录进视频）">
          <svg width="6" height="12" viewBox="0 0 6 12" fill="currentColor" style="pointer-events:none">
            <circle cx="1.5" cy="2" r="1" /><circle cx="4.5" cy="2" r="1" />
            <circle cx="1.5" cy="6" r="1" /><circle cx="4.5" cy="6" r="1" />
            <circle cx="1.5" cy="10" r="1" /><circle cx="4.5" cy="10" r="1" />
          </svg>
        </div>

        {#if st.status === 'recording' || st.status === 'paused'}
          <div
            class="timer-pill"
            data-tauri-drag-region
            title={st.width > 0 ? `录制范围 ${st.width}×${st.height} · ${st.fps} FPS` : '正在录屏'}
          >
            <span class="rec-dot" class:paused={st.status === 'paused'}></span>
            <span class="time-txt">{fmtTime(st.elapsed_ms)}</span>
          </div>

          <span class="sep"></span>

          <button
            class="item"
            onclick={togglePause}
            disabled={busy}
            title={st.status === 'paused' ? '继续录制' : '暂停录制'}
          >
            {#if st.status === 'paused'}
              <svg viewBox="0 0 16 16" width="12" height="12" fill="currentColor">
                <polygon points="4,2.5 13,8 4,13.5" />
              </svg>
              <span>继续</span>
            {:else}
              <svg viewBox="0 0 16 16" width="12" height="12" fill="currentColor">
                <rect x="3.5" y="3" width="3" height="10" rx="0.8" />
                <rect x="9.5" y="3" width="3" height="10" rx="0.8" />
              </svg>
              <span>暂停</span>
            {/if}
          </button>

          <button class="item stop-btn" onclick={stopAndSave} disabled={busy} title="停止并保存 MP4 视频 (Alt+R)">
            <span class="stop-sq"></span>
            <span>完成</span>
          </button>

          <span class="sep"></span>

          <button class="item icon-only cancel" onclick={cancelRec} disabled={busy} title="取消并丢弃本次录制">
            <svg viewBox="0 0 16 16" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round">
              <line x1="4" y1="4" x2="12" y2="12" />
              <line x1="12" y1="4" x2="4" y2="12" />
            </svg>
          </button>
        {:else if st.status === 'saving'}
          <div class="status-msg" data-tauri-drag-region>
            <span class="rec-dot saving"></span>
            <span>正在保存 MP4…</span>
          </div>
        {:else if st.status === 'done'}
          <div class="done-badge" data-tauri-drag-region title={st.output_path}>
            <span class="ok-icon">
              <svg viewBox="0 0 16 16" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="3.2 8.5 6.6 11.8 12.8 4.6" />
              </svg>
            </span>
            <span class="done-txt">已保存</span>
            {#if st.file_size_bytes > 0}
              <span class="size-lbl">{fmtSize(st.file_size_bytes)}</span>
            {/if}
          </div>

          <span class="sep"></span>

          <button class="item" class:ok={copied} onclick={copyFile} title="复制 MP4 视频文件到剪贴板（可直接粘贴到微信/飞书）">
            <svg viewBox="0 0 16 16" width="12.5" height="12.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
              <rect x="5.5" y="5.5" width="8" height="8" rx="1.5" />
              <path d="M3.5 10.5H3a1.5 1.5 0 0 1-1.5-1.5V3A1.5 1.5 0 0 1 3 1.5h6A1.5 1.5 0 0 1 10.5 3v.5" />
            </svg>
            <span>{copied ? '已复制' : '复制'}</span>
          </button>

          <button class="item" onclick={revealFile} title="在文件夹中定位视频文件">
            <svg viewBox="0 0 16 16" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
              <path d="M2 4.5A1.5 1.5 0 0 1 3.5 3h2.8l1.4 1.8h4.8A1.5 1.5 0 0 1 14 6.3v5.2a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 11.5v-7Z" />
            </svg>
            <span>文件夹</span>
          </button>

          <button class="item" onclick={sendViaLocalSend} title="通过 LocalSend 发送视频到局域网手机/电脑">
            <svg viewBox="0 0 16 16" width="12.5" height="12.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
              <path d="M14 2 7.5 8.5" />
              <path d="m14 2-4.2 12-2.3-5.5L2 6.2 14 2Z" />
            </svg>
            <span>发送</span>
          </button>

          <span class="sep"></span>

          <button class="item icon-only cancel" onclick={closeHud} title="关闭">
            <svg viewBox="0 0 16 16" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round">
              <line x1="4" y1="4" x2="12" y2="12" />
              <line x1="12" y1="4" x2="4" y2="12" />
            </svg>
          </button>
        {:else if st.status === 'error'}
          <div class="status-msg err" data-tauri-drag-region title={st.error ?? '录制失败'}>
            <span>录制失败：{st.error ? st.error.slice(0, 24) : '未知错误'}</span>
          </div>
          <span class="sep"></span>
          <button class="item icon-only cancel" onclick={closeHud} title="关闭">
            <svg viewBox="0 0 16 16" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round">
              <line x1="4" y1="4" x2="12" y2="12" />
              <line x1="12" y1="4" x2="4" y2="12" />
            </svg>
          </button>
        {:else}
          <div class="status-msg" data-tauri-drag-region>
            <span>准备录屏…</span>
          </div>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  /* ================= 1. 录制选区高亮框（鼠标穿透 + 防穿帮） ================= */
  .rec-region-frame {
    position: fixed;
    inset: 0;
    pointer-events: none;
    user-select: none;
    box-sizing: border-box;
    border: 2px dashed rgba(239, 68, 68, 0.92);
    border-radius: 3px;
    box-shadow:
      inset 0 0 0 1px rgba(0, 0, 0, 0.32),
      0 0 0 1px rgba(0, 0, 0, 0.25);
  }
  .rec-region-frame.paused {
    border-color: rgba(245, 158, 11, 0.95);
  }
  .corner {
    position: absolute;
    width: 14px;
    height: 14px;
    border-color: #ef4444;
    border-style: solid;
    pointer-events: none;
    filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.45));
  }
  .rec-region-frame.paused .corner {
    border-color: #f59e0b;
  }
  .corner.tl {
    top: -2px;
    left: -2px;
    border-width: 3px 0 0 3px;
    border-top-left-radius: 3px;
  }
  .corner.tr {
    top: -2px;
    right: -2px;
    border-width: 3px 3px 0 0;
    border-top-right-radius: 3px;
  }
  .corner.bl {
    bottom: -2px;
    left: -2px;
    border-width: 0 0 3px 3px;
    border-bottom-left-radius: 3px;
  }
  .corner.br {
    bottom: -2px;
    right: -2px;
    border-width: 0 3px 3px 0;
    border-bottom-right-radius: 3px;
  }
  .region-badge {
    position: absolute;
    top: 6px;
    left: 6px;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 20px;
    padding: 0 7px;
    border-radius: 4px;
    background: rgba(15, 23, 42, 0.72);
    color: #ffffff;
    font-family: 'Cascadia Code', 'JetBrains Mono', Consolas, monospace;
    font-size: 11px;
    font-weight: 600;
    line-height: 1;
    white-space: nowrap;
    pointer-events: none;
  }
  .badge-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #ef4444;
    animation: pulse 1.2s ease-in-out infinite;
    flex-shrink: 0;
  }
  .badge-dot.paused {
    background: #f59e0b;
    animation: none;
  }
  .badge-txt {
    letter-spacing: 0.03em;
  }
  .badge-dim {
    opacity: 0.8;
    font-weight: 500;
    font-size: 10.5px;
  }

  /* ================= 2. 极简悬浮控制胶囊栏（严格单行防折行） ================= */
  .wrap {
    padding: 4px;
    display: inline-flex;
    width: max-content;
    white-space: nowrap;
  }
  .bar {
    display: inline-flex;
    align-items: center;
    width: max-content;
    white-space: nowrap;
    flex-wrap: nowrap;
    box-sizing: border-box;
    padding: 4px 7px;
    background: var(--bg-solid);
    border: 1px solid var(--border);
    border-radius: 9px;
    box-shadow: var(--shadow);
  }
  .row {
    display: inline-flex;
    align-items: center;
    width: max-content;
    white-space: nowrap;
    flex-wrap: nowrap;
    gap: 2px;
  }
  .grip {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 12px;
    height: 24px;
    color: var(--fg-dim);
    cursor: grab;
    opacity: 0.45;
    flex-shrink: 0;
  }
  .grip:hover {
    opacity: 0.85;
  }
  .timer-pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 0 7px 0 5px;
    height: 28px;
    cursor: grab;
    white-space: nowrap;
    flex-shrink: 0;
  }
  .rec-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--danger);
    box-shadow: 0 0 0 2px rgba(239, 68, 68, 0.2);
    animation: pulse 1.2s ease-in-out infinite;
    flex-shrink: 0;
  }
  .rec-dot.paused {
    background: var(--warn);
    box-shadow: 0 0 0 2px rgba(245, 158, 11, 0.2);
    animation: none;
  }
  .rec-dot.saving {
    background: var(--accent-blue);
    box-shadow: none;
  }
  @keyframes pulse {
    0%,
    100% {
      opacity: 1;
      transform: scale(1);
    }
    50% {
      opacity: 0.38;
      transform: scale(0.88);
    }
  }
  .time-txt {
    font-family: 'Cascadia Code', 'JetBrains Mono', Consolas, monospace;
    font-variant-numeric: tabular-nums;
    font-size: 13px;
    font-weight: 600;
    color: var(--fg);
    letter-spacing: 0.02em;
    line-height: 1;
    white-space: nowrap;
    flex-shrink: 0;
  }
  .sep {
    width: 1px;
    height: 14px;
    background: var(--border);
    margin: 0 3px;
    flex-shrink: 0;
  }
  .item {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 4px;
    padding: 0 8px;
    height: 28px;
    border: none;
    background: transparent;
    color: var(--fg);
    font-size: 12.5px;
    font-weight: 500;
    line-height: 1;
    border-radius: 6px;
    white-space: nowrap;
    flex-shrink: 0;
    transition: background 0.12s ease, color 0.12s ease;
  }
  .item span,
  .item svg {
    white-space: nowrap;
    flex-shrink: 0;
  }
  .item:hover:not(:disabled) {
    background: var(--hover);
  }
  .item.icon-only {
    width: 26px;
    padding: 0;
  }
  .item.stop-btn {
    color: var(--danger);
    font-weight: 600;
  }
  .item.stop-btn:hover:not(:disabled) {
    background: rgba(239, 68, 68, 0.12);
  }
  .stop-sq {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    background: currentColor;
    display: inline-block;
    flex-shrink: 0;
  }
  .item.cancel {
    color: var(--fg-dim);
  }
  .item.cancel:hover:not(:disabled) {
    color: var(--danger);
    background: rgba(239, 68, 68, 0.12);
  }
  .item.ok {
    color: var(--ok);
  }
  .done-badge {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 0 6px 0 4px;
    height: 28px;
    cursor: grab;
    white-space: nowrap;
    flex-shrink: 0;
  }
  .ok-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: rgba(16, 185, 129, 0.14);
    color: var(--ok);
    flex-shrink: 0;
  }
  .done-txt {
    font-size: 12.5px;
    font-weight: 600;
    color: var(--fg);
    line-height: 1;
    white-space: nowrap;
    flex-shrink: 0;
  }
  .size-lbl {
    display: inline-flex;
    align-items: center;
    height: 18px;
    padding: 0 5px;
    border-radius: 4px;
    background: var(--hover);
    font-size: 11px;
    font-weight: 500;
    color: var(--fg-dim);
    font-family: 'Cascadia Code', Consolas, monospace;
    line-height: 1;
    white-space: nowrap;
    flex-shrink: 0;
  }
  .status-msg {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 0 8px;
    height: 28px;
    font-size: 12.5px;
    color: var(--fg-dim);
    white-space: nowrap;
    flex-shrink: 0;
  }
  .status-msg.err {
    color: var(--danger);
  }
</style>
