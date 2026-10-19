//! A keyboard that lives in memory, for screenshots and GUI tests.
//! Built with `--features fake-keyboard`; used when AULA_F87_FAKE is set
//! (`wired` for the cable, anything else for 2.4G).

use crate::device::{default_palette, Battery, Cache, Frame, Keyboard, Link, Result, DEFAULT_CONFIG, KEYS_LEN};
use crate::layout::PLANE;
use std::sync::Mutex;

/// Every write the fake keyboard received ("config", "lighting", "keys"), for tests.
pub static WRITES: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

pub struct FakeLink {
    wired: bool,
    cfg: Vec<u8>,
    pal: Vec<u8>,
    keys: Vec<u8>,
    pub frames: u64,
}

impl FakeLink {
    pub fn new(wired: bool) -> FakeLink {
        let mut cfg = DEFAULT_CONFIG.to_vec();
        cfg[10] = 2; // Breathing
        cfg[18] = 1;
        cfg[24] = 10;
        for e in 1..19 {
            cfg[56 + 2 * e] = 4;
            cfg[57 + 2 * e] = 0x30;
        }
        let mut pal = default_palette();
        pal[42..45].copy_from_slice(&[0, 140, 255]);
        let mut keys = vec![0u8; KEYS_LEN];
        for (led, [r, g, b]) in [
            (0, [255, 0, 0]), (14, [0, 255, 0]), (9, [0, 255, 0]), (15, [0, 255, 0]), (21, [0, 255, 0]),
            (89, [0, 120, 255]), (94, [0, 120, 255]), (95, [0, 120, 255]), (101, [0, 120, 255]),
        ] {
            keys[led] = r;
            keys[PLANE + led] = g;
            keys[2 * PLANE + led] = b;
        }
        if std::env::var("AULA_F87_FAKE").is_ok_and(|v| v == "damaged") {
            cfg = pal[21..149].to_vec(); // what the damage looked like on a real F87 Pro
        }
        FakeLink { wired, cfg, pal, keys, frames: 0 }
    }
}

impl Link for FakeLink {
    fn label(&self) -> &'static str {
        if self.wired { "USB cable" } else { "2.4G receiver" }
    }
    fn can_read_blocks(&self) -> bool {
        true
    }
    fn live_fps(&self) -> f64 {
        30.0
    }
    fn live_few_colors(&self) -> bool {
        false
    }
    fn probe(&mut self) -> Result<String> {
        Ok("F87 Pro".into())
    }
    fn read_config(&mut self) -> Result<Vec<u8>> {
        Ok(self.cfg.clone())
    }
    fn read_palette(&mut self) -> Result<Vec<u8>> {
        Ok(self.pal.clone())
    }
    fn read_keys(&mut self) -> Result<Vec<u8>> {
        Ok(self.keys.clone())
    }
    fn write_lighting(&mut self, cfg: &[u8], pal: &[u8]) -> Result<()> {
        WRITES.lock().unwrap().push("lighting");
        self.cfg = cfg.to_vec();
        self.pal = pal.to_vec();
        Ok(())
    }
    fn write_keys(&mut self, cfg: &[u8], keys: &[u8]) -> Result<()> {
        WRITES.lock().unwrap().push("keys");
        self.cfg = cfg.to_vec();
        self.keys = keys.to_vec();
        Ok(())
    }
    fn write_config(&mut self, cfg: &[u8]) -> Result<()> {
        WRITES.lock().unwrap().push("config");
        self.cfg = cfg.to_vec();
        Ok(())
    }
    fn read_battery(&mut self) -> Option<Battery> {
        (!self.wired).then_some(Battery { percent: Some(87), charging: false })
    }
    fn send_live(&mut self, _colors: &Frame) -> Result<()> {
        self.frames += 1;
        Ok(())
    }
    fn alive(&self) -> bool {
        true
    }
}

/// The stored settings survive reconnects, like a real keyboard's.
static STORED: Mutex<Option<(Vec<u8>, Vec<u8>, Vec<u8>)>> = Mutex::new(None);

pub fn keyboard() -> Keyboard {
    let wired = std::env::var("AULA_F87_FAKE").is_ok_and(|v| v == "wired");
    let mut link = FakeLink::new(wired);
    if let Some((cfg, pal, keys)) = STORED.lock().unwrap().clone() {
        (link.cfg, link.pal, link.keys) = (cfg, pal, keys);
    }
    Keyboard::new(Box::new(link), "F87 Pro".into(), Cache::load())
}

impl Drop for FakeLink {
    fn drop(&mut self) {
        *STORED.lock().unwrap() = Some((self.cfg.clone(), self.pal.clone(), self.keys.clone()));
    }
}
