//! What the keyboard looks like right now, for the Lighting preview.
//!
//! The keyboard can't report the colours its LEDs show, so this imitates each
//! built-in effect from its settings (effect, colour, brightness, speed), and
//! the light bar above the arrow keys and the side lights from their modes.
//! Effects that react to key presses show an example press now and then.

use std::f64::consts::TAU;
use std::sync::OnceLock;

use super::view::Strips;
use crate::device::{standard_color, Frame, Lighting};
use crate::effects::{effect_info, PER_KEY};
use crate::layout::{keys, HEIGHT_U, PLANE, WIDTH_U};

/// Centre of every LED's key, in key units, and the LEDs that have a key.
fn positions() -> &'static ([(f64, f64); PLANE], Vec<usize>) {
    static POS: OnceLock<([(f64, f64); PLANE], Vec<usize>)> = OnceLock::new();
    POS.get_or_init(|| {
        let mut pos = [(0.0, 0.0); PLANE];
        let mut leds = vec![];
        for k in keys() {
            pos[k.led] = (k.x + k.w / 2.0, k.y + 0.5);
            leds.push(k.led);
        }
        leds.sort();
        (pos, leds)
    })
}

/// A fixed pseudo-random number in [0, 1) for `n`.
fn noise(n: u64) -> f64 {
    let mut x = n.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
    x ^= x >> 29;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 32;
    (x % 1_000_000) as f64 / 1_000_000.0
}

fn frac(x: f64) -> f64 {
    x - x.floor()
}

/// `colorsys.hsv_to_rgb(h, 1, 1)` scaled to 0-255.
pub fn hue(h: f64) -> [f64; 3] {
    let h = frac(h);
    let i = (h * 6.0) as i64;
    let f = h * 6.0 - i as f64;
    let (v, p, q, t) = (1.0, 0.0, 1.0 - f, f);
    let c = match i.rem_euclid(6) {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    };
    c.map(|x| x * 255.0)
}

fn scale(c: [f64; 3], k: f64) -> [u8; 3] {
    c.map(|v| (v * k).clamp(0.0, 255.0) as u8)
}

/// Slow rise and fall, like the keyboard's breathing.
fn breath(t: f64, rate: f64) -> f64 {
    0.08 + 0.92 * (0.5 - 0.5 * (TAU * t * rate).cos())
}

/// Effects that keep moving (everything except Off, Static and Custom per-key).
pub fn animated(state: &Lighting) -> bool {
    let effect_moves = !matches!(state.effect, 0 | 1 | PER_KEY) && state.brightness > 0;
    effect_moves || matches!(state.side, 1 | 2 | 4) || matches!(state.bar, 1 | 2 | 4)
}

/// One key of the preview at time `t` (seconds).
pub fn key_color(state: &Lighting, per_key: &Frame, led: usize, t: f64) -> [u8; 3] {
    if state.effect == 0 {
        return [0, 0, 0];
    }
    if state.effect == PER_KEY {
        return per_key[led];
    }
    if (1..=18).contains(&state.effect) && state.brightness == 0 {
        return [0, 0, 0];
    }
    let (pos, leds) = positions();
    let (x, y) = pos[led];
    let (cx, cy) = (x / WIDTH_U, y / HEIGHT_U);
    let info = effect_info(state.effect);
    let rainbow = state.colorful || !info.color;
    let base = |h: f64| if rainbow { hue(h) } else { state.color.map(|c| c as f64) };
    let rate = 0.1 + 0.1 * state.speed as f64; // cycles per second at speed 0-4
    let level = 0.4 + 0.6 * state.brightness.min(4) as f64 / 4.0;

    // An example key press every `every` seconds, for effects that react to typing.
    let press = |every: f64| {
        let n = (t / every).floor() as u64;
        let key = leds[(noise(n) * leds.len() as f64) as usize % leds.len()];
        (key, t / every - n as f64) // pressed key, 0..1 since the press
    };

    let (color, k) = match state.effect {
        1 => (base(cx), 1.0),
        2 => (base(cx), breath(t, rate * 0.6)),
        3 => (hue(cx - t * rate), 1.0),
        4 | 12 => {
            // Flash away / Reactive: the pressed key lights up and fades.
            let (key, since) = press(0.9);
            (base(cx), if key == led { (-since * 4.0).exp() } else { 0.0 })
        }
        5 => {
            // Raindrops: keys light up one by one and fade.
            let phase = frac(t * (0.2 + 0.08 * state.speed as f64) + noise(led as u64));
            (base(cx + noise(led as u64 + 99)), if phase < 0.15 { 1.0 - phase / 0.15 } else { 0.0 })
        }
        7 => {
            // Ripples: a ring spreads from the pressed key.
            let (key, since) = press(1.6);
            let (kx, ky) = pos[key];
            let dist = ((x - kx).powi(2) + (y - ky).powi(2)).sqrt();
            let radius = since * 9.0;
            (base(cx), (-(dist - radius).powi(2) * 1.5).exp() * (1.0 - since))
        }
        8 => {
            // Twinkling stars.
            let phase = frac(t * (0.15 + 0.06 * state.speed as f64) + noise(led as u64) * 3.7);
            (base(noise(led as u64 + 7)), if phase < 0.25 { (std::f64::consts::PI * phase / 0.25).sin() } else { 0.0 })
        }
        10 => {
            // Retro snake: runs through the keys column by column.
            let head = (t * (6.0 + 6.0 * state.speed as f64)) % PLANE as f64;
            let behind = (head - led as f64).rem_euclid(PLANE as f64);
            (base(cx), if behind < 12.0 { 1.0 - behind / 12.0 } else { 0.0 })
        }
        11 => {
            // Neon stream: colour flowing across the board.
            if rainbow {
                (hue(cx + t * rate), 1.0)
            } else {
                (base(cx), 0.25 + 0.75 * (0.5 + 0.5 * (TAU * (cx * 1.5 - t * rate)).sin()))
            }
        }
        13 => (base(cx), 0.5 + 0.5 * (TAU * (cx * 2.0 - t * rate)).sin()),
        15 => {
            let angle = (y - HEIGHT_U / 2.0).atan2(x - WIDTH_U / 2.0) / TAU;
            (hue(angle + t * rate), 1.0)
        }
        16 => (hue(cy - t * rate), 1.0),
        17 => {
            let dist = ((x - WIDTH_U / 2.0).powi(2) + (y - HEIGHT_U / 2.0).powi(2)).sqrt() / WIDTH_U;
            (hue(dist * 1.5 - t * rate), 1.0)
        }
        _ => (base(cx), 1.0),
    };
    scale(color, k * level)
}

/// A light strip with `n` segments in one of the side-light modes
/// (0 off, 1 rainbow wave, 2 rainbow all together, 3 solid, 4 breathing).
fn strip(mode: u8, color: u8, brightness: u8, n: usize, t: f64) -> Vec<[u8; 3]> {
    let level = 0.4 + 0.6 * brightness.min(4) as f64 / 4.0;
    let solid = standard_color(color).map(|c| c as f64);
    match mode {
        1 => (0..n).map(|i| scale(hue(i as f64 / n as f64 - t * 0.3), level)).collect(),
        2 => vec![scale(hue(t * 0.15), level); n],
        3 => vec![scale(solid, level); n],
        4 => vec![scale(solid, level * breath(t, 0.25)); n],
        _ => vec![],
    }
}

pub fn strips(state: &Lighting, t: f64) -> Strips {
    Strips {
        bar: strip(state.bar, state.bar_color, state.bar_brightness, 16, t),
        sides: strip(state.side, state.side_color, 4, 24, t),
    }
}
