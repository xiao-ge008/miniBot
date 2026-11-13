document.addEventListener("DOMContentLoaded", () => {
  const serverUrlInput = document.getElementById("serverUrl");
  const apiKeyInput = document.getElementById("apiKey");
  const enableHoverImage = document.getElementById("enableHoverImage");
  const enableSelectionTip = document.getElementById("enableSelectionTip");
  const btnSave = document.getElementById("btnSave");
  const connStatus = document.getElementById("connStatus");

  // 加载已保存设置
  chrome.storage.sync.get({
    serverUrl: "http://127.0.0.1:18181",
    apiKey: "",
    enableHoverImage: true,
    enableSelectionTip: true
  }, (items) => {
    serverUrlInput.value = items.serverUrl;
    apiKeyInput.value = items.apiKey;
    enableHoverImage.checked = items.enableHoverImage;
    enableSelectionTip.checked = items.enableSelectionTip;

    checkHealth(items.serverUrl);
  });

  // 检测本地服务健康度
  async function checkHealth(url) {
    try {
      const resp = await fetch(`${url}/health`, { method: "GET" });
      if (resp.ok) {
        connStatus.textContent = "● 已连接 App";
        connStatus.className = "status-badge online";
      } else {
        connStatus.textContent = "● App 异常";
        connStatus.className = "status-badge";
      }
    } catch (_) {
      connStatus.textContent = "● 未连接 App";
      connStatus.className = "status-badge";
    }
  }

  // 保存设置
  btnSave.addEventListener("click", () => {
    const newSettings = {
      serverUrl: serverUrlInput.value.trim() || "http://127.0.0.1:18181",
      apiKey: apiKeyInput.value.trim(),
      enableHoverImage: enableHoverImage.checked,
      enableSelectionTip: enableSelectionTip.checked
    };

    chrome.storage.sync.set(newSettings, () => {
      btnSave.textContent = "已保存 ✔";
      checkHealth(newSettings.serverUrl);
      setTimeout(() => {
        btnSave.textContent = "保存设置";
      }, 1500);
    });
  });
});
