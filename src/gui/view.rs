//! The drawn 87-key board (lighting preview, per-key painter, music preview)
//! and the small colour swatches. Drawn with GTK snapshots, like the Python app.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene, gsk, pango};

use crate::layout::{keys, Key, HEIGHT_U, WIDTH_U};

const CASE_COLOR: [f32; 3] = [0.10, 0.10, 0.11];
const UNLIT_COLOR: [f32; 3] = [0.21, 0.21, 0.23];

pub type ColorFor = Rc<dyn Fn(usize) -> [u8; 3]>;
pub type Paint = Rc<dyn Fn(usize, bool)>;

fn fill_rounded(snapshot: &gtk::Snapshot, x: f32, y: f32, w: f32, h: f32, radius: f32, rgb: [f32; 3]) {
    let rect = graphene::Rect::new(x, y, w, h);
    let clip = gsk::RoundedRect::from_rect(rect, radius);
    snapshot.push_rounded_clip(&clip);
    snapshot.append_color(&gdk::RGBA::new(rgb[0], rgb[1], rgb[2], 1.0), &rect);
    snapshot.pop();
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct KeyboardView {
        pub color_for: RefCell<Option<ColorFor>>,
        pub paint: RefCell<Option<Paint>>,
        pub height: Cell<i32>,
        pub hits: RefCell<Vec<(f32, f32, f32, f32, usize)>>,
        pub origin: Cell<(f64, f64)>,
        pub button: Cell<u32>,
        pub keys: RefCell<Vec<Key>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for KeyboardView {
        const NAME: &'static str = "AulaKeyboardView";
        type Type = super::KeyboardView;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for KeyboardView {}

    impl WidgetImpl for KeyboardView {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            if orientation == gtk::Orientation::Horizontal {
                (480, 900, -1, -1)
            } else {
                (self.height.get(), self.height.get(), -1, -1)
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let (width, height) = (widget.width() as f32, widget.height() as f32);
            let pad = 14.0;
            let (wu, hu) = (WIDTH_U as f32, HEIGHT_U as f32);
            let unit = ((width - 2.0 * pad) / wu).min((height - 2.0 * pad) / hu);
            let ox = (width - unit * wu) / 2.0;
            let oy = (height - unit * hu) / 2.0;
            fill_rounded(snapshot, ox - unit * 0.3, oy - unit * 0.3, unit * (wu + 0.6), unit * (hu + 0.6),
                         unit * 0.35, CASE_COLOR);

            let mut font = pango::FontDescription::from_string("Sans");
            font.set_absolute_size((6.0f64).max(unit as f64 * 0.22) * pango::SCALE as f64);
            let dark_text = gdk::RGBA::new(25.0 / 255.0, 25.0 / 255.0, 25.0 / 255.0, 0.85);
            let light_text = gdk::RGBA::new(242.0 / 255.0, 242.0 / 255.0, 242.0 / 255.0, 0.85);
            let gap = unit * 0.08;
            let color_for = self.color_for.borrow().clone();
            let mut hits = vec![];
            for key in self.keys.borrow().iter() {
                let x = ox + key.x as f32 * unit + gap / 2.0;
                let y = oy + key.y as f32 * unit + gap / 2.0;
                let w = key.w as f32 * unit - gap;
                let h = unit - gap;
                let [r, g, b] = color_for.as_ref().map(|f| f(key.led)).unwrap_or([0, 0, 0]);
                let lit = r as u32 + g as u32 + b as u32 > 0;
                let fill = if lit { [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0] } else { UNLIT_COLOR };
                fill_rounded(snapshot, x, y, w, h, unit * 0.12, fill);

                let layout = widget.create_pango_layout(Some(key.label));
                layout.set_font_description(Some(&font));
                let (tw, th) = layout.pixel_size();
                snapshot.save();
                snapshot.translate(&graphene::Point::new(x + (w - tw as f32) / 2.0, y + (h - th as f32) / 2.0));
                let bright = lit && 0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64 > 140.0;
                snapshot.append_layout(&layout, if bright { &dark_text } else { &light_text });
                snapshot.restore();
                hits.push((x, y, w, h, key.led));
            }
            *self.hits.borrow_mut() = hits;
        }
    }

    #[derive(Default)]
    pub struct Swatch {
        pub rgb: Cell<[u8; 3]>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Swatch {
        const NAME: &'static str = "AulaSwatch";
        type Type = super::Swatch;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Swatch {}

    impl WidgetImpl for Swatch {
        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            (22, 22, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let (w, h) = (widget.width() as f32, widget.height() as f32);
            let [r, g, b] = self.rgb.get();
            fill_rounded(snapshot, 0.0, 0.0, w, h, 6.0, [0.5, 0.5, 0.5]);
            fill_rounded(snapshot, 1.0, 1.0, w - 2.0, h - 2.0, 5.0,
                         [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]);
        }
    }
}

glib::wrapper! {
    /// Draws the 87-key board. With `paint` set, click/drag paints keys.
    pub struct KeyboardView(ObjectSubclass<imp::KeyboardView>)
        @extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl KeyboardView {
    pub fn new(color_for: ColorFor, paint: Option<Paint>, height: i32) -> KeyboardView {
        let view: KeyboardView = glib::Object::builder().property("hexpand", true).build();
        let imp = view.imp();
        *imp.color_for.borrow_mut() = Some(color_for);
        *imp.keys.borrow_mut() = keys();
        imp.height.set(height);
        if let Some(paint) = paint {
            *imp.paint.borrow_mut() = Some(paint);
            let drag = gtk::GestureDrag::builder().button(0).build();
            let v = view.downgrade();
            drag.connect_drag_begin(move |gesture, x, y| {
                if let Some(view) = v.upgrade() {
                    view.imp().origin.set((x, y));
                    view.imp().button.set(gesture.current_button());
                    view.paint_at(x, y);
                }
            });
            let v = view.downgrade();
            drag.connect_drag_update(move |_, dx, dy| {
                if let Some(view) = v.upgrade() {
                    let (ox, oy) = view.imp().origin.get();
                    view.paint_at(ox + dx, oy + dy);
                }
            });
            view.add_controller(drag);
            view.set_cursor(gdk::Cursor::from_name("crosshair", None).as_ref());
        }
        view
    }

    fn paint_at(&self, x: f64, y: f64) {
        let (x, y) = (x as f32, y as f32);
        let hit = self.imp().hits.borrow().iter().find(|(kx, ky, kw, kh, _)| {
            *kx <= x && x <= kx + kw && *ky <= y && y <= ky + kh
        }).map(|h| h.4);
        let paint = self.imp().paint.borrow().clone();
        if let (Some(led), Some(paint)) = (hit, paint) {
            paint(led, self.imp().button.get() == 3);
        }
    }
}

glib::wrapper! {
    pub struct Swatch(ObjectSubclass<imp::Swatch>)
        @extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Swatch {
    pub fn new(rgb: [u8; 3]) -> Swatch {
        let s: Swatch = glib::Object::new();
        s.imp().rgb.set(rgb);
        s
    }
}

/// A flat button showing a colour, as in the Python app's quick colours.
pub fn swatch_button(name: &str, rgb: [u8; 3], on_click: impl Fn([u8; 3]) + 'static) -> gtk::Button {
    let button = gtk::Button::builder().child(&Swatch::new(rgb)).tooltip_text(name).valign(gtk::Align::Center).build();
    button.add_css_class("flat");
    button.connect_clicked(move |_| on_click(rgb));
    button
}

pub fn rgba(rgb: [u8; 3]) -> gdk::RGBA {
    gdk::RGBA::parse(format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])).unwrap()
}

pub fn rgb(c: &gdk::RGBA) -> [u8; 3] {
    [
        (c.red() as f64 * 255.0).round() as u8,
        (c.green() as f64 * 255.0).round() as u8,
        (c.blue() as f64 * 255.0).round() as u8,
    ]
}
