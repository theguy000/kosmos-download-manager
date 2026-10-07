const HOST_NAME = "com.kosmos.downloader";

// Extension configuration state
let config = {
  interceptDownloads: true
};

// Load saved settings
chrome.storage.local.get(["interceptDownloads"], (items) => {
  if (items.interceptDownloads !== undefined) {
    config.interceptDownloads = items.interceptDownloads;
  }
});

// Update settings on change
chrome.storage.onChanged.addListener((changes, area) => {
  if (area === "local" && changes.interceptDownloads) {
    config.interceptDownloads = changes.interceptDownloads.newValue;
  }
});

// Setup context menus on installation
chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({
      id: "kosmos_download_link",
      title: "Download with KDM",
      contexts: ["link"]
    });
    chrome.contextMenus.create({
      id: "kosmos_download_media",
      title: "Download Media with KDM",
      contexts: ["video", "audio", "image"]
    });
  });
});

// Handle context menu clicks
chrome.contextMenus.onClicked.addListener((info, tab) => {
  const targetUrl = info.linkUrl || info.srcUrl;
  if (!targetUrl) return;

  sendToNativeHost({
    action: "download",
    url: targetUrl,
    referrer: info.pageUrl || (tab ? tab.url : undefined)
  }, (response) => {
    if (chrome.runtime.lastError || !response || response.status !== "ok") {
      console.warn("[KDM] Context menu download trigger failed:", chrome.runtime.lastError, response);
    }
  });
});

// Intercept browser downloads
chrome.downloads.onDeterminingFilename.addListener((item, suggest) => {
  if (!config.interceptDownloads) {
    return false;
  }

  const downloadUrl = item.finalUrl || item.url;
  // Skip internal browser URLs, blobs, and data URLs that cannot be fetched over network
  if (!downloadUrl || downloadUrl.startsWith("blob:") || downloadUrl.startsWith("data:") || downloadUrl.startsWith("chrome:") || downloadUrl.startsWith("edge:") || downloadUrl.startsWith("about:")) {
    return false;
  }

  // Defer determination asynchronously and forward to Kosmos Download Manager
  sendToNativeHost({
    action: "download",
    url: downloadUrl,
    filename: item.filename || undefined,
    referrer: item.referrer || undefined,
    total_bytes: item.fileSize > 0 ? item.fileSize : (item.totalBytes > 0 ? item.totalBytes : undefined)
  }, (response) => {
    if (chrome.runtime.lastError || !response || response.status !== "ok") {
      console.warn("[KDM] Native host did not handle download; continuing in browser:", chrome.runtime.lastError, response);
      // Fallback: let browser proceed with download
      suggest();
    } else {
      // Successfully handed over to Kosmos Download Manager! Cancel browser download
      chrome.downloads.cancel(item.id, () => {
        if (chrome.runtime.lastError) {
          suggest();
        }
      });
    }
  });

  // Return true to keep the suggest callback active for asynchronous resolution
  return true;
});

// Helper function to send messages to the native host
function sendToNativeHost(message, callback) {
  try {
    chrome.runtime.sendNativeMessage(HOST_NAME, message, (response) => {
      if (callback) {
        callback(response);
      }
    });
  } catch (err) {
    console.error("[KDM] sendNativeMessage exception:", err);
    if (callback) {
      callback(null);
    }
  }
}
