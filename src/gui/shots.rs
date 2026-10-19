//! Screenshots for the docs and for comparing with the Python app:
//! AULA_F87_FAKE=1 AULA_F87_SCREENSHOTS=dir aula-f87-control (fake-keyboard builds only).

use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{glib, graphene};

use super::window::Win;

/// Save the widget as drawn; false if it hasn't been drawn yet.
fn try_snap(widget: &gtk::Widget, path: &str) -> bool {
    let (w, h) = (widget.width(), widget.height());
    let paintable = gtk::WidgetPaintable::new(Some(widget));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, w as f64, h as f64);
    let (Some(node), Some(native)) = (snapshot.to_node(), widget.native()) else { return false };
    let Some(renderer) = native.renderer() else { return false };
    let texture = renderer.render_texture(&node, Some(&graphene::Rect::new(0.0, 0.0, w as f32, h as f32)));
    if texture.save_to_png(path).is_err() {
        return false;
    }
    println!("saved {path}");
    true
}

fn walk(w: gtk::Widget, out: &mut Vec<gtk::Widget>) {
    out.push(w.clone());
    let mut child = w.first_child();
    while let Some(c) = child {
        walk(c.clone(), out);
        child = c.next_sibling();
    }
}

fn open_whats_new(w: &Rc<Win>) {
    let mut all = vec![];
    walk(w.window.clone().upcast(), &mut all);
    for row in all.into_iter().filter_map(|x| x.downcast::<adw::ActionRow>().ok()) {
        if row.title().contains("New") {
            if let Some(name) = row.action_name() {
                let _ = row.activate_action(&name, row.action_target_value().as_ref());
            }
        }
    }
}

enum Step {
    /// Wait, then do something.
    Do(u64, fn(&Rc<Win>)),
    /// Wait, then save the window as `name.png` (retrying while it isn't drawn yet).
    Snap(u64, &'static str),
}

fn run_from(win: Rc<Win>, dir: Rc<String>, steps: Rc<Vec<Step>>, i: usize, tries: u32) {
    let Some(step) = steps.get(i) else { return };
    let delay = match step {
        Step::Do(ms, _) | Step::Snap(ms, _) => if tries == 0 { *ms } else { 200 },
    };
    glib::timeout_add_local_once(Duration::from_millis(delay), move || match &steps[i] {
        Step::Do(_, f) => {
            f(&win);
            run_from(win, dir, steps, i + 1, 0);
        }
        Step::Snap(_, name) => {
            if try_snap(win.window.upcast_ref(), &format!("{dir}/{name}.png")) || tries >= 25 {
                run_from(win, dir, steps, i + 1, 0);
            } else {
                run_from(win, dir, steps, i, tries + 1);
            }
        }
    });
}

pub fn run(win: Rc<Win>, dir: String) {
    win.window.set_default_size(1060, 1180);
    let steps = vec![
        Step::Snap(1500, "lighting"),
        Step::Do(100, |w| w.stack.set_visible_child_name("per-key")),
        Step::Snap(500, "per-key"),
        Step::Do(100, |w| w.stack.set_visible_child_name("music")),
        Step::Snap(500, "music"),
        Step::Do(100, |w| {
            w.stack.set_visible_child_name("lighting");
            w.show_about();
        }),
        Step::Snap(1200, "about"),
        Step::Do(100, open_whats_new),
        Step::Snap(1000, "whats-new"),
        Step::Do(100, |w| w.quit_app()),
    ];
    run_from(win, Rc::new(dir), Rc::new(steps), 0, 0);
}
