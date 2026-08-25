//! JavaScript injected into the WhatsApp Web page.
//!
//! Each script lives in its own `web/*.js` file (compiled in with `include_str!`)
//! so the page logic stays real, lintable JavaScript instead of giant Rust
//! string literals. They are registered as initialization scripts on the WhatsApp
//! account windows in `window.rs`, in the order below, after [`bootstrap`].

pub const ROUNDED_CORNERS: &str = include_str!("web/rounded-corners.js");
pub const BACKGROUND_SYNC: &str = include_str!("web/background-sync.js");
pub const NOTIFICATIONS: &str = include_str!("web/notifications.js");
pub const MPRIS: &str = include_str!("web/mpris.js");
pub const UNREAD_BADGE: &str = include_str!("web/unread-badge.js");
pub const DOWNLOAD: &str = include_str!("web/download.js");
pub const AUTO_LOCK: &str = include_str!("web/auto-lock.js");
pub const PRIVACY_BLUR: &str = include_str!("web/privacy-blur.js");
pub const TITLEBAR: &str = include_str!("web/titlebar.js");
pub const RESIZE_HANDLES: &str = include_str!("web/resize-handles.js");
pub const CLIPBOARD_IMAGE: &str = include_str!("web/clipboard-image.js");
pub const LINKS: &str = include_str!("web/links.js");
pub const FIND: &str = include_str!("web/find.js");
pub const FULLSCREEN: &str = include_str!("web/fullscreen.js");
pub const WIPE_SESSION: &str = include_str!("web/wipe-session.js");
pub const DISABLE_MEDIA: &str = include_str!("web/disable-media.js");

/// The first script to run: seeds `window.__ZW` with global settings plus the
/// immutable account identity the other scripts attach to their events.
pub fn bootstrap(
    wa_theme: &str,
    auto_lock_minutes: u32,
    has_password: bool,
    spellcheck: bool,
    account_id: u32,
    account_name: &str,
) -> String {
    let account_name =
        serde_json::to_string(account_name).expect("account name is always JSON serializable");

    include_str!("web/bootstrap.js")
        .replace("\"__ZW_THEME__\"", &format!("{wa_theme:?}"))
        .replace(
            "\"__ZW_AUTO_LOCK_MINUTES__\"",
            &auto_lock_minutes.to_string(),
        )
        .replace("\"__ZW_HAS_PASSWORD__\"", &has_password.to_string())
        .replace("\"__ZW_SPELLCHECK__\"", &spellcheck.to_string())
        .replace("\"__ZW_ACCOUNT_ID__\"", &account_id.to_string())
        .replace("\"__ZW_ACCOUNT_NAME__\"", &account_name)
}

pub const SPELLCHECK: &str = include_str!("web/spellcheck.js");
pub const UPDATE_BANNER: &str = include_str!("web/update-banner.js");

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_bootstrap() -> String {
        bootstrap("dark", 15, true, true, 2, "Work")
    }

    #[test]
    fn bootstrap_substitutes_theme() {
        let script = sample_bootstrap();
        assert!(script.contains("\"dark\""));
        assert!(!script.contains("__ZW_THEME__"));
    }

    #[test]
    fn bootstrap_substitutes_auto_lock_minutes() {
        let script = sample_bootstrap();
        assert!(script.contains("15"));
        assert!(!script.contains("__ZW_AUTO_LOCK_MINUTES__"));
    }

    #[test]
    fn bootstrap_substitutes_has_password() {
        let script = sample_bootstrap();
        assert!(script.contains("true"));
        assert!(!script.contains("__ZW_HAS_PASSWORD__"));
    }

    #[test]
    fn bootstrap_substitutes_spellcheck() {
        let script = bootstrap("system", 0, false, false, 1, "Personal");
        assert!(!script.contains("__ZW_SPELLCHECK__"));
    }

    #[test]
    fn bootstrap_substitutes_account_identity() {
        let script = bootstrap("system", 0, false, true, 7, "Work \"QA\"");
        assert!(script.contains("accountId: 7"));
        assert!(script.contains("accountName: \"Work \\\"QA\\\"\""));
        assert!(!script.contains("__ZW_ACCOUNT_"));
    }

    #[test]
    fn bootstrap_no_placeholders_remain() {
        let script = sample_bootstrap();
        assert!(!script.contains("__ZW_"));
    }

    #[test]
    fn all_script_constants_non_empty() {
        assert!(!ROUNDED_CORNERS.is_empty());
        assert!(!BACKGROUND_SYNC.is_empty());
        assert!(!NOTIFICATIONS.is_empty());
        assert!(!MPRIS.is_empty());
        assert!(!UNREAD_BADGE.is_empty());
        assert!(!DOWNLOAD.is_empty());
        assert!(!AUTO_LOCK.is_empty());
        assert!(!PRIVACY_BLUR.is_empty());
        assert!(!TITLEBAR.is_empty());
        assert!(!RESIZE_HANDLES.is_empty());
        assert!(!CLIPBOARD_IMAGE.is_empty());
        assert!(!LINKS.is_empty());
        assert!(!FIND.is_empty());
        assert!(!FULLSCREEN.is_empty());
        assert!(!WIPE_SESSION.is_empty());
        assert!(!DISABLE_MEDIA.is_empty());
        assert!(!SPELLCHECK.is_empty());
        assert!(!UPDATE_BANNER.is_empty());
    }
}
