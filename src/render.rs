//! Analyzer state -> 126 LED colours. Port of `render()` in `aula_f87/music.py`.

use crate::analyzer::{Analyzer, BANDS};
pub use crate::device::Frame;
use crate::layout::{self, PLANE, WIDTH_U};

pub const STYLE_NAMES: [&str; 4] = ["bars", "meter", "rainbow", "pulse"];
const METER: usize = 1;
const RAINBOW: usize = 2;
const PULSE: usize = 3;

const RED: [f64; 3] = [255.0, 0.0, 0.0];
const YELLOW: [f64; 3] = [255.0, 170.0, 0.0];
const GREEN: [f64; 3] = [0.0, 255.0, 0.0];
const METER_COLORS: [[f64; 3]; 6] = [GREEN, GREEN, YELLOW, YELLOW, RED, RED]; // by height, bottom first

/// (led, band, height above the bottom row) for every key.
pub struct KeyBands {
    keys: Vec<(usize, usize, usize)>,
    rows: usize,
}

impl Default for KeyBands {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyBands {
    pub fn new() -> Self {
        let keys = layout::keys();
        let rows = layout::row_positions(&keys);
        let list = keys
            .iter()
            .map(|k| {
                let band = (((k.x + k.w / 2.0) / WIDTH_U * BANDS as f64) as usize).min(BANDS - 1);
                let row = rows.iter().position(|&y| y == k.y).unwrap();
                (k.led, band, rows.len() - 1 - row)
            })
            .collect();
        KeyBands { keys: list, rows: rows.len() }
    }
}

/// `colorsys.hsv_to_rgb`
fn hsv_to_rgb(h: f64, s: f64, v: f64) -> [f64; 3] {
    if s == 0.0 {
        return [v, v, v];
    }
    let i = (h * 6.0) as i64; // truncates, like int()
    let f = h * 6.0 - i as f64;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match i.rem_euclid(6) {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

fn hue_rgb(hue: f64) -> [f64; 3] {
    hsv_to_rgb(hue, 1.0, 1.0).map(|c| c * 255.0)
}

fn scale(rgb: [f64; 3], k: f64) -> [u8; 3] {
    rgb.map(|c| (c * k).clamp(0.0, 255.0) as u8) // int() truncation
}

fn band_color(band: usize) -> [f64; 3] {
    let third = BANDS / 3;
    if band < third {
        RED
    } else if band < 2 * third {
        YELLOW
    } else {
        GREEN
    }
}

/// few_colors: the 2.4G stream sends one group per distinct colour, so keep
/// the palette small there (no partial dimming, 6 rainbow hues).
pub fn render(a: &Analyzer, kb: &KeyBands, style: usize, brightness: f64, few_colors: bool, clock: f64) -> Frame {
    let mut colors = [[0u8; 3]; PLANE];
    if style == PULSE {
        let mut hue = (clock * 0.03) % 1.0;
        if few_colors {
            hue = (hue * 12.0).round_ties_even() / 12.0; // Python's round()
        }
        let rgb = hue_rgb(hue);
        let mut level = 0.08 + 0.92 * a.pulse; // faint glow between beats
        if few_colors {
            level = (level * 8.0).round_ties_even() / 8.0;
        }
        for &(led, _, _) in &kb.keys {
            colors[led] = scale(rgb, level * brightness);
        }
        return colors;
    }
    for &(led, band, height) in &kb.keys {
        let lit = a.levels[band] * kb.rows as f64 - height as f64;
        if lit < 0.15 {
            continue; // a barely-lit top key just looks like a dead key
        }
        let rgb = if style == RAINBOW {
            let mut hue = band as f64 / BANDS as f64 * 0.8;
            if few_colors {
                hue = (hue * 6.0).round_ties_even() / 6.0;
            }
            hue_rgb(hue)
        } else if style == METER {
            METER_COLORS[height]
        } else {
            band_color(band)
        };
        let k = if few_colors { brightness } else { brightness * lit.min(1.0) };
        colors[led] = scale(rgb, k);
    }
    colors
}
