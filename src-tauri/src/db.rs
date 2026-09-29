//! SQLite storage for DevDeck.
//!
//! Replaces the webview's `localStorage`, which had three problems: the Rust
//! side could not read it (so the phone could never see workspace names, pins
//! or custom project names), clearing site data wiped it, and there was no way
//! to query or relate anything.
//!
//! SQLite is compiled into the binary via rusqlite's `bundled` feature, so
//! there is no server, no install step and no system dependency - just one file
//! beside the app's config.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Bump when the schema changes and add a matching arm in [`migrate`].
const SCHEMA_VERSION: i32 = 1;

pub struct Db {
    conn: Connection,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Project {
    pub path: String,
    pub custom_name: Option<String>,
    pub hidden: bool,
    pub pinned: bool,
    /// "private" or "public" - see [`crate::auth::Visibility`].
    pub visibility: String,
}

/// The shape the frontend sends when handing over its localStorage contents.
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct LegacyState {
    pub workspaces: Vec<String>,
    pub known_projects: Vec<String>,
    pub hidden_projects: Vec<String>,
    pub pinned_projects: Vec<String>,
    pub custom_project_names: std::collections::HashMap<String, String>,
    pub default_ide: Option<String>,
    pub tour_completed: Option<bool>,
}

fn db_path() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("com", "upendradasanayaka", "devdeck")?;
    let dir = dirs.config_dir().to_path_buf();
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("devdeck.db"))
}

impl Db {
    pub fn open() -> Result<Self, String> {
        let path = db_path().ok_or("could not locate a config directory")?;
        let conn = Connection::open(&path).map_err(|e| format!("could not open database: {e}"))?;
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Self, String> {
        // WAL lets the gateway read while the UI writes, instead of the two
        // blocking each other.
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| e.to_string())?;

        let mut db = Db { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Apply any schema steps this database has not seen.
    ///
    /// Versioning rides on SQLite's own `user_version` pragma, so there is no
    /// bookkeeping table to keep in sync.
    fn migrate(&mut self) -> Result<(), String> {
        let current: i32 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;

        if current < 1 {
            self.conn
                .execute_batch(
                    "
                    CREATE TABLE IF NOT EXISTS workspaces (
                        path     TEXT PRIMARY KEY,
                        added_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
                    );

                    CREATE TABLE IF NOT EXISTS projects (
                        path        TEXT PRIMARY KEY,
                        custom_name TEXT,
                        hidden      INTEGER NOT NULL DEFAULT 0,
                        pinned      INTEGER NOT NULL DEFAULT 0,
                        visibility  TEXT    NOT NULL DEFAULT 'private',
                        last_seen   INTEGER
                    );

                    CREATE TABLE IF NOT EXISTS settings (
                        key   TEXT PRIMARY KEY,
                        value TEXT NOT NULL
                    );

                    -- Exactly one account: DevDeck is single-user, and the CHECK
                    -- makes that a schema guarantee rather than a convention.
                    CREATE TABLE IF NOT EXISTS account (
                        id            INTEGER PRIMARY KEY CHECK (id = 1),
                        username      TEXT NOT NULL,
                        password_hash TEXT NOT NULL
                    );

                    CREATE TABLE IF NOT EXISTS sessions (
                        token      TEXT PRIMARY KEY,
                        created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
                    );

                    CREATE INDEX IF NOT EXISTS idx_projects_pinned ON projects(pinned);
                    CREATE INDEX IF NOT EXISTS idx_projects_hidden ON projects(hidden);
                    ",
                )
                .map_err(|e| format!("schema migration failed: {e}"))?;
        }

        self.conn
            .pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ---------------------------------------------------------- workspaces

    pub fn workspaces(&self) -> Result<Vec<String>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT path FROM workspaces ORDER BY added_at")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }

    pub fn add_workspace(&self, path: &str) -> Result<(), String> {
        self.conn
            .execute("INSERT OR IGNORE INTO workspaces(path) VALUES (?1)", params![path])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn remove_workspace(&self, path: &str) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM workspaces WHERE path = ?1", params![path])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ------------------------------------------------------------ projects

    pub fn projects(&self) -> Result<Vec<Project>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, custom_name, hidden, pinned, visibility FROM projects")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Project {
                    path: r.get(0)?,
                    custom_name: r.get(1)?,
                    hidden: r.get::<_, i64>(2)? != 0,
                    pinned: r.get::<_, i64>(3)? != 0,
                    visibility: r.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }

    pub fn project(&self, path: &str) -> Result<Option<Project>, String> {
        self.conn
            .query_row(
                "SELECT path, custom_name, hidden, pinned, visibility FROM projects WHERE path = ?1",
                params![path],
                |r| {
                    Ok(Project {
                        path: r.get(0)?,
                        custom_name: r.get(1)?,
                        hidden: r.get::<_, i64>(2)? != 0,
                        pinned: r.get::<_, i64>(3)? != 0,
                        visibility: r.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    /// Update one field without disturbing the others.
    ///
    /// Upserts, because a project row may not exist until the first time the
    /// user renames, hides or pins it.
    fn set_project_field(&self, path: &str, column: &str, value: &dyn rusqlite::ToSql) -> Result<(), String> {
        // `column` is never user input - only the fixed names below.
        let sql = format!(
            "INSERT INTO projects(path, {column}) VALUES (?1, ?2)
             ON CONFLICT(path) DO UPDATE SET {column} = excluded.{column}"
        );
        self.conn
            .execute(&sql, params![path, value])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn set_custom_name(&self, path: &str, name: Option<&str>) -> Result<(), String> {
        self.set_project_field(path, "custom_name", &name)
    }

    pub fn set_hidden(&self, path: &str, hidden: bool) -> Result<(), String> {
        self.set_project_field(path, "hidden", &(hidden as i64))
    }

    pub fn set_pinned(&self, path: &str, pinned: bool) -> Result<(), String> {
        self.set_project_field(path, "pinned", &(pinned as i64))
    }

    pub fn set_visibility(&self, path: &str, visibility: &str) -> Result<(), String> {
        self.set_project_field(path, "visibility", &visibility)
    }

    pub fn visibility_of(&self, path: &str) -> String {
        self.project(path)
            .ok()
            .flatten()
            .map(|p| p.visibility)
            .unwrap_or_else(|| "private".to_string())
    }

    // ------------------------------------------------------------ settings

    pub fn setting(&self, key: &str) -> Option<String> {
        self.conn
            .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO settings(key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ------------------------------------------------------------- account

    pub fn account(&self) -> Option<(String, String)> {
        self.conn
            .query_row("SELECT username, password_hash FROM account WHERE id = 1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()
            .ok()
            .flatten()
    }

    pub fn set_account(&self, username: &str, password_hash: &str) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO account(id, username, password_hash) VALUES (1, ?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET username = excluded.username,
                                               password_hash = excluded.password_hash",
                params![username, password_hash],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn add_session(&self, token: &str) -> Result<(), String> {
        self.conn
            .execute("INSERT OR IGNORE INTO sessions(token) VALUES (?1)", params![token])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn has_session(&self, token: &str) -> bool {
        self.conn
            .query_row("SELECT 1 FROM sessions WHERE token = ?1", params![token], |_| Ok(()))
            .optional()
            .map(|o| o.is_some())
            .unwrap_or(false)
    }

    pub fn clear_sessions(&self) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM sessions", [])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ----------------------------------------------------------- migration

    /// Has the one-time import from localStorage already run?
    pub fn is_migrated(&self) -> bool {
        self.setting("migrated_from_localstorage").as_deref() == Some("1")
    }

    /// Import the frontend's localStorage state.
    ///
    /// Runs in a transaction so a partial import cannot leave half the
    /// workspaces imported and the flag set. Idempotent via the flag, but the
    /// inserts are upserts anyway.
    pub fn import_legacy(&mut self, legacy: &LegacyState) -> Result<usize, String> {
        let tx = self.conn.transaction().map_err(|e| e.to_string())?;
        let mut imported = 0usize;

        for w in &legacy.workspaces {
            tx.execute("INSERT OR IGNORE INTO workspaces(path) VALUES (?1)", params![w])
                .map_err(|e| e.to_string())?;
            imported += 1;
        }

        // Every path the frontend knew about, however it knew about it.
        let mut paths: Vec<&String> = legacy.known_projects.iter().collect();
        paths.extend(legacy.hidden_projects.iter());
        paths.extend(legacy.pinned_projects.iter());
        paths.extend(legacy.custom_project_names.keys());
        paths.sort();
        paths.dedup();

        for p in paths {
            let hidden = legacy.hidden_projects.contains(p) as i64;
            let pinned = legacy.pinned_projects.contains(p) as i64;
            let name = legacy.custom_project_names.get(p);
            tx.execute(
                "INSERT INTO projects(path, custom_name, hidden, pinned)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(path) DO UPDATE SET
                     custom_name = COALESCE(excluded.custom_name, projects.custom_name),
                     hidden      = excluded.hidden,
                     pinned      = excluded.pinned",
                params![p, name, hidden, pinned],
            )
            .map_err(|e| e.to_string())?;
            imported += 1;
        }

        if let Some(ide) = &legacy.default_ide {
            tx.execute(
                "INSERT INTO settings(key, value) VALUES ('defaultIde', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![ide],
            )
            .map_err(|e| e.to_string())?;
        }
        if legacy.tour_completed == Some(true) {
            tx.execute(
                "INSERT INTO settings(key, value) VALUES ('tourCompleted', '1')
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [],
            )
            .map_err(|e| e.to_string())?;
        }

        tx.execute(
            "INSERT INTO settings(key, value) VALUES ('migrated_from_localstorage', '1')
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [],
        )
        .map_err(|e| e.to_string())?;

        tx.commit().map_err(|e| e.to_string())?;
        Ok(imported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn db() -> Db {
        Db::open_in_memory().unwrap()
    }

    #[test]
    fn creates_its_schema_on_first_open() {
        let d = db();
        let v: i32 = d.conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        assert!(d.workspaces().unwrap().is_empty());
    }

    #[test]
    fn migrating_twice_is_harmless() {
        let mut d = db();
        d.migrate().unwrap();
        d.migrate().unwrap();
        assert_eq!(d.workspaces().unwrap().len(), 0);
    }

    #[test]
    fn round_trips_workspaces() {
        let d = db();
        d.add_workspace("D:/Projects").unwrap();
        d.add_workspace("D:/Work").unwrap();
        d.add_workspace("D:/Projects").unwrap(); // duplicate is ignored
        assert_eq!(d.workspaces().unwrap(), vec!["D:/Projects", "D:/Work"]);

        d.remove_workspace("D:/Work").unwrap();
        assert_eq!(d.workspaces().unwrap(), vec!["D:/Projects"]);
    }

    #[test]
    fn project_fields_update_independently() {
        let d = db();
        d.set_custom_name("D:/a", Some("Shop")).unwrap();
        d.set_pinned("D:/a", true).unwrap();
        d.set_hidden("D:/a", true).unwrap();

        let p = d.project("D:/a").unwrap().unwrap();
        assert_eq!(p.custom_name.as_deref(), Some("Shop"));
        assert!(p.pinned, "pinning cleared the name or flag");
        assert!(p.hidden);
        assert_eq!(p.visibility, "private", "default must be private");
    }

    #[test]
    fn unknown_projects_default_to_private() {
        let d = db();
        assert_eq!(d.visibility_of("D:/never-seen"), "private");
        d.set_visibility("D:/demo", "public").unwrap();
        assert_eq!(d.visibility_of("D:/demo"), "public");
    }

    #[test]
    fn settings_round_trip() {
        let d = db();
        assert!(d.setting("defaultIde").is_none());
        d.set_setting("defaultIde", "cursor").unwrap();
        d.set_setting("defaultIde", "code").unwrap();
        assert_eq!(d.setting("defaultIde").as_deref(), Some("code"));
    }

    #[test]
    fn only_one_account_can_exist() {
        let d = db();
        d.set_account("upe", "$argon2id$hash").unwrap();
        d.set_account("upe2", "$argon2id$other").unwrap();
        let (u, h) = d.account().unwrap();
        assert_eq!(u, "upe2");
        assert_eq!(h, "$argon2id$other");

        let n: i64 = d.conn.query_row("SELECT COUNT(*) FROM account", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1, "CHECK(id = 1) should keep this a single row");
    }

    #[test]
    fn sessions_add_and_clear() {
        let d = db();
        d.add_session("tok").unwrap();
        assert!(d.has_session("tok"));
        assert!(!d.has_session("nope"));
        d.clear_sessions().unwrap();
        assert!(!d.has_session("tok"));
    }

    #[test]
    fn imports_localstorage_without_losing_anything() {
        let mut d = db();
        assert!(!d.is_migrated());

        let mut names = HashMap::new();
        names.insert("D:/Projects/shop".to_string(), "Shop Frontend".to_string());

        let legacy = LegacyState {
            workspaces: vec!["D:/Projects".into(), "D:/Work".into()],
            known_projects: vec!["D:/Projects/shop".into(), "D:/Projects/api".into()],
            hidden_projects: vec!["D:/Projects/api".into()],
            pinned_projects: vec!["D:/Projects/shop".into()],
            custom_project_names: names,
            default_ide: Some("cursor".into()),
            tour_completed: Some(true),
        };

        d.import_legacy(&legacy).unwrap();

        assert_eq!(d.workspaces().unwrap(), vec!["D:/Projects", "D:/Work"]);

        let shop = d.project("D:/Projects/shop").unwrap().unwrap();
        assert_eq!(shop.custom_name.as_deref(), Some("Shop Frontend"));
        assert!(shop.pinned);
        assert!(!shop.hidden);

        let api = d.project("D:/Projects/api").unwrap().unwrap();
        assert!(api.hidden);
        assert!(!api.pinned);

        assert_eq!(d.setting("defaultIde").as_deref(), Some("cursor"));
        assert_eq!(d.setting("tourCompleted").as_deref(), Some("1"));
        assert!(d.is_migrated());
    }

    #[test]
    fn re_importing_does_not_duplicate_workspaces() {
        let mut d = db();
        let legacy = LegacyState {
            workspaces: vec!["D:/Projects".into()],
            ..Default::default()
        };
        d.import_legacy(&legacy).unwrap();
        d.import_legacy(&legacy).unwrap();
        assert_eq!(d.workspaces().unwrap().len(), 1);
    }

    #[test]
    fn an_empty_localstorage_still_marks_migration_done() {
        let mut d = db();
        d.import_legacy(&LegacyState::default()).unwrap();
        assert!(d.is_migrated(), "otherwise the prompt would reappear forever");
    }
}
