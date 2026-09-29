//! Local accounts and per-project visibility for the mobile gateway.
//!
//! There is no database and no server. DevDeck is single-user, so an account is
//! one record; it lives in a JSON file next to the app's other config. The
//! desktop UI keeps its state in the webview's `localStorage`, but credentials
//! cannot: it is the Rust gateway that must verify a login arriving from the
//! phone, and Rust cannot read the webview's storage.
//!
//! Passwords are stored as argon2id PHC strings, never in plaintext and never
//! reversibly. Session tokens come from the OS CSPRNG.

use argon2::{
    password_hash::{phc::PasswordHash, PasswordHasher, PasswordVerifier},
    Argon2,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

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

#[derive(Serialize, Deserialize, Default)]
pub struct AuthStore {
    username: Option<String>,
    /// argon2id PHC string, e.g. `$argon2id$v=19$m=19456,t=2,p=1$...`
    password_hash: Option<String>,
    /// Live session tokens. Persisted so a phone stays signed in when DevDeck
    /// restarts, which otherwise means re-pairing constantly during a work day.
    #[serde(default)]
    sessions: HashSet<String>,
    /// Per-project visibility, keyed by process key.
    #[serde(default)]
    visibility: HashMap<String, Visibility>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

/// 32 bytes of CSPRNG output, hex encoded.
pub fn random_token() -> String {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).expect("OS random number generator unavailable");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn store_path() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("com", "upendradasanayaka", "devdeck")?;
    let dir = dirs.config_dir().to_path_buf();
    fs::create_dir_all(&dir).ok()?;
    Some(dir.join("auth.json"))
}

impl AuthStore {
    /// Read the store from disk, or start an empty one.
    ///
    /// A corrupt file is replaced rather than fatal: losing the account means
    /// setting a password again, whereas refusing to start locks the user out
    /// of their own desktop app.
    pub fn load() -> Self {
        let path = store_path();
        let mut store = path
            .as_ref()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<AuthStore>(&s).ok())
            .unwrap_or_default();
        store.path = path;
        store
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, json);
        }
    }

    pub fn has_account(&self) -> bool {
        self.username.is_some() && self.password_hash.is_some()
    }

    pub fn username(&self) -> Option<&str> {
        self.username.as_deref()
    }

    /// Create or replace the account.
    pub fn set_account(&mut self, username: &str, password: &str) -> Result<(), String> {
        if username.trim().is_empty() {
            return Err("Username cannot be empty".into());
        }
        if password.len() < 8 {
            return Err("Password must be at least 8 characters".into());
        }

        let hash = Argon2::default()
            .hash_password(password.as_bytes())
            .map_err(|e| format!("could not hash password: {e}"))?
            .to_string();

        self.username = Some(username.trim().to_string());
        self.password_hash = Some(hash);
        // Changing the password must not leave old phones signed in.
        self.sessions.clear();
        self.save();
        Ok(())
    }

    /// Check credentials and mint a session token.
    pub fn login(&mut self, username: &str, password: &str) -> Result<String, String> {
        let (Some(user), Some(hash)) = (&self.username, &self.password_hash) else {
            return Err("No account has been created yet".into());
        };
        if username.trim() != user {
            return Err("Incorrect username or password".into());
        }

        let parsed =
            PasswordHash::new(hash).map_err(|_| "Stored password is unreadable".to_string())?;
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .map_err(|_| "Incorrect username or password".to_string())?;

        Ok(self.mint_session())
    }

    /// Issue a session without a password, for QR pairing on the LAN.
    pub fn mint_session(&mut self) -> String {
        let token = random_token();
        self.sessions.insert(token.clone());
        self.save();
        token
    }

    pub fn is_valid_session(&self, token: &str) -> bool {
        self.sessions.contains(token)
    }

    pub fn revoke_all_sessions(&mut self) {
        self.sessions.clear();
        self.save();
    }

    pub fn visibility_of(&self, project_key: &str) -> Visibility {
        self.visibility.get(project_key).copied().unwrap_or_default()
    }

    pub fn set_visibility(&mut self, project_key: &str, v: Visibility) {
        self.visibility.insert(project_key.to_string(), v);
        self.save();
    }

    pub fn all_visibility(&self) -> HashMap<String, Visibility> {
        self.visibility.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An in-memory store, so tests never touch the real config file.
    fn store() -> AuthStore {
        AuthStore::default()
    }

    #[test]
    fn rejects_a_weak_or_empty_account() {
        let mut s = store();
        assert!(s.set_account("", "longenough").is_err());
        assert!(s.set_account("upe", "short").is_err());
        assert!(!s.has_account());
    }

    #[test]
    fn round_trips_a_login() {
        let mut s = store();
        s.set_account("upe", "correct-horse").unwrap();
        assert!(s.has_account());

        let token = s.login("upe", "correct-horse").unwrap();
        assert!(s.is_valid_session(&token));
    }

    #[test]
    fn rejects_wrong_credentials() {
        let mut s = store();
        s.set_account("upe", "correct-horse").unwrap();
        assert!(s.login("upe", "wrong-password").is_err());
        assert!(s.login("someone", "correct-horse").is_err());
    }

    #[test]
    fn never_stores_the_password_in_plaintext() {
        let mut s = store();
        s.set_account("upe", "correct-horse").unwrap();
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("correct-horse"), "password leaked into {json}");
        assert!(json.contains("$argon2id$"), "not an argon2id hash: {json}");
    }

    #[test]
    fn changing_the_password_signs_existing_devices_out() {
        let mut s = store();
        s.set_account("upe", "correct-horse").unwrap();
        let old = s.login("upe", "correct-horse").unwrap();

        s.set_account("upe", "a-brand-new-one").unwrap();
        assert!(!s.is_valid_session(&old), "old phone session survived a password change");
    }

    #[test]
    fn projects_are_private_until_shared() {
        let mut s = store();
        assert_eq!(s.visibility_of("proj:dev"), Visibility::Private);
        s.set_visibility("proj:dev", Visibility::Public);
        assert_eq!(s.visibility_of("proj:dev"), Visibility::Public);
        assert_eq!(s.visibility_of("other:dev"), Visibility::Private);
    }

    #[test]
    fn qr_pairing_issues_a_usable_session() {
        let mut s = store();
        let t = s.mint_session();
        assert!(s.is_valid_session(&t));
        s.revoke_all_sessions();
        assert!(!s.is_valid_session(&t));
    }

    #[test]
    fn tokens_are_unique_and_full_length() {
        let (a, b) = (random_token(), random_token());
        assert_eq!(a.len(), 64, "32 bytes hex encoded");
        assert_ne!(a, b);
    }
}
