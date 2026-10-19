//! Built-in lighting effects, side-light modes, colour presets and other UI tables.

use crate::device::Battery;

#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    pub id: u8,
    pub name: String,
    pub speed: bool,
    pub color: bool,
}

/// Effect 18 plus the config "custom" flag shows the stored per-key map
/// (confirmed on an F87 Pro; effect 21, used by newer F87 boards, goes dark).
pub const PER_KEY: u8 = 18;

/// Firmware effect numbers. 6, 9 and 14 exist in the OEM table but misbehave
/// on this board, so they are left out.
const TABLE: [(u8, &str, bool, bool); 16] = [
    (0, "Off", false, false),
    (1, "Static", false, true),
    (2, "Breathing", true, true),
    (3, "Rainbow wave", true, false),
    (4, "Flash away", true, true),
    (5, "Raindrops", true, true),
    (7, "Ripples", true, true),
    (8, "Twinkling stars", true, true),
    (10, "Retro snake", true, true),
    (11, "Neon stream", true, true),
    (12, "Reactive (keypress)", true, true),
    (13, "Sine wave", true, true),
    (15, "Rotating windmill", true, false),
    (16, "Colorful waterfall", true, false),
    (17, "Blossoming", true, false),
    (PER_KEY, "Custom per-key", false, false),
];

pub fn effects() -> Vec<Effect> {
    TABLE.iter().map(|&(id, name, speed, color)| Effect { id, name: name.into(), speed, color }).collect()
}

pub fn effect_info(id: u8) -> Effect {
    effects().into_iter().find(|e| e.id == id).unwrap_or(Effect {
        id,
        name: format!("Effect {id}"),
        speed: true,
        color: true,
    })
}

/// Lights on the sides of the case (config byte 18; Fn + Right Shift on the keyboard).
pub const SIDE_MODES: [&str; 5] = ["Off", "Rainbow wave", "Rainbow, all together", "Solid color", "Breathing"];

pub const PRESETS: [(&str, [u8; 3]); 9] = [
    ("Red", [255, 0, 0]),
    ("Orange", [255, 96, 0]),
    ("Yellow", [255, 208, 0]),
    ("Green", [0, 255, 0]),
    ("Cyan", [0, 255, 255]),
    ("Blue", [0, 64, 255]),
    ("Purple", [140, 0, 255]),
    ("Pink", [255, 0, 160]),
    ("White", [255, 255, 255]),
];

/// Music sync styles; the index is used by the GUI and the CLI.
pub const MUSIC_STYLES: [&str; 4] = ["EQ bars", "EQ bars (meter colors)", "EQ bars (rainbow)", "Beat pulse"];

/// What music sync can follow (key, label). Nothing selected = everything.
pub const MUSIC_FOLLOW: [(&str, &str); 3] = [("bass", "Bass"), ("vocals", "Vocals"), ("beat", "Beat")];

/// "Turn the backlight off after" choices, in minutes (the keyboard stores 30 s steps).
pub const SLEEP_MINUTES: [f64; 7] = [1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0];

pub fn sleep_label(minutes: f64) -> String {
    format!("{} minute{}", fmt_g(minutes), if minutes == 1.0 { "" } else { "s" })
}

/// Python's `{:g}` for the small numbers used here.
pub fn fmt_g(v: f64) -> String {
    if v.fract() == 0.0 { format!("{}", v as i64) } else { format!("{v}") }
}

pub fn nearest_sleep_index(minutes: f64) -> usize {
    let mut best = 0;
    for i in 0..SLEEP_MINUTES.len() {
        if (SLEEP_MINUTES[i] - minutes).abs() < (SLEEP_MINUTES[best] - minutes).abs() {
            best = i;
        }
    }
    best
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
        None => String::new(),
    }
}

/// Text for the battery row. `last`: (percent, unix time) of the last 2.4G reading.
pub fn battery_text(battery: Option<&Battery>, wired: bool, last: Option<(i64, f64)>) -> String {
    if let Some(b) = battery {
        return capitalize(&b.to_string());
    }
    if wired {
        let mut text = "Not reported over the cable (it charges from USB)".to_string();
        if let Some((percent, when)) = last {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            let fmt = if now - when < 86400.0 { "%H:%M" } else { "%b %d %H:%M" };
            text += &format!(" · last 2.4G reading: {percent}% at {}", strftime(fmt, when));
        }
        return text;
    }
    "Not reported".to_string()
}

/// Local-time strftime via libc.
pub fn strftime(fmt: &str, unix: f64) -> String {
    let t = unix as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    let mut buf = [0u8; 64];
    let cfmt = std::ffi::CString::new(fmt).unwrap();
    let n = unsafe { libc::strftime(buf.as_mut_ptr() as *mut libc::c_char, buf.len(), cfmt.as_ptr(), &tm) };
    String::from_utf8_lossy(&buf[..n]).into_owned()
}
