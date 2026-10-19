//! `aula-f87-control setup`: the app-menu entry, icons and start at login for
//! the current user, e.g. after `cargo install aula-f87-control`. No root needed;
//! the udev rule (one time, with sudo) is printed.

use std::fs;
use std::path::PathBuf;

use crate::{autostart, device::home};

const APP_ID: &str = "io.github.aula_f87.Control";
const ICON_SVG: &[u8] = include_bytes!("../data/io.github.aula_f87.Control.svg");
const SYMBOLIC_SVG: &[u8] = include_bytes!("../data/io.github.aula_f87.Control-symbolic.svg");
pub const UDEV_RULE: &str = include_str!("../udev/70-aula-f87.rules");

fn data_home() -> PathBuf {
    match std::env::var("XDG_DATA_HOME") {
        Ok(d) if !d.is_empty() => PathBuf::from(d),
        _ => home().join(".local/share"),
    }
}

fn paths() -> (PathBuf, PathBuf, PathBuf) {
    let data = data_home();
    (
        data.join("applications").join(format!("{APP_ID}.desktop")),
        data.join("icons/hicolor/scalable/apps").join(format!("{APP_ID}.svg")),
        data.join("icons/hicolor/symbolic/apps").join(format!("{APP_ID}-symbolic.svg")),
    )
}

pub fn run(remove: bool) -> std::io::Result<()> {
    let (desktop, icon, symbolic) = paths();
    if remove {
        for p in [&desktop, &icon, &symbolic] {
            let _ = fs::remove_file(p);
        }
        let _ = autostart::disable();
        println!("Removed the app-menu entry, the icons and start at login.");
        println!("To also remove the udev rule: sudo rm /etc/udev/rules.d/70-aula-f87.rules");
        return Ok(());
    }
    let exe = std::env::current_exe()?.canonicalize()?;
    for (path, data) in [(&icon, ICON_SVG), (&symbolic, SYMBOLIC_SVG)] {
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, data)?;
    }
    fs::create_dir_all(desktop.parent().unwrap())?;
    fs::write(
        &desktop,
        format!(
            "[Desktop Entry]\nType=Application\nName=AULA F87 Control\n\
             Comment=Backlight, per-key colors, music sync and battery for the AULA F87 keyboard\n\
             Exec={}\nIcon={APP_ID}\nTerminal=false\nCategories=Settings;HardwareSettings;\n\
             Keywords=keyboard;rgb;backlight;aula;music;battery;\nStartupWMClass={APP_ID}\n",
            exe.display()
        ),
    )?;
    // Keep an earlier "don't start at login" choice; otherwise turn it on.
    if !autostart::is_configured() || autostart::is_enabled() {
        autostart::enable()?;
    } else {
        autostart::disable()?; // rewrites the entry for this binary, still off
    }
    println!("Added \"AULA F87 Control\" to the app menu.");
    if autostart::is_enabled() {
        println!("It starts at login with an icon in the top bar (turn off in the icon's menu).");
    }
    if !std::path::Path::new("/etc/udev/rules.d/70-aula-f87.rules").exists() {
        println!("\nOne more step: allow your user to talk to the keyboard (needs your password):");
        println!("  aula-f87-control setup --udev-rule | sudo tee /etc/udev/rules.d/70-aula-f87.rules");
        println!("  sudo udevadm control --reload-rules && sudo udevadm trigger");
    }
    Ok(())
}
