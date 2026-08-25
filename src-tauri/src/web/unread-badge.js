// Mirrors this WhatsApp account's unread count onto the system tray.
//
// WhatsApp keeps the total in `document.title` (e.g. "(3) WhatsApp"), so we
// watch the <title> node and forward the number together with the stable account
// id. Rust aggregates every account before drawing the single tray badge.
(() => {
  "use strict";

  const tauri = window.__TAURI__;
  const accountId = window.__ZW?.accountId;

  const readCount = () => {
    const match = (document.title || "").match(/\((\d+)\)/);
    return match ? parseInt(match[1], 10) : 0;
  };

  let lastCount = -1;
  const push = () => {
    const count = readCount();

    if (count === lastCount) return;
    lastCount = count;
    // App commands are blocked from this remote origin, so the count is sent as
    // an event the Rust side listens for (event emit is a core command).
    try {
      tauri?.event?.emit("zw://unread", { accountId, count });
    } catch (e) {
      console.error("[ZeroWhats] emit unread failed", e);
    }
  };

  const start = () => {
    push();
    const titleEl = document.querySelector("title");
    if (titleEl) {
      new MutationObserver(push).observe(titleEl, {
        childList: true,
        characterData: true,
        subtree: true,
      });
    }
    // WhatsApp occasionally swaps the whole <title> node, detaching the observer
    // above; a slow poll catches that without busy-watching the DOM.
    setInterval(push, 4000);
  };

  if (document.readyState !== "loading") start();
  else document.addEventListener("DOMContentLoaded", start);
})();
