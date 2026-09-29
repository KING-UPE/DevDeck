mod auth;
mod gateway;
mod tunnel;
mod ports;

use std::process::{Command, Stdio};
use std::io::{BufRead, BufReader};
use std::fs;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use std::thread;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, AppHandle, State, Manager};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

trait CommandExtCrossPlatform {
    fn apply_cross_platform_flags(&mut self) -> &mut Self;
}

impl CommandExtCrossPlatform for std::process::Command {
    fn apply_cross_platform_flags(&mut self) -> &mut Self {
        #[cfg(target_os = "windows")]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            self.creation_flags(CREATE_NO_WINDOW);
        }
        self
    }
}


#[derive(Serialize, Deserialize, Clone)]
struct ProjectInfo {
    path: String,
    name: String,
    project_type: String,
    scripts: HashMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone)]
struct NodeProcess {
    pid: u32,
    command: String,
    projectPath: String,
    #[serde(rename = "type")]
    process_type: String,
}

struct AppState {
    active_processes: Arc<Mutex<HashMap<String, std::process::Child>>>,
    /// Port each running dev server was seen binding to, keyed by process key.
    /// Populated by scraping process output; see [`ports::detect_port`].
    detected_ports: Arc<Mutex<HashMap<String, u16>>>,
    /// The mobile gateway, once started. `None` until the user turns it on.
    gateway: Arc<Mutex<Option<gateway::Gateway>>>,
    /// Public tunnels, keyed by the local port each exposes.
    tunnels: Arc<Mutex<tunnel::TunnelManager>>,
    /// Local accounts and per-project visibility. No database; see [`auth`].
    auth: Arc<Mutex<auth::AuthStore>>,
}

fn update_tray_menu(app: &tauri::AppHandle) {
    if let Some(tray) = app.tray_by_id("main") {
        let state = app.state::<AppState>();
        let processes = state.active_processes.lock().unwrap();

        let menu = tauri::menu::Menu::new(app).unwrap();
        let show_i = tauri::menu::MenuItem::with_id(app, "show", "Show DevDeck", true, None::<&str>).unwrap();
        let _ = menu.append(&show_i);
        let _ = menu.append(&tauri::menu::PredefinedMenuItem::separator(app).unwrap());

        let mut has_processes = false;
        for (key, _) in processes.iter() {
            has_processes = true;
            let title = format!("Stop: {}", key);
            let id = format!("stop_{}", key);
            let item = tauri::menu::MenuItem::with_id(app, &id, &title, true, None::<&str>).unwrap();
            let _ = menu.append(&item);
        }

        if !has_processes {
            let empty_i = tauri::menu::MenuItem::with_id(app, "empty", "No running projects", false, None::<&str>).unwrap();
            let _ = menu.append(&empty_i);
        }

        let _ = menu.append(&tauri::menu::PredefinedMenuItem::separator(app).unwrap());
        let stop_all_i = tauri::menu::MenuItem::with_id(app, "stop_all", "Stop All Commands", true, None::<&str>).unwrap();
        let _ = menu.append(&stop_all_i);
        let quit_i = tauri::menu::MenuItem::with_id(app, "quit", "Quit", true, None::<&str>).unwrap();
        let _ = menu.append(&quit_i);

        let _ = tray.set_menu(Some(menu));
    }
}

// Helpers
fn parse_projects(paths: Vec<PathBuf>) -> Vec<ProjectInfo> {
    let mut project_map: HashMap<String, ProjectInfo> = HashMap::new();
    
    for dir in paths {
        let dir_str = dir.to_string_lossy().to_string();
        let name = dir.file_name().unwrap_or_default().to_string_lossy().to_string();
        
        let mut proj = project_map.remove(&dir_str).unwrap_or_else(|| {
            ProjectInfo { path: dir_str.clone(), name: name.clone(), project_type: "Unknown".to_string(), scripts: HashMap::new() }
        });
        
        let mut types = Vec::new();
        
        if let Ok(content) = fs::read_to_string(dir.join("package.json")) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(n) = json.get("name").and_then(|n| n.as_str()) {
                    proj.name = n.to_string();
                }
                if let Some(s) = json.get("scripts").and_then(|s| s.as_object()) {
                    for (k, v) in s {
                        if let Some(_v_str) = v.as_str() {
                            proj.scripts.insert(k.clone(), format!("npm run {}", k));
                        }
                    }
                }
                if !proj.scripts.contains_key("install") {
                    proj.scripts.insert("install".to_string(), "npm install".to_string());
                }
                types.push("Node.js");
            }
        }
        
        if dir.join("Cargo.toml").exists() {
            proj.scripts.insert("cargo run".to_string(), "cargo run".to_string());
            proj.scripts.insert("cargo build".to_string(), "cargo build".to_string());
            proj.scripts.insert("cargo test".to_string(), "cargo test".to_string());
            types.push("Rust");
        }
        
        if dir.join("manage.py").exists() {
            proj.scripts.insert("runserver".to_string(), "python manage.py runserver".to_string());
            proj.scripts.insert("migrate".to_string(), "python manage.py migrate".to_string());
            types.push("Django");
        }
        
        if dir.join("go.mod").exists() {
            proj.scripts.insert("go run".to_string(), "go run .".to_string());
            proj.scripts.insert("go build".to_string(), "go build".to_string());
            types.push("Go");
        }
        
        if dir.join("main.py").exists() {
            proj.scripts.insert("run main".to_string(), "python main.py".to_string());
            types.push("Python");
        } else if dir.join("app.py").exists() {
            proj.scripts.insert("run app".to_string(), "python app.py".to_string());
            types.push("Python");
        }
        
        if dir.join("requirements.txt").exists() {
            if !proj.scripts.contains_key("run main") && !proj.scripts.contains_key("run app") && !types.contains(&"Django") {
                 proj.scripts.insert("python main".to_string(), "python main.py".to_string());
            }
            if !types.contains(&"Python") && !types.contains(&"Django") {
                types.push("Python");
            }
        }
        
        if dir.join("docker-compose.yml").exists() {
            proj.scripts.insert("docker up".to_string(), "docker-compose up".to_string());
            proj.scripts.insert("docker down".to_string(), "docker-compose down".to_string());
            types.push("Docker");
        }
        
        if dir.join("pom.xml").exists() {
            proj.scripts.insert("spring boot".to_string(), "mvn spring-boot:run".to_string());
            proj.scripts.insert("mvn install".to_string(), "mvn clean install".to_string());
            types.push("Java (Maven)");
        }
        
        if let Ok(content) = fs::read_to_string(dir.join("composer.json")) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(n) = json.get("name").and_then(|n| n.as_str()) {
                    let short_name = n.split('/').last().unwrap_or(n);
                    proj.name = short_name.to_string();
                }
                if let Some(s) = json.get("scripts").and_then(|s| s.as_object()) {
                    for (k, _v) in s {
                        proj.scripts.insert(k.clone(), format!("composer run-script {}", k));
                    }
                }
            }
            if !proj.scripts.contains_key("composer install") {
                proj.scripts.insert("composer install".to_string(), "composer install".to_string());
            }
            if !proj.scripts.contains_key("php serve") {
                proj.scripts.insert("php serve".to_string(), "php -S localhost:8000".to_string());
            }
            types.push("PHP (Composer)");
        }

        if dir.join("artisan").exists() {
            proj.scripts.insert("artisan serve".to_string(), "php artisan serve".to_string());
            proj.scripts.insert("artisan migrate".to_string(), "php artisan migrate".to_string());
            if !types.iter().any(|t| t.contains("PHP")) {
                types.push("Laravel (PHP)");
            }
        }

        if dir.join("wp-config.php").exists() || dir.join("wp-content").exists() {
            if !proj.scripts.contains_key("php serve") {
                proj.scripts.insert("php serve".to_string(), "php -S localhost:8000".to_string());
            }
            if !types.iter().any(|t| t.contains("PHP")) {
                types.push("WordPress (PHP)");
            }
        }

        if dir.join("symfony.lock").exists() || dir.join("bin/console").exists() {
            proj.scripts.insert("symfony serve".to_string(), "php bin/console server:run".to_string());
            if !types.iter().any(|t| t.contains("PHP")) {
                types.push("Symfony (PHP)");
            }
        }

        if (dir.join("index.php").exists() || dir.join("server.php").exists()) && types.is_empty() {
            proj.scripts.insert("php serve".to_string(), "php -S localhost:8000".to_string());
            types.push("PHP");
        }

        if types.iter().any(|t| t.contains("PHP")) {
            if !proj.scripts.contains_key("live server") {
                proj.scripts.insert("live server".to_string(), "npx -y live-server".to_string());
            }
        }
        
        if dir.join("Gemfile").exists() {
            proj.scripts.insert("rails server".to_string(), "rails server".to_string());
            types.push("Ruby on Rails");
        }
        
        if dir.join("index.html").exists() {
            proj.scripts.insert("live server".to_string(), "npx -y live-server".to_string());
            if !types.contains(&"Node.js") && !types.contains(&"Static Web") {
                types.push("Static Web");
            }
        }
        
        if !types.is_empty() {
            if types.contains(&"Rust") && types.contains(&"Node.js") {
                proj.project_type = "Tauri (Rust+Node)".to_string();
            } else {
                proj.project_type = types.join(" + ");
            }
        }
        
        if !proj.scripts.is_empty() {
            project_map.insert(dir_str, proj);
        }
    }
    
    project_map.into_values().collect()
}

fn find_projects_recursive(dir: &Path, depth: u32, max_depth: u32, app: Option<&AppHandle>) -> Vec<PathBuf> {
    let mut results = Vec::new();
    if depth > max_depth { return results; }

    if let Some(app) = app {
        if depth == 0 {
            let _ = app.emit("scan-progress", format!("Scanning {}...", dir.display()));
        }
    }

    if let Ok(entries) = fs::read_dir(dir) {
        let mut is_project = false;
        let mut subdirs = Vec::new();
        
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();
            let ignored = ["node_modules", ".git", "AppData", "Local", "Roaming", "Temp", ".npm", ".cache", "vendor", "$RECYCLE.BIN", "System Volume Information"];
            
            if file_name.starts_with('.') || ignored.contains(&file_name.as_str()) {
                continue;
            }

            if path.is_dir() {
                subdirs.push(path);
            } else {
                let indicators = ["package.json", "Cargo.toml", "manage.py", "go.mod", "main.py", "app.py", "requirements.txt", "docker-compose.yml", "pom.xml", "composer.json", "artisan", "wp-config.php", "index.php", "server.php", "symfony.lock", "Gemfile", "index.html"];
                if indicators.contains(&file_name.as_str()) {
                    is_project = true;
                }
            }
        }
        
        if is_project {
            results.push(dir.to_path_buf());
        }
        
        for sub in subdirs {
            results.extend(find_projects_recursive(&sub, depth + 1, max_depth, app));
        }
    }
    results
}

// Commands
#[tauri::command]
async fn scan_projects(root_dir: String) -> Result<Vec<ProjectInfo>, String> {
    let paths = find_projects_recursive(Path::new(&root_dir), 0, 5, None);
    Ok(parse_projects(paths))
}



#[tauri::command]
fn get_node_processes() -> Result<Vec<NodeProcess>, String> {
    let script = r#"
        Get-CimInstance Win32_Process -Filter "Name='node.exe'" | Select-Object ProcessId, CommandLine | ConvertTo-Json
    "#;
    let output = Command::new("powershell")
        .args(["-NoProfile", "-Command", script])
        .apply_cross_platform_flags()
        .output()
        .map_err(|e| e.to_string())?;
        
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap_or(serde_json::Value::Array(vec![]));
    
    let mut arr = vec![];
    if json.is_array() {
        arr = json.as_array().unwrap().clone();
    } else if json.is_object() {
        arr.push(json);
    }
    
    let mut result = Vec::new();
    let current_pid = std::process::id();
    
    for p in arr {
        let pid = p.get("ProcessId").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let cmd = p.get("CommandLine").and_then(|v| v.as_str()).unwrap_or("").to_string();
        
        if pid == current_pid || cmd.is_empty() { continue; }
        if cmd.to_lowercase().contains("electron") && cmd.to_lowercase().contains("project-manager-app") { continue; }
        
        let mut project_path = "Unknown Path".to_string();
        if let Some(idx) = cmd.find("node_modules") {
            let substr = &cmd[0..idx];
            if let Some(start) = substr.rfind(|c| c == '\'' || c == '"' || c == ' ') {
                project_path = substr[start+1..].trim().to_string();
            } else {
                project_path = substr.trim().to_string();
            }
            if project_path.ends_with("\\") {
                project_path.pop();
            }
        }
        
        let lower_path = project_path.to_lowercase();
        let mut is_global = false;
        let mut global_pkg = String::new();

        if lower_path.contains("npm-cache\\_npx") || 
           lower_path.contains("program files\\nodejs") || 
           lower_path.contains("program files (x86)\\nodejs") ||
           lower_path.contains("nvm\\v") ||
           lower_path.contains("yarn\\global") {
            is_global = true;
            if let Some(idx) = cmd.find("node_modules\\") {
                let after = &cmd[idx + 13..];
                if let Some(slash) = after.find('\\') {
                    global_pkg = after[0..slash].to_string();
                } else if let Some(space) = after.find(' ') {
                    global_pkg = after[0..space].to_string();
                } else {
                    global_pkg = after.to_string();
                }
            }
        }

        if is_global {
            if !global_pkg.is_empty() {
                project_path = format!("Global Tool: {}", global_pkg);
            } else {
                project_path = "Global Node Process".to_string();
            }
        }

        
        let mut script_type = "Node Process".to_string();
        if cmd.contains("npm-cli.js") { script_type = "NPM Wrapper".to_string(); }
        else if cmd.contains("next") { script_type = "Next.js".to_string(); }
        else if cmd.contains("vite") { script_type = "Vite".to_string(); }
        else if cmd.contains("nodemon") { script_type = "Nodemon".to_string(); }
        else if cmd.contains("npm") { script_type = "NPM Script".to_string(); }
        
        if script_type != "NPM Wrapper" {
            result.push(NodeProcess {
                pid,
                command: cmd,
                projectPath: project_path,
                process_type: script_type,
            });
        }
    }
    
    Ok(result)
}

#[tauri::command]
fn kill_process(pid: u32) -> Result<String, String> {
    let output = Command::new("C:\\Windows\\System32\\taskkill.exe")
        .args(["/PID", &pid.to_string(), "/F", "/T"])
        .apply_cross_platform_flags()
        .output()
        .map_err(|e| e.to_string())?;
        
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    
    if !output.status.success() {
        return Err(format!("Taskkill failed: {} {}", stdout, stderr));
    }
    
    Ok(format!("{} {}", stdout, stderr))
}

// Resolves the command's binary before spawning so that tools installed after
// this app started (or living in XAMPP/Herd/Laragon) are reachable from `cmd`.
#[cfg(target_os = "windows")]
fn prepare_command_environment(command_str: &str) {
    if let Some(binary) = command_str.trim().split_whitespace().next() {
        let binary = binary.trim_matches('"').to_lowercase();
        if !binary.is_empty() {
            ensure_tool_available(&binary);
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn prepare_command_environment(_command_str: &str) {}

#[tauri::command]
fn run_custom_command(app: AppHandle, state: State<AppState>, project_path: String, command_str: String) -> Result<(), String> {
    let script_name = format!("$ {}", command_str);
    let process_key = format!("{}:{}", project_path, script_name);
    
    if state.active_processes.lock().unwrap().contains_key(&process_key) {
        return Err("Process is already running".to_string());
    }
    prepare_command_environment(&command_str);

    let mut child = Command::new("cmd")
        .args(["/C", &command_str])
        .current_dir(&project_path)
        .apply_cross_platform_flags()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    
    state.active_processes.lock().unwrap().insert(process_key.clone(), child);
    let detected_ports = state.detected_ports.clone();
    let _ = app.emit("process-started", serde_json::json!({ "processKey": process_key }));
    
    let pk1 = process_key.clone();
    let app1 = app.clone();
    let ports1 = detected_ports.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            if let Ok(l) = line {
                note_detected_port(&app1, &ports1, &pk1, &l);
                let _ = app1.emit("process-output", serde_json::json!({ "processKey": pk1, "type": "stdout", "data": format!("{}\n", l) }));
            }
        }
    });

    let pk2 = process_key.clone();
    let app2 = app.clone();
    let ports2 = detected_ports.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            if let Ok(l) = line {
                note_detected_port(&app2, &ports2, &pk2, &l);
                let _ = app2.emit("process-output", serde_json::json!({ "processKey": pk2, "type": "stderr", "data": format!("{}\n", l) }));
            }
        }
    });

    let pk3 = process_key.clone();
    let app3 = app.clone();
    let active_procs = state.active_processes.clone();
    let ports3 = detected_ports.clone();
    thread::spawn(move || {
        loop {
            thread::sleep(std::time::Duration::from_millis(500));
            let mut is_running = false;
            if let Some(child_ref) = active_procs.lock().unwrap().get_mut(&pk3) {
                if let Ok(Some(_status)) = child_ref.try_wait() {
                    is_running = false;
                } else {
                    is_running = true;
                }
            }
            if !is_running {
                active_procs.lock().unwrap().remove(&pk3);
                ports3.lock().unwrap().remove(&pk3);
                unpublish_preview(&app3, &pk3);
                let _ = app3.emit("process-closed", serde_json::json!({ "processKey": pk3, "code": 0 }));
                update_tray_menu(&app3);
                break;
            }
        }
    });

    update_tray_menu(&app);
    Ok(())
}

/// Record a port scraped from process output and tell the frontend about it.
///
/// Re-announcing is suppressed: dev servers reprint their banner on every
/// rebuild, and the gateway only cares when the port actually changes.
fn note_detected_port(
    app: &AppHandle,
    ports: &Arc<Mutex<HashMap<String, u16>>>,
    process_key: &str,
    line: &str,
) {
    // Registering the preview needs the app state, which is reachable from the
    // handle; see `publish_preview` below.
    let Some(port) = ports::detect_port(line) else { return };

    {
        let mut map = ports.lock().unwrap();
        if map.get(process_key) == Some(&port) {
            return;
        }
        map.insert(process_key.to_string(), port);
    }

    let preview_port = publish_preview(app, process_key, port);

    let _ = app.emit(
        "process-port-detected",
        serde_json::json!({
            "processKey": process_key,
            "port": port,
            "previewPort": preview_port,
        }),
    );
}

/// Expose a freshly detected dev server through the gateway, if it is running.
///
/// Returns the public port the project is reachable on, or `None` when the
/// gateway is off — in which case the project is simply local-only.
fn publish_preview(app: &AppHandle, process_key: &str, upstream_port: u16) -> Option<u16> {
    let state = app.state::<AppState>();
    let guard = state.gateway.lock().unwrap();
    let gw = guard.as_ref()?;
    match gw.add_preview(process_key, upstream_port) {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("[devdeck] could not publish preview for {process_key}: {e}");
            None
        }
    }
}

/// Withdraw a project's preview once its process is gone.
fn unpublish_preview(app: &AppHandle, process_key: &str) {
    let state = app.state::<AppState>();
    // Bind the guard: `if let` would otherwise drop the temporary while `gw`
    // still borrows from it.
    let guard = state.gateway.lock().unwrap();
    if let Some(gw) = guard.as_ref() {
        gw.remove_preview(process_key);
    }
}

#[tauri::command]
fn gateway_start(state: State<AppState>) -> Result<gateway::GatewayInfo, String> {
    let mut guard = state.gateway.lock().unwrap();
    if let Some(gw) = guard.as_ref() {
        return Ok(gw.info());
    }
    let gw = gateway::Gateway::start(state.auth.clone())?;
    let info = gw.info();
    *guard = Some(gw);
    drop(guard);

    // Adopt any dev servers that were already running before the gateway came up.
    let known: Vec<(String, u16)> = state
        .detected_ports
        .lock()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    if let Some(gw) = state.gateway.lock().unwrap().as_ref() {
        for (key, port) in known {
            let _ = gw.add_preview(&key, port);
        }
    }
    Ok(info)
}

#[tauri::command]
fn gateway_stop(state: State<AppState>) -> Result<(), String> {
    // Tunnels point at gateway ports, so they die with it rather than lingering
    // as cloudflared processes routing to nothing.
    state.tunnels.lock().unwrap().close_all();
    if let Some(mut gw) = state.gateway.lock().unwrap().take() {
        gw.shutdown();
    }
    Ok(())
}

/// SVG QR encoding the pairing URL, for the desktop "Remote" panel.
#[tauri::command]
fn gateway_pair_qr(state: State<AppState>) -> Result<String, String> {
    let guard = state.gateway.lock().unwrap();
    let gw = guard.as_ref().ok_or("Gateway is not running")?;
    let url = gw
        .info()
        .pair_url
        .ok_or("No LAN address available - are you connected to a network?")?;
    gateway::qr_svg(&url)
}

/// Is cloudflared available for remote access?
#[tauri::command]
fn tunnel_available() -> bool {
    tunnel::is_installed()
}

/// Install cloudflared via Winget, for the "remote access" opt-in.
#[tauri::command]
async fn tunnel_install() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(tunnel::install)
        .await
        .map_err(|e| format!("install task failed: {e}"))?
}

/// Expose the gateway's control plane publicly and return its URL.
///
/// This is what makes the project list reachable from another network. Sharing
/// an individual project additionally needs [`tunnel_share_project`], because a
/// quick tunnel covers exactly one port.
#[tauri::command]
async fn tunnel_start(state: State<'_, AppState>) -> Result<String, String> {
    let port = {
        let guard = state.gateway.lock().unwrap();
        guard
            .as_ref()
            .ok_or("Turn on the gateway first")?
            .info()
            .control_port
    };
    open_tunnel(state.tunnels.clone(), port).await
}

/// Expose one project's preview port publicly.
#[tauri::command]
async fn tunnel_share_project(state: State<'_, AppState>, preview_port: u16) -> Result<String, String> {
    open_tunnel(state.tunnels.clone(), preview_port).await
}

/// Shared body for the two commands above.
///
/// Opening a tunnel blocks for up to 45s waiting on Cloudflare, so it runs on
/// the blocking pool rather than stalling the UI thread.
async fn open_tunnel(
    tunnels: Arc<Mutex<tunnel::TunnelManager>>,
    port: u16,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || tunnels.lock().unwrap().open(port))
        .await
        .map_err(|e| format!("tunnel task failed: {e}"))?
}

#[tauri::command]
fn tunnel_stop(state: State<AppState>, port: Option<u16>) -> Result<(), String> {
    let mut mgr = state.tunnels.lock().unwrap();
    match port {
        Some(p) => mgr.close(p),
        None => mgr.close_all(),
    }
    Ok(())
}

/// Every live tunnel, as a local-port to public-URL map.
#[tauri::command]
fn tunnel_status(state: State<AppState>) -> HashMap<u16, String> {
    state.tunnels.lock().unwrap().all()
}

#[derive(serde::Serialize)]
struct AuthStatus {
    has_account: bool,
    username: Option<String>,
}

/// Whether an account exists, for the desktop Remote panel.
#[tauri::command]
fn auth_status(state: State<AppState>) -> AuthStatus {
    let auth = state.auth.lock().unwrap();
    AuthStatus {
        has_account: auth.has_account(),
        username: auth.username().map(str::to_string),
    }
}

/// Create or replace the account used to sign in from a phone.
///
/// Hashing is deliberately slow, so this runs off the UI thread.
#[tauri::command]
async fn auth_set_account(
    state: State<'_, AppState>,
    username: String,
    password: String,
) -> Result<(), String> {
    let auth = state.auth.clone();
    tauri::async_runtime::spawn_blocking(move || {
        auth.lock().unwrap().set_account(&username, &password)
    })
    .await
    .map_err(|e| format!("account task failed: {e}"))?
}

/// Sign every paired device out.
#[tauri::command]
fn auth_revoke_sessions(state: State<AppState>) {
    state.auth.lock().unwrap().revoke_all_sessions();
}

/// Per-project visibility, keyed by process key.
#[tauri::command]
fn project_visibility(state: State<AppState>) -> HashMap<String, auth::Visibility> {
    state.auth.lock().unwrap().all_visibility()
}

/// Mark a project public (viewable without signing in) or private.
#[tauri::command]
fn set_project_visibility(state: State<AppState>, project_key: String, public: bool) {
    let v = if public {
        auth::Visibility::Public
    } else {
        auth::Visibility::Private
    };
    state.auth.lock().unwrap().set_visibility(&project_key, v);
}

#[tauri::command]
fn gateway_status(state: State<AppState>) -> gateway::GatewayInfo {
    match state.gateway.lock().unwrap().as_ref() {
        Some(gw) => gw.info(),
        None => gateway::GatewayInfo {
            running: false,
            lan_url: None,
            pair_url: None,
            control_port: gateway::CONTROL_PORT,
            token: String::new(),
        },
    }
}

/// Ports of every currently running dev server, keyed by process key.
#[tauri::command]
fn get_detected_ports(state: State<AppState>) -> HashMap<String, u16> {
    state.detected_ports.lock().unwrap().clone()
}

#[tauri::command]
fn run_script(app: AppHandle, state: State<AppState>, project_path: String, script_name: String, script_cmd: String) -> Result<(), String> {
    let process_key = format!("{}:{}", project_path, script_name);
    
    if state.active_processes.lock().unwrap().contains_key(&process_key) {
        return Err("Process is already running".to_string());
    }
    prepare_command_environment(&script_cmd);

    let mut child = Command::new("cmd")
        .args(["/C", &script_cmd])
        .current_dir(&project_path)
        .apply_cross_platform_flags()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    
    state.active_processes.lock().unwrap().insert(process_key.clone(), child);
    let detected_ports = state.detected_ports.clone();
    let _ = app.emit("process-started", serde_json::json!({ "processKey": process_key }));
    
    let pk1 = process_key.clone();
    let app1 = app.clone();
    let ports1 = detected_ports.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            if let Ok(l) = line {
                note_detected_port(&app1, &ports1, &pk1, &l);
                let _ = app1.emit("process-output", serde_json::json!({ "processKey": pk1, "type": "stdout", "data": format!("{}\n", l) }));
            }
        }
    });

    let pk2 = process_key.clone();
    let app2 = app.clone();
    let ports2 = detected_ports.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            if let Ok(l) = line {
                note_detected_port(&app2, &ports2, &pk2, &l);
                let _ = app2.emit("process-output", serde_json::json!({ "processKey": pk2, "type": "stderr", "data": format!("{}\n", l) }));
            }
        }
    });

    let pk3 = process_key.clone();
    let app3 = app.clone();
    let active_procs = state.active_processes.clone();
    let ports3 = detected_ports.clone();
    thread::spawn(move || {
        loop {
            thread::sleep(std::time::Duration::from_millis(500));
            let mut is_running = false;
            if let Some(child_ref) = active_procs.lock().unwrap().get_mut(&pk3) {
                if let Ok(Some(_status)) = child_ref.try_wait() {
                    is_running = false;
                } else {
                    is_running = true;
                }
            }
            if !is_running {
                active_procs.lock().unwrap().remove(&pk3);
                ports3.lock().unwrap().remove(&pk3);
                unpublish_preview(&app3, &pk3);
                let _ = app3.emit("process-closed", serde_json::json!({ "processKey": pk3, "code": 0 }));
                update_tray_menu(&app3);
                break;
            }
        }
    });

    update_tray_menu(&app);
    Ok(())
}



#[tauri::command]
fn stop_script(app: AppHandle, state: State<AppState>, process_key: String) {
    if let Some(mut child) = state.active_processes.lock().unwrap().remove(&process_key) {
        let pid = child.id();
        let _ = Command::new("C:\\Windows\\System32\\taskkill.exe")
            .args(["/PID", &pid.to_string(), "/F", "/T"])
            .apply_cross_platform_flags()
            .output();
    }
    update_tray_menu(&app);
}

#[tauri::command]
fn open_external_url(url: String) {
    let _ = Command::new("explorer")
        .arg(&url)
        .spawn();
}

#[tauri::command]
async fn select_directory(app: AppHandle) -> Result<Option<String>, String> {
    let folder = rfd::FileDialog::new()
        .set_title("Select a Workspace Directory")
        .pick_folder();
        
    if let Some(path) = folder {
        Ok(Some(path.to_string_lossy().to_string()))
    } else {
        Ok(None)
    }
}

#[tauri::command]
fn write_to_stdin(state: State<AppState>, process_key: String, input: String) -> Result<(), String> {
    let mut processes = state.active_processes.lock().unwrap();
    if let Some(child) = processes.get_mut(&process_key) {
        if let Some(stdin) = child.stdin.as_mut() {
            use std::io::Write;
            let formatted_input = format!("{}\n", input);
            stdin.write_all(formatted_input.as_bytes()).map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    Err("Process or stdin not found".to_string())
}

#[tauri::command]
fn open_in_editor(path: String, tool: String) -> Result<(), String> {
    let tool_lower = tool.to_lowercase();

    let is_cli_agent = matches!(
        tool_lower.as_str(),
        "antigravity" | "agy" | "claude" | "claude-cli" | "aider" | "copilot" | "gemini" | "terminal" | "cmd"
    );

    #[cfg(target_os = "windows")]
    {
        if tool_lower == "explorer" {
            Command::new("explorer")
                .arg(&path)
                .spawn()
                .map_err(|e| e.to_string())?;
        } else if is_cli_agent {
            let cli_cmd = match tool_lower.as_str() {
                "antigravity" | "agy" => "agy",
                "claude" | "claude-cli" => "claude",
                "aider" => "aider",
                "copilot" => "gh copilot",
                "gemini" => "gemini",
                _ => "",
            };

            let title = if cli_cmd.is_empty() {
                "Terminal".to_string()
            } else {
                format!("DevDeck - {}", tool)
            };

            let cmd_args = if cli_cmd.is_empty() {
                format!("start \"{}\" cmd /K \"cd /d {}\"", title, path)
            } else {
                format!("start \"{}\" cmd /K \"cd /d {} && {}\"", title, path, cli_cmd)
            };

            Command::new("cmd")
                .args(["/C", &cmd_args])
                .apply_cross_platform_flags()
                .spawn()
                .map_err(|e| format!("Failed to launch AI agent/terminal: {}", e))?;
        } else {
            let cmd_target = match tool_lower.as_str() {
                "code" => "code",
                "code-insiders" => "code-insiders",
                "cursor" => "cursor",
                "windsurf" => "windsurf",
                "idea" => "idea",
                "webstorm" => "webstorm",
                "pycharm" => "pycharm",
                "sublime" => "subl",
                "fleet" => "fleet",
                "studio" => "studio",
                "zed" => "zed",
                _ => &tool,
            };

            let ps_script = format!(
                "Start-Process -FilePath '{}' -ArgumentList '\"{}\"' -WindowStyle Hidden",
                cmd_target,
                path.replace("'", "''")
            );

            let status = Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", &ps_script])
                .apply_cross_platform_flags()
                .spawn();

            if let Err(e) = status {
                return Err(format!("Could not launch {}: {}", tool, e));
            }
        }
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        if tool_lower == "explorer" {
            Command::new("open")
                .arg(&path)
                .spawn()
                .map_err(|e| e.to_string())?;
        } else {
            let cmd_target = match tool_lower.as_str() {
                "antigravity" | "agy" => "agy",
                "claude" | "claude-cli" => "claude",
                "sublime" => "subl",
                _ => &tool,
            };
            Command::new(cmd_target)
                .arg(&path)
                .spawn()
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

#[tauri::command]
fn open_external_terminal(path: String) -> Result<(), String> {
    Command::new("cmd")
        .args(["/C", "start", "cmd"])
        .current_dir(path)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn expand_env_placeholders(raw: &str) -> String {
    let mut out = String::new();
    let mut rest = raw;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(val) => out.push_str(&val),
                    Err(_) => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push('%');
                rest = after;
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

// Winget writes new PATH entries into the registry. Processes that were already
// running (like this app) keep the PATH they inherited at launch, which is why a
// freshly installed tool still looks "missing" until a restart. Read it back.
#[cfg(target_os = "windows")]
fn read_registry_path(key: &str) -> Option<String> {
    let output = Command::new("reg")
        .args(["query", key, "/v", "Path"])
        .apply_cross_platform_flags()
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.to_lowercase().starts_with("path") {
            continue;
        }
        if let Some(idx) = trimmed.find("REG_") {
            let after_type = &trimmed[idx..];
            if let Some(gap) = after_type.find(char::is_whitespace) {
                let value = after_type[gap..].trim();
                if !value.is_empty() {
                    return Some(expand_env_placeholders(value));
                }
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn merge_path_entries(sources: Vec<String>) -> String {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut merged: Vec<String> = Vec::new();
    for source in sources {
        for entry in source.split(';') {
            let entry = entry.trim().trim_matches('"');
            if entry.is_empty() {
                continue;
            }
            let key = entry.trim_end_matches('\\').to_lowercase();
            if key.is_empty() {
                continue;
            }
            if seen.insert(key) {
                merged.push(entry.to_string());
            }
        }
    }
    merged.join(";")
}

#[cfg(target_os = "windows")]
fn refresh_process_path() {
    let mut sources: Vec<String> = Vec::new();
    if let Ok(current) = std::env::var("PATH") {
        sources.push(current);
    }
    if let Some(machine) =
        read_registry_path("HKLM\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment")
    {
        sources.push(machine);
    }
    if let Some(user) = read_registry_path("HKCU\\Environment") {
        sources.push(user);
    }
    let merged = merge_path_entries(sources);
    if !merged.is_empty() {
        std::env::set_var("PATH", merged);
    }
}

#[cfg(target_os = "windows")]
fn prepend_process_path(dir: &Path) {
    let dir_str = dir.to_string_lossy().to_string();
    let current = std::env::var("PATH").unwrap_or_default();
    let merged = merge_path_entries(vec![dir_str, current]);
    if !merged.is_empty() {
        std::env::set_var("PATH", merged);
    }
}

#[cfg(target_os = "windows")]
fn tool_executables(tool: &str) -> Vec<&'static str> {
    match tool {
        "php" => vec!["php.exe"],
        "composer" => vec!["composer.bat", "composer.exe", "composer.cmd"],
        "mysql" => vec!["mysql.exe"],
        "node" => vec!["node.exe"],
        "npm" => vec!["npm.cmd", "npm.exe"],
        "python" => vec!["python.exe"],
        "git" => vec!["git.exe"],
        "docker" => vec!["docker.exe"],
        _ => vec![],
    }
}

// Directories that commonly hold these tools but are not always on PATH.
// XAMPP, Laragon and Herd all ship PHP without exporting it.
#[cfg(target_os = "windows")]
fn candidate_tool_dirs(tool: &str) -> Vec<PathBuf> {
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let roaming = std::env::var("APPDATA").unwrap_or_default();
    let program_files =
        std::env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".to_string());
    let mut dirs: Vec<PathBuf> = Vec::new();

    if !local.is_empty() {
        // Winget portable packages land here; PHP ships as a portable zip.
        dirs.push(Path::new(&local).join("Microsoft\\WinGet\\Links"));
        dirs.push(Path::new(&local).join("Microsoft\\WinGet\\Packages"));
    }

    match tool {
        "php" => {
            dirs.push(PathBuf::from("C:\\xampp\\php"));
            dirs.push(PathBuf::from("C:\\laragon\\bin\\php"));
            dirs.push(PathBuf::from("C:\\tools\\php"));
            dirs.push(PathBuf::from("C:\\php"));
            dirs.push(PathBuf::from("C:\\wamp64\\bin\\php"));
            if !local.is_empty() {
                dirs.push(Path::new(&local).join("Herd\\bin"));
                dirs.push(Path::new(&local).join("Programs\\Herd\\resources\\bin"));
            }
        }
        "composer" => {
            dirs.push(PathBuf::from("C:\\xampp"));
            dirs.push(PathBuf::from("C:\\ProgramData\\ComposerSetup\\bin"));
            dirs.push(PathBuf::from("C:\\laragon\\bin\\composer"));
            if !roaming.is_empty() {
                dirs.push(Path::new(&roaming).join("Composer\\vendor\\bin"));
            }
            if !local.is_empty() {
                dirs.push(Path::new(&local).join("Herd\\bin"));
            }
        }
        "mysql" => {
            dirs.push(PathBuf::from("C:\\xampp\\mysql\\bin"));
            dirs.push(PathBuf::from("C:\\laragon\\bin\\mysql"));
            dirs.push(Path::new(&program_files).join("MySQL"));
        }
        "node" | "npm" => {
            dirs.push(Path::new(&program_files).join("nodejs"));
            if !roaming.is_empty() {
                dirs.push(Path::new(&roaming).join("npm"));
            }
        }
        _ => {}
    }
    dirs
}

// Looks for the executable directly in `dir`, then one level down (Laragon,
// Winget package folders and MySQL all nest a versioned directory), plus the
// `bin` subfolder of each of those.
#[cfg(target_os = "windows")]
fn find_executable_dir(dir: &Path, executables: &[&str]) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    let direct_hit = |candidate: &Path| -> Option<PathBuf> {
        for exe in executables {
            if candidate.join(exe).is_file() {
                return Some(candidate.to_path_buf());
            }
        }
        None
    };

    if let Some(found) = direct_hit(dir) {
        return Some(found);
    }
    if let Some(found) = direct_hit(&dir.join("bin")) {
        return Some(found);
    }

    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let child = entry.path();
        if !child.is_dir() {
            continue;
        }
        if let Some(found) = direct_hit(&child) {
            return Some(found);
        }
        if let Some(found) = direct_hit(&child.join("bin")) {
            return Some(found);
        }
        // Winget nests one more level: Packages/<id>/<extracted-folder>/php.exe
        if let Ok(grandchildren) = fs::read_dir(&child) {
            for grandchild in grandchildren.flatten() {
                let path = grandchild.path();
                if !path.is_dir() {
                    continue;
                }
                if let Some(found) = direct_hit(&path) {
                    return Some(found);
                }
                if let Some(found) = direct_hit(&path.join("bin")) {
                    return Some(found);
                }
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn tool_on_path(tool: &str) -> bool {
    Command::new("where.exe")
        .arg(tool)
        .apply_cross_platform_flags()
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

// True when the tool can actually be launched by `run_script`. As a side effect
// it repairs this process's PATH so the spawned `cmd /C ...` inherits it.
#[cfg(target_os = "windows")]
fn ensure_tool_available(tool: &str) -> bool {
    if tool_on_path(tool) {
        return true;
    }
    refresh_process_path();
    if tool_on_path(tool) {
        return true;
    }
    let executables = tool_executables(tool);
    if executables.is_empty() {
        return false;
    }
    for dir in candidate_tool_dirs(tool) {
        if let Some(found) = find_executable_dir(&dir, &executables) {
            prepend_process_path(&found);
            return true;
        }
    }
    false
}

// winget.exe is an App Execution Alias, so it is not always resolvable by name
// from a GUI process.
#[cfg(target_os = "windows")]
fn winget_path() -> Option<PathBuf> {
    if let Ok(out) = Command::new("where.exe")
        .arg("winget.exe")
        .apply_cross_platform_flags()
        .output()
    {
        if out.status.success() {
            if let Some(first) = String::from_utf8_lossy(&out.stdout).lines().next() {
                let path = PathBuf::from(first.trim());
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let alias = Path::new(&local).join("Microsoft\\WindowsApps\\winget.exe");
        if alias.is_file() {
            return Some(alias);
        }
    }
    if let Ok(program_files) = std::env::var("ProgramFiles") {
        if let Ok(entries) = fs::read_dir(Path::new(&program_files).join("WindowsApps")) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name.starts_with("microsoft.desktopappinstaller_") {
                    let candidate = entry.path().join("winget.exe");
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn tidy_winget_output(raw: &str) -> String {
    let cleaned: Vec<String> = raw
        .lines()
        .map(|line| {
            line.chars()
                .filter(|c| !c.is_control() && !('\u{2500}'..='\u{259F}').contains(c))
                .collect::<String>()
                .trim()
                .to_string()
        })
        .filter(|line| !line.is_empty())
        .collect();
    if cleaned.len() > 12 {
        cleaned[cleaned.len() - 12..].join("\n")
    } else {
        cleaned.join("\n")
    }
}

#[cfg(target_os = "windows")]
struct WingetFailure {
    message: String,
    cancelled: bool,
}

#[cfg(target_os = "windows")]
fn run_winget_install(winget: &Path, id: &str) -> Result<(), WingetFailure> {
    let output = Command::new(winget)
        .args([
            "install",
            "--id",
            id,
            "-e",
            "--source",
            "winget",
            "--silent",
            "--disable-interactivity",
            "--accept-source-agreements",
            "--accept-package-agreements",
        ])
        .apply_cross_platform_flags()
        .output();

    let output = match output {
        Ok(out) => out,
        Err(e) => {
            return Err(WingetFailure {
                message: format!("could not start winget ({})", e),
                cancelled: false,
            })
        }
    };

    let mut combined = tidy_winget_output(&String::from_utf8_lossy(&output.stdout));
    let stderr = tidy_winget_output(&String::from_utf8_lossy(&output.stderr));
    if !stderr.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(&stderr);
    }

    if output.status.success() {
        return Ok(());
    }

    let lowered = combined.to_lowercase();
    // Winget reports an already-present package with a failure exit code.
    if lowered.contains("already installed") || lowered.contains("no applicable upgrade") {
        return Ok(());
    }

    let code = output.status.code().unwrap_or(-1);
    let cancelled = lowered.contains("cancelled")
        || lowered.contains("canceled")
        || lowered.contains("elevation")
        || lowered.contains("administrator");

    let detail = if combined.is_empty() {
        String::new()
    } else {
        format!("\n{}", combined)
    };

    Err(WingetFailure {
        message: format!("winget exited with 0x{:08X}{}", code as u32, detail),
        cancelled,
    })
}

#[cfg(target_os = "windows")]
fn winget_candidates(tool: &str) -> Option<&'static [&'static str]> {
    match tool {
        "php" => Some(&[
            "PHP.PHP.8.4",
            "PHP.PHP.8.3",
            "PHP.PHP.8.2",
            "ApacheFriends.Xampp.8.2",
        ]),
        "composer" => Some(&["ApacheFriends.Xampp.8.2", "BeyondCode.Herd"]),
        "mysql" => Some(&["Oracle.MySQL", "ApacheFriends.Xampp.8.2"]),
        "node" | "npm" => Some(&["OpenJS.NodeJS"]),
        "python" => Some(&["Python.Python.3.12", "Python.Python.3.11"]),
        "docker" => Some(&["Docker.DockerDesktop"]),
        "git" => Some(&["Git.Git"]),
        _ => None,
    }
}

#[cfg(target_os = "windows")]
fn install_dependency_blocking(tool: String) -> Result<String, String> {
    let tool_lower = tool.trim().to_lowercase();

    if ensure_tool_available(&tool_lower) {
        return Ok(format!(
            "{} was already installed and is now available.",
            tool_lower
        ));
    }

    let candidates = winget_candidates(&tool_lower)
        .ok_or_else(|| format!("No automatic Winget installer is configured for '{}'.", tool))?;

    let winget = winget_path().ok_or_else(|| {
        "Windows Package Manager (winget) was not found. Install \"App Installer\" from the Microsoft Store, then try again."
            .to_string()
    })?;

    // The portable PHP build links against the VC++ runtime; without it php.exe
    // installs fine but refuses to start.
    if tool_lower == "php" {
        let _ = run_winget_install(&winget, "Microsoft.VCRedist.2015+.x64");
    }

    let mut failures: Vec<String> = Vec::new();
    for &id in candidates {
        match run_winget_install(&winget, id) {
            Ok(()) => {
                if ensure_tool_available(&tool_lower) {
                    return Ok(format!("Installed {} via Winget ({}).", tool_lower, id));
                }
                failures.push(format!(
                    "{}: installed, but no '{}' executable was found afterwards",
                    id, tool_lower
                ));
            }
            Err(failure) => {
                failures.push(format!("{}: {}", id, failure.message));
                if failure.cancelled {
                    break;
                }
            }
        }
    }

    Err(format!(
        "Could not auto-install {}.\n\n{}",
        tool_lower,
        failures.join("\n\n")
    ))
}

#[tauri::command]
fn check_system_dependency(tool: String) -> Result<bool, String> {
    let tool_clean = tool.trim().to_lowercase();
    #[cfg(target_os = "windows")]
    {
        Ok(ensure_tool_available(&tool_clean))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let output = Command::new("which").arg(&tool_clean).output();
        if let Ok(out) = output {
            return Ok(out.status.success());
        }
        Ok(false)
    }
}

#[tauri::command]
async fn auto_install_dependency(tool: String) -> Result<String, String> {
    #[cfg(target_os = "windows")]
    {
        // winget downloads can take minutes; keep them off the UI thread.
        tauri::async_runtime::spawn_blocking(move || install_dependency_blocking(tool))
            .await
            .map_err(|e| format!("Installer task failed to run: {}", e))?
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = tool;
        Err("Auto-installer currently supports Windows via Winget.".to_string())
    }
}

#[tauri::command]
fn auto_setup_database(project_path: String) -> Result<String, String> {
    let path = Path::new(&project_path);
    let mut actions = Vec::new();

    // 1. Check .env vs .env.example
    let env_file = path.join(".env");
    let env_example = path.join(".env.example");
    if !env_file.exists() && env_example.exists() {
        if let Ok(_) = fs::copy(&env_example, &env_file) {
            actions.push("Copied .env.example to .env".to_string());
        }
    }

    // 2. Read .env content if exists
    let mut db_type = String::new();
    if env_file.exists() {
        if let Ok(content) = fs::read_to_string(&env_file) {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("DB_CONNECTION=") {
                    db_type = trimmed.trim_start_matches("DB_CONNECTION=").trim_matches('"').trim_matches('\'').to_string();
                }
            }
        }
    }

    // 3. Handle SQLite auto-creation
    if db_type == "sqlite" || path.join("database").join("database.sqlite").exists() || path.join("database.sqlite").exists() {
        let db_dir = path.join("database");
        if !db_dir.exists() {
            let _ = fs::create_dir_all(&db_dir);
        }
        let sqlite_file = db_dir.join("database.sqlite");
        if !sqlite_file.exists() {
            if let Ok(_) = fs::File::create(&sqlite_file) {
                actions.push("Created database/database.sqlite".to_string());
            }
        }
    }

    // 4. Handle MySQL / Docker Compose check
    let has_docker = path.join("docker-compose.yml").exists() || path.join("docker-compose.yaml").exists();
    if db_type == "mysql" {
        if has_docker {
            actions.push("MySQL database configured via Docker Compose".to_string());
        } else {
            actions.push("MySQL database connection configured in .env".to_string());
        }
    }

    if actions.is_empty() {
        Ok("Database configuration verified.".to_string())
    } else {
        Ok(actions.join("\n"))
    }
}

#[tauri::command]
fn log_error(err: String) {
    eprintln!("[FRONTEND ERROR] {}", err);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            active_processes: Arc::new(Mutex::new(HashMap::new())),
            detected_ports: Arc::new(Mutex::new(HashMap::new())),
            gateway: Arc::new(Mutex::new(None)),
            tunnels: Arc::new(Mutex::new(tunnel::TunnelManager::default())),
            auth: Arc::new(Mutex::new(auth::AuthStore::load())),
        })
        .setup(|app| {
            let _tray = tauri::tray::TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().unwrap().clone())
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| {
                    let id = event.id.as_ref();
                    match id {
                        "show" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "stop_all" => {
                            let state = app.state::<AppState>();
                            let mut processes = state.active_processes.lock().unwrap();
                            for (_, child) in processes.iter_mut() {
                                let pid = child.id();
                                let _ = std::process::Command::new("C:\\Windows\\System32\\taskkill.exe")
                                    .args(["/PID", &pid.to_string(), "/F", "/T"])
                                    .apply_cross_platform_flags()
                                    .output();
                            }
                            processes.clear();
                            update_tray_menu(app);
                        }
                        "quit" => {
                            let state = app.state::<AppState>();
                            let mut processes = state.active_processes.lock().unwrap();
                            for (_, child) in processes.iter_mut() {
                                let pid = child.id();
                                let _ = std::process::Command::new("C:\\Windows\\System32\\taskkill.exe")
                                    .args(["/PID", &pid.to_string(), "/F", "/T"])
                                    .apply_cross_platform_flags()
                                    .output();
                            }
                            std::process::exit(0);
                        }
                        _ => {
                            if id.starts_with("stop_") {
                                let key = id.trim_start_matches("stop_");
                                let state = app.state::<AppState>();
                                if let Some(mut child) = state.active_processes.lock().unwrap().remove(key) {
                                    let pid = child.id();
                                    let _ = std::process::Command::new("C:\\Windows\\System32\\taskkill.exe")
                                        .args(["/PID", &pid.to_string(), "/F", "/T"])
                                        .apply_cross_platform_flags()
                                        .output();
                                }
                                update_tray_menu(app);
                            }
                        }
                    }
                })
                .build(app)?;
            
            update_tray_menu(app.handle());
            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            scan_projects, get_node_processes, kill_process, run_script, get_detected_ports, gateway_start, gateway_stop, gateway_status, gateway_pair_qr, tunnel_available, tunnel_install, tunnel_start, tunnel_share_project, tunnel_stop, tunnel_status, auth_status, auth_set_account, auth_revoke_sessions, project_visibility, set_project_visibility, run_custom_command, stop_script, open_external_url, select_directory, write_to_stdin, open_external_terminal, open_in_editor, check_system_dependency, auto_install_dependency, auto_setup_database, log_error
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
