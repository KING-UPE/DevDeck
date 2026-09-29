//! Embedded HTTP gateway that re-serves local dev servers to the network.
//!
//! Dev servers bind to loopback, so a phone on the same Wi-Fi cannot reach them
//! even though the PC can. The gateway runs *on* the PC: it dials 127.0.0.1
//! itself and re-serves that to 0.0.0.0, which sidesteps the problem without
//! touching a single dev-server flag.
//!
//! # Why a port per project, not a path prefix
//!
//! Serving project A at `/p/a/` and project B at `/p/b/` off one port looks
//! tidier, but it breaks every absolute URL the upstream emits: a bundle that
//! requests `/assets/index.js` would hit the gateway root rather than the
//! project, and rewriting HTML/CSS/JS to compensate is endlessly fragile.
//! Giving each project its own listener keeps the upstream's own URLs correct
//! and costs nothing. Auth still works across all of them because browsers
//! scope cookies by host and *ignore* the port.

use crate::auth::{AuthStore, Visibility};
use axum::{
    extract::{Query, Request, State},
    middleware::{self, Next},
    http::{header, StatusCode, Uri},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
    Json, Router,
};
use axum_reverse_proxy::ReverseProxy;
use serde::Serialize;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

/// Port the control plane listens on. Preview planes are allocated above it.
pub const CONTROL_PORT: u16 = 7420;
const PREVIEW_PORT_BASE: u16 = 7421;
const SESSION_COOKIE: &str = "devdeck_session";

/// A single project's preview server.
struct Preview {
    port: u16,
    /// Dropping or firing this stops the listener.
    shutdown: Option<oneshot::Sender<()>>,
}

/// Everything the running gateway owns.
pub struct Gateway {
    /// Bearer token; also the value of the session cookie.
    pub token: String,
    /// Best-guess LAN address, used for the pairing URL.
    pub lan_ip: Option<IpAddr>,
    /// Interface to bind on. 0.0.0.0 in production; loopback under test,
    /// where binding a wildcard address would trip Windows Firewall.
    bind: IpAddr,
    runtime: Arc<tokio::runtime::Runtime>,
    previews: Arc<Mutex<HashMap<String, Preview>>>,
    auth: Arc<Mutex<AuthStore>>,
    control_shutdown: Option<oneshot::Sender<()>>,
    control_port: u16,
}

#[derive(Clone)]
struct ControlState {
    token: String,
    previews: Arc<Mutex<HashMap<String, Preview>>>,
    lan_ip: Option<IpAddr>,
    auth: Arc<Mutex<AuthStore>>,
}

/// State for one project's preview listener.
#[derive(Clone)]
struct PreviewState {
    project_key: String,
    auth: Arc<Mutex<AuthStore>>,
}

/// What the desktop UI needs to render the pairing panel.
#[derive(Serialize, Clone)]
pub struct GatewayInfo {
    pub running: bool,
    pub lan_url: Option<String>,
    pub pair_url: Option<String>,
    pub control_port: u16,
    pub token: String,
}

#[derive(Serialize)]
struct PreviewEntry {
    key: String,
    port: u16,
    url: String,
}

/// Generate a URL-safe random token without pulling in a crypto dependency.
///
/// This gates LAN access only; it is a speed bump against another device on the
/// same Wi-Fi, not a defence against an attacker who can already run code on the
/// machine. Anything internet-facing goes through the tunnel's own auth.
fn random_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::with_capacity(32);
    while out.len() < 32 {
        let n = RandomState::new().build_hasher().finish();
        out.push_str(&format!("{n:016x}"));
    }
    out.truncate(32);
    out
}

fn lan_ip() -> Option<IpAddr> {
    local_ip_address::local_ip().ok()
}

impl Gateway {
    /// Bind the control plane and return a handle. Preview planes start later,
    /// as ports are detected.
    pub fn start(auth: Arc<Mutex<AuthStore>>) -> Result<Self, String> {
        Self::start_on(IpAddr::from([0, 0, 0, 0]), CONTROL_PORT, auth)
    }

    pub fn start_on(
        bind: IpAddr,
        control_port: u16,
        auth: Arc<Mutex<AuthStore>>,
    ) -> Result<Self, String> {
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .thread_name("devdeck-gateway")
                .build()
                .map_err(|e| format!("could not start gateway runtime: {e}"))?,
        );

        let token = random_token();
        let previews: Arc<Mutex<HashMap<String, Preview>>> = Arc::new(Mutex::new(HashMap::new()));
        let ip = lan_ip();

        let state = ControlState {
            token: token.clone(),
            previews: previews.clone(),
            lan_ip: ip,
            auth: auth.clone(),
        };

        // The shell, its assets and the session endpoints stay public: the
        // page must load in order to render a login form, and a service worker
        // that 401s would break installability.
        let protected = Router::new()
            .route("/api/previews", get(list_previews))
            .layer(middleware::from_fn_with_state(state.clone(), require_session));

        let app = Router::new()
            .route("/", get(index))
            .route("/manifest.webmanifest", get(manifest))
            .route("/sw.js", get(service_worker))
            .route("/icon-256.png", get(icon_256))
            .route("/icon-512.png", get(icon_512))
            .route("/api/health", get(|| async { "ok" }))
            .route("/api/session", get(session_info))
            .route("/api/login", axum::routing::post(login))
            .merge(protected)
            .with_state(state);

        let (tx, rx) = oneshot::channel::<()>();
        let addr = SocketAddr::new(bind, control_port);

        let listener = runtime
            .block_on(tokio::net::TcpListener::bind(addr))
            .map_err(|e| format!("port {control_port} unavailable: {e}"))?;

        // Port 0 means "pick one for me", so read back what was actually bound;
        // storing the requested value would report 0 to the UI.
        let control_port = listener
            .local_addr()
            .map(|a| a.port())
            .unwrap_or(control_port);

        runtime.spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await;
        });

        Ok(Gateway {
            bind,
            auth,
            token,
            lan_ip: ip,
            runtime,
            previews,
            control_shutdown: Some(tx),
            control_port,
        })
    }

    pub fn info(&self) -> GatewayInfo {
        let base = self
            .lan_ip
            .map(|ip| format!("http://{ip}:{}", self.control_port));
        GatewayInfo {
            running: true,
            pair_url: base.as_ref().map(|b| format!("{b}/?t={}", self.token)),
            lan_url: base,
            control_port: self.control_port,
            token: self.token.clone(),
        }
    }

    /// Stand up a preview server for `key` pointing at `upstream_port`.
    ///
    /// Re-registering an existing key replaces the old listener, which is what
    /// happens when a dev server restarts on a different port.
    pub fn add_preview(&self, key: &str, upstream_port: u16) -> Result<u16, String> {
        self.remove_preview(key);

        let upstream = format!("http://127.0.0.1:{upstream_port}");
        // Without this layer any device on the network - or anyone holding a
        // tunnel URL - could read the dev server directly.
        let pstate = PreviewState {
            project_key: key.to_string(),
            auth: self.auth.clone(),
        };
        let proxy: Router = ReverseProxy::new("/", &upstream).into();
        // Capture the per-project state in a closure rather than using
        // `from_fn_with_state`: the proxy router's state parameter is `()`, and
        // the `State<_>` extractor cannot infer its tuple through that.
        let app = proxy.layer(middleware::from_fn(move |req: Request, next: Next| {
            let st = pstate.clone();
            async move { require_preview_access(st, req, next).await }
        }));

        // Claim the lowest free port by *actually binding* rather than scanning
        // a bookkeeping map: another process on this machine may hold a port we
        // have never handed out, and only bind() knows that.
        let (listener, port) = {
            let mut found = None;
            for candidate in PREVIEW_PORT_BASE..PREVIEW_PORT_BASE + 200 {
                let addr = SocketAddr::new(self.bind, candidate);
                if let Ok(l) = self.runtime.block_on(tokio::net::TcpListener::bind(addr)) {
                    found = Some((l, candidate));
                    break;
                }
            }
            found.ok_or_else(|| "no free preview port in 7421-7620".to_string())?
        };

        let (tx, rx) = oneshot::channel::<()>();
        self.runtime.spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await;
        });

        self.previews.lock().unwrap().insert(
            key.to_string(),
            Preview {
                port,
                shutdown: Some(tx),
            },
        );
        Ok(port)
    }

    pub fn remove_preview(&self, key: &str) {
        if let Some(mut p) = self.previews.lock().unwrap().remove(key) {
            if let Some(tx) = p.shutdown.take() {
                let _ = tx.send(());
            }
        }
    }

    pub fn shutdown(&mut self) {
        let keys: Vec<String> = self.previews.lock().unwrap().keys().cloned().collect();
        for k in keys {
            self.remove_preview(&k);
        }
        if let Some(tx) = self.control_shutdown.take() {
            let _ = tx.send(());
        }
    }
}

/// Render the pairing URL as an inline SVG QR code.
///
/// Returned as SVG rather than a PNG so the desktop UI can drop it straight
/// into the DOM and have it stay crisp at any size.
pub fn qr_svg(data: &str) -> Result<String, String> {
    use qrcode::render::svg;
    use qrcode::{EcLevel, QrCode};

    let code = QrCode::with_error_correction_level(data.as_bytes(), EcLevel::M)
        .map_err(|e| format!("could not encode pairing QR: {e}"))?;

    Ok(code
        .render::<svg::Color>()
        .min_dimensions(220, 220)
        .quiet_zone(true)
        .dark_color(svg::Color("#050914"))
        .light_color(svg::Color("#ffffff"))
        .build())
}

// ---------------------------------------------------------------- handlers

/// Read the session cookie out of a request.
fn session_cookie(req: &Request) -> Option<String> {
    let raw = req.headers().get(header::COOKIE)?.to_str().ok()?;
    raw.split(';')
        .filter_map(|c| c.trim().split_once('='))
        .find(|(k, _)| *k == SESSION_COOKIE)
        .map(|(_, v)| v.to_string())
}

/// Gate the control plane's data endpoints on a valid session.
async fn require_session(
    State(st): State<ControlState>,
    req: Request,
    next: Next,
) -> Response {
    let ok = session_cookie(&req)
        .map(|t| st.auth.lock().unwrap().is_valid_session(&t))
        .unwrap_or(false);

    // With no account configured the gateway is LAN-convenience only, so the
    // pairing token alone is enough; demanding a login the user never created
    // would lock them out of their own machine.
    let no_account = !st.auth.lock().unwrap().has_account();

    if ok || no_account {
        next.run(req).await
    } else {
        (StatusCode::UNAUTHORIZED, "Sign in to view your projects").into_response()
    }
}

/// Gate one project's preview, unless the user marked it public.
async fn require_preview_access(st: PreviewState, req: Request, next: Next) -> Response {
    // Resolve every decision that needs the lock *before* any await: holding a
    // std MutexGuard across one makes the future non-Send, and axum requires
    // Send futures.
    let allow_anonymous = {
        let auth = st.auth.lock().unwrap();
        auth.visibility_of(&st.project_key) == Visibility::Public || !auth.has_account()
    };
    if allow_anonymous {
        return next.run(req).await;
    }

    let ok = {
        let auth = st.auth.lock().unwrap();
        session_cookie(&req)
            .map(|t| auth.is_valid_session(&t))
            .unwrap_or(false)
    };

    if ok {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            "This project is private. Sign in to DevDeck to view it.",
        )
            .into_response()
    }
}

#[derive(Serialize)]
struct SessionInfo {
    authenticated: bool,
    has_account: bool,
    username: Option<String>,
}

async fn session_info(State(st): State<ControlState>, req: Request) -> Json<SessionInfo> {
    let auth = st.auth.lock().unwrap();
    let authenticated = session_cookie(&req)
        .map(|t| auth.is_valid_session(&t))
        .unwrap_or(false);
    Json(SessionInfo {
        authenticated,
        has_account: auth.has_account(),
        username: auth.username().map(str::to_string),
    })
}

#[derive(serde::Deserialize)]
struct LoginBody {
    username: String,
    password: String,
}

async fn login(State(st): State<ControlState>, Json(body): Json<LoginBody>) -> Response {
    match st.auth.lock().unwrap().login(&body.username, &body.password) {
        Ok(token) => (
            [(
                header::SET_COOKIE,
                format!("{SESSION_COOKIE}={token}; Path=/; Max-Age=31536000; SameSite=Lax"),
            )],
            Json(serde_json::json!({ "ok": true })),
        )
            .into_response(),
        Err(e) => (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": e }))).into_response(),
    }
}


#[derive(serde::Deserialize)]
struct AuthQuery {
    t: Option<String>,
}

/// Serve the remote UI. A `?t=` token (as encoded in the pairing QR) is
/// exchanged for a session cookie so later asset requests carry it implicitly.
async fn index(State(st): State<ControlState>, Query(q): Query<AuthQuery>) -> Response {
    if let Some(t) = q.t {
        if t == st.token {
            // Trade the pairing token for a real session, so revoking sessions
            // logs paired phones out without having to rotate the QR.
            let session = st.auth.lock().unwrap().mint_session();
            return (
                [(
                    header::SET_COOKIE,
                    format!("{SESSION_COOKIE}={session}; Path=/; Max-Age=31536000; SameSite=Lax"),
                )],
                Redirect::to("/"),
            )
                .into_response();
        }
        return (StatusCode::UNAUTHORIZED, "Invalid pairing token").into_response();
    }
    Html(include_str!("../ui/remote.html")).into_response()
}

/// Static assets for the installable web app. Embedded in the binary rather
/// than read from disk so a packaged build has no external file dependencies.
async fn manifest() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/manifest+json")],
        include_str!("../ui/manifest.webmanifest"),
    )
}

async fn service_worker() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/javascript"),
            // Must not be cached by the browser, or a stale worker pins an old
            // shell forever.
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("../ui/sw.js"),
    )
}

async fn icon_256() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/png")],
        include_bytes!("../icons/128x128@2x.png").as_slice(),
    )
}

async fn icon_512() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/png")],
        include_bytes!("../icons/icon.png").as_slice(),
    )
}

async fn list_previews(State(st): State<ControlState>) -> Json<Vec<PreviewEntry>> {
    let host = st
        .lan_ip
        .map(|i| i.to_string())
        .unwrap_or_else(|| "127.0.0.1".into());
    let out = st
        .previews
        .lock()
        .unwrap()
        .iter()
        .map(|(k, p)| PreviewEntry {
            key: k.clone(),
            port: p.port,
            url: format!("http://{host}:{}", p.port),
        })
        .collect();
    Json(out)
}

/// Unused today; kept so the 404 path is explicit once routes grow.
#[allow(dead_code)]
async fn not_found(uri: Uri) -> impl IntoResponse {
    (StatusCode::NOT_FOUND, format!("no route for {uri}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{Ipv4Addr, TcpListener as StdListener, TcpStream};

    /// Minimal upstream that answers one fixed body, standing in for a dev server.
    fn spawn_upstream(body: &'static str) -> u16 {
        let l = StdListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in l.incoming().take(8) {
                let Ok(mut s) = stream else { continue };
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });
        port
    }

    fn get(port: u16, path: &str) -> String {
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        s.write_all(
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out
    }

    /// Byte-level fetch, for responses that are not valid UTF-8 (PNG icons).
    fn get_bytes(port: u16, path: &str) -> Vec<u8> {
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        s.write_all(
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .unwrap();
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
        out
    }

    /// GET with a session cookie attached.
    fn get_auth(port: u16, path: &str, cookie: &str) -> String {
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        s.write_all(
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nCookie: devdeck_session={cookie}\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out
    }

    fn test_gateway() -> Gateway {
        // Loopback + port 0 keeps the test off the wildcard address, which would
        // otherwise raise a Windows Firewall prompt on every run.
        Gateway::start_on(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            0,
            Arc::new(Mutex::new(AuthStore::default())),
        )
        .unwrap()
    }

    #[test]
    fn proxies_an_upstream_dev_server() {
        let upstream = spawn_upstream("hello from vite");
        let gw = test_gateway();
        let port = gw.add_preview("proj:dev", upstream).unwrap();

        let resp = get(port, "/");
        assert!(resp.contains("200 OK"), "no 200 in: {resp}");
        assert!(resp.contains("hello from vite"), "body missing in: {resp}");
    }

    #[test]
    fn each_project_gets_its_own_port() {
        let gw = test_gateway();
        let a = gw.add_preview("a:dev", spawn_upstream("a")).unwrap();
        let b = gw.add_preview("b:dev", spawn_upstream("b")).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn re_registering_replaces_the_listener() {
        let gw = test_gateway();
        gw.add_preview("a:dev", spawn_upstream("one")).unwrap();
        gw.add_preview("a:dev", spawn_upstream("two")).unwrap();
        assert_eq!(gw.previews.lock().unwrap().len(), 1);
    }

    #[test]
    fn removing_a_preview_forgets_it() {
        let gw = test_gateway();
        gw.add_preview("a:dev", spawn_upstream("x")).unwrap();
        gw.remove_preview("a:dev");
        assert!(gw.previews.lock().unwrap().is_empty());
    }

#[test]
    fn serves_the_installable_web_app() {
        let gw = test_gateway();
        let port = gw.control_port;

        let shell = get(port, "/");
        assert!(shell.contains("200 OK"), "shell not served: {shell}");
        assert!(shell.contains("manifest.webmanifest"), "shell missing manifest link");

        let manifest = get(port, "/manifest.webmanifest");
        assert!(manifest.contains("application/manifest+json"), "wrong manifest type");
        assert!(manifest.contains("\"display\": \"standalone\""), "not installable");

        let sw = get(port, "/sw.js");
        assert!(sw.contains("text/javascript"), "wrong sw type");
        assert!(sw.contains("no-cache"), "sw must not be cached");
    }

    #[test]
    fn serves_icons_for_the_home_screen() {
        let gw = test_gateway();
        for path in ["/icon-256.png", "/icon-512.png"] {
            let raw = get_bytes(gw.control_port, path);
            let head = String::from_utf8_lossy(&raw[..raw.len().min(220)]).to_string();
            assert!(head.contains("200 OK"), "{path} not served: {head}");
            assert!(head.contains("image/png"), "{path} wrong content type: {head}");
            // PNG magic proves the bytes survived the proxy intact.
            assert!(
                raw.windows(3).any(|w| w == b"PNG"),
                "{path} body is not a PNG"
            );
        }
    }

    #[test]
    fn reports_the_port_it_actually_bound() {
        let gw = test_gateway();
        assert_ne!(gw.control_port, 0, "ephemeral bind must report the real port");
        assert!(gw.info().running);
    }

    #[test]
    fn previews_endpoint_lists_registered_projects() {
        let gw = test_gateway();
        gw.add_preview("proj:dev", spawn_upstream("x")).unwrap();
        let body = get(gw.control_port, "/api/previews");
        assert!(body.contains("proj:dev"), "preview missing from list: {body}");
    }

/// Build a gateway whose auth store has a real account configured.
    fn gateway_with_account() -> (Gateway, Arc<Mutex<AuthStore>>) {
        let auth = Arc::new(Mutex::new(AuthStore::default()));
        auth.lock().unwrap().set_account("upe", "correct-horse").unwrap();
        let gw = Gateway::start_on(IpAddr::V4(Ipv4Addr::LOCALHOST), 0, auth.clone()).unwrap();
        (gw, auth)
    }

    #[test]
    fn a_private_project_refuses_anonymous_viewers() {
        let (gw, _auth) = gateway_with_account();
        let port = gw.add_preview("secret:dev", spawn_upstream("top secret")).unwrap();

        let resp = get(port, "/");
        assert!(resp.contains("401"), "private project was served: {resp}");
        assert!(!resp.contains("top secret"), "private body leaked: {resp}");
    }

    #[test]
    fn a_signed_in_viewer_reaches_a_private_project() {
        let (gw, auth) = gateway_with_account();
        let port = gw.add_preview("secret:dev", spawn_upstream("top secret")).unwrap();
        let token = auth.lock().unwrap().login("upe", "correct-horse").unwrap();

        let resp = get_auth(port, "/", &token);
        assert!(resp.contains("200 OK"), "signed-in viewer blocked: {resp}");
        assert!(resp.contains("top secret"), "body missing: {resp}");
    }

    #[test]
    fn a_public_project_is_viewable_without_signing_in() {
        let (gw, auth) = gateway_with_account();
        auth.lock().unwrap().set_visibility("demo:dev", Visibility::Public);
        let port = gw.add_preview("demo:dev", spawn_upstream("client demo")).unwrap();

        let resp = get(port, "/");
        assert!(resp.contains("200 OK"), "public project blocked: {resp}");
        assert!(resp.contains("client demo"));
    }

    #[test]
    fn a_stale_session_stops_working_after_a_password_change() {
        let (gw, auth) = gateway_with_account();
        let port = gw.add_preview("secret:dev", spawn_upstream("top secret")).unwrap();
        let token = auth.lock().unwrap().login("upe", "correct-horse").unwrap();
        assert!(get_auth(port, "/", &token).contains("200 OK"));

        auth.lock().unwrap().set_account("upe", "a-brand-new-one").unwrap();
        let resp = get_auth(port, "/", &token);
        assert!(resp.contains("401"), "revoked session still worked: {resp}");
    }

    #[test]
    fn the_project_list_needs_a_session_once_an_account_exists() {
        let (gw, auth) = gateway_with_account();
        gw.add_preview("secret:dev", spawn_upstream("x")).unwrap();

        let anon = get(gw.control_port, "/api/previews");
        assert!(anon.contains("401"), "project list leaked anonymously: {anon}");

        let token = auth.lock().unwrap().login("upe", "correct-horse").unwrap();
        let signed = get_auth(gw.control_port, "/api/previews", &token);
        assert!(signed.contains("secret:dev"), "signed-in list empty: {signed}");
    }

    #[test]
    fn without_an_account_the_gateway_stays_lan_convenient() {
        // No account configured: demanding a login nobody created would lock
        // the user out of their own machine.
        let gw = test_gateway();
        let port = gw.add_preview("proj:dev", spawn_upstream("hello")).unwrap();
        assert!(get(port, "/").contains("200 OK"));
        assert!(get(gw.control_port, "/api/previews").contains("proj:dev"));
    }

    #[test]
    fn tokens_are_unique_and_long_enough() {
        let (a, b) = (random_token(), random_token());
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
    }
}
