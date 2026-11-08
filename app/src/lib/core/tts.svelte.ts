// Edge TTS 全局语音朗读控制器（Svelte 5 Runes 响应式状态 + 首句极速合成 + 后台分句预取队列 + 跨窗口互斥 + 离线兜底）
import { ipc, onTtsEvent, uid } from './ipc';

export type TtsStatus = 'idle' | 'loading' | 'playing';

class TtsController {
  activeId = $state<string>('');
  status = $state<TtsStatus>('idle');
  error = $state<string>('');
}

export const tts = new TtsController();

let currentAudio: HTMLAudioElement | null = null;
let currentSessionToken = '';
let listenerInitialized = false;

function ensureCrossWindowListener() {
  if (listenerInitialized || typeof window === 'undefined') return;
  listenerInitialized = true;
  onTtsEvent((payload) => {
    if (payload.token && payload.token !== currentSessionToken) {
      stopLocalPlayback();
    }
  }).catch(() => {});
}

function stopLocalPlayback() {
  currentSessionToken = '';
  if (currentAudio) {
    try {
      currentAudio.pause();
      currentAudio.src = '';
    } catch {
      /* 忽略音频销毁异常 */
    }
    currentAudio = null;
  }
  if (typeof window !== 'undefined' && 'speechSynthesis' in window) {
    try {
      window.speechSynthesis.cancel();
    } catch {
      /* 忽略系统 TTS 取消异常 */
    }
  }
  tts.activeId = '';
  tts.status = 'idle';
}

/** 清洗 AI 输出或划词文本（剥离 <think> 思考过程、Markdown 语法符号、URL 噪音） */
export function cleanTextForTts(raw: string): string {
  if (!raw) return '';
  let s = raw;
  // 1. 移除 <think>...</think> 思考块
  s = s.replace(/<think>[\s\S]*?(<\/think>|$)/gi, '');
  // 2. 移除代码块围栏行（保留代码内注释或可读文本，去除 ```language 标记）
  s = s.replace(/```[a-zA-Z0-9_-]*\n?/g, '');
  // 3. 图片语法 ![alt](url) 移除，超链接 [text](url) 保留 text
  s = s.replace(/!\[[^\]]*\]\([^)]*\)/g, '');
  s = s.replace(/\[([^\]]+)\]\([^)]*\)/g, '$1');
  // 4. 移除裸露的超长 http/https 链接（除非整段文本本身就是短 URL）
  if (s.trim().includes(' ') || s.trim().includes('\n')) {
    s = s.replace(/https?:\/\/[^\s)>\]]+/g, '');
  }
  // 5. 移除 Markdown 标题、粗斜体、删除线、行内代码反引号、表格分割线、引用符号
  s = s.replace(/^\s*#{1,6}\s+/gm, '');
  s = s.replace(/^\s*>\s?/gm, '');
  s = s.replace(/^\s*\|?[\s\-:|]+\|[\s\-:|]*$/gm, '');
  s = s.replace(/\|/g, '，');
  s = s.replace(/(\*\*|__|\*|_|~~|`)/g, '');
  // 6. 规整空白与多余换行
  s = s.replace(/\n{2,}/g, '。\n').replace(/[ \t]{2,}/g, ' ').trim();
  return s;
}

/**
 * 将长文本切分为首句短块（~120字，极速开口）与后续标准块（~320字，后台预取无缝衔接）
 */
function splitIntoPlayableChunks(text: string): string[] {
  const clean = text.trim();
  if (!clean) return [];
  if (clean.length <= 160) return [clean];

  const sentences = clean
    .split(/(?<=[。！？；.!?;：\n])/)
    .map((s) => s.trim())
    .filter(Boolean);

  const chunks: string[] = [];
  let buf = '';

  for (const sent of sentences) {
    // 第一块控制在 110 字左右以获得最快的首包发声速度，后续块控制在 320 字左右
    const limit = chunks.length === 0 ? 110 : 320;
    if (buf.length + sent.length <= limit) {
      buf += (buf ? ' ' : '') + sent;
    } else {
      if (buf) {
        chunks.push(buf);
        buf = '';
      }
      if (sent.length > limit) {
        // 单句超长时按逗号或固定长度再切分
        let rest = sent;
        while (rest.length > limit) {
          let cut = rest.lastIndexOf('，', limit);
          if (cut < limit / 2) cut = rest.lastIndexOf(',', limit);
          if (cut < limit / 2) cut = limit;
          chunks.push(rest.slice(0, cut + 1).trim());
          rest = rest.slice(cut + 1).trim();
        }
        buf = rest;
      } else {
        buf = sent;
      }
    }
  }
  if (buf) chunks.push(buf);
  return chunks;
}

/** 播放单个 Data URI 音频并返回 Promise */
function playAudioUri(dataUri: string, token: string): Promise<void> {
  return new Promise<void>((resolve, reject) => {
    if (token !== currentSessionToken) {
      resolve();
      return;
    }
    const audio = new Audio(dataUri);
    currentAudio = audio;
    audio.onended = () => {
      if (currentAudio === audio) currentAudio = null;
      resolve();
    };
    audio.onerror = () => {
      if (currentAudio === audio) currentAudio = null;
      reject(new Error('音频解码播放失败'));
    };
    audio.play().catch((err) => {
      if (currentAudio === audio) currentAudio = null;
      reject(err);
    });
  });
}

/** 离线兜底：当网络不可用时回退至系统原生 Web Speech API */
function fallbackWebSpeech(text: string, token: string): Promise<void> {
  return new Promise<void>((resolve, reject) => {
    if (typeof window === 'undefined' || !('speechSynthesis' in window)) {
      reject(new Error('当前环境不支持语音合成'));
      return;
    }
    try {
      window.speechSynthesis.cancel();
      const utter = new SpeechSynthesisUtterance(text);
      const hasCjk = /[\u4e00-\u9fff]/.test(text);
      utter.lang = hasCjk ? 'zh-CN' : 'en-US';
      utter.onstart = () => {
        if (token === currentSessionToken) tts.status = 'playing';
      };
      utter.onend = () => resolve();
      utter.onerror = () => reject(new Error('本地语音朗读异常'));
      window.speechSynthesis.speak(utter);
    } catch (e) {
      reject(e);
    }
  });
}

/** 停止当前正在进行的语音朗读 */
export function stopSpeak(broadcast = true) {
  const token = uid('tts_stop');
  stopLocalPlayback();
  if (broadcast) {
    ipc.ttsBroadcast({ action: 'stop', token }).catch(() => {});
  }
}

/**
 * 切换朗读状态：若当前 id 正在朗读或加载则停止，否则使用 Edge TTS 朗读指定文本
 */
export async function toggleSpeak(
  id: string,
  rawText: string,
  opts: {
    voice?: string | null;
    rate?: string | null;
    onFinish?: () => void;
  } = {},
): Promise<void> {
  ensureCrossWindowListener();

  if (tts.activeId === id && (tts.status === 'loading' || tts.status === 'playing')) {
    stopSpeak(true);
    opts.onFinish?.();
    return;
  }

  const text = cleanTextForTts(rawText);
  if (!text) return;

  stopLocalPlayback();
  const token = uid('tts');
  currentSessionToken = token;
  tts.activeId = id;
  tts.status = 'loading';
  tts.error = '';

  // 通知其他窗口停止旧的 TTS 播放
  ipc.ttsBroadcast({ action: 'start', id, token }).catch(() => {});

  const chunks = splitIntoPlayableChunks(text);
  if (chunks.length === 0) {
    stopLocalPlayback();
    return;
  }

  try {
    // 流水线预取：立即发起第 0 段合成，并在播放第 i 段时并发预取第 i+1 段
    let nextPromise: Promise<string> | null = ipc.ttsSpeak(chunks[0], opts.voice, opts.rate);

    for (let i = 0; i < chunks.length; i++) {
      if (token !== currentSessionToken) return;

      const curPromise = nextPromise!;
      // 提前并发预取下一段
      if (i + 1 < chunks.length) {
        nextPromise = ipc.ttsSpeak(chunks[i + 1], opts.voice, opts.rate);
      } else {
        nextPromise = null;
      }

      const dataUri = await curPromise;
      if (token !== currentSessionToken) return;

      tts.status = 'playing';
      await playAudioUri(dataUri, token);
    }

    if (token === currentSessionToken) {
      stopLocalPlayback();
      opts.onFinish?.();
    }
  } catch (err) {
    if (token !== currentSessionToken) return;
    // 若 Edge TTS 网络请求失败，自动无缝回退到本机离线语音合成
    try {
      await fallbackWebSpeech(text, token);
      if (token === currentSessionToken) {
        stopLocalPlayback();
        opts.onFinish?.();
      }
    } catch (fallbackErr) {
      if (token === currentSessionToken) {
        tts.error = (err as Error)?.message || String(fallbackErr);
        stopLocalPlayback();
        opts.onFinish?.();
      }
    }
  }
}
