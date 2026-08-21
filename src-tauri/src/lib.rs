#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![cfg_attr(test, allow(dead_code))]

mod bootstrap;
mod dialogs;
mod job;
mod manifest;
mod paths;
mod process;
mod runtime;

#[cfg(not(test))]
mod host;

#[cfg(not(test))]
pub use host::run;

#[cfg(not(test))]
/// Removes the launcher-owned local runtime state before MSI removes the binary.
///
/// # Errors
/// Returns an error when the installer-owned local state directories cannot be removed.
pub fn cleanup_installer_owned_state() -> anyhow::Result<()> {
    paths::AppPaths::discover()?.remove_installer_owned_state()
}

#[cfg(test)]
pub fn run() {}
