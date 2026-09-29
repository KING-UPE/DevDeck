// Windows release builds should not pop a console window behind the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    devdeck_remote_lib::run()
}
