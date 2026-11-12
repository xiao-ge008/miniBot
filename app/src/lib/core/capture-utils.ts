// 截图元数据校验与独立坐标换算，避免异常帧和混合缩放污染交互状态。
export interface CapturePoint { x: number; y: number }
export interface CaptureMeta {
  session_id: string;
  mode: 'capture' | 'color' | 'chat' | 'record';
  frame: { x: number; y: number; width: number; height: number; scale: number };
  windows: [number, number, number, number][];
  cursor: CapturePoint;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
function finite(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

export function parseCaptureMeta(value: unknown): CaptureMeta | null {
  if (!record(value) || typeof value.session_id !== 'string' || !value.session_id) return null;
  const f = value.frame;
  if (!record(f) || !finite(f.x) || !finite(f.y) || !finite(f.width) || !finite(f.height)
    || !Number.isSafeInteger(f.width) || !Number.isSafeInteger(f.height) || f.width <= 0 || f.height <= 0) return null;
  const windows: CaptureMeta['windows'] = [];
  if (Array.isArray(value.windows)) {
    for (const r of value.windows) {
      if (Array.isArray(r) && r.length === 4 && r.every(finite) && r[2] > 0 && r[3] > 0) {
        windows.push([r[0], r[1], r[2], r[3]]);
      }
    }
  }
  const c = record(value.cursor) ? value.cursor : {};
  const mode = value.mode === 'color' || value.mode === 'chat' || value.mode === 'record' ? value.mode : 'capture';
  return {
    session_id: value.session_id,
    mode,
    frame: { x: f.x, y: f.y, width: f.width, height: f.height, scale: finite(f.scale) && f.scale > 0 ? f.scale : 1 },
    windows,
    cursor: { x: finite(c.x) ? Math.max(0, Math.min(f.width, c.x)) : 0, y: finite(c.y) ? Math.max(0, Math.min(f.height, c.y)) : 0 },
  };
}

export function toCapturePoint(clientX: number, clientY: number, width: number, height: number, viewportWidth: number, viewportHeight: number): CapturePoint {
  return {
    x: Math.max(0, Math.min(width, Math.round(clientX * width / Math.max(1, viewportWidth)))),
    y: Math.max(0, Math.min(height, Math.round(clientY * height / Math.max(1, viewportHeight)))),
  };
}

export function matchesCaptureReset(currentId: string | null, value: unknown): boolean {
  return currentId !== null && record(value) && value.session_id === currentId;
}
