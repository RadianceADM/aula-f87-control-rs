//! AULA F87 Control for Windows: a WinUI 3 window and tray icon, written in
//! Rust with windows-reactor, on the same core as the Linux app.

#![windows_subsystem = "windows"]

mod app;
mod autostart;
mod bridge;
mod keyview;
mod selfcheck;
mod sysinfo;
mod ui;
mod worker;

fn log_path() -> std::path::PathBuf {
    aula_f87::device::config_dir().join("app.log")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = selfcheck::run(&args) {
        std::process::exit(code);
    }
    // One copy: a second start (Start menu, login) shows the running one's window.
    if bridge::wake_running_copy() {
        return;
    }
    let background = args.iter().any(|a| a == "--background");
    std::panic::set_hook(Box::new(|info| {
        let _ = std::fs::create_dir_all(aula_f87::device::config_dir());
        let _ = std::fs::write(log_path(), format!("panic: {info}\n"));
    }));
    if let Err(e) = windows_reactor::App::run_application::<app::App>(app::Start { background }) {
        let _ = std::fs::create_dir_all(aula_f87::device::config_dir());
        let _ = std::fs::write(log_path(), format!("WinUI failed to start: {e}\n"));
        std::process::exit(1);
    }
}
