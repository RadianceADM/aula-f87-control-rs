//! The main window: Lighting, Per-key colors and Music sync pages, the banner,
//! toasts, the top-bar icon and everything that talks to the keyboard.
//! A port of `aula_f87/gui.py`, widget for widget.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};

use super::tray::{Item, Tray};
use super::view::{rgb, rgba, swatch_button, KeyboardView};
use super::worker::Worker;
use super::ICON;
use crate::about;
use crate::analyzer::Follow;
use crate::autostart;
use crate::device::{self, Battery, Error, Frame, Keyboard, Lighting};
use crate::effects::{
    battery_text, effect_info, effects, nearest_sleep_index, sleep_label, Effect, MUSIC_FOLLOW, MUSIC_STYLES, PER_KEY,
    PRESETS, SIDE_MODES, SLEEP_MINUTES,
};
use crate::layout::{keys, PLANE};
use crate::music::{MusicError, MusicParams, MusicSync, Sink};
use crate::settings::Settings;

const APPLY_DELAY_MS: u64 = 350;
const POLL_SECONDS: u32 = 3;
const BATTERY_EVERY_POLLS: u64 = 20; // about once a minute

pub fn app_id() -> String {
    #[cfg(feature = "fake-keyboard")]
    if let Ok(id) = std::env::var("AULA_F87_APP_ID") {
        return id; // lets tests run next to a running copy
    }
    super::APP_ID.to_string()
}

type Connected = (Arc<Keyboard>, Lighting, Frame, Option<Battery>);

fn level_scale(on_change: impl Fn() + 'static) -> gtk::Scale {
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 4.0, 1.0);
    scale.set_digits(0);
    scale.set_round_digits(0);
    scale.set_draw_value(false);
    scale.set_size_request(240, -1);
    scale.set_valign(gtk::Align::Center);
    for i in 0..5 {
        scale.add_mark(i as f64, gtk::PositionType::Bottom, Some(&i.to_string()));
    }
    scale.connect_value_changed(move |_| on_change());
    scale
}

fn string_list(items: &[&str]) -> gtk::StringList {
    gtk::StringList::new(items)
}

pub struct Win {
    pub window: adw::ApplicationWindow,
    pub stack: adw::ViewStack,
    app: adw::Application,
    toasts: adw::ToastOverlay,
    status_label: gtk::Label,
    refresh_button: gtk::Button,
    banner: adw::Banner,
    // lighting page
    preview: KeyboardView,
    effect_row: adw::ComboRow,
    rainbow_row: adw::SwitchRow,
    color_button: gtk::ColorDialogButton,
    color_row: adw::ActionRow,
    preset_row: adw::ActionRow,
    brightness: gtk::Scale,
    brightness_row: adw::ActionRow,
    speed: gtk::Scale,
    speed_row: adw::ActionRow,
    side_row: adw::ComboRow,
    battery_row: adw::ActionRow,
    keep_on_row: adw::SwitchRow,
    sleep_row: adw::ComboRow,
    // per-key page
    paint_view: KeyboardView,
    brush_button: gtk::ColorDialogButton,
    apply_keys_button: gtk::Button,
    // music page
    music_view: KeyboardView,
    music_button: gtk::Button,
    music_row: adw::ActionRow,
    music_style_row: adw::ComboRow,
    follow_checks: Vec<(&'static str, gtk::CheckButton)>,
    sensitivity: gtk::Scale,
    brightness_music: gtk::Scale,
    // state
    worker: Worker,
    settings: RefCell<Settings>,
    kb: RefCell<Option<Arc<Keyboard>>>,
    state: RefCell<Lighting>,
    pending: Arc<Mutex<Lighting>>, // newest state, read by a queued apply job
    keys: RefCell<Frame>,
    brush: Cell<[u8; 3]>,
    effects: RefCell<Vec<Effect>>,
    loading: Cell<bool>,
    connecting: Cell<bool>,
    apply_source: RefCell<Option<glib::SourceId>>,
    apply_queued: Arc<AtomicBool>,
    damaged: Cell<bool>, // the keyboard's stored settings need Repair
    music: RefCell<Option<MusicSync>>,
    music_starting: Cell<bool>,
    music_cancel: Cell<bool>, // window closed while music sync was starting
    music_colors: RefCell<Frame>,
    music_seen: Cell<u64>,
    follow_updating: Cell<bool>,
    polls: Cell<u64>,
    edits: Cell<u64>, // counts changes made in the window, see follow_keyboard
    battery: Cell<Option<Battery>>,
    tray: RefCell<Option<Tray>>,
}

impl Win {
    pub fn new(app: &adw::Application, icon_dir: &Path) -> Rc<Win> {
        let settings = Settings::load();
        let window = adw::ApplicationWindow::builder().application(app).title("AULA F87 Control").build();
        window.set_default_size(settings.window[0], settings.window[1]);
        let brush = settings.brush;

        // Widgets that callbacks refer to are created first, wired up below.
        let win = Rc::new_cyclic(|weak: &std::rc::Weak<Win>| {
            let w = weak.clone();
            let changed = move || {
                if let Some(win) = w.upgrade() {
                    win.changed();
                }
            };
            let w = weak.clone();
            let preview = KeyboardView::new(
                Rc::new(move |led, t| w.upgrade().map(|win| win.preview_color(led, t)).unwrap_or([0; 3])),
                None,
                230,
            );
            let w = weak.clone();
            let w2 = weak.clone();
            let paint_view = KeyboardView::new(
                Rc::new(move |led, _| w.upgrade().map(|win| win.keys.borrow()[led]).unwrap_or([0; 3])),
                Some(Rc::new(move |led, erase| {
                    if let Some(win) = w2.upgrade() {
                        win.paint_key(led, erase);
                    }
                })),
                330,
            );
            let w = weak.clone();
            let music_view = KeyboardView::new(
                Rc::new(move |led, _| w.upgrade().map(|win| win.music_colors.borrow()[led]).unwrap_or([0; 3])),
                None,
                230,
            );
            let c1 = changed.clone();
            let c2 = changed.clone();
            let effect_names: Vec<String> = effects().iter().map(|e| e.name.clone()).collect();
            let effect_refs: Vec<&str> = effect_names.iter().map(|s| s.as_str()).collect();
            let sleep_labels: Vec<String> = SLEEP_MINUTES.iter().map(|m| sleep_label(*m)).collect();
            let sleep_refs: Vec<&str> = sleep_labels.iter().map(|s| s.as_str()).collect();
            Win {
                window: window.clone(),
                stack: adw::ViewStack::new(),
                app: app.clone(),
                toasts: adw::ToastOverlay::new(),
                status_label: gtk::Label::new(Some("Looking for keyboard…")),
                refresh_button: gtk::Button::builder()
                    .icon_name("view-refresh-symbolic")
                    .tooltip_text("Reconnect and reload from keyboard")
                    .build(),
                banner: adw::Banner::builder().button_label("Retry").build(),
                preview,
                effect_row: adw::ComboRow::builder().title("Lighting effect").model(&string_list(&effect_refs)).build(),
                rainbow_row: adw::SwitchRow::builder()
                    .title("Rainbow")
                    .subtitle("Use all colors instead of a single one")
                    .build(),
                color_button: gtk::ColorDialogButton::builder()
                    .dialog(&gtk::ColorDialog::builder().with_alpha(false).build())
                    .valign(gtk::Align::Center)
                    .build(),
                color_row: adw::ActionRow::builder().title("Color").subtitle("Pick any color").build(),
                preset_row: adw::ActionRow::builder().title("Quick colors").build(),
                brightness: level_scale(c1),
                brightness_row: adw::ActionRow::builder().title("Brightness").build(),
                speed: level_scale(c2),
                speed_row: adw::ActionRow::builder().title("Speed").build(),
                side_row: adw::ComboRow::builder().title("Side lights effect").model(&string_list(&SIDE_MODES)).build(),
                battery_row: adw::ActionRow::builder().title("Battery").subtitle("—").build(),
                keep_on_row: adw::SwitchRow::builder()
                    .title("Keep backlight on")
                    .subtitle("Never turn the backlight off when idle. Uses more battery on 2.4G.")
                    .build(),
                sleep_row: adw::ComboRow::builder()
                    .title("Turn backlight off after")
                    .subtitle("Idle time without a key press")
                    .model(&string_list(&sleep_refs))
                    .build(),
                paint_view,
                brush_button: gtk::ColorDialogButton::builder()
                    .dialog(&gtk::ColorDialog::builder().with_alpha(false).build())
                    .rgba(&rgba(brush))
                    .valign(gtk::Align::Center)
                    .build(),
                apply_keys_button: gtk::Button::with_label("Apply to keyboard"),
                music_view,
                music_button: gtk::Button::builder().label("Start").valign(gtk::Align::Center).build(),
                music_row: adw::ActionRow::builder().title("Follow the music").subtitle("Stopped").build(),
                music_style_row: adw::ComboRow::builder().title("Style").model(&string_list(&MUSIC_STYLES)).build(),
                follow_checks: [("all", "All")]
                    .into_iter()
                    .chain(MUSIC_FOLLOW)
                    .map(|(k, label)| (k, gtk::CheckButton::with_label(label)))
                    .collect(),
                sensitivity: gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.5, 2.0, 0.1),
                brightness_music: gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.1, 1.0, 0.05),
                worker: Worker::new(),
                settings: RefCell::new(settings.clone()),
                kb: RefCell::new(None),
                state: RefCell::new(Lighting::default()),
                pending: Arc::new(Mutex::new(Lighting::default())),
                keys: RefCell::new([[0; 3]; PLANE]),
                brush: Cell::new(brush),
                effects: RefCell::new(effects()),
                loading: Cell::new(false),
                connecting: Cell::new(false),
                apply_source: RefCell::new(None),
                apply_queued: Arc::new(AtomicBool::new(false)),
                damaged: Cell::new(false),
                music: RefCell::new(None),
                music_starting: Cell::new(false),
                music_cancel: Cell::new(false),
                music_colors: RefCell::new([[0; 3]; PLANE]),
                music_seen: Cell::new(0),
                follow_updating: Cell::new(false),
                polls: Cell::new(0),
                edits: Cell::new(0),
                battery: Cell::new(None),
                tray: RefCell::new(None),
            }
        });
        win.build(&settings, icon_dir);
        win
    }

    fn build(self: &Rc<Self>, settings: &Settings, icon_dir: &Path) {
        let (w1, w2) = (Rc::downgrade(self), Rc::downgrade(self));
        self.preview.animate(
            Rc::new(move |t| w1.upgrade().map(|w| super::preview::strips(&w.state.borrow(), t)).unwrap_or_default()),
            Rc::new(move || w2.upgrade().is_some_and(|w| super::preview::animated(&w.state.borrow()))),
        );
        let stack = self.stack.clone();
        stack.add_titled_with_icon(&self.lighting_page(), Some("lighting"), "Lighting", "weather-clear-symbolic");
        stack.add_titled_with_icon(&self.per_key_page(), Some("per-key"), "Per-key colors", "applications-graphics-symbolic");
        stack.add_titled_with_icon(&self.music_page(settings), Some("music"), "Music sync", "audio-x-generic-symbolic");
        self.toasts.set_child(Some(&stack));
        stack.set_visible_child_name(&settings.tab);
        let w = Rc::downgrade(self);
        stack.connect_visible_child_name_notify(move |st| {
            if let (Some(win), Some(name)) = (w.upgrade(), st.visible_child_name()) {
                win.set_setting(|s| s.tab = name.to_string());
            }
        });

        let header = adw::HeaderBar::builder()
            .title_widget(&adw::ViewSwitcher::builder().stack(&stack).policy(adw::ViewSwitcherPolicy::Wide).build())
            .build();
        let status = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        status.append(&gtk::Image::from_icon_name("input-keyboard-symbolic"));
        self.status_label.add_css_class("dim-label");
        status.append(&self.status_label);
        header.pack_start(&status);
        let menu = gio::Menu::new();
        menu.append(Some(&format!("About {}", about::NAME)), Some("app.about"));
        header.pack_end(
            &gtk::MenuButton::builder()
                .icon_name("open-menu-symbolic")
                .menu_model(&menu)
                .tooltip_text("Main menu")
                .primary(true)
                .build(),
        );
        let w = Rc::downgrade(self);
        self.refresh_button.connect_clicked(move |_| {
            if let Some(win) = w.upgrade() {
                win.connect_keyboard(false);
            }
        });
        header.pack_end(&self.refresh_button);

        let w = Rc::downgrade(self);
        self.banner.connect_button_clicked(move |_| {
            if let Some(win) = w.upgrade() {
                if win.damaged.get() {
                    win.repair();
                } else {
                    win.connect_keyboard(false);
                }
            }
        });

        let view = adw::ToolbarView::new();
        view.set_content(Some(&self.toasts));
        view.add_top_bar(&header);
        view.add_top_bar(&self.banner);
        self.window.set_content(Some(&view));

        if let Some(conn) = self.app.dbus_connection() {
            let (w1, w2) = (Rc::downgrade(self), Rc::downgrade(self));
            let tray = Tray::new(
                &conn,
                "AULA F87 Control",
                &icon_dir.display().to_string(),
                move || w1.upgrade().map(|w| w.tray_menu()).unwrap_or_default(),
                move || {
                    if let Some(w) = w2.upgrade() {
                        w.show_window();
                    }
                },
            );
            *self.tray.borrow_mut() = Some(tray);
        }

        self.sync_widgets();
        self.connect_keyboard(false);
        let w = Rc::downgrade(self);
        glib::timeout_add_seconds_local(POLL_SECONDS, move || match w.upgrade() {
            Some(win) => {
                win.poll();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
        let w = Rc::downgrade(self);
        self.window.connect_close_request(move |_| match w.upgrade() {
            Some(win) => win.on_close(),
            None => glib::Propagation::Proceed,
        });
    }

    // ------------------------------------------------------------------ pages

    fn lighting_page(self: &Rc<Self>) -> adw::PreferencesPage {
        let page = adw::PreferencesPage::new();

        let preview = adw::PreferencesGroup::new();
        preview.add(&self.preview);
        page.add(&preview);

        let group = adw::PreferencesGroup::builder().title("Effect").build();
        let w = Rc::downgrade(self);
        self.effect_row.connect_selected_notify(move |_| {
            if let Some(win) = w.upgrade() {
                win.changed();
            }
        });
        group.add(&self.effect_row);
        page.add(&group);

        let group = adw::PreferencesGroup::builder().title("Color").build();
        let w = Rc::downgrade(self);
        self.rainbow_row.connect_active_notify(move |_| {
            if let Some(win) = w.upgrade() {
                win.changed();
            }
        });
        group.add(&self.rainbow_row);
        let w = Rc::downgrade(self);
        self.color_button.connect_rgba_notify(move |_| {
            if let Some(win) = w.upgrade() {
                win.color_picked();
            }
        });
        self.color_row.add_suffix(&self.color_button);
        self.color_row.set_activatable_widget(Some(&self.color_button));
        group.add(&self.color_row);

        let presets = gtk::Box::builder().spacing(2).valign(gtk::Align::Center).build();
        for (name, color) in PRESETS {
            let w = Rc::downgrade(self);
            presets.append(&swatch_button(name, color, move |c| {
                if let Some(win) = w.upgrade() {
                    win.pick_preset(c);
                }
            }));
        }
        self.preset_row.add_suffix(&presets);
        group.add(&self.preset_row);
        page.add(&group);

        let group = adw::PreferencesGroup::builder().title("Brightness and speed").build();
        self.brightness_row.add_suffix(&self.brightness);
        group.add(&self.brightness_row);
        self.speed_row.add_suffix(&self.speed);
        group.add(&self.speed_row);
        page.add(&group);

        let group = adw::PreferencesGroup::builder()
            .title("Side lights")
            .description(
                "The light bar above the arrow keys is set on the keyboard: press Fn + Left Shift to cycle its \
                 effects.",
            )
            .build();
        let w = Rc::downgrade(self);
        self.side_row.connect_selected_notify(move |_| {
            if let Some(win) = w.upgrade() {
                win.changed();
            }
        });
        group.add(&self.side_row);
        page.add(&group);

        let group = adw::PreferencesGroup::builder().title("Power").build();
        group.add(&self.battery_row);
        let w = Rc::downgrade(self);
        self.keep_on_row.connect_active_notify(move |_| {
            if let Some(win) = w.upgrade() {
                win.sleep_changed();
            }
        });
        group.add(&self.keep_on_row);
        self.sleep_row.set_selected(nearest_sleep_index(5.0) as u32);
        let w = Rc::downgrade(self);
        self.sleep_row.connect_selected_notify(move |_| {
            if let Some(win) = w.upgrade() {
                win.sleep_changed();
            }
        });
        group.add(&self.sleep_row);
        page.add(&group);
        page
    }

    fn lighting_widgets(&self) -> Vec<gtk::Widget> {
        vec![
            self.effect_row.clone().upcast(),
            self.rainbow_row.clone().upcast(),
            self.color_row.clone().upcast(),
            self.preset_row.clone().upcast(),
            self.brightness_row.clone().upcast(),
            self.speed_row.clone().upcast(),
            self.side_row.clone().upcast(),
            self.keep_on_row.clone().upcast(),
            self.sleep_row.clone().upcast(),
        ]
    }

    fn per_key_page(self: &Rc<Self>) -> gtk::Box {
        let page = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(14)
            .margin_top(18)
            .margin_bottom(18)
            .margin_start(18)
            .margin_end(18)
            .build();
        let hint = gtk::Label::builder()
            .label(
                "Click or drag over keys to paint them. Right-click clears a key. Press Apply to send the colors to \
                 the keyboard.",
            )
            .xalign(0.0)
            .wrap(true)
            .build();
        hint.add_css_class("dim-label");
        page.append(&hint);
        self.paint_view.set_vexpand(true);
        page.append(&self.paint_view);

        let tools = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        tools.append(&gtk::Label::new(Some("Brush")));
        let w = Rc::downgrade(self);
        self.brush_button.connect_rgba_notify(move |b| {
            if let Some(win) = w.upgrade() {
                win.set_brush(rgb(&b.rgba()));
            }
        });
        tools.append(&self.brush_button);
        for (name, color) in PRESETS {
            let b = self.brush_button.clone();
            tools.append(&swatch_button(name, color, move |c| b.set_rgba(&rgba(c))));
        }
        tools.append(&gtk::Separator::new(gtk::Orientation::Vertical));

        let fill = gtk::Button::with_label("Fill all");
        let w = Rc::downgrade(self);
        fill.connect_clicked(move |_| {
            if let Some(win) = w.upgrade() {
                win.fill_keys(win.brush.get());
            }
        });
        tools.append(&fill);
        let clear = gtk::Button::with_label("Clear all");
        let w = Rc::downgrade(self);
        clear.connect_clicked(move |_| {
            if let Some(win) = w.upgrade() {
                win.fill_keys([0, 0, 0]);
            }
        });
        tools.append(&clear);

        tools.append(&gtk::Box::builder().hexpand(true).build());
        self.apply_keys_button.add_css_class("suggested-action");
        self.apply_keys_button.add_css_class("pill");
        let w = Rc::downgrade(self);
        self.apply_keys_button.connect_clicked(move |_| {
            if let Some(win) = w.upgrade() {
                win.apply_keys();
            }
        });
        tools.append(&self.apply_keys_button);
        page.append(&tools);
        page
    }

    fn music_page(self: &Rc<Self>, settings: &Settings) -> adw::PreferencesPage {
        let page = adw::PreferencesPage::new();
        let preview = adw::PreferencesGroup::new();
        preview.add(&self.music_view);
        page.add(&preview);

        let group = adw::PreferencesGroup::builder()
            .title("Music sync")
            .description(
                "The keys move with whatever this computer is playing, from any app. Your saved lighting comes back \
                 when you stop.",
            )
            .build();
        self.music_button.add_css_class("pill");
        self.music_button.add_css_class("suggested-action");
        let w = Rc::downgrade(self);
        self.music_button.connect_clicked(move |_| {
            if let Some(win) = w.upgrade() {
                win.toggle_music();
            }
        });
        self.music_row.add_suffix(&self.music_button);
        group.add(&self.music_row);

        self.music_style_row.set_selected(settings.music_style);
        let w = Rc::downgrade(self);
        self.music_style_row.connect_selected_notify(move |_| {
            if let Some(win) = w.upgrade() {
                win.music_settings();
            }
        });
        group.add(&self.music_style_row);

        let follow_box = gtk::Box::builder().spacing(14).valign(gtk::Align::Center).build();
        for (key, check) in &self.follow_checks {
            let w = Rc::downgrade(self);
            let key = *key;
            check.connect_toggled(move |_| {
                if let Some(win) = w.upgrade() {
                    win.follow_toggled(key);
                }
            });
            follow_box.append(check);
        }
        let picked: Vec<&str> = settings.music_follow.iter().map(|s| s.as_str()).collect();
        self.show_follow(&picked);
        let follow_row = adw::ActionRow::builder().title("Follow").subtitle("What the lights react to").build();
        follow_row.add_suffix(&follow_box);
        group.add(&follow_row);

        for (scale, value, marks) in [
            (&self.sensitivity, settings.music_sensitivity, [0.5, 1.0, 1.5, 2.0]),
            (&self.brightness_music, settings.music_brightness, [0.25, 0.5, 0.75, 1.0]),
        ] {
            scale.set_value(value);
            scale.set_draw_value(false);
            scale.set_size_request(240, -1);
            scale.set_valign(gtk::Align::Center);
            for mark in marks {
                scale.add_mark(mark, gtk::PositionType::Bottom, None);
            }
            let w = Rc::downgrade(self);
            scale.connect_value_changed(move |_| {
                if let Some(win) = w.upgrade() {
                    win.music_settings();
                }
            });
        }
        let row = adw::ActionRow::builder().title("Sensitivity").subtitle("Higher = taller bars").build();
        row.add_suffix(&self.sensitivity);
        group.add(&row);
        let row = adw::ActionRow::builder().title("Brightness").build();
        row.add_suffix(&self.brightness_music);
        group.add(&row);
        page.add(&group);
        page
    }

    fn set_setting(&self, f: impl FnOnce(&mut Settings)) {
        let mut s = self.settings.borrow_mut();
        let before = s.clone();
        f(&mut s);
        if *s != before {
            s.save();
        }
    }

    fn toast(&self, title: &str, seconds: u32) {
        self.toasts.add_toast(adw::Toast::builder().title(title).timeout(seconds).build());
    }

    fn kb(&self) -> Option<Arc<Keyboard>> {
        self.kb.borrow().clone()
    }

    // ------------------------------------------------------------------ music

    fn set_brush(&self, color: [u8; 3]) {
        self.brush.set(color);
        self.set_setting(|s| s.brush = color);
    }

    /// Start/stop chosen by the user; remembered so it resumes next time.
    fn toggle_music(self: &Rc<Self>) {
        if self.music.borrow().is_some() {
            self.set_setting(|s| s.music_on = false);
            self.stop_music(None);
        } else {
            self.set_setting(|s| s.music_on = true);
            self.start_music();
        }
    }

    fn show_follow(&self, selected: &[&str]) {
        self.follow_updating.set(true);
        for (key, check) in &self.follow_checks {
            check.set_active(if *key == "all" { selected.is_empty() } else { selected.contains(key) });
        }
        self.follow_updating.set(false);
    }

    fn follow(&self) -> Vec<&'static str> {
        self.follow_checks.iter().filter(|(k, c)| *k != "all" && c.is_active()).map(|(k, _)| *k).collect()
    }

    fn check(&self, key: &str) -> &gtk::CheckButton {
        &self.follow_checks.iter().find(|(k, _)| *k == key).unwrap().1
    }

    /// "All" and the specific choices exclude each other; with nothing picked,
    /// "All" ticks itself again.
    fn follow_toggled(&self, key: &str) {
        if self.follow_updating.get() {
            return;
        }
        if key == "all" && self.check("all").is_active() {
            self.show_follow(&[]);
        } else if key != "all" && self.check(key).is_active() {
            self.show_follow(&self.follow());
        } else if self.follow().is_empty() {
            self.show_follow(&[]);
        }
        self.music_settings();
    }

    fn tray_follow(&self, key: &str) {
        if key == "all" {
            self.show_follow(&[]);
            self.music_settings();
        } else {
            let c = self.check(key);
            c.set_active(!c.is_active());
        }
    }

    fn music_params(&self) -> MusicParams {
        let f = self.follow();
        MusicParams {
            style: self.music_style_row.selected() as usize,
            sensitivity: self.sensitivity.value(),
            brightness: self.brightness_music.value(),
            follow: Follow { bass: f.contains(&"bass"), vocals: f.contains(&"vocals"), beat: f.contains(&"beat") },
        }
    }

    fn music_settings(&self) {
        let follow: Vec<String> = self.follow().iter().map(|s| s.to_string()).collect();
        let style = self.music_style_row.selected();
        let (sens, bright) = (self.sensitivity.value(), self.brightness_music.value());
        self.set_setting(|s| {
            s.music_follow = follow;
            s.music_style = style;
            s.music_sensitivity = (sens * 100.0).round() / 100.0;
            s.music_brightness = (bright * 100.0).round() / 100.0;
        });
        self.tray_refresh();
        if let Some(sync) = self.music.borrow().as_ref() {
            *sync.params.lock().unwrap() = self.music_params();
        }
    }

    fn start_music(self: &Rc<Self>) {
        let Some(kb) = self.kb() else { return };
        if self.music_starting.get() {
            return;
        }
        self.music_starting.set(true);
        self.music_cancel.set(false);
        self.sync_widgets();
        let params = self.music_params();
        let w = Rc::downgrade(self);
        // Queued on the worker so it starts after any pending keyboard writes.
        self.worker.submit(
            move || MusicSync::start(Sink::Keyboard(kb), params, None),
            move |result| {
                let Some(win) = w.upgrade() else { return };
                win.music_starting.set(false);
                match result {
                    Ok(mut sync) => {
                        if sync.running() && !win.music_cancel.get() {
                            *win.music.borrow_mut() = Some(sync);
                            win.music_seen.set(0);
                            win.music_settings();
                            win.watch_music();
                        } else {
                            let err = sync.error();
                            win.worker.run(move || sync.stop());
                            if let Some(e) = err {
                                win.music_failed(e);
                            }
                        }
                    }
                    Err(e) => {
                        win.set_setting(|s| s.music_on = false); // don't retry a start that can't work
                        win.toast(&format!("Couldn't start music sync: {e}"), 6);
                    }
                }
                win.sync_widgets();
            },
        );
    }

    /// While music sync runs: draw its frames and notice when it stops by itself.
    fn watch_music(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(33), move || {
            let Some(win) = w.upgrade() else { return glib::ControlFlow::Break };
            let (frames, latest, error) = match win.music.borrow().as_ref() {
                Some(sync) => (sync.frames.load(Ordering::SeqCst), *sync.latest.lock().unwrap(), sync.error()),
                None => return glib::ControlFlow::Break,
            };
            if let Some(e) = error {
                win.music_failed(e);
                return glib::ControlFlow::Break;
            }
            if frames != win.music_seen.get() {
                win.music_seen.set(frames);
                *win.music_colors.borrow_mut() = latest;
                win.music_view.queue_draw();
            }
            glib::ControlFlow::Continue
        });
    }

    fn stop_music(self: &Rc<Self>, then: Option<Box<dyn FnOnce()>>) {
        let sync = self.music.borrow_mut().take();
        self.sync_widgets();
        if let Some(mut sync) = sync {
            let w = Rc::downgrade(self);
            self.worker.submit(
                move || sync.stop(),
                move |_| {
                    if let Some(win) = w.upgrade() {
                        *win.music_colors.borrow_mut() = [[0; 3]; PLANE];
                        win.music_view.queue_draw();
                    }
                    if let Some(then) = then {
                        then();
                    }
                },
            );
        }
    }

    fn music_failed(self: &Rc<Self>, e: MusicError) {
        self.stop_music(None);
        if e.device || self.kb().is_some_and(|kb| !kb.alive()) {
            self.disconnected("Lost connection to the keyboard.", false);
        }
        self.toast(&format!("Music sync stopped: {e}"), 6);
    }

    fn remember_size(&self) {
        if self.window.is_visible() && !self.window.is_maximized() && self.window.width() > 0 {
            let size = [self.window.width(), self.window.height()];
            self.set_setting(|s| s.window = size);
        }
    }

    fn on_close(self: &Rc<Self>) -> glib::Propagation {
        // Closing the window stops music sync, so the keyboard goes back to its
        // own lighting within a second. With the top-bar icon showing, the app
        // stays there (music sync can be started from its menu); "Quit" ends it.
        self.remember_size();
        if self.tray.borrow().as_ref().is_some_and(|t| t.available()) {
            if self.music.borrow().is_some() {
                self.stop_music(None); // resumes when the window is opened again
            }
            self.music_cancel.set(self.music_starting.get());
            self.window.set_visible(false);
            return glib::Propagation::Stop;
        }
        self.shutdown();
        glib::Propagation::Proceed
    }

    fn shutdown(&self) {
        self.remember_size();
        if let Some(mut sync) = self.music.borrow_mut().take() {
            sync.stop();
        }
        if let Some(tray) = self.tray.borrow_mut().take() {
            tray.close();
        }
    }

    pub fn show_window(self: &Rc<Self>) {
        self.window.set_visible(true);
        self.window.present();
        self.resume_music();
    }

    /// Music sync was on when the window was last open (or before the keyboard
    /// dropped out): start it again.
    fn resume_music(self: &Rc<Self>) {
        if self.settings.borrow().music_on
            && self.kb().is_some()
            && self.window.is_visible()
            && self.music.borrow().is_none()
            && !self.music_starting.get()
        {
            self.start_music();
        }
    }

    pub fn quit_app(self: &Rc<Self>) {
        self.shutdown();
        self.app.quit();
    }

    /// Version, links, and the changelog under "What's New".
    pub fn show_about(self: &Rc<Self>) {
        self.show_window();
        let dialog = adw::AboutDialog::builder()
            .application_name(about::NAME)
            .application_icon(ICON)
            .version(about::VERSION)
            .developer_name(about::DEVELOPER)
            .website(about::WEBSITE)
            .issue_url(about::ISSUES)
            .license_type(gtk::License::Gpl30)
            .copyright(about::COPYRIGHT)
            .comments(about::SUMMARY)
            .debug_info(self.debug_info())
            .release_notes(about::release_notes_markup())
            .release_notes_version(about::VERSION)
            .build();
        dialog.add_link("Full changelog", about::CHANGELOG_URL);
        dialog.present(Some(&self.window));
    }

    fn debug_info(&self) -> String {
        let keyboard = self.kb().map(|k| k.description()).unwrap_or_else(|| "not connected".into());
        let mut uts: libc::utsname = unsafe { std::mem::zeroed() };
        unsafe { libc::uname(&mut uts) };
        let field = |f: &[libc::c_char]| unsafe { std::ffi::CStr::from_ptr(f.as_ptr()) }.to_string_lossy().into_owned();
        format!(
            "{} {} (Rust)\nKeyboard: {keyboard}\nGTK {}.{}.{}, libadwaita {}.{}.{}\nSystem: {} {} {}",
            about::NAME,
            about::VERSION,
            gtk::major_version(),
            gtk::minor_version(),
            gtk::micro_version(),
            adw::major_version(),
            adw::minor_version(),
            adw::micro_version(),
            field(&uts.sysname),
            field(&uts.release),
            field(&uts.machine),
        )
    }

    // ------------------------------------------------------------------ top-bar icon

    fn tray_menu(self: &Rc<Self>) -> Vec<Item> {
        let kb = self.kb();
        let connected = kb.is_some();
        let music_on = self.music.borrow().is_some();
        let editable = connected && !music_on && !self.music_starting.get();
        let state = self.state.borrow().clone();
        let header = kb.as_ref().map(|k| k.description()).unwrap_or_else(|| "Keyboard not connected".into());
        let mut items = vec![Item::label(header).key("keyboard")];
        if self.damaged.get() {
            let w = Rc::downgrade(self);
            items.push(Item::new("Repair keyboard settings", move || {
                if let Some(win) = w.upgrade() {
                    win.repair();
                }
            }));
        }
        if let (true, Some(b)) = (connected, self.battery.get()) {
            items.push(Item::label(format!("Battery: {}", battery_text(Some(&b), false, None))).key("battery"));
        }
        let w = Rc::downgrade(self);
        let win = move || w.upgrade();
        let wf = win.clone();
        items.push(Item::separator());
        items.push(
            Item::new("Music sync", {
                let win = wf.clone();
                move || {
                    if let Some(w) = win() {
                        w.toggle_music();
                    }
                }
            })
            .toggle("checkmark", music_on)
            .enabled(connected && !self.music_starting.get()),
        );
        let selected_style = self.music_style_row.selected();
        items.push(Item::submenu(
            "Music style",
            MUSIC_STYLES
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    let win = wf.clone();
                    Item::new(*name, move || {
                        if let Some(w) = win() {
                            w.music_style_row.set_selected(i as u32);
                        }
                    })
                    .toggle("radio", selected_style == i as u32)
                })
                .collect(),
        ));
        let follow = self.follow();
        let mut follow_items = vec![{
            let win = wf.clone();
            Item::new("All", move || {
                if let Some(w) = win() {
                    w.tray_follow("all");
                }
            })
            .toggle("checkmark", follow.is_empty())
        }];
        for (key, label) in MUSIC_FOLLOW {
            let win = wf.clone();
            follow_items.push(
                Item::new(label, move || {
                    if let Some(w) = win() {
                        w.tray_follow(key);
                    }
                })
                .toggle("checkmark", follow.contains(&key)),
            );
        }
        items.push(Item::submenu("Follow", follow_items));
        let keep_on = self.keep_on_row.is_active();
        items.push(
            Item::new("Keep backlight on", {
                let win = wf.clone();
                move || {
                    if let Some(w) = win() {
                        w.keep_on_row.set_active(!w.keep_on_row.is_active());
                    }
                }
            })
            .toggle("checkmark", keep_on)
            .enabled(editable),
        );
        items.push(Item::separator());
        let mut colors: Vec<Item> = PRESETS
            .iter()
            .map(|(name, c)| {
                let win = wf.clone();
                let c = *c;
                Item::new(*name, move || {
                    if let Some(w) = win() {
                        w.pick_preset(c);
                    }
                })
            })
            .collect();
        colors.push(Item::separator());
        colors.push(
            Item::new("Rainbow", {
                let win = wf.clone();
                move || {
                    if let Some(w) = win() {
                        w.tray_rainbow();
                    }
                }
            })
            .toggle("checkmark", state.colorful || !effect_info(state.effect).color),
        );
        items.push(Item::submenu("Color", colors).enabled(editable));
        let effect_items = self
            .effects
            .borrow()
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let win = wf.clone();
                Item::new(e.name.clone(), move || {
                    if let Some(w) = win() {
                        w.effect_row.set_selected(i as u32);
                    }
                })
                .toggle("radio", e.id == state.effect)
            })
            .collect();
        items.push(Item::submenu("Effect", effect_items).enabled(editable));
        let levels = (0..5u8)
            .map(|n| {
                let win = wf.clone();
                Item::new(if n == 0 { "Off".to_string() } else { format!("{}%", n as u32 * 25) }, move || {
                    if let Some(w) = win() {
                        w.brightness.set_value(n as f64);
                    }
                })
                .toggle("radio", state.brightness == n)
            })
            .collect();
        items.push(Item::submenu("Brightness", levels).enabled(editable && (1..=18).contains(&state.effect)));
        items.push(Item::separator());
        items.push(Item::new("Open AULA F87 Control", {
            let win = wf.clone();
            move || {
                if let Some(w) = win() {
                    w.show_window();
                }
            }
        }));
        items.push(Item::new("About", {
            let win = wf.clone();
            move || {
                if let Some(w) = win() {
                    w.show_about();
                }
            }
        }));
        items.push(
            Item::new("Start at login", {
                let win = wf.clone();
                move || {
                    if let Some(w) = win() {
                        w.toggle_autostart();
                    }
                }
            })
            .toggle("checkmark", autostart::is_enabled()),
        );
        items.push(Item::new("Quit", move || {
            if let Some(w) = wf() {
                w.quit_app();
            }
        }));
        items
    }

    fn tray_rainbow(self: &Rc<Self>) {
        if effect_info(self.state.borrow().effect).color {
            self.rainbow_row.set_active(!self.rainbow_row.is_active());
        } else {
            let i = self.effect_index(3); // Rainbow wave
            self.effect_row.set_selected(i);
        }
    }

    fn toggle_autostart(&self) {
        let _ = if autostart::is_enabled() { autostart::disable() } else { autostart::enable() };
        self.tray_refresh();
    }

    fn tray_refresh(&self) {
        let tray = self.tray.borrow().clone();
        let Some(tray) = tray else { return };
        let kb = self.kb();
        let label = match (self.battery.get().and_then(|b| b.percent), &kb) {
            (Some(p), Some(_)) => format!("{p}%"),
            _ => String::new(),
        };
        let tooltip = match &kb {
            Some(k) => k.description(),
            None if self.damaged.get() => "Keyboard settings damaged: open to repair".into(),
            None => "Keyboard not connected".into(),
        };
        tray.refresh(Some(&label), Some(&tooltip));
    }

    // ------------------------------------------------------------------ connection

    fn connect_keyboard(self: &Rc<Self>, quiet: bool) {
        if self.connecting.get() || self.music.borrow().is_some() || self.music_starting.get() {
            return;
        }
        self.connecting.set(true);
        let old = self.kb.borrow_mut().take();
        if !quiet {
            self.status_label.set_label("Connecting…");
        }
        let w = Rc::downgrade(self);
        self.worker.submit(
            move || -> device::Result<Connected> {
                drop(old);
                let kb = device::connect()?;
                let state = kb.read_lighting()?;
                let keys = kb.read_keys()?;
                let battery = kb.read_battery();
                Ok((Arc::new(kb), state, keys, battery))
            },
            move |result| {
                let Some(win) = w.upgrade() else { return };
                win.connecting.set(false);
                match result {
                    Ok((kb, state, keys, battery)) => {
                        *win.kb.borrow_mut() = Some(kb);
                        *win.keys.borrow_mut() = keys;
                        win.show_state(state);
                        win.banner.set_revealed(false);
                        win.show_battery(battery);
                        win.sync_widgets();
                        win.paint_view.queue_draw();
                        win.resume_music();
                    }
                    Err(e) => {
                        let damaged = matches!(e, Error::Damaged(_));
                        win.disconnected(&e.to_string(), damaged);
                    }
                }
            },
        );
    }

    fn disconnected(self: &Rc<Self>, message: &str, damaged: bool) {
        if let Some(kb) = self.kb.borrow_mut().take() {
            // Close on the worker so jobs already queued for this handle finish first.
            self.worker.run(move || drop(kb));
        }
        if damaged && !self.damaged.get() {
            self.show_window(); // the keyboard may not type until this is fixed
        }
        self.damaged.set(damaged);
        self.status_label.set_label(if damaged { "Settings damaged" } else { "Not connected" });
        self.battery_row.set_subtitle("—");
        self.banner.set_title(message);
        self.banner.set_button_label(Some(if damaged { "Repair" } else { "Retry" }));
        self.banner.set_revealed(true);
        self.sync_widgets();
    }

    /// Write the keyboard's last good settings back (see Keyboard::repair).
    fn repair(self: &Rc<Self>) {
        if self.connecting.get() {
            return;
        }
        self.connecting.set(true); // no reconnect attempts meanwhile
        self.banner.set_revealed(false);
        self.status_label.set_label("Repairing…");
        let w = Rc::downgrade(self);
        self.worker.submit(
            || device::connect().and_then(|kb| kb.repair()),
            move |result| {
                let Some(win) = w.upgrade() else { return };
                win.connecting.set(false);
                match result {
                    Ok(used) => {
                        win.damaged.set(false);
                        win.toast(&format!("Repaired: put back the keyboard's {used}"), 5);
                        win.connect_keyboard(false);
                    }
                    Err(e) => win.disconnected(&format!("Repair failed: {e}"), true),
                }
            },
        );
    }

    fn poll(self: &Rc<Self>) {
        self.polls.set(self.polls.get() + 1);
        let kb = self.kb();
        match kb {
            Some(kb) if !kb.alive() => {
                if self.music.borrow().is_some() {
                    self.stop_music(None);
                }
                self.disconnected("The keyboard was unplugged.", false);
            }
            None => self.connect_keyboard(true),
            Some(kb) => {
                self.follow_keyboard(&kb);
                if self.polls.get() % BATTERY_EVERY_POLLS == 0
                    && self.music.borrow().is_none()
                    && !self.music_starting.get()
                {
                    let w = Rc::downgrade(self);
                    let k = kb.clone();
                    self.worker.submit(
                        move || k.read_battery(),
                        move |battery| {
                            if let Some(win) = w.upgrade() {
                                if win.kb().is_some_and(|cur| Arc::ptr_eq(&cur, &kb)) {
                                    win.show_battery(battery);
                                }
                            }
                        },
                    );
                }
            }
        }
    }

    /// Pick up changes made on the keyboard itself (Fn shortcuts), so the
    /// window and its preview show what the keyboard shows. Reads only.
    fn follow_keyboard(self: &Rc<Self>, kb: &Arc<Keyboard>) {
        let every = if kb.wired() { 1 } else { 2 }; // radio time is scarcer over 2.4G
        let busy = self.music.borrow().is_some()
            || self.music_starting.get()
            || self.connecting.get()
            || self.apply_source.borrow().is_some()
            || self.apply_queued.load(Ordering::SeqCst);
        if busy || !self.window.is_visible() || self.polls.get() % every != 0 {
            return;
        }
        let edits = self.edits.get();
        let (k, w) = (kb.clone(), Rc::downgrade(self));
        self.worker.submit(
            move || k.read_lighting(),
            move |result| {
                let Some(win) = w.upgrade() else { return };
                // Ignore it if something was changed in the window meanwhile.
                if win.edits.get() != edits || win.apply_queued.load(Ordering::SeqCst) {
                    return;
                }
                match result {
                    Ok(state) if state != *win.state.borrow() => win.show_state(state),
                    Ok(_) => {}
                    Err(Error::Damaged(m)) => win.disconnected(&m, true),
                    Err(Error::Io(_)) => win.disconnected("Lost connection to the keyboard.", false),
                    Err(_) => {} // e.g. asleep over 2.4G; the next poll tries again
                }
            },
        );
    }

    /// Battery level over 2.4G; over the cable the keyboard runs on USB power.
    fn show_battery(&self, battery: Option<Battery>) {
        let Some(kb) = self.kb() else { return };
        let text = battery_text(battery.as_ref(), kb.wired(), kb.last_battery());
        self.battery.set(battery);
        self.battery_row.set_subtitle(&text);
        let suffix = battery.map(|b| format!(" · Battery {b}")).unwrap_or_default();
        self.status_label.set_label(&format!("{}{suffix}", kb.description()));
        self.tray_refresh();
    }

    // ------------------------------------------------------------------ power

    fn sleep_changed(self: &Rc<Self>) {
        let keep_on = self.keep_on_row.is_active();
        self.sleep_row.set_sensitive(self.kb().is_some() && !keep_on && self.music.borrow().is_none());
        self.tray_refresh();
        let Some(kb) = self.kb() else { return };
        if self.loading.get() {
            return;
        }
        self.edits.set(self.edits.get() + 1);
        let minutes = if keep_on { 0.0 } else { SLEEP_MINUTES[self.sleep_row.selected() as usize] };
        self.state.borrow_mut().sleep = minutes;
        self.pending.lock().unwrap().sleep = minutes;
        let message = if minutes == 0.0 {
            "Backlight stays on".to_string()
        } else {
            format!("Backlight turns off after {} idle", sleep_label(minutes))
        };
        let w = Rc::downgrade(self);
        self.worker.submit(
            move || kb.set_sleep(minutes),
            move |result| {
                let Some(win) = w.upgrade() else { return };
                match result {
                    Ok(()) => win.toast(&message, 2),
                    Err(e) => win.io_failed(e),
                }
            },
        );
    }

    // ------------------------------------------------------------------ lighting

    fn effect_index(&self, id: u8) -> u32 {
        if let Some(i) = self.effects.borrow().iter().position(|e| e.id == id) {
            return i as u32;
        }
        let info = effect_info(id);
        let name = format!("{} (set on keyboard)", info.name);
        self.effects.borrow_mut().push(Effect { id, name: name.clone(), speed: true, color: true });
        if let Some(model) = self.effect_row.model().and_downcast::<gtk::StringList>() {
            model.append(&name);
        }
        self.effects.borrow().len() as u32 - 1
    }

    fn show_state(self: &Rc<Self>, state: Lighting) {
        self.loading.set(true);
        *self.state.borrow_mut() = state.clone();
        *self.pending.lock().unwrap() = state.clone();
        let i = self.effect_index(state.effect);
        self.effect_row.set_selected(i);
        self.rainbow_row.set_active(state.colorful);
        self.color_button.set_rgba(&rgba(state.color));
        self.brightness.set_value(state.brightness as f64);
        self.speed.set_value(state.speed as f64);
        if (state.side as usize) < SIDE_MODES.len() {
            self.side_row.set_selected(state.side as u32);
        }
        self.keep_on_row.set_active(state.sleep == 0.0);
        if state.sleep != 0.0 {
            self.sleep_row.set_selected(nearest_sleep_index(state.sleep) as u32);
        }
        self.loading.set(false);
        self.sync_widgets();
    }

    fn read_widgets(&self) -> Lighting {
        let index = self.effect_row.selected() as usize;
        let effect = self.effects.borrow().get(index).map(|e| e.id).unwrap_or(self.state.borrow().effect);
        Lighting {
            effect,
            brightness: self.brightness.value() as u8,
            speed: self.speed.value() as u8,
            colorful: self.rainbow_row.is_active(),
            color: rgb(&self.color_button.rgba()),
            side: self.side_row.selected() as u8,
            ..self.state.borrow().clone() // sleep, and what only the keyboard sets
        }
    }

    fn sync_widgets(&self) {
        let running = self.music.borrow().is_some();
        let busy = running || self.music_starting.get();
        let has_kb = self.kb().is_some();
        let connected = has_kb && !busy; // music sync owns the keyboard while on
        let state = self.state.borrow().clone();
        let info = effect_info(state.effect);
        let timed = (1..=18).contains(&state.effect);
        for widget in self.lighting_widgets() {
            widget.set_sensitive(connected);
        }
        self.rainbow_row.set_sensitive(connected && info.color);
        self.brightness_row.set_sensitive(connected && timed);
        self.speed_row.set_sensitive(connected && timed && info.speed);
        self.sleep_row.set_sensitive(connected && !self.keep_on_row.is_active());
        self.apply_keys_button.set_sensitive(connected);
        self.refresh_button.set_sensitive(!busy);
        self.preview.queue_draw();

        self.music_button.set_label(if running { "Stop" } else { "Start" });
        self.music_button.remove_css_class(if running { "suggested-action" } else { "destructive-action" });
        self.music_button.add_css_class(if running { "destructive-action" } else { "suggested-action" });
        self.music_button.set_sensitive(has_kb && !self.music_starting.get());
        if running {
            let label = self.kb().map(|k| format!("On · {}", k.label)).unwrap_or_else(|| "On".into());
            self.music_row.set_subtitle(&label);
        } else {
            self.music_row.set_subtitle(if self.music_starting.get() {
                "Starting…"
            } else if has_kb {
                "Stopped"
            } else {
                "Connect the keyboard first"
            });
        }
        self.tray_refresh();
    }

    fn pick_preset(self: &Rc<Self>, color: [u8; 3]) {
        self.color_button.set_rgba(&rgba(color));
        self.color_picked(); // notify::rgba doesn't fire if the colour is unchanged
    }

    /// Picking a colour means "make it this colour": leave rainbow mode and,
    /// if the current effect can't take a colour, switch to Static.
    fn color_picked(self: &Rc<Self>) {
        if self.loading.get() {
            return;
        }
        self.loading.set(true);
        if !effect_info(self.state.borrow().effect).color {
            let i = self.effect_index(1);
            self.effect_row.set_selected(i);
        }
        self.rainbow_row.set_active(false);
        self.loading.set(false);
        self.changed();
    }

    fn changed(self: &Rc<Self>) {
        if self.loading.get() {
            return;
        }
        self.edits.set(self.edits.get() + 1);
        let state = self.read_widgets();
        *self.state.borrow_mut() = state.clone();
        *self.pending.lock().unwrap() = state;
        self.sync_widgets();
        if self.kb().is_some() {
            if let Some(id) = self.apply_source.borrow_mut().take() {
                id.remove();
            }
            let w = Rc::downgrade(self);
            let id = glib::timeout_add_local_once(Duration::from_millis(APPLY_DELAY_MS), move || {
                if let Some(win) = w.upgrade() {
                    win.apply_source.borrow_mut().take();
                    win.apply_lighting();
                }
            });
            *self.apply_source.borrow_mut() = Some(id);
        }
    }

    fn apply_lighting(self: &Rc<Self>) {
        let Some(kb) = self.kb() else { return };
        if self.apply_queued.swap(true, Ordering::SeqCst) {
            return; // a queued job will send the newest state
        }
        let (queued, pending) = (self.apply_queued.clone(), self.pending.clone());
        let w = Rc::downgrade(self);
        self.worker.submit(
            move || {
                queued.store(false, Ordering::SeqCst);
                let state = pending.lock().unwrap().clone();
                kb.apply_lighting(&state)
            },
            move |result| {
                if let (Some(win), Err(e)) = (w.upgrade(), result) {
                    win.io_failed(e);
                }
            },
        );
    }

    fn io_failed(self: &Rc<Self>, e: Error) {
        if let Error::Damaged(m) = &e {
            self.disconnected(m, true);
            return;
        }
        if matches!(e, Error::Io(_)) || self.kb().is_some_and(|kb| !kb.alive()) {
            self.disconnected("Lost connection to the keyboard.", false);
        }
        self.toast(&format!("Couldn't update the keyboard: {e}"), 5);
    }

    /// The preview imitates the keyboard's current effect (see preview.rs).
    fn preview_color(&self, led: usize, t: f64) -> [u8; 3] {
        super::preview::key_color(&self.state.borrow(), &self.keys.borrow(), led, t)
    }

    // ------------------------------------------------------------------ per-key

    fn paint_key(&self, led: usize, erase: bool) {
        let color = if erase { [0, 0, 0] } else { self.brush.get() };
        if self.keys.borrow()[led] != color {
            self.keys.borrow_mut()[led] = color;
            self.paint_view.queue_draw();
        }
    }

    fn fill_keys(&self, color: [u8; 3]) {
        {
            let mut keys = self.keys.borrow_mut();
            for key in keys_layout() {
                keys[key] = color;
            }
        }
        self.paint_view.queue_draw();
    }

    fn apply_keys(self: &Rc<Self>) {
        let Some(kb) = self.kb() else { return };
        let keys = *self.keys.borrow();
        self.apply_keys_button.set_sensitive(false);
        let w = Rc::downgrade(self);
        self.worker.submit(
            move || kb.apply_keys(&keys),
            move |result| {
                let Some(win) = w.upgrade() else { return };
                win.apply_keys_button.set_sensitive(true);
                match result {
                    Ok(()) => {
                        let mut state = win.state.borrow().clone();
                        state.effect = PER_KEY;
                        win.show_state(state);
                        win.toast("Per-key colors applied", 2);
                    }
                    Err(e) => win.io_failed(e),
                }
            },
        );
    }
}

fn keys_layout() -> impl Iterator<Item = usize> {
    keys().into_iter().map(|k| k.led)
}


// ---------------------------------------------------------------------- self-test

#[cfg(feature = "fake-keyboard")]
impl Win {
    /// Drives the window like a user and prints what happened (AULA_F87_GUI_TEST=1).
    pub fn self_test(self: &Rc<Self>) {
        use crate::fake::WRITES;
        let damaged_run = std::env::var("AULA_F87_FAKE").is_ok_and(|v| v == "damaged");
        type Step = Box<dyn Fn(&Rc<Win>)>;
        let report = |w: &Rc<Win>, what: &str| {
            println!(
                "{what}: connected={} banner={} button={:?} status={:?} music={} music_on={} visible={} writes={:?}",
                w.kb().is_some(),
                w.banner.is_revealed(),
                w.banner.button_label().map(|s| s.to_string()),
                w.status_label.label().to_string(),
                w.music.borrow().is_some(),
                w.settings.borrow().music_on,
                w.window.is_visible(),
                WRITES.lock().unwrap().clone(),
            );
        };
        let steps: Vec<(u64, Step)> = if damaged_run {
            vec![
                (3000, Box::new(move |w| {
                    report(w, "damaged");
                    let menu: Vec<String> = w.tray_menu().iter().map(|i| i.label.clone()).collect();
                    println!("tray has Repair: {}", menu.iter().any(|l| l == "Repair keyboard settings"));
                    WRITES.lock().unwrap().clear();
                    w.banner.emit_by_name::<()>("button-clicked", &[]);
                })),
                (3000, Box::new(move |w| report(w, "repaired"))),
                (100, Box::new(|w| w.quit_app())),
            ]
        } else {
            vec![
                (1500, Box::new(move |w| report(w, "start"))),
                (100, Box::new(|w| w.toggle_music())),
                (2500, Box::new(move |w| {
                    let frames = w.music.borrow().as_ref().map(|m| m.frames.load(Ordering::SeqCst)).unwrap_or(0);
                    report(w, "music on");
                    println!("frames sent: {frames}");
                    w.window.close();
                })),
                (1500, Box::new(move |w| report(w, "window closed"))),
                (100, Box::new(|w| w.show_window())),
                (2500, Box::new(move |w| report(w, "window reopened"))),
                (100, Box::new(|w| w.toggle_music())),
                (1500, Box::new(|w| {
                    WRITES.lock().unwrap().clear();
                    for n in [0.0, 1.0, 2.0, 3.0, 4.0, 3.0, 2.0] {
                        w.brightness.set_value(n);
                    }
                })),
                (2500, Box::new(move |w| report(w, "7 slider moves"))),
                (100, Box::new(|w| w.quit_app())),
            ]
        };
        let steps = Rc::new(steps);
        let mut delay = 0;
        for i in 0..steps.len() {
            delay += steps[i].0;
            let (w, s) = (self.clone(), steps.clone());
            glib::timeout_add_local_once(Duration::from_millis(delay), move || (s[i].1)(&w));
        }
    }
}
