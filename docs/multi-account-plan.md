# Multi-account implementation plan

This document defines the implementation constraints for adding multiple WhatsApp Web accounts to ZeroWhats without changing the project's architecture, design language, security model, or lightweight goals.

## Non-negotiable rules

- Keep WhatsApp Web as the only protocol/client surface. Do not add whatsmeow or any alternate WhatsApp protocol implementation.
- Follow the existing ZeroWhats architecture: Rust owns application state and native integration, local Preact windows own app UI, and injected page scripts bridge the remote WhatsApp origin through the existing `zw://*` event model.
- Do not grant the remote WhatsApp origin access to application `#[tauri::command]` handlers or broader local capabilities.
- Do not stack multiple webviews inside one OS window. Each account gets one WhatsApp webview window with an isolated persistent data store.
- Preserve the current single-account session during migration. Existing users must not be forced to scan a QR code again after upgrading.
- Avoid new dependencies unless the existing Tauri/Rust/Preact stack cannot provide the required behavior.
- Reuse existing UI components, tokens, icon language, menu behavior, formatting, linting, tests, and commit conventions.

## Account model

Each account has a stable internal identity and a user-editable display name.

Suggested persisted fields:

```text
Account {
  id: u32,
  name: String,
}

AccountsState {
  accounts: Vec<Account>,
  active_account_id: u32,
  next_account_id: u32,
}
```

The account ID is internal and must never be derived from a phone number, WhatsApp DOM content, or user-visible account name.

The existing session becomes account `1` on first migration and keeps the current legacy webview storage location. Additional accounts use dedicated per-account data directories. This avoids copying or relocating WebKit session databases during migration.

## Window architecture

- Legacy account 1 keeps the `main` window label so the existing storage/session behavior remains unchanged.
- Additional accounts use stable labels such as `account-2`, `account-3`, etc.
- Only one account window is visible at a time.
- Switching accounts hides the current window and reveals the selected one while preserving a single logical geometry/monitor/maximized state for the user experience.
- All account windows receive the same WhatsApp URL, navigation allow-list, user-agent behavior, injected scripts, download handling, spellcheck, privacy behavior, and titlebar logic.

## Storage isolation

Additional account webviews must use an isolated persistent Tauri/Wry webview data directory on platforms where that API is supported.

Isolation acceptance test:

1. Log account 1 into WhatsApp A.
2. Add account 2 and log it into WhatsApp B.
3. Restart ZeroWhats.
4. Both sessions must remain independently logged in.
5. Logging out/wiping account 2 must not alter account 1 cookies, IndexedDB, local storage, service worker state, or login status.

If a platform cannot provide reliable per-webview persistent storage isolation, multi-account must be disabled there rather than silently sharing a session store.

## Event routing

Events that are naturally account-scoped gain an `accountId` field:

- `zw://unread`
- `zw://notify`
- download request/result routing
- clipboard image request/response where the response must return to the originating account

Global events stay global:

- activity used by auto-lock
- app lock state
- open-external URL requests after validation

Rust must validate account IDs received from remote-origin events before using them to select windows or storage paths.

## Tray behavior

Rust maintains an unread count per account and renders the sum using the existing tray badge renderer.

The tray menu gains an account section using the existing native menu style:

- active account indicator
- one action per account
- Add account…
- Manage accounts…

Clicking the tray icon continues to show/hide the active account.

## Notifications

Native notifications remain implemented by the existing notification module.

Each notification keeps the originating account ID. Clicking a notification reveals and focuses that account instead of blindly showing `main`.

The first implementation should avoid unnecessary notification copy changes; account-name decoration can be added later if real-world testing shows ambiguity.

## App lock

The lock remains application-wide.

- Lock hides every account window.
- Unlock restores only the last active account.
- Settings/About/Shortcuts/Accounts windows remain unavailable while locked.
- Auto-lock activity is shared across all app windows.

Independent passwords per WhatsApp account are explicitly out of scope.

## User interface

### In-page titlebar

Reuse the current ZeroWhats titlebar design. Add a compact account selector near the ZeroWhats title without introducing a second toolbar or sidebar.

The selector lists:

- existing accounts
- active account state
- Add account…
- Manage accounts…

It uses the current dropdown styling and `zw://action` event path.

### Accounts window

Create a local Preact `Accounts` screen using the same architecture as Settings/About/Shortcuts:

- `AppWindow`
- existing `Group` / `Row` / shared button styles
- CSS Modules
- existing theme tokens

Functions:

- add account
- rename account
- switch account
- remove account

The final account cannot be removed.

Removing a non-legacy account closes its webview before deleting its dedicated data directory. Account 1 requires special handling because it owns the legacy session store.

## Configuration ownership

Account state must not become a generic frontend-owned `ConfigPatch` field. Account create/rename/remove/switch operations use dedicated backend functions so the local UI cannot accidentally replace account state while saving unrelated settings.

Existing app settings remain global in the first version: theme, proxy, downloads, notifications/privacy, hardware acceleration, startup, cache, spellcheck and app lock.

## Migration requirements

A clean upgrade from ZeroWhats 1.5.3 must:

- retain the existing WhatsApp session;
- synthesize a default Account 1 only when no multi-account state exists;
- preserve all current settings;
- avoid rewriting/copying the existing WebKit session database;
- remain reversible by checking out the original branch without deleting the original session data.

## Test requirements

Add unit tests for at least:

- empty/missing account state migration;
- stable IDs and next-ID allocation;
- rejecting unknown account IDs;
- rejecting removal of the last account;
- active-account fallback when persisted state is malformed;
- label generation;
- account-directory path generation and path-safety rules;
- aggregate unread count;
- account operations preserving unrelated app config;
- legacy account 1 storage behavior;
- serialized state round-trip.

Manual integration tests must cover Linux Wayland, restart persistence, tray switching, notification click routing, lock/unlock and session isolation.

## Implementation sequence

1. Account persistence/model + unit tests.
2. Generalize main-window creation into account-window creation without changing single-account behavior.
3. Add per-account storage isolation for additional accounts.
4. Make relevant event bridges account-aware.
5. Add unread aggregation and notification routing.
6. Extend lock/single-instance/tray behavior to active-account semantics.
7. Add account-management commands and local Accounts screen.
8. Add the titlebar account selector.
9. Add translations and accessibility labels.
10. Run format/lint/build/test gates and perform the migration/isolation manual tests.

## Merge gates

The feature must not be merged unless all of the following are true:

- an existing 1.5.3 session survives upgrade without QR re-pairing;
- two accounts persist across restart independently;
- removing/logging out one account cannot affect another;
- every account uses only WhatsApp Web and the existing minimal remote capability model;
- all project checks pass;
- the resulting UI looks native to ZeroWhats rather than like a separate subsystem.
