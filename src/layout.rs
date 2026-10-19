//! Physical 87-key ANSI layout and the hardware LED index of every key.
//!
//! LED indices come from the OEM driver's KB.ini; the board is wired in columns
//! of six LEDs (Esc=0, `=1, Tab=2, Caps=3, LShift=4, LCtrl=5, F1=12 ...).
//! Positions are in key units (1u = one alphanumeric key). Same table as
//! `aula_f87/layout.py`.

pub const WIDTH_U: f64 = 18.25;
pub const HEIGHT_U: f64 = 6.25;
pub const PLANE: usize = 126;

pub struct Key {
    pub label: &'static str,
    pub led: usize,
    pub x: f64,
    pub y: f64,
    pub w: f64,
}

enum Item {
    K(&'static str, usize),
    W(&'static str, usize, f64),
    Gap(f64),
}
use Item::{Gap, K, W};

fn rows() -> Vec<(f64, Vec<Item>)> {
    vec![
        (0.0, vec![K("Esc", 0), Gap(1.0),
                   K("F1", 12), K("F2", 18), K("F3", 24), K("F4", 30), Gap(0.5),
                   K("F5", 36), K("F6", 42), K("F7", 48), K("F8", 54), Gap(0.5),
                   K("F9", 60), K("F10", 66), K("F11", 72), K("F12", 78), Gap(0.25),
                   K("PrtSc", 84), K("ScrLk", 90), K("Pause", 96)]),
        (1.25, vec![K("`", 1), K("1", 7), K("2", 13), K("3", 19), K("4", 25), K("5", 31),
                    K("6", 37), K("7", 43), K("8", 49), K("9", 55), K("0", 61), K("-", 67),
                    K("=", 73), W("Backspace", 79, 2.0), Gap(0.25),
                    K("Ins", 85), K("Home", 91), K("PgUp", 97)]),
        (2.25, vec![W("Tab", 2, 1.5), K("Q", 8), K("W", 14), K("E", 20), K("R", 26),
                    K("T", 32), K("Y", 38), K("U", 44), K("I", 50), K("O", 56), K("P", 62),
                    K("[", 68), K("]", 74), W("\\", 80, 1.5), Gap(0.25),
                    K("Del", 86), K("End", 92), K("PgDn", 98)]),
        (3.25, vec![W("Caps", 3, 1.75), K("A", 9), K("S", 15), K("D", 21), K("F", 27),
                    K("G", 33), K("H", 39), K("J", 45), K("K", 51), K("L", 57), K(";", 63),
                    K("'", 69), W("Enter", 81, 2.25)]),
        (4.25, vec![W("Shift", 4, 2.25), K("Z", 10), K("X", 16), K("C", 22), K("V", 28),
                    K("B", 34), K("N", 40), K("M", 46), K(",", 52), K(".", 58), K("/", 64),
                    W("Shift", 82, 2.75), Gap(1.25), K("↑", 94)]),
        (5.25, vec![W("Ctrl", 5, 1.25), W("Win", 11, 1.25), W("Alt", 17, 1.25),
                    W("", 35, 6.25), W("Alt", 53, 1.25), W("Fn", 59, 1.25),
                    W("Menu", 65, 1.25), W("Ctrl", 83, 1.25), Gap(0.25),
                    K("←", 89), K("↓", 95), K("→", 101)]),
    ]
}

pub fn keys() -> Vec<Key> {
    let mut keys = Vec::new();
    for (y, items) in rows() {
        let mut x = 0.0;
        for item in items {
            let (label, led, w) = match item {
                Gap(g) => {
                    x += g;
                    continue;
                }
                K(label, led) => (label, led, 1.0),
                W(label, led, w) => (label, led, w),
            };
            keys.push(Key { label, led, x, y, w });
            x += w;
        }
    }
    keys
}

/// Distinct row positions, top row first.
pub fn row_positions(keys: &[Key]) -> Vec<f64> {
    let mut ys: Vec<f64> = keys.iter().map(|k| k.y).collect();
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
    ys.dedup();
    ys
}

/// Horizontal centre of each LED's key, in key units.
pub fn key_x(led: usize) -> f64 {
    keys().iter().find(|k| k.led == led).map(|k| k.x + k.w / 2.0).unwrap_or(0.0)
}
