/**
 * miniBot 插件后台 Service Worker
 * 统一代理所有网络请求，彻底绕过宿主网页的 CSP 限制与 HTTPS Mixed Content 拦截
 */

const DEFAULT_SERVER_URL = "http://127.0.0.1:18181";

// 注册右键菜单
chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({
    id: "minibot-save-page",
    title: "📥 复制/保存网页到 miniBot 知识库",
    contexts: ["page", "selection"]
  });

  chrome.contextMenus.create({
    id: "minibot-summarize-page",
    title: "✨ AI 总结整个网页",
    contexts: ["page"]
  });
});

// 处理右键菜单点击
chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  if (!tab || !tab.id) return;

  if (info.menuItemId === "minibot-save-page" || info.menuItemId === "minibot-summarize-page") {
    try {
      await chrome.tabs.sendMessage(tab.id, {
        action: info.menuItemId === "minibot-save-page" ? "EXTRACT_AND_SAVE" : "EXTRACT_AND_SUMMARIZE"
      });
    } catch (err) {
      console.error("[miniBot Background] 与页面通信失败:", err);
    }
  }
});

// 统一监听来自 Content Script 的网络请求代理
chrome.runtime.onMessage.addListener((request, sender, sendResponse) => {
  if (request.action === "GET_SETTINGS") {
    chrome.storage.sync.get({
      serverUrl: DEFAULT_SERVER_URL,
      apiKey: "",
      enableHoverImage: true,
      enableSelectionTip: true
    }, (items) => {
      sendResponse(items);
    });
    return true;
  }

  // 抑制桌面 Tip
  if (request.action === "SUPPRESS_DESKTOP_TIP") {
    fetch(`${request.serverUrl || DEFAULT_SERVER_URL}/v1/clip/suppress`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ ms: request.ms || 1200 })
    }).catch(() => {});
    sendResponse({ ok: true });
    return true;
  }

  // 代理 POST 请求 (保存网页、图片、划词)
  if (request.action === "PROXY_POST") {
    const { url, payload, apiKey } = request;
    const headers = { "Content-Type": "application/json" };
    if (apiKey) headers["Authorization"] = `Bearer ${apiKey}`;

    fetch(url, {
      method: "POST",
      headers,
      body: JSON.stringify(payload)
    })
      .then(async (res) => {
        const text = await res.text();
        let data = null;
        try {
          data = JSON.parse(text);
        } catch (_) {
          data = text;
        }
        sendResponse({ ok: res.ok, status: res.status, data });
      })
      .catch((err) => {
        sendResponse({ ok: false, error: err.message || "无法连接到 miniBot 本地服务" });
      });

    return true; // 异步响应
  }
});
