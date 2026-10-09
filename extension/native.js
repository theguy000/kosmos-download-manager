const HOST_NAME = "com.kosmos.downloader";
const TIMEOUT_MS = 5000;

// Sends one message to the native host. Resolves with the response, or null on
// error or timeout, so a hung host can never block the caller.
export function sendToNativeHost(message) {
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve(null), TIMEOUT_MS);
    const finish = (response) => {
      clearTimeout(timer);
      resolve(response ?? null);
    };
    try {
      chrome.runtime.sendNativeMessage(HOST_NAME, message, (response) => {
        const error = chrome.runtime.lastError;
        if (error) {
          console.warn("[KDM] Native host error:", error.message);
        }
        finish(error ? null : response);
      });
    } catch (err) {
      console.error("[KDM] sendNativeMessage exception:", err);
      finish(null);
    }
  });
}
