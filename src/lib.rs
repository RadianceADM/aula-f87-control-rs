//! AULA F87 Control: backlight effects, per-key colours, music sync, sleep
//! timer and battery level for AULA F87 / F87 Pro keyboards on Linux and
//! Windows. The Windows app (WinUI 3) lives in `windows/` and uses this crate
//! without its GTK `gui` feature.
//!
//! A Rust version of the Python app with the same window, files and protocol.

pub mod about;
pub mod analyzer;
#[cfg(unix)]
pub mod autostart;
pub mod capture;
#[cfg(unix)]
pub mod cli;
pub mod device;
pub mod effects;
#[cfg(feature = "fake-keyboard")]
pub mod fake;
#[cfg(feature = "gui")]
pub mod gui;
#[cfg(unix)]
pub mod hidraw;
#[cfg(windows)]
pub mod hidwin;
/// The HID transport: Linux hidraw, or the Win32 HID API on Windows.
#[cfg(windows)]
pub use hidwin as hidraw;
pub mod layout;
pub mod music;
pub mod preview;
pub mod render;
pub mod settings;
#[cfg(unix)]
pub mod setup;
#[cfg(windows)]
pub mod wasapi;
