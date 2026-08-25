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
  if (!tauri?.window) return;

  const win = tauri.window.getCurrentWindow();

  // Every account window is created hidden to avoid the Linux compositing race
  // described in `window.rs`. Only the persisted active account may reveal
  // itself automatically; background accounts stay loaded but invisible. The
  // password lock still takes precedence over the active-account flag.
  const zw = window.__ZW || {};
  if (!zw.hasPassword && zw.isActiveAccount) {
    setTimeout(() => {
      win.show().catch(() => {});
    }, 0);
  }
})();
