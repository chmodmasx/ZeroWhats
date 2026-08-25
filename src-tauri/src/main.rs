// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod accounts;
mod clipboard;
mod commands;
mod config;
mod lock;
mod notification;
mod password;
mod scripts;
mod tray;
mod unread;
mod updater;
mod window;

use config::{config_path, Config};
use tauri::{Emitter, Listener, Manager, WindowEvent};

fn main() {
    let mut builder = tauri::Builder::default();

    // Single-instance must be the FIRST plugin: a second launch re-focuses the
    // existing window instead of starting a new process.
    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            window::show_main(app);
        }));
    }

    builder
        .plugin(
            // Registered early so every later plugin/setup step can log. Writes to
            // both stdout (dev) and the OS log dir (release builds have no
            // terminal attached, so this is the only way to get diagnostics back
            // from a user-reported bug).
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: None,
                    }),
                ])
                .build(),
        )
        .plugin(
            tauri_plugin_window_state::Builder::default()
                // WhatsApp account windows keep their native window state; the
                // fixed-size React utility windows always reopen centered.
                .with_denylist(&["settings", "about", "shortcuts", "lock", "update"])
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::save_config,
            commands::get_accounts,
            commands::add_account,
            commands::switch_account,
            commands::rename_account,
            commands::set_password,
            commands::remove_password,
            commands::reset_password,
            commands::forget_password_wipe,
            commands::get_platform,
            commands::set_theme,
            commands::unlock,
            commands::open_url,
            updater::check_for_update,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let cfg = Config::load(&config_path(&handle));

            apply_environment(&cfg);
            commands::apply_autostart(&handle, cfg.auto_start);

            // Register the remote-event bridge and tray before creating account
            // WebViews. Initialization scripts can emit readiness/unread events as
            // soon as their page event loop runs, so no first event can race setup.
            register_web_events(&handle);
            tray::build(&handle)?;

            // Account 1 keeps the historical `main` WebKit profile; every later
            // account gets its own isolated WebView storage in `window`.
            window::build_accounts(&handle, &cfg)?;
            window::apply_spellcheck(
                &handle,
                cfg.spellcheck_enabled,
                cfg.spellcheck_languages.clone(),
            );
            lock::apply_auto_lock(&handle);

            if cfg.password_hash.is_some() {
                lock::lock(&handle);
                lock::show_lock_window(&handle);
            }

            reassert_tray_menu(&handle);
            updater::start_background_check(&handle);
            Ok(())
        })
        .on_window_event(|win, event| {
            // Focus on ANY app window resets the auto-lock idle clock — typing in
            // Settings, or simply switching back to an already-focused window,
            // both count as "the user is here". Handled before the WhatsApp-window
            // filter below so every local window benefits too.
            if let WindowEvent::Focused(true) = event {
                lock::record_activity();
            }

            // Only WhatsApp account windows get close-to-tray and privacy-blur
            // behaviour. Local React windows keep their normal close lifecycle.
            if window::account_id_from_label(win.label()).is_none() {
                return;
            }

            match event {
                WindowEvent::CloseRequested { api, .. } => {
                    let app = win.app_handle();
                    let cfg = Config::load(&config_path(app));
                    if cfg.lock_on_close && cfg.password_hash.is_some() {
                        lock::lock(app);
                    } else {
                        let _ = win.hide();
                    }
                    api.prevent_close();
                }
                // Blur only the account page whose focus actually changed.
                WindowEvent::Focused(focused) => {
                    window::apply_unfocus_blur_for_label(win.app_handle(), win.label(), *focused);
                }
                _ => {}
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Re-asserts the tray menu shortly after the initial build. GNOME's
/// AppIndicator extension reads the icon + DBusMenu layout off the session bus;
/// if that registration races the extension's own startup (login, or right
/// after waking from sleep), the first `set_menu` can be missed with no error
/// surfaced back to us. A second assert a moment later is the standard
/// mitigation. Linux-only: macOS/Windows use native menu APIs and don't have
/// this race.
#[cfg(target_os = "linux")]
fn reassert_tray_menu(app: &tauri::AppHandle) {
    let handle = app.clone();

    tauri::async_runtime::spawn_blocking(move || {
        std::thread::sleep(std::time::Duration::from_millis(400));

        let main_handle = handle.clone();
        if let Err(e) = handle.run_on_main_thread(move || tray::refresh(&main_handle)) {
            log::warn!("tray re-assert: run_on_main_thread failed: {e}");
        }
    });
}

#[cfg(not(target_os = "linux"))]
fn reassert_tray_menu(_app: &tauri::AppHandle) {}

/// Applies config that has to be set as process environment before the webview
/// starts (proxy, hardware acceleration, Linux WebKit rendering).
fn apply_environment(cfg: &Config) {
    if cfg.proxy_enabled && !cfg.proxy_url.is_empty() {
        std::env::set_var("http_proxy", &cfg.proxy_url);
        std::env::set_var("https_proxy", &cfg.proxy_url);
    }

    if !cfg.hardware_acceleration {
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
    }

    #[cfg(target_os = "linux")]
    apply_linux_rendering();
}

/// WebKitGTK's DMABUF renderer leaves the window blank on many Linux/Wayland +
/// GPU-driver combinations (the page loads and runs, but nothing ever paints) —
/// disabling it is the standard, reliable fix. An explicit user override of the
/// env var is honoured.
///
/// Note: forcing the integrated GPU (Mesa EGL) was tried to recover GPU
/// compositing on hybrid Intel+NVIDIA laptops, but it blanked the window on this
/// setup (cross-GPU buffer sharing with the compositor), so we keep the simple,
/// always-renders path. On such laptops the trade-off is real: the renderer that
/// paints is the slower one.
#[cfg(target_os = "linux")]
fn apply_linux_rendering() {
    // Escape hatch: ZW_FORCE_SOFTWARE=1 keeps the slow-but-always-safe software
    // path (disable WebKit's DMABUF renderer), for setups where GPU acceleration
    // leaves the window blank.
    if std::env::var_os("ZW_FORCE_SOFTWARE").is_some() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        return;
    }

    // On NVIDIA + integrated-GPU laptops, WebKitGTK's DMABUF renderer blanks on
    // the NVIDIA proprietary path, so it's commonly disabled system-wide — which
    // drops WebKit to slow software compositing. Instead, pin THIS process's whole
    // GL/EGL/GBM stack to the integrated GPU (Mesa), where DMABUF works, and enable
    // it: hardware-accelerated WebKit, no blank. The override must be consistent —
    // a half-switch (EGL=Mesa but GBM/GLX still NVIDIA) is what blanks.
    const MESA_EGL: &str = "/usr/share/glvnd/egl_vendor.d/50_mesa.json";

    if std::path::Path::new(MESA_EGL).exists() {
        std::env::set_var("__EGL_VENDOR_LIBRARY_FILENAMES", MESA_EGL);
        std::env::set_var("__GLX_VENDOR_LIBRARY_NAME", "mesa");
        std::env::set_var("LIBVA_DRIVER_NAME", "iHD");
        std::env::remove_var("GBM_BACKEND");
        std::env::remove_var("__NV_PRIME_RENDER_OFFLOAD");
        std::env::remove_var("WEBKIT_DISABLE_DMABUF_RENDERER");

        // Point VA-API's device at the integrated GPU's render node. Otherwise
        // GStreamer probes the NVIDIA node first and logs a harmless-but-noisy
        // "DRM_IOCTL_VERSION, unsupported drm device by media driver: nvid"
        // before falling back. Detect the Intel node by PCI vendor id (0x8086)
        // rather than hard-coding renderD12x, which differs per machine.
        if let Some(node) = intel_render_node() {
            std::env::set_var("LIBVA_DRI3_DEVICE", &node);
            std::env::set_var("VAAPI_DEVICE", &node);
        }
    } else if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
}

/// Finds the integrated-Intel DRM render node (`/dev/dri/renderD*` whose PCI
/// vendor is 0x8086). Returns `None` on non-Intel or single-GPU machines, where
/// the default VA-API probing is already correct.
#[cfg(target_os = "linux")]
fn intel_render_node() -> Option<String> {
    let mut nodes: Vec<_> = std::fs::read_dir("/dev/dri")
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with("renderD"))
        .collect();
    nodes.sort();

    for name in nodes {
        let vendor = std::fs::read_to_string(format!("/sys/class/drm/{name}/device/vendor"))
            .ok()?
            .trim()
            .to_string();
        if vendor == "0x8086" {
            return Some(format!("/dev/dri/{name}"));
        }
    }
    None
}

#[derive(serde::Deserialize)]
struct ActionPayload {
    action: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountPayload {
    account_id: accounts::AccountId,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnreadPayload {
    account_id: accounts::AccountId,
    count: u32,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct NotifyPayload {
    account_id: accounts::AccountId,
    title: Option<String>,
    body: Option<String>,
    /// The sender avatar as a `data:` URL (see `web/notifications.js`), or
    /// `None` when unavailable.
    icon: Option<String>,
}

#[derive(serde::Deserialize)]
struct UrlPayload {
    url: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadPayload {
    account_id: accounts::AccountId,
    name: String,
    /// Base64-encoded file bytes (see `web/download.js`).
    data: String,
}

#[derive(serde::Deserialize)]
struct RevealPayload {
    path: String,
}

fn account_exists(app: &tauri::AppHandle, account_id: accounts::AccountId) -> bool {
    Config::load(&config_path(app))
        .accounts
        .get(account_id)
        .is_some()
}

/// Reveals an account only if it is still the live persisted selection. Page JS
/// merely announces readiness; Rust owns visibility so a reload can never revive
/// an account that was switched away from in the meantime.
fn reveal_account_if_active(app: &tauri::AppHandle, account_id: accounts::AccountId) {
    if lock::is_locked() {
        return;
    }

    let cfg = Config::load(&config_path(app));
    if cfg.password_hash.is_some()
        || cfg.accounts.active_id != account_id
        || cfg.accounts.get(account_id).is_none()
    {
        return;
    }

    let label = window::account_label(account_id);
    if let Some(account) = app.get_webview_window(&label) {
        let _ = account.unminimize();
        let _ = account.show();
        let _ = account.set_focus();
    }
}

/// Bridges the page-injected scripts to the backend. App commands can't be
/// invoked from the remote WhatsApp origin (only core commands can be granted to
/// it), so scripts emit narrowly-scoped events. Account-sensitive events carry a
/// stable id, which is validated before any cross-window action is performed.
fn register_web_events(app: &tauri::AppHandle) {
    let handle = app.clone();
    app.listen("zw://account-ready", move |event| {
        if let Ok(payload) = serde_json::from_str::<AccountPayload>(event.payload()) {
            if !account_exists(&handle, payload.account_id) {
                return;
            }
            let handle = handle.clone();
            let _ = handle.clone().run_on_main_thread(move || {
                reveal_account_if_active(&handle, payload.account_id)
            });
        }
    });

    let handle = app.clone();
    app.listen("zw://action", move |event| {
        if let Ok(payload) = serde_json::from_str::<ActionPayload>(event.payload()) {
            let handle = handle.clone();
            let _ = handle
                .clone()
                .run_on_main_thread(move || dispatch_action(&handle, &payload.action));
        }
    });

    let handle = app.clone();
    app.listen("zw://unread", move |event| {
        if let Ok(payload) = serde_json::from_str::<UnreadPayload>(event.payload()) {
            if !account_exists(&handle, payload.account_id) {
                log::warn!("unread event from unknown account {} ignored", payload.account_id);
                return;
            }
            let handle = handle.clone();
            let _ = handle.clone().run_on_main_thread(move || {
                unread::set(&handle, payload.account_id, payload.count)
            });
        }
    });

    let handle = app.clone();
    app.listen("zw://notify", move |event| {
        if let Ok(payload) = serde_json::from_str::<NotifyPayload>(event.payload()) {
            if !account_exists(&handle, payload.account_id) {
                log::warn!(
                    "notification event from unknown account {} ignored",
                    payload.account_id
                );
                return;
            }
            let handle = handle.clone();
            let _ = handle.clone().run_on_main_thread(move || {
                notification::notify(
                    &handle,
                    payload.account_id,
                    payload.title,
                    payload.body,
                    payload.icon,
                )
            });
        }
    });

    let handle = app.clone();
    app.listen("zw://open-external", move |event| {
        if let Ok(payload) = serde_json::from_str::<UrlPayload>(event.payload()) {
            if let Ok(url) = tauri::Url::parse(&payload.url) {
                if matches!(url.scheme(), "http" | "https" | "mailto") {
                    commands::open_external(&handle, &payload.url);
                } else {
                    log::warn!("blocked open-external with scheme '{}'", url.scheme());
                }
            }
        }
    });

    // Blob-URL attachment downloads are decoded and saved in Rust. The account
    // id is used only to route the completion toast back to its originating
    // window; it never participates in filesystem path construction.
    let handle = app.clone();
    app.listen("zw://download", move |event| {
        if let Ok(mut payload) = serde_json::from_str::<DownloadPayload>(event.payload()) {
            if !account_exists(&handle, payload.account_id) {
                log::warn!("download event from unknown account {} ignored", payload.account_id);
                return;
            }

            payload.name = std::path::Path::new(&payload.name)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "download".into());

            const MAX_DOWNLOAD_B64: usize = 341_333_334; // ~256 MB decoded
            if payload.data.len() > MAX_DOWNLOAD_B64 {
                log::warn!("blob download '{}' exceeds 256 MB limit", payload.name);
                emit_download_result(
                    &handle,
                    payload.account_id,
                    &payload.name,
                    false,
                    None,
                );
                return;
            }

            use base64::Engine;
            let bytes = match base64::engine::general_purpose::STANDARD.decode(&payload.data) {
                Ok(b) => b,
                Err(e) => {
                    log::warn!("failed to decode blob download '{}': {e}", payload.name);
                    emit_download_result(
                        &handle,
                        payload.account_id,
                        &payload.name,
                        false,
                        None,
                    );
                    return;
                }
            };

            let cfg = Config::load(&config_path(&handle));
            if cfg.auto_download {
                let (ok, path) = match window::save_download_bytes(&handle, &payload.name, &bytes) {
                    Ok(p) => (true, Some(p.to_string_lossy().into_owned())),
                    Err(e) => {
                        log::warn!("failed to save blob download '{}': {e}", payload.name);
                        (false, None)
                    }
                };
                emit_download_result(
                    &handle,
                    payload.account_id,
                    &payload.name,
                    ok,
                    path,
                );
            } else {
                use tauri_plugin_dialog::DialogExt;
                let account_id = payload.account_id;
                let name = payload.name.clone();
                let h = handle.clone();
                handle
                    .dialog()
                    .file()
                    .set_file_name(&payload.name)
                    .save_file(move |chosen| {
                        let Some(file_path) = chosen else { return };
                        let target = file_path.as_path().unwrap_or(std::path::Path::new(""));
                        if let Some(parent) = target.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        let (ok, path) = match std::fs::write(target, &bytes) {
                            Ok(()) => (true, Some(target.to_string_lossy().into_owned())),
                            Err(e) => {
                                log::warn!("failed to save download '{}': {e}", name);
                                (false, None)
                            }
                        };
                        emit_download_result(&h, account_id, &name, ok, path);
                    });
            }
        }
    });

    // "Show in folder" from the download toast: reveals the file in the OS file
    // manager (Nautilus, Dolphin, Finder, Explorer).
    let handle = app.clone();
    app.listen("zw://reveal-download", move |event| {
        if let Ok(payload) = serde_json::from_str::<RevealPayload>(event.payload()) {
            let reveal = std::path::Path::new(&payload.path);
            let dl_dir = window::download_dir_public(&handle);
            let ok = reveal
                .canonicalize()
                .ok()
                .zip(dl_dir.canonicalize().ok())
                .is_some_and(|(r, d)| r.starts_with(&d));
            if ok {
                use tauri_plugin_opener::OpenerExt;
                if let Err(e) = handle.opener().reveal_item_in_dir(&payload.path) {
                    log::warn!("reveal_item_in_dir failed for '{}': {e}", payload.path);
                }
            } else {
                log::warn!(
                    "blocked reveal-download outside downloads dir: '{}'",
                    payload.path
                );
            }
        }
    });

    // Image paste bridge: read clipboard files once and emit them only to the
    // account WebView that requested them.
    let handle = app.clone();
    app.listen("zw://paste-image-request", move |event| {
        if let Ok(payload) = serde_json::from_str::<AccountPayload>(event.payload()) {
            if !account_exists(&handle, payload.account_id) {
                log::warn!(
                    "clipboard event from unknown account {} ignored",
                    payload.account_id
                );
                return;
            }
            let files = clipboard::read_clipboard_files();
            let label = window::account_label(payload.account_id);
            if let Some(account) = handle.get_webview_window(&label) {
                let _ = account.emit_to(&label, "zw://paste-image-data", files);
            }
        }
    });

    // Mouse/keyboard activity inside any WhatsApp WebView resets the same global
    // auto-lock clock. Window-focus activity is handled separately above.
    app.listen("zw://activity", move |_event| {
        lock::record_activity();
    });
}

/// Emits a download-result event only to the account WebView that initiated it.
fn emit_download_result(
    app: &tauri::AppHandle,
    account_id: accounts::AccountId,
    name: &str,
    ok: bool,
    path: Option<String>,
) {
    if !account_exists(app, account_id) {
        return;
    }

    let label = window::account_label(account_id);
    if let Some(account) = app.get_webview_window(&label) {
        let _ = account.emit_to(
            &label,
            "zw://download-result",
            serde_json::json!({ "ok": ok, "name": name, "path": path }),
        );
    }
}

/// Routes a titlebar/menu action (or the auto-lock timer) to its handler.
fn dispatch_action(app: &tauri::AppHandle, action: &str) {
    match action {
        "lock" => lock::lock(app),
        "settings" => window::open_settings(app),
        "shortcuts" => window::open_shortcuts(app),
        "about" => window::open_about(app),
        "update" => window::open_update(app),
        other => log::warn!("unknown menu action: {other}"),
    }
}
