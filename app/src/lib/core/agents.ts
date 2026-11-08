// 系统内置功能与简短提示词定义（无需用户手动配置）
import type { ChatMessage, Payload, PayloadKind, ReasoningEffort } from './types';
import type { Surface } from './actions';

export type { ReasoningEffort };

export interface ReasoningLevelOption {
  value: ReasoningEffort;
  label: string;
  shortLabel: string;
  budgetTokens: number;
}

/** 对齐 llama.cpp 内置 WebUI 的思考强度档位与 token 预算映射 (ize) */
export const REASONING_LEVELS: ReasoningLevelOption[] = [
  { value: 'off', label: '关闭思考', shortLabel: '关', budgetTokens: 0 },
  { value: 'low', label: '低强度 (512)', shortLabel: '低', budgetTokens: 512 },
  { value: 'medium', label: '中强度 (2K)', shortLabel: '中', budgetTokens: 2048 },
  { value: 'high', label: '高强度 (8K)', shortLabel: '高', budgetTokens: 8192 },
  { value: 'max', label: '满血无限制 (Max)', shortLabel: '满', budgetTokens: -1 },
];

/** 构建对齐 llama.cpp /v1/chat/completions 的思考控制参数 */
export function buildReasoningParams(effort: ReasoningEffort): Record<string, unknown> {
  const enableThinking = effort !== 'off';
  const budgetMap: Record<ReasoningEffort, number> = {
    off: 0,
    low: 512,
    medium: 2048,
    high: 8192,
    max: -1,
  };
  const budget = budgetMap[effort] ?? 8192;
  const out: Record<string, unknown> = {
    reasoning_format: 'auto',
    reasoning_control: true,
    chat_template_kwargs: { enable_thinking: enableThinking },
    thinking_budget_tokens: budget,
    reasoning_budget_tokens: budget,
  };
  if (enableThinking) {
    out.reasoning_effort = effort;
  }
  return out;
}

export interface AgentOption {
  key: string;
  label: string;
  choices: string[];
  default: string;
}

export interface Agent {
  id: string;
  name: string;
  icon: string;
  accepts: PayloadKind[];
  system: string;
  /** 模板变量：{{text}} {{target}} {{question}} {{opt.<key>}} */
  template: string;
  /** 输入为图片时：true=直接交给视觉模型；false=先 OCR 成文字 */
  vision?: boolean;
  /** 结果支持「替换原文」 */
  replace?: boolean;
  /** 多轮对话：先让用户输入问题 */
  chat?: boolean;
  /** 是否默认启用思考链 */
  thinking?: boolean;
  /** 默认思考强度（对齐 llama.cpp WebUI：off | low | medium | high | max） */
  reasoning_effort?: ReasoningEffort;
  surfaces?: Surface[];
  order?: number;
  temperature?: number;
  max_tokens?: number;
  options?: AgentOption[];
  builtin?: boolean;
}

export const BUILTIN_AGENTS: Agent[] = [
  {
    id: 'auto',
    name: '智能识别',
    icon: '✨',
    accepts: ['image'],
    surfaces: ['tip', 'capture'],
    order: 8,
    vision: true,
    temperature: 0.2,
    reasoning_effort: 'off',
    system:
      '你是智能视觉识别助手。若图片为文字则直接提取原文；若为公式则输出 $$ 包裹的 LaTeX；若为表格则输出 Markdown 表格；若为普通画面则用简短中文概括要点。',
    template: '请识别并提取这张图片的核心内容。',
  },
  {
    id: 'translate',
    name: '翻译',
    icon: '🌐',
    accepts: ['text', 'url', 'image'],
    surfaces: ['tip', 'capture', 'launcher'],
    order: 10,
    replace: true,
    vision: false,
    temperature: 0.2,
    reasoning_effort: 'off',
    system: '你是专业翻译助手。准确流畅地翻译内容，保留原格式与代码。只输出译文，不要解释。',
    template: '请把下面的内容翻译成{{target}}（风格：{{opt.style}}）：\n\n{{text}}',
    options: [{ key: 'style', label: '风格', choices: ['自然流畅', '直译', '学术严谨', '口语化'], default: '自然流畅' }],
  },
  {
    id: 'explain',
    name: '解释',
    icon: '💡',
    accepts: ['text', 'url', 'image', 'color'],
    surfaces: ['tip', 'capture', 'launcher'],
    order: 20,
    vision: true,
    thinking: true,
    reasoning_effort: 'high',
    temperature: 0.6,
    max_tokens: -1,
    system: '请用通俗中文先一句话概括核心含义，再分点说明关键细节或解决方案。',
    template: '请解释下面的内容：\n\n{{text}}',
  },
  {
    id: 'summarize',
    name: '总结',
    icon: '📝',
    accepts: ['text', 'url', 'image'],
    surfaces: ['tip', 'capture', 'launcher'],
    order: 30,
    vision: false,
    reasoning_effort: 'off',
    system: '你是高效摘要助手。用简练中文提炼核心要点，忠于原文，条理清晰。',
    template: '请用「{{opt.length}}」的形式总结下面的内容：\n\n{{text}}',
    options: [{ key: 'length', label: '长度', choices: ['三条要点', '一句话', '详细摘要'], default: '三条要点' }],
  },
  {
    id: 'polish',
    name: '润色',
    icon: '✍️',
    accepts: ['text'],
    surfaces: ['tip', 'launcher'],
    order: 40,
    replace: true,
    chat: true,
    temperature: 0.4,
    reasoning_effort: 'off',
    system: '你是资深文字编辑。请严格按照用户的润色方向或修改意见改写原文，保持原意与原语言。只输出改写后的正文，不加任何解释。',
    template: '润色要求：{{question}}\n\n待润色原文：\n"""\n{{text}}\n"""',
  },
  {
    id: 'qa',
    name: '问答',
    icon: '💬',
    accepts: ['text', 'url', 'image'],
    surfaces: ['tip', 'capture', 'launcher'],
    order: 45,
    chat: true,
    vision: true,
    thinking: true,
    reasoning_effort: 'high',
    temperature: 0.7,
    max_tokens: -1,
    system: '',
    template: '{{question}}\n\n参考内容：\n"""\n{{text}}\n"""',
  },
  {
    id: 'ocr_precise',
    name: '精准识别',
    icon: '🔍',
    accepts: ['image'],
    surfaces: [],
    order: 90,
    vision: true,
    temperature: 0,
    reasoning_effort: 'off',
    system: '你是高精度 OCR 引擎。只输出提取的文字与 Markdown 表格，不加任何解释。',
    template: '逐字提取图片中的全部文字，保持原有段落与排版。',
  },
];

const CJK = /[\u3400-\u9fff\uf900-\ufaff]/g;

/** 判断翻译目标语言：中文多 → 英文；否则 → 简体中文 */
export function translateTarget(text: string): string {
  const n = (text.match(CJK) || []).length;
  return n / Math.max(text.replace(/\s/g, '').length, 1) > 0.25 ? '英文' : '简体中文';
}

export function defaultOptions(agent: Agent): Record<string, string> {
  const o: Record<string, string> = {};
  for (const opt of agent.options ?? []) o[opt.key] = opt.default;
  return o;
}

export function fillTemplate(agent: Agent, vars: { text: string; question?: string; opts: Record<string, string> }) {
  return agent.template
    .replace(/\{\{text\}\}/g, vars.text)
    .replace(/\{\{question\}\}/g, vars.question || '请分析这段内容的要点。')
    .replace(/\{\{target\}\}/g, translateTarget(vars.text))
    .replace(/\{\{opt\.(\w+)\}\}/g, (_, k) => vars.opts[k] ?? '');
}

/**
 * 构建首轮消息。
 * - 文本输入：模板填充
 * - 图片 + vision：图片作为 image_url，模板中的 {{text}} 替换为“图片中的内容”
 */
export function buildMessages(
  agent: Agent,
  p: Payload,
  input: { text: string; image?: string | null; question?: string; opts: Record<string, string> },
): ChatMessage[] {
  const msgs: ChatMessage[] = [];
  if (agent.system && agent.system.trim()) {
    msgs.push({ role: 'system', content: agent.system.trim() });
  }
  if (input.image && agent.vision) {
    const prompt = input.text
      ? fillTemplate(agent, { ...input, text: input.text })
      : agent.id === 'qa'
        ? input.question || '请分析这张图片的内容。'
        : fillTemplate(agent, { ...input, text: '（见图片）' });
    msgs.push({
      role: 'user',
      content: [
        { type: 'image_url', image_url: { url: input.image } },
        { type: 'text', text: prompt },
      ],
    });
  } else {
    msgs.push({ role: 'user', content: fillTemplate(agent, input) });
  }
  void p;
  return msgs;
}
