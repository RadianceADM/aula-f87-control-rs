//! Screenshots for the docs and for comparing with the Python app:
//! AULA_F87_FAKE=1 AULA_F87_SCREENSHOTS=dir aula-f87-control (fake-keyboard builds only).

use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{glib, graphene};

use super::window::Win;

fn snap(widget: &impl IsA<gtk::Widget>, path: &str) {
    let widget = widget.as_ref();
    let (w, h) = (widget.width(), widget.height());
    let paintable = gtk::WidgetPaintable::new(Some(widget));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, w as f64, h as f64);
    let (Some(node), Some(native)) = (snapshot.to_node(), widget.native()) else { return };
    let Some(renderer) = native.renderer() else { return };
    let texture = renderer.render_texture(&node, Some(&graphene::Rect::new(0.0, 0.0, w as f32, h as f32)));
    let _ = texture.save_to_png(path);
    println!("saved {path}");
}

fn walk(w: gtk::Widget, out: &mut Vec<gtk::Widget>) {
    out.push(w.clone());
    let mut child = w.first_child();
    while let Some(c) = child {
        walk(c.clone(), out);
        child = c.next_sibling();
    }
}

pub fn run(win: Rc<Win>, dir: String) {
    win.window.set_default_size(1060, 1180);
    let steps: Vec<(u64, Box<dyn Fn(&Rc<Win>, &str)>)> = vec![
        (1500, Box::new(|w, d| snap(&w.window, &format!("{d}/lighting.png")))),
        (100, Box::new(|w, _| w.stack.set_visible_child_name("per-key"))),
        (500, Box::new(|w, d| snap(&w.window, &format!("{d}/per-key.png")))),
        (100, Box::new(|w, _| w.stack.set_visible_child_name("music"))),
        (500, Box::new(|w, d| snap(&w.window, &format!("{d}/music.png")))),
        (100, Box::new(|w, _| {
            w.stack.set_visible_child_name("lighting");
            w.show_about();
        })),
        (1200, Box::new(|w, d| snap(&w.window, &format!("{d}/about.png")))),
        (100, Box::new(|w, _| {
            let mut all = vec![];
            walk(w.window.clone().upcast(), &mut all);
            for row in all.into_iter().filter_map(|x| x.downcast::<adw::ActionRow>().ok()) {
                if row.title().contains("New") {
                    if let Some(name) = row.action_name() {
                        let _ = row.activate_action(&name, row.action_target_value().as_ref());
                    }
                }
            }
        })),
        (1000, Box::new(|w, d| snap(&w.window, &format!("{d}/whats-new.png")))),
        (100, Box::new(|w, _| w.quit_app())),
    ];
    let mut delay = 0;
    let steps = Rc::new(steps);
    for i in 0..steps.len() {
        delay += steps[i].0;
        let (w, d, s) = (win.clone(), dir.clone(), steps.clone());
        glib::timeout_add_local_once(Duration::from_millis(delay), move || (s[i].1)(&w, &d));
    }
}
