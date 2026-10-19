//! App preferences that live on the computer, not in the keyboard: music sync
//! (on/off, style, sensitivity, brightness, follow), the last tab, the brush
//! colour and the window size.
//!
//! Same file and format as the Python app: ~/.config/aula-f87-control/settings.json.

use std::fs;
use std::path::PathBuf;

use serde_json::{json, Value};

use crate::device::config_dir;

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub music_on: bool,
    pub music_style: u32,
    pub music_sensitivity: f64,
    pub music_brightness: f64,
    pub music_follow: Vec<String>, // any of "bass", "vocals", "beat"; empty = everything
    pub tab: String,
    pub brush: [u8; 3],
    pub window: [i32; 2],
    #[doc(hidden)]
    pub path: PathBuf,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            music_on: false,
            music_style: 0,
            music_sensitivity: 1.0,
            music_brightness: 1.0,
            music_follow: vec![],
            tab: "lighting".into(),
            brush: [255, 0, 0],
            window: [1060, 820],
            path: config_dir().join("settings.json"),
        }
    }
}

fn num(v: Option<&Value>) -> Option<f64> {
    v.and_then(|v| v.as_f64().or_else(|| v.as_bool().map(|b| b as u8 as f64)))
}

impl Settings {
    pub fn load() -> Settings {
        let mut s = Settings::default();
        let stored = fs::read_to_string(&s.path)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .unwrap_or(Value::Null);
        let get = |k: &str| stored.get(k);
        if let Some(v) = get("music_on") {
            s.music_on = v.as_bool().unwrap_or_else(|| v.as_f64().is_some_and(|n| n != 0.0));
        }
        if let Some(n) = num(get("music_style")) {
            s.music_style = (n as i64).clamp(0, 3) as u32;
        }
        if let Some(n) = num(get("music_sensitivity")) {
            s.music_sensitivity = n.clamp(0.5, 2.0);
        }
        if let Some(n) = num(get("music_brightness")) {
            s.music_brightness = n.clamp(0.1, 1.0);
        }
        if let Some(list) = get("music_follow").and_then(|v| v.as_array()) {
            let picked: Vec<&str> = list.iter().filter_map(|v| v.as_str()).collect();
            s.music_follow = ["bass", "vocals", "beat"]
                .iter()
                .filter(|k| picked.contains(k))
                .map(|k| k.to_string())
                .collect();
        }
        if let Some(t) = get("tab").and_then(|v| v.as_str()) {
            if ["lighting", "per-key", "music"].contains(&t) {
                s.tab = t.into();
            }
        }
        if let Some(b) = get("brush").and_then(|v| v.as_array()) {
            let c: Vec<u8> = b.iter().filter_map(|v| v.as_f64()).map(|v| v.clamp(0.0, 255.0) as u8).collect();
            if c.len() >= 3 {
                s.brush = [c[0], c[1], c[2]];
            }
        }
        if let Some(w) = get("window").and_then(|v| v.as_array()) {
            if let (Some(a), Some(b)) = (num(w.first()), num(w.get(1))) {
                s.window = [(a as i32).max(640), (b as i32).max(480)];
            }
        }
        s
    }

    /// Write the file (atomically: a crash never leaves a half-written file).
    pub fn save(&self) {
        let value = json!({
            "music_on": self.music_on,
            "music_style": self.music_style,
            "music_sensitivity": self.music_sensitivity,
            "music_brightness": self.music_brightness,
            "music_follow": self.music_follow,
            "tab": self.tab,
            "brush": self.brush,
            "window": self.window,
        });
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let tmp = self.path.with_extension("tmp");
        if fs::write(&tmp, serde_json::to_string_pretty(&value).unwrap_or_default()).is_ok() {
            let _ = fs::rename(&tmp, &self.path);
        }
    }
}
