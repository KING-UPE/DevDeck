
const fs = require("fs");
let code = fs.readFileSync("src-tauri/src/lib.rs", "utf-8");

code = code.replace("use std::os::windows::process::CommandExt;\n", "");
code = code.replace("const CREATE_NO_WINDOW: u32 = 0x08000000;\n", "");

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

code = code.replace(/\.creation_flags\(CREATE_NO_WINDOW\)/g, ".creation_flags_cross()");

fs.writeFileSync("src-tauri/src/lib.rs", code);
console.log("Refactored successfully!");

