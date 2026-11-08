<script lang="ts">
  // 通用 Edge TTS 语音朗读按钮组件：支持仅图标模式 / 图标+文字胶囊模式，内置合成旋转态与动态声波播放态
  import { toggleSpeak, tts } from '$lib/core/tts.svelte';

  let {
    id,
    text,
    label = '',
    activeLabel = '停止',
    loadingLabel = '合成中',
    title = '使用 Edge TTS 语音朗读',
    variant = 'ghost',
    onFinish,
  }: {
    id: string;
    text: string;
    label?: string;
    activeLabel?: string;
    loadingLabel?: string;
    title?: string;
    variant?: 'ghost' | 'pill' | 'tip';
    onFinish?: () => void;
  } = $props();

  const isCurrent = $derived(tts.activeId === id);
  const isLoading = $derived(isCurrent && tts.status === 'loading');
  const isPlaying = $derived(isCurrent && tts.status === 'playing');
  const isActive = $derived(isLoading || isPlaying);

  function handleClick(e: MouseEvent) {
    e.stopPropagation();
    if (!text?.trim()) return;
    toggleSpeak(id, text, { onFinish });
  }
</script>

<button
  type="button"
  class="speak-btn {variant}"
  class:active={isActive}
  class:playing={isPlaying}
  onclick={handleClick}
  disabled={!text?.trim()}
  title={isActive ? '点击停止朗读' : title}
>
  {#if isLoading}
    <span class="tts-spinner" aria-hidden="true"></span>
  {:else if isPlaying}
    <span class="tts-wave" aria-hidden="true">
      <i></i><i></i><i></i>
    </span>
  {:else}
    <svg
      class="tts-icon"
      viewBox="0 0 24 24"
      width="13.5"
      height="13.5"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" />
      <path d="M15.54 8.46a5 5 0 0 1 0 7.07" />
      <path d="M19.07 4.93a10 10 0 0 1 0 14.14" />
    </svg>
  {/if}

  {#if label}
    <span class="tts-label">
      {#if isLoading}
        {loadingLabel}
      {:else if isPlaying}
        {activeLabel}
      {:else}
        {label}
      {/if}
    </span>
  {/if}
</button>

<style>
  .speak-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 4px;
    border-radius: 5px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--fg-dim);
    font-size: 11.5px;
    line-height: 1;
    padding: 3px 7px;
    cursor: pointer;
    user-select: none;
    transition: background 0.14s ease, color 0.14s ease, border-color 0.14s ease;
  }
  .speak-btn:hover:not(:disabled) {
    color: var(--fg);
    background: var(--hover);
  }
  .speak-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  /* Tip 胶囊栏无边框紧凑样式 */
  .speak-btn.tip {
    padding: 0 8px;
    height: 28px;
    border: none;
    border-radius: 6px;
    color: var(--fg);
    font-size: 12.5px;
    font-weight: 500;
    gap: 4.5px;
  }

  /* 仅图标或轻量内联按钮 */
  .speak-btn.ghost {
    padding: 2px 7px;
    height: 22px;
  }

  /* 底部动作栏药丸样式 */
  .speak-btn.pill {
    padding: 2px 9px;
    height: 24px;
    font-size: 11.5px;
    border-radius: 4px;
    color: var(--fg);
  }

  /* 激活态（合成中 / 朗读中） */
  .speak-btn.active {
    color: var(--accent-blue, #3b82f6);
    border-color: color-mix(in srgb, var(--accent-blue, #3b82f6) 45%, transparent);
    background: color-mix(in srgb, var(--accent-blue, #3b82f6) 10%, transparent);
    font-weight: 500;
  }
  .speak-btn.tip.active {
    background: color-mix(in srgb, var(--accent-blue, #3b82f6) 14%, transparent);
  }

  .tts-icon {
    flex-shrink: 0;
  }

  /* 合成加载旋转圈 */
  .tts-spinner {
    width: 11px;
    height: 11px;
    border: 1.8px solid currentColor;
    border-right-color: transparent;
    border-radius: 50%;
    animation: tts-spin 0.65s linear infinite;
    flex-shrink: 0;
  }
  @keyframes tts-spin {
    to {
      transform: rotate(360deg);
    }
  }

  /* 动态三柱声波动画 */
  .tts-wave {
    display: inline-flex;
    align-items: flex-end;
    gap: 1.8px;
    height: 12px;
    width: 12px;
    justify-content: center;
    flex-shrink: 0;
  }
  .tts-wave i {
    display: block;
    width: 2px;
    background: currentColor;
    border-radius: 1px;
    animation: tts-bar 0.75s ease-in-out infinite;
  }
  .tts-wave i:nth-child(1) {
    height: 5px;
    animation-delay: 0s;
  }
  .tts-wave i:nth-child(2) {
    height: 11px;
    animation-delay: 0.18s;
  }
  .tts-wave i:nth-child(3) {
    height: 7px;
    animation-delay: 0.34s;
  }
  @keyframes tts-bar {
    0%,
    100% {
      transform: scaleY(0.45);
    }
    50% {
      transform: scaleY(1);
    }
  }
</style>
