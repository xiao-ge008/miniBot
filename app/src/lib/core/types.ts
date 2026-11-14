// 与 Rust 端 payload.rs / settings.rs 对应的类型定义

/** 内容类型：新增类型（如视频）时在此扩展，动作注册表据此匹配 */
export type PayloadKind = 'text' | 'url' | 'color' | 'image' | 'video' | 'file';
export type Origin = 'clipboard' | 'selection' | 'capture' | 'launcher' | 'external';

export interface Payload {
  id: string;
  kind: PayloadKind;
  origin: Origin;
  text?: string | null;
  /** data:image/png;base64,... */
  image?: string | null;
  source_app?: string | null;
  source_hwnd: number;
  meta: Record<string, any> | null;
}

export interface QrResult {
  text: string;
  format: string;
  points: [number, number][];
}

export interface OcrLine {
  text: string;
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface OcrResult {
  text: string;
  lines: OcrLine[];
  lang: string;
}

export interface GpuProcess {
  pid: number;
  name: string;
  used_mb: number | null;
}

export interface GpuSnapshot {
  available: boolean;
  name: string;
  total_mb: number;
  used_mb: number;
  free_mb: number;
  util: number;
  processes: GpuProcess[];
}

export type LlmStatus = 'released' | 'loading' | 'ready' | 'yielded';

export interface StatusInfo {
  status: LlmStatus;
  remaining_secs: number;
  reason: string;
  api_enabled: boolean;
  api_model: string;
  reasoning_effort?: ReasoningEffort;
}

export interface CloudTestResult {
  ok: boolean;
  latency_ms: number;
  model: string;
  reply: string;
}

export type LlmError =
  | { kind: 'gpu_busy'; snapshot: GpuSnapshot; required_mb: number }
  | { kind: 'port_busy'; port: number }
  | { kind: 'cancelled' }
  | { kind: 'other'; message: string };

export interface ChatStats {
  tokens: number;
  tokens_per_sec: number;
  duration_ms: number;
}

export type ChatEvent =
  | { type: 'loading' }
  | { type: 'started' }
  | { type: 'delta'; text: string }
  | ({ type: 'stats' } & ChatStats);

export type ReasoningEffort = 'off' | 'low' | 'medium' | 'high' | 'max';

export interface ChatMessage {
  role: 'system' | 'user' | 'assistant';
  content: string | ({ type: 'text'; text: string } | { type: 'image_url'; image_url: { url: string } })[];
  reasoning_content?: string;
}

export interface ModelParams {
  ctx: number;
  ngl: number;
  kv_quant: string;
  reasoning_effort: ReasoningEffort;
  idle_secs: number;
  extra_args: string;
}

export interface CustomLocalModel {
  id: string;
  name: string;
  desc: string;
  folder_path: string;
  model_path: string;
  mmproj_path: string;
  vision: boolean;
  quant_tag: string;
  size_label: string;
  params: ModelParams;
}

export interface ScannedGgufFile {
  path: string;
  file_name: string;
  suggested_name: string;
  size_label: string;
  quant_tag: string;
}

export interface ScannedFolderResult {
  folder_path: string;
  models: ScannedGgufFile[];
  mmprojs: ScannedGgufFile[];
}

export interface Settings {
  autostart: boolean;
  target_lang: string;
  history_days: number;
  history_keep_images: boolean;
  save_dir: string;
  ui: {
    theme: string;
    font_family: string;
    font_size: number;
    line_height: number;
    result_width: number;
    result_height: number;
  };
  hotkeys: { capture: string; record: string; launcher: string; selection: string; color: string };
  llm: {
    server_path: string;
    models_dir: string;
    model_path: string;
    mmproj_path: string;
    mirror: string;
    port: number;
    ctx: number;
    ngl?: number;
    kv_quant?: string;
    reasoning_effort?: ReasoningEffort;
    idle_secs: number;
    required_vram_mb: number;
    extra_args: string;
    model_params?: Record<string, ModelParams>;
    custom_models?: CustomLocalModel[];
    api_enabled: boolean;
    api_provider: string;
    api_base_url: string;
    api_key: string;
    api_model: string;
    api_vision: boolean;
  };
  tip: { enabled: boolean; hide_ms: number; min_chars: number; max_chars: number; blacklist: string[] };
  tts: { voice: string; rate: string; volume: string; auto_lang: boolean };
  api: { enabled: boolean; port: number; lan: boolean; api_key: string; busy_policy: string; thinking: boolean };
  knowledge: KnowledgeSettings;
}

export interface KnowledgeSettings {
  base_path: string;
  laya_enabled: boolean;
  auto_search: boolean;
  candidate_count: number;
  top_k: number;
  clip_auto_distill: boolean;
}

export interface PromptTemplate {
  category: 'vision' | 'clip' | string;
  id: string;
  name: string;
  title: string;
  description: string;
  max_tokens: number;
  temperature: number;
  path: string;
  is_builtin: boolean;
  modified_ms: number;
  content: string;
  body: string;
}

export interface VisionAnalysisResult {
  detectedType: string;
  template: PromptTemplate | null;
  ocrText: string;
  rawAnalysis: string;
  finalVisionDesc: string;
  elapsedMs: number;
}

export interface ClipDistillResult {
  template: PromptTemplate | null;
  summary: string;
  elapsedMs: number;
}

export interface LayaStatus {
  installed: boolean;
  model_ready?: boolean;
  engine_ready?: boolean;
  needs_redownload?: boolean;
  size_mb: number;
  path: string;
  enabled: boolean;
}

export interface KbStats {
  base_path: string;
  notes_count: number;
  images_count: number;
  web_count: number;
  chunks_count: number;
  total_size_bytes: number;
}

export interface KbSyncSummary {
  folder_path: string;
  scanned_docs: number;
  total_chunks: number;
}

export interface KbSearchHit {
  path: string;
  relative_path: string;
  title: string;
  heading: string;
  content: string;
  line_start: number;
  line_end: number;
  score: number;
  origin_image?: string | null;
}

export interface KbSearchResponse {
  query: string;
  hits: KbSearchHit[];
  context_text: string;
}

export interface DocFileInfo {
  path: string;
  name: string;
  content: string;
  size: number;
  modified_ms: number;
  is_kb_file: boolean;
}

export interface DocWriteResult {
  path: string;
  size: number;
  modified_ms: number;
  kb_reindexed: boolean;
}

export interface VoiceOption {
  id: string;
  name: string;
  lang: string;
  gender: string;
  desc: string;
}

export interface AppInfo {
  version: string;
  api_url: string;
  gateway_error: string | null;
  hotkey_errors: string[];
  config_dir: string;
  agents_dir: string;
}

export interface HistoryEntry {
  id?: number;
  ts?: number;
  action: string;
  kind: string;
  input: string;
  output: string;
  image?: string | null;
  source_app?: string | null;
}

export interface ChatSession {
  id: string;
  title: string;
  created_at: number;
  updated_at: number;
}

export interface ChatAttachment {
  kind: 'image' | 'video';
  name: string;
  data_url: string;
  thumb_url?: string | null;
  info?: string | null;
}

export interface ChatMessageRecord {
  id?: number;
  session_id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  attachments: ChatAttachment[];
  stats?: ChatStats | null;
  kb_hits?: KbSearchHit[] | null;
  ts?: number;
}

export interface HardwareInfo {
  gpu_name: string;
  gpu_vendor: 'nvidia' | 'amd' | 'intel' | 'apple' | 'cpu';
  vram_gb: number;
  ram_gb: number;
  cpu_name: string;
  cpu_cores: number;
  low_end_device: boolean;
  low_end_reason: string;
  recommended_backend: string;
  recommended_backend_label: string;
  recommended_model: 'q4m' | 'q2s';
}

export interface ModelCard {
  id: string;
  name: string;
  quant_tag: string;
  desc: string;
  size_label: string;
  recommended: boolean;
  recommend_reason: string;
  installed: boolean;
  active: boolean;
  is_custom: boolean;
  vision: boolean;
  folder_path: string;
  model_path: string;
  mmproj_path: string;
  params: ModelParams;
}

export interface EngineOverview {
  engine_installed: boolean;
  engine_version: string;
  engine_backend: string;
  engine_dir: string;
  models_dir: string;
  mirror: string;
  hardware: HardwareInfo;
  models: ModelCard[];
}

export interface DownloadProgress {
  target: string;
  stage: 'downloading' | 'extracting' | 'done' | 'error';
  file: string;
  downloaded_mb: number;
  total_mb: number;
  percent: number;
  message: string;
}

export interface SearchHit {
  title: string;
  url: string;
  snippet: string;
  source: string;
}

export interface SearchResponse {
  query: string;
  date: string;
  hits: SearchHit[];
}

export interface ImageSearchExternalUrl {
  id: string;
  name: string;
  url: string;
}

export interface ImageSearchItem {
  title: string;
  subtitle: string;
  image_url: string;
  source_url: string;
  badge: string;
  width: number;
  height: number;
}

export interface ImageSearchResponse {
  engine: 'baidu' | 'tracemoe' | 'animetrace' | string;
  guess: string;
  page_url: string;
  public_image_url: string;
  external_urls: ImageSearchExternalUrl[];
  items: ImageSearchItem[];
}

export interface LocalSendSelfInfo {
  alias: string;
  lan_ips: string[];
  context_menu_enabled: boolean;
  os_model: string;
}

export interface LocalSendPeer {
  ip: string;
  port: number;
  alias: string;
  device_model: string;
  device_type: 'mobile' | 'desktop' | 'web' | 'headless' | 'server' | string;
  protocol: 'https' | 'http' | string;
  version: string;
  fingerprint: string;
}

export interface LocalSendItem {
  id: string;
  kind: 'file' | 'text' | 'image';
  name: string;
  path?: string | null;
  text?: string | null;
  data_url?: string | null;
  size: number;
  file_type: string;
}

export interface LocalSendProgress {
  stage: 'preparing' | 'waiting_accept' | 'sending' | 'done' | 'rejected' | 'pin_required' | 'cancelled' | 'error';
  file_id: string;
  file_name: string;
  current_index: number;
  total_files: number;
  sent_bytes: number;
  total_bytes: number;
  speed_mbps: number;
  percent: number;
  message: string;
}

export interface UpdateInfo {
  current_version: string;
  latest_version: string;
  has_update: boolean;
  release_name: string;
  release_notes: string;
  published_at: string;
  html_url: string;
  asset_name: string;
  download_url: string;
  asset_size_mb: number;
  platform: 'windows' | 'macos' | 'linux' | string;
}

export interface UpdateDownloadProgress {
  stage: 'downloading' | 'done' | 'cancelled' | 'error';
  file_name: string;
  downloaded_mb: number;
  total_mb: number;
  speed_mbps: number;
  percent: number;
  saved_path: string;
  message: string;
}

export interface RecordStartArgs {
  monitor_x: number;
  monitor_y: number;
  monitor_w: number;
  monitor_h: number;
  scale: number;
  crop_x: number;
  crop_y: number;
  crop_w: number;
  crop_h: number;
  fps: number;
  system_audio: boolean;
  mic_audio: boolean;
  capture_cursor: boolean;
}

export interface RecorderStateInfo {
  status: 'idle' | 'recording' | 'paused' | 'saving' | 'done' | 'error' | string;
  elapsed_ms: number;
  fps: number;
  width: number;
  height: number;
  system_audio: boolean;
  mic_audio: boolean;
  output_path: string;
  file_size_bytes: number;
  error?: string | null;
}

export interface KbQueueInput {
  kind: 'image' | 'clip' | 'web';
  title?: string | null;
  text?: string | null;
  url?: string | null;
  image_data?: string | null;
  existing_ocr?: string | null;
  existing_vision?: string | null;
  source_app?: string | null;
  force_type?: string | null;
}

export interface KbQueueTaskInfo {
  id: string;
  kind: string;
  title: string;
  source_app?: string | null;
  stage: 'queued' | 'starting' | 'ocr' | 'classify' | 'extract' | 'distill' | 'saving' | 'done' | 'error' | string;
  stage_label: string;
  detected_type?: string | null;
  saved_path?: string | null;
  error?: string | null;
  created_at: number;
  finished_at?: number | null;
}

export interface KbQueueStatus {
  max_size: number;
  active?: KbQueueTaskInfo | null;
  pending: KbQueueTaskInfo[];
  active_count: number;
  pending_count: number;
  total_in_flight: number;
  completed_count: number;
  failed_count: number;
  recent: KbQueueTaskInfo[];
  last_notice?: string | null;
}

export interface KbEnqueueResult {
  accepted: boolean;
  task_id?: string | null;
  position: number;
  in_flight: number;
  max_size: number;
  message: string;
  status: KbQueueStatus;
}



