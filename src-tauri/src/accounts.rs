//! Persisted account identities and invariants for multi-account WhatsApp.
//!
//! Account 1 is special: it represents the pre-multi-account WhatsApp session
//! and therefore keeps using ZeroWhats' historical WebKit storage location.
//! Additional accounts will receive their own storage directory when
//! account-aware webviews are introduced. Keeping that distinction in the model
//! lets migration add metadata without moving, copying, or invalidating the
//! existing WhatsApp session.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub type AccountId = u32;

/// Stable id reserved for the session that existed before multi-account support.
pub const PRIMARY_ACCOUNT_ID: AccountId = 1;
const FIRST_DYNAMIC_ACCOUNT_ID: AccountId = PRIMARY_ACCOUNT_ID + 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct Account {
    pub id: AccountId,
    pub name: String,
}

impl Account {
    fn new(id: AccountId, name: impl Into<String>) -> Self {
        let mut account = Account {
            id,
            name: name.into(),
        };
        account.normalize_name();
        account
    }

    /// Account 1 deliberately keeps the legacy WebKit profile. This is the
    /// migration guarantee that preserves an already-linked WhatsApp session.
    pub fn uses_legacy_storage(&self) -> bool {
        self.id == PRIMARY_ACCOUNT_ID
    }

    fn normalize_name(&mut self) {
        self.name = self.name.trim().to_string();
        if self.name.is_empty() {
            self.name = default_name(self.id);
        }
    }
}

/// Persisted account collection. IDs are stable and monotonically allocated;
/// deleting an account later must never rewind `next_id`, so a removed profile
/// id cannot silently become a different WhatsApp session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(default)]
pub struct Accounts {
    pub items: Vec<Account>,
    pub active_id: AccountId,
    pub next_id: AccountId,
}

impl Default for Accounts {
    fn default() -> Self {
        Accounts {
            items: vec![Account::new(PRIMARY_ACCOUNT_ID, default_name(PRIMARY_ACCOUNT_ID))],
            active_id: PRIMARY_ACCOUNT_ID,
            next_id: FIRST_DYNAMIC_ACCOUNT_ID,
        }
    }
}

impl Accounts {
    pub fn active(&self) -> &Account {
        self.items
            .iter()
            .find(|account| account.id == self.active_id)
            .unwrap_or_else(|| self.items.first().expect("accounts are normalized"))
    }

    /// Repairs user-edited / partially migrated config without ever touching
    /// WebKit profile data. The first valid occurrence of an id wins; invalid
    /// zero/duplicate ids are discarded, an empty list becomes legacy Account 1,
    /// and `next_id` is advanced past every id that is still present.
    pub fn normalize(&mut self) {
        let mut seen = HashSet::new();
        self.items
            .retain(|account| account.id != 0 && seen.insert(account.id));

        for account in &mut self.items {
            account.normalize_name();
        }

        if self.items.is_empty() {
            *self = Accounts::default();
            return;
        }

        if !self.items.iter().any(|account| account.id == self.active_id) {
            self.active_id = self.items[0].id;
        }

        let min_next = self
            .items
            .iter()
            .map(|account| account.id)
            .max()
            .and_then(|id| id.checked_add(1))
            .unwrap_or(AccountId::MAX);
        self.next_id = self.next_id.max(min_next).max(FIRST_DYNAMIC_ACCOUNT_ID);
    }
}

fn default_name(id: AccountId) -> String {
    format!("Account {id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_legacy_account_one() {
        let accounts = Accounts::default();
        assert_eq!(accounts.items.len(), 1);
        assert_eq!(accounts.items[0].id, PRIMARY_ACCOUNT_ID);
        assert_eq!(accounts.items[0].name, "Account 1");
        assert_eq!(accounts.active_id, PRIMARY_ACCOUNT_ID);
        assert_eq!(accounts.next_id, 2);
        assert!(accounts.active().uses_legacy_storage());
    }

    #[test]
    fn account_names_are_normalized() {
        let mut accounts = Accounts {
            items: vec![Account {
                id: 2,
                name: "  Work  ".into(),
            }],
            active_id: 2,
            next_id: 3,
        };
        accounts.normalize();
        assert_eq!(accounts.items[0].name, "Work");
    }

    #[test]
    fn blank_names_fall_back_to_account_id() {
        let mut accounts = Accounts {
            items: vec![Account {
                id: 2,
                name: "   ".into(),
            }],
            active_id: 2,
            next_id: 3,
        };
        accounts.normalize();
        assert_eq!(accounts.items[0].name, "Account 2");
    }

    #[test]
    fn normalize_repairs_empty_collection_to_legacy_account() {
        let mut accounts = Accounts {
            items: Vec::new(),
            active_id: 99,
            next_id: 0,
        };
        accounts.normalize();
        assert_eq!(accounts, Accounts::default());
    }

    #[test]
    fn normalize_drops_invalid_and_duplicate_ids() {
        let mut accounts = Accounts {
            items: vec![
                Account {
                    id: 0,
                    name: "Invalid".into(),
                },
                Account {
                    id: 2,
                    name: "Work".into(),
                },
                Account {
                    id: 2,
                    name: "Duplicate".into(),
                },
                Account {
                    id: 5,
                    name: "  Personal  ".into(),
                },
            ],
            active_id: 99,
            next_id: 2,
        };

        accounts.normalize();

        assert_eq!(accounts.items.len(), 2);
        assert_eq!(accounts.items[0], Account::new(2, "Work"));
        assert_eq!(accounts.items[1], Account::new(5, "Personal"));
        assert_eq!(accounts.active_id, 2);
        assert_eq!(accounts.next_id, 6);
    }

    #[test]
    fn normalize_never_rewinds_next_id() {
        let mut accounts = Accounts {
            items: vec![Account::new(1, "Primary"), Account::new(2, "Work")],
            active_id: 1,
            next_id: 50,
        };
        accounts.normalize();
        assert_eq!(accounts.next_id, 50);
    }

    #[test]
    fn normalize_handles_maximum_id_without_wrapping() {
        let mut accounts = Accounts {
            items: vec![Account::new(AccountId::MAX, "Last")],
            active_id: AccountId::MAX,
            next_id: 2,
        };
        accounts.normalize();
        assert_eq!(accounts.next_id, AccountId::MAX);
    }
}
