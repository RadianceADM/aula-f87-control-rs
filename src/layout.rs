//! Physical 87-key ANSI layout and the hardware LED index of every key.
//! Same table as `aula_f87/layout.py`; positions are in key units.

pub const WIDTH_U: f64 = 18.25;
pub const PLANE: usize = 126;

pub struct Key {
    pub led: usize,
    pub x: f64,
    pub y: f64,
    pub w: f64,
}

enum Item {
    K(usize),
    W(usize, f64),
    Gap(f64),
}
use Item::{Gap, K, W};

fn rows() -> Vec<(f64, Vec<Item>)> {
    vec![
        (0.0, vec![K(0), Gap(1.0), K(12), K(18), K(24), K(30), Gap(0.5), K(36), K(42), K(48),
                   K(54), Gap(0.5), K(60), K(66), K(72), K(78), Gap(0.25), K(84), K(90), K(96)]),
        (1.25, vec![K(1), K(7), K(13), K(19), K(25), K(31), K(37), K(43), K(49), K(55), K(61),
                    K(67), K(73), W(79, 2.0), Gap(0.25), K(85), K(91), K(97)]),
        (2.25, vec![W(2, 1.5), K(8), K(14), K(20), K(26), K(32), K(38), K(44), K(50), K(56),
                    K(62), K(68), K(74), W(80, 1.5), Gap(0.25), K(86), K(92), K(98)]),
        (3.25, vec![W(3, 1.75), K(9), K(15), K(21), K(27), K(33), K(39), K(45), K(51), K(57),
                    K(63), K(69), W(81, 2.25)]),
        (4.25, vec![W(4, 2.25), K(10), K(16), K(22), K(28), K(34), K(40), K(46), K(52), K(58),
                    K(64), W(82, 2.75), Gap(1.25), K(94)]),
        (5.25, vec![W(5, 1.25), W(11, 1.25), W(17, 1.25), W(35, 6.25), W(53, 1.25), W(59, 1.25),
                    W(65, 1.25), W(83, 1.25), Gap(0.25), K(89), K(95), K(101)]),
    ]
}

pub fn keys() -> Vec<Key> {
    let mut keys = Vec::new();
    for (y, items) in rows() {
        let mut x = 0.0;
        for item in items {
            let (led, w) = match item {
                Gap(g) => {
                    x += g;
                    continue;
                }
                K(led) => (led, 1.0),
                W(led, w) => (led, w),
            };
            keys.push(Key { led, x, y, w });
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
