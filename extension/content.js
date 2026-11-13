/**
 * miniBot 网页助手 Content Script
 * 包含：
 * 1. 剪贴板抑制指纹（方案 A）
 * 2. 网页正文智能提纯与入库 (通过 Background 代理，彻底规避宿主网页 CSP / Mixed Content)
 * 3. 图片悬停快捷采集
 * 4. 划词 / 复制 Shadow DOM 沉浸式浮窗
 */

(function () {
  if (window.__MINIBOT_INJECTED__) return;
  window.__MINIBOT_INJECTED__ = true;

  const ZERO_WIDTH_FINGERPRINT = "\u200B\u200C\u200B"; // 方案 A：零宽防冲突指纹
  let settings = {
    serverUrl: "http://127.0.0.1:18181",
    apiKey: "",
    enableHoverImage: true,
    enableSelectionTip: true
  };

  // 获取配置
  chrome.runtime.sendMessage({ action: "GET_SETTINGS" }, (res) => {
    if (res) settings = Object.assign(settings, res);
  });

  // 安全代理 POST 请求
  function safePost(endpoint, payload) {
    return new Promise((resolve, reject) => {
      chrome.runtime.sendMessage({
        action: "PROXY_POST",
        url: `${settings.serverUrl}${endpoint}`,
        apiKey: settings.apiKey,
        payload
      }, (resp) => {
        if (chrome.runtime.lastError) {
          return reject(new Error(chrome.runtime.lastError.message));
        }
        if (!resp) {
          return reject(new Error("插件后台未响应"));
        }
        if (resp.ok) {
          resolve(resp.data);
        } else {
          reject(new Error(resp.error || `请求失败 (HTTP ${resp.status})`));
        }
      });
    });
  }

  // =========================================================================
  // 1. 方案 A：剪贴板指纹注入与桌面 Tip 抑制
  // =========================================================================
  document.addEventListener("copy", (e) => {
    chrome.runtime.sendMessage({
      action: "SUPPRESS_DESKTOP_TIP",
      serverUrl: settings.serverUrl,
      ms: 1200
    });

    const selection = window.getSelection();
    if (!selection || selection.rangeCount === 0) return;
    const selectedText = selection.toString();
    if (!selectedText) return;

    try {
      if (e.clipboardData) {
        e.clipboardData.setData("text/plain", selectedText + ZERO_WIDTH_FINGERPRINT);
        e.clipboardData.setData("text/minibot-handled", "1");
        e.preventDefault();
      }
    } catch (_) {}
  }, true);

  // =========================================================================
  // 2. UI 宿主构建 (Shadow DOM 隔离)
  // =========================================================================
  const hostDiv = document.createElement("div");
  hostDiv.id = "minibot-extension-host";
  hostDiv.style.all = "initial";
  document.documentElement.appendChild(hostDiv);

  const shadow = hostDiv.attachShadow({ mode: "open" });

  const styleEl = document.createElement("style");
  styleEl.textContent = `
    * {
      box-sizing: border-box;
      margin: 0;
      padding: 0;
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
    }

    .minibot-tip-bar {
      position: absolute;
      z-index: 2147483647;
      display: flex;
      align-items: center;
      gap: 6px;
      padding: 6px 10px;
      background: rgba(28, 30, 36, 0.94);
      backdrop-filter: blur(12px);
      -webkit-backdrop-filter: blur(12px);
      border: 1px solid rgba(255, 255, 255, 0.12);
      border-radius: 10px;
      box-shadow: 0 8px 24px rgba(0, 0, 0, 0.28), 0 2px 6px rgba(0, 0, 0, 0.15);
      font-size: 13px;
      color: #e4e7ec;
      user-select: none;
      animation: minibot-fade-in 0.15s ease-out;
      pointer-events: auto;
    }

    @keyframes minibot-fade-in {
      from { opacity: 0; transform: translateY(4px) scale(0.96); }
      to { opacity: 1; transform: translateY(0) scale(1); }
    }

    .minibot-btn {
      display: inline-flex;
      align-items: center;
      gap: 4px;
      padding: 4px 8px;
      background: transparent;
      border: none;
      border-radius: 6px;
      color: #cbd5e1;
      cursor: pointer;
      font-size: 12px;
      font-weight: 500;
      transition: all 0.15s ease;
      white-space: nowrap;
    }
    .minibot-btn:hover {
      background: rgba(255, 255, 255, 0.12);
      color: #ffffff;
    }
    .minibot-btn:active {
      transform: scale(0.96);
    }
    .minibot-btn-primary {
      background: #4f46e5;
      color: #ffffff;
    }
    .minibot-btn-primary:hover {
      background: #6366f1;
    }

    .minibot-panel {
      position: absolute;
      z-index: 2147483647;
      width: 400px;
      max-height: 480px;
      display: flex;
      flex-direction: column;
      background: #18191f;
      border: 1px solid rgba(255, 255, 255, 0.12);
      border-radius: 12px;
      box-shadow: 0 16px 36px rgba(0, 0, 0, 0.4);
      color: #e2e8f0;
      overflow: hidden;
      animation: minibot-fade-in 0.18s ease-out;
    }

    .minibot-panel-header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 10px 14px;
      background: rgba(255, 255, 255, 0.04);
      border-bottom: 1px solid rgba(255, 255, 255, 0.08);
      font-size: 13px;
      font-weight: 600;
      color: #a5b4fc;
    }

    .minibot-panel-close {
      background: transparent;
      border: none;
      color: #94a3b8;
      cursor: pointer;
      font-size: 16px;
      line-height: 1;
      padding: 2px 4px;
      border-radius: 4px;
    }
    .minibot-panel-close:hover {
      color: #ffffff;
      background: rgba(255, 255, 255, 0.1);
    }

    .minibot-panel-body {
      padding: 14px;
      font-size: 13px;
      line-height: 1.6;
      overflow-y: auto;
      max-height: 380px;
      color: #f1f5f9;
      white-space: pre-wrap;
      word-break: break-word;
    }

    .minibot-img-badge {
      position: absolute;
      z-index: 2147483646;
      display: inline-flex;
      align-items: center;
      gap: 4px;
      padding: 5px 10px;
      background: rgba(17, 24, 39, 0.92);
      backdrop-filter: blur(8px);
      border: 1px solid rgba(255, 255, 255, 0.15);
      border-radius: 20px;
      color: #ffffff;
      font-size: 12px;
      font-weight: 500;
      cursor: pointer;
      box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3);
      transition: all 0.2s ease;
      opacity: 0;
      pointer-events: none;
      transform: translateY(-4px);
    }
    .minibot-img-badge.visible {
      opacity: 1;
      pointer-events: auto;
      transform: translateY(0);
    }
    .minibot-img-badge:hover {
      background: #4f46e5;
      transform: scale(1.05);
    }

    .minibot-toast {
      position: fixed;
      top: 24px;
      right: 24px;
      z-index: 2147483647;
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 10px 16px;
      background: #1e1b4b;
      color: #e0e7ff;
      border: 1px solid #4338ca;
      border-radius: 8px;
      box-shadow: 0 10px 25px rgba(0, 0, 0, 0.3);
      font-size: 13px;
      font-weight: 500;
      animation: minibot-fade-in 0.2s ease-out;
    }
  `;
  shadow.appendChild(styleEl);

  function showToast(message, duration = 3000) {
    const toast = document.createElement("div");
    toast.className = "minibot-toast";
    toast.textContent = message;
    shadow.appendChild(toast);
    setTimeout(() => {
      toast.style.transition = "opacity 0.3s ease";
      toast.style.opacity = "0";
      setTimeout(() => toast.remove(), 300);
    }, duration);
  }

  // =========================================================================
  // 3. 功能 1：网页正文提纯与知识库归档 (右键菜单调用)
  // =========================================================================
  async function extractPageArticle() {
    showToast("🔍 正在智能提取网页正文并提纯...");
    try {
      const reader = new window.MiniReadability(document);
      const article = reader.parse();
      const turndown = new window.MiniTurndown();
      const markdown = turndown.turndown(article.contentHtml);

      return {
        url: window.location.href,
        title: article.title || document.title,
        author: article.byline,
        content: markdown
      };
    } catch (err) {
      console.error("[miniBot] 正文提纯失败:", err);
      showToast("❌ 提取网页正文失败: " + err.message);
      return null;
    }
  }

  chrome.runtime.onMessage.addListener(async (msg) => {
    if (msg.action === "EXTRACT_AND_SAVE") {
      const data = await extractPageArticle();
      if (!data) return;

      try {
        await safePost("/v1/kb/web", {
          url: data.url,
          title: data.title,
          content: data.content,
          summary: data.author ? `作者: ${data.author}` : ""
        });
        showToast("✅ 已成功将网页保存到 miniBot 知识库！");
      } catch (err) {
        showToast("❌ 归档失败: " + err.message + " (请确认 App 已运行最新版)");
      }
    } else if (msg.action === "EXTRACT_AND_SUMMARIZE") {
      const data = await extractPageArticle();
      if (!data) return;
      openAiPanel("✨ AI 网页总结", `请对以下网页文章进行结构化要点总结：\n\n标题：${data.title}\n网址：${data.url}\n\n${data.content.slice(0, 4000)}`);
    }
  });

  // =========================================================================
  // 4. 功能 2：图片悬浮（Hover）快速采集
  // =========================================================================
  let hoverImgBadge = document.createElement("div");
  hoverImgBadge.className = "minibot-img-badge";
  hoverImgBadge.innerHTML = `<span>📥</span> <span>采集入库</span>`;
  shadow.appendChild(hoverImgBadge);

  let currentHoverImg = null;
  let hoverTimer = null;

  document.addEventListener("mouseover", (e) => {
    if (!settings.enableHoverImage) return;
    const target = e.target;
    if (target && target.tagName === "IMG") {
      const rect = target.getBoundingClientRect();
      if (rect.width >= 120 && rect.height >= 120) {
        clearTimeout(hoverTimer);
        hoverTimer = setTimeout(() => {
          currentHoverImg = target;
          const scrollX = window.scrollX || document.documentElement.scrollLeft;
          const scrollY = window.scrollY || document.documentElement.scrollTop;
          
          hoverImgBadge.style.left = `${rect.right + scrollX - 95}px`;
          hoverImgBadge.style.top = `${rect.top + scrollY + 8}px`;
          hoverImgBadge.classList.add("visible");
        }, 180);
      }
    }
  }, true);

  document.addEventListener("mouseout", (e) => {
    const related = e.relatedTarget;
    if (related && (related === hoverImgBadge || hostDiv.contains(related))) return;
    clearTimeout(hoverTimer);
    hoverTimer = setTimeout(() => {
      hoverImgBadge.classList.remove("visible");
      currentHoverImg = null;
    }, 250);
  }, true);

  hoverImgBadge.addEventListener("click", async (e) => {
    e.stopPropagation();
    if (!currentHoverImg) return;

    const imgUrl = currentHoverImg.currentSrc || currentHoverImg.src;
    hoverImgBadge.innerHTML = `<span>⏳</span> <span>采集...</span>`;

    try {
      let base64 = "";
      try {
        const canvas = document.createElement("canvas");
        canvas.width = currentHoverImg.naturalWidth || currentHoverImg.width;
        canvas.height = currentHoverImg.naturalHeight || currentHoverImg.height;
        const ctx = canvas.getContext("2d");
        ctx.drawImage(currentHoverImg, 0, 0);
        base64 = canvas.toDataURL("image/png");
      } catch (_) {
        // 跨域受阻时 base64 为空，由后端自动带 Referer 下载原图
      }

      await safePost("/v1/kb/image", {
        image_url: imgUrl,
        image_base64: base64,
        source: `${document.title} (${window.location.href})`
      });

      hoverImgBadge.innerHTML = `<span>✅</span> <span>已入库</span>`;
      showToast("🖼️ 图片已成功采集至知识库");
    } catch (err) {
      hoverImgBadge.innerHTML = `<span>❌</span> <span>失败</span>`;
      showToast("❌ 采集失败: " + err.message);
    }

    setTimeout(() => {
      hoverImgBadge.classList.remove("visible");
      hoverImgBadge.innerHTML = `<span>📥</span> <span>采集入库</span>`;
    }, 1500);
  });

  // =========================================================================
  // 5. 功能 3：划词 / 复制浮窗交互 (Tip & 流式 AI 问答)
  // =========================================================================
  let tipBar = document.createElement("div");
  tipBar.className = "minibot-tip-bar";
  tipBar.style.display = "none";
  tipBar.innerHTML = `
    <button class="minibot-btn minibot-btn-primary" data-action="summarize">✨ 总结</button>
    <button class="minibot-btn" data-action="explain">❓ 解释</button>
    <button class="minibot-btn" data-action="translate">🌐 翻译</button>
    <button class="minibot-btn" data-action="save">💾 入库</button>
    <button class="minibot-btn" data-action="copy">📋 复制</button>
  `;
  shadow.appendChild(tipBar);

  let aiPanel = document.createElement("div");
  aiPanel.className = "minibot-panel";
  aiPanel.style.display = "none";
  aiPanel.innerHTML = `
    <div class="minibot-panel-header">
      <span class="minibot-panel-title">🤖 miniBot AI</span>
      <button class="minibot-panel-close">&times;</button>
    </div>
    <div class="minibot-panel-body"></div>
  `;
  shadow.appendChild(aiPanel);

  aiPanel.querySelector(".minibot-panel-close").addEventListener("click", () => {
    aiPanel.style.display = "none";
  });

  let selectedTextCache = "";

  document.addEventListener("mouseup", (e) => {
    if (!settings.enableSelectionTip) return;
    if (hostDiv.contains(e.target)) return;

    setTimeout(() => {
      const selection = window.getSelection();
      const text = selection ? selection.toString().trim() : "";

      if (!text || text.length < 2) {
        tipBar.style.display = "none";
        return;
      }

      selectedTextCache = text;
      const range = selection.getRangeAt(0);
      const rect = range.getBoundingClientRect();

      const scrollX = window.scrollX || document.documentElement.scrollLeft;
      const scrollY = window.scrollY || document.documentElement.scrollTop;

      tipBar.style.left = `${Math.max(10, rect.left + scrollX)}px`;
      tipBar.style.top = `${rect.bottom + scrollY + 8}px`;
      tipBar.style.display = "flex";
    }, 50);
  });

  document.addEventListener("mousedown", (e) => {
    if (!hostDiv.contains(e.target)) {
      tipBar.style.display = "none";
    }
  });

  tipBar.addEventListener("click", async (e) => {
    const btn = e.target.closest("button");
    if (!btn) return;
    const action = btn.dataset.action;
    tipBar.style.display = "none";

    if (action === "copy") {
      navigator.clipboard.writeText(selectedTextCache + ZERO_WIDTH_FINGERPRINT);
      showToast("📋 已复制并抑制桌面 Tip");
      return;
    }

    if (action === "save") {
      try {
        await safePost("/v1/kb/clip", {
          text: selectedTextCache,
          url: window.location.href,
          summary: document.title
        });
        showToast("💾 划词已成功追加至今日知识库笔记！");
      } catch (err) {
        showToast("❌ 保存失败: " + err.message);
      }
      return;
    }

    let prompt = "";
    let title = "🤖 miniBot AI";
    if (action === "summarize") {
      title = "✨ 核心要点总结";
      prompt = `请对以下内容进行简明扼要的要点总结：\n\n${selectedTextCache}`;
    } else if (action === "explain") {
      title = "❓ 概念深入解释";
      prompt = `请通俗易懂地解释以下概念或语句：\n\n${selectedTextCache}`;
    } else if (action === "translate") {
      title = "🌐 智能翻译";
      prompt = `请将以下内容翻译为地道流畅的中文（如果已是中文则翻译为英文）：\n\n${selectedTextCache}`;
    }

    openAiPanel(title, prompt);
  });

  async function openAiPanel(title, prompt) {
    const titleEl = aiPanel.querySelector(".minibot-panel-title");
    const bodyEl = aiPanel.querySelector(".minibot-panel-body");

    titleEl.textContent = title;
    bodyEl.textContent = "正在思考中...";

    const scrollX = window.scrollX || document.documentElement.scrollLeft;
    const scrollY = window.scrollY || document.documentElement.scrollTop;
    aiPanel.style.left = `${Math.min(window.innerWidth - 420, Math.max(20, scrollX + 40))}px`;
    aiPanel.style.top = `${scrollY + 80}px`;
    aiPanel.style.display = "flex";

    try {
      const resp = await fetch(`${settings.serverUrl}/v1/chat/completions`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          ...(settings.apiKey ? { "Authorization": `Bearer ${settings.apiKey}` } : {})
        },
        body: JSON.stringify({
          model: "minibot-2b",
          messages: [
            { role: "system", content: "你是一个智能高效的阅读与知识提取助手，用清晰优美的排版回答。" },
            { role: "user", content: prompt }
          ],
          stream: true
        })
      });

      if (!resp.ok) {
        bodyEl.textContent = `⚠️ 请求失败 (HTTP ${resp.status})，请检查 App 设置与模型状态。`;
        return;
      }

      bodyEl.textContent = "";
      const reader = resp.body.getReader();
      const decoder = new TextDecoder("utf-8");
      let done = false;
      let buffer = "";

      while (!done) {
        const { value, done: readerDone } = await reader.read();
        done = readerDone;
        if (value) {
          buffer += decoder.decode(value, { stream: true });
          const lines = buffer.split("\n");
          buffer = lines.pop();

          for (const line of lines) {
            const trimmed = line.trim();
            if (trimmed.startsWith("data: ")) {
              const dataStr = trimmed.slice(6);
              if (dataStr === "[DONE]") continue;
              try {
                const parsed = JSON.parse(dataStr);
                const delta = parsed.choices?.[0]?.delta?.content || "";
                bodyEl.textContent += delta;
                bodyEl.scrollTop = bodyEl.scrollHeight;
              } catch (_) {}
            }
          }
        }
      }
    } catch (err) {
      bodyEl.textContent = "❌ 连接本地大模型失败: " + err.message + "\n\n请确保 miniBot 桌面 App 正在运行。";
    }
  }
})();
