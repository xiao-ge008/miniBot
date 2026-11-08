// 颜色工具：格式转换 + K-means 主色提取（纯本地计算）

export type RGB = [number, number, number];

export const toHex = ([r, g, b]: RGB) =>
  '#' + [r, g, b].map((v) => Math.round(v).toString(16).padStart(2, '0')).join('').toUpperCase();

export const toRgbStr = ([r, g, b]: RGB) => `rgb(${Math.round(r)}, ${Math.round(g)}, ${Math.round(b)})`;

export function toHsl([r, g, b]: RGB): [number, number, number] {
  r /= 255;
  g /= 255;
  b /= 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  let h = 0;
  let s = 0;
  const l = (max + min) / 2;
  if (max !== min) {
    const d = max - min;
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
    if (max === r) h = (g - b) / d + (g < b ? 6 : 0);
    else if (max === g) h = (b - r) / d + 2;
    else h = (r - g) / d + 4;
    h *= 60;
  }
  return [Math.round(h), Math.round(s * 100), Math.round(l * 100)];
}

export const toHslStr = (c: RGB) => {
  const [h, s, l] = toHsl(c);
  return `hsl(${h}, ${s}%, ${l}%)`;
};

/** 解析 #RGB / #RRGGBB / #RRGGBBAA / rgb() / rgba() / hsl() */
export function parseColor(s: string): RGB | null {
  s = s.trim().toLowerCase();
  let m = s.match(/^#([0-9a-f]{3})$/);
  if (m) return m[1].split('').map((c) => parseInt(c + c, 16)) as RGB;
  m = s.match(/^#([0-9a-f]{6})([0-9a-f]{2})?$/);
  if (m) return [0, 2, 4].map((i) => parseInt(m![1].slice(i, i + 2), 16)) as RGB;
  m = s.match(/^rgba?\(\s*(\d+)[\s,]+(\d+)[\s,]+(\d+)/);
  if (m) return [+m[1], +m[2], +m[3]];
  m = s.match(/^hsl\(\s*(\d+)[\s,]+(\d+)%[\s,]+(\d+)%/);
  if (m) {
    const h = +m[1] / 360;
    const sat = +m[2] / 100;
    const l = +m[3] / 100;
    const q = l < 0.5 ? l * (1 + sat) : l + sat - l * sat;
    const p = 2 * l - q;
    const f = (t: number) => {
      if (t < 0) t += 1;
      if (t > 1) t -= 1;
      if (t < 1 / 6) return p + (q - p) * 6 * t;
      if (t < 1 / 2) return q;
      if (t < 2 / 3) return p + (q - p) * (2 / 3 - t) * 6;
      return p;
    };
    return [f(h + 1 / 3) * 255, f(h) * 255, f(h - 1 / 3) * 255].map(Math.round) as RGB;
  }
  return null;
}

/** K-means 提取主色，返回按占比排序的颜色与比例 */
export function extractPalette(data: Uint8ClampedArray, k = 6): { color: RGB; ratio: number }[] {
  // 下采样到最多约 2 万个像素
  const total = data.length / 4;
  const step = Math.max(1, Math.floor(total / 20000));
  const pts: RGB[] = [];
  for (let i = 0; i < total; i += step) {
    const o = i * 4;
    if (data[o + 3] < 128) continue;
    pts.push([data[o], data[o + 1], data[o + 2]]);
  }
  if (pts.length === 0) return [];
  k = Math.min(k, pts.length);
  // k-means++ 初始化
  const centers: RGB[] = [pts[Math.floor(Math.random() * pts.length)]];
  const d2 = (a: RGB, b: RGB) => (a[0] - b[0]) ** 2 + (a[1] - b[1]) ** 2 + (a[2] - b[2]) ** 2;
  while (centers.length < k) {
    const dist = pts.map((p) => Math.min(...centers.map((c) => d2(p, c))));
    const sum = dist.reduce((a, b) => a + b, 0);
    let r = Math.random() * sum;
    let idx = 0;
    while (r > dist[idx] && idx < pts.length - 1) r -= dist[idx++];
    centers.push([...pts[idx]] as RGB);
  }
  const assign = new Array(pts.length).fill(0);
  for (let iter = 0; iter < 12; iter++) {
    for (let i = 0; i < pts.length; i++) {
      let best = 0;
      let bd = Infinity;
      for (let c = 0; c < k; c++) {
        const d = d2(pts[i], centers[c]);
        if (d < bd) {
          bd = d;
          best = c;
        }
      }
      assign[i] = best;
    }
    const sums = centers.map(() => [0, 0, 0, 0]);
    for (let i = 0; i < pts.length; i++) {
      const s = sums[assign[i]];
      s[0] += pts[i][0];
      s[1] += pts[i][1];
      s[2] += pts[i][2];
      s[3]++;
    }
    sums.forEach((s, c) => {
      if (s[3]) centers[c] = [s[0] / s[3], s[1] / s[3], s[2] / s[3]];
    });
  }
  const counts = centers.map(() => 0);
  assign.forEach((a) => counts[a]++);
  return centers
    .map((c, i) => ({ color: c.map(Math.round) as RGB, ratio: counts[i] / pts.length }))
    .filter((x) => x.ratio > 0.005)
    .sort((a, b) => b.ratio - a.ratio);
}

/** 从 data URI 读取像素 */
export async function imageData(src: string): Promise<ImageData> {
  const img = new Image();
  img.src = src;
  await img.decode();
  const c = document.createElement('canvas');
  c.width = img.naturalWidth;
  c.height = img.naturalHeight;
  const ctx = c.getContext('2d')!;
  ctx.drawImage(img, 0, 0);
  return ctx.getImageData(0, 0, c.width, c.height);
}
