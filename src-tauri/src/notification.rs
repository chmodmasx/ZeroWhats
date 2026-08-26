//! Native OS notifications on behalf of WhatsApp account pages, plus the mute
//! toggle shared by the whole app.

use tauri::AppHandle;
#[cfg(not(target_os = "linux"))]
use tauri::Manager;
#[cfg(not(target_os = "linux"))]
use tauri_plugin_notification::NotificationExt;

use crate::accounts::AccountId;
use crate::config::{config_path, Config, NotificationPrivacy};

/// Shows a native OS notification for one WhatsApp account. The account id is
/// validated against persisted config before anything is surfaced, and Linux
/// notification clicks return to the account that originated the message.
pub fn notify(
    app: &AppHandle,
    account_id: AccountId,
    title: Option<String>,
    body: Option<String>,
    icon: Option<String>,
) {
    let cfg = Config::load(&config_path(app));
    if cfg.accounts.get(account_id).is_none() {
        log::warn!("notification from unknown account id {account_id} ignored");
        return;
    }

    // Apply the privacy level; `None` means suppress the notification entirely.
    let privacy = cfg.notification_privacy;
    let Some((title, body)) = privacy.apply(title, body) else {
        return;
    };

    let title = title.unwrap_or_else(|| "WhatsApp".to_string());
    let body = body.unwrap_or_default();

    // The sender avatar identifies who wrote — only surface it when previews are
    // allowed (`Full`); `Generic` deliberately hides the sender's identity.
    // Account-scoped filenames prevent concurrent accounts from overwriting each
    // other's image before the notification daemon reads it.
    let avatar = if privacy.shows_preview() {
        icon.as_deref()
            .and_then(|data_url| save_avatar(data_url, account_id))
    } else {
        None
    };

    #[cfg(target_os = "linux")]
    {
        show_clickable_linux(app, account_id, title, body, avatar);
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = &avatar; // silence unused on the plugin path below
        let mut builder = app.notification().builder().title(title).body(body);
        if let Some(icon) = avatar.or_else(|| notification_icon(app)) {
            builder = builder.icon(icon);
        }
        let _ = builder.show();
    }
}

/// Decodes a `data:image/...;base64,...` avatar URL into a file and returns its
/// path — notification daemons take an icon by path/name, not by a data URL.
///
/// The file lives under `$XDG_RUNTIME_DIR/zerowhats` (or the temp dir). One file
/// per account avoids cross-account races while still overwriting old avatars
/// instead of accumulating them indefinitely.
fn save_avatar(data_url: &str, account_id: AccountId) -> Option<String> {
    use base64::Engine;
    use std::io::Write;

    let rest = data_url.strip_prefix("data:")?;
    let (mime, b64) = rest.split_once(";base64,")?;
    let ext = match mime {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        _ => "png",
    };

    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;

    let dir = avatar_dir()?;
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("zerowhats-notify-avatar-{account_id}.{ext}"));

    let mut f = std::fs::File::create(&path).ok()?;
    f.write_all(&bytes).ok()?;

    path.to_str().map(str::to_string)
}

/// Directory for notification avatar files. Prefers
/// `$XDG_RUNTIME_DIR/zerowhats`; falls back to the temp dir when unset.
fn avatar_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(|r| std::path::PathBuf::from(r).join("zerowhats"))
        .or_else(|| Some(std::env::temp_dir()))
}

/// Linux: driven through `notify-rust` directly (not the notification plugin)
/// so that clicking the notification focuses the originating account. The plugin
/// calls `notify_rust::Notification::show()` and throws away the returned handle,
/// but action/close callbacks are delivered through that handle, so we keep it
/// alive on a detached thread and dispatch the default click back to Rust/Tauri.
#[cfg(target_os = "linux")]
fn show_clickable_linux(
    app: &AppHandle,
    account_id: AccountId,
    title: String,
    body: String,
    avatar: Option<String>,
) {
    let app = app.clone();

    // `wait_for_action` blocks until the notification is actioned or dismissed,
    // so it must run off the main thread.
    std::thread::spawn(move || {
        let mut notification = notify_rust::Notification::new();
        notification
            .summary(&title)
            .body(&body)
            // The app icon (resolved by name against the XDG icon theme). Stays
            // as the notification's app icon; the sender avatar is layered on
            // top via `image-data` below.
            .icon("zerowhats")
            // GNOME Shell only routes a notification's `default` click back to
            // the app when it can tie the notification to a desktop entry.
            .hint(notify_rust::Hint::DesktopEntry("ZeroWhats".to_string()))
            // The "default" action has no button; it fires when the user clicks
            // the notification popup itself.
            .action("default", "Open");

        // Sender avatar. GNOME Shell ignores a per-notification icon *path*, so
        // attach the decoded image as inline `image-data`. With no avatar, use
        // the embedded app icon so dev/unpackaged runs still show an image.
        let icon_image = match avatar.as_deref() {
            Some(path) => match notify_rust::Image::open(path) {
                Ok(image) => Some(image),
                Err(e) => {
                    log::warn!("failed to load notification avatar '{path}': {e}");
                    app_icon_image()
                }
            },
            None => app_icon_image(),
        };
        if let Some(image) = icon_image {
            notification.hint(notify_rust::Hint::ImageData(image));
        }

        match notification.show() {
            Ok(handle) => handle.wait_for_action(|action| {
                if action == "default" {
                    let app = app.clone();
                    let _ = app.clone().run_on_main_thread(move || {
                        crate::window::show_account(&app, account_id);
                    });
                }
            }),
            Err(e) => log::warn!("failed to show notification: {e}"),
        }
    });
}

/// The app icon as a `notify_rust::Image`, attached to notifications that have
/// no sender avatar. Built from the icon PNG embedded in the binary so it works
/// regardless of install layout (dev, AppImage, or packaged).
#[cfg(target_os = "linux")]
fn app_icon_image() -> Option<notify_rust::Image> {
    const ICON_PNG: &[u8] = include_bytes!("../icons/128x128.png");

    let img = tauri::image::Image::from_bytes(ICON_PNG).ok()?;
    notify_rust::Image::from_rgba(img.width() as i32, img.height() as i32, img.rgba().to_vec()).ok()
}

/// The icon to attach to a notification on non-Linux platforms. Linux drives
/// notifications through `notify-rust` directly and uses [`app_icon_image`].
#[cfg(not(target_os = "linux"))]
fn notification_icon(app: &AppHandle) -> Option<String> {
    app.path()
        .resource_dir()
        .ok()
        .map(|d| d.join("icons/128x128.png"))
        .filter(|p| p.exists())
        .and_then(|p| p.to_str().map(str::to_string))
}

/// Tray "Mute" toggle: flips between fully suppressing notifications
/// (`Hidden`) and showing them in full (`Full`). The `Generic` middle level is
/// only reachable from the Settings dropdown. Returns whether notifications are
/// now muted so the tray action can reflect it.
pub fn toggle_muted(app: &AppHandle) -> bool {
    let path = config_path(app);
    let mut cfg = Config::load(&path);

    cfg.notification_privacy = if cfg.notification_privacy.is_hidden() {
        NotificationPrivacy::Full
    } else {
        NotificationPrivacy::Hidden
    };

    let _ = cfg.save(&path);
    cfg.notification_privacy.is_hidden()
}
