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

#[cfg(test)]
pub fn run() {}
