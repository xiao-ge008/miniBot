<script lang="ts">
  // LocalSend 局域网文件 / 文本 / 截图快传独立浮窗（与主界面极简黑白灰风格统一）
  // 兼容所有安装了官方 LocalSend (https://github.com/localsend/localsend) 的客户端（iOS / Android / Windows / macOS / Linux）
  import { onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
  import { ipc, onPayload, uid } from '$lib/core/ipc';
  import type {
    LocalSendItem,
    LocalSendPeer,
    LocalSendProgress,
    LocalSendSelfInfo,
    Payload,
  } from '$lib/core/types';

  const win = getCurrentWindow();
  const webviewWin = getCurrentWebviewWindow();
  const STORAGE_KEY_IPS = 'minibot_localsend_extra_ips';

  let selfInfo = $state<LocalSendSelfInfo | null>(null);
  let items = $state<LocalSendItem[]>([]);
  let peers = $state<LocalSendPeer[]>([]);
  let scanning = $state(false);
  let pinned = $state(true);
  let dragOver = $state(false);

  // 手动添加 IP
  let showManualIp = $state(false);
  let manualIpInput = $state('');
  let probingIp = $state(false);
  let extraIps = $state<string[]>([]);

  // 传输状态与 PIN 码
  let sendingPeer = $state<LocalSendPeer | null>(null);
  let progress = $state<LocalSendProgress | null>(null);
  let needPin = $state(false);
  let pinCode = $state('');
  let toast = $state('');
  let toastTimer: ReturnType<typeof setTimeout> | undefined;

  function showToast(msg: string, ms = 2200) {
    clearTimeout(toastTimer);
    toast = msg;
    toastTimer = setTimeout(() => {
      toast = '';
    }, ms);
  }

  function fmtBytes(bytes: number): string {
    if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }

  const totalBytes = $derived(items.reduce((acc, it) => acc + (it.size || 0), 0));
  const isBusy = $derived(
    progress?.stage === 'preparing' ||
      progress?.stage === 'waiting_accept' ||
      progress?.stage === 'sending',
  );

  function estimateBase64Bytes(dataUrl: string): number {
    const comma = dataUrl.indexOf(',');
    const b64 = comma >= 0 ? dataUrl.slice(comma + 1) : dataUrl;
    return Math.max(1, Math.floor((b64.length * 3) / 4));
  }

  function nowTag(): string {
    const d = new Date();
    const pad = (n: number) => String(n).padStart(2, '0');
    return `${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}`;
  }

  // ---------- 接收外部投递的 Payload（截图 / 划词 / 右键菜单文件） ----------

  async function handleIncomingPayload(p: Payload) {
    if (!p) return;

    // 1. 来自系统右键菜单传入的本地文件/文件夹路径列表
    const filePaths = p.meta?.files as string[] | undefined;
    if (Array.isArray(filePaths) && filePaths.length > 0) {
      const resolved = await ipc.localSendResolvePaths(filePaths);
      mergeItems(resolved);
      if (peers.length === 0 && !scanning) {
        scanPeers();
      }
      return;
    }

    // 2. 来自截图或复制图片的 Base64 PNG
    if (p.kind === 'image' && p.image) {
      const item: LocalSendItem = {
        id: uid('ls_img'),
        kind: 'image',
        name: `截图_${nowTag()}.png`,
        path: null,
        text: null,
        data_url: p.image,
        size: estimateBase64Bytes(p.image),
        file_type: 'image/png',
      };
      items = [item, ...items];
      if (peers.length === 0 && !scanning) {
        scanPeers();
      }
      return;
    }

    // 3. 来自划词 Tip 或链接的文本内容
    if (p.text && p.text.trim()) {
      const txt = p.text;
      const bytes = new TextEncoder().encode(txt).length;
      const item: LocalSendItem = {
        id: uid('ls_txt'),
        kind: 'text',
        name: `文本消息_${nowTag()}.txt`,
        path: null,
        text: txt,
        data_url: null,
        size: bytes,
        file_type: 'text/plain',
      };
      items = [item, ...items];
      if (peers.length === 0 && !scanning) {
        scanPeers();
      }
    }
  }

  function mergeItems(newItems: LocalSendItem[]) {
    if (!newItems.length) return;
    const existingPaths = new Set(items.map((i) => i.path).filter(Boolean));
    const filtered = newItems.filter((i) => !i.path || !existingPaths.has(i.path));
    if (filtered.length > 0) {
      items = [...items, ...filtered];
    }
  }

  function removeItem(id: string) {
    if (isBusy) return;
    items = items.filter((i) => i.id !== id);
  }

  function clearItems() {
    if (isBusy) return;
    items = [];
    progress = null;
    needPin = false;
  }

  function updateTextItem(id: string, newText: string) {
    items = items.map((it) => {
      if (it.id !== id) return it;
      return {
        ...it,
        text: newText,
        size: new TextEncoder().encode(newText).length,
      };
    });
  }

  // ---------- 添加文件 / 文件夹 / 文本 / 粘贴 ----------

  async function pickFiles() {
    if (isBusy) return;
    try {
      const picked = await ipc.localSendPickFiles();
      mergeItems(picked);
    } catch (e) {
      showToast(String(e));
    }
  }

  async function pickFolder() {
    if (isBusy) return;
    try {
      const picked = await ipc.localSendPickFolder();
      if (picked.length > 0) {
        mergeItems(picked);
        showToast(`已添加文件夹内 ${picked.length} 个文件`);
      }
    } catch (e) {
      showToast(String(e));
    }
  }

  function addBlankText() {
    if (isBusy) return;
    const item: LocalSendItem = {
      id: uid('ls_txt'),
      kind: 'text',
      name: `文本_${nowTag()}.txt`,
      path: null,
      text: '',
      data_url: null,
      size: 0,
      file_type: 'text/plain',
    };
    items = [item, ...items];
  }

  function onPaste(e: ClipboardEvent) {
    if (isBusy) return;
    const tag = (e.target as HTMLElement)?.tagName;
    if (tag === 'INPUT' || tag === 'TEXTAREA') return;

    const clipItems = e.clipboardData?.items;
    if (!clipItems) return;

    for (const ci of Array.from(clipItems)) {
      if (ci.kind === 'file' && ci.type.startsWith('image/')) {
        const f = ci.getAsFile();
        if (f) {
          e.preventDefault();
          const reader = new FileReader();
          reader.onload = () => {
            const dataUrl = reader.result as string;
            items = [
              {
                id: uid('ls_img'),
                kind: 'image',
                name: `剪贴板图片_${nowTag()}.png`,
                path: null,
                text: null,
                data_url: dataUrl,
                size: f.size || estimateBase64Bytes(dataUrl),
                file_type: 'image/png',
              },
              ...items,
            ];
          };
          reader.readAsDataURL(f);
          return;
        }
      }
    }

    const txt = e.clipboardData?.getData('text/plain');
    if (txt && txt.trim()) {
      e.preventDefault();
      items = [
        {
          id: uid('ls_txt'),
          kind: 'text',
          name: `剪贴板文本_${nowTag()}.txt`,
          path: null,
          text: txt,
          data_url: null,
          size: new TextEncoder().encode(txt).length,
          file_type: 'text/plain',
        },
        ...items,
      ];
    }
  }

  // ---------- 扫描局域网设备与手动连接 IP ----------

  async function scanPeers() {
    if (scanning) return;
    scanning = true;
    try {
      const list = await ipc.localSendScan(extraIps);
      peers = list;
    } catch (e) {
      showToast(`扫描异常: ${e}`);
    } finally {
      scanning = false;
    }
  }

  async function addManualIp() {
    const raw = manualIpInput.trim();
    if (!raw || probingIp) return;
    probingIp = true;
    try {
      const peer = await ipc.localSendProbe(raw);
      const exists = peers.some((p) => p.ip === peer.ip && p.port === peer.port);
      if (!exists) {
        peers = [peer, ...peers];
      }
      if (!extraIps.includes(raw)) {
        extraIps = [raw, ...extraIps].slice(0, 10);
        try {
          localStorage.setItem(STORAGE_KEY_IPS, JSON.stringify(extraIps));
        } catch {
          /* ignore */
        }
      }
      manualIpInput = '';
      showManualIp = false;
      showToast(`✓ 已连接设备：${peer.alias} (${peer.ip})`);
    } catch {
      showToast(`连接失败：请确认 ${raw} 已打开 LocalSend`);
    } finally {
      probingIp = false;
    }
  }

  // ---------- 发送文件 / 文本到目标设备 ----------

  async function sendToPeer(peer: LocalSendPeer, withPin: string | null = null) {
    if (isBusy) return;
    const validItems = items.filter((it) => (it.kind === 'text' ? (it.text?.trim().length ?? 0) > 0 : true));
    if (validItems.length === 0) {
      showToast('请先添加要发送的文件、截图或输入文本内容');
      return;
    }

    sendingPeer = peer;
    needPin = false;
    progress = {
      stage: 'waiting_accept',
      file_id: '',
      file_name: validItems[0].name,
      current_index: 0,
      total_files: validItems.length,
      sent_bytes: 0,
      total_bytes: totalBytes,
      speed_mbps: 0,
      percent: 0,
      message: `正在请求 ${peer.alias} 接收，请在对方设备上点击「接受」…`,
    };

    try {
      await ipc.localSendSend(peer, validItems, withPin, (p) => {
        progress = p;
        if (p.stage === 'pin_required') {
          needPin = true;
        }
      });
    } catch (e) {
      const msg = String(e);
      if (msg.includes('401') || needPin) {
        needPin = true;
      } else if (!progress || (progress.stage !== 'rejected' && progress.stage !== 'cancelled')) {
        progress = {
          stage: 'error',
          file_id: '',
          file_name: '',
          current_index: 0,
          total_files: validItems.length,
          sent_bytes: 0,
          total_bytes: totalBytes,
          speed_mbps: 0,
          percent: 0,
          message: msg,
        };
      }
    }
  }

  async function cancelTransfer() {
    await ipc.localSendCancel();
  }

  async function submitPinAndRetry() {
    if (!sendingPeer || !pinCode.trim()) return;
    const p = pinCode.trim();
    await sendToPeer(sendingPeer, p);
  }

  async function toggleContextMenu() {
    if (!selfInfo) return;
    try {
      const next = !selfInfo.context_menu_enabled;
      const enabled = await ipc.localSendSetContextMenu(next);
      selfInfo = { ...selfInfo, context_menu_enabled: enabled };
      showToast(enabled ? '✓ 已开启系统文件右键「发送局域网文件」菜单' : '已关闭系统文件右键菜单');
    } catch (e) {
      showToast(String(e));
    }
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
    ipc.hideSelf();
    win.hide().catch(() => {});
  }

  onMount(() => {
    try {
      const saved = localStorage.getItem(STORAGE_KEY_IPS);
      if (saved) {
        const parsed = JSON.parse(saved);
        if (Array.isArray(parsed)) extraIps = parsed;
      }
    } catch {
      /* ignore */
    }

    ipc.localSendInfo().then((info) => {
      selfInfo = info;
    });

    const unP = onPayload((p) => {
      handleIncomingPayload(p);
    });

    // 监听 Tauri 原生文件拖拽事件，直接获取本地真实路径（支持任意超大文件与文件夹流式直传）
    const unDrag = webviewWin.onDragDropEvent(async (event) => {
      if (event.payload.type === 'enter' || event.payload.type === 'over') {
        dragOver = true;
      } else if (event.payload.type === 'leave') {
        dragOver = false;
      } else if (event.payload.type === 'drop') {
        dragOver = false;
        const paths = event.payload.paths;
        if (paths && paths.length > 0) {
          const resolved = await ipc.localSendResolvePaths(paths);
          mergeItems(resolved);
        }
      }
    });

    scanPeers();

    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !isBusy) {
        e.preventDefault();
        closeWindow();
      }
    };
    window.addEventListener('keydown', onKey);
    window.addEventListener('paste', onPaste);

    return () => {
      unP.then((f) => f());
      unDrag.then((f) => f());
      window.removeEventListener('keydown', onKey);
      window.removeEventListener('paste', onPaste);
    };
  });
</script>

<div class="ls-win">
  {#if dragOver}
    <div class="drop-overlay">
      <div class="drop-card">
        <svg viewBox="0 0 24 24" width="28" height="28" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
          <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
          <polyline points="17 8 12 3 7 8" />
          <line x1="12" y1="3" x2="12" y2="15" />
        </svg>
        <div>松开鼠标添加文件或文件夹至发送列表</div>
      </div>
    </div>
  {/if}

  <!-- 顶部标题栏：与主界面统一极简黑白灰风格 -->
  <header class="ls-header">
    <div class="header-drag" data-tauri-drag-region>
      <div class="brand" data-tauri-drag-region>
        <span class="brand-badge" data-tauri-drag-region>
          <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="pointer-events:none">
            <line x1="22" y1="2" x2="11" y2="13" />
            <polygon points="22 2 15 22 11 13 2 9 22 2" />
          </svg>
        </span>
        <div class="brand-text" data-tauri-drag-region>
          <span class="brand-title" data-tauri-drag-region>局域网快传</span>
          {#if selfInfo}
            <span class="brand-sub" data-tauri-drag-region>
              {selfInfo.alias} · {selfInfo.lan_ips.join(' / ') || '未连接局域网'}
            </span>
          {/if}
        </div>
      </div>
    </div>

    <div class="win-actions no-drag" role="toolbar" tabindex="-1" onmousedown={(e) => e.stopPropagation()}>
      <button
        type="button"
        class="win-btn"
        class:active={pinned}
        onclick={togglePin}
        title={pinned ? '已固定置顶（点击取消置顶）' : '置顶窗口'}
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

  <main class="ls-body">
    <!-- 第一部分：待发送内容选择区 -->
    <section class="panel">
      <div class="panel-head">
        <div class="panel-title">
          <span>待发送内容</span>
          {#if items.length > 0}
            <span class="count-pill">{items.length} 项 · {fmtBytes(totalBytes)}</span>
          {/if}
        </div>
        <div class="action-bar">
          <button type="button" class="act-btn primary-btn" onclick={pickFiles} disabled={isBusy} title="选择单个或多个文件">
            <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" style="pointer-events:none">
              <line x1="12" y1="5" x2="12" y2="19" />
              <line x1="5" y1="12" x2="19" y2="12" />
            </svg>
            <span>文件</span>
          </button>
          <button type="button" class="act-btn" onclick={pickFolder} disabled={isBusy} title="选择整个文件夹">
            <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" style="pointer-events:none">
              <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
            </svg>
            <span>文件夹</span>
          </button>
          <button type="button" class="act-btn" onclick={addBlankText} disabled={isBusy} title="直接发送一段文字消息">
            <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" style="pointer-events:none">
              <path d="M12 20h9" />
              <path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z" />
            </svg>
            <span>文本</span>
          </button>
          {#if items.length > 0}
            <button type="button" class="act-btn ghost-danger" onclick={clearItems} disabled={isBusy} title="清空列表">
              清空
            </button>
          {/if}
        </div>
      </div>

      {#if items.length === 0}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="empty-drop" onclick={pickFiles}>
          <div class="empty-icon">
            <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
              <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
              <polyline points="17 8 12 3 7 8" />
              <line x1="12" y1="3" x2="12" y2="15" />
            </svg>
          </div>
          <div class="empty-main">点击选择文件、拖拽文件到此，或按 Ctrl+V 粘贴截图/文字</div>
          <div class="empty-sub">支持在电脑资源管理器 / Finder 中右键任意文件选择「发送局域网文件」</div>
        </div>
      {:else}
        <div class="item-list">
          {#each items as item (item.id)}
            <div class="item-row">
              {#if item.kind === 'image' && item.data_url}
                <img class="item-thumb" src={item.data_url} alt={item.name} />
                <div class="item-info">
                  <div class="item-name">{item.name}</div>
                  <div class="item-meta">截图 PNG · {fmtBytes(item.size)}</div>
                </div>
              {:else if item.kind === 'text'}
                <div class="item-type-badge txt">TXT</div>
                <div class="item-info">
                  <textarea
                    class="txt-editor"
                    rows="2"
                    placeholder="输入要直接发往对方设备的文本内容…"
                    value={item.text ?? ''}
                    oninput={(e) => updateTextItem(item.id, (e.target as HTMLTextAreaElement).value)}
                    disabled={isBusy}
                  ></textarea>
                  <div class="item-meta">文本直传 · {fmtBytes(item.size)}（对方设备将直接弹出复制框）</div>
                </div>
              {:else}
                <div class="item-type-badge file">
                  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
                    <polyline points="14 2 14 8 20 8" />
                  </svg>
                </div>
                <div class="item-info">
                  <div class="item-name" title={item.path ?? item.name}>{item.name}</div>
                  <div class="item-meta" title={item.path ?? ''}>
                    {fmtBytes(item.size)}
                    {#if item.path}
                      · <span class="path-dim">{item.path}</span>
                    {/if}
                  </div>
                </div>
              {/if}
              <button
                type="button"
                class="item-del"
                onclick={() => removeItem(item.id)}
                disabled={isBusy}
                title="移除此项"
              >
                ×
              </button>
            </div>
          {/each}
        </div>
      {/if}
    </section>

    <!-- 传输实时进度与 PIN 码面板 -->
    {#if progress}
      <section class="progress-card {progress.stage}">
        <div class="prog-top">
          <span class="prog-msg">{progress.message}</span>
          {#if isBusy}
            <button type="button" class="cancel-btn" onclick={cancelTransfer}>取消</button>
          {:else}
            <button type="button" class="cancel-btn" onclick={() => (progress = null)}>关闭</button>
          {/if}
        </div>

        {#if progress.stage === 'sending' || progress.stage === 'done'}
          <div class="prog-bar-wrap">
            <div class="prog-bar-fill" style="width: {progress.percent}%"></div>
          </div>
          <div class="prog-stats">
            <span>{fmtBytes(progress.sent_bytes)} / {fmtBytes(progress.total_bytes)}</span>
            <span>
              {#if progress.speed_mbps > 0}
                {progress.speed_mbps.toFixed(1)} MB/s ·
              {/if}
              {progress.percent}%
            </span>
          </div>
        {/if}

        {#if needPin}
          <div class="pin-box">
            <input
              type="text"
              class="pin-input"
              placeholder="输入对方设备的 PIN 码"
              bind:value={pinCode}
              onkeydown={(e) => e.key === 'Enter' && submitPinAndRetry()}
            />
            <button type="button" class="act-btn primary-btn" onclick={submitPinAndRetry}>确认重发</button>
          </div>
        {/if}
      </section>
    {/if}

    <!-- 第二部分：附近的在线 LocalSend 设备 -->
    <section class="panel peers-panel">
      <div class="panel-head">
        <div class="panel-title">
          <span class="radar-dot" class:scanning class:online={peers.length > 0}></span>
          <span>附近的设备</span>
          <span class="count-pill">{peers.length} 台在线</span>
        </div>
        <div class="action-bar">
          <button
            type="button"
            class="act-btn"
            class:active={showManualIp}
            onclick={() => (showManualIp = !showManualIp)}
            title="手动输入对方局域网 IP 地址连接"
          >
            <span>手动 IP</span>
          </button>
          <button type="button" class="act-btn" onclick={scanPeers} disabled={scanning} title="重新扫描局域网内的 LocalSend 设备">
            <svg class:spin={scanning} viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="pointer-events:none">
              <polyline points="23 4 23 10 17 10" />
              <polyline points="1 20 1 14 7 14" />
              <path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15" />
            </svg>
            <span>{scanning ? '扫描中' : '刷新'}</span>
          </button>
        </div>
      </div>

      {#if showManualIp}
        <div class="manual-ip-row">
          <input
            type="text"
            class="ip-input"
            placeholder="输入安装了 LocalSend 的设备 IP（如 192.168.1.105）"
            bind:value={manualIpInput}
            onkeydown={(e) => e.key === 'Enter' && addManualIp()}
          />
          <button type="button" class="act-btn primary-btn" onclick={addManualIp} disabled={probingIp || !manualIpInput.trim()}>
            {probingIp ? '连接中…' : '连接设备'}
          </button>
        </div>
      {/if}

      <div class="peer-list">
        {#if peers.length === 0}
          <div class="peers-empty">
            {#if scanning}
              <div class="pulse-ring"></div>
              <div class="pe-title">正在搜索同一 Wi-Fi / 局域网下的 LocalSend 设备…</div>
              <div class="pe-sub">请确保接收端手机或电脑已打开 LocalSend 应用（端口 53317）</div>
            {:else}
              <div class="pe-icon">
                <svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
                  <rect x="2" y="3" width="20" height="14" rx="2" ry="2" />
                  <line x1="8" y1="21" x2="16" y2="21" />
                  <line x1="12" y1="17" x2="12" y2="21" />
                </svg>
              </div>
              <div class="pe-title">暂未发现局域网内的其他 LocalSend 设备</div>
              <div class="pe-sub">
                请确认目标设备与本机处于同一局域网并已打开 LocalSend，或点击右上角「手动 IP」直连
              </div>
            {/if}
          </div>
        {:else}
          {#each peers as peer (peer.ip + ':' + peer.port)}
            <button
              type="button"
              class="peer-card"
              onclick={() => sendToPeer(peer)}
              disabled={isBusy}
              title={`点击立即发送给 ${peer.alias} (${peer.ip}:${peer.port})`}
            >
              <div class="peer-avatar">
                {#if peer.device_type === 'mobile'}
                  <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                    <rect x="5" y="2" width="14" height="20" rx="2" ry="2" />
                    <line x1="12" y1="18" x2="12.01" y2="18" />
                  </svg>
                {:else}
                  <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                    <rect x="2" y="3" width="20" height="14" rx="2" ry="2" />
                    <line x1="8" y1="21" x2="16" y2="21" />
                    <line x1="12" y1="17" x2="12" y2="21" />
                  </svg>
                {/if}
              </div>

              <div class="peer-main">
                <div class="peer-name">{peer.alias}</div>
                <div class="peer-tags">
                  <span class="ptag">#{peer.ip.split('.').pop()}</span>
                  <span class="ptag">{peer.device_model}</span>
                  <span class="ptag dim">{peer.ip}</span>
                </div>
              </div>

              <div class="peer-send-btn">
                <span>发送</span>
                <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <line x1="22" y1="2" x2="11" y2="13" />
                  <polygon points="22 2 15 22 11 13 2 9 22 2" />
                </svg>
              </div>
            </button>
          {/each}
        {/if}
      </div>
    </section>
  </main>

  <!-- 底部栏：系统文件右键菜单开关与协议状态 -->
  <footer class="ls-footer">
    {#if selfInfo}
      <button
        type="button"
        class="ctx-toggle"
        class:on={selfInfo.context_menu_enabled}
        onclick={toggleContextMenu}
        title="开启后，在系统文件资源管理器 / Finder 中右键任意文件或文件夹即可直接选择「发送局域网文件」"
      >
        <span class="chk-box" class:checked={selfInfo.context_menu_enabled}>
          {#if selfInfo.context_menu_enabled}
            <svg viewBox="0 0 24 24" width="10" height="10" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">
              <polyline points="20 6 9 17 4 12" />
            </svg>
          {/if}
        </span>
        <span>系统右键菜单「发送局域网文件」</span>
      </button>
    {/if}
    <span class="footer-sp"></span>
    <span class="proto-note">LocalSend v2 · 53317</span>
  </footer>

  {#if toast}
    <div class="ls-toast">{toast}</div>
  {/if}
</div>

<style>
  :global(html, body) {
    margin: 0;
    padding: 0;
    height: 100%;
    background: var(--bg-solid) !important;
  }

  .ls-win {
    display: flex;
    flex-direction: column;
    width: 100vw;
    height: 100vh;
    background: var(--bg-solid);
    color: var(--fg);
    border: none;
    border-radius: 0;
    overflow: hidden;
    position: relative;
    user-select: none;
  }

  .drop-overlay {
    position: absolute;
    inset: 0;
    z-index: 50;
    background: rgba(15, 23, 42, 0.52);
    backdrop-filter: blur(4px);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 20px;
  }

  .drop-card {
    border: 2px dashed rgba(255, 255, 255, 0.5);
    background: rgba(15, 23, 42, 0.78);
    color: #ffffff;
    border-radius: 14px;
    padding: 28px 36px;
    text-align: center;
    font-size: 13.5px;
    font-weight: 500;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    box-shadow: var(--shadow);
  }

  /* 顶部栏：与主界面 chat-header 保持一致 */
  .ls-header {
    height: 44px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 10px 0 14px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-solid);
    flex-shrink: 0;
  }

  .header-drag {
    flex: 1;
    min-width: 0;
    height: 100%;
    display: flex;
    align-items: center;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 9px;
    min-width: 0;
  }

  .brand-badge {
    width: 28px;
    height: 28px;
    border-radius: 7px;
    background: var(--panel);
    border: 1px solid var(--border);
    color: var(--fg);
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .brand-text {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }

  .brand-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--fg);
    white-space: nowrap;
  }

  .brand-sub {
    font-size: 11.5px;
    color: var(--fg-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
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
    background: transparent;
    color: var(--fg-dim);
    border-radius: 6px;
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
    color: var(--danger);
  }

  /* 主体区域 */
  .ls-body {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 14px 16px;
    overflow-y: auto;
    min-height: 0;
  }

  .panel {
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .peers-panel {
    flex: 1;
    min-height: 210px;
  }

  .panel-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    flex-wrap: wrap;
  }

  .panel-title {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 13px;
    font-weight: 600;
    color: var(--fg);
  }

  .count-pill {
    font-size: 11px;
    font-weight: 500;
    color: var(--fg-dim);
    background: var(--bg-solid);
    border: 1px solid var(--border);
    padding: 1.5px 8px;
    border-radius: 999px;
  }

  .action-bar {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .act-btn {
    height: 27px;
    padding: 0 10px;
    font-size: 12px;
    font-weight: 500;
    border-radius: 7px;
    border: 1px solid var(--border);
    background: var(--bg-solid);
    color: var(--fg);
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }

  .act-btn:hover:not(:disabled) {
    background: var(--hover);
    border-color: var(--fg-dim);
  }

  .act-btn.active {
    background: var(--hover);
    border-color: var(--fg-dim);
  }

  .act-btn.primary-btn {
    background: var(--accent);
    color: var(--accent-fg);
    border-color: transparent;
  }

  .act-btn.primary-btn:hover:not(:disabled) {
    opacity: 0.9;
    background: var(--accent);
    border-color: transparent;
  }

  .act-btn.ghost-danger {
    color: var(--fg-dim);
    border-color: transparent;
    background: transparent;
    padding: 0 6px;
  }

  .act-btn.ghost-danger:hover:not(:disabled) {
    color: var(--danger);
    background: rgba(239, 68, 68, 0.1);
    border-color: transparent;
  }

  .empty-drop {
    border: 1.5px dashed var(--border);
    border-radius: 9px;
    background: var(--bg-solid);
    padding: 18px 14px;
    text-align: center;
    cursor: pointer;
    transition: border-color 0.15s, background 0.15s;
  }

  .empty-drop:hover {
    border-color: var(--fg-dim);
    background: var(--hover);
  }

  .empty-icon {
    color: var(--fg-dim);
    margin-bottom: 6px;
    display: flex;
    justify-content: center;
  }

  .empty-main {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--fg);
  }

  .empty-sub {
    font-size: 11px;
    color: var(--fg-dim);
    margin-top: 4px;
  }

  .item-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 165px;
    overflow-y: auto;
  }

  .item-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 10px;
    background: var(--bg-solid);
    border: 1px solid var(--border);
    border-radius: 8px;
  }

  .item-thumb {
    width: 36px;
    height: 36px;
    object-fit: cover;
    border-radius: 6px;
    border: 1px solid var(--border);
    flex-shrink: 0;
  }

  .item-type-badge {
    width: 34px;
    height: 34px;
    border-radius: 7px;
    background: var(--panel);
    border: 1px solid var(--border);
    color: var(--fg-dim);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 10.5px;
    font-weight: 600;
    flex-shrink: 0;
  }

  .item-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .item-name {
    font-size: 12.5px;
    font-weight: 550;
    color: var(--fg);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .item-meta {
    font-size: 11px;
    color: var(--fg-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .path-dim {
    opacity: 0.75;
  }

  .txt-editor {
    width: 100%;
    font-size: 12.5px;
    padding: 5px 8px;
    border-radius: 6px;
    background: var(--panel);
    border: 1px solid var(--border);
    resize: vertical;
    min-height: 44px;
  }

  .txt-editor:focus {
    border-color: var(--fg-dim);
  }

  .item-del {
    width: 24px;
    height: 24px;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--fg-dim);
    font-size: 16px;
    border-radius: 6px;
    flex-shrink: 0;
  }

  .item-del:hover:not(:disabled) {
    background: rgba(239, 68, 68, 0.12);
    color: var(--danger);
  }

  /* 进度卡片 */
  .progress-card {
    padding: 10px 12px;
    border-radius: 10px;
    border: 1px solid var(--border);
    background: var(--panel);
    display: flex;
    flex-direction: column;
    gap: 7px;
  }

  .progress-card.done {
    border-color: color-mix(in srgb, var(--ok) 45%, var(--border));
  }

  .progress-card.error,
  .progress-card.rejected {
    border-color: rgba(239, 68, 68, 0.35);
    background: rgba(239, 68, 68, 0.06);
  }

  .prog-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    font-size: 12.5px;
    font-weight: 500;
  }

  .cancel-btn {
    height: 23px;
    padding: 0 9px;
    font-size: 11.5px;
    border-radius: 6px;
    background: var(--bg-solid);
  }

  .prog-bar-wrap {
    height: 5px;
    background: var(--border);
    border-radius: 999px;
    overflow: hidden;
  }

  .prog-bar-fill {
    height: 100%;
    background: var(--fg);
    transition: width 0.12s linear;
  }

  .progress-card.done .prog-bar-fill {
    background: var(--ok);
  }

  .prog-stats {
    display: flex;
    justify-content: space-between;
    font-size: 11px;
    color: var(--fg-dim);
    font-family: Consolas, monospace;
  }

  .pin-box {
    display: flex;
    gap: 6px;
    margin-top: 2px;
  }

  .pin-input {
    flex: 1;
    height: 28px;
    font-size: 12px;
    background: var(--bg-solid);
  }

  /* 在线设备区 */
  .radar-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--fg-dim);
    opacity: 0.45;
    display: inline-block;
  }

  .radar-dot.online {
    background: var(--ok);
    opacity: 1;
  }

  .radar-dot.scanning {
    background: var(--fg);
    opacity: 1;
    animation: radarPulse 1s infinite ease-in-out;
  }

  .spin {
    animation: spin 0.85s linear infinite;
  }

  @keyframes radarPulse {
    0%, 100% {
      transform: scale(0.85);
      opacity: 0.4;
    }
    50% {
      transform: scale(1.25);
      opacity: 1;
    }
  }

  .manual-ip-row {
    display: flex;
    gap: 6px;
  }

  .ip-input {
    flex: 1;
    height: 28px;
    font-size: 12px;
    background: var(--bg-solid);
  }

  .ip-input:focus {
    border-color: var(--fg-dim);
  }

  .peer-list {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 7px;
    overflow-y: auto;
  }

  .peers-empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    text-align: center;
    padding: 24px 14px;
    gap: 6px;
    background: var(--bg-solid);
    border: 1px dashed var(--border);
    border-radius: 9px;
  }

  .pe-icon {
    color: var(--fg-dim);
    opacity: 0.65;
    margin-bottom: 2px;
  }

  .pulse-ring {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    border: 2px solid var(--border);
    border-top-color: var(--fg);
    animation: spin 0.75s linear infinite;
    margin-bottom: 4px;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .pe-title {
    font-size: 12.5px;
    font-weight: 550;
    color: var(--fg);
  }

  .pe-sub {
    font-size: 11.5px;
    color: var(--fg-dim);
    max-width: 390px;
    line-height: 1.48;
  }

  .peer-card {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    text-align: left;
    padding: 10px 12px;
    border-radius: 9px;
    background: var(--bg-solid);
    border: 1px solid var(--border);
    transition: border-color 0.14s, background 0.14s;
  }

  .peer-card:hover:not(:disabled) {
    border-color: var(--fg-dim);
    background: var(--hover);
  }

  .peer-avatar {
    width: 38px;
    height: 38px;
    border-radius: 9px;
    background: var(--panel);
    border: 1px solid var(--border);
    color: var(--fg);
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .peer-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .peer-name {
    font-size: 13.5px;
    font-weight: 600;
    color: var(--fg);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .peer-tags {
    display: flex;
    align-items: center;
    gap: 5px;
    flex-wrap: wrap;
  }

  .ptag {
    font-size: 11px;
    padding: 1px 6px;
    border-radius: 4px;
    background: var(--panel);
    border: 1px solid var(--border);
    color: var(--fg);
    font-family: Consolas, monospace;
  }

  .ptag.dim {
    color: var(--fg-dim);
  }

  .peer-send-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 5px 11px;
    border-radius: 7px;
    background: var(--accent);
    color: var(--accent-fg);
    font-size: 12px;
    font-weight: 500;
    flex-shrink: 0;
  }

  .ls-footer {
    height: 36px;
    display: flex;
    align-items: center;
    padding: 0 14px;
    border-top: 1px solid var(--border);
    background: var(--panel);
    font-size: 11.5px;
    color: var(--fg-dim);
    flex-shrink: 0;
  }

  .ctx-toggle {
    border: none;
    background: transparent;
    color: var(--fg-dim);
    padding: 3px 6px;
    font-size: 11.5px;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    border-radius: 6px;
  }

  .ctx-toggle:hover {
    background: var(--hover);
    color: var(--fg);
  }

  .ctx-toggle.on {
    color: var(--fg);
  }

  .chk-box {
    width: 14px;
    height: 14px;
    border-radius: 4px;
    border: 1px solid var(--fg-dim);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: var(--bg-solid);
    transition: background 0.12s, border-color 0.12s;
  }

  .chk-box.checked {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-fg);
  }

  .footer-sp {
    flex: 1;
  }

  .proto-note {
    font-size: 11px;
    opacity: 0.7;
  }

  .ls-toast {
    position: fixed;
    bottom: 44px;
    left: 50%;
    transform: translateX(-50%);
    background: var(--fg);
    color: var(--bg-solid);
    padding: 6px 14px;
    border-radius: 8px;
    font-size: 12px;
    font-weight: 500;
    box-shadow: var(--shadow);
    z-index: 99;
    max-width: 90%;
  }
</style>
