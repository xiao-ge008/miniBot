import { getCurrentWindow, type Window } from '@tauri-apps/api/window';

type WindowInternals = Window & {
  onFocusChanged: (handler: (event: { payload: boolean }) => void) => Promise<() => void>;
};

const noop = async () => {};

// 开发预览或 WebView2 注入异常时，窗口 API 不应阻断整个 Svelte 页面渲染。
const fallbackWindow: WindowInternals = {
  setTitle: noop,
  setAlwaysOnTop: noop,
  startDragging: noop,
  minimize: noop,
  hide: noop,
  onFocusChanged: async () => () => {},
} as unknown as WindowInternals;

export function getSafeCurrentWindow(): WindowInternals {
  if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) {
    return fallbackWindow;
  }
  try {
    return getCurrentWindow() as WindowInternals;
  } catch {
    return fallbackWindow;
  }
}
