//! Live reload for projects that have no hot-reload of their own.
//!
//! A Vite or Next dev server ships its own HMR client, and the gateway's
//! WebSocket passthrough is enough to make that work on the phone. A static
//! HTML folder served by `live-server`, or a PHP project behind `php -S`, has
//! nothing of the sort: edit a file and the phone keeps showing the old page.
//!
//! This watches the project directory and tells the page to reload. The page
//! learns about it through a script injected into HTML responses, which
//! long-polls a single endpoint - no WebSocket, no streaming machinery, and no
//! extra dependency to keep the client side honest.

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};
use std::path::Path;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

/// Path the injected script polls. Prefixed so it cannot collide with a real
/// route in the project being proxied.
pub const RELOAD_PATH: &str = "/__devdeck_livereload";

/// Editors write a file several times per save (truncate, write, rename), and
/// bundlers touch whole directories. Collapse that into one reload.
const DEBOUNCE: Duration = Duration::from_millis(180);

/// How long a poll waits before answering "nothing yet". Short enough that a
/// proxy or phone radio will not silently drop the connection.
const POLL_TIMEOUT: Duration = Duration::from_secs(25);

/// Directories whose churn is never worth a reload.
const IGNORED: &[&str] = &[
    "node_modules", ".git", "target", "dist", ".next", ".nuxt", "vendor",
    "__pycache__", ".venv", "venv", ".cache", "storage", ".idea", ".vscode",
];

/// The script handed to pages that would otherwise never refresh.
///
/// Deliberately tiny and dependency-free: it is injected into someone else's
/// document and must not disturb it.
fn client_script() -> String {
    format!(
        "<script>(function(){{var f=0;async function p(){{try{{var r=await fetch('{RELOAD_PATH}',\
{{cache:'no-store'}});f=0;if(r.status===200){{location.reload();return;}}}}catch(e){{f++;}}\
setTimeout(p,f?Math.min(1000*f,10000):0);}}p();}})();</script>"
    )
}

/// Watches one project directory and fans changes out to connected pages.
pub struct Reloader {
    tx: broadcast::Sender<()>,
    /// Dropping the watcher stops the OS notifications, so it is held here for
    /// exactly as long as the preview lives.
    _watcher: RecommendedWatcher,
}

impl Reloader {
    /// Begin watching `dir`. Returns `None` if the path cannot be watched,
    /// which simply means that project goes without live reload.
    pub fn watch(dir: &Path) -> Option<Arc<Self>> {
        let (tx, _) = broadcast::channel::<()>(16);
        let (raw_tx, raw_rx) = mpsc::channel::<notify::Result<notify::Event>>();

        let mut watcher = notify::recommended_watcher(move |res| {
            let _ = raw_tx.send(res);
        })
        .ok()?;
        watcher.watch(dir, RecursiveMode::Recursive).ok()?;

        let out = tx.clone();
        std::thread::spawn(move || {
            let mut last = Instant::now() - DEBOUNCE;
            for event in raw_rx {
                let Ok(event) = event else { continue };
                if !event.paths.iter().any(|p| is_interesting(p)) {
                    continue;
                }
                if last.elapsed() < DEBOUNCE {
                    continue;
                }
                last = Instant::now();
                // No receivers simply means nobody has the page open.
                let _ = out.send(());
            }
        });

        Some(Arc::new(Reloader { tx, _watcher: watcher }))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.tx.subscribe()
    }
}

/// Should a change to this path trigger a reload?
fn is_interesting(path: &Path) -> bool {
    let s = path.to_string_lossy().replace('\\', "/");

    if IGNORED.iter().any(|d| s.contains(&format!("/{d}/")) || s.ends_with(&format!("/{d}"))) {
        return false;
    }

    // Editor scratch files: vim swap, Emacs locks, JetBrains and Office temps.
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if name.starts_with('.') && name.ends_with(".swp") {
            return false;
        }
        if name.starts_with('~') || name.starts_with(".#") || name.ends_with('~') {
            return false;
        }
    }
    true
}

/// Does this project serve pages that will never reload themselves?
///
/// Keyed on the script DevDeck generated, so it stays conservative: injecting
/// into a framework that already has HMR would cause double reloads.
pub fn needs_injection(script_name: &str) -> bool {
    matches!(script_name.trim(), "live server" | "php serve")
}

/// Insert the reload client into an HTML document.
///
/// Prefers just inside `</body>`, falling back to `</html>` and finally to
/// appending, so a fragment without a body tag still gets it.
pub fn inject(html: &str) -> String {
    let script = client_script();
    for tag in ["</body>", "</BODY>", "</html>", "</HTML>"] {
        if let Some(idx) = html.rfind(tag) {
            let mut out = String::with_capacity(html.len() + script.len());
            out.push_str(&html[..idx]);
            out.push_str(&script);
            out.push_str(&html[idx..]);
            return out;
        }
    }
    format!("{html}{script}")
}

/// Wait for a change, or report that none arrived.
///
/// `true` means reload. A lagged receiver also means reload: the page missed
/// events, so it is certainly out of date.
pub async fn wait_for_change(mut rx: broadcast::Receiver<()>) -> bool {
    matches!(
        tokio::time::timeout(POLL_TIMEOUT, rx.recv()).await,
        Ok(Ok(())) | Ok(Err(broadcast::error::RecvError::Lagged(_)))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn injects_before_the_closing_body() {
        let out = inject("<html><body><h1>hi</h1></body></html>");
        assert!(out.contains(RELOAD_PATH));
        let script_at = out.find("<script>").unwrap();
        let body_at = out.find("</body>").unwrap();
        assert!(script_at < body_at, "script landed after </body>");
        assert!(out.ends_with("</body></html>"));
    }

    #[test]
    fn falls_back_when_there_is_no_body_tag() {
        assert!(inject("<html><h1>hi</h1></html>").contains(RELOAD_PATH));
        assert!(inject("just a fragment").contains(RELOAD_PATH));
    }

    #[test]
    fn injects_only_once_per_document() {
        let out = inject("<html><body></body></html>");
        assert_eq!(out.matches(RELOAD_PATH).count(), 1);
    }

    #[test]
    fn uses_the_last_closing_body_not_the_first() {
        // A literal "</body>" inside page text must not capture the injection.
        let html = "<html><body><pre>&lt;/body&gt;</pre></body></html>";
        let out = inject(html);
        assert!(out.ends_with("</body></html>"));
    }

    #[test]
    fn only_frameworkless_scripts_get_injected() {
        assert!(needs_injection("live server"));
        assert!(needs_injection("php serve"));
        // These ship their own HMR; injecting would double-reload.
        for s in ["dev", "start", "cargo run", "docker up", "spring boot"] {
            assert!(!needs_injection(s), "would have injected into {s}");
        }
    }

    #[test]
    fn ignores_build_output_and_editor_scratch_files() {
        let boring = [
            "D:/proj/node_modules/react/index.js",
            "D:/proj/.git/index",
            "D:/proj/target/debug/app.exe",
            "D:/proj/dist/bundle.js",
            "D:/proj/.index.html.swp",
            "D:/proj/index.html~",
            "D:/proj/.#index.html",
        ];
        for p in boring {
            assert!(!is_interesting(&PathBuf::from(p)), "would reload for {p}");
        }
    }

    #[test]
    fn real_edits_are_interesting() {
        for p in [
            "D:/proj/index.html",
            "D:/proj/css/site.css",
            "D:/proj/src/app.php",
            "D:/proj/assets/logo.svg",
        ] {
            assert!(is_interesting(&PathBuf::from(p)), "missed edit to {p}");
        }
    }

    #[test]
    fn a_directory_named_like_an_ignored_one_still_counts() {
        // "distribution" is not "dist".
        assert!(is_interesting(&PathBuf::from("D:/proj/distribution/index.html")));
        assert!(is_interesting(&PathBuf::from("D:/proj/src/vendors.css")));
    }
}
