<script lang="ts">
  // 颜色视图：复制的是颜色代码时，显示色块与多种格式
  import { ipc } from '$lib/core/ipc';
  import { parseColor, toHex, toHslStr, toRgbStr } from '$lib/core/color';
  import type { Payload } from '$lib/core/types';

  let { payload }: { payload: Payload } = $props();
  const c = $derived(parseColor(payload.text ?? ''));
  let copied = $state('');

  async function copy(t: string) {
    await ipc.copyText(t);
    copied = t;
    setTimeout(() => (copied = ''), 1200);
  }
</script>

<div class="color">
  {#if c}
    <div class="sw" style="background:{toHex(c)}"></div>
    {#each [toHex(c), toRgbStr(c), toHslStr(c), toHex(c).toLowerCase(), `${c[0]}, ${c[1]}, ${c[2]}`] as f (f)}
      <button class="fmt" onclick={() => copy(f)}>
        <span>{f}</span><span class="dim">{copied === f ? '✔ 已复制' : '复制'}</span>
      </button>
    {/each}
  {:else}
    <div class="dim">无法解析颜色：{payload.text}</div>
  {/if}
</div>

<style>
  .color {
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .sw {
    height: 90px;
    border-radius: var(--radius);
    border: 1px solid var(--border);
  }
  .fmt {
    display: flex;
    justify-content: space-between;
    font-family: 'Cascadia Code', Consolas, monospace;
  }
  .dim {
    color: var(--fg-dim);
    font-size: 12px;
  }
</style>
