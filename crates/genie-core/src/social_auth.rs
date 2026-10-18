//! Stable provider identities never imply ownership of an account with the same email.

use rusqlite::{OptionalExtension, params};

use crate::server_db::{ServerDb, User, new_secret};
use crate::{GenieError, Result};

impl ServerDb {
    pub fn social_providers(&self, user: i64) -> Result<Vec<String>> {
        let mut stmt = self.conn().prepare("SELECT provider FROM social_identities WHERE user = ?1 ORDER BY provider")?;
        Ok(stmt.query_map([user], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
    }

    /// Resolve or explicitly link an identity and issue the normal session atomically.
    /// New accounts have neither a password, server privileges nor project memberships.
    pub fn social_login(
        &self,
        provider: &str,
        subject: &str,
        name: &str,
        allow_registration: bool,
        link_user: Option<i64>,
    ) -> Result<(String, User)> {
        if !matches!(provider, "github" | "google") || subject.is_empty() || subject.len() > 255 {
            return Err(GenieError::invalid("invalid provider identity"));
        }
        self.tx(|| {
            let existing: Option<i64> = self
                .conn()
                .query_row("SELECT user FROM social_identities WHERE provider = ?1 AND subject = ?2", params![provider, subject], |r| {
                    r.get(0)
                })
                .optional()?;
            let user = match (existing, link_user) {
                (Some(found), Some(target)) if found != target => {
                    return Err(GenieError::Denied("this provider identity belongs to another account".into()));
                }
                (Some(found), _) => self.user(found)?,
                (None, target) => {
                    let user = if let Some(target) = target {
                        if self.social_providers(target)?.iter().any(|p| p == provider) {
                            return Err(GenieError::Denied("this account already has a different identity for this provider".into()));
                        }
                        self.user(target)?
                    } else {
                        if !allow_registration || self.user_count()? == 0 {
                            return Err(GenieError::Denied(
                                "registration is closed; sign in with your password and connect the provider in profile settings".into(),
                            ));
                        }
                        self.create_user(&format!("{provider}-{}", &new_secret()[..16]), name, None, None, false)?
                    };
                    self.conn().execute(
                        "INSERT INTO social_identities(provider, subject, user) VALUES (?1, ?2, ?3)",
                        params![provider, subject, user.id],
                    )?;
                    user
                }
            };
            if user.disabled {
                return Err(GenieError::Denied("this account is disabled".into()));
            }
            Ok((self.create_session(user.id)?, user))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_linking_and_permissions() {
        let tmp = tempfile::tempdir().unwrap();
        let db = ServerDb::open(&tmp.path().join("server.db")).unwrap();
        assert!(db.social_login("github", "1", "Admin?", true, None).is_err(), "cannot bootstrap an administrator remotely");
        let owner = db.create_user("owner", "Owner", Some("same@example.com"), None, true).unwrap();
        assert!(db.social_login("github", "1", "Name", false, None).is_err());
        let (session, user) = db.social_login("github", "1", "Name", true, None).unwrap();
        assert_ne!(user.id, owner.id);
        assert!(!user.is_admin);
        assert_eq!(db.session_user(&session).unwrap().unwrap().id, user.id);
        assert_eq!(db.social_login("github", "1", "Renamed", false, None).unwrap().1.id, user.id);
        assert!(db.social_login("github", "1", "Name", false, Some(owner.id)).is_err());
        db.social_login("google", "sub", "Name", false, Some(owner.id)).unwrap();
        assert!(db.social_login("google", "other", "Name", false, Some(owner.id)).is_err());
        assert_eq!(db.social_login("google", "sub", "Name", false, None).unwrap().1.id, owner.id);
        db.conn().execute("UPDATE users SET disabled = 1 WHERE id = ?1", [user.id]).unwrap();
        assert!(db.social_login("github", "1", "Name", true, None).is_err());
        assert!(db.social_login("google", "new", "Name", false, Some(user.id)).is_err());
        assert!(db.social_providers(user.id).unwrap().iter().all(|p| p != "google"), "failed linking rolls back");
    }
    #[test]
    fn upgrades_v6_without_changing_existing_users_or_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("server.db");
        let db = ServerDb::open(&path).unwrap();
        let user = db.create_user("owner", "Owner", None, None, true).unwrap();
        let session = db.create_session(user.id).unwrap();
        db.conn().execute_batch("DROP TABLE social_identities; UPDATE meta SET value = '6' WHERE key = 'schema';").unwrap();
        drop(db);
        let upgraded = ServerDb::open(&path).unwrap();
        assert_eq!(upgraded.user(user.id).unwrap(), user);
        assert_eq!(upgraded.session_user(&session).unwrap().unwrap().id, user.id);
        assert!(upgraded.social_providers(user.id).unwrap().is_empty());
        upgraded.social_login("github", "123", "Owner", false, Some(user.id)).unwrap();
        assert_eq!(upgraded.conn().query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0)).unwrap(), "ok");
        upgraded.conn().execute("DELETE FROM users WHERE id = ?1", [user.id]).unwrap();
        assert!(upgraded.social_providers(user.id).unwrap().is_empty());
    }
}
