<script lang="ts">
  // 极简提示词目录入口：仅向高级用户展示提示词目录路径与快速打开/恢复默认操作
  import { onMount } from 'svelte';
  import { ipc } from '$lib/core/ipc';

  let {
    onNotify,
    basePath = '',
  }: {
    onNotify?: (msg: string, ms?: number) => void;
    basePath?: string;
  } = $props();

  let promptsDir = $state('');
  let resetting = $state(false);

  async function refreshDir() {
    try {
      const list = await ipc.kbListPrompts();
      const first = list[0]?.path;
      if (first) {
        promptsDir = first.replace(/[\\/]+(vision|clip)[\\/]+[^\\/]+$/i, '');
      }
    } catch {
      /* 忽略目录探测异常 */
    }
  }

  async function handleOpenFolder() {
    try {
      const path = await ipc.kbOpenPromptsFolder('work');
      if (path) promptsDir = path;
    } catch (e) {
      onNotify?.(`打开提示词目录失败: ${e}`, 3200);
    }
  }

  async function handleResetDefaults() {
    if (resetting) return;
    if (!confirm('确定将所有内置提示词恢复为默认模板吗？（自行新增的文件不会被删除）')) {
      return;
    }
    resetting = true;
    try {
      await ipc.kbResetPrompt(null, null);
      await refreshDir();
      onNotify?.('✓ 已恢复所有内置提示词为默认模板');
    } catch (e) {
      onNotify?.(`恢复默认失败: ${e}`, 3200);
    } finally {
      resetting = false;
    }
  }

  $effect(() => {
    void basePath;
    refreshDir();
  });

  onMount(() => {
    refreshDir();
  });
</script>

<div class="pm-row">
  <div class="pm-meta">
    <div class="pm-title">提示词文件夹（进阶）</div>
    <div class="pm-desc">自定义图片识别与文本提纯的 <code>.md</code> 提示词文件，保存即生效</div>
  </div>
  <div class="pm-ctrl">
    <input
      type="text"
      class="txt pm-path"
      readonly
      value={promptsDir || '知识库目录/.prompts'}
      title={promptsDir || '知识库目录/.prompts'}
    />
    <button type="button" class="btn pm-btn" onclick={handleOpenFolder} title="打开提示词文件夹">
      📂 打开
    </button>
    <button
      type="button"
      class="btn pm-btn"
      onclick={handleResetDefaults}
      disabled={resetting}
      title="恢复内置提示词为默认版本"
    >
      {resetting ? '⏳' : '↺ 重置'}
    </button>
  </div>
</div>

<style>
  .pm-row {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px 16px;
  }
  .pm-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .pm-title {
    font-size: 13px;
    font-weight: 500;
    color: var(--fg);
  }
  .pm-desc {
    font-size: 11.5px;
    color: var(--fg-dim);
  }
  .pm-desc code {
    font-size: 11px;
    padding: 0 4px;
    border-radius: 3px;
    background: color-mix(in srgb, var(--fg) 8%, transparent);
  }
  .pm-ctrl {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .pm-path {
    flex: 1;
    min-width: 0;
    font-size: 12px;
    color: var(--fg-dim);
  }
  .pm-btn {
    white-space: nowrap;
  }
</style>
