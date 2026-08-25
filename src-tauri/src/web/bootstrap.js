// The first script to run: seeds `window.__ZW` with global config plus the
// stable identity of this WhatsApp account before the rest of the injected
// scripts execute. The placeholders keep this file valid/lintable on its own;
// `scripts.rs` replaces them with concrete values before injection.
window.__ZW = Object.assign(window.__ZW || {}, {
  theme: "__ZW_THEME__",
  autoLockMinutes: "__ZW_AUTO_LOCK_MINUTES__",
  hasPassword: "__ZW_HAS_PASSWORD__",
  spellcheck: "__ZW_SPELLCHECK__",
  accountId: "__ZW_ACCOUNT_ID__",
  accountName: "__ZW_ACCOUNT_NAME__",
});

try {
  localStorage.setItem("theme", JSON.stringify("__ZW_THEME__"));
} catch {}
