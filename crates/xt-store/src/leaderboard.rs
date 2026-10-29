//! The saved public-leaderboard membership: the secret key the leaderboard
//! gave when the user joined, the nickname it accepted, and the last number
//! the app sent with when it was sent. It lives in the existing `settings`
//! table under one key; no key means not joined. The email given when
//! joining is never saved here.
//!
//! Every change after joining names the key it is about, and applies only
//! while that key is still the saved one, so an answer to a request made for
//! an earlier membership never changes a newer one.
//!
//! A stored value this build cannot read is an error, never a silent default.

use crate::{Error, Result, Store};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

const SETTING: &str = "leaderboard.membership";

/// The longest key or nickname this build saves or reads back; the
/// leaderboard's own are far shorter.
const MAX_TEXT: usize = 256;

/// One number the app sent: the 7-day leverage in tenths, or none when the
/// app had no number, and when it was accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentScore {
    pub leverage_tenths: Option<u32>,
    pub at_ms: i64,
}

/// The saved membership. Its `Debug` never shows the key.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaderboardMembership {
    pub key: String,
    pub nickname: String,
    pub last_sent: Option<SentScore>,
}

impl std::fmt::Debug for LeaderboardMembership {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LeaderboardMembership")
            .field("key", &"<hidden>")
            .field("nickname", &self.nickname)
            .field("last_sent", &self.last_sent)
            .finish()
    }
}

impl LeaderboardMembership {
    fn check(&self) -> Result<()> {
        let fits = |text: &str| !text.is_empty() && text.len() <= MAX_TEXT;
        if fits(&self.key) && fits(&self.nickname) {
            Ok(())
        } else {
            Err(Error::InvalidInput("leaderboard membership is invalid"))
        }
    }
}

impl Store {
    /// The saved membership; none when not joined.
    pub fn leaderboard_membership(&self) -> Result<Option<LeaderboardMembership>> {
        read(&self.connection)
    }

    /// Saves a new membership, replacing any saved one, and returns it as
    /// read back after the commit.
    pub fn join_leaderboard(
        &mut self,
        membership: &LeaderboardMembership,
    ) -> Result<LeaderboardMembership> {
        membership.check()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        write(&transaction, membership)?;
        let saved = read(&transaction)?.ok_or(Error::InvalidInput("membership was not saved"))?;
        transaction.commit()?;
        Ok(saved)
    }

    /// Changes the saved nickname while `key` is the saved one; returns the
    /// membership read back, or none when another (or no) membership is
    /// saved, which is left unchanged.
    pub fn rename_leaderboard_member(
        &mut self,
        key: &str,
        nickname: &str,
    ) -> Result<Option<LeaderboardMembership>> {
        self.update_member(key, |membership| membership.nickname = nickname.to_owned())
    }

    /// Records the number accepted for `key` while it is the saved one;
    /// returns the membership read back, or none as for a rename.
    pub fn record_leaderboard_score(
        &mut self,
        key: &str,
        sent: SentScore,
    ) -> Result<Option<LeaderboardMembership>> {
        self.update_member(key, |membership| membership.last_sent = Some(sent))
    }

    /// Forgets the membership while `key` is the saved one; returns whether
    /// it was removed.
    pub fn leave_leaderboard(&mut self, key: &str) -> Result<bool> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = read(&transaction)?;
        if current.is_none_or(|membership| membership.key != key) {
            return Ok(false);
        }
        transaction.execute("DELETE FROM settings WHERE key=?1", [SETTING])?;
        transaction.commit()?;
        Ok(true)
    }

    fn update_member(
        &mut self,
        key: &str,
        change: impl FnOnce(&mut LeaderboardMembership),
    ) -> Result<Option<LeaderboardMembership>> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(mut membership) = read(&transaction)?.filter(|saved| saved.key == key) else {
            return Ok(None);
        };
        change(&mut membership);
        membership.check()?;
        write(&transaction, &membership)?;
        let saved = read(&transaction)?;
        transaction.commit()?;
        Ok(saved)
    }
}

fn write(transaction: &Transaction<'_>, membership: &LeaderboardMembership) -> Result<()> {
    transaction.execute(
        "INSERT INTO settings(key,value_json) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json",
        params![SETTING, serde_json::to_string(membership)?],
    )?;
    Ok(())
}

fn read(connection: &rusqlite::Connection) -> Result<Option<LeaderboardMembership>> {
    let saved: Option<String> = connection
        .query_row(
            "SELECT value_json FROM settings WHERE key=?1",
            [SETTING],
            |row| row.get(0),
        )
        .optional()?;
    let Some(saved) = saved else {
        return Ok(None);
    };
    let membership: LeaderboardMembership = serde_json::from_str(&saved)
        .map_err(|_| Error::InvalidInput("stored leaderboard membership is invalid"))?;
    membership
        .check()
        .map_err(|_| Error::InvalidInput("stored leaderboard membership is invalid"))?;
    Ok(Some(membership))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(key: &str, nickname: &str) -> LeaderboardMembership {
        LeaderboardMembership {
            key: key.into(),
            nickname: nickname.into(),
            last_sent: None,
        }
    }

    #[test]
    fn absent_means_not_joined() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.leaderboard_membership().unwrap(), None);
    }

    #[test]
    fn a_membership_survives_reopening_and_only_its_own_key_changes_it() {
        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("leaderboard.sqlite");
        let mut store = Store::open(&path).unwrap();
        let saved = store.join_leaderboard(&member("k1", "ada")).unwrap();
        assert_eq!(saved, member("k1", "ada"));
        drop(store);
        let mut store = Store::open(&path).unwrap();
        assert_eq!(
            store.leaderboard_membership().unwrap(),
            Some(member("k1", "ada"))
        );
        // Another key's answers change nothing.
        assert_eq!(store.rename_leaderboard_member("k0", "bob").unwrap(), None);
        let sent = SentScore {
            leverage_tenths: Some(123),
            at_ms: 5,
        };
        assert_eq!(store.record_leaderboard_score("k0", sent).unwrap(), None);
        assert!(!store.leave_leaderboard("k0").unwrap());
        assert_eq!(
            store.leaderboard_membership().unwrap(),
            Some(member("k1", "ada"))
        );
        // Its own do.
        let renamed = store.rename_leaderboard_member("k1", "Ada_2").unwrap();
        assert_eq!(renamed.unwrap().nickname, "Ada_2");
        let recorded = store.record_leaderboard_score("k1", sent).unwrap().unwrap();
        assert_eq!(recorded.last_sent, Some(sent));
        let none = SentScore {
            leverage_tenths: None,
            at_ms: 6,
        };
        let recorded = store.record_leaderboard_score("k1", none).unwrap().unwrap();
        assert_eq!(recorded.last_sent, Some(none));
        assert!(store.leave_leaderboard("k1").unwrap());
        assert_eq!(store.leaderboard_membership().unwrap(), None);
        assert!(!store.leave_leaderboard("k1").unwrap());
    }

    #[test]
    fn the_key_never_appears_in_debug_output() {
        let text = format!("{:?}", member("secret-key-value", "ada"));
        assert!(!text.contains("secret-key-value"));
        assert!(text.contains("ada"));
    }

    #[test]
    fn empty_or_oversized_fields_are_refused_and_stored_invalid_data_fails_visibly() {
        let mut store = Store::open_in_memory().unwrap();
        for bad in [
            member("", "ada"),
            member("k", ""),
            member(&"k".repeat(257), "a"),
        ] {
            assert!(store.join_leaderboard(&bad).is_err());
        }
        assert_eq!(store.leaderboard_membership().unwrap(), None);
        for value in [
            "null",
            "[]",
            "{}",
            r#"{"key":"","nickname":"a","last_sent":null}"#,
        ] {
            store
                .connection
                .execute(
                    "INSERT INTO settings(key,value_json) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json",
                    params![SETTING, value],
                )
                .unwrap();
            assert!(
                matches!(store.leaderboard_membership(), Err(Error::InvalidInput(_))),
                "{value}"
            );
        }
    }

    #[test]
    fn the_saved_value_holds_no_email() {
        let mut store = Store::open_in_memory().unwrap();
        store.join_leaderboard(&member("k1", "ada")).unwrap();
        let raw: String = store
            .connection
            .query_row(
                "SELECT value_json FROM settings WHERE key=?1",
                [SETTING],
                |row| row.get(0),
            )
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let keys: std::collections::BTreeSet<_> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, ["key", "last_sent", "nickname"].into());
    }
}
