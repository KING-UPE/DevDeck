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
        
        if dir.join("composer.json").exists() {
            proj.scripts.insert("php serve".to_string(), "php -S localhost:8000".to_string());
            types.push("PHP (Composer)");
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
                let indicators = ["package.json", "Cargo.toml", "manage.py", "go.mod", "main.py", "app.py", "requirements.txt", "docker-compose.yml", "pom.xml", "composer.json", "Gemfile", "index.html"];
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

#[tauri::command]
fn run_custom_command(app: AppHandle, state: State<AppState>, project_path: String, command_str: String) -> Result<(), String> {
    let script_name = format!("$ {}", command_str);
    let process_key = format!("{}:{}", project_path, script_name);
    
    if state.active_processes.lock().unwrap().contains_key(&process_key) {
        return Err("Process is already running".to_string());
    }

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
    let _ = app.emit("process-started", serde_json::json!({ "processKey": process_key }));
    
    let pk1 = process_key.clone();
    let app1 = app.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            if let Ok(l) = line {
                let _ = app1.emit("process-output", serde_json::json!({ "processKey": pk1, "type": "stdout", "data": format!("{}\n", l) }));
            }
        }
    });

    let pk2 = process_key.clone();
    let app2 = app.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            if let Ok(l) = line {
                let _ = app2.emit("process-output", serde_json::json!({ "processKey": pk2, "type": "stderr", "data": format!("{}\n", l) }));
            }
        }
    });

    let pk3 = process_key.clone();
    let app3 = app.clone();
    let active_procs = state.active_processes.clone();
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
fn run_script(app: AppHandle, state: State<AppState>, project_path: String, script_name: String, script_cmd: String) -> Result<(), String> {
    let process_key = format!("{}:{}", project_path, script_name);
    
    if state.active_processes.lock().unwrap().contains_key(&process_key) {
        return Err("Process is already running".to_string());
    }

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
    let _ = app.emit("process-started", serde_json::json!({ "processKey": process_key }));
    
    let pk1 = process_key.clone();
    let app1 = app.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            if let Ok(l) = line {
                let _ = app1.emit("process-output", serde_json::json!({ "processKey": pk1, "type": "stdout", "data": format!("{}\n", l) }));
            }
        }
    });

    let pk2 = process_key.clone();
    let app2 = app.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            if let Ok(l) = line {
                let _ = app2.emit("process-output", serde_json::json!({ "processKey": pk2, "type": "stderr", "data": format!("{}\n", l) }));
            }
        }
    });

    let pk3 = process_key.clone();
    let app3 = app.clone();
    let active_procs = state.active_processes.clone();
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
fn open_external_terminal(path: String) -> Result<(), String> {
    Command::new("cmd")
        .args(["/C", "start", "cmd"])
        .current_dir(path)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState { active_processes: Arc::new(Mutex::new(HashMap::new())) })
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
            scan_projects, get_node_processes, kill_process, run_script, run_custom_command, stop_script, open_external_url, select_directory, write_to_stdin, open_external_terminal
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
