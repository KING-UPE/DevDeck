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

use crate::auth::{self, Visibility};
use crate::cloud;
use crate::db::Db;
use crate::livereload::{self, Reloader};
use crate::processes::Registry;
use axum::{
    body::Body,
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

/// How the gateway asks the app to stop or restart a process.
///
/// Callbacks rather than a Tauri handle: the gateway has no business knowing
/// what kind of application hosts it, and linking the desktop runtime into this
/// module also breaks the test binary.
#[derive(Clone)]
pub struct Controls {
    pub stop: Arc<dyn Fn(String) + Send + Sync>,
    pub restart: Arc<dyn Fn(String) -> Result<(), String> + Send + Sync>,
    pub start: Arc<dyn Fn(String) -> Result<(), String> + Send + Sync>,
    /// Publish one preview port and return the public URL it is reachable at.
    pub share: Arc<dyn Fn(u16) -> Result<String, String> + Send + Sync>,
}

/// A single project's preview server.
struct Preview {
    port: u16,
    /// Held so the file watcher lives exactly as long as the preview does.
    _reloader: Option<Arc<Reloader>>,
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
    db: Arc<Mutex<Db>>,
    control_shutdown: Option<oneshot::Sender<()>>,
    control_port: u16,
}

#[derive(Clone)]
struct ControlState {
    token: String,
    processes: Registry,
    /// Absent in tests, where there is no app to drive.
    controls: Option<Controls>,
    previews: Arc<Mutex<HashMap<String, Preview>>>,
    lan_ip: Option<IpAddr>,
    db: Arc<Mutex<Db>>,
}

/// State for one project's preview listener.
#[derive(Clone)]
struct PreviewState {
    project_key: String,
    db: Arc<Mutex<Db>>,
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
    pub fn start(
        db: Arc<Mutex<Db>>,
        processes: Registry,
        controls: Controls,
    ) -> Result<Self, String> {
        Self::start_on(
            IpAddr::from([0, 0, 0, 0]),
            CONTROL_PORT,
            db,
            processes,
            Some(controls),
        )
    }

    pub fn start_on(
        bind: IpAddr,
        control_port: u16,
        db: Arc<Mutex<Db>>,
        processes: Registry,
        controls: Option<Controls>,
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
            processes: processes.clone(),
            controls,
            previews: previews.clone(),
            lan_ip: ip,
            db: db.clone(),
        };

        // The shell, its assets and the session endpoints stay public: the
        // page must load in order to render a login form, and a service worker
        // that 401s would break installability.
        let protected = Router::new()
            .route("/api/previews", get(list_previews))
            .route("/api/logs", get(read_logs))
            .route("/api/logs/poll", get(poll_logs))
            .route("/api/process/stop", axum::routing::post(stop_process))
            .route("/api/process/restart", axum::routing::post(restart_process))
            .route("/api/process/start", axum::routing::post(start_process))
            .route("/api/projects", get(list_projects))
            .route("/api/share", axum::routing::post(share_project))
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
            .route("/api/cloud-login", axum::routing::post(cloud_login))
            .merge(protected)
            .layer(middleware::from_fn(allow_cross_origin))
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
            db,
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
            db: self.db.clone(),
        };
        // A project key is "<path>:<script>"; the script says whether the
        // framework already hot-reloads, and the path is what to watch.
        let (project_path, script_name) = match key.rfind(':') {
            Some(i) if i > 2 => (&key[..i], &key[i + 1..]),
            _ => (key, ""),
        };

        let reloader = if livereload::needs_injection(script_name) {
            Reloader::watch(std::path::Path::new(project_path))
        } else {
            None
        };

        let proxy: Router = ReverseProxy::new("/", &upstream).into();

        let mut app = Router::new();
        if let Some(r) = reloader.clone() {
            // Registered before the proxy so the project cannot shadow it.
            app = app.route(
                livereload::RELOAD_PATH,
                get(move || {
                    let r = r.clone();
                    async move {
                        if livereload::wait_for_change(r.subscribe()).await {
                            StatusCode::OK
                        } else {
                            StatusCode::NO_CONTENT
                        }
                    }
                }),
            );
        }
        let mut app = app.merge(proxy);

        if reloader.is_some() {
            app = app.layer(middleware::from_fn(inject_livereload));
        }

        // Capture the per-project state in a closure rather than using
        // `from_fn_with_state`: the proxy router's state parameter is `()`, and
        // the `State<_>` extractor cannot infer its tuple through that.
        let app = app.layer(middleware::from_fn(move |req: Request, next: Next| {
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
                _reloader: reloader,
                shutdown: Some(tx),
            },
        );
        Ok(port)
    }

/// The live preview list, for the desktop UI.
    ///
    /// Read straight from state rather than over HTTP: the desktop webview has
    /// a different origin to the gateway, so fetching its own API is blocked by
    /// CORS and looks like the gateway is unreachable.
    pub fn preview_list(&self) -> Vec<(String, u16, String)> {
        let host = self
            .lan_ip
            .map(|i| i.to_string())
            .unwrap_or_else(|| "127.0.0.1".into());
        self.previews
            .lock()
            .unwrap()
            .iter()
            .map(|(k, p)| (k.clone(), p.port, format!("http://{host}:{}", p.port)))
            .collect()
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

/// Give a project's preview its own public address.
///
/// A quick tunnel maps one local port to one hostname, so the control plane's
/// tunnel cannot carry previews too. Path-prefixing them under it is not an
/// option either: dev servers request their assets from absolute paths like
/// `/assets/app.js`, which no `<base>` tag redirects. Each previewed project
/// therefore gets its own tunnel, opened on demand rather than up front.
async fn share_project(State(st): State<ControlState>, Query(q): Query<KeyQuery>) -> Response {
    let Some(c) = st.controls.clone() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "not available").into_response();
    };

    let Some(port) = st.previews.lock().unwrap().get(&q.key).map(|p| p.port) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "That script is not running." })),
        )
            .into_response();
    };

    // Opening a tunnel blocks while the service assigns a hostname.
    let result = tokio::task::spawn_blocking(move || (c.share)(port)).await;

    match result {
        Ok(Ok(url)) => Json(serde_json::json!({ "url": url })).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": e }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}


/// Allow a packaged app to call this gateway from its own origin.
///
/// Deliberately not credentialed: `*` and cookies are mutually exclusive by
/// spec, and auth travels as a bearer token instead. Reaching an endpoint
/// still requires a valid session, so a wide origin grants nothing on its own.
async fn allow_cross_origin(req: Request, next: Next) -> Response {
    let preflight = req.method() == axum::http::Method::OPTIONS;

    let mut res = if preflight {
        // Answer the preflight here; routing it would 405.
        Response::new(Body::empty())
    } else {
        next.run(req).await
    };

    let h = res.headers_mut();
    h.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        axum::http::HeaderValue::from_static("*"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        axum::http::HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        axum::http::HeaderValue::from_static("Authorization, Content-Type"),
    );
    h.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        axum::http::HeaderValue::from_static("86400"),
    );
    res
}


#[derive(Serialize)]
struct ProjectScript {
    name: String,
    /// Port this script's server is on, when it is running.
    port: Option<u16>,
    url: Option<String>,
    running: bool,
}

#[derive(Serialize)]
struct ProjectRow {
    path: String,
    name: String,
    kind: String,
    visibility: String,
    scripts: Vec<ProjectScript>,
    /// True when any of its scripts is running.
    running: bool,
}

/// Every project the desktop has scanned, whether running or not.
///
/// This is what lets the phone start something rather than only watch what was
/// already started at the desk.
async fn list_projects(State(st): State<ControlState>) -> Json<Vec<ProjectRow>> {
    let host = st
        .lan_ip
        .map(|i| i.to_string())
        .unwrap_or_else(|| "127.0.0.1".into());

    // Which process keys currently have a preview, and on what port.
    let live: HashMap<String, u16> = st
        .previews
        .lock()
        .unwrap()
        .iter()
        .map(|(k, p)| (k.clone(), p.port))
        .collect();

    let db = st.db.lock().unwrap();
    let mut rows: Vec<ProjectRow> = db
        .projects()
        .unwrap_or_default()
        .into_iter()
        .filter(|p| !p.hidden && !p.scripts.is_empty())
        .map(|p| {
            let mut scripts: Vec<ProjectScript> = p
                .scripts
                .keys()
                .map(|name| {
                    let key = format!("{}:{}", p.path, name);
                    let port = live.get(&key).copied();
                    ProjectScript {
                        name: name.clone(),
                        url: port.map(|n| format!("http://{host}:{n}")),
                        running: port.is_some(),
                        port,
                    }
                })
                .collect();
            scripts.sort_by(|a, b| a.name.cmp(&b.name));

            let display = p.custom_name.clone().unwrap_or_else(|| {
                p.path
                    .trim_end_matches(['/', '\\'])
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or(&p.path)
                    .to_string()
            });

            ProjectRow {
                running: scripts.iter().any(|s| s.running),
                path: p.path,
                name: display,
                kind: p.project_type,
                visibility: p.visibility,
                scripts,
            }
        })
        .collect();

    // Running first, then alphabetical: what is live is what you came for.
    rows.sort_by(|a, b| b.running.cmp(&a.running).then(a.name.cmp(&b.name)));
    Json(rows)
}

async fn start_process(State(st): State<ControlState>, Query(q): Query<KeyQuery>) -> Response {
    let Some(c) = st.controls.clone() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "not available").into_response();
    };
    let key = q.key.clone();
    let result = tokio::task::spawn_blocking(move || (c.start)(key)).await;

    match result {
        Ok(Ok(())) => Json(serde_json::json!({ "ok": true })).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}


#[derive(serde::Deserialize)]
struct CloudLoginBody {
    access_token: String,
}

/// Trade a cloud access token for a local session.
///
/// This is what makes one login enough: the phone signs in to the cloud
/// account, and the machine independently confirms with the cloud service that
/// the token is genuine *and* belongs to this machine's owner before letting it
/// in. Verification goes to the service rather than checking a signature here,
/// so no signing secret has to live on the user's machine.
async fn cloud_login(State(st): State<ControlState>, Json(body): Json<CloudLoginBody>) -> Response {
    let (cfg, owner) = {
        let db = st.db.lock().unwrap();
        match (cloud::config(&db), cloud::owner(&db)) {
            (Some(c), Some(o)) => (c, o),
            _ => {
                return (
                    StatusCode::NOT_IMPLEMENTED,
                    Json(serde_json::json!({ "error": "This machine is not attached to a cloud account." })),
                )
                    .into_response()
            }
        }
    };

    let token = body.access_token;
    let verified = tokio::task::spawn_blocking(move || cloud::verify_token(&cfg, &token)).await;

    let user_id = match verified {
        Ok(Ok(id)) => id,
        Ok(Err(e)) => {
            return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": e }))).into_response()
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };

    if user_id != owner {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "That account does not own this machine." })),
        )
            .into_response();
    }

    let session = match auth::mint_session(&st.db.lock().unwrap()) {
        Ok(t) => t,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e })))
                .into_response()
        }
    };

    (
        [(
            header::SET_COOKIE,
            format!("{SESSION_COOKIE}={session}; Path=/; Max-Age=31536000; SameSite=Lax"),
        )],
        Json(serde_json::json!({ "ok": true, "token": session })),
    )
        .into_response()
}


/// A process key carries drive letters and colons, so it travels as a query
/// parameter rather than a path segment that would need escaping.
#[derive(serde::Deserialize)]
struct LogQuery {
    key: String,
    /// Return only lines newer than this sequence number.
    after: Option<u64>,
}

async fn read_logs(State(st): State<ControlState>, Query(q): Query<LogQuery>) -> Response {
    Json(st.processes.since(&q.key, q.after)).into_response()
}

/// Wait for new output, then return it.
///
/// Long-polling rather than a WebSocket: it survives a phone changing networks,
/// and needs nothing on the page beyond `fetch`.
async fn poll_logs(State(st): State<ControlState>, Query(q): Query<LogQuery>) -> Response {
    // Anything already buffered is returned immediately.
    let existing = st.processes.since(&q.key, q.after);
    if !existing.is_empty() {
        return Json(existing).into_response();
    }

    let mut rx = st.processes.subscribe();
    let deadline = tokio::time::Duration::from_secs(25);
    let wanted = q.key.clone();

    let woke = tokio::time::timeout(deadline, async {
        loop {
            match rx.recv().await {
                Ok(key) if key == wanted => return true,
                // Lagging means output was missed, which is still news.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => return true,
                Err(_) => return false,
                _ => continue,
            }
        }
    })
    .await
    .unwrap_or(false);

    if woke {
        Json(st.processes.since(&q.key, q.after)).into_response()
    } else {
        Json(Vec::<crate::processes::LogLine>::new()).into_response()
    }
}

#[derive(serde::Deserialize)]
struct KeyQuery {
    key: String,
}

async fn stop_process(State(st): State<ControlState>, Query(q): Query<KeyQuery>) -> Response {
    let Some(c) = st.controls.clone() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "not available").into_response();
    };
    let key = q.key.clone();
    // Killing a process tree blocks, so it must not run on the async runtime.
    let _ = tokio::task::spawn_blocking(move || (c.stop)(key)).await;
    Json(serde_json::json!({ "ok": true })).into_response()
}

async fn restart_process(State(st): State<ControlState>, Query(q): Query<KeyQuery>) -> Response {
    let Some(c) = st.controls.clone() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "not available").into_response();
    };
    let key = q.key.clone();
    let result = tokio::task::spawn_blocking(move || (c.restart)(key)).await;

    match result {
        Ok(Ok(())) => Json(serde_json::json!({ "ok": true })).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}


/// Append the live-reload client to HTML the project serves.
///
/// Only touches `text/html`, and skips anything compressed: rewriting an
/// encoded body would corrupt it, and these dev servers do not compress by
/// default anyway.
async fn inject_livereload(req: Request, next: Next) -> Response {
    let res = next.run(req).await;
    let (mut parts, body) = res.into_parts();

    let is_html = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.starts_with("text/html"))
        .unwrap_or(false);
    let encoded = parts.headers.contains_key(header::CONTENT_ENCODING);

    if !is_html || encoded {
        return Response::from_parts(parts, body);
    }

    // 8 MiB is far beyond any hand-written page; past that, pass it through
    // untouched rather than buffering without limit.
    let Ok(bytes) = axum::body::to_bytes(body, 8 * 1024 * 1024).await else {
        return Response::from_parts(parts, Body::empty());
    };

    let injected = livereload::inject(&String::from_utf8_lossy(&bytes));
    // The body grew, so the upstream's length no longer describes it.
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(injected))
}


/// Read the session cookie out of a request.
fn session_cookie(req: &Request) -> Option<String> {
    let raw = req.headers().get(header::COOKIE)?.to_str().ok()?;
    raw.split(';')
        .filter_map(|c| c.trim().split_once('='))
        .find(|(k, _)| *k == SESSION_COOKIE)
        .map(|(_, v)| v.to_string())
}

/// Read a session token from an `Authorization: Bearer` header.
///
/// The packaged app runs on its own origin, so its requests are cross-site and
/// a cookie would be dropped. A bearer token travels in a header instead,
/// which also means the API never has to opt into credentialed CORS.
fn bearer_token(req: &Request) -> Option<String> {
    let raw = req.headers().get(header::AUTHORIZATION)?.to_str().ok()?;
    raw.strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Whichever the caller supplied.
fn request_session(req: &Request) -> Option<String> {
    bearer_token(req).or_else(|| session_cookie(req))
}

/// Gate the control plane's data endpoints on a valid session.
async fn require_session(
    State(st): State<ControlState>,
    req: Request,
    next: Next,
) -> Response {
    let ok = request_session(&req)
        .map(|t| auth::is_valid_session(&st.db.lock().unwrap(), &t))
        .unwrap_or(false);

    // With no account configured the gateway is LAN-convenience only, so the
    // pairing token alone is enough; demanding a login the user never created
    // would lock them out of their own machine.
    let no_account = !auth::has_account(&st.db.lock().unwrap());

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
        let db = st.db.lock().unwrap();
        auth::visibility_of(&db, &st.project_key) == Visibility::Public || !auth::has_account(&db)
    };
    if allow_anonymous {
        return next.run(req).await;
    }

    let ok = {
        let db = st.db.lock().unwrap();
        request_session(&req)
            .map(|t| auth::is_valid_session(&db, &t))
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
    /// Present when this machine is attached to a cloud account, so the phone
    /// can offer a cloud sign-in. Both values are public by design: the anon
    /// key ships in every Supabase client and Row Level Security is what
    /// protects the data.
    cloud: Option<CloudHint>,
}

#[derive(Serialize)]
struct CloudHint {
    url: String,
    anon_key: String,
}

async fn session_info(State(st): State<ControlState>, req: Request) -> Json<SessionInfo> {
    let db = st.db.lock().unwrap();
    let authenticated = request_session(&req)
        .map(|t| auth::is_valid_session(&db, &t))
        .unwrap_or(false);
    // Only advertise cloud sign-in once a machine has an owner; before that
    // there is no account to match a token against.
    let cloud = cloud::config(&db)
        .filter(|_| cloud::owner(&db).is_some())
        .map(|c| CloudHint {
            url: c.url,
            anon_key: c.anon_key,
        });

    Json(SessionInfo {
        authenticated,
        has_account: auth::has_account(&db),
        username: auth::username(&db),
        cloud,
    })
}

#[derive(serde::Deserialize)]
struct LoginBody {
    username: String,
    password: String,
}

async fn login(State(st): State<ControlState>, Json(body): Json<LoginBody>) -> Response {
    match auth::login(&st.db.lock().unwrap(), &body.username, &body.password) {
        Ok(token) => (
            [(
                header::SET_COOKIE,
                format!("{SESSION_COOKIE}={token}; Path=/; Max-Age=31536000; SameSite=Lax"),
            )],
            // The packaged app keeps this and sends it as a bearer token; the
            // browser build ignores it and uses the cookie.
            Json(serde_json::json!({ "ok": true, "token": token })),
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
            let session = auth::mint_session(&st.db.lock().unwrap()).unwrap_or_default();
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

    const CRLF: &str = "\r\n";

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

    /// Upstream that answers with an HTML document.
    fn spawn_html_upstream() -> u16 {
        let l = StdListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in l.incoming().take(8) {
                let Ok(mut s) = stream else { continue };
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let body = "<html><body><h1>static site</h1></body></html>";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });
        port
    }

    fn test_gateway() -> Gateway {
        // Loopback + port 0 keeps the test off the wildcard address, which would
        // otherwise raise a Windows Firewall prompt on every run.
        Gateway::start_on(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            0,
            Arc::new(Mutex::new(Db::open_in_memory().unwrap())),
            Registry::default(),
            None,
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

    /// A gateway backed by an in-memory database with an account configured.
    fn gateway_with_account() -> (Gateway, Arc<Mutex<Db>>) {
        let db = Arc::new(Mutex::new(Db::open_in_memory().unwrap()));
        auth::set_account(&db.lock().unwrap(), "upe", "correct-horse").unwrap();
        let gw = Gateway::start_on(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            0,
            db.clone(),
            Registry::default(),
            None,
        )
        .unwrap();
        (gw, db)
    }

    #[test]
    fn a_private_project_refuses_anonymous_viewers() {
        let (gw, _db) = gateway_with_account();
        let port = gw.add_preview("secret:dev", spawn_upstream("top secret")).unwrap();

        let resp = get(port, "/");
        assert!(resp.contains("401"), "private project was served: {resp}");
        assert!(!resp.contains("top secret"), "private body leaked: {resp}");
    }

    #[test]
    fn a_signed_in_viewer_reaches_a_private_project() {
        let (gw, db) = gateway_with_account();
        let port = gw.add_preview("secret:dev", spawn_upstream("top secret")).unwrap();
        let token = auth::login(&db.lock().unwrap(), "upe", "correct-horse").unwrap();

        let resp = get_auth(port, "/", &token);
        assert!(resp.contains("200 OK"), "signed-in viewer blocked: {resp}");
        assert!(resp.contains("top secret"), "body missing: {resp}");
    }

    #[test]
    fn a_public_project_is_viewable_without_signing_in() {
        let (gw, db) = gateway_with_account();
        auth::set_visibility(&db.lock().unwrap(), "demo:dev", Visibility::Public).unwrap();
        let port = gw.add_preview("demo:dev", spawn_upstream("client demo")).unwrap();

        let resp = get(port, "/");
        assert!(resp.contains("200 OK"), "public project blocked: {resp}");
        assert!(resp.contains("client demo"));
    }

    #[test]
    fn a_stale_session_stops_working_after_a_password_change() {
        let (gw, db) = gateway_with_account();
        let port = gw.add_preview("secret:dev", spawn_upstream("top secret")).unwrap();
        let token = auth::login(&db.lock().unwrap(), "upe", "correct-horse").unwrap();
        assert!(get_auth(port, "/", &token).contains("200 OK"));

        auth::set_account(&db.lock().unwrap(), "upe", "a-brand-new-one").unwrap();
        let resp = get_auth(port, "/", &token);
        assert!(resp.contains("401"), "revoked session still worked: {resp}");
    }

    #[test]
    fn the_project_list_needs_a_session_once_an_account_exists() {
        let (gw, db) = gateway_with_account();
        gw.add_preview("secret:dev", spawn_upstream("x")).unwrap();

        let anon = get(gw.control_port, "/api/previews");
        assert!(anon.contains("401"), "project list leaked anonymously: {anon}");

        let token = auth::login(&db.lock().unwrap(), "upe", "correct-horse").unwrap();
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
    fn a_static_project_gets_the_live_reload_client() {
        let gw = test_gateway();
        let dir = std::env::temp_dir();
        let key = format!("{}:live server", dir.display());
        let port = gw.add_preview(&key, spawn_html_upstream()).unwrap();

        let resp = get(port, "/");
        assert!(resp.contains("200 OK"), "not served: {resp}");
        assert!(resp.contains("static site"), "original content lost");
        assert!(
            resp.contains(crate::livereload::RELOAD_PATH),
            "live-reload client was not injected: {resp}"
        );
    }

    #[test]
    fn a_framework_project_is_left_alone() {
        // Vite ships its own HMR; injecting would cause double reloads.
        let gw = test_gateway();
        let key = format!("{}:dev", std::env::temp_dir().display());
        let port = gw.add_preview(&key, spawn_html_upstream()).unwrap();

        let resp = get(port, "/");
        assert!(resp.contains("static site"));
        assert!(
            !resp.contains(crate::livereload::RELOAD_PATH),
            "injected into a project that hot-reloads itself"
        );
    }

    #[test]
    fn injection_does_not_touch_non_html_responses() {
        let gw = test_gateway();
        let key = format!("{}:live server", std::env::temp_dir().display());
        // spawn_upstream serves text/plain.
        let port = gw.add_preview(&key, spawn_upstream("body { color: red }")).unwrap();

        let resp = get(port, "/site.css");
        assert!(resp.contains("body { color: red }"));
        assert!(!resp.contains(crate::livereload::RELOAD_PATH), "injected into CSS");
    }

/// A gateway sharing one registry with the test, so output can be staged.
    fn gateway_with_logs() -> (Gateway, Registry) {
        let reg = Registry::default();
        let gw = Gateway::start_on(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            0,
            Arc::new(Mutex::new(Db::open_in_memory().unwrap())),
            reg.clone(),
            None,
        )
        .unwrap();
        (gw, reg)
    }

    #[test]
    fn serves_a_process_log_tail() {
        let (gw, reg) = gateway_with_logs();
        reg.register("proj:dev", "D:/proj", "dev", "npm run dev");
        reg.append("proj:dev", "stderr", "Error: build failed");

        let body = get(gw.control_port, "/api/logs?key=proj%3Adev");
        assert!(body.contains("200 OK"), "logs not served: {body}");
        assert!(body.contains("build failed"), "log line missing: {body}");
        assert!(body.contains("stderr"), "stream not reported: {body}");
    }

    #[test]
    fn a_log_tail_can_resume_from_a_sequence() {
        let (gw, reg) = gateway_with_logs();
        reg.register("proj:dev", "D:/proj", "dev", "npm run dev");
        for i in 0..4 {
            reg.append("proj:dev", "stdout", &format!("line {i}"));
        }

        let body = get(gw.control_port, "/api/logs?key=proj%3Adev&after=2");
        assert!(body.contains("line 3"), "newest line missing: {body}");
        assert!(!body.contains("line 0"), "already-seen line resent: {body}");
    }

    #[test]
    fn logs_for_an_unknown_process_are_empty_not_an_error() {
        let (gw, _reg) = gateway_with_logs();
        let body = get(gw.control_port, "/api/logs?key=ghost%3Adev");
        assert!(body.contains("200 OK"));
        assert!(body.contains("[]"), "expected an empty list: {body}");
    }

    #[test]
    fn process_control_is_refused_when_no_app_is_attached() {
        // The test gateway has no controls, standing in for a gateway whose
        // host has gone away; it must refuse rather than panic.
        let (gw, _reg) = gateway_with_logs();
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, gw.control_port)).unwrap();
        let req = format!(
            "POST /api/process/stop?key=proj%3Adev HTTP/1.1{}Host: 127.0.0.1{}Content-Length: 0{}Connection: close{}{}",
            CRLF, CRLF, CRLF, CRLF, CRLF
        );
        s.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        assert!(out.contains("503"), "expected 503, got: {out}");
    }

#[test]
    fn cloud_sign_in_is_refused_when_no_account_is_attached() {
        // Without an owner there is nothing to match a token against, so the
        // gateway must decline rather than trust the token on its own.
        let gw = test_gateway();
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, gw.control_port)).unwrap();
        let body = r#"{"access_token":"anything"}"#;
        let req = format!(
            "POST /api/cloud-login HTTP/1.1{}Host: 127.0.0.1{}Content-Type: application/json{}Content-Length: {}{}Connection: close{}{}{}",
            CRLF, CRLF, CRLF, body.len(), CRLF, CRLF, CRLF, body
        );
        s.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        assert!(out.contains("501"), "expected 501, got: {out}");
        assert!(out.contains("not attached"), "unhelpful message: {out}");
    }

    #[test]
    fn the_session_endpoint_hides_cloud_details_until_a_machine_is_claimed() {
        let gw = test_gateway();
        let body = get(gw.control_port, "/api/session");
        assert!(body.contains("200 OK"));
        // `cloud` must be null, so the phone does not offer a sign-in that
        // cannot possibly succeed.
        assert!(
            body.contains("\"cloud\":null"),
            "cloud details exposed before an owner was set: {body}"
        );
    }

#[test]
    fn signing_in_to_the_cloud_locks_the_machine_down() {
        // Regression: the "unclaimed machine" escape hatch only looked at the
        // local account table, so a cloud sign-in left every project readable
        // by anyone - over a tunnel, that is the whole internet.
        let db = Arc::new(Mutex::new(Db::open_in_memory().unwrap()));
        crate::cloud::set_owner(&db.lock().unwrap(), "user-uuid-1").unwrap();

        let gw = Gateway::start_on(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            0,
            db.clone(),
            Registry::default(),
            None,
        )
        .unwrap();
        let port = gw.add_preview("secret:dev", spawn_upstream("top secret")).unwrap();

        let preview = get(port, "/");
        assert!(preview.contains("401"), "private project served anonymously: {preview}");
        assert!(!preview.contains("top secret"), "private body leaked");

        let list = get(gw.control_port, "/api/previews");
        assert!(list.contains("401"), "project list served anonymously: {list}");
    }

/// GET with an Authorization header, as the packaged app sends.
    fn get_bearer(port: u16, path: &str, token: &str) -> String {
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        s.write_all(
            format!("GET {path} HTTP/1.1{CRLF}Host: 127.0.0.1{CRLF}Authorization: Bearer {token}{CRLF}Connection: close{CRLF}{CRLF}").as_bytes(),
        )
        .unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out
    }

    #[test]
    fn a_bearer_token_works_where_a_cookie_cannot() {
        // The packaged app is cross-site, so its cookie would be dropped.
        let (gw, db) = gateway_with_account();
        gw.add_preview("secret:dev", spawn_upstream("x")).unwrap();
        let token = auth::login(&db.lock().unwrap(), "upe", "correct-horse").unwrap();

        let ok = get_bearer(gw.control_port, "/api/previews", &token);
        assert!(ok.contains("secret:dev"), "bearer token rejected: {ok}");

        let bad = get_bearer(gw.control_port, "/api/previews", "not-a-session");
        assert!(bad.contains("401"), "any bearer token was accepted: {bad}");
    }

    #[test]
    fn responses_carry_cross_origin_headers() {
        let gw = test_gateway();
        let body = get(gw.control_port, "/api/session");
        assert!(
            body.to_lowercase().contains("access-control-allow-origin: *"),
            "no CORS header, a packaged app could not call this: {body}"
        );
    }

    #[test]
    fn a_preflight_is_answered_rather_than_405() {
        let gw = test_gateway();
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, gw.control_port)).unwrap();
        s.write_all(
            format!("OPTIONS /api/previews HTTP/1.1{CRLF}Host: 127.0.0.1{CRLF}Origin: tauri://localhost{CRLF}Access-Control-Request-Method: GET{CRLF}Connection: close{CRLF}{CRLF}").as_bytes(),
        )
        .unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        assert!(!out.contains("405"), "preflight was rejected: {out}");
        assert!(out.to_lowercase().contains("access-control-allow-headers"), "{out}");
    }

    #[test]
    fn signing_in_returns_a_token_for_the_app_to_keep() {
        let (gw, _db) = gateway_with_account();
        let body = r#"{"username":"upe","password":"correct-horse"}"#;
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, gw.control_port)).unwrap();
        s.write_all(
            format!("POST /api/login HTTP/1.1{CRLF}Host: 127.0.0.1{CRLF}Content-Type: application/json{CRLF}Content-Length: {}{CRLF}Connection: close{CRLF}{CRLF}{}", body.len(), body).as_bytes(),
        )
        .unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        assert!(out.contains("\"token\""), "no token in the login response: {out}");
    }

    #[test]
    fn tokens_are_unique_and_long_enough() {
        let (a, b) = (random_token(), random_token());
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
    }
}
