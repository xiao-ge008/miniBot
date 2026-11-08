<script lang="ts">
  import '$lib/styles.css';
  import { onMount } from 'svelte';
  import { ipc, onSettings } from '$lib/core/ipc';
  import type { Settings } from '$lib/core/types';

  let { children } = $props();

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

{@render children()}
