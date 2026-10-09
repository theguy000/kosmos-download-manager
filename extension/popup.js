import { sendToNativeHost } from "./native.js";

const statusBadge = document.getElementById("status-badge");
const interceptToggle = document.getElementById("intercept-toggle");

function setStatus(text, state) {
  statusBadge.textContent = text;
  statusBadge.className = `status-badge ${state}`;
}

// Load current interception setting (default: on, matching background.js)
chrome.storage.local.get({ interceptDownloads: true }).then((items) => {
  interceptToggle.checked = items.interceptDownloads;
});

interceptToggle.addEventListener("change", () => {
  chrome.storage.local.set({ interceptDownloads: interceptToggle.checked });
});

// The host answers "pong_offline" when it is registered but the app is not running.
sendToNativeHost({ action: "ping" }).then((response) => {
  if (response?.status !== "ok") {
    setStatus("Offline", "disconnected");
  } else if (response.message === "pong_offline") {
    setStatus("App closed", "warning");
  } else {
    setStatus("Connected", "connected");
  }
});
