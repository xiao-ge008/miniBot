<script lang="ts">
  // 显卡繁忙提示：当显存超过 15GB 自动释放模型（含正在使用中中断）或剩余显存不足时展示
  import type { GpuSnapshot } from '$lib/core/types';
  let {
    snapshot,
    required,
    interrupted = false,
    onforce,
    oncancel,
  }: {
    snapshot: GpuSnapshot;
    required: number;
    interrupted?: boolean;
    onforce: () => void;
    oncancel: () => void;
  } = $props();
  const over15g = $derived(snapshot.used_mb >= 15 * 1024);
</script>

<div class="gpu">
  <div class="title">
    {interrupted ? '⚠ 显存超过 15GB，已自动中断并释放模型' : '⚠ 显卡正忙（已自动释放模型让道）'}
  </div>
  <div class="sub">
    已用 {(snapshot.used_mb / 1024).toFixed(1)} GB / {(snapshot.total_mb / 1024).toFixed(1)} GB（剩余 {snapshot.free_mb} MB，模型需约 {required} MB） · 占用率 {snapshot.util}%
  </div>
  {#if over15g}
    <div class="warn">
      检测到显存占用已超过 15 GB，为保障当前大型任务流畅运行，已自动释放本地模型归还全部显存。
    </div>
  {/if}
  <ul>
    {#each snapshot.processes as p (p.pid)}
      <li>
        <span class="name">{p.name}</span>
        <span class="mem">{p.used_mb != null ? `~${p.used_mb} MB` : ''}</span>
      </li>
    {:else}
      <li class="dim">未获取到具体进程（可能是游戏或全屏渲染程序）</li>
    {/each}
  </ul>
  <div class="btns">
    <button class="primary" onclick={onforce}>仍然强制加载</button>
    <button class="ghost" onclick={oncancel}>稍后再用</button>
  </div>
</div>

<style>
  .gpu {
    padding: 12px;
    border: 1px solid var(--warn);
    border-radius: var(--radius);
    background: color-mix(in srgb, var(--warn) 8%, transparent);
  }
  .title {
    font-weight: 600;
    margin-bottom: 4px;
  }
  .sub,
  .dim {
    color: var(--fg-dim);
    font-size: 12px;
  }
  .warn {
    margin-top: 6px;
    font-size: 12px;
    color: var(--warn);
  }
  ul {
    list-style: none;
    padding: 0;
    margin: 8px 0;
    max-height: 120px;
    overflow: auto;
  }
  li {
    display: flex;
    justify-content: space-between;
    padding: 3px 0;
    border-bottom: 1px dashed var(--border);
    font-size: 13px;
  }
  .mem {
    color: var(--fg-dim);
  }
  .btns {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
</style>
