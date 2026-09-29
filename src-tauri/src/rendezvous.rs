//! Publishes this installation's current tunnel URL so a paired phone can find
//! it again.
//!
//! Quick tunnels hand out a new hostname on every restart, which would mean
//! re-pairing constantly. The phone instead remembers one random id and asks
//! the rendezvous service what that id currently points at.
//!
//! # What leaves the machine
//!
//! Exactly one thing: the public `*.trycloudflare.com` hostname, under a random
//! id. No account, no email, no folder paths, nothing identifying. The id is a
//! capability generated locally; the token proving ownership never leaves as a
//! readable value, and the service never returns it.

use crate::db::Db;
use serde::Serialize;
use std::time::Duration;

/// Where installations publish by default. Overridable per-install via the
/// `rendezvous_url` setting, so a fork can point at its own deployment.
pub const DEFAULT_SERVICE: &str = "https://devdeck-rendezvous.workers.dev";

const SETTING_URL: &str = "rendezvous_url";
const SETTING_ID: &str = "rendezvous_id";
const SETTING_TOKEN: &str = "rendezvous_token";

const TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Serialize, Clone, Debug)]
pub struct Identity {
    /// The capability the phone stores and resolves.
    pub id: String,
    /// Where to resolve it.
    pub service: String,
    /// A stable address the phone can bookmark; redirects to the live tunnel.
    pub shortcut: String,
}

/// Read this install's identity, creating one on first use.
///
/// The id and token are generated locally and never derived from anything
/// about the user, so the service cannot correlate installs to people.
pub fn identity(db: &Db) -> Result<Identity, String> {
    let service = db
        .setting(SETTING_URL)
        .unwrap_or_else(|| DEFAULT_SERVICE.to_string());

    let id = match db.setting(SETTING_ID) {
        Some(v) if !v.is_empty() => v,
        _ => {
            let v = crate::auth::random_token();
            db.set_setting(SETTING_ID, &v)?;
            v
        }
    };
    if db.setting(SETTING_TOKEN).unwrap_or_default().is_empty() {
        db.set_setting(SETTING_TOKEN, &crate::auth::random_token())?;
    }

    Ok(Identity {
        shortcut: format!("{}/go/{}", service.trim_end_matches('/'), id),
        service,
        id,
    })
}

fn token(db: &Db) -> Result<String, String> {
    db.setting(SETTING_TOKEN)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| "rendezvous identity is missing".to_string())
}

/// Point this install's id at `tunnel_url`.
pub fn publish(db: &Db, tunnel_url: &str) -> Result<Identity, String> {
    let ident = identity(db)?;
    let tok = token(db)?;

    let endpoint = format!("{}/r/{}", ident.service.trim_end_matches('/'), ident.id);
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .new_agent();

    let res = agent
        .put(&endpoint)
        .header("Authorization", &format!("Bearer {tok}"))
        .header("Content-Type", "application/json")
        .send_json(serde_json::json!({ "url": tunnel_url }));

    match res {
        Ok(_) => Ok(ident),
        Err(ureq::Error::StatusCode(401)) => Err(
            "This rendezvous id belongs to another installation. Reset it in the Remote panel."
                .to_string(),
        ),
        Err(e) => Err(format!("Could not reach the rendezvous service: {e}")),
    }
}

/// Stop advertising this install.
pub fn withdraw(db: &Db) -> Result<(), String> {
    let ident = identity(db)?;
    let tok = token(db)?;
    let endpoint = format!("{}/r/{}", ident.service.trim_end_matches('/'), ident.id);

    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .new_agent();

    // A failure here is not worth surfacing: the record expires on its own, and
    // the user has already asked to stop sharing.
    let _ = agent
        .delete(&endpoint)
        .header("Authorization", &format!("Bearer {tok}"))
        .call();
    Ok(())
}

/// Forget this install's id, so it can claim a fresh one.
pub fn reset(db: &Db) -> Result<Identity, String> {
    db.set_setting(SETTING_ID, "")?;
    db.set_setting(SETTING_TOKEN, "")?;
    identity(db)
}

pub fn set_service(db: &Db, url: &str) -> Result<(), String> {
    db.set_setting(SETTING_URL, url.trim_end_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Db {
        Db::open_in_memory().unwrap()
    }

    #[test]
    fn creates_an_identity_on_first_use() {
        let d = db();
        let a = identity(&d).unwrap();
        assert_eq!(a.id.len(), 64, "id should be a 32-byte capability");
        assert!(a.shortcut.ends_with(&a.id));
        assert!(a.service.starts_with("https://"));
    }

    #[test]
    fn the_identity_is_stable_across_reads() {
        let d = db();
        let a = identity(&d).unwrap();
        let b = identity(&d).unwrap();
        assert_eq!(a.id, b.id, "id must not change between launches");
    }

    #[test]
    fn resetting_issues_a_different_id() {
        let d = db();
        let before = identity(&d).unwrap();
        let after = reset(&d).unwrap();
        assert_ne!(before.id, after.id);
    }

    #[test]
    fn the_write_token_is_separate_from_the_id() {
        let d = db();
        let ident = identity(&d).unwrap();
        let tok = token(&d).unwrap();
        assert_ne!(ident.id, tok, "the id is public; the token must not equal it");
        assert_eq!(tok.len(), 64);
    }

    #[test]
    fn the_service_can_be_pointed_elsewhere() {
        let d = db();
        set_service(&d, "https://my-own.example.com/").unwrap();
        let ident = identity(&d).unwrap();
        assert_eq!(ident.service, "https://my-own.example.com");
        assert!(ident.shortcut.starts_with("https://my-own.example.com/go/"));
    }
}
