//! The on-screen keyboard: the same drawing as the Linux app's view.rs (dark
//! case, rounded keys in their colours, labels that stay readable, the light
//! bar above the arrows and the side lights), built from WinUI elements on a
//! Canvas in key units. A Viewbox scales it to the page.

use aula_f87::layout::{HEIGHT_U, WIDTH_U, keys};
use aula_f87::preview::Strips;
use windows_reactor::*;

/// Pixels per key unit before the Viewbox scales it.
pub const UNIT: f64 = 44.0;
/// Free space around the case, in key units.
const PAD: f64 = 0.5;
const CASE: Color = Color::rgb(26, 26, 28);
const UNLIT: Color = Color::rgb(54, 54, 59);

pub fn canvas_size() -> (f64, f64) {
    ((WIDTH_U + 2.0 * PAD) * UNIT, (HEIGHT_U + 2.0 * PAD) * UNIT)
}

fn color(c: [u8; 3]) -> Color {
    Color::rgb(c[0], c[1], c[2])
}

/// A light strip: unlit, or split into segments of the given colours.
fn strip(x: f64, y: f64, w: f64, h: f64, colors: &[[u8; 3]], vertical: bool) -> View {
    let radius = w.min(h) / 2.0;
    let outer = Border::new().width(w).height(h).canvas_left(x).canvas_top(y).corner_radius(radius);
    if colors.is_empty() {
        return outer.background(UNLIT).into();
    }
    let n = colors.len() as f64;
    let segments: Vec<View> = colors
        .iter()
        .map(|c| {
            let seg = Border::new().background(color(*c));
            if vertical { seg.width(w).height(h / n).into() } else { seg.width(w / n).height(h).into() }
        })
        .collect();
    let panel = StackPanel::new()
        .orientation(if vertical { Orientation::Vertical } else { Orientation::Horizontal })
        .children(segments);
    outer.content(panel).into()
}

/// The keyboard with each LED's colour from `color_for`.
pub fn board(color_for: &dyn Fn(usize) -> [u8; 3], strips: Option<&Strips>) -> Canvas {
    let u = UNIT;
    let (ox, oy) = (PAD * u, PAD * u);
    let (width, height) = canvas_size();
    let mut children: Vec<View> = Vec::with_capacity(100);
    children.push(
        Border::new()
            .width((WIDTH_U + 0.6) * u)
            .height((HEIGHT_U + 0.6) * u)
            .canvas_left(ox - 0.3 * u)
            .canvas_top(oy - 0.3 * u)
            .corner_radius(0.35 * u)
            .background(CASE)
            .into(),
    );
    let gap = 0.08 * u;
    for key in keys() {
        let [r, g, b] = color_for(key.led);
        let lit = r as u32 + g as u32 + b as u32 > 0;
        let bright = lit && 0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64 > 140.0;
        let text = if bright { Color::argb(217, 25, 25, 25) } else { Color::argb(217, 242, 242, 242) };
        children.push(
            Border::new()
                .width(key.w * u - gap)
                .height(u - gap)
                .canvas_left(ox + key.x * u + gap / 2.0)
                .canvas_top(oy + key.y * u + gap / 2.0)
                .corner_radius(0.12 * u)
                .background(if lit { Color::rgb(r, g, b) } else { UNLIT })
                .content(
                    TextBlock::new()
                        .text(key.label)
                        .font_size((0.22 * u).max(6.0))
                        .foreground(text)
                        .horizontal_alignment(HorizontalAlignment::Center)
                        .vertical_alignment(VerticalAlignment::Center),
                )
                .into(),
        );
    }
    if let Some(s) = strips {
        // The light bar sits in the free space above the arrow keys.
        children.push(strip(ox + 15.45 * u, oy + 3.62 * u, 2.6 * u, 0.26 * u, &s.bar, false));
        // Side lights along the left and right edges of the case.
        let (top, len, w) = (oy + 0.4 * u, (HEIGHT_U - 0.8) * u, 0.1 * u);
        children.push(strip(ox - 0.22 * u, top, w, len, &s.sides, true));
        children.push(strip(ox + WIDTH_U * u + 0.12 * u, top, w, len, &s.sides, true));
    }
    Canvas::new().width(width).height(height).children(children)
}

/// The LED under a point in canvas coordinates, if any.
pub fn hit(x: f64, y: f64) -> Option<usize> {
    let u = UNIT;
    let (ox, oy) = (PAD * u, PAD * u);
    keys().into_iter().find_map(|k| {
        let (kx, ky) = (ox + k.x * u, oy + k.y * u);
        (x >= kx && x < kx + k.w * u && y >= ky && y < ky + u).then_some(k.led)
    })
}
