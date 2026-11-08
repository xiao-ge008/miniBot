<script lang="ts">
  // 以图搜图视图（参考 PicImageSearch + eSearch）：
  // 多引擎聚合（百度识图同款/相似图瀑布流 + AnimeTrace 动漫游戏角色 + Trace.moe 番剧搜番）
  // 支持瀑布流图片墙、单张图片选择文件夹保存、点击打开图片源地址、一键发送单张图片到 AI 聊天
  import { onMount } from 'svelte';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { ipc } from '$lib/core/ipc';
  import type {
    ImageSearchExternalUrl,
    ImageSearchItem,
    ImageSearchResponse,
    Payload,
  } from '$lib/core/types';

  let { payload }: { payload: Payload } = $props();

  type EngineId = 'baidu' | 'animetrace' | 'tracemoe';
  const ENGINES: { id: EngineId; label: string; desc: string }[] = [
    { id: 'baidu', label: '综合搜图', desc: '百度识图：全网同款原图与相似图片瀑布流（国内直连）' },
    { id: 'animetrace', label: '角色识别', desc: 'AnimeTrace：动漫 / 游戏角色与作品出处识别（国内直连）' },
    { id: 'tracemoe', label: '番剧搜番', desc: 'Trace.moe：动画截图定位番剧集数与精确时间点' },
  ];

  let activeEngine = $state<EngineId>('baidu');
  let loading = $state(true);
  let err = $state('');
  let toast = $state('');
  let busyIdx = $state<string | null>(null);
  let cache = $state<Partial<Record<EngineId, ImageSearchResponse>>>({});
  let sharedExternalUrls = $state<ImageSearchExternalUrl[]>([]);

  const curResp = $derived(cache[activeEngine] ?? null);
  const items = $derived<ImageSearchItem[]>(curResp?.items ?? []);
  const guess = $derived(curResp?.guess || cache.baidu?.guess || '');
  const externalUrls = $derived(
    curResp?.external_urls?.length ? curResp.external_urls : sharedExternalUrls,
  );

  function flash(msg: string) {
    toast = msg;
    setTimeout(() => {
      if (toast === msg) toast = '';
    }, 1800);
  }

  async function runSearch(engine: EngineId, force = false) {
    activeEngine = engine;
    err = '';
    if (!force && cache[engine]) {
      loading = false;
      return;
    }
    if (!payload.image) {
      err = '未获取到截图数据';
      loading = false;
      return;
    }
    loading = true;
    try {
      const resp = await ipc.imageSearch(payload.image, engine);
      cache = { ...cache, [engine]: resp };
      if (resp.external_urls?.length) {
        sharedExternalUrls = resp.external_urls;
      }
      if (engine === 'baidu' && (resp.guess || resp.items.length)) {
        ipc
          .historyAdd(
            {
              action: 'image_search',
              kind: 'image',
              input: '[截图搜图]',
              output: resp.guess ? `识别为：${resp.guess}（共 ${resp.items.length} 张相似图）` : `搜出 ${resp.items.length} 张相关图片`,
              source_app: payload.source_app ?? null,
            },
            payload.image,
          )
          .catch(() => {});
      }
    } catch (e) {
      err = String(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    runSearch('baidu');
  });

  async function saveSingleImage(item: ImageSearchItem, idx: number) {
    const key = `save-${idx}`;
    if (busyIdx) return;
    busyIdx = key;
    try {
      const dataUri = await ipc.fetchRemoteImage(item.image_url);
      const savedPath = await ipc.saveImage(dataUri);
      if (savedPath) {
        flash(`✓ 已保存图片`);
      }
    } catch (e) {
      flash(`保存失败：${e}`);
    } finally {
      busyIdx = null;
    }
  }

  async function sendItemToChat(item: ImageSearchItem, idx: number) {
    const key = `chat-${idx}`;
    if (busyIdx) return;
    busyIdx = key;
    try {
      const dataUri = await ipc.fetchRemoteImage(item.image_url);
      await ipc.sendToChat(dataUri, false);
      flash('✓ 已放入 AI 聊天输入框');
    } catch (e) {
      flash(`发送失败：${e}`);
    } finally {
      busyIdx = null;
    }
  }

  function openSource(url: string) {
    if (!url) return;
    ipc.openExternalUrl(url).catch(() => {
      openUrl(url).catch(() => {});
    });
  }
</script>

<div class="img-search">
  <!-- 顶部栏：原图预览 + AI 猜词 + 引擎切换 -->
  <div class="top-bar">
    {#if payload.image}
      <img class="src-thumb" src={payload.image} alt="原截图" />
    {/if}
    <div class="meta">
      {#if loading}
        <span class="status">🔍 正在检索匹配图片…</span>
      {:else if guess}
        <button
          class="guess-btn"
          title="点击复制识别结果"
          onclick={() => {
            ipc.copyText(guess);
            flash(`已复制「${guess}」`);
          }}
        >
          <span class="guess-tag">识别</span>
          <span class="guess-txt">{guess}</span>
        </button>
      {:else if err}
        <span class="status err-txt">检索异常</span>
      {:else}
        <span class="status ok-txt">找到 {items.length} 个结果</span>
      {/if}
      <span class="sub">
        {#if !loading && !err}
          共 {items.length} 张结果图 · 点击图片看源网址
        {:else}
          支持保存单张图 / 打开源地址 / 发到 AI 聊天
        {/if}
      </span>
    </div>

    <div class="engine-tabs">
      {#each ENGINES as eng (eng.id)}
        <button
          class="eng-tab"
          class:on={activeEngine === eng.id}
          title={eng.desc}
          onclick={() => runSearch(eng.id)}
        >
          {eng.label}
        </button>
      {/each}
    </div>
  </div>

  <!-- 主体：图片墙瀑布流 -->
  <div class="wall-wrap">
    {#if loading}
      <div class="masonry">
        {#each [130, 175, 145, 160, 135, 165] as h, i (i)}
          <div class="pin-card sk-card">
            <div class="sk-img" style="height:{h}px"></div>
            <div class="sk-line"></div>
          </div>
        {/each}
      </div>
    {:else if err}
      <div class="empty-state">
        <div class="empty-title">❌ 搜图请求未成功：{err}</div>
        <div class="empty-sub">请检查网络连接，或直接点击下方按钮在浏览器中打开对应搜图引擎。</div>
        <button class="retry-btn" onclick={() => runSearch(activeEngine, true)}>重试当前引擎</button>
      </div>
    {:else if items.length === 0}
      <div class="empty-state">
        <div class="empty-title">未直接匹配到同款或相似图片卡片</div>
        <div class="empty-sub">可尝试切换上方「角色识别 / 番剧搜番」，或点击下方「百度识图 / 必应搜图 / Yandex」打开完整网页结果。</div>
        {#if curResp?.page_url}
          <button class="primary" onclick={() => openSource(curResp!.page_url)}>🌐 打开完整搜图结果页</button>
        {/if}
      </div>
    {:else}
      <div class="masonry">
        {#each items as item, idx (idx)}
          <div class="pin-card">
            <div
              class="img-box"
              role="button"
              tabindex="0"
              title={`点击打开图片来源网页：${item.source_url}`}
              onclick={() => openSource(item.source_url)}
              onkeydown={(e) => e.key === 'Enter' && openSource(item.source_url)}
            >
              <img
                src={item.image_url}
                alt={item.title}
                width={item.width || undefined}
                height={item.height || undefined}
                referrerpolicy="no-referrer"
                loading="lazy"
              />
              {#if item.badge}
                <span class="badge" class:same={item.badge === '同款' || item.badge === '首选角色'}>
                  {item.badge}
                </span>
              {/if}
            </div>

            <div class="card-info">
              <button
                class="card-title"
                title={`点击访问源网址：${item.source_url}`}
                onclick={() => openSource(item.source_url)}
              >
                {item.title}
              </button>
              {#if item.subtitle}
                <div class="card-sub" title={item.subtitle}>{item.subtitle}</div>
              {/if}

              <div class="card-btns">
                <button
                  class="act-btn"
                  title={`在浏览器打开图片来源地址：${item.source_url}`}
                  onclick={() => openSource(item.source_url)}
                >
                  <svg viewBox="0 0 16 16" width="11.5" height="11.5" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M7 3H3.8A1.8 1.8 0 0 0 2 4.8v7.4A1.8 1.8 0 0 0 3.8 14h7.4a1.8 1.8 0 0 0 1.8-1.8V9" />
                    <path d="M10 2h4v4M14 2L7.5 8.5" />
                  </svg>
                  <span>源址</span>
                </button>
                <button
                  class="act-btn"
                  disabled={busyIdx === `save-${idx}`}
                  title="选择文件夹保存此张图片"
                  onclick={() => saveSingleImage(item, idx)}
                >
                  <svg viewBox="0 0 16 16" width="11.5" height="11.5" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M8 2.5v7.5M5 7.5L8 10.5l3-3" />
                    <path d="M2.5 11.5v1.2a1.3 1.3 0 0 0 1.3 1.3h8.4a1.3 1.3 0 0 0 1.3-1.3v-1.2" />
                  </svg>
                  <span>{busyIdx === `save-${idx}` ? '…' : '保存'}</span>
                </button>
                <button
                  class="act-btn chat-btn"
                  disabled={busyIdx === `chat-${idx}`}
                  title="将此张图片发送到 AI 聊天窗口"
                  onclick={() => sendItemToChat(item, idx)}
                >
                  <svg viewBox="0 0 16 16" width="11.5" height="11.5" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M2.5 3.8A1.8 1.8 0 0 1 4.3 2h7.4a1.8 1.8 0 0 1 1.8 1.8v5.4a1.8 1.8 0 0 1-1.8 1.8H6.5l-3.2 2.5V11H4.3a1.8 1.8 0 0 1-1.8-1.8V3.8z" />
                  </svg>
                  <span>{busyIdx === `chat-${idx}` ? '…' : 'AI聊天'}</span>
                </button>
              </div>
            </div>
          </div>
        {/each}
      </div>
    {/if}
  </div>

  <!-- 底部：全网搜图引擎一键直达栏 + 原图发聊天 -->
  <div class="footer-bar">
    <span class="ft-lbl">网页直达</span>
    {#if externalUrls.length}
      {#each externalUrls as ext, i (ext.id || `${ext.name}-${i}`)}
        <button class="ext-btn" onclick={() => openSource(ext.url)} title={`在浏览器中打开 ${ext.name} 完整搜图结果`}>
          {ext.name}
        </button>
      {/each}
    {:else if curResp?.page_url}
      <button class="ext-btn" onclick={() => openSource(curResp!.page_url)}>打开完整结果页</button>
    {/if}
    <span class="spacer"></span>
    {#if payload.image}
      <button
        class="ext-btn primary-soft"
        title="将原截图发送到 AI 聊天窗口提问"
        onclick={() => ipc.sendToChat(payload.image!, false)}
      >
        原图发到 AI 聊天
      </button>
    {/if}
  </div>

  {#if toast}
    <div class="toast">{toast}</div>
  {/if}
</div>

<style>
  .img-search {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: 8px 10px;
    gap: 8px;
    overflow: hidden;
  }
  .top-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 8px;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 8px;
    flex-shrink: 0;
  }
  .src-thumb {
    width: 34px;
    height: 34px;
    object-fit: cover;
    border-radius: 5px;
    border: 1px solid var(--border);
    flex-shrink: 0;
  }
  .meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }
  .status {
    font-size: 12px;
    font-weight: 600;
  }
  .ok-txt {
    color: var(--ok);
  }
  .err-txt {
    color: var(--danger);
  }
  .guess-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    border: none;
    background: transparent;
    padding: 0;
    text-align: left;
    max-width: 100%;
  }
  .guess-btn:hover .guess-txt {
    color: var(--accent-blue);
  }
  .guess-tag {
    font-size: 10.5px;
    font-weight: 600;
    padding: 1px 5px;
    border-radius: 4px;
    background: rgba(37, 99, 235, 0.14);
    color: var(--accent-blue);
    flex-shrink: 0;
  }
  .guess-txt {
    font-size: 12.5px;
    font-weight: 600;
    color: var(--fg);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub {
    color: var(--fg-dim);
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .engine-tabs {
    display: flex;
    align-items: center;
    gap: 3px;
    background: var(--bg-solid);
    border: 1px solid var(--border);
    border-radius: 7px;
    padding: 2px;
    flex-shrink: 0;
  }
  .eng-tab {
    border: none;
    background: transparent;
    color: var(--fg-dim);
    font-size: 11.5px;
    font-weight: 500;
    padding: 3px 8px;
    border-radius: 5px;
  }
  .eng-tab:hover {
    color: var(--fg);
    background: var(--hover);
  }
  .eng-tab.on {
    background: var(--accent);
    color: var(--accent-fg);
  }

  /* 瀑布流图片墙 */
  .wall-wrap {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding-right: 2px;
  }
  .masonry {
    columns: 155px;
    column-gap: 8px;
  }
  .pin-card {
    break-inside: avoid;
    margin-bottom: 8px;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 8px;
    overflow: hidden;
    transition: border-color 0.15s ease, transform 0.15s ease;
  }
  .pin-card:hover {
    border-color: rgba(59, 130, 246, 0.45);
  }
  .img-box {
    position: relative;
    width: 100%;
    background: var(--bg-solid);
    cursor: pointer;
    overflow: hidden;
  }
  .img-box img {
    display: block;
    width: 100%;
    height: auto;
    min-height: 64px;
    max-height: 240px;
    object-fit: cover;
    transition: transform 0.2s ease;
  }
  .img-box:hover img {
    transform: scale(1.03);
  }
  .badge {
    position: absolute;
    top: 5px;
    left: 5px;
    padding: 1.5px 6px;
    border-radius: 4px;
    font-size: 10.5px;
    font-weight: 600;
    background: rgba(18, 18, 21, 0.76);
    color: #fafafa;
    backdrop-filter: blur(6px);
  }
  .badge.same {
    background: rgba(16, 185, 129, 0.9);
    color: #fff;
  }
  .card-info {
    padding: 6px 7px;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .card-title {
    border: none;
    background: transparent;
    padding: 0;
    text-align: left;
    font-size: 12px;
    font-weight: 500;
    color: var(--fg);
    display: -webkit-box;
    -webkit-line-clamp: 1;
    line-clamp: 1;
    -webkit-box-orient: vertical;
    overflow: hidden;
    word-break: break-all;
  }
  .card-title:hover {
    color: var(--accent-blue);
    text-decoration: underline;
  }
  .card-sub {
    font-size: 10.5px;
    color: var(--fg-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .card-btns {
    display: flex;
    align-items: center;
    gap: 3px;
    margin-top: 3px;
  }
  .act-btn {
    flex: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 2.5px;
    padding: 3px 4px;
    font-size: 11px;
    border-radius: 5px;
    background: var(--bg-solid);
    border: 1px solid var(--border);
    color: var(--fg-dim);
  }
  .act-btn:hover:not(:disabled) {
    color: var(--fg);
    border-color: var(--fg-dim);
    background: var(--hover);
  }
  .act-btn.chat-btn:hover:not(:disabled) {
    color: var(--accent-blue);
    border-color: var(--accent-blue);
  }

  /* 骨架屏与空状态 */
  .sk-card {
    padding: 6px;
  }
  .sk-img {
    width: 100%;
    border-radius: 5px;
    background: var(--bg-solid);
    margin-bottom: 6px;
  }
  .sk-line {
    height: 10px;
    width: 70%;
    border-radius: 4px;
    background: var(--bg-solid);
  }
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    text-align: center;
    gap: 8px;
    padding: 32px 16px;
    color: var(--fg-dim);
  }
  .empty-title {
    font-size: 13px;
    font-weight: 500;
    color: var(--fg);
  }
  .empty-sub {
    font-size: 12px;
    max-width: 360px;
    line-height: 1.5;
  }
  .retry-btn {
    margin-top: 4px;
  }

  /* 底部全网引擎跳转栏 */
  .footer-bar {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-wrap: wrap;
    padding-top: 6px;
    border-top: 1px solid var(--border);
    flex-shrink: 0;
  }
  .ft-lbl {
    font-size: 11px;
    color: var(--fg-dim);
    margin-right: 2px;
  }
  .ext-btn {
    font-size: 11.5px;
    padding: 3px 8px;
    border-radius: 5px;
  }
  .ext-btn.primary-soft {
    color: var(--accent-blue);
    border-color: rgba(37, 99, 235, 0.35);
    font-weight: 500;
  }
  .spacer {
    flex: 1;
  }
  .toast {
    position: fixed;
    bottom: 42px;
    left: 50%;
    transform: translateX(-50%);
    background: var(--fg);
    color: var(--bg-solid);
    padding: 4px 12px;
    border-radius: 6px;
    font-size: 12px;
    box-shadow: var(--shadow);
    z-index: 20;
    pointer-events: none;
  }
</style>
