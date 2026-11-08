<script lang="ts">
  // 二维码生成：文本 / 网址 / WiFi / 名片，实时预览，可加 Logo，导出 PNG/SVG 或钉到桌面
  import { onMount } from 'svelte';
  import { ipc, onPayload } from '$lib/core/ipc';

  type Kind = 'text' | 'wifi' | 'vcard';
  let kind = $state<Kind>('text');
  let text = $state('');
  let wifi = $state({ ssid: '', password: '', auth: 'WPA' });
  let card = $state({ name: '', tel: '', email: '', org: '' });
  let ec = $state('M');
  let dark = $state('#000000');
  let light = $state('#ffffff');
  let logo = $state<string | null>(null);
  let svg = $state('');
  let err = $state('');
  let toast = $state('');
  let fileInput: HTMLInputElement;

  const esc = (s: string) => s.replace(/([\\;,:"])/g, '\\$1');
  const content = $derived(
    kind === 'wifi'
      ? `WIFI:T:${wifi.auth};S:${esc(wifi.ssid)};P:${esc(wifi.password)};;`
      : kind === 'vcard'
        ? `BEGIN:VCARD\nVERSION:3.0\nFN:${card.name}\nORG:${card.org}\nTEL:${card.tel}\nEMAIL:${card.email}\nEND:VCARD`
        : text,
  );

  $effect(() => {
    const c = content;
    const level = logo ? 'H' : ec; // 有 Logo 时自动提升容错，保证能扫出
    if (!c.trim()) {
      svg = '';
      return;
    }
    ipc
      .qrEncode(c, level, dark, light)
      .then((s) => {
        svg = s;
        err = '';
      })
      .catch((e) => (err = String(e)));
  });

  onMount(() => {
    const un = onPayload((p) => {
      text = p.text ?? '';
      kind = 'text';
    });
    return () => un.then((f) => f());
  });

  /** 渲染为 PNG（含 Logo） */
  async function toPng(size = 768): Promise<string> {
    const c = document.createElement('canvas');
    c.width = c.height = size;
    const ctx = c.getContext('2d')!;
    const im = new Image();
    im.src = 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(svg);
    await im.decode();
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(im, 0, 0, size, size);
    if (logo) {
      const lg = new Image();
      lg.src = logo;
      await lg.decode();
      const s = size * 0.22;
      const x = (size - s) / 2;
      ctx.fillStyle = light;
      ctx.beginPath();
      ctx.roundRect(x - 8, x - 8, s + 16, s + 16, 12);
      ctx.fill();
      ctx.drawImage(lg, x, x, s, s);
    }
    return c.toDataURL('image/png');
  }

  function flash(t: string) {
    toast = t;
    setTimeout(() => (toast = ''), 1500);
  }

  async function copy() {
    await ipc.copyImage(await toPng());
    flash('已复制图片');
  }
  async function savePng() {
    flash(`已保存 ${await ipc.saveImage(await toPng())}`);
  }
  async function copySvg() {
    await ipc.copyText(svg);
    flash('SVG 代码已复制');
  }
  async function pin() {
    await ipc.pinImage(await toPng(512), 200, 200, 280, 280);
  }

  function pickLogo(e: Event) {
    const f = (e.target as HTMLInputElement).files?.[0];
    if (!f) return;
    const r = new FileReader();
    r.onload = () => (logo = r.result as string);
    r.readAsDataURL(f);
  }
</script>

<div class="qr">
  <div class="qr-head" data-tauri-drag-region>
    <div class="tabs">
      {#each [['text', '文本 / 网址'], ['wifi', 'WiFi'], ['vcard', '名片']] as [k, l] (k)}
        <button class:active={kind === k} onclick={() => (kind = k as Kind)}>{l}</button>
      {/each}
    </div>
    <span class="qr-sp" data-tauri-drag-region></span>
    <button class="ghost qr-close" onclick={() => ipc.hideSelf()} title="关闭">✕</button>
  </div>

  {#if kind === 'text'}
    <textarea bind:value={text} rows="3" placeholder="输入文字或网址"></textarea>
  {:else if kind === 'wifi'}
    <div class="form">
      <input bind:value={wifi.ssid} placeholder="网络名称 (SSID)" />
      <input bind:value={wifi.password} placeholder="密码" />
      <select bind:value={wifi.auth}><option>WPA</option><option>WEP</option><option value="nopass">无密码</option></select>
    </div>
  {:else}
    <div class="form">
      <input bind:value={card.name} placeholder="姓名" />
      <input bind:value={card.tel} placeholder="电话" />
      <input bind:value={card.email} placeholder="邮箱" />
      <input bind:value={card.org} placeholder="单位" />
    </div>
  {/if}

  <div class="preview">
    {#if svg}
      <div class="code" style="background:{light}">
        {@html svg}
        {#if logo}<img class="logo" src={logo} alt="" style="background:{light}" />{/if}
      </div>
    {:else}
      <div class="empty">{err || '输入内容后实时生成'}</div>
    {/if}
  </div>

  <div class="opts">
    <label>容错
      <select bind:value={ec} disabled={!!logo}>
        <option value="L">L 7%</option><option value="M">M 15%</option><option value="Q">Q 25%</option><option value="H">H 30%</option>
      </select>
    </label>
    <label>前景 <input type="color" bind:value={dark} /></label>
    <label>背景 <input type="color" bind:value={light} /></label>
    <button class="ghost" onclick={() => (logo ? (logo = null) : fileInput.click())}>{logo ? '移除 Logo' : '＋ Logo'}</button>
    <input type="file" accept="image/*" bind:this={fileInput} onchange={pickLogo} hidden />
  </div>

  <div class="btns">
    <button class="primary" onclick={copy} disabled={!svg}>📋 复制图片</button>
    <button onclick={savePng} disabled={!svg}>💾 PNG</button>
    <button onclick={copySvg} disabled={!svg}>SVG</button>
    <button onclick={pin} disabled={!svg}>📌 钉到桌面</button>
  </div>
  {#if toast}<div class="toast">{toast}</div>{/if}
</div>

<style>
  :global(body) {
    background: var(--bg-solid) !important;
  }
  .qr {
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    height: 100vh;
  }
  .qr-head {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .qr-sp {
    flex: 1;
    align-self: stretch;
  }
  .qr-close {
    width: 28px;
    height: 28px;
    padding: 0;
  }
  .tabs {
    display: flex;
    gap: 4px;
  }
  .tabs button.active {
    background: var(--accent);
    color: var(--accent-fg);
    border-color: transparent;
  }
  textarea {
    width: 100%;
    resize: none;
  }
  .form {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px;
  }
  .preview {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 0;
  }
  .code {
    position: relative;
    width: 260px;
    height: 260px;
    border-radius: 8px;
    border: 1px solid var(--border);
    overflow: hidden;
  }
  .code :global(svg) {
    width: 100%;
    height: 100%;
    display: block;
  }
  .logo {
    position: absolute;
    width: 22%;
    height: 22%;
    left: 39%;
    top: 39%;
    object-fit: contain;
    border-radius: 6px;
    padding: 3px;
  }
  .empty {
    color: var(--fg-dim);
  }
  .opts {
    display: flex;
    gap: 10px;
    align-items: center;
    flex-wrap: wrap;
    font-size: 12.5px;
  }
  .opts input[type='color'] {
    width: 30px;
    height: 24px;
    padding: 0;
    vertical-align: middle;
  }
  .btns {
    display: flex;
    gap: 6px;
  }
  .toast {
    position: fixed;
    bottom: 60px;
    left: 50%;
    transform: translateX(-50%);
    background: var(--fg);
    color: var(--bg-solid);
    padding: 4px 12px;
    border-radius: 6px;
    font-size: 12px;
    max-width: 90%;
    word-break: break-all;
  }
</style>
