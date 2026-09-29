//! Detection of the HTTP port a spawned dev server binds to.
//!
//! DevDeck launches dev servers as opaque `cmd` invocations, so the only
//! reliable signal about where a server ended up listening is the banner it
//! prints on startup. This module turns those banners into a port number so the
//! gateway knows where to proxy.

use regex::Regex;
use std::sync::OnceLock;

/// Escape sequences dev servers use for coloured output. Vite in particular
/// wraps its URLs in them, so stripping has to happen before matching.
fn ansi_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\x1b\[[0-9;?]*[A-Za-z]").unwrap())
}

/// `http://localhost:5173/`, `http://127.0.0.1:8000`, `http://[::1]:3000`.
/// Deliberately limited to loopback hosts so documentation links in a banner
/// are never mistaken for the server's own address.
fn url_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)https?://(?:localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1\]):(\d{1,5})").unwrap()
    })
}

/// `Server running on port 4000`, `listening on port 3000`.
fn port_word_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\bport[:= ]\s*(\d{2,5})\b").unwrap())
}

/// Lines reporting a *conflict* rather than a successful bind. Vite prints
/// "Port 5173 is in use, trying another one..." before settling elsewhere, so
/// treating that as a detection would point the proxy at the wrong server.
fn conflict_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)in use|EADDRINUSE|address already").unwrap())
}

pub fn strip_ansi(line: &str) -> String {
    ansi_re().replace_all(line, "").into_owned()
}

/// Extract the port a dev server reports listening on, if this line announces one.
///
/// A URL wins over a bare "port N" mention, because the latter also appears in
/// prose. Returns `None` for conflict notices and for anything unparseable.
pub fn detect_port(line: &str) -> Option<u16> {
    let clean = strip_ansi(line);

    if conflict_re().is_match(&clean) {
        return None;
    }

    let from = |re: &Regex| -> Option<u16> {
        re.captures(&clean)
            .and_then(|c| c.get(1))
            .and_then(|m| m.as_str().parse::<u16>().ok())
            .filter(|p| *p > 0)
    };

    from(url_re()).or_else(|| from(port_word_re()))
}

#[cfg(test)]
mod tests {
    use super::detect_port;

    #[test]
    fn reads_vite_banner_through_ansi_codes() {
        let line = "  \x1b[32m➜\x1b[39m  \x1b[1mLocal\x1b[22m:   \x1b[36mhttp://localhost:\x1b[1m5173\x1b[22m/\x1b[39m";
        assert_eq!(detect_port(line), Some(5173));
    }

    #[test]
    fn reads_common_framework_banners() {
        let cases = [
            ("- Local:        http://localhost:3000", 3000),
            ("PHP 8.3.0 Development Server (http://localhost:8000) started", 8000),
            ("Starting development server at http://127.0.0.1:8000/", 8000),
            ("* Listening on http://127.0.0.1:3000", 3000),
            ("Serving \"./\" at http://127.0.0.1:8080", 8080),
            (" * Running on http://127.0.0.1:5000", 5000),
            ("Server running on port 4000", 4000),
            ("listening on port 9229", 9229),
        ];
        for (line, want) in cases {
            assert_eq!(detect_port(line), Some(want), "line: {line}");
        }
    }

    #[test]
    fn ignores_port_conflict_notices() {
        assert_eq!(detect_port("Port 5173 is in use, trying another one..."), None);
        assert_eq!(detect_port("Error: listen EADDRINUSE: address already in use :::3000"), None);
    }

    #[test]
    fn ignores_non_loopback_urls() {
        assert_eq!(detect_port("See https://vitejs.dev:443/guide for help"), None);
        assert_eq!(detect_port("  Network: http://192.168.1.8:5173/"), None);
    }

    #[test]
    fn ignores_ordinary_output() {
        assert_eq!(detect_port("compiled successfully in 412ms"), None);
        assert_eq!(detect_port(""), None);
    }
}
