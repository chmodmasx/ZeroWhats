// Sets up a WhatsApp account window's page chrome. WhatsApp windows are squared
// off — no rounded corners, border or shadow — unlike the secondary React
// windows (Settings/About/…), which keep their floating-card look via their own
// CSS. We still normalize `html`/`body` here and keep the `body` transform so
// fixed children such as the injected titlebar stay positioned correctly.
(() => {
  "use strict";

  if (document.getElementById("zw-rounded-style")) return;

  const isDark = () => {
    const theme = (window.__ZW && window.__ZW.theme) || "system";
    return (
      theme === "dark" ||
      (theme === "system" && window.matchMedia?.("(prefers-color-scheme: dark)").matches)
    );
  };

  const style = document.createElement("style");
  style.id = "zw-rounded-style";
  style.textContent = `
    html{
      height:100% !important;
      background:${isDark() ? "#1d1d1f" : "#fafafb"} !important;
    }
    body{
      margin:0 !important;
      height:100% !important;
      overflow:hidden !important;
      background:${isDark() ? "#1d1d1f" : "#fafafb"} !important;
      transform:translateZ(0) !important;
      box-sizing:border-box !important;
    }
  `;

  (document.head || document.documentElement).appendChild(style);

  const tauri = window.__TAURI__;
  const accountId = window.__ZW?.accountId;
  if (!tauri?.event || !accountId) return;

  // Account windows are created hidden to avoid the Linux compositing race that
  // occurs when a transparent WebView is shown before its first page chrome is
  // installed. Rather than deciding visibility in page JS (which would become
  // stale after account switches/reloads), announce readiness and let Rust check
  // the current persisted active account and app-lock state before revealing it.
  setTimeout(() => {
    tauri.event.emit("zw://account-ready", { accountId }).catch(() => {});
  }, 0);
})();
