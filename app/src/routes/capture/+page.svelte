<script lang="ts">
  // 截图遮罩：框选 / 单击选窗口 / 取色放大镜 / 标注（矩形 椭圆 箭头 画笔 文字 序号 马赛克）/ 动作工具条
  import { onMount, tick } from 'svelte';
  import { ipc, onPayload, uid } from '$lib/core/ipc';
  import { execute, loadActions } from '$lib/core/actions';
  import { toHex, toRgbStr, type RGB } from '$lib/core/color';
  import type { Payload } from '$lib/core/types';

  type Rect = { x: number; y: number; w: number; h: number };
  type Tool = 'rect' | 'ellipse' | 'arrow' | 'pen' | 'text' | 'number' | 'mosaic' | 'blur';
  type Shape =
    | { t: 'rect' | 'ellipse' | 'arrow'; x1: number; y1: number; x2: number; y2: number; color: string; w: number }
    | { t: 'pen'; pts: [number, number][]; color: string; w: number }
    | { t: 'text'; x: number; y: number; text: string; color: string; size: number }
    | { t: 'number'; x: number; y: number; n: number; color: string; size: number }
    | { t: 'mosaic'; cells: Map<string, string>; w: number }
    | { t: 'blur'; pts: [number, number][]; w: number };
  type Phase = 'idle' | 'selecting' | 'selected' | 'moving' | 'resizing' | 'drawing';

  const COLORS = ['#FF3B30', '#FF9500', '#FFCC00', '#34C759', '#007AFF', '#FFFFFF', '#000000'];
  const GRADIENTS: [string, string][] = [
    ['#667eea', '#764ba2'],
    ['#00c6ff', '#0072ff'],
    ['#f857a6', '#ff5858'],
    ['#11998e', '#38ef7d'],
    ['#232526', '#414345'],
  ];
  const TOOLS: { id: Tool; icon: string; label: string }[] = [
    { id: 'rect', icon: '▭', label: '矩形' },
    { id: 'ellipse', icon: '◯', label: '椭圆' },
    { id: 'arrow', icon: '➜', label: '箭头' },
    { id: 'pen', icon: '✎', label: '画笔' },
    { id: 'text', icon: 'T', label: '文字' },
    { id: 'number', icon: '①', label: '序号' },
    { id: 'mosaic', icon: '▦', label: '马赛克' },
    { id: 'blur', icon: '◌', label: '模糊' },
  ];

  let base: HTMLCanvasElement;
  let overlay: HTMLCanvasElement;
  let img: HTMLImageElement | null = null;
  let pixels: ImageData | null = null;
  let payload: Payload | null = null;

  let mode = $state<'capture' | 'color' | 'chat'>('capture');
  let W = $state(0);
  let H = $state(0);
  let scale = 1;
  let originX = 0;
  let originY = 0;
  let windows: Rect[] = [];

  let phase = $state<Phase>('idle');
  let sel = $state<Rect | null>(null);
  let hoverWin: Rect | null = null;
  let tool = $state<Tool | null>(null);
  let color = $state(COLORS[0]);
  let sizeIdx = $state(1);
  let shapes = $state.raw<Shape[]>([]);
  let redoStack: Shape[] = [];
  let drawing: Shape | null = null;
  let mouse = { x: 0, y: 0 };
  let down = { x: 0, y: 0 };
  let start: Rect | null = null;
  let handle = -1;
  let qrCount = $state(0);
  let toast = $state('');
  let textEdit = $state<{ x: number; y: number; value: string } | null>(null);
  let textInput = $state<HTMLTextAreaElement>();
  let beautify = $state(false);
  let gradIdx = $state(0);
  let sizeEdit = $state(false);
  let sizeWStr = $state('');
  let sizeHStr = $state('');
  let barW = $state(0);
  let barH = $state(0);
  let raf = 0;

  /** 参考 eSearch #clip_wh：支持四则运算表达式（如 800+20, 1920/2） */
  function evalMath(expr: string, fallback: number): number {
    const clean = expr.replace(/[^0-9+\-*/.()]/g, '');
    if (!clean) return fallback;
    try {
      const v = Function(`"use strict"; return (${clean});`)();
      return typeof v === 'number' && Number.isFinite(v) && v >= 4 ? Math.round(v) : fallback;
    } catch {
      return fallback;
    }
  }

  function applySizeEdit() {
    if (!sel) return;
    const nw = Math.min(W - sel.x, Math.max(8, evalMath(sizeWStr, Math.round(sel.w))));
    const nh = Math.min(H - sel.y, Math.max(8, evalMath(sizeHStr, Math.round(sel.h))));
    sel = { ...sel, w: nw, h: nh };
    sizeEdit = false;
    detectQr();
    render();
  }

  // ---------- 坐标 ----------
  const k = () => window.innerWidth / Math.max(W, 1); // CSS px / 物理 px
  const toPhys = (e: MouseEvent) => ({ x: Math.round(e.clientX / k()), y: Math.round(e.clientY / k()) });
  const norm = (x1: number, y1: number, x2: number, y2: number): Rect => ({
    x: Math.min(x1, x2),
    y: Math.min(y1, y2),
    w: Math.abs(x2 - x1),
    h: Math.abs(y2 - y1),
  });
  const inRect = (r: Rect, x: number, y: number) => x >= r.x && y >= r.y && x <= r.x + r.w && y <= r.y + r.h;
  const lineW = () => [2, 4, 8][sizeIdx] * scale;
  const fontSize = () => [16, 22, 32][sizeIdx] * scale;

  function pixel(x: number, y: number): RGB {
    if (!pixels) return [0, 0, 0];
    x = Math.max(0, Math.min(W - 1, x));
    y = Math.max(0, Math.min(H - 1, y));
    const o = (y * W + x) * 4;
    return [pixels.data[o], pixels.data[o + 1], pixels.data[o + 2]];
  }

  function handles(r: Rect): [number, number][] {
    const { x, y, w, h } = r;
    return [
      [x, y], [x + w / 2, y], [x + w, y],
      [x + w, y + h / 2], [x + w, y + h], [x + w / 2, y + h],
      [x, y + h], [x, y + h / 2],
    ];
  }

  function hitHandle(x: number, y: number) {
    if (!sel) return -1;
    const r = 7 * scale;
    return handles(sel).findIndex(([hx, hy]) => Math.abs(hx - x) <= r && Math.abs(hy - y) <= r);
  }

  // ---------- 绘制 ----------
  function drawShape(ctx: CanvasRenderingContext2D, s: Shape) {
    ctx.save();
    if (s.t === 'mosaic') {
      const c = 10 * scale;
      for (const [key, col] of s.cells) {
        const [cx, cy] = key.split(',').map(Number);
        ctx.fillStyle = col;
        ctx.fillRect(cx * c, cy * c, c + 0.5, c + 0.5);
      }
    } else if (s.t === 'blur' && img) {
      ctx.beginPath();
      for (const [x, y] of s.pts) {
        ctx.moveTo(x + s.w, y);
        ctx.arc(x, y, s.w, 0, Math.PI * 2);
      }
      ctx.clip();
      ctx.filter = `blur(${Math.max(6, Math.round(7 * scale))}px)`;
      ctx.drawImage(img, 0, 0);
      ctx.filter = 'none';
    } else if (s.t === 'text') {
      ctx.font = `bold ${s.size}px "Microsoft YaHei UI", sans-serif`;
      ctx.textBaseline = 'top';
      ctx.lineWidth = Math.max(2, s.size / 8);
      ctx.strokeStyle = s.color === '#FFFFFF' ? '#000' : '#fff';
      s.text.split('\n').forEach((line, i) => {
        ctx.strokeText(line, s.x, s.y + i * s.size * 1.25);
        ctx.fillStyle = s.color;
        ctx.fillText(line, s.x, s.y + i * s.size * 1.25);
      });
    } else if (s.t === 'number') {
      const r = s.size * 0.62;
      ctx.beginPath();
      ctx.arc(s.x, s.y, r, 0, Math.PI * 2);
      ctx.fillStyle = s.color;
      ctx.fill();
      ctx.lineWidth = 2 * scale;
      ctx.strokeStyle = '#fff';
      ctx.stroke();
      ctx.fillStyle = s.color === '#FFFFFF' || s.color === '#FFCC00' ? '#000' : '#fff';
      ctx.font = `bold ${r * 1.2}px "Segoe UI", sans-serif`;
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText(String(s.n), s.x, s.y + r * 0.05);
    } else if (s.t !== 'blur') {
      ctx.strokeStyle = s.color;
      ctx.fillStyle = s.color;
      ctx.lineWidth = s.w;
      ctx.lineCap = 'round';
      ctx.lineJoin = 'round';
      if (s.t === 'pen') {
        ctx.beginPath();
        s.pts.forEach(([x, y], i) => (i ? ctx.lineTo(x, y) : ctx.moveTo(x, y)));
        ctx.stroke();
      } else if (s.t === 'rect') {
        const r = norm(s.x1, s.y1, s.x2, s.y2);
        ctx.strokeRect(r.x, r.y, r.w, r.h);
      } else if (s.t === 'ellipse') {
        const r = norm(s.x1, s.y1, s.x2, s.y2);
        ctx.beginPath();
        ctx.ellipse(r.x + r.w / 2, r.y + r.h / 2, r.w / 2, r.h / 2, 0, 0, Math.PI * 2);
        ctx.stroke();
      } else if (s.t === 'arrow') {
        const ang = Math.atan2(s.y2 - s.y1, s.x2 - s.x1);
        const head = Math.max(12 * scale, s.w * 4);
        const bx = s.x2 - head * 0.8 * Math.cos(ang);
        const by = s.y2 - head * 0.8 * Math.sin(ang);
        ctx.beginPath();
        ctx.moveTo(s.x1, s.y1);
        ctx.lineTo(bx, by);
        ctx.stroke();
        ctx.beginPath();
        ctx.moveTo(s.x2, s.y2);
        ctx.lineTo(s.x2 - head * Math.cos(ang - 0.42), s.y2 - head * Math.sin(ang - 0.42));
        ctx.lineTo(s.x2 - head * Math.cos(ang + 0.42), s.y2 - head * Math.sin(ang + 0.42));
        ctx.closePath();
        ctx.fill();
      }
    }
    ctx.restore();
  }

  function drawMagnifier(ctx: CanvasRenderingContext2D) {
    const zoom = 8;
    const n = 15; // 采样 15×15 像素
    const box = n * zoom * (scale >= 1.5 ? 1 : 1);
    const pad = 20 * scale;
    let x = mouse.x + pad;
    let y = mouse.y + pad;
    const infoH = 56 * scale;
    if (x + box > W) x = mouse.x - pad - box;
    if (y + box + infoH > H) y = mouse.y - pad - box - infoH;
    ctx.save();
    ctx.imageSmoothingEnabled = false;
    if (img) ctx.drawImage(img, mouse.x - (n >> 1), mouse.y - (n >> 1), n, n, x, y, box, box);
    // 网格
    ctx.strokeStyle = 'rgba(255,255,255,0.15)';
    ctx.lineWidth = 1;
    for (let i = 1; i < n; i++) {
      ctx.beginPath();
      ctx.moveTo(x + i * zoom, y);
      ctx.lineTo(x + i * zoom, y + box);
      ctx.moveTo(x, y + i * zoom);
      ctx.lineTo(x + box, y + i * zoom);
      ctx.stroke();
    }
    // 中心像素框
    ctx.strokeStyle = '#3b82f6';
    ctx.lineWidth = 2;
    ctx.strokeRect(x + (n >> 1) * zoom, y + (n >> 1) * zoom, zoom, zoom);
    ctx.strokeStyle = '#fff';
    ctx.lineWidth = 1.5;
    ctx.strokeRect(x, y, box, box);
    // 信息区
    const c = pixel(mouse.x, mouse.y);
    ctx.fillStyle = 'rgba(18,18,21,0.94)';
    ctx.fillRect(x, y + box, box, infoH);
    ctx.fillStyle = toHex(c);
    ctx.fillRect(x + 8 * scale, y + box + 8 * scale, 16 * scale, 16 * scale);
    ctx.fillStyle = '#fafafa';
    ctx.font = `${12 * scale}px Consolas, monospace`;
    ctx.textBaseline = 'top';
    ctx.fillText(toHex(c), x + 30 * scale, y + box + 9 * scale);
    ctx.fillStyle = '#a1a1aa';
    ctx.fillText(`${c[0]},${c[1]},${c[2]}`, x + 8 * scale, y + box + 30 * scale);
    const tip = sel && phase !== 'idle' ? `${Math.round(sel.w)}×${Math.round(sel.h)}` : `${mouse.x},${mouse.y}`;
    ctx.fillText(tip, x + 8 * scale, y + box + 44 * scale - 2);
    ctx.restore();
  }

  function drawNow() {
    if (!overlay) return;
    const ctx = overlay.getContext('2d')!;
    ctx.clearRect(0, 0, W, H);
    ctx.fillStyle = mode === 'color' ? 'rgba(0,0,0,0.08)' : 'rgba(0,0,0,0.45)';
    ctx.fillRect(0, 0, W, H);

    // 参考 eSearch #clip_line：未选定时的全屏十字准星参考线
    if (phase === 'idle' || phase === 'selecting') {
      ctx.save();
      ctx.strokeStyle = 'rgba(255,255,255,0.28)';
      ctx.lineWidth = 1;
      ctx.setLineDash([4 * scale, 4 * scale]);
      ctx.beginPath();
      ctx.moveTo(mouse.x, 0);
      ctx.lineTo(mouse.x, H);
      ctx.moveTo(0, mouse.y);
      ctx.lineTo(W, mouse.y);
      ctx.stroke();
      ctx.restore();
    }

    const region = sel ?? (phase === 'idle' && mode !== 'color' ? hoverWin : null);
    if (region) {
      ctx.clearRect(region.x, region.y, region.w, region.h);
      if (sel) {
        ctx.save();
        ctx.beginPath();
        ctx.rect(sel.x, sel.y, sel.w, sel.h);
        ctx.clip();
        shapes.forEach((s) => drawShape(ctx, s));
        if (drawing) drawShape(ctx, drawing);
        ctx.restore();
      }
      ctx.strokeStyle = beautify && sel ? GRADIENTS[gradIdx][0] : '#3b82f6';
      ctx.lineWidth = Math.max(1.5, (beautify && sel ? 3 : 1.5) * scale);
      ctx.strokeRect(region.x, region.y, region.w, region.h);
      if (sel && phase !== 'selecting' && !tool) {
        ctx.fillStyle = '#3b82f6';
        const hs = 4 * scale;
        handles(sel).forEach(([hx, hy]) => ctx.fillRect(hx - hs, hy - hs, hs * 2, hs * 2));
      }
    }
    const showMag = mode === 'color' || phase === 'idle' || phase === 'selecting' || phase === 'resizing';
    if (showMag) drawMagnifier(ctx);
  }

  function render() {
    cancelAnimationFrame(raf);
    raf = requestAnimationFrame(drawNow);
  }

  // ---------- 马赛克 ----------
  function addMosaic(s: Extract<Shape, { t: 'mosaic' }>, x: number, y: number) {
    const c = 10 * scale;
    const r = Math.max(1, Math.round(s.w / c));
    const cx = Math.floor(x / c);
    const cy = Math.floor(y / c);
    for (let dx = -r; dx <= r; dx++)
      for (let dy = -r; dy <= r; dy++) {
        if (dx * dx + dy * dy > r * r) continue;
        const key = `${cx + dx},${cy + dy}`;
        if (!s.cells.has(key)) s.cells.set(key, toHex(pixel(Math.round((cx + dx + 0.5) * c), Math.round((cy + dy + 0.5) * c))));
      }
  }

  function focusTextInput() {
    tick().then(() => {
      textInput?.focus();
      requestAnimationFrame(() => textInput?.focus());
    });
  }

  // ---------- 鼠标 ----------
  function onDown(e: MouseEvent) {
    if (e.button === 2) return;
    const p = toPhys(e);
    if (textEdit) {
      const hadText = textEdit.value.trim().length > 0;
      commitText();
      if (hadText) return;
    }
    down = p;
    mouse = p;
    if (mode === 'color') {
      copyColor(false, true);
      return;
    }
    if (sel && tool && inRect(sel, p.x, p.y)) {
      if (tool === 'text') {
        e.preventDefault();
        e.stopPropagation();
      }
      startDraw(p.x, p.y);
      return;
    }
    if (sel) {
      const h = hitHandle(p.x, p.y);
      if (h >= 0) {
        phase = 'resizing';
        handle = h;
        start = { ...sel };
        return;
      }
      if (inRect(sel, p.x, p.y) && !tool) {
        phase = 'moving';
        start = { ...sel };
        return;
      }
      if (tool) return; // 有工具时点选区外不重选
    }
    // 新建选区
    shapes = [];
    redoStack = [];
    qrCount = 0;
    sel = null;
    phase = 'selecting';
  }

  function startDraw(x: number, y: number) {
    const c = color;
    if (tool === 'text') {
      const fs = fontSize();
      const tx = sel ? Math.max(sel.x + 2, Math.min(sel.x + sel.w - 24, x)) : x;
      const ty = sel ? Math.max(sel.y + 2, Math.min(sel.y + sel.h - fs, Math.round(y - fs * 0.45))) : y;
      textEdit = { x: tx, y: ty, value: '' };
      focusTextInput();
      return;
    }
    if (tool === 'number') {
      const n = shapes.filter((s) => s.t === 'number').length + 1;
      pushShape({ t: 'number', x, y, n, color: c, size: fontSize() });
      return;
    }
    phase = 'drawing';
    if (tool === 'pen') drawing = { t: 'pen', pts: [[x, y]], color: c, w: lineW() };
    else if (tool === 'blur') drawing = { t: 'blur', pts: [[x, y]], w: lineW() * 3.5 };
    else if (tool === 'mosaic') {
      drawing = { t: 'mosaic', cells: new Map(), w: lineW() * 3 };
      addMosaic(drawing, x, y);
    } else drawing = { t: tool!, x1: x, y1: y, x2: x, y2: y, color: c, w: lineW() } as Shape;
  }

  function onMove(e: MouseEvent) {
    const p = toPhys(e);
    mouse = p;
    if (phase === 'idle') {
      hoverWin = windows.find((r) => inRect(r, p.x, p.y)) ?? null;
    } else if (phase === 'selecting') {
      sel = norm(down.x, down.y, p.x, p.y);
    } else if (phase === 'moving' && start && sel) {
      sel = {
        ...sel,
        x: Math.max(0, Math.min(W - start.w, start.x + p.x - down.x)),
        y: Math.max(0, Math.min(H - start.h, start.y + p.y - down.y)),
      };
    } else if (phase === 'resizing' && start) {
      let { x, y, w, h } = start;
      let x2 = x + w;
      let y2 = y + h;
      if ([0, 6, 7].includes(handle)) x = p.x;
      if ([2, 3, 4].includes(handle)) x2 = p.x;
      if ([0, 1, 2].includes(handle)) y = p.y;
      if ([4, 5, 6].includes(handle)) y2 = p.y;
      sel = norm(x, y, x2, y2);
    } else if (phase === 'drawing' && drawing) {
      if (drawing.t === 'pen' || drawing.t === 'blur') drawing.pts.push([p.x, p.y]);
      else if (drawing.t === 'mosaic') addMosaic(drawing, p.x, p.y);
      else if ('x2' in drawing) {
        drawing.x2 = p.x;
        drawing.y2 = p.y;
      }
    }
    updateCursor(p);
    render();
  }

  function onUp(e: MouseEvent) {
    const p = toPhys(e);
    if (phase === 'selecting') {
      const moved = Math.abs(p.x - down.x) > 4 || Math.abs(p.y - down.y) > 4;
      if (!moved) sel = hoverWin ? { ...hoverWin } : { x: 0, y: 0, w: W, h: H };
      phase = sel && sel.w > 2 && sel.h > 2 ? 'selected' : 'idle';
      if (phase === 'idle') sel = null;
      else detectQr();
    } else if (phase === 'moving' || phase === 'resizing') {
      phase = 'selected';
      detectQr();
    } else if (phase === 'drawing' && drawing) {
      const d = drawing;
      drawing = null;
      phase = 'selected';
      const tiny = 'x2' in d && Math.abs(d.x2 - d.x1) < 3 && Math.abs(d.y2 - d.y1) < 3;
      if (!tiny) pushShape(d);
    }
    render();
  }

  function updateCursor(p: { x: number; y: number }) {
    let c = 'crosshair';
    if (sel && phase === 'selected') {
      const h = hitHandle(p.x, p.y);
      if (h >= 0 && !tool) c = ['nwse-resize', 'ns-resize', 'nesw-resize', 'ew-resize', 'nwse-resize', 'ns-resize', 'nesw-resize', 'ew-resize'][h];
      else if (inRect(sel, p.x, p.y)) c = tool ? (tool === 'text' ? 'text' : 'crosshair') : 'move';
      else c = 'default';
    }
    overlay.style.cursor = c;
  }

  function pushShape(s: Shape) {
    shapes = [...shapes, s];
    redoStack = [];
    render();
  }

  function undo() {
    if (textEdit) commitText();
    if (!shapes.length) return;
    redoStack.push(shapes[shapes.length - 1]);
    shapes = shapes.slice(0, -1);
    render();
  }

  function redo() {
    if (textEdit) commitText();
    const s = redoStack.pop();
    if (s) {
      shapes = [...shapes, s];
      render();
    }
  }

  function commitText() {
    if (!textEdit) return;
    const val = textEdit.value.replace(/\r/g, '');
    if (val.trim()) {
      pushShape({ t: 'text', x: textEdit.x, y: textEdit.y, text: val, color, size: fontSize() });
    }
    textEdit = null;
  }

  // ---------- 导出与动作 ----------
  function exportImage(withShapes = true, withBeautify = beautify): string {
    if (withShapes && textEdit) commitText();
    const r = sel!;
    const rawW = Math.max(1, Math.round(r.w));
    const rawH = Math.max(1, Math.round(r.h));
    const inner = document.createElement('canvas');
    inner.width = rawW;
    inner.height = rawH;
    const ictx = inner.getContext('2d')!;
    ictx.drawImage(img!, r.x, r.y, r.w, r.h, 0, 0, rawW, rawH);
    if (withShapes) {
      ictx.save();
      ictx.translate(-r.x, -r.y);
      shapes.forEach((s) => drawShape(ictx, s));
      ictx.restore();
    }
    if (!withBeautify) return inner.toDataURL('image/png');

    // 参考 eSearch 截屏美化：外层渐变背景 + 圆角裁切 + 柔和投影
    const pad = Math.round(Math.max(24, Math.min(rawW, rawH) * 0.08));
    const rad = Math.round(12 * scale);
    const out = document.createElement('canvas');
    out.width = rawW + pad * 2;
    out.height = rawH + pad * 2;
    const octx = out.getContext('2d')!;
    const [c1, c2] = GRADIENTS[gradIdx];
    const g = octx.createLinearGradient(0, 0, out.width, out.height);
    g.addColorStop(0, c1);
    g.addColorStop(1, c2);
    octx.fillStyle = g;
    octx.fillRect(0, 0, out.width, out.height);

    octx.save();
    octx.shadowColor = 'rgba(0,0,0,0.38)';
    octx.shadowBlur = Math.round(22 * scale);
    octx.shadowOffsetY = Math.round(8 * scale);
    octx.beginPath();
    octx.roundRect(pad, pad, rawW, rawH, rad);
    octx.fillStyle = '#000';
    octx.fill();
    octx.restore();

    octx.save();
    octx.beginPath();
    octx.roundRect(pad, pad, rawW, rawH, rad);
    octx.clip();
    octx.drawImage(inner, pad, pad);
    octx.restore();
    return out.toDataURL('image/png');
  }

  let qrTimer: ReturnType<typeof setTimeout> | undefined;
  function detectQr() {
    clearTimeout(qrTimer);
    qrTimer = setTimeout(async () => {
      if (!sel) return;
      try {
        qrCount = (await ipc.qrDecode(exportImage(false, false))).length;
      } catch {
        qrCount = 0;
      }
    }, 200);
  }

  function imagePayload(data: string): Payload {
    return {
      id: uid('cap'),
      kind: 'image',
      origin: 'capture',
      image: data,
      text: null,
      source_app: payload?.source_app ?? null,
      source_hwnd: payload?.source_hwnd ?? 0,
      meta: {},
    };
  }

  async function act(id: string) {
    if (!sel) return;
    if (textEdit) commitText();
    let data = '';
    try {
      const useBeauty =
        beautify &&
        (id === 'copy' ||
          id === 'save' ||
          id === 'pin' ||
          id === 'send_chat' ||
          id === 'attach_chat' ||
          id === 'localsend');
      data = exportImage(id !== 'qr', useBeauty);
    } catch {
      close();
      return;
    }
    switch (id) {
      case 'localsend': {
        const p = imagePayload(data);
        phase = 'idle';
        sel = null;
        shapes = [];
        tool = null;
        await ipc.openLocalSend(p);
        break;
      }
      case 'send_chat':
        phase = 'idle';
        sel = null;
        shapes = [];
        tool = null;
        await ipc.sendToChat(data, true);
        break;
      case 'attach_chat':
        phase = 'idle';
        sel = null;
        shapes = [];
        tool = null;
        await ipc.sendToChat(data, false);
        break;
      case 'copy':
        close();
        await ipc.copyImage(data);
        break;
      case 'save':
        close();
        await ipc.saveImage(data);
        break;
      case 'pin': {
        const px = originX + Math.round(sel.x);
        const py = originY + Math.round(sel.y);
        const pw = sel.w / scale;
        const ph = sel.h / scale;
        close();
        await ipc.pinImage(data, px, py, pw, ph);
        break;
      }
      default:
        phase = 'idle';
        sel = null;
        shapes = [];
        tool = null;
        textEdit = null;
        sizeEdit = false;
        beautify = false;
        await execute(id, imagePayload(data));
    }
  }

  async function copyColor(rgb: boolean, closeAfter = false) {
    const c = pixel(mouse.x, mouse.y);
    const s = rgb ? toRgbStr(c) : toHex(c);
    await ipc.copyText(s);
    if (closeAfter || mode === 'color') {
      close();
    } else {
      toast = `已复制 ${s}`;
      setTimeout(() => (toast = ''), 1200);
    }
  }

  function close() {
    const returnToChat = mode === 'chat';
    phase = 'idle';
    sel = null;
    shapes = [];
    tool = null;
    textEdit = null;
    sizeEdit = false;
    beautify = false;
    ipc.captureClose();
    if (returnToChat) {
      ipc.trigger('launcher');
    }
  }

  function onKey(e: KeyboardEvent) {
    if (textEdit) {
      if (e.key === 'Escape') textEdit = null;
      return;
    }
    if (sizeEdit) {
      if (e.key === 'Escape') sizeEdit = false;
      return;
    }
    const key = e.key.toLowerCase();
    if (e.key === 'Escape') close();
    else if (e.ctrlKey && key === 'z' && !e.shiftKey) undo();
    else if (e.ctrlKey && (key === 'y' || (key === 'z' && e.shiftKey))) redo();
    else if (e.ctrlKey && key === 'c' && sel) act('copy');
    else if (e.key === 'Enter' && sel) act(mode === 'chat' ? 'attach_chat' : 'copy');
    else if (key === 'c' && !e.ctrlKey) copyColor(e.shiftKey);
    else if (key === 'f' && mode !== 'color') {
      sel = { x: 0, y: 0, w: W, h: H };
      phase = 'selected';
      detectQr();
      render();
    } else if (sel && ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(e.key)) {
      // 参考 eSearch：方向键微调选区位置（Shift 10px，Ctrl 调整宽高）
      e.preventDefault();
      const step = e.shiftKey ? 10 : 1;
      const dx = e.key === 'ArrowLeft' ? -step : e.key === 'ArrowRight' ? step : 0;
      const dy = e.key === 'ArrowUp' ? -step : e.key === 'ArrowDown' ? step : 0;
      if (e.ctrlKey) {
        sel = {
          ...sel,
          w: Math.max(8, Math.min(W - sel.x, sel.w + dx)),
          h: Math.max(8, Math.min(H - sel.y, sel.h + dy)),
        };
      } else {
        sel = {
          ...sel,
          x: Math.max(0, Math.min(W - sel.w, sel.x + dx)),
          y: Math.max(0, Math.min(H - sel.h, sel.y + dy)),
        };
      }
      render();
    }
  }

  function onContext(e: MouseEvent) {
    e.preventDefault();
    if (textEdit) {
      commitText();
      return;
    }
    if (tool) tool = null;
    else if (sel) {
      sel = null;
      shapes = [];
      phase = 'idle';
    } else close();
    render();
  }

  let prevBlobUrl = '';

  // ---------- 加载一帧 ----------
  async function load(p: Payload) {
    await loadActions();
    payload = p;
    const f = p.meta!.frame;
    mode = p.meta!.mode === 'color' ? 'color' : p.meta!.mode === 'chat' ? 'chat' : 'capture';
    W = f.width;
    H = f.height;
    scale = f.scale || 1;
    originX = f.x;
    originY = f.y;
    windows = (p.meta!.windows ?? []).map(([x, y, w, h]: number[]) => ({ x, y, w, h }));
    phase = 'idle';
    sel = null;
    shapes = [];
    redoStack = [];
    tool = null;
    beautify = false;
    sizeEdit = false;
    qrCount = 0;
    hoverWin = null;
    if (prevBlobUrl) {
      URL.revokeObjectURL(prevBlobUrl);
      prevBlobUrl = '';
    }
    const rawUrl = `http://mbcap.localhost/frame.png?t=${p.meta!.t}`;
    let srcUrl = rawUrl;
    try {
      const res = await fetch(rawUrl);
      const blob = await res.blob();
      prevBlobUrl = URL.createObjectURL(blob);
      srcUrl = prevBlobUrl;
    } catch {
      /* 回退至 crossOrigin 协议直读 */
    }
    const im = new Image();
    im.crossOrigin = 'anonymous';
    im.src = srcUrl;
    try {
      await im.decode();
    } catch {
      /* 即使解码异常也继续显示窗口，避免死锁 */
    }
    img = im;
    await tick();
    if (base && overlay) {
      base.width = overlay.width = W;
      base.height = overlay.height = H;
      const bctx = base.getContext('2d')!;
      bctx.drawImage(im, 0, 0);
      try {
        pixels = bctx.getImageData(0, 0, W, H);
      } catch {
        pixels = null;
      }
      // 同步绘制首帧并立即通知后端显示窗口（严禁放在 requestAnimationFrame 内，因为隐藏窗口下 WebView2 会挂起 rAF）
      drawNow();
    }
    await ipc.captureReady();
  }

  onMount(() => {
    const un = onPayload(load);
    return () => un.then((f) => f());
  });

  // 选区左上角尺寸胶囊（CSS px，与选区左边缘精准对齐）
  const whPos = $derived.by(() => {
    if (!sel) return null;
    const kx = window.innerWidth / Math.max(W, 1);
    const ky = window.innerHeight / Math.max(H, 1);
    const rx = Math.round(sel.x * kx);
    const ry = Math.round(sel.y * ky);
    const left = Math.max(6, Math.min(window.innerWidth - 110, rx));
    const top = ry > 32 ? ry - 28 : ry + 6;
    return { left, top };
  });

  // 工具条位置（CSS px，基于真实 DOM 宽高与截图框精准对齐）
  const bar = $derived.by(() => {
    if (!sel || phase === 'selecting' || phase === 'moving' || phase === 'resizing') return null;
    const kx = window.innerWidth / Math.max(W, 1);
    const ky = window.innerHeight / Math.max(H, 1);
    const rx = Math.round(sel.x * kx);
    const ry = Math.round(sel.y * ky);
    const rw = Math.round(sel.w * kx);
    const rh = Math.round(sel.h * ky);

    const bw = barW || 560;
    const bh = barH || (tool || beautify ? 76 : 36);

    // 默认与截图框左边缘对齐；若右侧空间不足则与截图框右边缘对齐
    let left = rx;
    if (left + bw > window.innerWidth - 8) {
      left = rx + rw - bw;
    }
    left = Math.max(8, Math.min(window.innerWidth - bw - 8, left));

    let top = ry + rh + 6;
    if (top + bh > window.innerHeight - 8) {
      top = ry - bh - 6;
    }
    if (top < 8) {
      top = Math.max(8, ry + rh - bh - 8);
    }
    return { left, top };
  });
</script>

<svelte:window onkeydown={onKey} />

<div class="root">
  <canvas class="layer" bind:this={base}></canvas>
  <canvas
    class="layer"
    bind:this={overlay}
    onmousedown={onDown}
    onmousemove={onMove}
    onmouseup={onUp}
    ondblclick={() => sel && !tool && act(mode === 'chat' ? 'attach_chat' : 'copy')}
    oncontextmenu={onContext}
  ></canvas>

  {#if sel && whPos}
    <div class="wh-pill" style="left:{whPos.left}px; top:{whPos.top}px">
      {#if sizeEdit}
        <input
          class="wh-in"
          bind:value={sizeWStr}
          onkeydown={(e) => {
            if (e.key === 'Enter') applySizeEdit();
            e.stopPropagation();
          }}
        />
        <span>×</span>
        <input
          class="wh-in"
          bind:value={sizeHStr}
          onkeydown={(e) => {
            if (e.key === 'Enter') applySizeEdit();
            e.stopPropagation();
          }}
          onblur={applySizeEdit}
        />
      {:else}
        <button
          class="wh-btn"
          title="点击输入精确尺寸（支持四则运算如 800+20, 1920/2）· 方向键微调"
          onclick={() => {
            sizeWStr = String(Math.round(sel!.w));
            sizeHStr = String(Math.round(sel!.h));
            sizeEdit = true;
          }}
        >
          {Math.round(sel.w)} × {Math.round(sel.h)}
        </button>
      {/if}
    </div>
  {/if}

  {#if textEdit}
    {@const fsCss = fontSize() * k()}
    {@const strokeCol = color === '#FFFFFF' ? '#000' : '#fff'}
    <div
      class="text-edit-wrap"
      role="presentation"
      style="left:{textEdit.x * k()}px; top:{textEdit.y * k()}px; font-size:{fsCss}px; --t-color:{color}; --t-stroke:{strokeCol};"
      onmousedown={(e) => e.stopPropagation()}
    >
      <span class="text-mirror" aria-hidden="true">{(textEdit.value || '输入文字') + '\u200b'}</span>
      <textarea
        class="text-edit"
        bind:this={textInput}
        bind:value={textEdit.value}
        placeholder="输入文字"
        rows="1"
        spellcheck="false"
        onkeydown={(e) => {
          if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            commitText();
          } else if (e.key === 'Escape') {
            e.preventDefault();
            textEdit = null;
          }
          e.stopPropagation();
        }}
      ></textarea>
    </div>
  {/if}

  {#if bar}
    <div
      class="bar"
      bind:offsetWidth={barW}
      bind:offsetHeight={barH}
      style="left:{bar.left}px; top:{bar.top}px"
    >
      <div class="row">
        {#each TOOLS as t (t.id)}
          <button
            class="tb"
            class:on={tool === t.id}
            title={t.label}
            onclick={() => {
              if (textEdit) commitText();
              tool = tool === t.id ? null : t.id;
            }}>{t.icon}</button
          >
        {/each}
        <button class="tb label-tb" class:on={beautify} title="截屏美化（渐变背景+圆角阴影）" onclick={() => (beautify = !beautify)}>美化</button>
        <span class="sep"></span>
        <button class="tb" title="撤销 Ctrl+Z" onclick={undo} disabled={!shapes.length}>↶</button>
        <button class="tb" title="重做 Ctrl+Y" onclick={redo}>↷</button>
        <span class="sep"></span>
        {#if mode === 'chat'}
          <button class="tb label-tb ok" title="将截图放入聊天输入框（补充问题后点发送，Enter / 双击）" onclick={() => act('attach_chat')}>📎 放入输入</button>
          <button class="tb label-tb" title="打开临时问答会话（先列出提问选项或输入问题后回答）" onclick={() => act('qa')}>提问</button>
          <button class="tb label-tb" title="通过 LocalSend 发送截图到局域网在线设备" onclick={() => act('localsend')}>📡 局域网发送</button>
          <button class="tb" title="取消并返回聊天 Esc" onclick={close}>✕</button>
        {:else}
          <button class="tb label-tb" title="智能识别图片内容" onclick={() => act('auto')}>识图</button>
          <button class="tb label-tb" title="提取文字 (OCR)" onclick={() => act('ocr')}>文字</button>
          <button class="tb label-tb" title="截图翻译" onclick={() => act('translate')}>翻译</button>
          <button class="tb label-tb" title="打开临时问答会话（先列出提问选项或输入问题后回答）" onclick={() => act('qa')}>提问</button>
          {#if qrCount > 0}
            <button class="tb label-tb hot" title={`已识别 ${qrCount} 个二维码`} onclick={() => act('qr')}>二维码</button>
          {/if}
          <span class="sep"></span>
          <button class="tb label-tb" title="通过 LocalSend 发送截图到局域网在线设备" onclick={() => act('localsend')}>📡 发送</button>
          <button class="tb label-tb" title="钉到桌面 (屏幕贴图)" onclick={() => act('pin')}>贴图</button>
          <button class="tb label-tb" title="保存图片" onclick={() => act('save')}>保存</button>
          <button class="tb label-tb ok" title="复制图片 Enter" onclick={() => act('copy')}>复制</button>
          <button class="tb" title="取消 Esc" onclick={close}>✕</button>
        {/if}
      </div>
      {#if tool || beautify}
        <div class="row props">
          {#if tool && tool !== 'mosaic' && tool !== 'blur'}
            {#each COLORS as c (c)}
              <button
                class="color"
                class:on={color === c}
                style="background:{c}"
                onmousedown={(e) => e.preventDefault()}
                onclick={() => {
                  color = c;
                  if (textEdit) focusTextInput();
                }}
                aria-label={c}
              ></button>
            {/each}
            <span class="sep"></span>
          {/if}
          {#if tool}
            {#each [0, 1, 2] as i (i)}
              <button
                class="size"
                class:on={sizeIdx === i}
                onmousedown={(e) => e.preventDefault()}
                onclick={() => {
                  sizeIdx = i;
                  if (textEdit) focusTextInput();
                }}
                aria-label="粗细 {i + 1}"
              >
                <span style="width:{4 + i * 4}px;height:{4 + i * 4}px"></span>
              </button>
            {/each}
          {/if}
          {#if beautify}
            {#if tool}<span class="sep"></span>{/if}
            <span class="prop-lbl">美化背景</span>
            {#each GRADIENTS as [g1, g2], gi (gi)}
              <button
                class="color"
                class:on={gradIdx === gi}
                style="background:linear-gradient(135deg,{g1},{g2})"
                onclick={() => {
                  gradIdx = gi;
                  render();
                }}
                aria-label="渐变 {gi + 1}"
              ></button>
            {/each}
          {/if}
        </div>
      {/if}
    </div>
  {/if}

  {#if mode === 'color'}
    <div class="hint">单击复制 HEX · Shift+C 复制 RGB · Esc 退出</div>
  {:else if !sel}
    <div class="hint">拖动框选 · 单击选窗口 · F 全屏 · 方向键微调 · C 复制颜色 · Esc 退出</div>
  {/if}
  {#if toast}<div class="toast">{toast}</div>{/if}
</div>

<style>
  .root {
    position: fixed;
    inset: 0;
    overflow: hidden;
    background: #000;
  }
  .layer {
    position: absolute;
    inset: 0;
    width: 100vw;
    height: 100vh;
  }
  .wh-pill {
    position: absolute;
    z-index: 6;
    display: inline-flex;
    align-items: center;
    height: 22px;
    gap: 3px;
    background: var(--bg-solid);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0 7px;
    color: var(--fg);
    font-family: Consolas, monospace;
    font-size: 12px;
    line-height: 1;
    box-shadow: var(--shadow);
  }
  .wh-btn {
    border: none;
    background: transparent;
    color: var(--fg);
    padding: 0 2px;
    height: 20px;
    display: inline-flex;
    align-items: center;
    font-family: Consolas, monospace;
    font-size: 12px;
    line-height: 1;
  }
  .wh-in {
    width: 52px;
    height: 18px;
    padding: 0 4px;
    font-size: 12px;
    font-family: Consolas, monospace;
    background: var(--panel);
    color: var(--fg);
    border: 1px solid var(--accent-blue);
    border-radius: 4px;
  }
  .bar {
    position: absolute;
    width: max-content;
    background: var(--bg-solid);
    border: 1px solid var(--border);
    color: var(--fg);
    border-radius: 8px;
    box-shadow: var(--shadow);
    padding: 4px 6px;
    z-index: 5;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .props {
    border-top: 1px solid var(--border);
    margin-top: 4px;
    padding-top: 4px;
    gap: 6px;
  }
  .prop-lbl {
    font-size: 11.5px;
    color: var(--fg-dim);
    margin-right: 2px;
    line-height: 1;
  }
  .tb {
    position: relative;
    background: transparent;
    border: none;
    color: var(--fg);
    width: 28px;
    height: 28px;
    padding: 0;
    font-size: 13.5px;
    line-height: 1;
    border-radius: 6px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }
  .tb.label-tb {
    width: auto;
    padding: 0 8px;
    font-size: 12.5px;
    font-weight: 500;
    letter-spacing: 0.01em;
  }
  .tb:hover {
    background: var(--hover);
  }
  .tb.on {
    background: var(--accent);
    color: var(--accent-fg);
  }
  .tb.ok {
    color: var(--ok);
    font-weight: 550;
  }
  .tb.hot {
    color: var(--accent-blue);
  }
  .sep {
    width: 1px;
    height: 14px;
    background: var(--border);
    margin: 0 4px;
    flex-shrink: 0;
  }
  .color {
    width: 18px;
    height: 18px;
    padding: 0;
    border-radius: 50%;
    border: 2px solid var(--border);
  }
  .color.on {
    border-color: var(--fg);
    transform: scale(1.12);
  }
  .size {
    width: 24px;
    height: 24px;
    padding: 0;
    background: transparent;
    border: none;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .size span {
    background: var(--fg-dim);
    border-radius: 50%;
    display: block;
  }
  .size.on span {
    background: var(--accent-blue);
  }
  .text-edit-wrap {
    position: absolute;
    z-index: 6;
    display: inline-grid;
    align-items: start;
    transform: translate(-5.5px, -3.5px);
  }
  .text-mirror,
  .text-edit {
    grid-area: 1 / 1 / 2 / 2;
    font-family: 'Microsoft YaHei UI', sans-serif;
    font-size: inherit;
    font-weight: bold;
    line-height: 1.25;
    padding: 2px 4px;
    margin: 0;
    box-sizing: border-box;
    white-space: pre;
    min-width: 84px;
    min-height: 1.4em;
  }
  .text-mirror {
    visibility: hidden;
    pointer-events: none;
    border: 1.5px solid transparent;
  }
  .text-edit {
    width: 100%;
    height: 100%;
    resize: none;
    overflow: hidden;
    background: rgba(15, 18, 25, 0.22);
    border: 1.5px dashed #3b82f6;
    border-radius: 4px;
    color: var(--t-color, #ff3b30);
    caret-color: var(--t-color, #ff3b30);
    outline: none;
    text-shadow:
      -1px -1px 0 var(--t-stroke, #fff),
      1px -1px 0 var(--t-stroke, #fff),
      -1px 1px 0 var(--t-stroke, #fff),
      1px 1px 0 var(--t-stroke, #fff);
  }
  .text-edit::placeholder {
    color: rgba(255, 255, 255, 0.55);
    text-shadow: 0 1px 2px rgba(0, 0, 0, 0.75);
    font-weight: 500;
  }
  .hint {
    position: absolute;
    top: 14px;
    left: 50%;
    transform: translateX(-50%);
    background: var(--bg-solid);
    border: 1px solid var(--border);
    color: var(--fg);
    padding: 5px 14px;
    border-radius: 8px;
    font-size: 12px;
    box-shadow: var(--shadow);
    pointer-events: none;
  }
  .toast {
    position: absolute;
    bottom: 30px;
    left: 50%;
    transform: translateX(-50%);
    background: var(--bg-solid);
    border: 1px solid var(--border);
    color: var(--fg);
    padding: 5px 12px;
    border-radius: 6px;
    box-shadow: var(--shadow);
    pointer-events: none;
  }
</style>
