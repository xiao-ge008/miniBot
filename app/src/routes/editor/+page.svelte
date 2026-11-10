<script lang="ts">
  // 高性能极简 Markdown 查看器与修改器 (基于 CodeMirror 6)
  // 支持分屏实时预览 / 纯编辑 / 纯阅读模式、热保存、知识库增量索引同步、关键词快速定位
  import { onMount, tick } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { EditorView, basicSetup } from 'codemirror';
  import { markdown } from '@codemirror/lang-markdown';
  import { EditorState } from '@codemirror/state';
  import { ipc, onDocOpen } from '$lib/core/ipc';
  import { renderMarkdown } from '$lib/core/markdown';
  import type { DocFileInfo } from '$lib/core/types';

  type ViewMode = 'split' | 'edit' | 'preview';

  let editorContainer: HTMLDivElement;
  let previewContainer: HTMLDivElement;
  let editorView: EditorView | null = null;

  let filePath = $state('');
  let fileName = $state('未命名.md');
  let content = $state('');
  let fileSize = $state(0);
  let lastSavedMs = $state(0);
  let isKbFile = $state(false);
  let kbReindexed = $state(false);

  let dirty = $state(false);
  let saving = $state(false);
  let loading = $state(false);
  let err = $state('');
  let toastMsg = $state('');
  let viewMode = $state<ViewMode>('split');
  let isPinned = $state(false);
  let isMaximized = $state(false);

  const win = getCurrentWindow();

  // 统计信息
  const wordCount = $derived(content.trim() ? content.trim().length : 0);
  const lineCount = $derived(content ? content.split('\n').length : 1);

  function formatBytes(bytes: number): string {
    if (bytes <= 0) return '0 B';
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }

  function formatTime(ms: number): string {
    if (!ms) return '';
    const d = new Date(ms);
    return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
  }

  function flash(msg: string) {
    toastMsg = msg;
    setTimeout(() => {
      if (toastMsg === msg) toastMsg = '';
    }, 2200);
  }

  async function loadFile(targetPath: string, searchHighlight?: string | null) {
    if (!targetPath) return;
    loading = true;
    err = '';
    try {
      const doc: DocFileInfo = await ipc.docRead(targetPath);
      filePath = doc.path;
      fileName = doc.name;
      content = doc.content;
      fileSize = doc.size;
      lastSavedMs = doc.modified_ms;
      isKbFile = doc.is_kb_file;
      dirty = false;

      await tick();
      initOrUpdateEditor(doc.content, searchHighlight);
    } catch (e: any) {
      err = String(e?.message ?? e);
      flash(`加载失败: ${err}`);
    } finally {
      loading = false;
    }
  }

  function initOrUpdateEditor(text: string, searchHighlight?: string | null) {
    if (!editorContainer) return;

    if (editorView) {
      editorView.destroy();
      editorView = null;
    }

    const updateListener = EditorView.updateListener.of((update) => {
      if (update.docChanged) {
        content = update.state.doc.toString();
        dirty = true;
      }
    });

    const customTheme = EditorView.theme(
      {
        '&': {
          height: '100%',
          fontSize: '13.5px',
          fontFamily:
            'ui-monospace, "Cascadia Code", "Fira Code", Consolas, "PingFang SC", "Microsoft YaHei", monospace',
          backgroundColor: 'transparent',
          color: 'var(--fg)',
        },
        '.cm-scroller': {
          overflow: 'auto',
          lineHeight: '1.65',
        },
        '.cm-content': {
          padding: '14px 18px',
          caretColor: 'var(--accent-blue)',
        },
        '.cm-line': {
          padding: '0 4px',
        },
        '.cm-gutters': {
          backgroundColor: 'transparent',
          color: 'var(--fg-dim)',
          borderRight: '1px solid var(--border)',
          opacity: 0.75,
          paddingRight: '6px',
        },
        '.cm-activeLineGutter': {
          backgroundColor: 'rgba(59, 130, 246, 0.12)',
          color: 'var(--accent-blue)',
          fontWeight: '600',
        },
        '.cm-activeLine': {
          backgroundColor: 'rgba(59, 130, 246, 0.05)',
        },
        '.cm-cursor': {
          borderLeftColor: 'var(--accent-blue)',
          borderLeftWidth: '2px',
        },
        '.cm-selectionBackground, ::selection': {
          backgroundColor: 'rgba(59, 130, 246, 0.28) !important',
        },
      },
      { dark: true },
    );

    const startState = EditorState.create({
      doc: text,
      extensions: [
        basicSetup,
        markdown(),
        updateListener,
        customTheme,
        EditorView.lineWrapping,
      ],
    });

    editorView = new EditorView({
      state: startState,
      parent: editorContainer,
    });

    // 关键词锚点自动定位并滚动
    if (searchHighlight && searchHighlight.trim()) {
      locateQuery(searchHighlight.trim());
    }
  }

  function locateQuery(q: string) {
    if (!editorView || !q) return;
    const lowerText = editorView.state.doc.toString().toLowerCase();
    const lowerQuery = q.toLowerCase();
    const pos = lowerText.indexOf(lowerQuery);
    if (pos >= 0) {
      editorView.dispatch({
        selection: { anchor: pos, head: pos + q.length },
        scrollIntoView: true,
      });
      editorView.focus();
    }
  }

  async function saveDoc() {
    if (!filePath || saving) return;
    saving = true;
    try {
      const res = await ipc.docWrite(filePath, content);
      dirty = false;
      lastSavedMs = res.modified_ms;
      fileSize = res.size;
      kbReindexed = res.kb_reindexed;
      flash(res.kb_reindexed ? '✓ 已保存并同步至知识库！' : '✓ 文件保存成功！');
    } catch (e: any) {
      flash(`保存失败: ${e?.message ?? e}`);
    } finally {
      saving = false;
    }
  }

  async function togglePin() {
    isPinned = !isPinned;
    await win.setAlwaysOnTop(isPinned);
  }

  async function toggleMaximize() {
    isMaximized = !isMaximized;
    if (isMaximized) {
      await win.maximize();
    } else {
      await win.unmaximize();
    }
  }

  function closeWin() {
    if (dirty) {
      if (!confirm('当前文档有未保存的修改，确定要退出吗？')) {
        return;
      }
    }
    win.close();
  }

  function onKey(e: KeyboardEvent) {
    const mod = e.ctrlKey || e.metaKey;
    if (mod && e.key.toLowerCase() === 's') {
      e.preventDefault();
      saveDoc();
    } else if (mod && e.key === '/') {
      e.preventDefault();
      viewMode = viewMode === 'split' ? 'edit' : viewMode === 'edit' ? 'preview' : 'split';
    } else if (e.key === 'Escape') {
      closeWin();
    }
  }

  onMount(() => {
    window.addEventListener('keydown', onKey);

    // 监听后续从其他窗口传入的新文档请求
    const unlistenDoc = onDocOpen((payload) => {
      if (payload.path) {
        loadFile(payload.path, payload.query);
      }
    });

    // 读取初次启动可能携带的待编辑文件
    ipc.docGetPending().then((res) => {
      if (res && res[0]) {
        loadFile(res[0], res[1]);
      }
    });

    return () => {
      window.removeEventListener('keydown', onKey);
      unlistenDoc.then((f) => f());
      if (editorView) editorView.destroy();
    };
  });
</script>

<div class="editor-window">
  <!-- 顶栏：自定义无边框拖拽标题栏与全能工具栏 -->
  <header class="title-bar" data-tauri-drag-region>
    <div class="file-meta" data-tauri-drag-region>
      <span class="file-icon">📝</span>
      <span class="file-name" title={filePath} data-tauri-drag-region>
        {fileName}
      </span>
      {#if dirty}
        <span class="dirty-badge" title="有未保存的修改">• 未保存</span>
      {/if}
      {#if isKbFile}
        <span class="kb-badge" title="此文件收录于个人知识库中，保存将自动增量构建索引">🧠 知识库</span>
      {/if}
    </div>

    <div class="center-meta" data-tauri-drag-region>
      <span class="stat-pill">{lineCount} 行</span>
      <span class="stat-pill">{wordCount} 字</span>
      <span class="stat-pill">{formatBytes(fileSize)}</span>
      {#if lastSavedMs > 0}
        <span class="save-time">上次保存 {formatTime(lastSavedMs)}</span>
      {/if}
    </div>

    <div class="actions">
      <!-- 视图模式切换 -->
      <div class="view-tabs">
        <button
          class="v-tab"
          class:active={viewMode === 'split'}
          title="双栏分屏：实时代码与渲染 (Ctrl+/)"
          onclick={() => (viewMode = 'split')}
        >
          ◫ 分屏
        </button>
        <button
          class="v-tab"
          class:active={viewMode === 'edit'}
          title="纯编辑模式：无干扰代码编辑"
          onclick={() => (viewMode = 'edit')}
        >
          ✎ 编辑
        </button>
        <button
          class="v-tab"
          class:active={viewMode === 'preview'}
          title="沉浸预览模式：完整排版阅读"
          onclick={() => (viewMode = 'preview')}
        >
          👁 预览
        </button>
      </div>

      <button
        class="act-btn primary"
        disabled={saving || !dirty}
        onclick={saveDoc}
        title="保存文件并同步知识库 (Ctrl+S)"
      >
        <span>{saving ? '保存中…' : '💾 保存'}</span>
      </button>

      {#if filePath}
        <button
          class="act-btn"
          onclick={() => ipc.kbRevealFile(filePath)}
          title="在系统文件资源管理器中定位"
        >
          📂 定位
        </button>
      {/if}

      <span class="sep"></span>

      <button
        class="win-btn"
        class:on={isPinned}
        onclick={togglePin}
        title={isPinned ? '取消置顶' : '置顶窗口'}
      >
        📌
      </button>
      <button
        class="win-btn"
        onclick={toggleMaximize}
        title={isMaximized ? '还原窗口' : '最大化窗口'}
      >
        {isMaximized ? '❐' : '□'}
      </button>
      <button
        class="win-btn close"
        onclick={closeWin}
        title="关闭窗口 (Esc)"
      >
        ✕
      </button>
    </div>
  </header>

  <!-- 主编辑区：CodeMirror 6 极速编辑 + 实时 Markdown 流式预览 -->
  <main class="main-body" class:mode-edit={viewMode === 'edit'} class:mode-prev={viewMode === 'preview'}>
    {#if loading}
      <div class="state-overlay">
        <div class="pulse-ring"></div>
        <span>正在极速载入文档…</span>
      </div>
    {:else if err}
      <div class="state-overlay err-box">
        <div class="err-title">❌ 加载文档失败</div>
        <div class="err-desc">{err}</div>
        <button class="retry-btn" onclick={() => loadFile(filePath)}>重新加载</button>
      </div>
    {/if}

    <!-- 左栏：CodeMirror 6 编辑容器 -->
    <div
      class="editor-pane"
      class:hidden={viewMode === 'preview'}
      bind:this={editorContainer}
    ></div>

    <!-- 中间分割线（分屏时显示） -->
    {#if viewMode === 'split'}
      <div class="split-divider"></div>
    {/if}

    <!-- 右栏：实时 Markdown 渲染预览 -->
    <div
      class="preview-pane"
      class:hidden={viewMode === 'edit'}
      bind:this={previewContainer}
    >
      <div class="md-render selectable">
        {@html renderMarkdown(content || '*(文档内容为空)*')}
      </div>
    </div>
  </main>

  <!-- 底部状态栏 -->
  <footer class="footer-bar">
    <button
      type="button"
      class="path-btn"
      title="点击复制完整路径"
      onclick={() => filePath && ipc.copyText(filePath)}
    >
      {filePath || '未载入文件'}
    </button>
    <span class="spacer"></span>
    {#if isKbFile}
      <span class="sync-tag">⚡ 已关联知识库 FTS5 引擎</span>
    {/if}
    <span class="enc-tag">UTF-8</span>
  </footer>

  <!-- 浮动通知 Toast -->
  {#if toastMsg}
    <div class="toast">{toastMsg}</div>
  {/if}
</div>

<style>
  :global(html, body) {
    margin: 0;
    padding: 0;
    height: 100%;
    overflow: hidden;
    background: var(--bg-solid);
    color: var(--fg);
    user-select: none;
  }

  .editor-window {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--bg-solid);
    border: 1px solid var(--border);
    box-sizing: border-box;
    overflow: hidden;
  }

  /* 顶栏 */
  .title-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 42px;
    padding: 0 10px 0 14px;
    background: var(--panel);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }

  .file-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    max-width: 38%;
  }

  .file-icon {
    font-size: 15px;
  }

  .file-name {
    font-weight: 600;
    font-size: 13.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--fg);
  }

  .dirty-badge {
    font-size: 11px;
    font-weight: 600;
    color: var(--warn, #f59e0b);
    padding: 1px 5px;
    border-radius: 4px;
    background: rgba(245, 158, 11, 0.12);
  }

  .kb-badge {
    font-size: 11px;
    font-weight: 500;
    color: var(--accent-blue, #3b82f6);
    padding: 1px 6px;
    border-radius: 4px;
    background: rgba(59, 130, 246, 0.12);
    border: 1px solid rgba(59, 130, 246, 0.25);
  }

  .center-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11.5px;
    color: var(--fg-dim);
  }

  .stat-pill {
    padding: 1px 6px;
    background: var(--bg-solid);
    border-radius: 4px;
    border: 1px solid var(--border);
  }

  .save-time {
    margin-left: 4px;
    font-size: 11px;
    opacity: 0.8;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .view-tabs {
    display: flex;
    align-items: center;
    background: var(--bg-solid);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 2px;
    gap: 2px;
  }

  .v-tab {
    border: none;
    background: transparent;
    color: var(--fg-dim);
    font-size: 11.5px;
    padding: 3px 8px;
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .v-tab:hover {
    color: var(--fg);
  }

  .v-tab.active {
    background: var(--panel-elevated, var(--hover));
    color: var(--accent-blue, #3b82f6);
    font-weight: 600;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.1);
  }

  .act-btn {
    border: 1px solid var(--border);
    background: var(--panel-elevated, var(--panel));
    color: var(--fg);
    font-size: 11.5px;
    font-weight: 500;
    padding: 4px 10px;
    border-radius: 5px;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .act-btn:hover:not(:disabled) {
    background: var(--hover);
    border-color: var(--fg-dim);
  }

  .act-btn.primary {
    background: var(--accent-blue, #3b82f6);
    color: #fff;
    border-color: transparent;
  }

  .act-btn.primary:hover:not(:disabled) {
    opacity: 0.9;
  }

  .act-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .sep {
    width: 1px;
    height: 16px;
    background: var(--border);
    margin: 0 2px;
  }

  .win-btn {
    width: 28px;
    height: 28px;
    border: none;
    background: transparent;
    color: var(--fg-dim);
    border-radius: 5px;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 12.5px;
  }

  .win-btn:hover {
    background: var(--hover);
    color: var(--fg);
  }

  .win-btn.on {
    color: var(--accent-blue);
    background: rgba(59, 130, 246, 0.12);
  }

  .win-btn.close:hover {
    background: #ef4444;
    color: #fff;
  }

  /* 主编辑区 */
  .main-body {
    flex: 1;
    min-height: 0;
    display: flex;
    position: relative;
    overflow: hidden;
  }

  .editor-pane {
    flex: 1;
    min-width: 0;
    height: 100%;
    overflow: hidden;
    position: relative;
  }

  .split-divider {
    width: 1px;
    background: var(--border);
    flex-shrink: 0;
  }

  .preview-pane {
    flex: 1;
    min-width: 0;
    height: 100%;
    overflow-y: auto;
    padding: 24px 32px;
    box-sizing: border-box;
    background: var(--bg-solid);
  }

  .editor-pane.hidden,
  .preview-pane.hidden {
    display: none !important;
  }

  .md-render {
    max-width: 820px;
    margin: 0 auto;
    line-height: 1.7;
    font-size: 14.5px;
    color: var(--fg);
  }

  .selectable {
    user-select: text;
  }

  /* 底栏 */
  .footer-bar {
    display: flex;
    align-items: center;
    height: 24px;
    padding: 0 12px;
    background: var(--panel);
    border-top: 1px solid var(--border);
    font-size: 11px;
    color: var(--fg-dim);
    flex-shrink: 0;
  }

  .path-btn {
    border: none;
    background: transparent;
    padding: 0;
    font-size: 11px;
    color: var(--fg-dim);
    max-width: 60%;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    cursor: pointer;
    text-align: left;
  }

  .path-btn:hover {
    color: var(--accent-blue);
    text-decoration: underline;
  }

  .spacer {
    flex: 1;
  }

  .sync-tag {
    color: var(--ok, #10b981);
    margin-right: 12px;
    font-weight: 500;
  }

  .enc-tag {
    font-family: monospace;
    opacity: 0.75;
  }

  /* 状态遮罩与通知 */
  .state-overlay {
    position: absolute;
    inset: 0;
    background: rgba(0, 0, 0, 0.45);
    backdrop-filter: blur(4px);
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    z-index: 50;
  }

  .pulse-ring {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    border: 3px solid rgba(59, 130, 246, 0.25);
    border-top-color: var(--accent-blue, #3b82f6);
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .err-box {
    color: var(--danger, #ef4444);
    text-align: center;
    padding: 20px;
  }

  .err-title {
    font-size: 15px;
    font-weight: 600;
  }

  .err-desc {
    font-size: 12.5px;
    color: var(--fg-dim);
    max-width: 480px;
    margin: 8px 0 14px;
  }

  .retry-btn {
    border: 1px solid var(--border);
    background: var(--panel);
    color: var(--fg);
    padding: 4px 14px;
    border-radius: 6px;
    cursor: pointer;
  }

  .toast {
    position: fixed;
    bottom: 34px;
    left: 50%;
    transform: translateX(-50%);
    background: var(--fg);
    color: var(--bg-solid);
    padding: 6px 16px;
    border-radius: 8px;
    font-size: 12.5px;
    font-weight: 500;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.25);
    z-index: 100;
    pointer-events: none;
    animation: fadeIn 0.2s ease;
  }

  @keyframes fadeIn {
    from {
      opacity: 0;
      transform: translate(-50%, 6px);
    }
    to {
      opacity: 1;
      transform: translate(-50%, 0);
    }
  }
</style>
