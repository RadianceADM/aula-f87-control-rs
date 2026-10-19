//! The keyboard's stored settings are never damaged by the app.
//! Same checks as the Python app's tests/test_safety.py, with a fake link.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use aula_f87::device::{
    config_ok, default_palette, palette_ok, Battery, Cache, DeviceLock, Error, Frame, Keyboard, Lighting, Link,
    Result, DEFAULT_CONFIG, KEYS_LEN,
};

fn damaged_config() -> Vec<u8> {
    default_palette()[21..149].to_vec() // palette bytes in the config
}
fn shifted_palette() -> Vec<u8> {
    let mut p = default_palette()[21..].to_vec();
    p.extend([0u8; 21]);
    p
}

#[derive(Default)]
struct State {
    cfg: Vec<u8>,
    pal: Vec<u8>,
    writes: Vec<&'static str>,
    stale_reads: u32,
    damage_next_write: bool,
}

#[derive(Clone)]
struct FakeLink(Arc<Mutex<State>>);

impl FakeLink {
    fn new(cfg: &[u8], pal: &[u8]) -> FakeLink {
        FakeLink(Arc::new(Mutex::new(State { cfg: cfg.to_vec(), pal: pal.to_vec(), ..Default::default() })))
    }
    fn store(&self, kind: &'static str, cfg: &[u8], pal: Option<&[u8]>) {
        let mut s = self.0.lock().unwrap();
        s.writes.push(kind);
        if s.damage_next_write {
            s.damage_next_write = false;
            s.cfg = damaged_config();
            s.pal = shifted_palette();
            return;
        }
        s.cfg = cfg.to_vec();
        if let Some(p) = pal {
            s.pal = p.to_vec();
        }
    }
    fn writes(&self) -> Vec<&'static str> {
        self.0.lock().unwrap().writes.clone()
    }
}

impl Link for FakeLink {
    fn label(&self) -> &'static str { "USB cable" }
    fn can_read_blocks(&self) -> bool { true }
    fn live_fps(&self) -> f64 { 30.0 }
    fn live_few_colors(&self) -> bool { false }
    fn probe(&mut self) -> Result<String> { Ok("F87 Pro".into()) }
    fn read_config(&mut self) -> Result<Vec<u8>> {
        let mut s = self.0.lock().unwrap();
        if s.stale_reads > 0 {
            s.stale_reads -= 1;
            return Ok(s.pal[..128].to_vec()); // a reply that isn't a config
        }
        Ok(s.cfg.clone())
    }
    fn read_palette(&mut self) -> Result<Vec<u8>> { Ok(self.0.lock().unwrap().pal.clone()) }
    fn read_keys(&mut self) -> Result<Vec<u8>> { Ok(vec![0; KEYS_LEN]) }
    fn write_lighting(&mut self, cfg: &[u8], pal: &[u8]) -> Result<()> { self.store("lighting", cfg, Some(pal)); Ok(()) }
    fn write_keys(&mut self, cfg: &[u8], _keys: &[u8]) -> Result<()> { self.store("keys", cfg, None); Ok(()) }
    fn write_config(&mut self, cfg: &[u8]) -> Result<()> { self.store("config", cfg, None); Ok(()) }
    fn read_battery(&mut self) -> Option<Battery> { None }
    fn send_live(&mut self, _c: &Frame) -> Result<()> { Ok(()) }
    fn alive(&self) -> bool { true }
}

fn keyboard(link: &FakeLink) -> Keyboard {
    Keyboard::new(Box::new(link.clone()), "F87 Pro".into(), Cache::load())
}

/// One test, so the environment (config and lock directories) is set once.
#[test]
fn settings_are_protected() {
    let dir = std::env::temp_dir().join(format!("aula-f87-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("XDG_CONFIG_HOME", &dir);
    std::env::set_var("XDG_RUNTIME_DIR", &dir);

    // validators
    assert!(config_ok(&DEFAULT_CONFIG));
    assert!(palette_ok(&default_palette()));
    assert!(!config_ok(&damaged_config()));
    assert!(!palette_ok(&shifted_palette()));

    // only what changed is written
    let link = FakeLink::new(&DEFAULT_CONFIG, &default_palette());
    let kb = keyboard(&link);
    let mut state = kb.read_lighting().unwrap();
    kb.apply_lighting(&state).unwrap();
    assert_eq!(link.writes(), Vec::<&str>::new());
    state.brightness = 2;
    kb.apply_lighting(&state).unwrap();
    assert_eq!(link.writes(), ["config"]);
    state.effect = 1;
    state.colorful = false;
    state.color = [10, 20, 30];
    kb.apply_lighting(&state).unwrap();
    assert_eq!(link.writes().last(), Some(&"lighting"));
    assert_eq!(link.0.lock().unwrap().pal[21..24], [10, 20, 30]);

    // stale replies are read again
    link.0.lock().unwrap().stale_reads = 2;
    assert_eq!(kb.read_lighting().unwrap().effect, 1);

    // a damaging write is undone
    let before = { let s = link.0.lock().unwrap(); (s.cfg.clone(), s.pal.clone()) };
    link.0.lock().unwrap().damage_next_write = true;
    match kb.apply_lighting(&Lighting { effect: 2, ..state.clone() }) {
        Err(Error::Device(m)) => assert!(m.contains("put back"), "{m}"),
        other => panic!("expected put back, got {other:?}"),
    }
    { let s = link.0.lock().unwrap(); assert_eq!((s.cfg.clone(), s.pal.clone()), before); }

    // damaged stored settings are reported, never written back, and repaired
    let good = link.0.lock().unwrap().cfg.clone();
    let bad = FakeLink::new(&damaged_config(), &shifted_palette());
    let kb2 = keyboard(&bad);
    assert!(matches!(kb2.read_lighting(), Err(Error::Damaged(_))));
    assert!(matches!(kb2.apply_lighting(&Lighting { effect: 3, ..Default::default() }), Err(Error::Damaged(_))));
    assert!(matches!(kb2.set_sleep(0.0), Err(Error::Damaged(_))));
    assert!(bad.writes().is_empty());
    assert_eq!(kb2.repair().unwrap(), "last good settings");
    assert_eq!(bad.0.lock().unwrap().cfg, good);
    assert!(palette_ok(&bad.0.lock().unwrap().pal));

    // repair without a backup uses the defaults
    std::fs::remove_file(dir.join("aula-f87-control/cache.json")).unwrap();
    let bad = FakeLink::new(&damaged_config(), &shifted_palette());
    assert_eq!(keyboard(&bad).repair().unwrap(), "default settings");
    assert_eq!(bad.0.lock().unwrap().cfg, DEFAULT_CONFIG.to_vec());

    // one program at a time
    let first = DeviceLock::try_acquire(Duration::from_millis(100)).expect("lock");
    assert!(DeviceLock::try_acquire(Duration::from_millis(100)).is_none());
    assert!(matches!(keyboard(&link).apply_lighting(&state), Err(Error::Device(m)) if m.contains("using the keyboard")));
    drop(first);
    assert!(DeviceLock::try_acquire(Duration::from_millis(100)).is_some());

    std::fs::remove_dir_all(&dir).ok();
}
