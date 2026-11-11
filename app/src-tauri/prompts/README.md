# miniBot 知识库提示词模板目录

本目录存放 miniBot 在提取图片视觉特征、提纯与总结文本/网页片段时使用的全部 Markdown (`.md`) 提示词模板。**每次入库保存都会实时从磁盘重新读取，您修改后即时生效，无需重启软件。**

## 目录结构

- `vision/`：图片入库视觉识别与解析提示词（按类别独立加载）
  - `classifier.md`：阶段一图像快筛分类器（输出对应类别 ID）
  - `character.md`：唯一人物角色模板（单人、多人、穿搭、神态及亲密/裸露特征统一走此模板）
  - `scenery.md`：自然风光、城市街景与室内外空间解析
  - `ui_software.md`：软件界面、代码与报错堆栈解析
  - `object_food.md`：美食料理与器物静物解析
  - `document_chart.md`：数据图表与架构流程图解析
  - `general.md`：无法归入上述明确类别时的通用视觉解析
- `clip/`：片段与网页入库提纯总结提示词
  - `clip_distill.md`：划词/复制片段入库时的提纯与结构化总结
  - `web_distill.md`：网页链接/正文入库时的知识提炼与摘要

## 模板 Frontmatter 字段说明

每个 `.md` 文件顶部均可包含 YAML Frontmatter：

```yaml
---
id: character              # 模板唯一标识（与文件名一致）
name: 人物形象深度解析      # 模板显示名称
title: 👤 角色形象深度解析  # 入库生成 Markdown 的三级标题
description: 模板说明       # 简要用途说明
max_tokens: 4096           # 模型单次最大输出 Token 数
temperature: 0.2           # 模型采样温度（0.0 ~ 1.0）
---
```

## 支持的动态占位符

- `vision/` 类模板：
  - `{{HINT_CONTEXT}}`：已知前置线索或用户补充说明（若无则自动留空）
  - `{{OCR_CONTEXT}}`：画面中 OCR 提取出的原始文字（若无则自动留空）
- `clip/` 类模板：
  - `{{SOURCE_CONTEXT}}`：来源应用名与关联网址信息
  - `{{CLIP_TEXT}}`：待提纯与总结的原始文本片段或网页内容