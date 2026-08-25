import { useEffect, useState } from "react";
import { ask } from "@tauri-apps/plugin-dialog";
import {
  Account,
  AccountsState,
  addAccount,
  getAccounts,
  removeAccount,
  renameAccount,
  switchAccount,
} from "../../lib/api";
import { t } from "../../lib/translations";
import { cx } from "../../lib/cx";
import { Group, Row, ui } from "../../ui/components";
import styles from "./Settings.module.css";

const PRIMARY_ACCOUNT_ID = 1;

export default function AccountsPanel() {
  const [accounts, setAccounts] = useState<AccountsState | null>(null);
  const [drafts, setDrafts] = useState<Record<number, string>>({});
  const [newName, setNewName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const sync = (next: AccountsState) => {
    setAccounts(next);
    setDrafts(Object.fromEntries(next.items.map((account) => [account.id, account.name])));
  };

  useEffect(() => {
    getAccounts()
      .then(sync)
      .catch((e) => setError(String(e)));
  }, []);

  const run = async (operation: () => Promise<AccountsState>) => {
    setBusy(true);
    setError("");
    try {
      sync(await operation());
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const add = () => {
    void run(async () => {
      const next = await addAccount(newName);
      setNewName("");
      return next;
    });
  };

  const rename = (account: Account) => {
    const name = drafts[account.id] ?? account.name;
    if (name.trim() === account.name) return;
    void run(() => renameAccount(account.id, name));
  };

  const remove = async (account: Account) => {
    const confirmed = await ask(t.removeAccountConfirm, {
      title: `${t.removeAccount}: ${account.name}`,
      kind: "warning",
      okLabel: t.removeAccount,
    });
    if (confirmed) void run(() => removeAccount(account.id));
  };

  if (!accounts) {
    return <div className={styles.accountsStatus}>{error || t.checking}</div>;
  }

  return (
    <>
      <Group title={t.accounts}>
        <Row title={t.addAccount} subtitle={t.accountsDesc}>
          <div className={styles.accountActions}>
            <input
              className={ui.input}
              value={newName}
              placeholder={t.accountName}
              disabled={busy}
              onChange={(e) => setNewName(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && !busy && add()}
            />
            <button className={cx(ui.btn, ui.accent)} disabled={busy} onClick={add}>
              {t.addAccount}
            </button>
          </div>
        </Row>
      </Group>

      <Group>
        {accounts.items.map((account) => {
          const active = account.id === accounts.active_id;
          const draft = drafts[account.id] ?? account.name;
          const changed = draft.trim() !== account.name;
          const primary = account.id === PRIMARY_ACCOUNT_ID;

          return (
            <Row
              key={account.id}
              title={account.name}
              subtitle={primary ? t.primaryAccountHint : undefined}
            >
              <div className={styles.accountActions}>
                {active && <span className={styles.activePill}>{t.activeAccount}</span>}
                <input
                  className={ui.input}
                  value={draft}
                  aria-label={t.accountName}
                  disabled={busy}
                  onChange={(e) => setDrafts({ ...drafts, [account.id]: e.target.value })}
                  onKeyDown={(e) => e.key === "Enter" && !busy && rename(account)}
                />
                {changed && (
                  <button className={ui.btn} disabled={busy} onClick={() => rename(account)}>
                    {t.renameAccount}
                  </button>
                )}
                {!active && (
                  <button
                    className={ui.btn}
                    disabled={busy}
                    onClick={() => void run(() => switchAccount(account.id))}
                  >
                    {t.switchAccount}
                  </button>
                )}
                {!primary && (
                  <button
                    className={cx(ui.btn, ui.danger)}
                    disabled={busy}
                    onClick={() => void remove(account)}
                  >
                    {t.removeAccount}
                  </button>
                )}
              </div>
            </Row>
          );
        })}
      </Group>

      {error && <div className={styles.accountError}>{error}</div>}
    </>
  );
}
