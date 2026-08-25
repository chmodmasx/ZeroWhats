//! Per-account unread state and aggregation for the single system-tray badge.
//!
//! Each WhatsApp WebView reports only its own count. The tray remains an app-wide
//! surface, so this module owns the small amount of shared state needed to sum
//! those counts without teaching `tray.rs` about account lifecycles.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use tauri::AppHandle;

use crate::accounts::AccountId;
use crate::tray;

static COUNTS: OnceLock<Mutex<HashMap<AccountId, u32>>> = OnceLock::new();

fn counts() -> &'static Mutex<HashMap<AccountId, u32>> {
    COUNTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Updates one account and redraws the tray with the saturated app-wide total.
/// Zero removes the entry so deleted/quiet accounts do not accumulate state.
pub fn set(app: &AppHandle, account_id: AccountId, count: u32) {
    let total = match counts().lock() {
        Ok(mut counts) => {
            if count == 0 {
                counts.remove(&account_id);
            } else {
                counts.insert(account_id, count);
            }
            total(&counts)
        }
        Err(e) => {
            log::warn!("unread state lock poisoned: {e}");
            return;
        }
    };

    tray::set_unread(app, total);
}

/// Drops a removed account's unread contribution and refreshes the tray.
pub fn remove(app: &AppHandle, account_id: AccountId) {
    let total = match counts().lock() {
        Ok(mut counts) => {
            counts.remove(&account_id);
            total(&counts)
        }
        Err(e) => {
            log::warn!("unread state lock poisoned: {e}");
            return;
        }
    };

    tray::set_unread(app, total);
}

fn total(counts: &HashMap<AccountId, u32>) -> u32 {
    counts
        .values()
        .copied()
        .fold(0u32, |sum, count| sum.saturating_add(count))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn total_sums_accounts() {
        let counts = HashMap::from([(1, 2), (2, 5), (3, 1)]);
        assert_eq!(total(&counts), 8);
    }

    #[test]
    fn total_empty_is_zero() {
        assert_eq!(total(&HashMap::new()), 0);
    }

    #[test]
    fn total_saturates_instead_of_wrapping() {
        let counts = HashMap::from([(1, u32::MAX), (2, 1)]);
        assert_eq!(total(&counts), u32::MAX);
    }
}
