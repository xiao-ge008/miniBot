// 类型化的 IPC 封装：前端只通过这里调用 Rust
import { invoke, Channel } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type {
  AppInfo,
  ChatAttachment,
  ChatEvent,
  ChatMessage,
  ChatMessageRecord,
  ChatSession,
  ChatStats,
  CloudTestResult,
  CustomLocalModel,
  DocFileInfo,
  DocWriteResult,
  DownloadProgress,
  EngineOverview,
  GpuSnapshot,
  HistoryEntry,
  ImageSearchResponse,
  KbSearchHit,
  KbSearchResponse,
  KbStats,
  KbSyncSummary,
  LayaStatus,
  LocalSendItem,
  LocalSendPeer,
  LocalSendProgress,
  LocalSendSelfInfo,
  ModelParams,
  OcrResult,
  Payload,
  QrResult,
  RecorderStateInfo,
  RecordStartArgs,
  ScannedFolderResult,
  SearchHit,
  SearchResponse,
  Settings,
  StatusInfo,
  UpdateDownloadProgress,
  UpdateInfo,
  VoiceOption,
} from './types';

export const ipc = {
  takePayload: () => invoke<Payload | null>('take_payload'),
  hideSelf: () => invoke<void>('hide_self'),
  minimizeSelf: () => invoke<void>('minimize_self'),
  openResult: (payload: Payload) => invoke<void>('open_result', { payload }),
  openPage: (route: string, title: string) => invoke<void>('open_page', { route, title }),
  closePage: () => invoke<void>('close_page'),
  openQr: (text: string) => invoke<void>('open_qr', { text }),
  openLocalSend: (payload?: Payload | null) => invoke<void>('open_localsend', { payload: payload ?? null }),
  trigger: (name: string) => invoke<void>('trigger', { name }),
  replaceText: (hwnd: number, text: string) => invoke<void>('replace_text', { hwnd, text }),

  getSettings: () => invoke<Settings>('get_settings'),
  saveSettings: (settings: Settings) => invoke<string[]>('save_settings', { settings }),
  hotkeysSuspend: (suspend: boolean) => invoke<void>('hotkeys_suspend', { suspend }),
  appInfo: () => invoke<AppInfo>('app_info'),

  // 知识库 API
  kbSaveClip: (text: string, sourceApp?: string, url?: string) =>
    invoke<string>('kb_save_clip', { text, sourceApp: sourceApp ?? null, url: url ?? null }),
  kbSaveImage: (imgData: string, ocrText?: string, visionDesc?: string, source?: string) =>
    invoke<string>('kb_save_image', {
      imgData,
      ocrText: ocrText ?? null,
      visionDesc: visionDesc ?? null,
      source: source ?? null,
    }),
  kbSaveWeb: (url: string, content: string, title?: string) =>
    invoke<string>('kb_save_web', { url, content, title: title ?? null }),
  kbGetStats: () => invoke<KbStats>('kb_get_stats'),
  kbOpenFolder: () => invoke<void>('kb_open_folder'),
  kbPickFolder: () => invoke<KbSyncSummary | null>('kb_pick_folder'),
  kbSyncIndex: () => invoke<KbSyncSummary>('kb_sync_index'),
  kbLayaStatus: () => invoke<LayaStatus>('kb_laya_status'),
  kbDownloadLaya: (onProgress: (p: DownloadProgress) => void) => {
    const ch = new Channel<DownloadProgress>();
    ch.onmessage = onProgress;
    return invoke<LayaStatus>('kb_download_laya', { onProgress: ch });
  },
  kbDeleteLaya: () => invoke<void>('kb_delete_laya'),
  kbOpenFile: (path: string) => invoke<void>('kb_open_file', { path }),
  kbRevealFile: (path: string) => invoke<void>('kb_reveal_file', { path }),
  kbReadImageBase64: (path: string) => invoke<string>('kb_read_image_base64', { path }),
  docRead: (path: string) => invoke<DocFileInfo>('doc_read_file', { path }),
  docWrite: (path: string, content: string) =>
    invoke<DocWriteResult>('doc_write_file', { path, content }),
  docOpenEditor: (path?: string | null, query?: string | null) =>
    invoke<void>('doc_open_editor', { path: path ?? null, query: query ?? null }),
  docGetPending: () => invoke<[string, string | null] | null>('doc_get_pending'),
  docSetContextMenu: (enable: boolean) =>
    invoke<boolean>('doc_set_context_menu', { enable }),
  docIsContextMenuRegistered: () => invoke<boolean>('doc_is_context_menu_registered'),
  kbSearch: (query: string, limit = 20) =>
    invoke<KbSearchResponse>('kb_search', { query, limit }),
  saveImageToKnowledgeWithVision: async (
    imageData: string,
    sourceApp?: string | null,
    existingOcr?: string | null,
    existingVision?: string | null,
  ): Promise<{ path: string; ocr: string; vision: string }> => {
    // 1. OCR 提取文字
    let ocrText = existingOcr ?? '';
    if (!ocrText) {
      try {
        const ocrRes = await ipc.ocrImage(imageData);
        ocrText = ocrRes.text ?? '';
      } catch {
        /* OCR 容错 */
      }
    }

    // =========================================================================
    // 阶段一：极速图像类型判定 (Fast Image Classification)
    // 耗时 < 0.5 秒，快速判别图片核心类型，以便加载对应领域的终极专家提示词
    // =========================================================================
    let detectedType = 'general';
    try {
      const clsId = uid('vis_cls');
      const clsMessages: ChatMessage[] = [
        {
          role: 'user',
          content: [
            { type: 'image_url', image_url: { url: imageData } },
            {
              type: 'text',
              text: `请快速判定这张图片最核心的单一类别，只输出以下 5 个英文标识之一，严禁输出任何多余标点或废话：
- character（包含人物、角色、动漫立绘、人像写真、模特、Cosplay、以人物为主体的照片）
- scenery（自然风光、名胜古迹、城市街景、建筑景观、室内空间环境，无人物为主体）
- ui_software（电脑或手机系统、软件界面、网页长图、聊天记录、代码编辑器、系统报错对话框）
- object_food（美食料理、日常器物、数码产品、手办摆件、车辆、道具静物特写）
- document_chart（图表数据报告、流程架构图、思维导图、文档通知、海报）
输出：`,
            },
          ],
        },
      ];
      let clsRaw = '';
      await ipc.llmChat(
        clsId,
        clsMessages,
        (ev) => {
          if (ev.type === 'delta') clsRaw += ev.text;
        },
        {
          params: {
            max_tokens: 20,
            temperature: 0.1,
          },
        },
      );
      const cleanCls = clsRaw.toLowerCase().trim();
      if (cleanCls.includes('character') || cleanCls.includes('人物') || cleanCls.includes('角色') || cleanCls.includes('立绘')) {
        detectedType = 'character';
      } else if (cleanCls.includes('scenery') || cleanCls.includes('风景') || cleanCls.includes('建筑') || cleanCls.includes('街景')) {
        detectedType = 'scenery';
      } else if (cleanCls.includes('ui_software') || cleanCls.includes('ui') || cleanCls.includes('界面') || cleanCls.includes('软件') || cleanCls.includes('聊天')) {
        detectedType = 'ui_software';
      } else if (cleanCls.includes('object_food') || cleanCls.includes('美食') || cleanCls.includes('静物') || cleanCls.includes('物体') || cleanCls.includes('食物')) {
        detectedType = 'object_food';
      } else if (cleanCls.includes('document_chart') || cleanCls.includes('图表') || cleanCls.includes('文档') || cleanCls.includes('架构')) {
        detectedType = 'document_chart';
      }
    } catch (e) {
      console.warn('图像类型快筛失败，回退到通用深度解析:', e);
    }

    // =========================================================================
    // 阶段二：针对命中类型的专家特化深度提示词 (Type-Specialized Profiler)
    // 让人类或AI后续以“穿什么衣服”、“什么风景光影”、“什么报错文字”都能精准命中
    // =========================================================================
    const hintContext = existingVision?.trim() ? `【已知前置线索/问答主题】：\n${existingVision.trim()}\n\n` : '';
    const ocrContext = ocrText.trim() ? `【画面中提取到的文字原词】：\n${ocrText.slice(0, 500).trim()}\n\n` : '';

    let specializedPrompt = '';
    let categoryTitle = '综合视觉特征解析';

    if (detectedType === 'character') {
      categoryTitle = '👤 角色形象与服饰穿搭深度解析';
      specializedPrompt = `你是一位顶级人物形象与服饰造型专家、视觉知识库索引专家。当前图片的核心主体是【人物/角色/立绘/人像】。
请对画面中的人物进行【巨细靡遗、穷尽一切细节】的深度描摹，绝对不要概括省略、不要写“等等”等套话。用户日后会通过诸如“穿什么衣服”、“什么发型发色”、“衣服上的花纹与配饰”、“佩戴什么首饰领结”、“什么鞋袜表情动作”等细微自然语言提问进行检索，你的解析必须确保 100% 准确命中！

${hintContext}${ocrContext}
请严格按以下 5 个结构展开极尽详尽的分析（使用丰富具体的形容词与名词）：

### 1. 角色身份、画风与核心情境
- 画面艺术风格（如动漫美少女立绘、原神二次元风格、写实真人写真、国风古装、赛博朋克角色、厚涂插画等）；
- 角色大致身份/设定特征（如贴身伴侣、女仆管家、学生校花、职场OL白领、古风刺客、魔法少女等）与画面主要情境。

### 2. 五官容貌与发型细节
- **发型发色**：发型具体款式（长直发/高双马尾/低双马尾/波浪大卷发/及肩短发/半扎丸子头/姬发式等）、刘海设计（齐刘海/八字空气刘海/碎刘海/中分）、具体发色（纯黑/浅金/银白/樱花粉/紫灰/渐变挑染与光泽质感）、发饰（发箍/发卡发夹/蝴蝶结丝带/猫耳发带/发簪等具体位置与颜色款式）；
- **容貌神情**：脸型轮廓、眉毛形态、眼型、瞳孔颜色（红瞳/蓝瞳/金瞳/异瞳、眼部高光与反光细节）、眼部妆容（眼线/眼影/腮红/唇彩）、面部微表情（温柔微笑/清冷傲然/害羞红晕/俏皮眨眼/深情凝视等情绪）。

### 3. 全身服饰穿搭（重中之重，必须穷尽一切细节）
- **上衣/内搭**：款式（修身衬衫/宽松T恤/露肩吊带背心/高领紧身毛衣/JK制服上衣/水手服/法式复古开衫/西服小外套等）、领口设计（翻领/立领/V领/水手领/镂空心形领）、袖型长短（泡泡袖/喇叭长袖/无袖/中袖）、布料面料质感（纯棉/丝绸光泽/蕾丝花边/皮革质感/薄纱透光/粗棒针织）、主体与边缘拼色、印花图案、胸口纽扣或系带细节；
- **下装**：款式（高腰百褶超短裙/百褶短裙/A字包臀裙/百褶长裙/阔腿牛仔裤/紧身瑜伽裤/短裤/西装长裤等）、颜色（深蓝/纯白/黑格纹/米色等）、裙摆褶皱程度与蓬松感、裤脚长度、腰带与金属扣环装饰；
- **全身外套/整体礼服**：长裙裙摆轮廓、风衣大衣、斗篷披肩、围裙围兜等外层服饰；
- **腿部与鞋袜（极细描摹）**：袜子款式（黑丝连裤袜/白丝过膝袜/半透明薄黑丝/透肉程度/蕾丝花边短袜/堆堆袜/长筒袜等）、袜口花边或束口圈；鞋子类型（黑色玛丽珍皮鞋/尖头高跟鞋/系带马丁靴/帆布鞋/老爹运动鞋/过膝长筒皮靴等）与鞋带绑带扣环细节；
- **首饰与配件**：耳环耳坠、项链锁骨链、领结/领花/蝴蝶结领带、戒指手镯手链、腰间挂饰挂坠、眼镜墨镜、手套袖套、随身挎包、手持道具（书本、折扇、法杖、饮料杯、雨伞、手机等）每一处物件。

### 4. 姿态动作、体态与镜头构图
- 身体姿态（端正站姿/侧身微倾/坐姿优雅/回眸顾盼/屈膝俯身/双手抱胸）、双手摆放位置与手指微动作、视线注视方向；
- 身材体态特征与镜头景别（全身完整立绘/半身中景/大头特写肖像/侧颜角度）与光影氛围。

### 5. 人物与穿搭高频检索关键词库 (Search Keywords)
请在最后列出 30-50 个逗号分隔的高频检索关键词（全面涵盖：角色名/主题名、别名、所有出现的衣服款式名、所有服饰颜色材质、发型发色特征、配饰道具名、姿态动作词、风格标签、日常自然语言提问高频词等），直接赋能全文检索精准命中。`;

    } else if (detectedType === 'scenery') {
      categoryTitle = '🏞️ 自然风光与建筑空间深度解析';
      specializedPrompt = `你是一位资深地理景观与环境摄影专家、视觉知识库索引专家。当前图片的核心主体是【自然风光/城市景观/建筑环境/室内空间】。
请对画面中的地理、地貌、空间建筑、光影天候进行【无死角、极尽详尽】的深度剖析。用户日后会搜索“海边日落”、“雪山森林”、“日式房间”、“现代办公室”、“复古街道”、“雨夜霓虹”等场景细节，你的解析必须确保 100% 准确命中！

${hintContext}${ocrContext}
请严格按以下 4 个结构展开详尽分析：

### 1. 景观地貌、空间属性与核心地标
- 场景具体归类：自然地理（雪山、森林、海滩岛屿、湖泊河流、草原荒漠、云海梯田等）/ 城市景观（摩天大楼天际线、立交桥公路、古镇街巷、夜市街景、商业步行街等）/ 室内空间（现代客厅、温馨书房、咖啡厅一隅、日式榻榻米和室、工业风工作室、豪华酒店等）；
- 核心地标或视觉主体景物（如孤岛灯塔、欧式古堡、跨海大桥、落地大玻璃窗、悬崖瀑布等）。

### 2. 空间构图与景物层次
- **前景元素**：画面最前方的具体物体（岩石块、野花丛、木质栏杆、桌椅家具、地面石板纹理、水面微波等）；
- **中景层次**：主要的建筑立面形态、树林林冠、山丘轮廓、街区街道细节；
- **远景与天际线**：远方山脉隐约轮廓、天际云层形状、海平线位置、远方建筑虚影；
- 构图手法（对称构图、三分法、对角线引导演示、仰角大透视、鸟瞰俯瞰全景等）。

### 3. 天候时段、光影质感与色彩氛围
- **时间与天候**：具体时段（清晨日出破晓、正午阳光、日落火烧晚霞、蓝调时刻、静谧深夜）、天气状况（晴空万里、云雾翻滚、细雨朦胧、暴风雨、大雪纷飞）；
- **光照质感**：光线方向（逆光剪影、侧光立体阴影、丁达尔神光耶稣光、泛光漫反射、水面倒影反光、夜间霓虹反射）；
- **主色调与氛围**：主色调搭配（暖金色系、冷蓝夜色、森系青绿、赛博朋克紫粉、复古胶片暖色等）与传达的情绪感（宁静、震撼、苍凉、温馨、神秘）。

### 4. 场景专属高频检索关键词库 (Search Keywords)
请在最后列出 25-45 个逗号分隔的高频检索关键词（全面涵盖：地理地貌名、空间类型、具体景物名、建筑风格、季节天气词、时段词、光影质感词、色彩色调词等）。`;

    } else if (detectedType === 'ui_software') {
      categoryTitle = '🖥️ 软件界面与文字功能深度解析';
      specializedPrompt = `你是一位资深全栈系统架构师与 UI/UX 软件界面分析专家。当前图片的核心主体是【软件界面/网页/系统报错/代码/聊天截图】。
请对界面中的软件所属、布局分区、菜单按钮、正文文字、错误日志与交互状态进行【逐字逐行、穷尽一切】的结构化记录。用户日后会搜索“某个界面的报错代码”、“某条对话记录原句”、“某按钮的位置”、“某个设置选项”，你的解析必须确保 100% 准确命中！

${hintContext}${ocrContext}
请严格按以下 5 个结构展开详尽分析：

### 1. 软件应用识别与界面形态
- 操作系统环境（Windows / macOS / Web 浏览器 / 移动端 App / Linux）；
- 软件或平台名称（如 VSCode、微信、Chrome、Terminal终端、IDEA、后台控制台、Docker、Database工具等）；
- 当前页面或窗口功能（如设置面板、代码编辑区、聊天对话会话、表格数据管理、登录认证页等）。

### 2. 窗口分区与界面布局
- 整体模块划分（顶部标题与菜单栏、左侧树形导航栏、中央主工作区、右侧属性检查器、底部控制台/状态栏等）；
- 处于激活/选中高亮状态的选项卡 Tab、面包屑路径与导航分支。

### 3. 关键文字、代码与内容记录（保留原词原句，便于精确文本检索）
- **核心文本内容**：标题文字、各个按钮名称、输入框占位符提示、列表项文字；
- **若为聊天记录**：发言人昵称、头像特征、对话时间线、每条气泡消息的完整文本；
- **若为代码片段**：编程语言（TypeScript/Rust/Python等）、类名、函数名、关键变量与逻辑语句；
- **若为错误与报警弹窗（重中之重）**：错误类型（TypeError/NullPointer等）、错误码、报错提示原句、关键堆栈行号。

### 4. 交互控件与视觉状态
- 复选框选中状态、开关开关状态、滑块数值、高亮警示图标（红标报错/黄标警告/绿色正常）。

### 5. UI 与软件高频检索关键词库 (Search Keywords)
请在最后列出 25-45 个逗号分隔的高频检索关键词（全面涵盖：软件名称、功能模块名、所有按钮标题名、报错错误代码、关键代码函数名、聊天主题词等）。`;

    } else if (detectedType === 'object_food') {
      categoryTitle = '☕ 美食料理与器物静物深度解析';
      specializedPrompt = `你是一位资深商品鉴定与美食摄影专家、视觉知识库索引专家。当前图片的核心主体是【生活实物/商品静物/美食料理/数码道具】。
请对物品名称、品牌型号、外观轮廓、材质质感、工艺细节、摆放环境进行【极尽详尽】的深度剖析。用户日后会搜索“什么品牌型号”、“金属还是塑料材质”、“什么菜品配料”、“放在什么桌上”，你的解析必须确保 100% 准确命中！

${hintContext}${ocrContext}
请严格按以下 4 个结构展开详尽分析：

### 1. 物品/料理品类与核心名称
- 明确的具体名称（如数码产品、机械腕表、手办模型、特定美食料理、饮品、跑车交通工具、家具家居等）；
- 品牌标识、Logo、具体型号或特征名（若画面有出现）。

### 2. 外观形态、材质工艺与色彩细节
- **外观形状与轮廓比例**：几何造型、线条曲线、结构组成；
- **表面材质与触感质地**：材质种类（磨砂金属/高光烤漆/透明亚克力/陶瓷釉面/实木纹理/头层真皮/碳纤维等）、反光高光特征与接缝工艺；
- **若为美食饮品**：菜系类别、烹饪方式（煎烤/烘焙/刺身/炖煮）、主料与配料食材、酱汁色泽黏稠度、盛放器皿餐具样式；
- **色彩与涂装**：主体颜色、点缀拼色、渐变与纹饰图案。

### 3. 陈设环境与摆放场景
- 摆放台面（大理石桌面、原木台面、展柜、纯色背景布等）、背景虚化程度、辅助陈设道具；
- 拍摄角度（正视俯拍、45度经典展示角、微距特写）与布光效果。

### 4. 实物道具高频检索关键词库 (Search Keywords)
请在最后列出 25-40 个逗号分隔的高频检索关键词（全面涵盖：物品准确名称、品类词、品牌型号、材质工艺词、颜色搭配词、食材配料词、场景词等）。`;

    } else if (detectedType === 'document_chart') {
      categoryTitle = '📊 数据图表与架构文档深度解析';
      specializedPrompt = `你是一位资深数据分析师与信息图表专家。当前图片的核心主体是【图表报表/流程架构图/思维导图/文档海报】。
请对图表类型、标题主旨、节点数据、逻辑上下游及核心结论进行【穷尽无遗】的结构化记录。用户日后会搜索“柱状图走势”、“架构图里的某个模块”、“文档中的某个规定数据”，你的解析必须确保 100% 准确命中！

${hintContext}${ocrContext}
请严格按以下 4 个结构展开详尽分析：

### 1. 图表文档类型与主标题
- 类型（折线趋势图、柱状对比图、饼图占比、微服务架构图、UML时序图、甘特图、思维脑图、说明书等）；
- 核心大标题、副标题、统计范围与作者/数据来源机构。

### 2. 数据指标与核心走势结论
- 坐标轴维度（X轴时间/分类、Y轴数值单位）；
- 关键数值极值（最高点、最低点、突变转折点）、增长率、占比百分比；
- 图表传达的核心结论与主要观点。

### 3. 架构流程与模块逻辑关系（若为架构/流程图）
- 各个核心节点/子系统名称（如 Gateway、Auth服务、Redis缓存、Database主从等）；
- 箭头指向方向、数据流转逻辑与上下游依赖关系。

### 4. 数据与图表高频检索关键词库 (Search Keywords)
请在最后列出 25-40 个逗号分隔的高频检索关键词（全面涵盖：主标题、所有节点名、指标词、核心数据词、技术栈名称、流程动作词等）。`;

    } else {
      categoryTitle = '🖼️ 综合视觉特征与全息要素解析';
      specializedPrompt = `你是一位顶尖的图像资产分析与视觉知识库索引专家。请对当前图片进行【深度、详尽、穷尽一切细节】的结构化视觉解析。
你的解析将直接用于个人知识库以文搜图。请务必巨细靡遗地描摹画面中出现的每一个元素与特征，绝对不要概括省略、不要写“等等”或套话，确保用户日后搜查任何细节时都能 100% 命中！

${hintContext}${ocrContext}
请从以下维度展开详尽分析：
### 1. 核心主题与画面情境
### 2. 主体对象与细节描摹（人物外貌服装、景物特征、物品材质或界面文字）
### 3. 场景背景、道具陈设与光影色彩
### 4. 全文检索高频关键词库（列出 25-45 个逗号分隔的关键词）。`;
    }

    let aiVisionAnalysis = '';
    try {
      const reqId = uid('vis_kb');
      const messages: ChatMessage[] = [
        {
          role: 'user',
          content: [
            {
              type: 'image_url',
              image_url: { url: imageData },
            },
            {
              type: 'text',
              text: specializedPrompt,
            },
          ],
        },
      ];
      let collected = '';
      await ipc.llmChat(
        reqId,
        messages,
        (ev) => {
          if (ev.type === 'delta') collected += ev.text;
        },
        {
          params: {
            max_tokens: 2048,
            temperature: 0.2,
          },
        },
      );
      aiVisionAnalysis = collected.trim();
    } catch (e) {
      console.warn('视觉特化深度解析调用异常，降级保存:', e);
    }

    // 组合最终存储的视觉描述（保留前置线索，融合专家模型穷尽式细节，冠以分类标签）
    let finalVisionDesc = `## ${categoryTitle}\n\n`;
    if (existingVision?.trim()) {
      finalVisionDesc += `### 📌 问答/主题摘要\n${existingVision.trim()}\n\n`;
    }
    if (aiVisionAnalysis) {
      finalVisionDesc += aiVisionAnalysis;
    } else if (existingVision?.trim()) {
      finalVisionDesc += existingVision.trim();
    }

    // 3. 后端保存原图与伴生同名 Markdown 文件
    const savedPath = await ipc.kbSaveImage(
      imageData,
      ocrText || undefined,
      finalVisionDesc || undefined,
      sourceApp ?? undefined,
    );

    // 4. 自动增量构建索引
    try {
      await ipc.kbSyncIndex();
    } catch {
      /* 忽略索引异常 */
    }

    return { path: savedPath, ocr: ocrText, vision: finalVisionDesc };
  },
  saveToKnowledge: async (p: Payload): Promise<string> => {
    if (p.kind === 'image' && p.image) {
      const res = await ipc.saveImageToKnowledgeWithVision(
        p.image,
        p.origin,
        p.text ?? undefined,
        p.meta?.vision_desc as string | undefined,
      );
      return res.path;
    } else if (p.kind === 'url' && p.text) {
      return await ipc.kbSaveWeb(p.text, p.text, p.meta?.title as string | undefined);
    } else if (p.text) {
      return await ipc.kbSaveClip(p.text, p.origin, (p.meta?.url as string) ?? undefined);
    }
    throw new Error('不支持的内容类型');
  },

  llmChat: (
    id: string,
    messages: ChatMessage[],
    onEvent: (e: ChatEvent) => void,
    opts: { params?: Record<string, unknown>; force?: boolean } = {},
  ) => {
    const ch = new Channel<ChatEvent>();
    ch.onmessage = onEvent;
    return invoke<string>('llm_chat', { id, messages, params: opts.params ?? null, force: opts.force ?? false, onEvent: ch });
  },
  llmCancel: (id: string) => invoke<void>('llm_cancel', { id }),
  llmStatus: () => invoke<StatusInfo>('llm_status'),
  llmTestApi: (baseUrl: string, apiKey: string, model: string) =>
    invoke<CloudTestResult>('llm_test_api', { baseUrl, apiKey, model }),
  llmFetchModels: (baseUrl: string, apiKey: string) =>
    invoke<string[]>('llm_fetch_models', { baseUrl, apiKey }),
  llmRelease: () => invoke<void>('llm_release'),
  llmStart: () => invoke<void>('llm_start'),
  llmCheckPort: (port: number) =>
    invoke<{ available: boolean; port: number; suggested_port: number }>('llm_check_port', { port }),
  engineOverview: () => invoke<EngineOverview>('engine_overview'),
  engineActivateModel: (modelId: string) => invoke<EngineOverview>('engine_activate_model', { modelId }),
  engineUpdateModelParams: (modelId: string, params: ModelParams) =>
    invoke<EngineOverview>('engine_update_model_params', { modelId, params }),
  engineScanCustomFolder: () => invoke<ScannedFolderResult | null>('engine_scan_custom_folder'),
  engineSaveCustomModel: (model: CustomLocalModel) =>
    invoke<EngineOverview>('engine_save_custom_model', { model }),
  engineDeleteCustomModel: (modelId: string) =>
    invoke<EngineOverview>('engine_delete_custom_model', { modelId }),
  enginePickModelsDir: () => invoke<[EngineOverview, number] | null>('engine_pick_models_dir'),
  engineDownloadModel: (modelId: string, mirror: string, onProgress: (p: DownloadProgress) => void) => {
    const ch = new Channel<DownloadProgress>();
    ch.onmessage = onProgress;
    return invoke<EngineOverview>('engine_download_model', { modelId, mirror, onProgress: ch });
  },
  engineDownloadRuntime: (backend: string, onProgress: (p: DownloadProgress) => void) => {
    const ch = new Channel<DownloadProgress>();
    ch.onmessage = onProgress;
    return invoke<EngineOverview>('engine_download_runtime', { backend, onProgress: ch });
  },
  engineCancelDownload: () => invoke<void>('engine_cancel_download'),
  gpuSnapshot: () => invoke<GpuSnapshot>('gpu_snapshot'),

  captureReady: () => invoke<void>('capture_ready'),
  captureClose: () => invoke<void>('capture_close'),
  sendToChat: (image: string, autoSend = true) => invoke<void>('send_to_chat', { image, autoSend }),
  webSearch: (query: string, maxResults = 5) => invoke<SearchResponse>('web_search', { query, maxResults }),
  copyText: (text: string) => invoke<void>('copy_text', { text }),
  copyImage: (data: string) => invoke<void>('copy_image', { data }),
  saveImage: async (data: string) => {
    if (typeof window !== 'undefined') (window as any).__mb_dialog_open = true;
    try {
      return await invoke<string>('save_image', { data });
    } finally {
      if (typeof window !== 'undefined') {
        setTimeout(() => {
          (window as any).__mb_dialog_open = false;
        }, 300);
      }
    }
  },
  ocrImage: (data: string) => invoke<OcrResult>('ocr_image', { data }),
  qrDecode: (data: string) => invoke<QrResult[]>('qr_decode', { data }),
  qrEncode: (text: string, ec: string, dark: string, light: string) => invoke<string>('qr_encode', { text, ec, dark, light }),
  pinImage: (data: string, x: number, y: number, w: number, h: number) => invoke<void>('pin_image', { data, x, y, w, h }),
  imageSearch: (data: string, engine = 'baidu') => invoke<ImageSearchResponse>('image_search', { data, engine }),
  fetchRemoteImage: (url: string) => invoke<string>('fetch_remote_image', { url }),

  historyAdd: (entry: HistoryEntry, image?: string | null) => invoke<number>('history_add', { entry, image: image ?? null }),
  historyList: (query: string, limit = 200) => invoke<HistoryEntry[]>('history_list', { query, limit }),
  historyDelete: (id: number) => invoke<void>('history_delete', { id }),
  historyClear: () => invoke<void>('history_clear'),
  readImageFile: (path: string) => invoke<string>('read_image_file', { path }),

  chatSessionList: () => invoke<ChatSession[]>('chat_session_list'),
  chatSessionCreate: (title = '新对话') => invoke<ChatSession>('chat_session_create', { title }),
  chatSessionRename: (id: string, title: string) => invoke<void>('chat_session_rename', { id, title }),
  chatSessionDelete: (id: string) => invoke<void>('chat_session_delete', { id }),
  chatMessageList: (sessionId: string) => invoke<ChatMessageRecord[]>('chat_message_list', { sessionId }),
  chatMessageAdd: (
    sessionId: string,
    role: string,
    content: string,
    attachments: ChatAttachment[] = [],
    stats?: ChatStats | null,
    kbHits?: KbSearchHit[] | null,
  ) =>
    invoke<number>('chat_message_add', {
      sessionId,
      role,
      content,
      attachments,
      stats: stats ?? null,
      kbHits: kbHits ?? null,
    }),
  chatMessageDeleteFrom: (sessionId: string, fromId: number) =>
    invoke<void>('chat_message_delete_from', { sessionId, fromId }),

  listUserAgents: () => invoke<unknown[]>('list_user_agents'),
  saveUserAgent: (agent: unknown) => invoke<void>('save_user_agent', { agent }),
  deleteUserAgent: (id: string) => invoke<void>('delete_user_agent', { id }),
  openAgentsDir: () => invoke<void>('open_agents_dir'),

  ttsVoices: () => invoke<VoiceOption[]>('tts_voices'),
  ttsSpeak: (text: string, voice?: string | null, rate?: string | null) =>
    invoke<string>('tts_speak', { text, voice: voice ?? null, rate: rate ?? null }),
  ttsBroadcast: (payload: Record<string, unknown>) => invoke<void>('tts_broadcast', { payload }),

  localSendInfo: () => invoke<LocalSendSelfInfo>('localsend_info'),
  localSendScan: (extraIps: string[] = []) => invoke<LocalSendPeer[]>('localsend_scan', { extraIps }),
  localSendProbe: (ip: string, port?: number) => invoke<LocalSendPeer>('localsend_probe', { ip, port: port ?? null }),
  localSendPickFiles: () => invoke<LocalSendItem[]>('localsend_pick_files'),
  localSendPickFolder: () => invoke<LocalSendItem[]>('localsend_pick_folder'),
  localSendResolvePaths: (paths: string[]) => invoke<LocalSendItem[]>('localsend_resolve_paths', { paths }),
  localSendSend: (
    peer: LocalSendPeer,
    items: LocalSendItem[],
    pin: string | null,
    onProgress: (p: LocalSendProgress) => void,
  ) => {
    const ch = new Channel<LocalSendProgress>();
    ch.onmessage = onProgress;
    return invoke<void>('localsend_send', { peer, items, pin: pin ?? null, onProgress: ch });
  },
  localSendCancel: () => invoke<void>('localsend_cancel'),
  localSendSetContextMenu: (enable: boolean) => invoke<boolean>('localsend_set_context_menu', { enable }),

  appCheckUpdate: () => invoke<UpdateInfo>('app_check_update'),
  appDownloadUpdate: (
    downloadUrl: string,
    assetName: string,
    useMirror: boolean,
    onProgress: (p: UpdateDownloadProgress) => void,
  ) => {
    const ch = new Channel<UpdateDownloadProgress>();
    ch.onmessage = onProgress;
    return invoke<string>('app_download_update', {
      downloadUrl,
      assetName,
      useMirror,
      onProgress: ch,
    });
  },
  appCancelUpdateDownload: () => invoke<void>('app_cancel_update_download'),
  appInstallUpdate: (installerPath: string) => invoke<void>('app_install_update', { installerPath }),
  openExternalUrl: (url: string) => invoke<void>('open_external_url', { url }),

  recorderStart: (args: RecordStartArgs) => invoke<RecorderStateInfo>('recorder_start', { args }),
  recorderPause: () => invoke<RecorderStateInfo>('recorder_pause'),
  recorderResume: () => invoke<RecorderStateInfo>('recorder_resume'),
  recorderStop: () => invoke<RecorderStateInfo>('recorder_stop'),
  recorderCancel: () => invoke<void>('recorder_cancel'),
  recorderStatus: () => invoke<RecorderStateInfo>('recorder_status'),
  recorderReveal: (path: string) => invoke<void>('recorder_reveal', { path }),
  recorderCopyFile: (path: string) => invoke<void>('recorder_copy_file', { path }),
};

/** 订阅本窗口的 Payload 投递：挂载时取一次，之后每次投递再取 */
export async function onPayload(cb: (p: Payload) => void): Promise<() => void> {
  const first = await ipc.takePayload();
  if (first) cb(first);
  return listen('minibot://payload', async () => {
    const p = await ipc.takePayload();
    if (p) cb(p);
  });
}

export function onLlmStatus(cb: (s: StatusInfo) => void) {
  return listen<StatusInfo>('llm://status', (e) => cb(e.payload));
}

export function onSettings(cb: (s: Settings) => void) {
  return listen<Settings>('minibot://settings', (e) => cb(e.payload));
}

export function onChatCapture(cb: (data: { image: string; auto_send: boolean }) => void) {
  return listen<{ image: string; auto_send: boolean }>('minibot://chat-capture', (e) => cb(e.payload));
}

export function onOutsideClick(cb: () => void) {
  return listen('minibot://outside-click', () => cb());
}

export function onDocOpen(cb: (payload: { path?: string | null; query?: string | null }) => void) {
  return listen<{ path?: string | null; query?: string | null }>('minibot://open-doc', (ev) => cb(ev.payload));
}

export function onTtsEvent(cb: (data: { action: string; id?: string; token?: string }) => void) {
  return listen<{ action: string; id?: string; token?: string }>('minibot://tts', (e) => cb(e.payload));
}

export function onRecorderState(cb: (s: RecorderStateInfo) => void) {
  return listen<RecorderStateInfo>('minibot://recorder-state', (e) => cb(e.payload));
}

/** 默认关闭自动联网搜索 */
export function shouldAutoWebSearch(_text: string): boolean {
  return false;
}

/** 将联网搜索结果格式化为注入给模型的上下文文本 */
export function formatSearchContext(resp: SearchResponse): string {
  if (!resp.hits || resp.hits.length === 0) return '';
  const lines = resp.hits.map(
    (h: SearchHit, idx: number) => `[${idx + 1}] 标题：${h.title}\n摘要：${h.snippet}\n链接：${h.url}`,
  );
  return `\n\n【实时联网搜索参考资料（当前时间：${resp.date}，关键词：${resp.query}）】\n${lines.join('\n\n')}\n\n请优先依据上述实时联网搜索资料准确、简洁地回答用户问题。`;
}

/** 格式化 AI 回复的 token 速度与消耗时长 */
export function formatChatStats(stats?: ChatStats | null): string {
  if (!stats || (stats.tokens <= 0 && stats.duration_ms <= 0)) return '';
  const sec = (stats.duration_ms / 1000).toFixed(stats.duration_ms < 10000 ? 2 : 1);
  const tps = stats.tokens_per_sec > 0 ? stats.tokens_per_sec.toFixed(1) : '0.0';
  return `${tps} tokens/s · 耗时 ${sec}s (${stats.tokens} tokens)`;
}

let seq = 0;
export const uid = (prefix = 'r') => `${prefix}${Date.now().toString(36)}${(seq++).toString(36)}`;
