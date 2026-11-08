<script lang="ts">
  // 二维码识别视图：按内容类型（网址 / WiFi / 名片 / 文本）给出不同操作；网址不自动打开
  import { onMount } from 'svelte';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { ipc } from '$lib/core/ipc';
  import { execute } from '$lib/core/actions';
  import type { Payload, QrResult } from '$lib/core/types';

  let { payload }: { payload: Payload } = $props();
  let codes = $state<QrResult[]>([]);
  let loading = $state(true);
  let copiedIdx = $state(-1);

  const SHORT_HOSTS = ['t.cn', 'bit.ly', 'tinyurl.com', 'url.cn', 'dwz.cn', 'suo.im', 'goo.gl', 't.co', 'u.nu', 'is.gd', 'w.url.cn'];

  type Parsed =
    | { type: 'url'; url: string; host: string; short: boolean }
    | { type: 'wifi'; ssid: string; password: string; auth: string }
    | { type: 'vcard'; name: string; tel: string; email: string; org: string }
    | { type: 'text' };

  function parse(t: string): Parsed {
    if (/^https?:\/\//i.test(t)) {
      let host = '';
      try {
        host = new URL(t).host;
      } catch {
        /* ignore */
      }
      return { type: 'url', url: t, host, short: SHORT_HOSTS.includes(host.toLowerCase()) };
    }
    if (/^WIFI:/i.test(t)) {
      const get = (k: string) => (t.match(new RegExp(`${k}:((?:\\\\;|[^;])*)`, 'i'))?.[1] ?? '').replace(/\\;/g, ';');
      return { type: 'wifi', ssid: get('S'), password: get('P'), auth: get('T') || 'nopass' };
    }
    if (/^BEGIN:VCARD/i.test(t)) {
      const get = (k: string) => t.match(new RegExp(`^${k}[^:]*:(.*)$`, 'im'))?.[1]?.trim() ?? '';
      return { type: 'vcard', name: get('FN') || get('N'), tel: get('TEL'), email: get('EMAIL'), org: get('ORG') };
    }
    return { type: 'text' };
  }

  onMount(async () => {
    try {
      codes = payload.meta?.qr?.length ? payload.meta.qr : await ipc.qrDecode(payload.image!);
    } finally {
      loading = false;
    }
  });

  async function copy(t: string, i: number) {
    await ipc.copyText(t);
    copiedIdx = i;
    setTimeout(() => (copiedIdx = -1), 1200);
  }

  const asText = (t: string) => ({ ...payload, kind: 'text' as const, image: null, text: t, meta: {} });
</script>

<div class="qr">
  {#if payload.image}<img class="src" src={payload.image} alt="" />{/if}
  {#if loading}
    <div class="hint">▣ 识别中…</div>
  {:else if codes.length === 0}
    <div class="hint">没有在图片中找到二维码或条码。可以框选得更贴近二维码再试一次。</div>
  {:else}
    {#each codes as c, i (i)}
      {@const p = parse(c.text)}
      <div class="card">
        <div class="fmt">{c.format}</div>
        {#if p.type === 'url'}
          <div class="label">🔗 网址 {#if p.short}<span class="warn">短链接，打开前请确认来源</span>{/if}</div>
          <div class="val selectable">{p.url}</div>
          <div class="btns">
            <button onclick={() => copy(p.url, i)}>{copiedIdx === i ? '✔ 已复制' : '📋 复制'}</button>
            <button onclick={() => openUrl(p.url)}>🌍 在浏览器打开</button>
          </div>
        {:else if p.type === 'wifi'}
          <div class="label">📶 WiFi</div>
          <div class="val">网络：<b class="selectable">{p.ssid}</b></div>
          <div class="val">密码：<b class="selectable">{p.password || '（无）'}</b> · {p.auth}</div>
          <div class="btns"><button onclick={() => copy(p.password, i)}>{copiedIdx === i ? '✔ 已复制' : '📋 复制密码'}</button></div>
        {:else if p.type === 'vcard'}
          <div class="label">👤 名片</div>
          {#if p.name}<div class="val">姓名：{p.name}</div>{/if}
          {#if p.org}<div class="val">单位：{p.org}</div>{/if}
          {#if p.tel}<div class="val">电话：{p.tel}</div>{/if}
          {#if p.email}<div class="val">邮箱：{p.email}</div>{/if}
          <div class="btns"><button onclick={() => copy(c.text, i)}>{copiedIdx === i ? '✔ 已复制' : '📋 复制全部'}</button></div>
        {:else}
          <div class="label">📄 文本</div>
          <div class="val selectable pre">{c.text}</div>
          <div class="btns">
            <button onclick={() => copy(c.text, i)}>{copiedIdx === i ? '✔ 已复制' : '📋 复制'}</button>
            <button onclick={() => execute('translate', asText(c.text))}>🌐 翻译</button>
            <button onclick={() => execute('explain', asText(c.text))}>💡 解释</button>
          </div>
        {/if}
      </div>
    {/each}
  {/if}
</div>

<style>
  .qr {
    padding: 8px 12px;
    overflow: auto;
    height: 100%;
  }
  .src {
    max-height: 90px;
    max-width: 100%;
    border-radius: 6px;
    border: 1px solid var(--border);
  }
  .hint {
    color: var(--fg-dim);
    padding: 8px 0;
  }
  .card {
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 8px 10px;
    margin-top: 8px;
    position: relative;
  }
  .fmt {
    position: absolute;
    right: 8px;
    top: 6px;
    font-size: 11px;
    color: var(--fg-dim);
  }
  .label {
    font-weight: 600;
    margin-bottom: 4px;
  }
  .warn {
    color: var(--warn);
    font-weight: normal;
    font-size: 12px;
    margin-left: 6px;
  }
  .val {
    word-break: break-all;
    margin: 2px 0;
  }
  .pre {
    white-space: pre-wrap;
    max-height: 160px;
    overflow: auto;
  }
  .btns {
    display: flex;
    gap: 6px;
    margin-top: 6px;
    flex-wrap: wrap;
  }
</style>
