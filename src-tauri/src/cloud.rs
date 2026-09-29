//! Cloud accounts, so one login works on every device.
//!
//! Backed by Supabase: Postgres with authentication built in. Signup, email
//! verification, password reset and token issuing are all handled there rather
//! than hand-rolled here, because getting credential handling subtly wrong is
//! how people get breached.
//!
//! # What is stored in the cloud, and what is not
//!
//! The cloud holds an account and a short list of that account's machines:
//!
//! ```text
//! auth.users   email, password hash, verification state   (managed by Supabase)
//! devices      name, current tunnel URL, last seen
//! ```
//!
//! It does **not** hold workspaces, project paths, custom names or logs. Those
//! are absolute paths that mean nothing on another machine, and they leak
//! client names and employers - so they stay in the local database. The cloud's
//! only job is answering "where is this user's PC right now".
//!
//! Row Level Security scopes every row to its owner in Postgres, so isolation
//! does not depend on this client behaving.
//!
//! # Layered access
//!
//! A cloud login tells the phone *where* the PC is. The PC still decides
//! whether to let it in: it checks the presented token against Supabase and
//! only mints a local session when the account matches the machine's owner.
//! A compromised cloud account therefore yields a URL, not a dev server.

use crate::db::Db;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// The project shipped builds use. Safe to compile in: the anon key carries no
/// privileges of its own and is meant to be public - it ships in every Supabase
/// browser client - while Row Level Security is what protects the data. The
/// service_role key must never appear here.
pub const DEFAULT_URL: &str = "https://bedkemoieumrwlhkfnoh.supabase.co";
pub const DEFAULT_ANON_KEY: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6ImJlZGtlbW9pZXVtcndsaGtmbm9oIiwicm9sZSI6ImFub24iLCJpYXQiOjE3OTA2Njk0NDgsImV4cCI6MjEwNjI0NTQ0OH0.B0L4hhCgBSAJ1JCpoINGWQD-vCcCHQpsfZ7QEMwxQwo";

const SETTING_URL: &str = "cloud_url";
const SETTING_KEY: &str = "cloud_anon_key";
const SETTING_OWNER: &str = "cloud_owner_id";

const TIMEOUT: Duration = Duration::from_secs(20);

/// Connection details for a Supabase project.
///
/// The anon key is designed to be public - it ships in every Supabase browser
/// client - and carries no privileges of its own; Row Level Security is what
/// protects the data.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct CloudConfig {
    pub url: String,
    pub anon_key: String,
}

#[derive(Clone, Serialize, Debug)]
pub struct CloudSession {
    pub access_token: String,
    pub refresh_token: String,
    pub user_id: String,
    pub email: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Device {
    pub name: String,
    pub tunnel_url: Option<String>,
    pub updated_at: Option<String>,
}

/// Read the configured project, if this build has one.
pub fn config(db: &Db) -> Option<CloudConfig> {
    // All-or-nothing: a stored URL is only ever paired with its own stored key.
    // Falling back field by field could pair someone's own project URL with the
    // shipped key, which would authenticate against the wrong project.
    let stored_url = db.setting(SETTING_URL).filter(|s| !s.trim().is_empty());
    let stored_key = db.setting(SETTING_KEY).filter(|s| !s.trim().is_empty());

    let (url, anon_key) = match (stored_url, stored_key) {
        (Some(u), Some(k)) => (u, k),
        _ if !DEFAULT_URL.is_empty() => (DEFAULT_URL.to_string(), DEFAULT_ANON_KEY.to_string()),
        _ => return None,
    };

    Some(CloudConfig {
        url: url.trim_end_matches('/').to_string(),
        anon_key,
    })
}

pub fn set_config(db: &Db, url: &str, anon_key: &str) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("The project URL must start with https://".into());
    }
    db.set_setting(SETTING_URL, url.trim().trim_end_matches('/'))?;
    db.set_setting(SETTING_KEY, anon_key.trim())?;
    Ok(())
}

pub fn is_configured(db: &Db) -> bool {
    config(db).is_some()
}

/// The account this machine belongs to, if one has signed in here.
pub fn owner(db: &Db) -> Option<String> {
    db.setting(SETTING_OWNER).filter(|s| !s.is_empty())
}

pub fn set_owner(db: &Db, user_id: &str) -> Result<(), String> {
    db.set_setting(SETTING_OWNER, user_id)
}

pub fn clear_owner(db: &Db) -> Result<(), String> {
    db.set_setting(SETTING_OWNER, "")
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        // Without this, ureq turns a 4xx into an error that has already
        // discarded the body - and Supabase puts the useful message there.
        .http_status_as_error(false)
        .build()
        .new_agent()
}

/// Split a response into its status and body text.
fn read_body(res: &mut ureq::http::Response<ureq::Body>) -> (u16, String) {
    let status = res.status().as_u16();
    let text = res.body_mut().read_to_string().unwrap_or_default();
    (status, text)
}

fn ok(status: u16) -> bool {
    (200..300).contains(&status)
}

/// Turn a Supabase error body into something worth showing a person.
///
/// Its messages are aimed at developers ("invalid_grant"), so the common cases
/// are translated and anything unrecognised is passed through rather than
/// swallowed.
pub fn friendly_error(status: u16, body: &str) -> String {
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let code = parsed
        .get("error_code")
        .or_else(|| parsed.get("error"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let msg = parsed
        .get("msg")
        .or_else(|| parsed.get("error_description"))
        .or_else(|| parsed.get("message"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    match (status, code) {
        (400, "invalid_grant") | (400, "invalid_credentials") => {
            "Incorrect email or password.".into()
        }
        // An expired or malformed token comes back as 403/bad_jwt, not 401.
        (_, "bad_jwt") | (401, _) | (403, "bad_jwt") => {
            "That sign-in has expired. Sign in again.".into()
        }
        (400, "email_not_confirmed") | (_, "email_not_confirmed") => {
            "Check your inbox and confirm your email address first.".into()
        }
        (422, _) if msg.contains("already registered") => {
            "That email already has an account. Sign in instead.".into()
        }
        (422, _) if msg.to_lowercase().contains("password") => {
            "That password is too weak. Use at least 8 characters.".into()
        }
        (429, _) => "Too many attempts. Wait a minute and try again.".into(),
        _ if !msg.is_empty() => msg.to_string(),
        _ => format!("The cloud service returned an error ({status})."),
    }
}

fn parse_session(body: &serde_json::Value) -> Result<CloudSession, String> {
    let access_token = body
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or("The cloud service did not return a session")?
        .to_string();
    let user = body.get("user").ok_or("The cloud response had no user")?;

    Ok(CloudSession {
        access_token,
        refresh_token: body
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        user_id: user
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or("The cloud response had no user id")?
            .to_string(),
        email: user
            .get("email")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
    })
}

/// Create an account. Whether a confirmation email is required is a project
/// setting on the Supabase side, not something this client decides.
pub fn sign_up(cfg: &CloudConfig, email: &str, password: &str) -> Result<String, String> {
    if password.len() < 8 {
        return Err("Password must be at least 8 characters".into());
    }
    let mut res = agent()
        .post(&format!("{}/auth/v1/signup", cfg.url))
        .header("apikey", &cfg.anon_key)
        .header("Content-Type", "application/json")
        .send_json(serde_json::json!({ "email": email.trim(), "password": password }))
        .map_err(|e| format!("Could not reach the cloud service: {e}"))?;

    let (status, body) = read_body(&mut res);
    if !ok(status) {
        return Err(friendly_error(status, &body));
    }
    Ok("Account created. Check your email if confirmation is required.".into())
}

/// Exchange an email and password for a session.
pub fn sign_in(cfg: &CloudConfig, email: &str, password: &str) -> Result<CloudSession, String> {
    let mut res = agent()
        .post(&format!("{}/auth/v1/token?grant_type=password", cfg.url))
        .header("apikey", &cfg.anon_key)
        .header("Content-Type", "application/json")
        .send_json(serde_json::json!({ "email": email.trim(), "password": password }))
        .map_err(|e| format!("Could not reach the cloud service: {e}"))?;

    let (status, text) = read_body(&mut res);
    if !ok(status) {
        return Err(friendly_error(status, &text));
    }
    let body: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("Unexpected response from the cloud service: {e}"))?;
    parse_session(&body)
}

/// Confirm an access token really belongs to an account, and say which one.
///
/// Used by the gateway when a phone presents a cloud token: the check goes to
/// Supabase rather than verifying a signature locally, so no signing secret has
/// to live on the user's machine.
pub fn verify_token(cfg: &CloudConfig, access_token: &str) -> Result<String, String> {
    let mut res = agent()
        .get(&format!("{}/auth/v1/user", cfg.url))
        .header("apikey", &cfg.anon_key)
        .header("Authorization", &format!("Bearer {access_token}"))
        .call()
        .map_err(|e| format!("Could not reach the cloud service: {e}"))?;

    let (status, text) = read_body(&mut res);
    if !ok(status) {
        return Err(friendly_error(status, &text));
    }
    let body: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("Unexpected response from the cloud service: {e}"))?;

    body.get("id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| "The cloud service did not identify the account".into())
}

/// Advertise this machine's current tunnel URL under the signed-in account.
pub fn upsert_device(
    cfg: &CloudConfig,
    session_token: &str,
    user_id: &str,
    name: &str,
    tunnel_url: Option<&str>,
) -> Result<(), String> {
    let mut res = agent()
        .post(&format!("{}/rest/v1/devices", cfg.url))
        .header("apikey", &cfg.anon_key)
        .header("Authorization", &format!("Bearer {session_token}"))
        .header("Content-Type", "application/json")
        // Re-registering the same machine updates it rather than piling up rows.
        .header("Prefer", "resolution=merge-duplicates,return=minimal")
        .send_json(serde_json::json!([{
            "user_id": user_id,
            "name": name,
            "tunnel_url": tunnel_url,
        }]))
        .map_err(|e| format!("Could not reach the cloud service: {e}"))?;

    let (status, body) = read_body(&mut res);
    if !ok(status) {
        return Err(friendly_error(status, &body));
    }
    Ok(())
}

/// Every machine registered to the signed-in account.
///
/// Row Level Security restricts this to the caller's own rows in Postgres; the
/// filter is not something this client is trusted to apply.
pub fn list_devices(cfg: &CloudConfig, session_token: &str) -> Result<Vec<Device>, String> {
    let mut res = agent()
        .get(&format!(
            "{}/rest/v1/devices?select=name,tunnel_url,updated_at&order=updated_at.desc",
            cfg.url
        ))
        .header("apikey", &cfg.anon_key)
        .header("Authorization", &format!("Bearer {session_token}"))
        .call()
        .map_err(|e| format!("Could not reach the cloud service: {e}"))?;

    let (status, text) = read_body(&mut res);
    if !ok(status) {
        return Err(friendly_error(status, &text));
    }
    serde_json::from_str::<Vec<Device>>(&text)
        .map_err(|e| format!("Unexpected response from the cloud service: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Db {
        Db::open_in_memory().unwrap()
    }

    #[test]
    fn ships_with_a_project_configured() {
        let d = db();
        assert!(is_configured(&d), "builds should work out of the box");
        assert_eq!(config(&d).unwrap().url, DEFAULT_URL);
    }

    #[test]
    fn the_compiled_key_is_an_anon_key_not_service_role() {
        // service_role bypasses Row Level Security entirely; shipping it would
        // expose every user's rows to every other user.
        let payload = DEFAULT_ANON_KEY.split('.').nth(1).expect("malformed JWT");
        let mut b64 = payload.replace('-', "+").replace('_', "/");
        while b64.len() % 4 != 0 {
            b64.push('=');
        }
        let decoded = decode_b64(&b64);
        assert!(decoded.contains("\"role\":\"anon\""), "not an anon key: {decoded}");
        assert!(!decoded.contains("service_role"), "SERVICE ROLE KEY COMPILED IN");
    }

    /// Minimal base64 decode, so the guard above needs no dependency.
    fn decode_b64(input: &str) -> String {
        const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = Vec::new();
        let mut buf = 0u32;
        let mut bits = 0u32;
        for c in input.bytes() {
            if c == b'=' {
                break;
            }
            let Some(v) = TABLE.iter().position(|&t| t == c) else { continue };
            buf = (buf << 6) | v as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((buf >> bits) as u8);
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    #[test]
    fn stores_and_normalises_the_project_url() {
        let d = db();
        set_config(&d, "https://abc.supabase.co/", "anon-key-123").unwrap();
        let c = config(&d).unwrap();
        assert_eq!(c.url, "https://abc.supabase.co", "trailing slash not trimmed");
        assert_eq!(c.anon_key, "anon-key-123");
        assert!(is_configured(&d));
    }

    #[test]
    fn refuses_a_plaintext_project_url() {
        // Credentials would cross the network in the clear.
        let d = db();
        assert!(set_config(&d, "http://abc.supabase.co", "k").is_err());
        // The shipped project still applies; the bad override simply is not stored.
        assert_eq!(config(&d).unwrap().url, DEFAULT_URL);
    }

    #[test]
    fn a_half_configured_override_falls_back_whole() {
        // A custom URL with no custom key must not borrow the shipped key and
        // authenticate against the wrong project.
        let d = db();
        d.set_setting(SETTING_URL, "https://someone-elses.supabase.co").unwrap();
        assert_eq!(config(&d).unwrap().url, DEFAULT_URL);
    }

    #[test]
    fn remembers_which_account_owns_this_machine() {
        let d = db();
        assert!(owner(&d).is_none());
        set_owner(&d, "user-uuid-1").unwrap();
        assert_eq!(owner(&d).as_deref(), Some("user-uuid-1"));
        clear_owner(&d).unwrap();
        assert!(owner(&d).is_none());
    }

    #[test]
    fn rejects_a_short_password_before_any_network_call() {
        let cfg = CloudConfig {
            url: "https://abc.supabase.co".into(),
            anon_key: "k".into(),
        };
        assert!(sign_up(&cfg, "a@b.com", "short").is_err());
    }

    #[test]
    fn translates_supabase_errors_into_plain_language() {
        assert_eq!(
            friendly_error(400, r#"{"error_code":"invalid_grant"}"#),
            "Incorrect email or password."
        );
        assert!(friendly_error(429, "{}").contains("Too many attempts"));
        assert!(friendly_error(422, r#"{"msg":"User already registered"}"#).contains("Sign in instead"));
        assert!(friendly_error(400, r#"{"error_code":"email_not_confirmed"}"#).contains("confirm your email"));
    }

#[test]
    fn handles_the_error_shapes_the_live_service_actually_returns() {
        // Captured from a real Supabase project rather than guessed.
        assert_eq!(
            friendly_error(400, r#"{"code":400,"error_code":"invalid_credentials","msg":"Invalid login credentials"}"#),
            "Incorrect email or password."
        );
        // An expired or malformed token is 403/bad_jwt, not 401.
        let expired = friendly_error(
            403,
            r#"{"code":403,"error_code":"bad_jwt","msg":"invalid JWT: unable to parse or verify signature"}"#,
        );
        assert!(expired.contains("expired"), "unhelpful message: {expired}");
    }

    #[test]
    fn an_unrecognised_error_is_passed_through_not_swallowed() {
        let out = friendly_error(500, r#"{"msg":"database is on fire"}"#);
        assert_eq!(out, "database is on fire");
        assert!(friendly_error(503, "not json").contains("503"));
    }

    #[test]
    fn parses_a_session_out_of_a_token_response() {
        let body = serde_json::json!({
            "access_token": "jwt-here",
            "refresh_token": "refresh-here",
            "user": { "id": "user-uuid-1", "email": "a@b.com" }
        });
        let s = parse_session(&body).unwrap();
        assert_eq!(s.user_id, "user-uuid-1");
        assert_eq!(s.email, "a@b.com");
        assert_eq!(s.access_token, "jwt-here");
    }

    #[test]
    fn a_response_without_a_session_is_an_error() {
        assert!(parse_session(&serde_json::json!({ "user": { "id": "x" } })).is_err());
        assert!(parse_session(&serde_json::json!({ "access_token": "t" })).is_err());
    }
}
