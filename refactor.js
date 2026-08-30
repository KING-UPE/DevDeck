
const fs = require("fs");
let code = fs.readFileSync("src-tauri/src/lib.rs", "utf-8");

// 1. Remove the old imports and constant
code = code.replace("use std::os::windows::process::CommandExt;\n", "");
code = code.replace("const CREATE_NO_WINDOW: u32 = 0x08000000;\n", "");

// 2. Add the cross platform trait at the top (after imports)
const crossPlatformTrait = `
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

trait CommandExtCrossPlatform {
    fn creation_flags_cross(&mut self) -> &mut Self;
}

impl CommandExtCrossPlatform for std::process::Command {
    fn creation_flags_cross(&mut self) -> &mut Self {
        #[cfg(target_os = "windows")]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            self.creation_flags(CREATE_NO_WINDOW);
        }
        self
    }
}
`;
code = code.replace("use tauri::{Emitter, AppHandle, State, Manager};\n", "use tauri::{Emitter, AppHandle, State, Manager};\n" + crossPlatformTrait);

// 3. Replace creation_flags
code = code.replace(/\.creation_flags\(CREATE_NO_WINDOW\)/g, ".creation_flags_cross()");

// 4. Cross platform shell wrapper
code = code.replace(/Command::new\("powershell"\)[\s\n]+\.args\(\["-NoProfile", "-Command", script\]\)/, `#[cfg(target_os = "windows")]
    let mut cmd = Command::new("powershell");
    #[cfg(target_os = "windows")]
    cmd.args(["-NoProfile", "-Command", script]);
    
    #[cfg(not(target_os = "windows"))]
    let mut cmd = Command::new("sh");
    #[cfg(not(target_os = "windows"))]
    cmd.args(["-c", script]);
    
    let output = cmd`);
    
// 5. Cross platform cmd wrapper in run_custom_command
code = code.replace(/Command::new\("cmd"\)[\s\n]+\.args\(\["\/C", &command_str\]\)/, `#[cfg(target_os = "windows")]
    let mut cmd = Command::new("cmd");
    #[cfg(target_os = "windows")]
    cmd.args(["/C", &command_str]);
    
    #[cfg(not(target_os = "windows"))]
    let mut cmd = Command::new("sh");
    #[cfg(not(target_os = "windows"))]
    cmd.args(["-c", &command_str]);
    
    let mut child = cmd`);
    
// 6. Cross platform cmd wrapper in run_script
code = code.replace(/Command::new\("cmd"\)[\s\n]+\.args\(\["\/C", &script_cmd\]\)/, `#[cfg(target_os = "windows")]
    let mut cmd = Command::new("cmd");
    #[cfg(target_os = "windows")]
    cmd.args(["/C", &script_cmd]);
    
    #[cfg(not(target_os = "windows"))]
    let mut cmd = Command::new("sh");
    #[cfg(not(target_os = "windows"))]
    cmd.args(["-c", &script_cmd]);
    
    let mut child = cmd`);

// 7. Cross platform taskkill everywhere
code = code.replace(/Command::new\("C:\\\\Windows\\\\System32\\\\taskkill\.exe"\)[\s\n]+\.args\(\["\/PID", &pid\.to_string\(\), "\/F", "\/T"\]\)/g, `#[cfg(target_os = "windows")]
                                let mut cmd = Command::new("C:\\\\Windows\\\\System32\\\\taskkill.exe");
                                #[cfg(target_os = "windows")]
                                cmd.args(["/PID", &pid.to_string(), "/F", "/T"]);
                                #[cfg(not(target_os = "windows"))]
                                let mut cmd = Command::new("kill");
                                #[cfg(not(target_os = "windows"))]
                                cmd.args(["-9", &pid.to_string()]);
                                let _ = cmd`);
                                
// 8. One taskkill (in kill_process) returns an output, so we need a slightly different replacement for the first one that defines output:
code = code.replace(/let output = Command::new\("C:\\\\Windows\\\\System32\\\\taskkill\.exe"\)/, `#[cfg(target_os = "windows")]
    let mut cmd = Command::new("C:\\\\Windows\\\\System32\\\\taskkill.exe");
    #[cfg(target_os = "windows")]
    cmd.args(["/PID", &pid.to_string(), "/F", "/T"]);
    #[cfg(not(target_os = "windows"))]
    let mut cmd = Command::new("kill");
    #[cfg(not(target_os = "windows"))]
    cmd.args(["-9", &pid.to_string()]);
    let output = cmd`);
code = code.replace(/#[cfg(target_os = "windows")][\s\n]+let mut cmd = Command::new\("C:\\\\Windows\\\\System32\\\\taskkill\.exe"\);[\s\n]+#[cfg(target_os = "windows")][\s\n]+cmd.args\(\["\/PID", &pid\.to_string\(\), "\/F", "\/T"\]\);[\s\n]+#[cfg(not(target_os = "windows"))][\s\n]+let mut cmd = Command::new\("kill"\);[\s\n]+#[cfg(not(target_os = "windows"))][\s\n]+cmd.args\(\["-9", &pid\.to_string\(\)\]\);[\s\n]+let output = cmd[\s\n]+\.args\(\["\/PID", &pid\.to_string\(\), "\/F", "\/T"\]\)/, `#[cfg(target_os = "windows")]
    let mut cmd = Command::new("C:\\\\Windows\\\\System32\\\\taskkill.exe");
    #[cfg(target_os = "windows")]
    cmd.args(["/PID", &pid.to_string(), "/F", "/T"]);
    #[cfg(not(target_os = "windows"))]
    let mut cmd = Command::new("kill");
    #[cfg(not(target_os = "windows"))]
    cmd.args(["-9", &pid.to_string()]);
    let output = cmd`); // cleanup the double args from regex above

// 9. Explorer
code = code.replace(/Command::new\("explorer"\)/, `#[cfg(target_os = "windows")]
    let mut cmd = Command::new("explorer");
    #[cfg(target_os = "macos")]
    let mut cmd = Command::new("open");
    #[cfg(target_os = "linux")]
    let mut cmd = Command::new("xdg-open");
    let _ = cmd`);

fs.writeFileSync("src-tauri/src/lib.rs", code);
console.log("Refactored successfully!");

