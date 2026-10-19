//! AULA F87 Control: backlight effects, per-key colours, music sync, sleep
//! timer and battery level for AULA F87 / F87 Pro keyboards on Linux.
//!
//! A Rust version of the Python app with the same window, files and protocol.

pub mod about;
pub mod analyzer;
pub mod autostart;
pub mod capture;
pub mod cli;
pub mod device;
pub mod effects;
#[cfg(feature = "fake-keyboard")]
pub mod fake;
#[cfg(feature = "gui")]
pub mod gui;
pub mod hidraw;
pub mod layout;
pub mod music;
pub mod render;
pub mod settings;
pub mod setup;
