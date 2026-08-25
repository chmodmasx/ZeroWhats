//! The thin IPC layer: every `#[tauri::command]` invokable from the local React
//! windows (Settings / About / Shortcuts / Lock / Accounts). Each delegates to a
//! domain module (config / accounts / lock / window / password). App commands can
//! NOT be invoked from the remote WhatsApp page, so that page talks to the
//! backend via events instead (see `register_web_events` in main.rs).

use tauri::Manager;

use crate::accounts::{AccountError, AccountId, Accounts};
use crate::config::{config_path, Config, ConfigPatch, ConfigView, Theme};
use crate::{lock, password, scripts, window};

#[tauri::command]
pub fn get_config(app: tauri::AppHandle) -> ConfigView {
    Config::load(&config_path(&app)).into()
}

#[tauri::command]
pub fn save_config(app: tauri::AppHandle, patch: ConfigPatch) {
    let path = config_path(&app);
    let mut cfg = Config::load(&path);

    patch.apply_to(&mut cfg);

    let _ = cfg.save(&path);
    apply_autostart(&app, cfg.auto_start);
    lock::apply_auto_lock(&app);
    window::apply_spellcheck(
        &app,
        cfg.spellcheck_enabled,
        cfg.spellcheck_languages.clone(),
    );
}

/// Returns account metadata to the local management UI. `next_id` is included
/// because [`Accounts`] is the persisted domain type, but the frontend treats it
/// as opaque and never writes account state back through `save_config`.
#[tauri::command]
pub fn get_accounts(app: tauri::AppHandle) -> Accounts {
    Config::load(&config_path(&app)).accounts
}

/// Adds a new isolated WhatsApp session and makes it active. Config is persisted
/// before WebView creation so the remote page's early readiness event validates;
/// if window creation fails, metadata is rolled back to the previous account.
#[tauri::command]
pub fn add_account(app: tauri::AppHandle, name: String) -> Result<Accounts, String> {
    let path = config_path(&app);
    let mut cfg = Config::load(&path);
    let previous_active = cfg.accounts.active_id;
    let account = cfg.accounts.add(name).map_err(account_error)?;

    cfg.save(&path)
        .map_err(|e| format!("failed to save account: {e}"))?;

    window::hide_all_accounts(&app);
    if let Err(e) = window::build_account(&app, &cfg, &account) {
        // Restore metadata first; the previous account can then safely become
        // visible again without leaving a broken active id in config.
        let _ = cfg.accounts.remove(account.id);
        let _ = cfg.accounts.set_active(previous_active);
        let _ = cfg.save(&path);
        window::show_account(&app, previous_active);
        return Err(format!("failed to create account window: {e}"));
    }

    Ok(cfg.accounts)
}

/// Switches presentation to an already-loaded account. Background WebViews stay
/// alive for notifications/unread updates; only one account window is visible.
#[tauri::command]
pub fn switch_account(app: tauri::AppHandle, account_id: AccountId) -> Result<Accounts, String> {
    if !window::show_account(&app, account_id) {
        return Err("account not found or could not be shown".to_string());
    }
    Ok(Config::load(&config_path(&app)).accounts)
}

/// Renames account metadata without touching its WhatsApp profile. The live page
/// receives the new display name immediately; the persisted value is authoritative
/// and will be injected again the next time the WebView is constructed.
#[tauri::command]
pub fn rename_account(
    app: tauri::AppHandle,
    account_id: AccountId,
    name: String,
) -> Result<Accounts, String> {
    let path = config_path(&app);
    let mut cfg = Config::load(&path);
    cfg.accounts.rename(account_id, name).map_err(account_error)?;
    cfg.save(&path)
        .map_err(|e| format!("failed to save account: {e}"))?;

    if let Some(account) = cfg.accounts.get(account_id) {
        let label = window::account_label(account_id);
        if let Some(webview) = app.get_webview_window(&label) {
            let name = serde_json::to_string(&account.name)
                .expect("account name is always JSON serializable");
            let _ = webview.eval(format!(
                "if (window.__ZW) window.__ZW.accountName = {name};"
            ));
        }
    }

    Ok(cfg.accounts)
}

fn account_error(error: AccountError) -> String {
    match error {
        AccountError::NotFound => "account not found".to_string(),
        AccountError::LastAccount => "the last account cannot be removed".to_string(),
        AccountError::IdExhausted => "no account ids remain available".to_string(),
    }
}

/// Sets (or replaces) the app-lock password. Replacing an existing password
/// requires proving ownership — `current` must verify against the stored hash,
/// otherwise the change is refused. Setting a password for the first time (no
/// stored hash) needs no proof. Returns whether the password was changed.
#[tauri::command]
pub fn set_password(app: tauri::AppHandle, plain: String, current: Option<String>) -> bool {
    let path = config_path(&app);
    let mut cfg = Config::load(&path);

    if plain.is_empty() {
        return false;
    }

    if let Some(existing) = &cfg.password_hash {
        let ok = current
            .as_deref()
            .is_some_and(|c| password::verify(c, existing));
        if !ok {
            return false;
        }
    }

    cfg.password_hash = password::hash(&plain).ok();

    let _ = cfg.save(&path);
    lock::apply_auto_lock(&app);

    crate::tray::refresh(&app);
    window::sync_has_password(&app, cfg.password_hash.is_some());
    true
}

/// Removes the app-lock password. Requires either the current password
/// (`current` verifies against the stored hash) or a successful system-admin
/// authentication (polkit on Linux, admin/sudo elsewhere via `reset_with_admin`).
/// Returns whether the password was removed.
#[tauri::command]
pub fn remove_password(app: tauri::AppHandle, current: Option<String>) -> bool {
    let path = config_path(&app);
    let mut cfg = Config::load(&path);

    let Some(existing) = &cfg.password_hash else {
        return true;
    };

    let by_password = current
        .as_deref()
        .is_some_and(|c| password::verify(c, existing));
    let authorized = by_password || password::reset_with_admin();
    if !authorized {
        return false;
    }

    cfg.password_hash = None;
    let _ = cfg.save(&path);
    lock::apply_auto_lock(&app);

    crate::tray::refresh(&app);
    window::sync_has_password(&app, false);
    true
}

#[tauri::command]
pub fn reset_password(app: tauri::AppHandle) -> bool {
    if password::reset_with_admin() {
        let path = config_path(&app);
        let mut cfg = Config::load(&path);

        cfg.password_hash = None;
        let _ = cfg.save(&path);

        true
    } else {
        false
    }
}

/// Non-Linux "forgot password" recovery. There's no cross-platform system-auth
/// reset (polkit is Linux-only), so remove the lock by wiping every linked
/// WhatsApp session plus the config. Each already-created account WebView clears
/// its own isolated storage through the same page-side wipe script.
#[tauri::command]
pub fn forget_password_wipe(app: tauri::AppHandle) {
    let cfg = Config::load(&config_path(&app));

    for account in &cfg.accounts.items {
        let label = window::account_label(account.id);
        if let Some(webview) = app.get_webview_window(&label) {
            let _ = webview.eval(scripts::WIPE_SESSION);
        }
    }

    let _ = std::fs::remove_file(config_path(&app));

    // The config (and its password) is gone, so an empty unlock now succeeds and
    // reveals legacy Account 1. The tray needs an explicit refresh because its
    // conditional Lock item is built from config rather than page state.
    crate::tray::refresh(&app);
    lock::unlock(&app, "");
}

/// The OS we're running on, so the UI can branch on Linux-only affordances such
/// as the polkit-based "forgot password" reset.
#[tauri::command]
pub fn get_platform() -> String {
    std::env::consts::OS.to_string()
}

#[tauri::command]
pub fn set_theme(app: tauri::AppHandle, theme: Theme) {
    window::apply_theme(&app, theme);
}

#[tauri::command]
pub fn unlock(app: tauri::AppHandle, password: String) -> bool {
    lock::unlock(&app, &password)
}

/// Opens a URL (or `mailto:`) in the user's default handler. Only http(s) and
/// mailto schemes are allowed to prevent local file or protocol handler abuse.
#[tauri::command]
pub fn open_url(app: tauri::AppHandle, url: String) {
    if let Ok(parsed) = tauri::Url::parse(&url) {
        if matches!(parsed.scheme(), "http" | "https" | "mailto") {
            open_external(&app, &url);
        } else {
            log::warn!("open_url: blocked scheme '{}'", parsed.scheme());
        }
    }
}

/// Cross-platform "open in the user's default app" via the opener plugin. Used by
/// `open_url` and the window navigation guard.
pub fn open_external(app: &tauri::AppHandle, url: &str) {
    use tauri_plugin_opener::OpenerExt;
    let _ = app.opener().open_url(url, None::<&str>);
}

/// Enables or disables launch-at-login to match the config. Used by `save_config`
/// and at startup.
pub fn apply_autostart(app: &tauri::AppHandle, enabled: bool) {
    use tauri_plugin_autostart::ManagerExt;
    let autolaunch = app.autolaunch();

    let _ = if enabled {
        autolaunch.enable()
    } else {
        autolaunch.disable()
    };
}
