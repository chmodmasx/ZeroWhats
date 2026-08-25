// The custom in-page titlebar for each WhatsApp account webview.
//
// Every account gets its own native webview (rather than stacking multiple
// webviews), so input remains reliable on Linux and each session can use an
// isolated browser profile. The titlebar itself stays inside the page, where its
// dropdowns can overlay WhatsApp without a second webview clipping them.
(() => {
  "use strict";

  if (window.__zwTitlebar) return;
  const tauri = window.__TAURI__;

  if (!tauri) {
    console.error("[ZeroWhats] Tauri global is missing — titlebar disabled.");
    return;
  }

  window.__zwTitlebar = true;

  const BAR_HEIGHT = 44;

  const IS_MAC = /mac/i.test(navigator.userAgentData?.platform || navigator.platform || "");

  const STRINGS = (() => {
    const pt = (navigator.language || "en").toLowerCase().startsWith("pt");
    return pt
      ? {
          lock: "Bloquear",
          prefs: "Preferências",
          shortcuts: "Atalhos de Teclado",
          about: "Sobre o ZeroWhats",
          accounts: "Contas",
          manageAccounts: "Gerenciar contas…",
          menu: "Menu",
          minimize: "Minimizar",
          maximize: "Maximizar",
          restore: "Restaurar",
          close: "Fechar",
        }
      : {
          lock: "Lock",
          prefs: "Preferences",
          shortcuts: "Keyboard Shortcuts",
          about: "About ZeroWhats",
          accounts: "Accounts",
          manageAccounts: "Manage accounts…",
          menu: "Menu",
          minimize: "Minimize",
          maximize: "Maximize",
          restore: "Restore",
          close: "Close",
        };
  })();

  const ICONS = {
    menu: '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><line x1="3" y1="6" x2="21" y2="6"/><line x1="3" y1="12" x2="21" y2="12"/><line x1="3" y1="18" x2="21" y2="18"/></svg>',
    chevron:
      '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9"/></svg>',
    minimize:
      '<svg width="16" height="16" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2" stroke-linecap="round"><line x1="5" y1="12" x2="19" y2="12"/></svg>',
    // A single square = maximize; the offset double square = restore. The button
    // swaps between them as the window's maximized state changes.
    maximize:
      '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round"><rect x="4" y="4" width="16" height="16" rx="1.5"/></svg>',
    restore:
      '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round"><rect x="7" y="7" width="12" height="12" rx="1.5"/><path d="M5 15V6a1.5 1.5 0 0 1 1.5-1.5H16"/></svg>',
    close:
      '<svg width="15" height="15" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2" stroke-linecap="round"><line x1="6" y1="6" x2="18" y2="18"/><line x1="18" y1="6" x2="6" y2="18"/></svg>',
  };

  const STYLE = `
    #zw-bar{position:fixed;top:0;left:0;right:0;height:${BAR_HEIGHT}px;background:#161717;color:#fff;
      display:flex;align-items:center;gap:4px;padding:0 6px;z-index:2147483647;box-sizing:border-box;
      font-family:system-ui,'Segoe UI',sans-serif;font-size:13px;-webkit-user-select:none;user-select:none;}
    #zw-bar .zw-title{font-weight:600;padding:0 4px 0 6px;}
    #zw-bar .zw-spacer{flex:1;align-self:stretch;}
    #zw-bar button{all:unset;display:grid;place-items:center;width:36px;height:30px;border-radius:8px;cursor:pointer;color:#fff;}
    #zw-bar button:hover{background:rgba(255,255,255,.12);}
    #zw-bar button.zw-close:hover{background:#e01b24;}
    #zw-bar button.zw-account{display:flex;align-items:center;gap:5px;width:auto;max-width:190px;padding:0 8px;
      color:rgba(255,255,255,.78);font-size:12px;}
    #zw-bar button.zw-account .zw-account-name{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;}
    #zw-menu,#zw-account-menu{position:fixed;top:${BAR_HEIGHT + 2}px;z-index:2147483647;background:#2a2a2c;color:#fff;
      border:1px solid rgba(255,255,255,.12);border-radius:10px;padding:6px;min-width:248px;
      box-shadow:0 12px 34px rgba(0,0,0,.45);font-family:system-ui,sans-serif;font-size:13px;}
    #zw-menu button,#zw-account-menu button{all:unset;display:flex;justify-content:space-between;align-items:center;gap:24px;width:100%;
      box-sizing:border-box;padding:8px 10px;border-radius:7px;cursor:pointer;}
    #zw-menu button:hover,#zw-account-menu button:hover{background:rgba(255,255,255,.08);}
    #zw-menu .zw-acc,#zw-account-menu .zw-acc{opacity:.5;font-family:monospace;}
    #zw-menu .zw-sep,#zw-account-menu .zw-sep{height:1px;background:rgba(255,255,255,.12);margin:4px 2px;}
    #zw-account-menu .zw-current{color:#25d366;font-weight:600;}
    /* Shift WhatsApp down via a TRANSFORM (not top): a transformed ancestor
       becomes the containing block for its position:fixed descendants, so the
       media/image viewer and other fixed overlays stay below the bar. */
    html.zw-shift #app{transform:translateY(${BAR_HEIGHT}px);height:calc(100vh - ${BAR_HEIGHT}px) !important;will-change:transform;}
  `;

  const currentWindow = () => tauri.window.getCurrentWindow();
  const accountId = () => Number(window.__ZW?.accountId || 0);
  const accountName = () => window.__ZW?.accountName || `Account ${accountId() || 1}`;

  // App commands cannot be invoked from a remote origin (only core commands can
  // be granted to it), so menu actions are sent as events the Rust side listens
  // for. Event emit is a core command, allowed via the capability.
  const emitAction = (action) => {
    try {
      return tauri.event.emit("zw://action", { action });
    } catch (e) {
      console.error(`[ZeroWhats] emit action '${action}' failed`, e);
    }
  };

  const requestAccounts = () => {
    const id = accountId();
    if (!id) return;
    tauri.event.emit("zw://accounts-request", { accountId: id }).catch(() => {});
  };

  const switchAccount = (id) => {
    if (!id || id === accountId()) return;
    tauri.event.emit("zw://account-switch", { accountId: id }).catch((e) => {
      console.error(`[ZeroWhats] account switch to '${id}' failed`, e);
    });
  };

  // Lock is only offered once a password is configured (mirrors the tray).
  const hasPassword = () => !!(window.__ZW && window.__ZW.hasPassword);

  /** A lightweight in-page dropdown menu anchored under a titlebar control. */
  class Dropdown {
    constructor(items, id = "zw-menu") {
      this.items = items;
      this.id = id;
      this.el = null;
      this.anchorLeft = 0;
      this._onOutsidePointerDown = this._onOutsidePointerDown.bind(this);
    }

    get isOpen() {
      return this.el !== null;
    }

    toggle(anchorLeft) {
      if (this.isOpen) this.close();
      else this.open(anchorLeft);
    }

    open(anchorLeft) {
      this.anchorLeft = anchorLeft;
      this.el = document.createElement("div");
      this.el.id = this.id;
      this.el.style.left = `${anchorLeft}px`;

      for (const item of this.items) {
        if (item.when && !item.when()) continue;
        this.el.appendChild(item.separator ? this._renderSeparator() : this._renderItem(item));
      }

      (document.body || document.documentElement).appendChild(this.el);
      // Defer so the click that opened the menu doesn't immediately close it.
      setTimeout(() => document.addEventListener("mousedown", this._onOutsidePointerDown, true), 0);
    }

    close() {
      if (!this.el) return;
      this.el.remove();
      this.el = null;
      document.removeEventListener("mousedown", this._onOutsidePointerDown, true);
    }

    _renderSeparator() {
      const sep = document.createElement("div");
      sep.className = "zw-sep";
      return sep;
    }

    _renderItem({ label, accelerator, action, onSelect, current }) {
      const button = document.createElement("button");
      const text = document.createElement("span");

      text.textContent = label;
      button.appendChild(text);

      if (accelerator || current) {
        const acc = document.createElement("span");
        acc.className = current ? "zw-current" : "zw-acc";
        acc.textContent = current ? "✓" : accelerator;
        button.appendChild(acc);
      }

      button.addEventListener("click", () => {
        this.close();
        if (onSelect) onSelect();
        else if (action) emitAction(action);
      });

      return button;
    }

    _onOutsidePointerDown(event) {
      if (this.el && !this.el.contains(event.target)) this.close();
    }
  }

  /** The fixed top bar: menus, drag region and window controls. */
  class Titlebar {
    constructor() {
      this.accountsState = null;
      this.accountButton = null;
      this.accountMenu = null;
      this.menu = new Dropdown([
        { label: STRINGS.lock, accelerator: "Ctrl+L", action: "lock", when: hasPassword },
        { separator: true, when: hasPassword },
        { label: STRINGS.prefs, accelerator: "Ctrl+,", action: "settings" },
        { label: STRINGS.shortcuts, accelerator: "Ctrl+/", action: "shortcuts" },
        { label: STRINGS.about, action: "about" },
      ]);
    }

    mount() {
      this._injectStyle();
      document.documentElement.classList.add("zw-shift");

      this._listenAccounts();
      this._buildBar();
      requestAccounts();

      // WhatsApp can wipe the bar on navigation; cheaply re-add it if it vanishes.
      // A low-frequency poll avoids a subtree MutationObserver on WhatsApp's very
      // busy DOM (which firing per-mutation caused noticeable jank).
      setInterval(() => {
        if (!document.getElementById("zw-bar")) this._buildBar();
      }, 1500);

      // Keyboard accelerators mirroring the menu (Ctrl/Cmd + L / , / /).
      document.addEventListener(
        "keydown",
        (e) => {
          if (!(e.ctrlKey || e.metaKey) || e.altKey || e.shiftKey) return;
          const key = e.key.toLowerCase();
          if (key === "l" && hasPassword()) {
            e.preventDefault();
            emitAction("lock");
          } else if (key === ",") {
            e.preventDefault();
            emitAction("settings");
          } else if (key === "/") {
            e.preventDefault();
            emitAction("shortcuts");
          } else if (key === "w") {
            // Mirror the titlebar close button: the CloseRequested handler
            // hides (or locks) the window instead of quitting the app.
            e.preventDefault();
            currentWindow().close();
          }
        },
        true,
      );
    }

    _listenAccounts() {
      tauri.event
        .listen("zw://accounts-state", (event) => {
          const state = event.payload;
          if (!state || !Array.isArray(state.items)) return;
          this.accountsState = state;

          const own = state.items.find((account) => account.id === accountId());
          if (own && window.__ZW) window.__ZW.accountName = own.name;
          this._syncAccountButton();

          // If metadata changed while the switcher is open (rename/add), rebuild
          // it in place so the menu never displays stale account names.
          if (this.accountMenu?.isOpen) {
            const left = this.accountMenu.anchorLeft;
            this.accountMenu.close();
            this._openAccountMenu(left);
          }
        })
        .catch((e) => console.error("[ZeroWhats] account-state listener failed", e));
    }

    _injectStyle() {
      if (document.getElementById("zw-style")) return;

      const style = document.createElement("style");
      style.id = "zw-style";
      style.textContent = STYLE;

      (document.head || document.documentElement).appendChild(style);
    }

    _iconButton(svg, title, className) {
      const button = document.createElement("button");

      if (className) button.className = className;
      button.innerHTML = svg;
      button.title = title;

      return button;
    }

    _buildAccountButton() {
      const button = document.createElement("button");
      button.className = "zw-account";
      button.title = STRINGS.accounts;

      const name = document.createElement("span");
      name.className = "zw-account-name";
      name.textContent = accountName();
      button.appendChild(name);

      const chevron = document.createElement("span");
      chevron.innerHTML = ICONS.chevron;
      button.appendChild(chevron);

      button.addEventListener("click", (event) => {
        event.stopPropagation();
        requestAccounts();
        const left = Math.max(6, Math.round(button.getBoundingClientRect().left));
        if (this.accountMenu?.isOpen) this.accountMenu.close();
        else this._openAccountMenu(left);
      });

      this.accountButton = button;
      return button;
    }

    _syncAccountButton() {
      const name = this.accountButton?.querySelector(".zw-account-name");
      if (name) name.textContent = accountName();
    }

    _openAccountMenu(left) {
      const items = this.accountsState?.items?.length
        ? this.accountsState.items.map((account) => ({
            label: account.name,
            current: account.id === this.accountsState.active_id,
            onSelect: () => switchAccount(account.id),
          }))
        : [{ label: accountName(), current: true, onSelect: () => {} }];

      items.push({ separator: true });
      items.push({ label: STRINGS.manageAccounts, action: "settings" });

      this.accountMenu = new Dropdown(items, "zw-account-menu");
      this.accountMenu.open(left);
    }

    _buildBar() {
      if (document.getElementById("zw-bar")) return;

      const bar = document.createElement("div");
      bar.id = "zw-bar";

      const hamburger = this._iconButton(ICONS.menu, STRINGS.menu);

      const title = document.createElement("span");
      title.className = "zw-title";
      title.textContent = "ZeroWhats";

      const account = this._buildAccountButton();
      const spacer = document.createElement("div");
      spacer.className = "zw-spacer";

      const minimize = this._iconButton(ICONS.minimize, STRINGS.minimize);
      const maximize = this._iconButton(ICONS.maximize, STRINGS.maximize, "zw-max");
      const close = this._iconButton(ICONS.close, STRINGS.close, "zw-close");

      // Reflect the live maximized state on the button (icon + tooltip) so it
      // reads "restore" once maximized. Runs on mount and on every resize.
      // `maxState` mirrors it for the drag handler below (which must not start a
      // drag while maximized — see there).
      const win = currentWindow();
      let maxState = false;
      const syncMaxIcon = async () => {
        try {
          const isMax = await win.isMaximized();
          maxState = isMax;
          maximize.innerHTML = isMax ? ICONS.restore : ICONS.maximize;
          maximize.title = isMax ? STRINGS.restore : STRINGS.maximize;
        } catch (e) {
          console.error("[ZeroWhats] isMaximized failed", e);
        }
      };
      syncMaxIcon();
      try {
        win.onResized(() => syncMaxIcon());
      } catch (e) {
        console.error("[ZeroWhats] onResized listen failed", e);
      }

      const layout = IS_MAC
        ? [close, minimize, maximize, hamburger, title, account, spacer]
        : [hamburger, title, account, spacer, minimize, maximize, close];

      for (const el of layout) bar.appendChild(el);
      (document.body || document.documentElement).appendChild(bar);

      hamburger.addEventListener("click", (event) => {
        event.stopPropagation();
        this.menu.toggle(6);
      });

      minimize.addEventListener("click", () => currentWindow().minimize());
      maximize.addEventListener("click", async () => {
        try {
          await currentWindow().toggleMaximize();
          syncMaxIcon();
        } catch (e) {
          console.error("[ZeroWhats] toggleMaximize failed", e);
        }
      });
      close.addEventListener("click", () => currentWindow().close());

      // data-tauri-drag-region isn't reliably honoured inside a remote page, so
      // start the window drag explicitly on a primary press outside the buttons.
      //
      // `startDragging()` hands the pointer to the window manager, which swallows
      // the `dblclick` that would otherwise fire. So we detect the double click
      // ourselves (second primary press, fast + near the first) and toggle
      // maximize instead.
      //
      // The subtle bug this fixes: on a *maximized* window the WM restores as
      // soon as a drag starts (its tear-off behaviour). If the first press of a
      // double had started a drag, that press alone would restore, and the
      // second would re-maximize — so "restore" looked broken. Fix: never start
      // a drag while maximized (a maximized window has nowhere to drag to
      // anyway), so the double click's toggleMaximize is the only thing that
      // acts, and restore works. `maxState` is kept fresh by `syncMaxIcon`.
      let lastDown = 0;
      let lastX = 0;
      let lastY = 0;
      bar.addEventListener("mousedown", (event) => {
        if (event.button !== 0 || event.target.closest("button")) return;

        const now = Date.now();
        const isDouble =
          now - lastDown < 400 &&
          Math.abs(event.clientX - lastX) < 6 &&
          Math.abs(event.clientY - lastY) < 6;
        lastX = event.clientX;
        lastY = event.clientY;
        lastDown = isDouble ? 0 : now; // reset so a triple-click isn't two doubles

        if (isDouble) {
          (async () => {
            try {
              await currentWindow().toggleMaximize();
              syncMaxIcon();
            } catch (e) {
              console.error("[ZeroWhats] toggleMaximize failed", e);
            }
          })();
          return;
        }

        // Don't drag a maximized window — that would trigger the WM's restore
        // and fight the double-click-to-restore above.
        if (maxState) return;

        try {
          currentWindow().startDragging();
        } catch (e) {
          console.error("[ZeroWhats] startDragging failed", e);
        }
      });
    }
  }

  const start = () => new Titlebar().mount();
  if (document.readyState !== "loading") start();
  else document.addEventListener("DOMContentLoaded", start);
})();
