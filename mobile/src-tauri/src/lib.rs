//! DevDeck Remote.
//!
//! A thin shell around the interface: everything it does is talk to the
//! DevDeck on the user's computer and to the account service, both over
//! ordinary HTTP from the web layer. There are deliberately no commands here -
//! nothing the desktop app does (scanning drives, spawning processes, picking
//! folders) belongs on a phone.

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running DevDeck Remote");
}
