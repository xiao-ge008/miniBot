// 动作注册表：内容类型 × 动作。新增功能 = 注册一个 Action，Tip / 截图工具条 / 菜单自动出现
import { openUrl } from '@tauri-apps/plugin-opener';
import { BUILTIN_AGENTS, type Agent } from './agents';
import { ipc } from './ipc';
import { toggleSpeak } from './tts.svelte';
import type { Payload, PayloadKind } from './types';

/** 动作出现的位置 */
export type Surface = 'tip' | 'more' | 'capture' | 'launcher';

export interface Action {
  id: string;
  label: string;
  icon: string;
  accepts: PayloadKind[];
  surfaces: Surface[];
  order: number;
  /** 额外显示条件 */
  when?: (p: Payload) => boolean;
  /** 在结果浮窗中以哪个视图执行（chat / ocr / qr / palette / color …） */
  view?: string;
  /** 视图为 chat 时对应的智能体 */
  agent?: Agent;
  /** 不进结果浮窗、直接执行的动作 */
  run?: (p: Payload) => Promise<void> | void;
}

const registry = new Map<string, Action>();

export function register(a: Action) {
  registry.set(a.id, a);
}

export function getAction(id: string) {
  return registry.get(id);
}

export function allActions() {
  return [...registry.values()].sort((a, b) => a.order - b.order);
}

export function actionsFor(p: Payload, surface: Surface): Action[] {
  return allActions().filter((a) => a.surfaces.includes(surface) && a.accepts.includes(p.kind) && (!a.when || a.when(p)));
}

export function agents(): Agent[] {
  return allActions()
    .filter((a) => a.agent)
    .map((a) => a.agent!);
}

/** 执行动作：有视图的进结果浮窗，否则直接运行 */
export async function execute(id: string, p: Payload, extraMeta: Record<string, unknown> = {}) {
  const a = registry.get(id);
  if (!a) return;
  if (a.view) {
    await ipc.openResult({ ...p, meta: { ...(p.meta ?? {}), ...extraMeta, action: id } });
  } else if (a.run) {
    await a.run(p);
  }
}

function registerAgent(agent: Agent) {
  register({
    id: agent.id,
    label: agent.name,
    icon: agent.icon,
    accepts: agent.accepts,
    surfaces: agent.surfaces ?? ['more', 'launcher'],
    order: agent.order ?? 100,
    view: 'chat',
    agent,
  });
}

let loaded: Promise<void> | null = null;

/** 加载全部动作（内置 + 用户智能体），各窗口启动时调用一次 */
export function loadActions(): Promise<void> {
  if (loaded) return loaded;
  loaded = (async () => {
    BUILTIN_AGENTS.forEach((a) => registerAgent({ ...a, builtin: true }));

    // ---- 本地动作（不需要模型） ----
    register({ id: 'ocr', label: '识别文字', icon: '🔤', accepts: ['image'], surfaces: ['tip', 'capture'], order: 1, view: 'ocr' });
    register({
      id: 'image_search',
      label: '以图搜图',
      icon: '🔍',
      accepts: ['image'],
      surfaces: ['tip', 'capture', 'more'],
      order: 2,
      view: 'image_search',
    });
    register({
      id: 'qr',
      label: '识别二维码',
      icon: '▣',
      accepts: ['image'],
      surfaces: ['tip', 'capture'],
      order: 3,
      view: 'qr',
      when: (p) => p.origin === 'capture' || (p.meta?.qr?.length ?? 0) > 0,
    });
    register({ id: 'palette', label: '提取配色', icon: '🎨', accepts: ['image'], surfaces: ['capture', 'more'], order: 60, view: 'palette' });
    register({ id: 'color', label: '颜色', icon: '🎨', accepts: ['color'], surfaces: ['tip'], order: 1, view: 'color' });
    register({
      id: 'speak',
      label: '朗读',
      icon: '🔊',
      accepts: ['text', 'url'],
      surfaces: ['tip', 'more', 'launcher'],
      order: 48,
      run: (p) => toggleSpeak('action-speak', p.text ?? ''),
    });
    register({
      id: 'open_url',
      label: '打开链接',
      icon: '🔗',
      accepts: ['url', 'video'],
      surfaces: ['tip'],
      order: 3,
      run: (p) => openUrl(p.text ?? ''),
    });
    register({
      id: 'gen_qr',
      label: '生成二维码',
      icon: '▣',
      accepts: ['text', 'url', 'video'],
      surfaces: ['more', 'launcher'],
      order: 70,
      run: (p) => ipc.openQr(p.text ?? ''),
    });
    register({
      id: 'localsend',
      label: '局域网发送',
      icon: '📡',
      accepts: ['text', 'url', 'image', 'video', 'file'],
      surfaces: ['tip', 'capture', 'more', 'launcher'],
      order: 75,
      run: (p) => ipc.openLocalSend(p),
    });
    // 视频：预留入口，后续版本注册「抽帧描述 / 字幕总结 / 转 GIF」等动作即可

    // ---- 用户自定义智能体 ----
    try {
      const user = (await ipc.listUserAgents()) as Partial<Agent>[];
      for (const a of user) {
        if (a?.id && a?.template)
          registerAgent({ ...a, id: a.id, template: a.template, accepts: a.accepts ?? ['text'], system: a.system ?? '', icon: a.icon ?? '🧩', name: a.name ?? a.id, builtin: false });
      }
    } catch {
      /* 忽略损坏的用户配置 */
    }
  })();
  return loaded;
}
