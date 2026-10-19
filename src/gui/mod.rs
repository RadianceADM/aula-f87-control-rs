//! GTK 4 / libadwaita app: the same window, top-bar icon and behaviour as the
//! Python app.

#[cfg(feature = "fake-keyboard")]
mod shots;
mod tray;
mod view;
mod window;
mod worker;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;

use crate::{autostart, device};

pub const APP_ID: &str = "io.github.aula_f87.Control";
pub const ICON: &str = "io.github.aula_f87.Control";

const ICON_SVG: &[u8] = include_bytes!("../../data/io.github.aula_f87.Control.svg");
const SYMBOLIC_SVG: &[u8] = include_bytes!("../../data/io.github.aula_f87.Control-symbolic.svg");

thread_local! {
    static WINDOW: RefCell<Option<Rc<window::Win>>> = const { RefCell::new(None) };
}

/// The app's icons as plain files, for installs that didn't put them in the
/// icon theme (e.g. `cargo install`). Used by the window, About and the top bar.
fn icon_dir() -> PathBuf {
    let base = match std::env::var("XDG_DATA_HOME") {
        Ok(d) if !d.is_empty() => PathBuf::from(d),
        _ => device::home().join(".local/share"),
    };
    let dir = base.join("aula-f87-control").join("icons");
    let _ = std::fs::create_dir_all(&dir);
    for (name, data) in [(format!("{ICON}.svg"), ICON_SVG), (format!("{ICON}-symbolic.svg"), SYMBOLIC_SVG)] {
        let path = dir.join(name);
        if std::fs::read(&path).ok().as_deref() != Some(data) {
            let _ = std::fs::write(&path, data);
        }
    }
    dir
}

fn window() -> Option<Rc<window::Win>> {
    WINDOW.with(|w| w.borrow().clone())
}

pub fn main(background: bool) -> i32 {
    autostart::enable_on_first_run();
    let app = adw::Application::builder().application_id(window::app_id()).build();
    let icons = icon_dir();

    app.connect_startup(move |app| {
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::IconTheme::for_display(&display).add_search_path(&icons);
        }
        gtk::Window::set_default_icon_name(ICON);
        // "quit" is reachable over D-Bus, so installers can stop a running copy:
        // gdbus call --session --dest io.github.aula_f87.Control \
        //   --object-path /io/github/aula_f87/Control --method org.gtk.Actions.Activate quit [] {}
        let quit = gio::SimpleAction::new("quit", None);
        let a = app.clone();
        quit.connect_activate(move |_, _| match window() {
            Some(w) => w.quit_app(),
            None => a.quit(),
        });
        app.add_action(&quit);
        let about = gio::SimpleAction::new("about", None);
        about.connect_activate(|_, _| {
            if let Some(w) = window() {
                w.show_about();
            }
        });
        app.add_action(&about);
    });

    let background_start = Rc::new(Cell::new(background));
    let icons = icon_dir();
    app.connect_activate(move |app| {
        let existing = window();
        let win = match &existing {
            Some(w) => w.clone(),
            None => {
                let w = window::Win::new(app, &icons);
                WINDOW.with(|cell| *cell.borrow_mut() = Some(w.clone()));
                w
            }
        };
        #[cfg(feature = "fake-keyboard")]
        if existing.is_none() {
            if let Ok(dir) = std::env::var("AULA_F87_SCREENSHOTS") {
                shots::run(win.clone(), dir);
            }
            if std::env::var_os("AULA_F87_GUI_TEST").is_some() {
                win.self_test();
            }
        }
        if background_start.get() && existing.is_none() {
            // Started at login: stay in the top bar. Opening the app again shows the window.
            background_start.set(false);
            return;
        }
        win.show_window();
    });

    if background {
        if app.register(gio::Cancellable::NONE).is_ok() && app.is_remote() {
            return 0; // already running; don't pop up a window at login
        }
    }
    let code = app.run_with_args(&["aula-f87-control"]);
    WINDOW.with(|w| w.borrow_mut().take());
    i32::from(code)
}
