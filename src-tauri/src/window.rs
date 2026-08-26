//! Window creation and management: account-scoped WhatsApp windows (with page
//! scripts injected and a navigation allow-list) and the frameless React windows
//! (Settings / About / Shortcuts).

use std::path::{Path, PathBuf};
use tauri::webview::DownloadEvent;
use tauri::{AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindowBuilder};

use crate::accounts::{Account, AccountId, Accounts, PRIMARY_ACCOUNT_ID};
use crate::config::{config_path, Config, Theme};
use crate::{commands, lock, scripts};

/// Where downloads land: the configured `download_path`, else the OS Downloads
/// folder, else the current dir.
pub fn download_dir_public(app: &AppHandle) -> PathBuf {
    let cfg = Config::load(&config_path(app));

    if let Some(path) = cfg.download_path.filter(|p| !p.trim().is_empty()) {
        return PathBuf::from(path);
    }

    app.path()
        .download_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn download_dir(app: &AppHandle) -> PathBuf {
    download_dir_public(app)
}

/// Avoids clobbering an existing file by appending " (1)", " (2)", …
fn unique_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }

    let dir = path.parent().map(PathBuf::from).unwrap_or_default();
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (1..)
        .map(|i| dir.join(format!("{stem} ({i}){ext}")))
        .find(|candidate| !candidate.exists())
        .unwrap_or(path)
}

/// Picks the save path for a download: the suggested filename (or the URL's last
/// segment) under [`download_dir`], de-duplicated.
fn download_target(app: &AppHandle, url: &Url, suggested: &Path) -> PathBuf {
    let name = suggested
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .or_else(|| {
            url.path_segments()
                .and_then(|mut s| s.next_back())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "download".to_string());

    target_for_name(app, &name)
}

/// Same de-duplicated target-picking as [`download_target`], for callers that
/// only have a suggested filename (no URL) — see [`save_download_bytes`].
fn target_for_name(app: &AppHandle, name: &str) -> PathBuf {
    let name = if name.trim().is_empty() {
        "download"
    } else {
        name
    };

    let target = unique_path(download_dir(app).join(name));
    if let Some(parent) = target.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    target
}

/// Writes bytes captured from a page-side blob download (see `web/download.js`)
/// to the configured downloads folder.
pub fn save_download_bytes(app: &AppHandle, name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let target = target_for_name(app, name);
    std::fs::write(&target, bytes)?;
    Ok(target)
}

/// Fully transparent: these windows are `.transparent(true)` for rounded
/// corners, and the React screen's own CSS already paints the themed background.
pub fn transparent_bg() -> tauri::window::Color {
    tauri::window::Color(0, 0, 0, 0)
}

/// Historical label of Account 1. Keeping it unchanged is part of the migration
/// contract: the pre-multi-account window and its WebKit profile stay intact.
pub const MAIN_LABEL: &str = "main";
const ACCOUNT_LABEL_PREFIX: &str = "account-";
const WHATSAPP_URL: &str = "https://web.whatsapp.com";

/// Geometry copied between account windows so switching sessions feels like one
/// logical native window instead of opening a different window at another size.
#[derive(Debug, Clone, Copy)]
struct SwitchGeometry {
    position: tauri::PhysicalPosition<i32>,
    size: tauri::PhysicalSize<u32>,
    maximized: bool,
}

fn capture_switch_geometry(window: &tauri::WebviewWindow) -> Option<SwitchGeometry> {
    Some(SwitchGeometry {
        position: window.outer_position().ok()?,
        size: window.inner_size().ok()?,
        maximized: window.is_maximized().unwrap_or(false),
    })
}

fn apply_switch_geometry(window: &tauri::WebviewWindow, geometry: SwitchGeometry) {
    // Move a maximized target onto the source monitor before maximizing it. We
    // intentionally do not copy the maximized physical size as its restore size;
    // for normal windows both position and inner size are copied exactly.
    let _ = window.unmaximize();
    let _ = window.set_position(geometry.position);
    if geometry.maximized {
        let _ = window.maximize();
    } else {
        let _ = window.set_size(geometry.size);
    }
}

/// Stable window label for an account. Account 1 keeps `main`; newer accounts use
/// `account-N`, which also matches the restricted remote capability glob.
pub fn account_label(id: AccountId) -> String {
    if id == PRIMARY_ACCOUNT_ID {
        MAIN_LABEL.to_string()
    } else {
        format!("{ACCOUNT_LABEL_PREFIX}{id}")
    }
}

/// Reverse mapping used by the global window-event handler to distinguish
/// WhatsApp windows from local React windows.
pub fn account_id_from_label(label: &str) -> Option<AccountId> {
    if label == MAIN_LABEL {
        return Some(PRIMARY_ACCOUNT_ID);
    }

    label
        .strip_prefix(ACCOUNT_LABEL_PREFIX)
        .and_then(|id| id.parse::<AccountId>().ok())
        .filter(|id| *id != PRIMARY_ACCOUNT_ID && *id != 0)
}

/// Dedicated persistent browser data for non-legacy accounts. Account 1 never
/// calls this path: omitting `data_directory` is how its existing session stays
/// exactly where old ZeroWhats versions left it.
fn account_storage_dir(app: &AppHandle, id: AccountId) -> PathBuf {
    app.path()
        .app_data_dir()
        .expect("app data dir resolvable")
        .join("accounts")
        .join(id.to_string())
}

/// Stable 16-byte WKWebsiteDataStore identifier for macOS 14+. The first twelve
/// bytes are a ZeroWhats namespace; the final four encode the account id.
#[cfg(any(target_os = "macos", test))]
fn account_data_store_identifier(id: AccountId) -> [u8; 16] {
    let mut value = [0u8; 16];
    value[..12].copy_from_slice(b"ZeroWhatsAcc");
    value[12..].copy_from_slice(&id.to_be_bytes());
    value
}

fn chosen_user_agent() -> &'static str {
    if std::env::var("ZW_LIGHT_UA").is_ok() {
        "Mozilla/5.0 (Linux; Android 12; Pixel 5) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/116.0.0.0 Mobile Safari/537.36"
    } else if std::env::var("ZW_CHROME_UA").is_ok() {
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36"
    } else {
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.6 Safari/605.1.15"
    }
}

/// Builds every persisted WhatsApp account. Each account is a separate native
/// window/WebView instead of stacking WebViews, preserving the input behaviour
/// the original client intentionally relies on. Only the active account reveals
/// itself on startup; the others stay loaded in the background.
pub fn build_accounts(app: &AppHandle, cfg: &Config) -> tauri::Result<()> {
    for account in &cfg.accounts.items {
        build_account(app, cfg, account)?;
    }
    Ok(())
}

/// Builds one frameless WhatsApp window with an isolated browser profile when it
/// is not the legacy Account 1.
pub fn build_account(app: &AppHandle, cfg: &Config, account: &Account) -> tauri::Result<()> {
    let label = account_label(account.id);
    if app.get_webview_window(&label).is_some() {
        return Ok(());
    }

    let start_locked = cfg.password_hash.is_some();
    let auto_lock_minutes = lock::effective_auto_lock_minutes(cfg);
    let is_active = cfg.accounts.active_id == account.id;
    let nav_app = app.clone();
    let dl_app = app.clone();
    let download_label = label.clone();

    let minimal = std::env::var("ZW_MINIMAL").is_ok();
    let force_show = std::env::var("ZW_FORCE_SHOW").is_ok();

    let mut builder = WebviewWindowBuilder::new(
        app,
        label.clone(),
        WebviewUrl::External(WHATSAPP_URL.parse().unwrap()),
    )
    .title("ZeroWhats")
    .inner_size(1100.0, 800.0)
    .decorations(false)
    .transparent(true)
    .background_color(transparent_bg())
    .visible(force_show && is_active)
    .user_agent(chosen_user_agent())
    .on_navigation(move |url| allow_navigation(&nav_app, url))
    .on_download(move |webview, event| {
        match event {
            DownloadEvent::Requested { url, destination } => {
                let target = download_target(&dl_app, &url, destination);
                *destination = target;
            }
            DownloadEvent::Finished {
                url: _,
                path,
                success,
            } => {
                let name = path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned());
                let path_str = path.as_ref().map(|p| p.to_string_lossy().into_owned());
                let _ = webview.emit_to(
                    &download_label,
                    "zw://download-result",
                    serde_json::json!({ "ok": success, "name": name, "path": path_str }),
                );
            }
            _ => {}
        }
        true
    });

    if !account.uses_legacy_storage() {
        #[cfg(target_os = "macos")]
        {
            builder = builder.data_store_identifier(account_data_store_identifier(account.id));
        }

        #[cfg(not(target_os = "macos"))]
        {
            builder = builder.data_directory(account_storage_dir(app, account.id));
        }
    }

    builder = builder.initialization_script(scripts::bootstrap(
        cfg.theme.wa_value(),
        auto_lock_minutes,
        start_locked,
        cfg.spellcheck_enabled,
        account.id,
        &account.name,
        is_active,
    ));

    builder = builder.initialization_script(scripts::ROUNDED_CORNERS);

    if !minimal {
        builder = builder.initialization_script(scripts::BACKGROUND_SYNC);
        builder = builder.initialization_script(scripts::NOTIFICATIONS);
        builder = builder.initialization_script(scripts::UNREAD_BADGE);
    }

    if minimal {
        builder = builder.initialization_script(scripts::DISABLE_MEDIA);
    }

    builder = builder
        .initialization_script(scripts::MPRIS)
        .initialization_script(scripts::DOWNLOAD)
        .initialization_script(scripts::AUTO_LOCK)
        .initialization_script(scripts::PRIVACY_BLUR)
        .initialization_script(scripts::LINKS)
        .initialization_script(scripts::FIND)
        .initialization_script(scripts::FULLSCREEN)
        .initialization_script(scripts::TITLEBAR)
        .initialization_script(scripts::RESIZE_HANDLES)
        .initialization_script(scripts::CLIPBOARD_IMAGE)
        .initialization_script(scripts::SPELLCHECK)
        .initialization_script(scripts::UPDATE_BANNER);

    let _ = builder.build()?;
    log::info!(
        "built account window (id={} label={} active={} legacy_storage={} minimal={})",
        account.id,
        label,
        is_active,
        account.uses_legacy_storage(),
        minimal
    );
    Ok(())
}

/// Navigation allow-list (security): only WhatsApp may load inside the app
/// window. Any other http(s) destination is opened in the user's real browser.
fn allow_navigation(app: &AppHandle, url: &Url) -> bool {
    if is_whatsapp_url(url) {
        return true;
    }

    if matches!(url.scheme(), "http" | "https") {
        commands::open_external(app, url.as_str());
    }

    false
}

fn is_whatsapp_url(url: &Url) -> bool {
    if matches!(url.scheme(), "about" | "blob" | "data") {
        return true;
    }

    matches!(url.host_str(), Some(host)
        if host == "web.whatsapp.com"
            || host.ends_with(".whatsapp.com")
            || host.ends_with(".whatsapp.net"))
}

/// Runs `f` for every currently-created WhatsApp account window.
fn for_each_account_window(app: &AppHandle, mut f: impl FnMut(AccountId, tauri::WebviewWindow)) {
    let cfg = Config::load(&config_path(app));
    for account in &cfg.accounts.items {
        let label = account_label(account.id);
        if let Some(window) = app.get_webview_window(&label) {
            f(account.id, window);
        }
    }
}

/// Updates account names/active state in every live WhatsApp page and publishes
/// the authoritative collection for the in-page account switcher. The remote
/// page never writes this state; it only renders the Rust-owned metadata.
pub fn sync_account_metadata(app: &AppHandle, accounts: &Accounts) {
    for account in &accounts.items {
        let label = account_label(account.id);
        let Some(window) = app.get_webview_window(&label) else {
            continue;
        };

        let name =
            serde_json::to_string(&account.name).expect("account name is always JSON serializable");
        let active = accounts.active_id == account.id;
        let _ = window.eval(format!(
            "if (window.__ZW) {{ window.__ZW.accountName = {name}; window.__ZW.isActiveAccount = {active}; }}"
        ));
        let _ = window.emit_to(&label, "zw://accounts-state", accounts.clone());
    }
}

pub fn sync_has_password(app: &AppHandle, has_password: bool) {
    for_each_account_window(app, |_id, window| {
        let _ = window.eval(format!(
            "if (window.__ZW) window.__ZW.hasPassword = {has_password};"
        ));
    });
}

/// Blurs or clears one account page when that account window changes focus.
pub fn apply_unfocus_blur_for_label(app: &AppHandle, label: &str, focused: bool) {
    let cfg = Config::load(&config_path(app));
    let blur = !focused && cfg.hide_content_on_unfocus;

    log::debug!("apply_unfocus_blur: label={label} focused={focused} blur={blur}");

    if let Some(window) = app.get_webview_window(label) {
        let _ = window.eval(format!(
            "if (window.__ZW && window.__ZW.setBlur) window.__ZW.setBlur({blur});"
        ));
    }
}

/// Pushes the WhatsApp theme into every account page and reloads each so it takes
/// effect consistently across active and background sessions.
pub fn apply_theme(app: &AppHandle, theme: Theme) {
    let wa = theme.wa_value();
    for_each_account_window(app, |_id, window| {
        let _ = window.eval(format!(
            "(function(){{ try {{ localStorage.setItem('theme', '\"{wa}\"'); location.reload(); }} catch (e) {{}} }})();"
        ));
    });
}

/// Applies spell-check settings to every account WebView.
pub fn apply_spellcheck(app: &AppHandle, enabled: bool, languages: Vec<String>) {
    #[cfg(target_os = "linux")]
    {
        use webkit2gtk::{WebContextExt, WebViewExt};

        for_each_account_window(app, |_id, window| {
            let languages = languages.clone();
            let _ = window.with_webview(move |webview| {
                let wv = webview.inner();
                if let Some(ctx) = wv.context() {
                    ctx.set_spell_checking_enabled(enabled);
                    if enabled && !languages.is_empty() {
                        let langs: Vec<&str> = languages.iter().map(String::as_str).collect();
                        ctx.set_spell_checking_languages(&langs);
                    }
                }
            });
        });
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = (app, enabled, languages);
    }
}

/// Hides every WhatsApp account window. Used by the global app lock and while
/// switching accounts so only one account is presented at a time.
pub fn hide_all_accounts(app: &AppHandle) {
    for_each_account_window(app, |_id, window| {
        let _ = window.hide();
    });
}

/// Reveals one persisted account, creating its WebView lazily if necessary.
/// Returns false for an unknown account or if building the WebView fails.
pub fn show_account(app: &AppHandle, id: AccountId) -> bool {
    if lock::is_locked() {
        lock::show_lock_window(app);
        return false;
    }

    let path = config_path(app);
    let mut cfg = Config::load(&path);
    let Some(account) = cfg.accounts.get(id).cloned() else {
        log::warn!("show_account: unknown account id {id}");
        return false;
    };

    let previous_active = cfg.accounts.active_id;
    let switch_geometry = if previous_active != id {
        let previous_label = account_label(previous_active);
        app.get_webview_window(&previous_label)
            .and_then(|window| capture_switch_geometry(&window))
    } else {
        None
    };

    if cfg.accounts.set_active(id).is_err() {
        return false;
    }
    if let Err(e) = cfg.save(&path) {
        log::warn!("show_account: failed to persist active account {id}: {e}");
    }

    let label = account_label(id);
    if app.get_webview_window(&label).is_none() {
        if let Err(e) = build_account(app, &cfg, &account) {
            log::error!("show_account: failed to build account {id}: {e}");
            return false;
        }
    }

    sync_account_metadata(app, &cfg.accounts);
    hide_all_accounts(app);
    if let Some(window) = app.get_webview_window(&label) {
        if let Some(geometry) = switch_geometry {
            apply_switch_geometry(&window, geometry);
        }
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        true
    } else {
        false
    }
}

/// Reveals the persisted active account (or the lock screen if locked).
pub fn show_main(app: &AppHandle) {
    if lock::is_locked() {
        lock::show_lock_window(app);
        return;
    }

    let cfg = Config::load(&config_path(app));
    let id = cfg.accounts.active_id;
    log::info!("show_main: active account id={id}");
    if !show_account(app, id) {
        log::warn!("show_main: active account {id} could not be shown");
    }
}

/// Opens (or focuses) a frameless React window that renders the screen matching
/// its label.
fn open_react_window(app: &AppHandle, label: &str, title: &str, size: (f64, f64), resizable: bool) {
    if lock::is_locked() {
        lock::show_lock_window(app);
        return;
    }

    if let Some(win) = app.get_webview_window(label) {
        let _ = win.show();
        let _ = win.set_focus();
        return;
    }

    log::info!(
        "open_react_window: creating window label={} title={}",
        label,
        title
    );

    let result = WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
        .title(title)
        .inner_size(size.0, size.1)
        .resizable(resizable)
        .maximizable(false)
        .center()
        .decorations(false)
        .transparent(true)
        .visible(false)
        .background_color(transparent_bg())
        .build();
    if let Err(e) = result {
        log::error!("failed to open '{label}' window: {e}");
    }
}

pub fn open_settings(app: &AppHandle) {
    open_react_window(
        app,
        "settings",
        "ZeroWhats — Settings",
        (640.0, 680.0),
        true,
    );
}

pub fn open_about(app: &AppHandle) {
    open_react_window(app, "about", "About ZeroWhats", (400.0, 600.0), false);
}

pub fn open_update(app: &AppHandle) {
    open_react_window(app, "update", "ZeroWhats — Update", (480.0, 520.0), false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_account_uses_main_label() {
        assert_eq!(account_label(PRIMARY_ACCOUNT_ID), MAIN_LABEL);
        assert_eq!(account_id_from_label(MAIN_LABEL), Some(PRIMARY_ACCOUNT_ID));
    }

    #[test]
    fn additional_accounts_use_prefixed_labels() {
        assert_eq!(account_label(2), "account-2");
        assert_eq!(account_id_from_label("account-2"), Some(2));
        assert_eq!(account_id_from_label("account-42"), Some(42));
    }

    #[test]
    fn invalid_account_labels_are_rejected() {
        for label in [
            "account-0",
            "account-1",
            "account-",
            "account-x",
            "settings",
        ] {
            assert_eq!(account_id_from_label(label), None);
        }
    }

    #[test]
    fn data_store_ids_are_stable_and_distinct() {
        assert_eq!(
            account_data_store_identifier(2),
            account_data_store_identifier(2)
        );
        assert_ne!(
            account_data_store_identifier(2),
            account_data_store_identifier(3)
        );
    }

    #[test]
    fn whatsapp_main_url() {
        let url: Url = "https://web.whatsapp.com".parse().unwrap();
        assert!(is_whatsapp_url(&url));
    }

    #[test]
    fn whatsapp_subdomain() {
        let url: Url = "https://static.whatsapp.com/something".parse().unwrap();
        assert!(is_whatsapp_url(&url));
    }

    #[test]
    fn whatsapp_net_media() {
        let url: Url = "https://mmg.whatsapp.net/media/123".parse().unwrap();
        assert!(is_whatsapp_url(&url));
    }

    #[test]
    fn whatsapp_net_subdomain() {
        let url: Url = "https://pps.whatsapp.net/v/t61/photo.jpg".parse().unwrap();
        assert!(is_whatsapp_url(&url));
    }

    #[test]
    fn blob_url_allowed() {
        let url: Url = "blob:https://web.whatsapp.com/12345".parse().unwrap();
        assert!(is_whatsapp_url(&url));
    }

    #[test]
    fn data_url_allowed() {
        let url: Url = "data:text/html,<h1>hi</h1>".parse().unwrap();
        assert!(is_whatsapp_url(&url));
    }

    #[test]
    fn about_blank_allowed() {
        let url: Url = "about:blank".parse().unwrap();
        assert!(is_whatsapp_url(&url));
    }

    #[test]
    fn external_url_rejected() {
        let url: Url = "https://google.com".parse().unwrap();
        assert!(!is_whatsapp_url(&url));
    }

    #[test]
    fn similar_domain_rejected() {
        let url: Url = "https://notwhatsapp.com".parse().unwrap();
        assert!(!is_whatsapp_url(&url));
    }

    #[test]
    fn whatsapp_in_path_rejected() {
        let url: Url = "https://evil.com/whatsapp.com".parse().unwrap();
        assert!(!is_whatsapp_url(&url));
    }

    #[test]
    fn unique_path_no_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new_file.txt");
        assert_eq!(unique_path(path.clone()), path);
    }

    #[test]
    fn unique_path_with_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.txt");
        std::fs::write(&path, "existing").unwrap();

        let result = unique_path(path.clone());
        assert_eq!(result, dir.path().join("file (1).txt"));
    }

    #[test]
    fn unique_path_multiple_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.txt");
        std::fs::write(&path, "").unwrap();
        std::fs::write(dir.path().join("file (1).txt"), "").unwrap();
        std::fs::write(dir.path().join("file (2).txt"), "").unwrap();

        let result = unique_path(path);
        assert_eq!(result, dir.path().join("file (3).txt"));
    }

    #[test]
    fn unique_path_no_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("README");
        std::fs::write(&path, "").unwrap();

        let result = unique_path(path);
        assert_eq!(result, dir.path().join("README (1)"));
    }
}

pub fn open_shortcuts(app: &AppHandle) {
    open_react_window(
        app,
        "shortcuts",
        "Keyboard Shortcuts",
        (400.0, 360.0),
        false,
    );
}
