//! Cloudflare Tunnel integration, for reaching the gateway from another network.
//!
//! On the same Wi-Fi the LAN address is enough. Across networks something has to
//! punch through NAT, and `cloudflared` does it without port forwarding, a
//! static IP, or any inbound firewall rule.
//!
//! # One tunnel per exposed port
//!
//! A quick tunnel maps a single local port to a single `*.trycloudflare.com`
//! hostname. Because [`crate::gateway`] gives every project its own listener,
//! sharing N projects means N tunnels. That is a real cost (one `cloudflared`
//! process each) and the reason sharing is opt-in per project rather than
//! blanket-on.
//!
//! A *named* tunnel on a domain you control can route many hostnames through
//! one process, which is the better answer once you have a domain on
//! Cloudflare. Quick tunnels need no account at all, so they are the default.

use regex::Regex;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::OnceLock;
use std::time::Duration;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

/// How long to wait for Cloudflare to hand back a hostname before giving up.
const URL_TIMEOUT: Duration = Duration::from_secs(45);

/// The hostname cloudflared prints once a quick tunnel is live.
fn url_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"https://[a-z0-9-]+\.trycloudflare\.com").unwrap())
}

fn no_window(cmd: &mut Command) -> &mut Command {
    #[cfg(target_os = "windows")]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// A live quick tunnel.
pub struct Tunnel {
    pub url: String,
    child: Child,
}

#[derive(Default)]
pub struct TunnelManager {
    /// Keyed by the local port being exposed.
    active: HashMap<u16, Tunnel>,
}

/// Is `cloudflared` on PATH?
pub fn is_installed() -> bool {
    Command::new("cloudflared")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .apply_no_window()
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Install cloudflared via Winget.
///
/// Winget reports an already-present package with a non-zero exit code, so a
/// failure here is only real if the binary still cannot be found afterwards.
pub fn install() -> Result<String, String> {
    if is_installed() {
        return Ok("cloudflared is already installed".into());
    }

    #[cfg(target_os = "windows")]
    {
        let out = no_window(&mut Command::new("winget"))
            .args([
                "install",
                "--id",
                "Cloudflare.cloudflared",
                "--silent",
                "--accept-source-agreements",
                "--accept-package-agreements",
            ])
            .output()
            .map_err(|e| format!("could not run winget: {e}"))?;

        if is_installed() {
            return Ok("cloudflared installed".into());
        }
        let err = String::from_utf8_lossy(&out.stderr);
        Err(format!(
            "winget could not install cloudflared. It may need a new terminal for PATH to refresh. {err}"
        ))
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("Automatic install is Windows-only. Install cloudflared with your package manager.".into())
    }
}

impl TunnelManager {
    /// Expose `port` publicly, returning the generated hostname.
    ///
    /// Blocks until Cloudflare reports the hostname, because a URL that is not
    /// yet routable is worse than a spinner: the user would scan a QR that
    /// 404s.
    pub fn open(&mut self, port: u16) -> Result<String, String> {
        if let Some(t) = self.active.get(&port) {
            return Ok(t.url.clone());
        }
        if !is_installed() {
            return Err("cloudflared is not installed".into());
        }

        let mut child = no_window(&mut Command::new("cloudflared"))
            .args([
                "tunnel",
                "--url",
                &format!("http://127.0.0.1:{port}"),
                // Quick tunnels are noisy on stdout; we only parse stderr.
                "--no-autoupdate",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not start cloudflared: {e}"))?;

        let stderr = child.stderr.take().ok_or("cloudflared produced no output")?;
        let (tx, rx) = mpsc::channel::<String>();

        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let Some(m) = url_re().find(&line) {
                    let _ = tx.send(m.as_str().to_string());
                    // Keep draining so cloudflared never blocks on a full pipe.
                }
            }
        });

        match rx.recv_timeout(URL_TIMEOUT) {
            Ok(url) => {
                self.active.insert(port, Tunnel { url: url.clone(), child });
                Ok(url)
            }
            Err(_) => {
                let _ = child.kill();
                Err("cloudflared did not return a URL in time. Check your internet connection.".into())
            }
        }
    }

    pub fn close(&mut self, port: u16) {
        if let Some(mut t) = self.active.remove(&port) {
            let _ = t.child.kill();
            let _ = t.child.wait();
        }
    }

    pub fn close_all(&mut self) {
        let ports: Vec<u16> = self.active.keys().copied().collect();
        for p in ports {
            self.close(p);
        }
    }

    pub fn all(&self) -> HashMap<u16, String> {
        self.active.iter().map(|(k, v)| (*k, v.url.clone())).collect()
    }
}

/// Small extension so the `no_window` helper reads the same as elsewhere.
trait NoWindow {
    fn apply_no_window(&mut self) -> &mut Self;
}
impl NoWindow for Command {
    fn apply_no_window(&mut self) -> &mut Self {
        no_window(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_the_quick_tunnel_hostname() {
        let line = "2026-09-29T10:00:00Z INF |  https://frozen-maple-tree-42.trycloudflare.com  |";
        assert_eq!(
            url_re().find(line).unwrap().as_str(),
            "https://frozen-maple-tree-42.trycloudflare.com"
        );
    }

    #[test]
    fn ignores_unrelated_log_lines() {
        for line in [
            "2026-09-29T10:00:00Z INF Requesting new quick Tunnel on trycloudflare.com...",
            "2026-09-29T10:00:00Z INF Starting tunnel tunnelID=abc-123",
            "",
        ] {
            assert!(url_re().find(line).is_none(), "matched: {line}");
        }
    }

    #[test]
    fn opening_without_cloudflared_is_an_error_not_a_panic() {
        if is_installed() {
            return; // nothing to assert on a machine that has it
        }
        let mut m = TunnelManager::default();
        assert!(m.open(7420).is_err());
    }

    #[test]
    fn closing_an_unknown_port_is_harmless() {
        let mut m = TunnelManager::default();
        m.close(9999);
        m.close_all();
        assert!(m.all().is_empty());
    }
}
