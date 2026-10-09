import { sendToNativeHost } from "./native.js";

// Only schemes the desktop app can fetch (matches the CLI parser in src/main.rs).
const FETCHABLE_URL = /^(https?|ftp):/i;

function notify(message) {
  chrome.notifications.create({
    type: "basic",
    iconUrl: "icons/icon48.png",
    title: "KDM Integration Module",
    message
  });
}

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
chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  const targetUrl = info.linkUrl || info.srcUrl;
  if (!targetUrl) return;

  if (!FETCHABLE_URL.test(targetUrl)) {
    notify("KDM can only download http, https and ftp links.");
    return;
  }

  const response = await sendToNativeHost({
    action: "download",
    url: targetUrl,
    referrer: info.pageUrl || tab?.url
  });
  if (response?.status !== "ok") {
    console.warn("[KDM] Context menu download trigger failed:", response);
    notify("Could not reach KDM. Make sure the app is installed and registered.");
  }
});

// Hands a browser download to KDM. Resolves true when KDM took it and the
// browser download was cancelled, false when the browser should continue.
async function handOffDownload(item, url) {
  // Read per event: the service worker can be woken by this very download,
  // so an in-memory cache could still hold its default value.
  const { interceptDownloads = true } = await chrome.storage.local.get("interceptDownloads");
  if (!interceptDownloads) return false;

  const response = await sendToNativeHost({
    action: "download",
    url,
    // Chrome may report an absolute path; the host only needs the file name.
    filename: item.filename?.split(/[\\/]/).pop() || undefined,
    referrer: item.referrer || undefined,
    total_bytes: item.fileSize > 0 ? item.fileSize : (item.totalBytes > 0 ? item.totalBytes : undefined)
  });
  if (response?.status !== "ok") {
    console.warn("[KDM] Native host did not handle download; continuing in browser:", response);
    return false;
  }

  try {
    await chrome.downloads.cancel(item.id);
  } catch (err) {
    console.warn("[KDM] Could not cancel browser download:", err);
    return false;
  }
  // Best effort: drop the "Cancelled" row from the browser's download list.
  chrome.downloads.erase({ id: item.id }).catch((err) => {
    console.warn("[KDM] Could not erase cancelled download:", err);
  });
  return true;
}

// Intercept browser downloads
chrome.downloads.onDeterminingFilename.addListener((item, suggest) => {
  const url = item.finalUrl || item.url;
  // Leave other extensions' downloads alone, and skip blob:, data:, file: and
  // browser-internal URLs that cannot be fetched over the network.
  if (item.byExtensionId || !url || !FETCHABLE_URL.test(url)) {
    return false;
  }

  handOffDownload(item, url)
    .catch((err) => {
      console.error("[KDM] Download hand-off failed:", err);
      return false;
    })
    .then((handled) => {
      if (!handled) suggest();
    });

  // Return true to keep the suggest callback active for asynchronous resolution
  return true;
});
