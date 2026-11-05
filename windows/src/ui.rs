//! The window: Lighting, Per-key colors, Music sync and About pages in a
//! NavigationView, with WinUI's own controls on Mica. It shows the app's
//! state (UiState) and sends what the user does back to the app (app.rs).
//! Closing it only closes the window; the app stays in the tray.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use aula_f87::about;
use aula_f87::device::{Frame, Lighting};
use aula_f87::effects::{MUSIC_FOLLOW, MUSIC_STYLES, PRESETS, SIDE_MODES, SLEEP_MINUTES, effect_info, sleep_label};
use aula_f87::preview;
use windows_reactor::*;

use crate::app::{Banner, Msg, SharedFrame, icon_path};
use crate::keyview;

/// What the window shows; built by App::ui_state.
#[derive(Clone, PartialEq)]
pub struct UiState {
    pub on: Callback<Msg>,
    pub status: String,
    pub banner: Option<Banner>,
    pub toast: Option<String>,
    pub has_kb: bool,
    pub connected: bool,
    pub busy: bool,
    pub running: bool,
    pub starting: bool,
    pub state: Lighting,
    pub effect_names: Vec<String>,
    pub effect_index: Option<usize>,
    pub keep_on: bool,
    pub sleep_index: usize,
    pub battery_text: String,
    pub keys: Box<Frame>,
    pub brush: [u8; 3],
    pub music_style: usize,
    pub sensitivity: f64,
    pub music_brightness: f64,
    pub follow: Vec<String>,
    pub music_status: String,
    pub slow_port: bool,
    pub music_frame: Option<SharedFrame>,
    pub tab: String,
    pub window_size: [i32; 2],
    pub keyboard: Option<String>,
}

pub enum UiMsg {
    Fwd(Msg),
    /// Pointer over the paint board: canvas x, y, left / right button down.
    Paint(f64, f64, bool, bool),
    PaintEnd,
}

pub struct MainWindow {
    on: Callback<Msg>,
    last_paint: Option<usize>,
    debug: String,
}

/// What the live preview shows.
#[derive(Clone, PartialEq)]
pub enum PreviewSource {
    /// An imitation of the keyboard's own effect (and its light bar / side lights).
    Effect(Lighting, Box<Frame>),
    /// Music sync's latest frame (or unlit while it's off).
    Music(Option<SharedFrame>),
}

/// The animated keyboard preview: its own component with its own timer, so
/// only the board redraws on each tick, not the whole page.
pub struct Preview {
    start: Instant,
    t: Cell<f64>,
    fast: Cell<bool>,
    timer: Option<ComponentTimer>,
}

pub struct Tick;

impl Component for Preview {
    type Input = PreviewSource;
    type Message = Tick;

    fn create(_input: &PreviewSource, ctx: &ComponentContext<Self>) -> Self {
        Preview {
            start: Instant::now(),
            t: Cell::new(0.0),
            fast: Cell::new(true),
            timer: Some(ctx.set_timeout(Duration::from_millis(50), Tick)),
        }
    }

    fn update(&mut self, _tick: Tick, ctx: &ComponentContext<Self>) {
        self.t.set(self.start.elapsed().as_secs_f64());
        // 20 fps while something moves (the keyboard itself runs at about 15
        // over the cable), otherwise a slow check once a second.
        let next = if self.fast.get() { 50 } else { 1000 };
        self.timer = Some(ctx.set_timeout(Duration::from_millis(next), Tick));
    }

    fn view(&self, input: &PreviewSource, _ctx: &mut ViewContext<Self>) -> View {
        let t = self.t.get();
        let board = match input {
            PreviewSource::Effect(state, keys) => {
                self.fast.set(preview::animated(state));
                let strips = preview::strips(state, t);
                let (st, keys) = (state.clone(), keys.clone());
                keyview::board(&move |led| preview::key_color(&st, &keys, led, t), Some(&strips))
            }
            PreviewSource::Music(frame) => {
                self.fast.set(frame.is_some());
                let f: Frame = frame.as_ref().map(|f| *f.0.lock().unwrap()).unwrap_or([[0; 3]; aula_f87::layout::PLANE]);
                keyview::board(&move |led| f[led], None)
            }
        };
        Viewbox::new().stretch(Stretch::Uniform).content(board).into()
    }
}

fn fwd(m: Msg) -> UiMsg {
    UiMsg::Fwd(m)
}

fn rgb(c: [u8; 3]) -> Color {
    Color::rgb(c[0], c[1], c[2])
}

/// A button with a leading icon (every button carries one).
fn icon_button(glyph: &str, label: &str) -> Button {
    Button::new().content(
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(8.0)
            .children((FontIcon::new().glyph(glyph).font_size(16.0), TextBlock::new().text(label))),
    )
}

fn title(text: &str) -> TextBlock {
    TextBlock::new().text(text).font_size(18.0).font_weight(FontWeight::SEMI_BOLD)
}

fn dim(text: &str) -> TextBlock {
    TextBlock::new().text(text).opacity(0.7).text_wrapping(TextWrapping::Wrap)
}

/// A setting row: title and explanation on the left, the control on the right.
thread_local! {
    /// The page is narrow (phone-sized window): rows put the control under its label.
    static NARROW: Cell<bool> = const { Cell::new(false) };
}

fn narrow() -> bool {
    NARROW.with(|n| n.get())
}

fn row(name: &str, subtitle: Option<&str>, control: impl Into<View>) -> View {
    if narrow() {
        return row_stacked(name, subtitle, control);
    }
    let mut text: Vec<View> = vec![TextBlock::new().text(name).into()];
    if let Some(s) = subtitle {
        text.push(dim(s).font_size(12.0).into());
    }
    Grid::new()
        .columns([GridLength::STAR, GridLength::Auto])
        .column_spacing(16.0)
        .children((
            StackPanel::new().spacing(2.0).vertical_alignment(VerticalAlignment::Center).children(text),
            Border::new().grid_column(1).vertical_alignment(VerticalAlignment::Center).content(control),
        ))
        .into()
}

/// Label (and explanation) above, the control below at full width.
fn row_stacked(name: &str, subtitle: Option<&str>, control: impl Into<View>) -> View {
    let mut items: Vec<View> = vec![TextBlock::new().text(name).into()];
    if let Some(s) = subtitle {
        items.push(dim(s).font_size(12.0).into());
    }
    items.push(Border::new().margin(Thickness::new(0.0, 4.0, 0.0, 0.0)).content(control).into());
    StackPanel::new().spacing(2.0).children(items).into()
}

/// Items that wrap onto the next line when the page is narrow.
fn wrap(items: Vec<View>) -> View {
    VariableSizedWrapGrid::new().orientation(Orientation::Horizontal).children(items).into()
}

/// A combo box: fixed width beside its label, full width under it.
fn sized_combo(c: ComboBox) -> ComboBox {
    if narrow() { c.horizontal_alignment(HorizontalAlignment::Stretch) } else { c.min_width(240.0) }
}

fn sized_slider(s: Slider) -> Slider {
    if narrow() { s.horizontal_alignment(HorizontalAlignment::Stretch) } else { s.width(240.0) }
}

fn card(heading: &str, rows: Vec<View>) -> View {
    let mut items: Vec<View> = vec![title(heading).font_size(16.0).into()];
    items.extend(rows);
    Border::new()
        .background(ThemeBrush::CardBackground)
        .border_brush(ThemeBrush::CardStroke)
        .border_thickness(1.0)
        .corner_radius(10.0)
        .padding(Thickness::xy(18.0, 16.0))
        .content(StackPanel::new().spacing(14.0).children(items))
        .into()
}

fn swatch(c: [u8; 3], size: f64) -> Border {
    Border::new()
        .width(size)
        .height(size)
        .corner_radius(size / 3.0)
        .background(rgb(c))
        .border_brush(Color::argb(90, 128, 128, 128))
        .border_thickness(1.0)
}

/// The live preview, scaled to the page width, at most `max_height` tall.
fn preview_box(source: PreviewSource, max_height: f64) -> View {
    Border::new().corner_radius(12.0).padding(8.0).max_height(max_height).content(View::component::<Preview>(source)).into()
}

/// Controls that music sync has paused: hovering explains why.
fn paused(content: View, busy: bool) -> View {
    if busy {
        Border::new().content(content).tooltip("Music sync is on, so the lighting can't be changed right now. Stop music sync to edit it.")
    } else {
        content
    }
}

impl Component for MainWindow {
    type Input = UiState;
    type Message = UiMsg;

    fn create(input: &UiState, ctx: &ComponentContext<Self>) -> Self {
        let _ = ctx;
        input.on.call(Msg::WindowOpened);
        MainWindow { on: input.on.clone(), last_paint: None, debug: debug_info() }
    }

    fn input_changed(&mut self, input: &UiState, _ctx: &ComponentContext<Self>) {
        self.on = input.on.clone();
    }

    fn update(&mut self, msg: UiMsg, _ctx: &ComponentContext<Self>) {
        match msg {
            UiMsg::Fwd(m) => self.on.call(m),
            UiMsg::Paint(x, y, left, right) => {
                if !(left || right) {
                    self.last_paint = None;
                    return;
                }
                if let Some(led) = keyview::hit(x, y) {
                    if self.last_paint != Some(led) {
                        self.last_paint = Some(led);
                        self.on.call(Msg::Paint(led, right));
                    }
                }
            }
            UiMsg::PaintEnd => self.last_paint = None,
        }
    }

    fn view(&self, s: &UiState, ctx: &mut ViewContext<Self>) -> View {
        ctx.window_visuals(
            WindowVisuals::new()
                .backdrop(WindowBackdrop::Mica)
                .client_size(s.window_size[0] as f64, s.window_size[1] as f64)
                .constraints(WindowConstraints { min_width: Some(400.0), min_height: Some(480.0), ..Default::default() })
                .icon(icon_path().to_string_lossy().to_string()),
        );
        ctx.on_window_size(ctx.callback(|size: WindowSize| fwd(Msg::WindowSize(size.width as i32, size.height as i32))));
        // The menu folds by itself (NavigationView Auto: full pane from 1008 px,
        // icons from 641, a ☰ button below); the page follows its own width.
        let w = s.window_size[0] as f64;
        let pane = if w >= 1008.0 { 220.0 } else if w >= 641.0 { 48.0 } else { 0.0 };
        let content = w - pane - 56.0;
        let wide = content >= 860.0;
        NARROW.with(|n| n.set(content < 560.0));
        let page = match s.tab.as_str() {
            "per-key" => self.per_key_page(s, ctx),
            "music" => self.music_page(s, ctx),
            "about" => self.about_page(s),
            _ => self.lighting_page(s, ctx, wide),
        };
        let item = |tag: &str, label: &str, glyph: &str| -> View {
            NavigationViewItem::new()
                .content(label)
                .icon(Icon::font(glyph))
                .tag(tag)
                .is_selected(s.tab == tag || (tag == "lighting" && !["per-key", "music", "about"].contains(&s.tab.as_str())))
                .into()
        };
        let nav = NavigationView::new()
            .pane_display_mode(NavigationViewPaneDisplayMode::Auto)
            .is_settings_visible(false)
            .is_back_button_visible(NavigationViewBackButtonVisible::Collapsed)
            .open_pane_length(220.0)
            .menu_items((
                item("lighting", "Lighting", "\u{E706}"),
                item("per-key", "Per-key colors", "\u{E765}"),
                item("music", "Music sync", "\u{E8D6}"),
            ))
            .footer_menu_items((item("about", "About", "\u{E946}"),))
            .on_selected_tag_changed(ctx.callback(|tag: Option<Rc<str>>| match tag {
                Some(t) => fwd(Msg::Tab(t.to_string())),
                None => fwd(Msg::Noop),
            }))
            .content(
                ScrollViewer::new().content(
                    StackPanel::new()
                        .spacing(16.0)
                        // Below 641 px the menu is a ☰ button over the page's top-left corner.
                        .margin(if s.window_size[0] < 641 { Thickness::new(16.0, 52.0, 16.0, 24.0) } else { Thickness::new(28.0, 20.0, 28.0, 28.0) })
                        .children((self.header(s, ctx), page)),
                ),
            );
        ctx.window_frame("AULA F87 Control", nav)
    }
}

impl Drop for MainWindow {
    fn drop(&mut self) {
        self.on.call(Msg::WindowClosed);
    }
}

impl MainWindow {
    /// Status line, the problem banner with its fix, and short notices.
    fn header(&self, s: &UiState, ctx: &mut ViewContext<Self>) -> View {
        let mut items: Vec<View> = vec![
            Grid::new()
                .columns([GridLength::STAR, GridLength::Auto])
                .children((
                    StackPanel::new().spacing(2.0).children((
                        title(match s.tab.as_str() {
                            "per-key" => "Per-key colors",
                            "music" => "Music sync",
                            "about" => "About",
                            _ => "Lighting",
                        })
                        .font_size(26.0),
                        dim(&s.status),
                    )),
                    icon_button("\u{E72C}", "Reconnect")
                        .grid_column(1)
                        .vertical_alignment(VerticalAlignment::Center)
                        .is_enabled(!s.busy)
                        .on_click(ctx.callback(|_| fwd(Msg::Reconnect)))
                        .tooltip("Reconnect and reload from keyboard"),
                ))
                .into(),
        ];
        if let Some(b) = &s.banner {
            items.push(
                StackPanel::new()
                    .spacing(8.0)
                    .children((
                        InfoBar::new()
                            .is_open(true)
                            .is_closable(false)
                            .severity(if b.damaged { InfoBarSeverity::Error } else { InfoBarSeverity::Warning })
                            .title(if b.damaged { "Keyboard settings damaged" } else { "Keyboard not connected" })
                            .message(&b.message),
                        if b.damaged { icon_button("\u{E90F}", "Repair") } else { icon_button("\u{E72C}", "Retry") }
                            .style(ButtonStyle::Accent)
                            .on_click(ctx.callback(|_| fwd(Msg::BannerAction))),
                    ))
                    .into(),
            );
        }
        // Music sync owns the keyboard while it runs: say so where the controls are paused,
        // with the way out right under it.
        if (s.running || s.starting) && s.tab != "music" && s.tab != "about" {
            items.push(
                StackPanel::new()
                    .spacing(8.0)
                    .children((
                        InfoBar::new()
                            .is_open(true)
                            .is_closable(false)
                            .severity(InfoBarSeverity::Informational)
                            .title("Music sync is on")
                            .message("The keys are following the music, so the lighting can't be changed right now. Stop music sync to edit it; your saved lighting comes back by itself."),
                        StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((
                            icon_button("\u{E71A}", "Stop music sync")
                                .style(ButtonStyle::Accent)
                                .is_enabled(s.running)
                                .on_click(ctx.callback(|_| fwd(Msg::ToggleMusic))),
                            icon_button("\u{E8D6}", "Music sync settings").on_click(ctx.callback(|_| fwd(Msg::Tab("music".into())))),
                        )),
                    ))
                    .into(),
            );
        }
        if let Some(t) = &s.toast {
            items.push(InfoBar::new().is_open(true).is_closable(false).severity(InfoBarSeverity::Informational).message(t).into());
        }
        StackPanel::new().spacing(12.0).children(items).into()
    }

    // ------------------------------------------------------------------ lighting

    fn lighting_page(&self, s: &UiState, ctx: &mut ViewContext<Self>, wide: bool) -> View {
        let state = &s.state;

        let info = effect_info(state.effect);
        let timed = (1..=18).contains(&state.effect);
        let on = s.connected;

        let effect = card(
            "Effect",
            vec![
                row(
                    "Lighting effect",
                    None,
                    sized_combo(ComboBox::new())
                        .items_source(s.effect_names.iter().map(|n| n.as_str()))
                        .selected_index(s.effect_index)
                        .is_enabled(on)
                        .on_selection_changed(ctx.callback(|i: Option<usize>| match i {
                            Some(i) => fwd(Msg::Effect(i)),
                            None => fwd(Msg::Noop),
                        })),
                ),
                row(
                    "Rainbow",
                    Some("Use all colors instead of a single one"),
                    ToggleSwitch::new()
                        .is_on(state.colorful)
                        .is_enabled(on && info.color)
                        .on_content("")
                        .off_content("")
                        .on_toggled(ctx.callback(|v: bool| fwd(Msg::Rainbow(v)))),
                ),
            ],
        );

        let presets: Vec<View> = PRESETS
            .iter()
            .map(|(name, c)| {
                let c = *c;
                Button::new()
                    .is_enabled(on)
                    .content(swatch(c, 22.0))
                    .on_click(ctx.callback(move |_| fwd(Msg::Preset(c))))
                    .tooltip(*name)
            })
            .collect();
        let color = card(
            "Color",
            vec![
                row(
                    "Color",
                    Some("Pick any color"),
                    Button::new()
                        .is_enabled(on)
                        .content(
                            StackPanel::new()
                                .orientation(Orientation::Horizontal)
                                .spacing(8.0)
                                .children((FontIcon::new().glyph("\u{E790}").font_size(16.0), swatch(state.color, 18.0), TextBlock::new().text("Choose"))),
                        )
                        .flyout_with(Flyout::rich(
                            ColorPicker::new()
                                .color(rgb(state.color))
                                .is_alpha_enabled(false)
                                .is_hex_input_visible(true)
                                .on_color_changed(ctx.callback(|c: Color| fwd(Msg::Color([c.r, c.g, c.b])))),
                        )),
                ),
                row_stacked("Quick colors", None, wrap(presets)),
            ],
        );

        let level = |value: u8, enabled: bool, cb: Callback<f64>| {
            sized_slider(Slider::new())
                .minimum(0.0)
                .maximum(4.0)
                .step_frequency(1.0)
                .value(value as f64)
                .is_enabled(enabled)
                .on_value_changed(cb)
        };
        let levels = card(
            "Brightness and speed",
            vec![
                row("Brightness", None, level(state.brightness, on && timed, ctx.callback(|v: f64| fwd(Msg::Brightness(v))))),
                row("Speed", None, level(state.speed, on && timed && info.speed, ctx.callback(|v: f64| fwd(Msg::Speed(v))))),
            ],
        );

        let side = card(
            "Side lights",
            vec![row(
                "Side lights effect",
                Some("Fn + Right Shift on the keyboard"),
                sized_combo(ComboBox::new())
                    .items_source(SIDE_MODES.iter().copied())
                    .selected_index(((state.side as usize) < SIDE_MODES.len()).then_some(state.side as usize))
                    .is_enabled(on)
                    .on_selection_changed(ctx.callback(|i: Option<usize>| match i {
                        Some(i) => fwd(Msg::Side(i)),
                        None => fwd(Msg::Noop),
                    })),
            )],
        );

        let sleep_names: Vec<String> = SLEEP_MINUTES.iter().map(|m| sleep_label(*m)).collect();
        let power = card(
            "Power",
            vec![
                row("Battery", Some(&s.battery_text), TextBlock::new().text("")),
                row(
                    "Keep backlight on",
                    Some("Never turn the backlight off when idle. Uses more battery on 2.4G."),
                    ToggleSwitch::new()
                        .is_on(s.keep_on)
                        .is_enabled(on)
                        .on_content("")
                        .off_content("")
                        .on_toggled(ctx.callback(|v: bool| fwd(Msg::KeepOn(v)))),
                ),
                row(
                    "Turn backlight off after",
                    Some("Idle time without a key press"),
                    sized_combo(ComboBox::new())
                        .items_source(sleep_names.iter().map(|n| n.as_str()))
                        .selected_index(Some(s.sleep_index))
                        .is_enabled(on && !s.keep_on)
                        .on_selection_changed(ctx.callback(|i: Option<usize>| match i {
                            Some(i) => fwd(Msg::SleepIndex(i)),
                            None => fwd(Msg::Noop),
                        })),
                ),
            ],
        );

        let b = s.busy;
        let (effect, color, levels, side, power) =
            (paused(effect, b), paused(color, b), paused(levels, b), paused(side, b), paused(power, b));
        let cards: View = if wide {
            Grid::new()
                .columns([GridLength::STAR, GridLength::STAR])
                .column_spacing(16.0)
                .children((
                    StackPanel::new().spacing(16.0).children((effect, color)),
                    StackPanel::new().grid_column(1).spacing(16.0).children((levels, side, power)),
                ))
                .into()
        } else {
            StackPanel::new().spacing(16.0).children((effect, color, levels, side, power)).into()
        };
        let preview = if s.running {
            PreviewSource::Music(s.music_frame.clone()) // what the keyboard shows right now
        } else {
            PreviewSource::Effect(state.clone(), s.keys.clone())
        };
        StackPanel::new().spacing(16.0).children((preview_box(preview, 360.0), cards)).into()
    }

    // ------------------------------------------------------------------ per-key

    fn per_key_page(&self, s: &UiState, ctx: &mut ViewContext<Self>) -> View {
        let keys = s.keys.clone();
        let board = keyview::board(&move |led| keys[led], None);
        let paint = Border::new()
            .background(Color::transparent())
            .content(board)
            .on_pointer_pressed(ctx.callback(|p: PointerEventInfo| {
                UiMsg::Paint(p.x, p.y, p.is_left_button_pressed, p.is_right_button_pressed)
            }))
            .on_pointer_moved(ctx.callback(|p: PointerEventInfo| {
                UiMsg::Paint(p.x, p.y, p.is_left_button_pressed, p.is_right_button_pressed)
            }))
            .on_pointer_released(ctx.callback(|_: PointerEventInfo| UiMsg::PaintEnd))
            .on_pointer_exited(ctx.callback(|_: PointerEventInfo| UiMsg::PaintEnd));
        let presets: Vec<View> = PRESETS
            .iter()
            .map(|(name, c)| {
                let c = *c;
                Button::new()
                    .content(swatch(c, 22.0))
                    .on_click(ctx.callback(move |_| fwd(Msg::Brush(c))))
                    .tooltip(*name)
            })
            .collect();
        let brush = StackPanel::new().orientation(Orientation::Horizontal).spacing(10.0).children((
            TextBlock::new().text("Brush").vertical_alignment(VerticalAlignment::Center),
            Button::new()
                .content(
                    StackPanel::new()
                        .orientation(Orientation::Horizontal)
                        .spacing(8.0)
                        .children((FontIcon::new().glyph("\u{E790}").font_size(16.0), swatch(s.brush, 18.0))),
                )
                .flyout_with(Flyout::rich(
                    ColorPicker::new()
                        .color(rgb(s.brush))
                        .is_alpha_enabled(false)
                        .is_hex_input_visible(true)
                        .on_color_changed(ctx.callback(|c: Color| fwd(Msg::Brush([c.r, c.g, c.b])))),
                )),
        ));
        let actions = StackPanel::new()
            .orientation(if narrow() { Orientation::Vertical } else { Orientation::Horizontal })
            .spacing(8.0)
            .children((
                icon_button("\u{E7E6}", "Fill all").on_click(ctx.callback(|_| fwd(Msg::Fill))),
                icon_button("\u{E894}", "Clear all").on_click(ctx.callback(|_| fwd(Msg::Clear))),
                icon_button("\u{E8FB}", "Apply to keyboard")
                    .style(ButtonStyle::Accent)
                    .is_enabled(s.connected)
                    .on_click(ctx.callback(|_| fwd(Msg::ApplyKeys))),
            ));
        let tools: View = StackPanel::new().spacing(10.0).children((brush, wrap(presets), actions)).into();
        StackPanel::new()
            .spacing(16.0)
            .children((
                dim("Click or drag over keys to paint them. Right-click clears a key. Press Apply to send the colors to the keyboard."),
                paused(tools, s.busy),
                Border::new()
                    .corner_radius(12.0)
                    .padding(8.0)
                    .max_height(460.0)
                    .content(Viewbox::new().stretch(Stretch::Uniform).content(paint)),
            ))
            .into()
    }

    // ------------------------------------------------------------------ music

    fn music_page(&self, s: &UiState, ctx: &mut ViewContext<Self>) -> View {

        let start = if s.running {
            icon_button("\u{E71A}", "Stop")
        } else {
            icon_button("\u{E768}", "Start").style(ButtonStyle::Accent)
        }
        .is_enabled(s.has_kb && !s.starting)
        .on_click(ctx.callback(|_| fwd(Msg::ToggleMusic)));

        let all = s.follow.is_empty();
        let mut checks: Vec<View> = vec![
            CheckBox::new()
                .content("All")
                .is_checked(Some(all))
                .on_click(ctx.callback(|_| fwd(Msg::Follow("all", true))))
                .into(),
        ];
        for (key, label) in MUSIC_FOLLOW {
            let picked = s.follow.iter().any(|f| f == key);
            checks.push(
                CheckBox::new()
                    .content(label)
                    .is_checked(Some(picked))
                    .on_click(ctx.callback(move |_| fwd(Msg::Follow(key, !picked))))
                    .into(),
            );
        }

        let settings = card(
            "Music sync",
            vec![
                dim("The keys move with whatever this computer is playing, from any app. Your saved lighting comes back when you stop.").into(),
                row("Follow the music", Some(&s.music_status), start),
                row(
                    "Style",
                    None,
                    sized_combo(ComboBox::new())
                        .items_source(MUSIC_STYLES.iter().copied())
                        .selected_index(Some(s.music_style))
                        .on_selection_changed(ctx.callback(|i: Option<usize>| match i {
                            Some(i) => fwd(Msg::MusicStyle(i)),
                            None => fwd(Msg::Noop),
                        })),
                ),
                row_stacked("Follow", Some("What the lights react to"), wrap(checks)),
                row(
                    "Sensitivity",
                    Some("Higher = taller bars"),
                    sized_slider(Slider::new())
                        .minimum(0.2)
                        .maximum(2.0)
                        .step_frequency(0.05)
                        .value(s.sensitivity)
                        .on_value_changed(ctx.callback(|v: f64| fwd(Msg::Sensitivity(v)))),
                ),
                row(
                    "Brightness",
                    None,
                    sized_slider(Slider::new())
                        .minimum(0.1)
                        .maximum(1.0)
                        .step_frequency(0.05)
                        .value(s.music_brightness)
                        .on_value_changed(ctx.callback(|v: f64| fwd(Msg::MusicBrightness(v)))),
                ),
            ],
        );
        let mut items: Vec<View> = vec![];
        if s.slow_port {
            items.push(
                InfoBar::new()
                    .is_open(true)
                    .is_closable(false)
                    .severity(InfoBarSeverity::Warning)
                    .title("This USB port is slow for music sync")
                    .message("The keyboard takes fewer than 20 updates a second here, so the lights lag and sweep from left to right. Plug the keyboard cable into a USB port wired to the processor, usually the blue ones at the back of the PC.")
                    .into(),
            );
        }
        items.push(preview_box(PreviewSource::Music(s.music_frame.clone()), 360.0));
        items.push(settings);
        StackPanel::new().spacing(16.0).children(items).into()
    }

    // ------------------------------------------------------------------ about

    fn about_page(&self, s: &UiState) -> View {
        let mut news: Vec<View> = vec![];
        for release in about::releases().into_iter().take(3) {
            news.push(title(&format!("v{} — {}", release.version, release.date)).font_size(15.0).into());
            for (depth, text) in release.items {
                let plain = text.replace("**", "").replace('`', "").replace('*', "");
                news.push(
                    TextBlock::new()
                        .text(format!("{}• {plain}", if depth > 0 { "      " } else { "" }))
                        .text_wrapping(TextWrapping::Wrap)
                        .into(),
                );
            }
        }
        let link = |label: &str, url: &str| -> View {
            match HyperlinkButton::new().content(label).navigate_uri(url) {
                Ok(b) => b.into(),
                Err(_) => TextBlock::new().text(url).into(),
            }
        };
        let keyboard = s.keyboard.clone().unwrap_or_else(|| "not connected".into());
        StackPanel::new()
            .spacing(16.0)
            .children((
                card(
                    about::NAME,
                    vec![
                        dim(&format!("Version {} (Windows, WinUI 3)", env!("CARGO_PKG_VERSION"))).into(),
                        TextBlock::new().text(about::SUMMARY).text_wrapping(TextWrapping::Wrap).into(),
                        StackPanel::new()
                            .orientation(if narrow() { Orientation::Vertical } else { Orientation::Horizontal })
                            .spacing(8.0)
                            .children((
                                link("Website", about::WEBSITE),
                                link("Report a problem", about::ISSUES),
                                link("Full changelog", about::CHANGELOG_URL),
                            ))
                            .into(),
                        dim(&format!("{} · GPL-3.0-or-later", about::COPYRIGHT)).into(),
                    ],
                ),
                card("What's new", news),
                card(
                    "Debugging information",
                    vec![
                        TextBlock::new()
                            .text(format!("Keyboard: {keyboard}\n{}", self.debug))
                            .is_text_selection_enabled(true)
                            .font_family("Cascadia Mono, Consolas")
                            .text_wrapping(TextWrapping::Wrap)
                            .into(),
                    ],
                ),
            ))
            .into()
    }
}

/// Version, Windows build and the keyboard's HID collections.
fn debug_info() -> String {
    let mut lines = vec![format!("{} {} (Rust, WinUI 3 via windows-reactor)", about::NAME, env!("CARGO_PKG_VERSION"))];
    lines.push(format!("System: {}", crate::sysinfo::windows_version()));
    for (name, id) in [("USB cable", aula_f87::device::WIRED_ID), ("2.4G receiver", aula_f87::device::DONGLE_ID)] {
        let found = aula_f87::hidraw::describe(id);
        if found.is_empty() {
            lines.push(format!("{name} {:04x}:{:04x}: not present", id.0, id.1));
        } else {
            lines.push(format!("{name} {:04x}:{:04x}: {}", id.0, id.1, found.join("; ")));
        }
    }
    lines.push(format!("Settings: {}", aula_f87::device::config_dir().display()));
    lines.join("\n")
}
