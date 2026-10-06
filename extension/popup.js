const HOST_NAME = "com.kosmos.downloader";

document.addEventListener("DOMContentLoaded", () => {
  const statusBadge = document.getElementById("status-badge");
  const interceptToggle = document.getElementById("intercept-toggle");

  // Load current interception setting
  chrome.storage.local.get(["interceptDownloads"], (items) => {
    if (items.interceptDownloads !== undefined) {
      interceptToggle.checked = items.interceptDownloads;
    }
  });

  // Handle toggle change
  interceptToggle.addEventListener("change", () => {
    chrome.storage.local.set({ interceptDownloads: interceptToggle.checked });
  });

  // Check connection status with native messaging host
  try {
    chrome.runtime.sendNativeMessage(HOST_NAME, { action: "ping" }, (response) => {
      if (chrome.runtime.lastError || !response || response.status !== "ok") {
        statusBadge.textContent = "Offline";
        statusBadge.className = "status-badge disconnected";
      } else {
        statusBadge.textContent = "Connected";
        statusBadge.className = "status-badge connected";
      }
    });
  } catch (err) {
    statusBadge.textContent = "Offline";
    statusBadge.className = "status-badge disconnected";
  }
});
