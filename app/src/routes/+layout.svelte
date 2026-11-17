<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import '$lib/styles.css';
  import { onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { ipc, onSettings } from '$lib/core/ipc';
  import type { Settings } from '$lib/core/types';

  let { children } = $props();
  const win = getCurrentWindow();
  const tabs = [
    { label: '聊天', href: '/launcher' },
    { label: '快传', href: '/localsend' },
    { label: '快聊日志', href: '/history' },
    { label: '设置', href: '/settings' },
  ] as const;
  let pinned = $state(false);
  const isMainPage = $derived(tabs.some((tab) => tab.href === page.url.pathname));
  const activeTab = $derived(tabs.find((tab) => tab.href === page.url.pathname) ?? tabs[0]);

  $effect(() => {
    if (typeof document === 'undefined') return;
    if (isMainPage) document.body.dataset.mainShell = 'true';
    else delete document.body.dataset.mainShell;
    win.setTitle(`miniBot · ${activeTab.label}`).catch(() => {});
  });

  async function togglePin() {
    pinned = !pinned;
    await win.setAlwaysOnTop(pinned).catch(() => {});
  }

  function closeMainWindow() {
    ipc.hideSelf();
  }

  const DEFAULT_FONT = "'Inter', 'Segoe UI Variable', 'Segoe UI', 'PingFang SC', 'Microsoft YaHei UI', system-ui, sans-serif";

  function applyUi(s?: Settings | null) {
    if (!s?.ui || typeof document === 'undefined') return;
    const root = document.documentElement;
    const theme = s.ui.theme || 'system';
    root.dataset.theme = theme;
    const fam = s.ui.font_family?.trim();
    root.style.setProperty('--app-font', fam ? `${fam}, ${DEFAULT_FONT}` : DEFAULT_FONT);
    root.style.setProperty('--app-font-size', `${Math.max(12, Math.min(22, s.ui.font_size || 14))}px`);
    root.style.setProperty('--app-line-height', String(Math.max(1.3, Math.min(2.4, s.ui.line_height || 1.68))));
  }

  onMount(() => {
    const sync = () => ipc.getSettings().then(applyUi).catch(() => {});
    sync();
    window.addEventListener('focus', sync);
    const un = onSettings(applyUi);
    return () => {
      window.removeEventListener('focus', sync);
      un.then((f) => f());
    };
  });
</script>

{#if isMainPage}
  <div class="main-shell">
    <header class="main-topbar">
      <span class="topbar-space" aria-hidden="true"></span>
      <nav class="main-tabs" aria-label="主导航">
        {#each tabs as tab (tab.href)}
          <button
            class="main-tab"
            class:active={page.url.pathname === tab.href}
            aria-current={page.url.pathname === tab.href ? 'page' : undefined}
            onclick={() => goto(tab.href)}
          >{tab.label}</button>
        {/each}
      </nav>
      <div class="window-actions">
        <button class="window-action pin" class:active={pinned} onclick={togglePin} title={pinned ? '取消置顶' : '窗口置顶'} aria-label={pinned ? '取消置顶' : '窗口置顶'}>
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 4h8l-1 5 3 3v2h-5v6l-1 1-1-1v-6H6v-2l3-3-1-5Z" /></svg>
        </button>
        <button class="window-action" onclick={() => ipc.minimizeSelf()} title="最小化" aria-label="最小化">
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 12h14" /></svg>
        </button>
        <button class="window-action close" onclick={closeMainWindow} title="隐藏窗口" aria-label="隐藏窗口">
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" /></svg>
        </button>
      </div>
    </header>
    <main class="main-view">{@render children()}</main>
  </div>
{:else}
  {@render children()}
{/if}

<style>
  .main-shell {
    display: flex;
    flex-direction: column;
    width: 100vw;
    height: 100vh;
    overflow: hidden;
    background: var(--bg-solid);
    color: var(--fg);
  }
  .main-topbar {
    display: grid;
    grid-template-columns: minmax(150px, 1fr) auto minmax(150px, 1fr);
    align-items: center;
    height: 44px;
    padding: 0 12px 0 16px;
    border-bottom: 1px solid var(--border);
    flex: 0 0 44px;
    -webkit-app-region: drag;
  }
  .main-tabs {
    display: flex;
    align-items: center;
    gap: 12px;
    -webkit-app-region: no-drag;
  }
  .main-tab {
    min-width: 68px;
    height: 42px;
    padding: 0 10px;
    border: 0;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    background: transparent;
    color: var(--fg-dim);
    font-size: 13px;
    transition: color var(--transition), border-color var(--transition);
  }
  .main-tab:hover { color: var(--fg); background: transparent; }
  .main-tab.active {
    color: var(--fg);
    border-bottom-color: var(--accent-blue);
    font-weight: 500;
  }
  .window-actions {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 4px;
    -webkit-app-region: no-drag;
  }
  .window-action {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    height: 30px;
    padding: 0;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--fg-dim);
  }
  .window-action:hover { background: var(--hover); color: var(--fg); }
  .window-action.active { color: var(--accent-blue); background: transparent; }
  .window-action.close:hover { color: var(--danger); background: color-mix(in srgb, var(--danger) 12%, transparent); }
  .window-action svg { width: 16px; height: 16px; fill: none; stroke: currentColor; stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }
  .window-action.pin svg { fill: none; }
  .main-view { flex: 1; min-width: 0; min-height: 0; overflow: hidden; }

  :global(body[data-main-shell='true'] .window),
  :global(body[data-main-shell='true'] .page),
  :global(body[data-main-shell='true'] .ls-win) {
    width: 100% !important;
    height: 100% !important;
    min-height: 0;
  }
  :global(body[data-main-shell='true'] .main-shell .win-actions),
  :global(body[data-main-shell='true'] .main-shell .win-ctrl) { display: none !important; }

  @media (max-width: 720px) {
    .main-topbar { grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr); padding-inline: 8px; }
    .main-tabs { justify-self: center; gap: 1px; }
    .main-tab { min-width: 0; padding-inline: 8px; }
    .window-actions { gap: 0; }
  }
</style>
