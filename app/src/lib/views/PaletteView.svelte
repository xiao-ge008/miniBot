<script lang="ts">
  // 配色提取视图：K-means 主色，点击复制，可导出 CSS 变量 / Tailwind
  import { onMount } from 'svelte';
  import { ipc } from '$lib/core/ipc';
  import { extractPalette, imageData, toHex, toHslStr, toRgbStr, type RGB } from '$lib/core/color';
  import type { Payload } from '$lib/core/types';

  let { payload }: { payload: Payload } = $props();
  let colors = $state<{ color: RGB; ratio: number }[]>([]);
  let k = $state(6);
  let toast = $state('');
  let data: ImageData | null = null;

  async function compute() {
    if (!data) data = await imageData(payload.image!);
    colors = extractPalette(data.data, k);
  }

  onMount(compute);

  async function copy(text: string, tip = text) {
    await ipc.copyText(text);
    toast = `已复制 ${tip}`;
    setTimeout(() => (toast = ''), 1400);
  }

  const css = () => `:root {\n${colors.map((c, i) => `  --color-${i + 1}: ${toHex(c.color)};`).join('\n')}\n}`;
  const tailwind = () =>
    `colors: {\n  brand: {\n${colors.map((c, i) => `    ${(i + 1) * 100}: '${toHex(c.color)}',`).join('\n')}\n  },\n}`;
</script>

<div class="pal">
  {#if payload.image}<img class="src" src={payload.image} alt="" />{/if}
  <div class="row">
    <label>颜色数 <input type="range" min="3" max="10" bind:value={k} onchange={compute} /> {k}</label>
  </div>
  <div class="list">
    {#each colors as c, i (i)}
      <div class="item">
        <button class="sw" style="background:{toHex(c.color)}" onclick={() => copy(toHex(c.color))} title="复制 HEX"></button>
        <div class="info">
          <button class="ghost code" onclick={() => copy(toHex(c.color))}>{toHex(c.color)}</button>
          <button class="ghost code dim" onclick={() => copy(toRgbStr(c.color))}>{toRgbStr(c.color)}</button>
          <button class="ghost code dim" onclick={() => copy(toHslStr(c.color))}>{toHslStr(c.color)}</button>
        </div>
        <div class="ratio">{(c.ratio * 100).toFixed(1)}%</div>
      </div>
    {/each}
  </div>
  <div class="btns">
    <button onclick={() => copy(css(), 'CSS 变量')}>导出 CSS 变量</button>
    <button onclick={() => copy(tailwind(), 'Tailwind 配置')}>导出 Tailwind</button>
    <button onclick={() => copy(colors.map((c) => toHex(c.color)).join(', '), '全部 HEX')}>复制全部 HEX</button>
  </div>
  {#if toast}<div class="toast">{toast}</div>{/if}
</div>

<style>
  .pal {
    padding: 8px 12px;
    overflow: auto;
    height: 100%;
  }
  .src {
    max-height: 80px;
    max-width: 100%;
    border-radius: 6px;
  }
  .row {
    margin: 6px 0;
    color: var(--fg-dim);
    font-size: 12px;
  }
  .row input {
    vertical-align: middle;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 0;
  }
  .sw {
    width: 38px;
    height: 38px;
    border-radius: 8px;
    border: 1px solid var(--border);
    padding: 0;
  }
  .info {
    display: flex;
    flex-wrap: wrap;
    flex: 1;
  }
  .code {
    font-family: 'Cascadia Code', Consolas, monospace;
    font-size: 12.5px;
    padding: 2px 6px;
  }
  .dim {
    color: var(--fg-dim);
  }
  .ratio {
    color: var(--fg-dim);
    font-size: 12px;
  }
  .btns {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    margin-top: 8px;
  }
  .toast {
    position: fixed;
    bottom: 12px;
    left: 50%;
    transform: translateX(-50%);
    background: var(--fg);
    color: var(--bg-solid);
    padding: 4px 12px;
    border-radius: 6px;
    font-size: 12px;
  }
</style>
