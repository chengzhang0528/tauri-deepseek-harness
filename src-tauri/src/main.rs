#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|argument| argument == "--cleanup-installer-owned-state")
    {
        if let Err(error) = dsh_desktop_lib::cleanup_installer_owned_state() {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
        return;
    }
    dsh_desktop_lib::run();
}
