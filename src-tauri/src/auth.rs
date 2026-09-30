//! Password hashing, sessions and visibility rules.
//!
//! Storage lives in [`crate::db`]; this module is the policy around it. Keeping
//! the split means the hashing is testable on its own and there is exactly one
//! place that owns persistence.
//!
//! Passwords are argon2id PHC strings, never plaintext and never reversible.
//! Session tokens come from the OS CSPRNG.

use crate::db::Db;
use argon2::{
    password_hash::{phc::PasswordHash, PasswordHasher, PasswordVerifier},
    Argon2,
};
use serde::{Deserialize, Serialize};

/// Whether a project may be viewed without signing in.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    /// Requires a session. The default, so a new project is never exposed by
    /// accident when a tunnel happens to be open.
    #[default]
    Private,
    /// Anyone holding the URL may view it - for showing work to a client.
    Public,
}

impl Visibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Visibility::Private => "private",
            Visibility::Public => "public",
        }
    }

    pub fn from_str(s: &str) -> Self {
        if s == "public" {
            Visibility::Public
        } else {
            Visibility::Private
        }
    }
}

/// 32 bytes of CSPRNG output, hex encoded.
pub fn random_token() -> String {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).expect("OS random number generator unavailable");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn hash_password(password: &str) -> Result<String, String> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| format!("could not hash password: {e}"))
}

pub fn verify_password(password: &str, stored_hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(stored_hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// Is this machine claimed by anybody?
///
/// An unclaimed machine stays open on the LAN, because demanding a login
/// nobody has created would lock a user out of their own computer. Once
/// claimed, access is gated.
///
/// A cloud sign-in counts. It is the only account most users will ever create,
/// and checking the local table alone left a cloud-signed-in machine looking
/// unclaimed - which served every project to anyone who could reach it.
pub fn has_account(db: &Db) -> bool {
    db.account().is_some() || crate::cloud::owner(db).is_some()
}

pub fn username(db: &Db) -> Option<String> {
    db.account().map(|(u, _)| u)
}

/// Create or replace the account.
///
/// Existing sessions are dropped: a password change must not leave an old
/// phone signed in.
pub fn set_account(db: &Db, username: &str, password: &str) -> Result<(), String> {
    if username.trim().is_empty() {
        return Err("Username cannot be empty".into());
    }
    if password.len() < 8 {
        return Err("Password must be at least 8 characters".into());
    }
    let hash = hash_password(password)?;
    db.set_account(username.trim(), &hash)?;
    db.clear_sessions()?;
    Ok(())
}

/// Check credentials and mint a session token.
///
/// The same message covers a wrong username and a wrong password, so the
/// response does not reveal whether a username exists.
pub fn login(db: &Db, username_in: &str, password: &str) -> Result<String, String> {
    let Some((user, hash)) = db.account() else {
        return Err("No account has been created yet".into());
    };
    if username_in.trim() != user || !verify_password(password, &hash) {
        return Err("Incorrect username or password".into());
    }
    mint_session(db)
}

/// Issue a session without a password, for QR pairing on the LAN.
pub fn mint_session(db: &Db) -> Result<String, String> {
    let token = random_token();
    db.add_session(&token)?;
    Ok(token)
}

pub fn is_valid_session(db: &Db, token: &str) -> bool {
    db.has_session(token)
}

pub fn revoke_all_sessions(db: &Db) -> Result<(), String> {
    db.clear_sessions()
}

/// Strip the script name from a `"<path>:<script>"` process key.
///
/// The index guard skips a Windows drive letter's colon, so `"D:\proj"` with
/// no script is left whole.
pub fn project_path_of(key: &str) -> &str {
    match key.rfind(':') {
        Some(i) if i > 2 => &key[..i],
        _ => key,
    }
}

/// Sharing is a property of the project, not of each script it happens to be
/// running, so both sides resolve the key down to its path first. Otherwise
/// `npm run dev` and `npm start` on one folder would need sharing separately.
pub fn visibility_of(db: &Db, project_key: &str) -> Visibility {
    Visibility::from_str(&db.visibility_of(project_path_of(project_key)))
}

pub fn set_visibility(db: &Db, project_key: &str, v: Visibility) -> Result<(), String> {
    db.set_visibility(project_path_of(project_key), v.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Db {
        Db::open_in_memory().unwrap()
    }

#[test]
    fn a_cloud_sign_in_claims_the_machine() {
        let d = db();
        assert!(!has_account(&d), "a fresh machine is unclaimed");

        crate::cloud::set_owner(&d, "user-uuid-1").unwrap();
        assert!(has_account(&d), "cloud sign-in must claim the machine");

        crate::cloud::clear_owner(&d).unwrap();
        assert!(!has_account(&d), "signing out releases it again");
    }

    #[test]
    fn either_kind_of_account_claims_the_machine() {
        let d = db();
        set_account(&d, "upe", "correct-horse").unwrap();
        assert!(has_account(&d));
    }

    #[test]
    fn rejects_a_weak_or_empty_account() {
        let d = db();
        assert!(set_account(&d, "", "longenough").is_err());
        assert!(set_account(&d, "upe", "short").is_err());
        assert!(!has_account(&d));
    }

    #[test]
    fn round_trips_a_login() {
        let d = db();
        set_account(&d, "upe", "correct-horse").unwrap();
        assert!(has_account(&d));
        assert_eq!(username(&d).as_deref(), Some("upe"));

        let token = login(&d, "upe", "correct-horse").unwrap();
        assert!(is_valid_session(&d, &token));
    }

    #[test]
    fn rejects_wrong_credentials() {
        let d = db();
        set_account(&d, "upe", "correct-horse").unwrap();
        assert!(login(&d, "upe", "wrong-password").is_err());
        assert!(login(&d, "someone", "correct-horse").is_err());
    }

    #[test]
    fn never_stores_the_password_in_plaintext() {
        let d = db();
        set_account(&d, "upe", "correct-horse").unwrap();
        let (_, hash) = d.account().unwrap();
        assert!(!hash.contains("correct-horse"), "password leaked into {hash}");
        assert!(hash.starts_with("$argon2id$"), "not argon2id: {hash}");
    }

    #[test]
    fn the_same_password_hashes_differently_each_time() {
        // A per-hash salt; identical hashes would mean the salt was fixed.
        assert_ne!(
            hash_password("correct-horse").unwrap(),
            hash_password("correct-horse").unwrap()
        );
    }

    #[test]
    fn changing_the_password_signs_existing_devices_out() {
        let d = db();
        set_account(&d, "upe", "correct-horse").unwrap();
        let old = login(&d, "upe", "correct-horse").unwrap();

        set_account(&d, "upe", "a-brand-new-one").unwrap();
        assert!(!is_valid_session(&d, &old), "old phone session survived");
    }

#[test]
    fn sharing_covers_a_whole_project_not_one_script() {
        let d = db();
        set_visibility(&d, "D:/proj:dev", Visibility::Public).unwrap();

        // Another script in the same folder inherits it.
        assert_eq!(visibility_of(&d, "D:/proj:start"), Visibility::Public);
        assert_eq!(visibility_of(&d, "D:/proj"), Visibility::Public);
        // A different project does not.
        assert_eq!(visibility_of(&d, "D:/other:dev"), Visibility::Private);
    }

    #[test]
    fn a_drive_letter_colon_is_not_a_script_separator() {
        assert_eq!(project_path_of("D:/proj"), "D:/proj");
        assert_eq!(project_path_of("D:/proj:dev"), "D:/proj");
        assert_eq!(project_path_of("D:/a/b:live server"), "D:/a/b");
    }

    #[test]
    fn projects_are_private_until_shared() {
        let d = db();
        assert_eq!(visibility_of(&d, "proj:dev"), Visibility::Private);
        set_visibility(&d, "proj:dev", Visibility::Public).unwrap();
        assert_eq!(visibility_of(&d, "proj:dev"), Visibility::Public);
        assert_eq!(visibility_of(&d, "other:dev"), Visibility::Private);
    }

    #[test]
    fn qr_pairing_issues_a_usable_session() {
        let d = db();
        let t = mint_session(&d).unwrap();
        assert!(is_valid_session(&d, &t));
        revoke_all_sessions(&d).unwrap();
        assert!(!is_valid_session(&d, &t));
    }

    #[test]
    fn tokens_are_unique_and_full_length() {
        let (a, b) = (random_token(), random_token());
        assert_eq!(a.len(), 64, "32 bytes hex encoded");
        assert_ne!(a, b);
    }
}
