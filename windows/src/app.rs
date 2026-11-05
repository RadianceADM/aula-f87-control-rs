//! The app: everything that talks to the keyboard, music sync, the tray icon
//! and its menu. It outlives the window: closing the window keeps the app in
//! the tray. The window (ui.rs) shows a snapshot of this state and sends back
//! what the user does. The logic is a port of the Linux app's window.rs.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use aula_f87::analyzer::Follow;
use aula_f87::device::{self, Battery, Error, Frame, Keyboard, Lighting};
use aula_f87::effects::{
    Effect, MUSIC_FOLLOW, MUSIC_STYLES, PER_KEY, PRESETS, SLEEP_MINUTES, battery_text, effect_info, effects,
    nearest_sleep_index, sleep_label,
};
use aula_f87::layout::{PLANE, keys};
use aula_f87::music::{MusicError, MusicParams, MusicSync, Sink};
use aula_f87::settings::Settings;
use windows_reactor::*;

use crate::{autostart, bridge, ui, worker::Worker};

const APPLY_DELAY_MS: u32 = 350;
const POLL_MS: u32 = 3000;
const MUSIC_WATCH_MS: u32 = 250;
const BATTERY_EVERY_POLLS: u64 = 20; // about once a minute

pub const TIMER_POLL: usize = 1;
pub const TIMER_APPLY: usize = 2;
pub const TIMER_TOAST: usize = 3;
pub const TIMER_MUSIC: usize = 4;

pub const WINDOW: &str = "main";

type Connected = (Arc<Keyboard>, Lighting, Frame, Option<Battery>);

/// A finished keyboard job's continuation, run on the UI thread.
pub type Done = Box<dyn FnOnce(&mut App, &ApplicationContext<App>) + Send>;

pub enum Msg {
    Done(Done),
    Timer(usize),
    ShowWindow,
    Quit,
    Noop,
    // window
    WindowOpened,
    WindowClosed,
    WindowSize(i32, i32),
    Tab(String),
    // lighting
    Effect(usize),
    Rainbow(bool),
    Color([u8; 3]),
    Preset([u8; 3]),
    Brightness(f64),
    Speed(f64),
    Side(usize),
    KeepOn(bool),
    SleepIndex(usize),
    Reconnect,
    BannerAction,
    // per-key
    Paint(usize, bool),
    Brush([u8; 3]),
    Fill,
    Clear,
    ApplyKeys,
    // music
    ToggleMusic,
    MusicStyle(usize),
    Follow(&'static str, bool),
    Sensitivity(f64),
    MusicBrightness(f64),
    // tray
    Menu(String),
    ToggleAutostart,
}

pub struct Start {
    pub background: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Banner {
    pub message: String,
    pub damaged: bool,
}

/// The latest music sync frame, shared with the window's preview; compared by identity.
#[derive(Clone)]
pub struct SharedFrame(pub Arc<Mutex<Frame>>);

impl PartialEq for SharedFrame {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

pub struct App {
    settings: Settings,
    worker: Worker,
    kb: Option<Arc<Keyboard>>,
    state: Lighting,
    pending: Arc<Mutex<Lighting>>, // newest state, read by a queued apply job
    keys: Frame,
    effects: Vec<Effect>,
    sleep_index: usize,
    keep_on: bool,
    connecting: bool,
    apply_armed: bool,
    apply_queued: Arc<AtomicBool>,
    damaged: bool,
    music: Option<MusicSync>,
    music_starting: bool,
    music_cancel: bool, // window closed while music sync was starting
    polls: u64,
    edits: u64, // counts changes made in the window, see follow_keyboard
    battery: Option<Battery>,
    battery_text: String,
    status: String,
    banner: Option<Banner>,
    toast: Option<String>,
    tab: String,
    window_open: bool,
    window_size: [i32; 2],
    autostart: bool,
    icon: PathBuf,
}

fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from)).unwrap_or_default()
}

pub fn icon_path() -> PathBuf {
    exe_dir().join("aula-f87-control.ico")
}

impl Application for App {
    type Input = Start;
    type Message = Msg;

    fn create(input: &Start, ctx: &ApplicationContext<Self>) -> Self {
        let sender = ctx.sender();
        bridge::install(move |m| {
            let _ = sender.send(m);
        });
        autostart::enable_on_first_run();
        autostart::refresh();
        let settings = Settings::load();
        let tab = settings.tab.clone();
        let window_size = settings.window;
        let mut app = App {
            settings,
            worker: Worker::new(),
            kb: None,
            state: Lighting::default(),
            pending: Arc::new(Mutex::new(Lighting::default())),
            keys: [[0; 3]; PLANE],
            effects: effects(),
            sleep_index: nearest_sleep_index(5.0),
            keep_on: false,
            connecting: false,
            apply_armed: false,
            apply_queued: Arc::new(AtomicBool::new(false)),
            damaged: false,
            music: None,
            music_starting: false,
            music_cancel: false,
            polls: 0,
            edits: 0,
            battery: None,
            battery_text: "—".into(),
            status: "Looking for keyboard…".into(),
            banner: None,
            toast: None,
            tab,
            window_open: false,
            window_size,
            autostart: autostart::is_enabled(),
            icon: icon_path(),
        };
        bridge::set_timer(TIMER_POLL, POLL_MS);
        app.connect_keyboard(false);
        if !input.background {
            ctx.show_window(WINDOW);
        }
        app
    }

    fn update(&mut self, msg: Msg, ctx: &ApplicationContext<Self>) {
        match msg {
            Msg::Done(done) => done(self, ctx),
            Msg::Timer(TIMER_POLL) => self.poll(ctx),
            Msg::Timer(TIMER_APPLY) => {
                bridge::kill_timer(TIMER_APPLY);
                self.apply_armed = false;
                self.apply_lighting();
            }
            Msg::Timer(TIMER_TOAST) => {
                bridge::kill_timer(TIMER_TOAST);
                self.toast = None;
            }
            Msg::Timer(TIMER_MUSIC) => self.watch_music(ctx),
            Msg::Timer(_) | Msg::Noop => {}
            Msg::ShowWindow => ctx.show_window(WINDOW),
            Msg::Quit => self.quit(ctx),
            Msg::WindowOpened => {
                self.window_open = true;
                self.resume_music();
            }
            Msg::WindowClosed => self.window_closed(),
            Msg::WindowSize(w, h) => self.window_size = [w.max(1), h.max(1)],
            Msg::Tab(tab) => {
                if ["lighting", "per-key", "music"].contains(&tab.as_str()) {
                    let t = tab.clone();
                    self.set_setting(|s| s.tab = t);
                }
                self.tab = tab;
            }
            // Controls also report values the app set itself (a refresh from the
            // keyboard); only real changes are written, so nothing is stored twice.
            Msg::Effect(i) => {
                if let Some(id) = self.effects.get(i).map(|e| e.id) {
                    if id != self.state.effect {
                        self.state.effect = id;
                        self.changed();
                    }
                }
            }
            Msg::Rainbow(on) => {
                if on != self.state.colorful {
                    self.state.colorful = on;
                    self.changed();
                }
            }
            Msg::Color(c) => {
                if c != self.state.color {
                    self.color_picked(c);
                }
            }
            Msg::Preset(c) => self.color_picked(c),
            Msg::Brightness(v) => {
                let v = v.round().clamp(0.0, 4.0) as u8;
                if v != self.state.brightness {
                    self.state.brightness = v;
                    self.changed();
                }
            }
            Msg::Speed(v) => {
                let v = v.round().clamp(0.0, 4.0) as u8;
                if v != self.state.speed {
                    self.state.speed = v;
                    self.changed();
                }
            }
            Msg::Side(i) => {
                if i as u8 != self.state.side {
                    self.state.side = i as u8;
                    self.changed();
                }
            }
            Msg::KeepOn(on) => {
                if on != self.keep_on {
                    self.keep_on = on;
                    self.sleep_changed();
                }
            }
            Msg::SleepIndex(i) => {
                let i = i.min(SLEEP_MINUTES.len() - 1);
                if i != self.sleep_index {
                    self.sleep_index = i;
                    if !self.keep_on {
                        self.sleep_changed();
                    }
                }
            }
            Msg::Reconnect => self.connect_keyboard(false),
            Msg::BannerAction => {
                if self.damaged {
                    self.repair();
                } else {
                    self.connect_keyboard(false);
                }
            }
            Msg::Paint(led, erase) => {
                let color = if erase { [0, 0, 0] } else { self.settings.brush };
                if led < PLANE {
                    self.keys[led] = color;
                }
            }
            Msg::Brush(c) => self.set_setting(|s| s.brush = c),
            Msg::Fill => self.fill_keys(self.settings.brush),
            Msg::Clear => self.fill_keys([0, 0, 0]),
            Msg::ApplyKeys => self.apply_keys(),
            Msg::ToggleMusic => self.toggle_music(),
            Msg::MusicStyle(i) => {
                if i as u32 != self.settings.music_style {
                    self.set_setting(|s| s.music_style = i.min(MUSIC_STYLES.len() - 1) as u32);
                    self.music_settings_changed();
                }
            }
            Msg::Follow(key, on) => self.follow(key, on),
            Msg::Sensitivity(v) => {
                self.set_setting(|s| s.music_sensitivity = ((v * 100.0).round() / 100.0).clamp(0.2, 2.0));
                self.music_settings_changed();
            }
            Msg::MusicBrightness(v) => {
                self.set_setting(|s| s.music_brightness = ((v * 100.0).round() / 100.0).clamp(0.1, 1.0));
                self.music_settings_changed();
            }
            Msg::Menu(key) => self.menu(ctx, &key),
            Msg::ToggleAutostart => self.toggle_autostart(),
        }
    }

    fn view(&self, _input: &Start, ctx: &ApplicationViewContext<Self>) -> ApplicationView {
        let tray = NotifyIcon::new(self.icon.clone())
            .tooltip(self.tooltip())
            .on_activate(ctx.callback(|_: ScreenPoint| Msg::ShowWindow))
            .menu(Menu::new(self.tray_menu(), ctx.callback(|k: Key| Msg::Menu(k.as_str().unwrap_or("").to_string()))));
        ApplicationView::new().window::<ui::MainWindow>(WINDOW, self.ui_state(ctx.forward())).notify_icon("tray", tray)
    }
}

impl App {
    fn set_setting(&mut self, f: impl FnOnce(&mut Settings)) {
        let before = self.settings.clone();
        f(&mut self.settings);
        if self.settings != before {
            self.settings.save();
        }
    }

    fn toast(&mut self, text: impl Into<String>, seconds: u32) {
        self.toast = Some(text.into());
        bridge::set_timer(TIMER_TOAST, seconds * 1000);
    }

    /// What the window shows.
    fn ui_state(&self, on: Callback<Msg>) -> ui::UiState {
        let running = self.music.is_some();
        let busy = running || self.music_starting;
        let has_kb = self.kb.is_some();
        let music_status = if running {
            self.kb.as_ref().map(|k| format!("On · {}", k.label)).unwrap_or_else(|| "On".into())
        } else if self.music_starting {
            "Starting…".into()
        } else if has_kb {
            "Stopped".into()
        } else {
            "Connect the keyboard first".into()
        };
        ui::UiState {
            on,
            status: self.status.clone(),
            banner: self.banner.clone(),
            toast: self.toast.clone(),
            has_kb,
            connected: has_kb && !busy, // music sync owns the keyboard while on
            busy,
            running,
            starting: self.music_starting,
            state: self.state.clone(),
            effect_names: self.effects.iter().map(|e| e.name.clone()).collect(),
            effect_index: self.effects.iter().position(|e| e.id == self.state.effect),
            keep_on: self.keep_on,
            sleep_index: self.sleep_index,
            battery_text: self.battery_text.clone(),
            keys: Box::new(self.keys),
            brush: self.settings.brush,
            music_style: self.settings.music_style as usize,
            sensitivity: self.settings.music_sensitivity,
            music_brightness: self.settings.music_brightness,
            follow: self.settings.music_follow.clone(),
            music_status,
            music_frame: self.music.as_ref().map(|m| SharedFrame(m.latest.clone())),
            tab: self.tab.clone(),
            window_size: self.window_size,
            keyboard: self.kb.as_ref().map(|k| k.description()),
        }
    }

    // ------------------------------------------------------------------ window

    fn window_closed(&mut self) {
        // Closing the window stops music sync, so the keyboard goes back to its
        // own lighting within a second; the app stays in the tray (music sync
        // can be started from its menu). Quit ends it.
        self.window_open = false;
        let size = [self.window_size[0].max(640), self.window_size[1].max(480)];
        self.set_setting(|s| s.window = size);
        if self.music.is_some() {
            self.stop_music(); // resumes when the window is opened again
        }
        self.music_cancel = self.music_starting;
    }

    fn quit(&mut self, ctx: &ApplicationContext<Self>) {
        let size = [self.window_size[0].max(640), self.window_size[1].max(480)];
        self.set_setting(|s| s.window = size);
        if let Some(mut sync) = self.music.take() {
            sync.stop();
        }
        bridge::kill_timer(TIMER_POLL);
        ctx.exit();
    }

    // ------------------------------------------------------------------ music

    /// Start/stop chosen by the user; remembered so it resumes next time.
    fn toggle_music(&mut self) {
        if self.music.is_some() {
            self.set_setting(|s| s.music_on = false);
            self.stop_music();
        } else {
            self.set_setting(|s| s.music_on = true);
            self.start_music();
        }
    }

    /// "All" and the specific choices exclude each other; with nothing picked,
    /// "All" ticks itself again.
    fn follow(&mut self, key: &str, on: bool) {
        let mut picked: Vec<String> = self.settings.music_follow.clone();
        if key == "all" {
            if on {
                picked.clear();
            }
        } else if on {
            if !picked.iter().any(|k| k == key) {
                picked.push(key.to_string());
            }
        } else {
            picked.retain(|k| k != key);
        }
        let ordered: Vec<String> =
            MUSIC_FOLLOW.iter().map(|(k, _)| *k).filter(|k| picked.iter().any(|p| p == k)).map(String::from).collect();
        self.set_setting(|s| s.music_follow = ordered);
        self.music_settings_changed();
    }

    fn music_params(&self) -> MusicParams {
        let f = &self.settings.music_follow;
        let has = |k: &str| f.iter().any(|x| x == k);
        MusicParams {
            style: self.settings.music_style as usize,
            sensitivity: self.settings.music_sensitivity,
            brightness: self.settings.music_brightness,
            follow: Follow { bass: has("bass"), vocals: has("vocals"), beat: has("beat") },
        }
    }

    fn music_settings_changed(&mut self) {
        if let Some(sync) = self.music.as_ref() {
            *sync.params.lock().unwrap() = self.music_params();
        }
    }

    fn start_music(&mut self) {
        let Some(kb) = self.kb.clone() else { return };
        if self.music_starting {
            return;
        }
        self.music_starting = true;
        self.music_cancel = false;
        let params = self.music_params();
        // Queued on the worker so it starts after any pending keyboard writes.
        self.worker.submit(
            move || MusicSync::start(Sink::Keyboard(kb), params, None),
            |app, ctx, result| {
                app.music_starting = false;
                match result {
                    Ok(mut sync) => {
                        if sync.running() && !app.music_cancel {
                            app.music = Some(sync);
                            app.music_settings_changed();
                            bridge::set_timer(TIMER_MUSIC, MUSIC_WATCH_MS);
                        } else {
                            let err = sync.error();
                            app.worker.run(move || sync.stop());
                            if let Some(e) = err {
                                app.music_failed(ctx, e);
                            }
                        }
                    }
                    Err(e) => {
                        app.set_setting(|s| s.music_on = false); // don't retry a start that can't work
                        app.toast(format!("Couldn't start music sync: {e}"), 6);
                    }
                }
            },
        );
    }

    /// Notice when music sync stops by itself.
    fn watch_music(&mut self, ctx: &ApplicationContext<Self>) {
        let error = match self.music.as_ref() {
            Some(sync) => sync.error(),
            None => {
                bridge::kill_timer(TIMER_MUSIC);
                return;
            }
        };
        if let Some(e) = error {
            self.music_failed(ctx, e);
        }
    }

    fn stop_music(&mut self) {
        bridge::kill_timer(TIMER_MUSIC);
        if let Some(mut sync) = self.music.take() {
            self.worker.run(move || sync.stop());
        }
    }

    fn music_failed(&mut self, ctx: &ApplicationContext<Self>, e: MusicError) {
        self.stop_music();
        if e.device || self.kb.as_ref().is_some_and(|kb| !kb.alive()) {
            self.disconnected(ctx, "Lost connection to the keyboard.", false);
        }
        self.toast(format!("Music sync stopped: {e}"), 6);
    }

    /// Music sync was on when the window was last open (or before the keyboard
    /// dropped out): start it again.
    fn resume_music(&mut self) {
        if self.settings.music_on && self.kb.is_some() && self.window_open && self.music.is_none() && !self.music_starting
        {
            self.start_music();
        }
    }

    // ------------------------------------------------------------------ connection

    fn connect_keyboard(&mut self, quiet: bool) {
        if self.connecting || self.music.is_some() || self.music_starting {
            return;
        }
        self.connecting = true;
        let old = self.kb.take();
        if !quiet {
            self.status = "Connecting…".into();
        }
        self.worker.submit(
            move || -> device::Result<Connected> {
                drop(old);
                let kb = device::connect()?;
                let state = kb.read_lighting()?;
                let keys = kb.read_keys()?;
                let battery = kb.read_battery();
                Ok((Arc::new(kb), state, keys, battery))
            },
            |app, ctx, result| {
                app.connecting = false;
                match result {
                    Ok((kb, state, keys, battery)) => {
                        app.kb = Some(kb);
                        app.keys = keys;
                        app.show_state(state);
                        app.banner = None;
                        app.damaged = false;
                        app.show_battery(battery);
                        app.resume_music();
                    }
                    Err(e) => {
                        let damaged = matches!(e, Error::Damaged(_));
                        app.disconnected(ctx, &e.to_string(), damaged);
                    }
                }
            },
        );
    }

    fn disconnected(&mut self, ctx: &ApplicationContext<Self>, message: &str, damaged: bool) {
        if let Some(kb) = self.kb.take() {
            // Close on the worker so jobs already queued for this handle finish first.
            self.worker.run(move || drop(kb));
        }
        if damaged && !self.damaged {
            ctx.show_window(WINDOW); // the keyboard may not type until this is fixed
        }
        self.damaged = damaged;
        self.status = if damaged { "Settings damaged".into() } else { "Not connected".into() };
        self.battery_text = "—".into();
        self.battery = None;
        self.banner = Some(Banner { message: message.to_string(), damaged });
    }

    /// Write the keyboard's last good settings back (see Keyboard::repair).
    fn repair(&mut self) {
        if self.connecting {
            return;
        }
        self.connecting = true; // no reconnect attempts meanwhile
        self.banner = None;
        self.status = "Repairing…".into();
        self.worker.submit(
            || device::connect().and_then(|kb| kb.repair()),
            |app, ctx, result| {
                app.connecting = false;
                match result {
                    Ok(used) => {
                        app.damaged = false;
                        app.toast(format!("Repaired: put back the keyboard's {used}"), 5);
                        app.connect_keyboard(false);
                    }
                    Err(e) => app.disconnected(ctx, &format!("Repair failed: {e}"), true),
                }
            },
        );
    }

    fn poll(&mut self, ctx: &ApplicationContext<Self>) {
        self.polls += 1;
        match self.kb.clone() {
            Some(kb) if !kb.alive() => {
                if self.music.is_some() {
                    self.stop_music();
                }
                self.disconnected(ctx, "The keyboard was unplugged.", false);
            }
            None => {
                if !self.damaged {
                    self.connect_keyboard(true);
                }
            }
            Some(kb) => {
                self.follow_keyboard(&kb);
                if self.polls % BATTERY_EVERY_POLLS == 0 && self.music.is_none() && !self.music_starting {
                    let k = kb.clone();
                    self.worker.submit(
                        move || k.read_battery(),
                        move |app, _, battery| {
                            if app.kb.as_ref().is_some_and(|cur| Arc::ptr_eq(cur, &kb)) {
                                app.show_battery(battery);
                            }
                        },
                    );
                }
            }
        }
    }

    /// Pick up changes made on the keyboard itself (Fn shortcuts), so the
    /// window and its preview show what the keyboard shows. Reads only.
    fn follow_keyboard(&mut self, kb: &Arc<Keyboard>) {
        let every = if kb.wired() { 1 } else { 2 }; // radio time is scarcer over 2.4G
        let busy = self.music.is_some()
            || self.music_starting
            || self.connecting
            || self.apply_armed
            || self.apply_queued.load(Ordering::SeqCst);
        if busy || !self.window_open || self.polls % every != 0 {
            return;
        }
        let edits = self.edits;
        let k = kb.clone();
        self.worker.submit(
            move || k.read_lighting(),
            move |app, ctx, result| {
                // Ignore it if something was changed in the window meanwhile.
                if app.edits != edits || app.apply_queued.load(Ordering::SeqCst) {
                    return;
                }
                match result {
                    Ok(state) if state != app.state => app.show_state(state),
                    Ok(_) => {}
                    Err(Error::Damaged(m)) => app.disconnected(ctx, &m, true),
                    Err(Error::Io(_)) => app.disconnected(ctx, "Lost connection to the keyboard.", false),
                    Err(_) => {} // e.g. asleep over 2.4G; the next poll tries again
                }
            },
        );
    }

    /// Battery level over 2.4G; over the cable the keyboard runs on USB power.
    fn show_battery(&mut self, battery: Option<Battery>) {
        let Some(kb) = self.kb.clone() else { return };
        self.battery_text = battery_text(battery.as_ref(), kb.wired(), kb.last_battery());
        self.battery = battery;
        let suffix = battery.map(|b| format!(" · Battery {b}")).unwrap_or_default();
        self.status = format!("{}{suffix}", kb.description());
    }

    // ------------------------------------------------------------------ power

    fn sleep_changed(&mut self) {
        let Some(kb) = self.kb.clone() else { return };
        self.edits += 1;
        let minutes = if self.keep_on { 0.0 } else { SLEEP_MINUTES[self.sleep_index] };
        self.state.sleep = minutes;
        self.pending.lock().unwrap().sleep = minutes;
        let message = if minutes == 0.0 {
            "Backlight stays on".to_string()
        } else {
            format!("Backlight turns off after {} idle", sleep_label(minutes))
        };
        self.worker.submit(
            move || kb.set_sleep(minutes),
            move |app, ctx, result| match result {
                Ok(()) => app.toast(message, 2),
                Err(e) => app.io_failed(ctx, e),
            },
        );
    }

    // ------------------------------------------------------------------ lighting

    /// The effect's row in the list; an effect this app doesn't list (set on
    /// the keyboard) is added as "<name> (set on keyboard)".
    fn effect_index(&mut self, id: u8) -> usize {
        if let Some(i) = self.effects.iter().position(|e| e.id == id) {
            return i;
        }
        let info = effect_info(id);
        self.effects.push(Effect { id, name: format!("{} (set on keyboard)", info.name), speed: true, color: true });
        self.effects.len() - 1
    }

    fn show_state(&mut self, state: Lighting) {
        self.effect_index(state.effect);
        self.keep_on = state.sleep == 0.0;
        if state.sleep != 0.0 {
            self.sleep_index = nearest_sleep_index(state.sleep);
        }
        *self.pending.lock().unwrap() = state.clone();
        self.state = state;
    }

    /// Picking a colour means "make it this colour": leave rainbow mode and,
    /// if the current effect can't take a colour, switch to Static.
    fn color_picked(&mut self, color: [u8; 3]) {
        if !effect_info(self.state.effect).color {
            let i = self.effect_index(1);
            self.state.effect = self.effects[i].id;
        }
        self.state.colorful = false;
        self.state.color = color;
        self.changed();
    }

    fn changed(&mut self) {
        self.edits += 1;
        *self.pending.lock().unwrap() = self.state.clone();
        if self.kb.is_some() {
            self.apply_armed = true;
            bridge::set_timer(TIMER_APPLY, APPLY_DELAY_MS); // restarts: quick moves send one update
        }
    }

    fn apply_lighting(&mut self) {
        let Some(kb) = self.kb.clone() else { return };
        if self.apply_queued.swap(true, Ordering::SeqCst) {
            return; // a queued job will send the newest state
        }
        let (queued, pending) = (self.apply_queued.clone(), self.pending.clone());
        self.worker.submit(
            move || {
                queued.store(false, Ordering::SeqCst);
                let state = pending.lock().unwrap().clone();
                kb.apply_lighting(&state)
            },
            |app, ctx, result| {
                if let Err(e) = result {
                    app.io_failed(ctx, e);
                }
            },
        );
    }

    fn io_failed(&mut self, ctx: &ApplicationContext<Self>, e: Error) {
        if let Error::Damaged(m) = &e {
            let m = m.clone();
            self.disconnected(ctx, &m, true);
            return;
        }
        if matches!(e, Error::Io(_)) || self.kb.as_ref().is_some_and(|kb| !kb.alive()) {
            self.disconnected(ctx, "Lost connection to the keyboard.", false);
        }
        self.toast(format!("Couldn't update the keyboard: {e}"), 5);
    }

    // ------------------------------------------------------------------ per-key

    fn fill_keys(&mut self, color: [u8; 3]) {
        for key in keys() {
            self.keys[key.led] = color;
        }
    }

    fn apply_keys(&mut self) {
        let Some(kb) = self.kb.clone() else { return };
        let keys = self.keys;
        self.worker.submit(
            move || kb.apply_keys(&keys),
            |app, ctx, result| match result {
                Ok(()) => {
                    let mut state = app.state.clone();
                    state.effect = PER_KEY;
                    app.show_state(state);
                    app.toast("Per-key colors applied", 2);
                }
                Err(e) => app.io_failed(ctx, e),
            },
        );
    }

    // ------------------------------------------------------------------ tray

    fn tooltip(&self) -> String {
        let text = match &self.kb {
            Some(k) => match self.battery.and_then(|b| b.percent) {
                Some(p) => format!("{} · {p}%", k.description()),
                None => k.description(),
            },
            None if self.damaged => "Keyboard settings damaged: open to repair".into(),
            None => "Keyboard not connected".into(),
        };
        let mut t = format!("AULA F87 Control\n{text}");
        if self.music.is_some() {
            t.push_str("\nMusic sync on");
        }
        t.chars().take(120).collect()
    }

    fn tray_menu(&self) -> Vec<MenuItem> {
        fn mark(on: bool, label: &str) -> String {
            if on { format!("✓  {label}") } else { format!("     {label}") }
        }
        fn item(enabled: bool, key: impl Into<Key>, label: impl Into<std::rc::Rc<str>>) -> MenuItem {
            if enabled { MenuItem::item(key, label) } else { MenuItem::disabled(key, label) }
        }
        let connected = self.kb.is_some();
        let music_on = self.music.is_some();
        let editable = connected && !music_on && !self.music_starting;
        let state = &self.state;
        let header = self.kb.as_ref().map(|k| k.description()).unwrap_or_else(|| "Keyboard not connected".into());
        let mut items = vec![MenuItem::disabled("keyboard", header)];
        if self.damaged {
            items.push(MenuItem::item("repair", "Repair keyboard settings"));
        }
        if let (true, Some(b)) = (connected, self.battery) {
            items.push(MenuItem::disabled("battery", format!("Battery: {}", battery_text(Some(&b), false, None))));
        }
        items.push(MenuItem::separator("sep1"));
        items.push(item(connected && !self.music_starting, "music", mark(music_on, "Music sync")));
        items.push(MenuItem::submenu(
            "styles",
            "Music style",
            MUSIC_STYLES
                .iter()
                .enumerate()
                .map(|(i, name)| MenuItem::item(format!("style:{i}"), mark(self.settings.music_style as usize == i, name))),
        ));
        let follow = &self.settings.music_follow;
        let mut follow_items = vec![MenuItem::item("follow:all", mark(follow.is_empty(), "All"))];
        for (key, label) in MUSIC_FOLLOW {
            follow_items.push(MenuItem::item(format!("follow:{key}"), mark(follow.iter().any(|f| f == key), label)));
        }
        items.push(MenuItem::submenu("follow", "Follow", follow_items));
        items.push(item(editable, "keep_on", mark(self.keep_on, "Keep backlight on")));
        items.push(MenuItem::separator("sep2"));
        if music_on || self.music_starting {
            items.push(MenuItem::disabled("paused", "Music sync is on: turn it off to change the lighting"));
        }
        let mut colors: Vec<MenuItem> =
            PRESETS.iter().enumerate().map(|(i, (name, _))| item(editable, format!("color:{i}"), *name)).collect();
        colors.push(MenuItem::separator("colors-sep"));
        colors.push(item(editable, "rainbow", mark(state.colorful || !effect_info(state.effect).color, "Rainbow")));
        items.push(MenuItem::submenu("colors", "Color", colors));
        items.push(MenuItem::submenu(
            "effects",
            "Effect",
            self.effects
                .iter()
                .enumerate()
                .map(|(i, e)| item(editable, format!("effect:{i}"), mark(e.id == state.effect, &e.name))),
        ));
        let timed = (1..=18).contains(&state.effect);
        items.push(MenuItem::submenu(
            "brightness",
            "Brightness",
            (0..5u8).map(|n| {
                let label = if n == 0 { "Off".to_string() } else { format!("{}%", n as u32 * 25) };
                item(editable && timed, format!("bright:{n}"), mark(state.brightness == n, &label))
            }),
        ));
        items.push(MenuItem::separator("sep3"));
        items.push(MenuItem::item("open", "Open AULA F87 Control"));
        items.push(MenuItem::item("about", "About"));
        items.push(MenuItem::item("autostart", mark(self.autostart, "Start at login")));
        items.push(MenuItem::item("quit", "Quit"));
        items
    }

    fn menu(&mut self, ctx: &ApplicationContext<Self>, key: &str) {
        let (name, arg) = key.split_once(':').unwrap_or((key, ""));
        let n: usize = arg.parse().unwrap_or(0);
        match name {
            "repair" => self.repair(),
            "music" => self.toggle_music(),
            "style" => {
                self.set_setting(|s| s.music_style = n.min(MUSIC_STYLES.len() - 1) as u32);
                self.music_settings_changed();
            }
            "follow" => {
                if arg == "all" {
                    self.follow("all", true);
                } else if let Some((k, _)) = MUSIC_FOLLOW.iter().find(|(k, _)| *k == arg) {
                    let on = !self.settings.music_follow.iter().any(|f| f == k);
                    self.follow(k, on);
                }
            }
            "keep_on" => {
                self.keep_on = !self.keep_on;
                self.sleep_changed();
            }
            "color" => {
                if let Some((_, c)) = PRESETS.get(n) {
                    self.color_picked(*c);
                }
            }
            "rainbow" => {
                if effect_info(self.state.effect).color {
                    self.state.colorful = !self.state.colorful;
                } else {
                    let i = self.effect_index(3); // Rainbow wave
                    self.state.effect = self.effects[i].id;
                }
                self.changed();
            }
            "effect" => {
                if let Some(e) = self.effects.get(n) {
                    self.state.effect = e.id;
                    self.changed();
                }
            }
            "bright" => {
                self.state.brightness = (n as u8).min(4);
                self.changed();
            }
            "open" => ctx.show_window(WINDOW),
            "about" => {
                self.tab = "about".into();
                ctx.show_window(WINDOW);
            }
            "autostart" => self.toggle_autostart(),
            "quit" => self.quit(ctx),
            _ => {}
        }
    }

    fn toggle_autostart(&mut self) {
        if autostart::is_enabled() {
            autostart::disable();
        } else {
            autostart::enable();
        }
        self.autostart = autostart::is_enabled();
    }
}
