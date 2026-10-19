//! Start the app at login: it runs in the background with its top-bar icon.
//!
//! Uses the freedesktop autostart folder. Turning it off writes the entry with
//! `Hidden=true` (the standard way to say "don't start this"), so the choice is
//! remembered. On its first run the app turns start-at-login on, unless an
//! entry already exists. Same entry as the Python app, so the two share it.

use std::fs;
use std::path::PathBuf;

use crate::device::home;

pub const DESKTOP_ID: &str = "io.github.aula_f87.Control";

fn path() -> PathBuf {
    let base = match std::env::var("XDG_CONFIG_HOME") {
        Ok(d) if !d.is_empty() => PathBuf::from(d),
        _ => home().join(".config"),
    };
    base.join("autostart").join(format!("{DESKTOP_ID}.desktop"))
}

fn command() -> String {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "aula-f87-control".into());
    if exe.contains(' ') { format!("\"{exe}\"") } else { exe }
}

fn write(hidden: bool) -> std::io::Result<()> {
    let path = path();
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(
        path,
        format!(
            "[Desktop Entry]\nType=Application\nName=AULA F87 Control\n\
             Comment=Keyboard lighting, music sync and battery in the top bar\n\
             Exec={} --background\nIcon={DESKTOP_ID}\nTerminal=false\nNoDisplay=true\n\
             X-GNOME-Autostart-enabled=true\n{}",
            command(),
            if hidden { "Hidden=true\n" } else { "" }
        ),
    )
}

pub fn is_configured() -> bool {
    path().exists()
}

pub fn is_enabled() -> bool {
    fs::read_to_string(path()).is_ok_and(|t| !t.contains("Hidden=true"))
}

pub fn enable() -> std::io::Result<()> {
    write(false)
}

pub fn disable() -> std::io::Result<()> {
    write(true)
}

/// Turn start-at-login on the first time the app runs (never after the user chose).
pub fn enable_on_first_run() {
    if !is_configured() {
        let _ = enable();
    }
}
