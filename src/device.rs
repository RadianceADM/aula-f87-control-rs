//! Talk to the AULA F87 / F87 Pro over USB HID (no vendor software, no driver).
//!
//! Two transports carry the same three data blocks:
//!
//! * USB cable (258a:010c): 520-byte HID feature reports, report ID 0x06.
//!   Header `06 cmd 00 00 01 00 len_lo len_hi`; a read is `cmd | 0x80`.
//! * 2.4G receiver (3554:fa09): 20-byte output reports, report ID 0x13,
//!   `13 cmd sub seq len data[14] checksum`. The keyboard echoes every frame.
//!
//! Data blocks: config (128 bytes: effect, per-effect brightness/speed/colour
//! slot, side light, sleep), palette (512 bytes: 7 RGB slots per effect) and
//! keys (378 bytes: per-key colours as three 126-byte planes).
//!
//! A port of `aula_f87/device.py`, including its safeguards for the stored settings.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread::sleep;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use crate::effects::{effect_info, PER_KEY};
use crate::hidraw::{self, Hidraw};
use crate::layout::PLANE;

pub const WIRED_ID: (u32, u32) = (0x258A, 0x010C);
pub const DONGLE_ID: (u32, u32) = (0x3554, 0xFA09);

pub const CFG_LEN: usize = 128;
pub const PAL_LEN: usize = 512;
pub const KEYS_LEN: usize = 3 * PLANE;

pub const CFG_CUSTOM: usize = 9; // 1 = show the per-key map (with effect PER_KEY)
pub const CFG_EFFECT: usize = 10;
pub const CFG_COLOR_MODE: usize = 12; // 1 = palette / rainbow, 3 = factory colours
pub const CFG_SIDE: usize = 18;
pub const CFG_SLEEP: usize = 24; // idle time before sleep, in 30 s steps; 0 = never
pub const COLORFUL: u8 = 7; // colour-slot value meaning "rainbow"

pub type Frame = [[u8; 3]; PLANE];

const STD_COLORS: [u8; 21] = [
    0xff, 0, 0, 0, 0xff, 0, 0, 0, 0xff, 0xff, 0xff, 0, 0xff, 0, 0xff, 0, 0xff, 0xff, 0xff, 0xff, 0xff,
];

pub fn param_offset(effect: u8) -> usize {
    56 + 2 * effect as usize
}

pub fn slot_offset(effect: u8, slot: usize) -> usize {
    21 * effect as usize + 3 * slot
}

pub fn default_palette() -> Vec<u8> {
    let mut pal = vec![0u8; PAL_LEN];
    for effect in 1..14u8 {
        let o = slot_offset(effect, 0);
        pal[o..o + 21].copy_from_slice(&STD_COLORS);
    }
    pal[506] = 0x5a;
    pal[507] = 0xa5;
    pal
}

// ------------------------------------------------------------------------- errors

#[derive(Debug)]
pub enum Error {
    /// Shown to the user as-is.
    Device(String),
    /// The keyboard's stored settings don't look valid; Repair fixes this.
    Damaged(String),
    /// The transfer failed, e.g. the keyboard was unplugged.
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Device(s) | Error::Damaged(s) => f.write_str(s),
            Error::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

const DAMAGED: &str = "The keyboard's saved settings look damaged, which can stop it from typing. \
                       Repair puts back its last good settings.";

// ------------------------------------------------------------------------- validation

/// Bytes that are the same in every healthy config seen: the F87 Pro's, and the
/// factory template of the newer F87. A read without them is not a real config:
/// either a stale reply, or damaged stored settings. Writing such data back is
/// what can stop the keyboard from typing, so it is never done.
const CFG_FIXED: [(usize, u8); 10] = [
    (1, 0x03), (2, 0x03), (6, 0x04), (7, 0x04), (8, 0x07), (11, 0x20),
    (56, 0xFF), (57, 0xFF), (126, 0x5A), (127, 0xA5),
];

pub fn config_ok(cfg: &[u8]) -> bool {
    if cfg.len() < CFG_LEN || !CFG_FIXED.iter().all(|&(i, v)| cfg[i] == v) {
        return false;
    }
    (1..19u8).all(|effect| {
        // brightness 0-4, speed 0-4, colour slot 0-7
        let p = param_offset(effect);
        cfg[p] <= 4 && cfg[p + 1] >> 4 <= 4 && cfg[p + 1] & 0x0F <= 7
    })
}

/// Effect 0 (off) has no colours and the block ends with the 5A A5 marker.
pub fn palette_ok(pal: &[u8]) -> bool {
    pal.len() >= PAL_LEN && pal[..21].iter().all(|&b| b == 0) && pal[506..508] == [0x5a, 0xa5]
}

/// Healthy settings read from an F87 Pro (Colorful waterfall, side lights on,
/// backlight sleeps after 5 minutes). Repair writes this only when this
/// computer never saw good settings; otherwise it puts back the last good ones.
pub const DEFAULT_CONFIG: [u8; CFG_LEN] = hex128(concat!(
    "00030302000004040700102001000000",
    "00000103010200ff0a00000103010401",
    "00000000000000000000000000000000",
    "0000000000000000ffff043704370437",
    "04370437043701370437043704370430",
    "04310437043704070437043704370437",
    "04370747074707440744074407440744",
    "07440744040404040404040404045aa5",
));

const fn hex128(s: &str) -> [u8; CFG_LEN] {
    const fn nib(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => panic!("bad hex"),
        }
    }
    let b = s.as_bytes();
    assert!(b.len() == 2 * CFG_LEN);
    let mut out = [0u8; CFG_LEN];
    let mut i = 0;
    while i < CFG_LEN {
        out[i] = nib(b[2 * i]) << 4 | nib(b[2 * i + 1]);
        i += 1;
    }
    out
}

// ------------------------------------------------------------------------- lock

/// One program at a time: a lock file shared with the Python app, its music
/// sync and the command line, so two of them never talk to the keyboard at once.
pub struct DeviceLock {
    _file: File,
}

impl DeviceLock {
    pub fn path() -> PathBuf {
        #[cfg(feature = "fake-keyboard")]
        if std::env::var_os("AULA_F87_FAKE").is_some() {
            return config_dir().join("fake.lock"); // never wait for the real keyboard's users
        }
        let base = match std::env::var("XDG_RUNTIME_DIR") {
            Ok(d) if !d.is_empty() => PathBuf::from(d),
            _ => match std::env::var("XDG_CACHE_HOME") {
                Ok(d) => PathBuf::from(d),
                Err(_) => home().join(".cache"),
            },
        };
        base.join("aula-f87-control.lock")
    }

    pub fn try_acquire(timeout: Duration) -> Option<DeviceLock> {
        let path = Self::path();
        let _ = fs::create_dir_all(path.parent()?);
        let file = OpenOptions::new().create(true).append(true).read(true).open(&path).ok()?;
        let deadline = Instant::now() + timeout;
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                return Some(DeviceLock { _file: file }); // released when the file closes
            }
            if Instant::now() > deadline {
                return None;
            }
            sleep(Duration::from_millis(50));
        }
    }

    pub fn acquire() -> Result<DeviceLock> {
        Self::try_acquire(Duration::from_secs(3)).ok_or_else(|| {
            Error::Device(
                "Another program (or this app's music sync) is using the keyboard right now. \
                 Stop it and try again."
                    .into(),
            )
        })
    }
}

/// Waits used by the safety logic; skipped in unit tests.
fn pause(ms: u64) {
    if !cfg!(test) {
        sleep(Duration::from_millis(ms));
    }
}

pub fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
}

pub fn config_dir() -> PathBuf {
    match std::env::var("XDG_CONFIG_HOME") {
        Ok(d) if !d.is_empty() => PathBuf::from(d),
        _ => home().join(".config"),
    }
    .join("aula-f87-control")
}

// ------------------------------------------------------------------------- battery

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Battery {
    pub percent: Option<u8>, // 1-100, or None when unknown (e.g. while charging)
    pub charging: bool,
}

impl fmt::Display for Battery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.charging {
            f.write_str("charging")
        } else if let Some(p) = self.percent {
            write!(f, "{p}%")
        } else {
            f.write_str("unknown")
        }
    }
}

// ------------------------------------------------------------------------- transports

/// What the keyboard logic needs from a transport. Implemented by the cable,
/// the 2.4G receiver, and fakes in tests.
pub trait Link: Send {
    fn label(&self) -> &'static str;
    fn can_read_blocks(&self) -> bool;
    fn live_fps(&self) -> f64;
    fn live_few_colors(&self) -> bool;
    fn probe(&mut self) -> Result<String>;
    fn read_config(&mut self) -> Result<Vec<u8>>;
    fn read_palette(&mut self) -> Result<Vec<u8>>;
    fn read_keys(&mut self) -> Result<Vec<u8>>;
    fn write_lighting(&mut self, cfg: &[u8], pal: &[u8]) -> Result<()>;
    fn write_keys(&mut self, cfg: &[u8], keys: &[u8]) -> Result<()>;
    fn write_config(&mut self, cfg: &[u8]) -> Result<()>;
    fn read_battery(&mut self) -> Option<Battery>;
    fn send_live(&mut self, colors: &Frame) -> Result<()>;
    fn alive(&self) -> bool;
}

const TRANSIENT: [i32; 4] = [libc::EPROTO, libc::EPIPE, libc::ETIMEDOUT, libc::EAGAIN];

fn transient(e: &io::Error) -> bool {
    e.raw_os_error().is_some_and(|c| TRANSIENT.contains(&c))
}

fn model_name(byte: u8) -> &'static str {
    match byte {
        0x0B => "F87 Pro",
        0x66 => "F87 TK",
        _ => "F87",
    }
}

pub struct WiredLink {
    dev: Hidraw,
}

impl WiredLink {
    const REPORT_SIZE: usize = 520;
    /// Writes are spaced out: the keyboard stores each block in flash, and
    /// hurrying it (a write straight after a read, or right after another
    /// write) has made it restart in the middle of one.
    pub const WRITE_PAUSE: Duration = Duration::from_millis(200);
    pub const WRITE_SETTLE: Duration = Duration::from_millis(350);

    pub fn new(dev: Hidraw) -> Self {
        WiredLink { dev }
    }

    fn set(&self, data: &[u8]) -> io::Result<()> {
        let mut buf = vec![0u8; Self::REPORT_SIZE];
        buf[..data.len()].copy_from_slice(data);
        self.dev.set_feature(&buf)?;
        sleep(Duration::from_millis(10));
        Ok(())
    }

    fn header(cmd: u8, length: usize) -> Vec<u8> {
        vec![0x06, cmd, 0, 0, 1, 0, (length & 0xFF) as u8, (length >> 8) as u8]
    }

    /// The keyboard stalls requests (EPROTO/EPIPE) for a moment while it stores
    /// a previous write in flash; retrying after a short pause works.
    fn retry<T>(attempts: u32, mut transfer: impl FnMut() -> io::Result<T>) -> io::Result<T> {
        let mut attempt = 0;
        loop {
            match transfer() {
                Ok(v) => return Ok(v),
                Err(e) if transient(&e) && attempt + 1 < attempts => {
                    sleep(Duration::from_millis(50 * (attempt as u64 + 1)));
                    attempt += 1;
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn request(&self, data: &[u8], attempts: u32) -> io::Result<Vec<u8>> {
        Self::retry(attempts, || {
            self.set(data)?;
            self.dev.get_feature(0x06, Self::REPORT_SIZE)
        })
    }

    fn read(&self, cmd: u8, length: usize) -> Result<Vec<u8>> {
        let resp = self.request(&Self::header(cmd | 0x80, length), 6)?;
        if resp.len() < 8 + length || resp[1] != cmd | 0x80 || (resp[6] as usize | (resp[7] as usize) << 8) != length {
            return Err(Error::Device(
                "The keyboard sent an unexpected reply over USB. Is another program (or this app's \
                 music sync) using it right now?"
                    .into(),
            ));
        }
        Ok(resp[8..8 + length].to_vec())
    }

    fn write(&self, cmd: u8, body: &[u8]) -> Result<()> {
        sleep(Self::WRITE_PAUSE);
        let mut data = Self::header(cmd, body.len());
        data.extend_from_slice(body);
        Self::retry(6, || self.set(&data))?;
        sleep(Self::WRITE_SETTLE);
        Ok(())
    }
}

impl Link for WiredLink {
    fn label(&self) -> &'static str {
        "USB cable"
    }
    fn can_read_blocks(&self) -> bool {
        true
    }
    /// Live frames (cmd 0x08) are shown but not stored; the keyboard goes back
    /// to its saved lighting about a second after they stop.
    fn live_fps(&self) -> f64 {
        30.0
    }
    fn live_few_colors(&self) -> bool {
        false
    }

    fn probe(&mut self) -> Result<String> {
        let resp = self.request(&[0x06, 0x82, 0x01, 0x00, 0x01, 0x00, 0x06], 2)?;
        if resp.len() < 14 || resp[1] != 0x82 {
            return Err(Error::Device(
                "The keyboard did not answer over USB. Is another program (or this app's music \
                 sync) using it right now?"
                    .into(),
            ));
        }
        Ok(model_name(resp[13]).into())
    }

    fn read_config(&mut self) -> Result<Vec<u8>> {
        self.read(0x04, CFG_LEN)
    }
    fn read_palette(&mut self) -> Result<Vec<u8>> {
        self.read(0x0A, PAL_LEN)
    }
    fn read_keys(&mut self) -> Result<Vec<u8>> {
        self.read(0x06, KEYS_LEN)
    }
    fn write_lighting(&mut self, cfg: &[u8], pal: &[u8]) -> Result<()> {
        self.write(0x0A, pal)?;
        self.write(0x04, cfg)
    }
    fn write_keys(&mut self, cfg: &[u8], keys: &[u8]) -> Result<()> {
        self.write(0x06, keys)?;
        self.write(0x04, cfg)
    }
    fn write_config(&mut self, cfg: &[u8]) -> Result<()> {
        self.write(0x04, cfg)
    }
    fn read_battery(&mut self) -> Option<Battery> {
        None // the wired interface has no battery report; the cable charges it
    }

    fn send_live(&mut self, colors: &Frame) -> Result<()> {
        let mut buf = [0u8; Self::REPORT_SIZE];
        buf[..8].copy_from_slice(&[0x06, 0x08, 0, 0, 1, 0, 0x7A, 0x01]);
        for (led, rgb) in colors.iter().enumerate() {
            buf[8 + 3 * led..11 + 3 * led].copy_from_slice(rgb);
        }
        match self.dev.set_feature(&buf) {
            Err(e) if !transient(&e) => Err(e.into()),
            _ => Ok(()), // a stalled frame is simply dropped; the next one follows shortly
        }
    }

    fn alive(&self) -> bool {
        self.dev.alive()
    }
}

pub struct DongleLink {
    dev: Hidraw,
}

/// A 20-byte 2.4G frame: `13 cmd sub seq payload... checksum`.
pub fn frame(cmd: u8, sub: u8, seq: u8, payload: &[u8]) -> [u8; 20] {
    let mut f = [0u8; 20];
    f[..4].copy_from_slice(&[0x13, cmd, sub, seq]);
    f[4..4 + payload.len()].copy_from_slice(payload);
    f[19] = f[..19].iter().fold(0u8, |s, b| s.wrapping_add(*b));
    f
}

fn frames(cmd: u8, sub: u8, body: &[u8]) -> Vec<[u8; 20]> {
    body.chunks(14)
        .enumerate()
        .map(|(seq, chunk)| {
            let mut payload = vec![chunk.len() as u8];
            payload.extend_from_slice(chunk);
            frame(cmd, sub, seq as u8, &payload)
        })
        .collect()
}

/// The OEM "audio dance" stream: colour groups `R G B count led...`, biggest
/// groups first, at most 14 fragments of 14 bytes. LEDs not listed are off.
pub fn live_frames(colors: &Frame) -> Vec<[u8; 20]> {
    let mut groups: Vec<([u8; 3], Vec<u8>)> = vec![];
    for (led, rgb) in colors.iter().enumerate() {
        if rgb.iter().any(|&c| c != 0) {
            match groups.iter_mut().find(|(c, _)| c == rgb) {
                Some((_, leds)) => leds.push(led as u8),
                None => groups.push((*rgb, vec![led as u8])),
            }
        }
    }
    groups.sort_by_key(|(_, leds)| std::cmp::Reverse(leds.len())); // stable
    let mut data: Vec<u8> = vec![];
    for (rgb, leds) in &groups {
        if data.len() + 4 + leds.len() <= 14 * 14 {
            data.extend_from_slice(rgb);
            data.push(leds.len() as u8);
            data.extend_from_slice(leds);
        }
    }
    if data.is_empty() {
        return vec![frame(0x88, 0x01, 0, &[0x23])]; // idle: all off
    }
    let chunks: Vec<&[u8]> = data.chunks(14).collect();
    chunks
        .iter()
        .enumerate()
        .map(|(seq, chunk)| {
            let mut payload = vec![0x10 + chunk.len() as u8];
            payload.extend_from_slice(chunk);
            frame(0x88, chunks.len() as u8, seq as u8, &payload)
        })
        .collect()
}

impl DongleLink {
    /// Tested: 2 ms spacing stutters over 2.4G, 10 ms is smooth.
    const FRAGMENT_GAP: Duration = Duration::from_millis(10);

    pub fn new(dev: Hidraw) -> Self {
        DongleLink { dev }
    }

    fn drain(&self) {
        while self.dev.read(0.0).is_some() {}
    }

    fn send_all(&self, frames: &[[u8; 20]]) -> Result<()> {
        self.drain();
        let mut echoed = 0;
        for f in frames {
            self.dev.write(f)?;
            let deadline = Instant::now() + Duration::from_millis(250);
            while Instant::now() < deadline {
                let left = deadline.saturating_duration_since(Instant::now()).as_secs_f64();
                if let Some(reply) = self.dev.read(left) {
                    if reply.len() >= 4 && reply[1..4] == f[1..4] {
                        echoed += 1;
                        break;
                    }
                }
            }
        }
        if echoed == 0 {
            return Err(Error::Device("The keyboard stopped answering over 2.4G. Press a key to wake it up.".into()));
        }
        Ok(())
    }

    fn read_config_tries(&self, attempts: u32) -> Result<Vec<u8>> {
        let mut parts: [Option<Vec<u8>>; 10] = Default::default();
        for _ in 0..attempts {
            // the radio occasionally drops a reply
            self.drain();
            self.dev.write(&frame(0x44, 0x01, 0, &[]))?;
            parts = Default::default();
            let deadline = Instant::now() + Duration::from_secs(1);
            while parts.iter().any(|p| p.is_none()) && Instant::now() < deadline {
                let left = deadline.saturating_duration_since(Instant::now()).as_secs_f64();
                if let Some(r) = self.dev.read(left) {
                    if r.len() >= 20 && r[..3] == [0x13, 0x44, 0x0a] && r[3] < 10 {
                        let seq = r[3] as usize;
                        parts[seq] = Some(r);
                    }
                }
            }
            if parts.iter().all(|p| p.is_some()) {
                break;
            }
        }
        if parts.iter().any(|p| p.is_none()) {
            return Err(Error::Device(
                "The 2.4G receiver is plugged in but the keyboard is not answering. Switch the \
                 keyboard to 2.4G mode and press a key to wake it."
                    .into(),
            ));
        }
        let mut body = vec![];
        for p in parts.iter().flatten() {
            let n = (p[4] as usize).min(14);
            body.extend_from_slice(&p[5..5 + n]);
        }
        body.truncate(CFG_LEN);
        Ok(body)
    }

    /// Commit to the keyboard's memory. It doesn't always echo this frame (seen
    /// on an F87 Pro: no echo, yet the change was stored), so don't wait on it.
    fn save(&self) -> Result<()> {
        self.dev.write(&frame(0x0A, 0x01, 0, &[0x04, 0x07]))?;
        sleep(Duration::from_millis(50));
        self.drain();
        Ok(())
    }
}

impl Link for DongleLink {
    fn label(&self) -> &'static str {
        "2.4G receiver"
    }
    fn can_read_blocks(&self) -> bool {
        false // only the config block has a known read command
    }
    fn live_fps(&self) -> f64 {
        20.0
    }
    fn live_few_colors(&self) -> bool {
        true // every distinct colour costs bytes in the stream
    }

    fn probe(&mut self) -> Result<String> {
        self.read_config_tries(1)?; // quick check; a sleeping keyboard never answers
        Ok("F87".into())
    }

    fn read_config(&mut self) -> Result<Vec<u8>> {
        self.read_config_tries(3)
    }
    fn read_palette(&mut self) -> Result<Vec<u8>> {
        Err(Error::Device("The palette can't be read over 2.4G.".into()))
    }
    fn read_keys(&mut self) -> Result<Vec<u8>> {
        Err(Error::Device("Per-key colors can't be read over 2.4G.".into()))
    }
    fn write_lighting(&mut self, cfg: &[u8], pal: &[u8]) -> Result<()> {
        let mut all = frames(0x04, 0x0A, cfg);
        all.extend(frames(0x09, 0x25, pal));
        self.send_all(&all)?;
        self.save()
    }
    fn write_keys(&mut self, cfg: &[u8], keys: &[u8]) -> Result<()> {
        let mut body = keys.to_vec();
        body.extend_from_slice(&[0x00, 0x00, 0x5a, 0xa5, 0x00, 0x00]);
        let mut all = frames(0x04, 0x0A, cfg);
        all.extend(frames(0x02, 0x1C, &body));
        self.send_all(&all)?;
        self.save()
    }
    fn write_config(&mut self, cfg: &[u8]) -> Result<()> {
        self.send_all(&frames(0x04, 0x0A, cfg))?;
        self.save()
    }

    /// Request `13 4A 00…` -> reply `13 4A 01 00 02 <level> <state> … sum`.
    /// State 0x01 = on battery, 0x10 = cable plugged in (the level then reads
    /// 100 regardless, so it is reported as charging without a percentage).
    fn read_battery(&mut self) -> Option<Battery> {
        for _ in 0..2 {
            self.drain();
            self.dev.write(&frame(0x4A, 0x00, 0, &[])).ok()?;
            let deadline = Instant::now() + Duration::from_secs(1);
            while Instant::now() < deadline {
                let left = deadline.saturating_duration_since(Instant::now()).as_secs_f64();
                if let Some(r) = self.dev.read(left) {
                    let sum = r.iter().take(19).fold(0u8, |s, b| s.wrapping_add(*b));
                    if r.len() >= 20 && r[0] == 0x13 && r[1] & 0x7F == 0x4A && sum == r[19] {
                        let (level, state) = (r[5], r[6]);
                        let charging = state == 0x10;
                        let percent = ((1..=100).contains(&level) && !charging).then_some(level);
                        return Some(Battery { percent, charging });
                    }
                }
            }
            sleep(Duration::from_millis(200));
        }
        None
    }

    fn send_live(&mut self, colors: &Frame) -> Result<()> {
        for f in live_frames(colors) {
            self.dev.write(&f)?;
            sleep(Self::FRAGMENT_GAP);
        }
        self.drain(); // discard the echoes
        Ok(())
    }

    fn alive(&self) -> bool {
        self.dev.alive()
    }
}

// ------------------------------------------------------------------------- cache

/// Remembers the palette, per-key colours and last good config (the first two
/// can't be read back over 2.4G). Same file and format as the Python app.
pub struct Cache {
    path: PathBuf,
    data: Map<String, Value>,
}

impl Cache {
    pub fn load() -> Cache {
        let path = config_dir().join("cache.json");
        let data = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        Cache { path, data }
    }

    pub fn get(&self, name: &str, length: usize) -> Option<Vec<u8>> {
        let hex = self.data.get(name)?.as_str()?;
        let blob = decode_hex(hex)?;
        (blob.len() == length).then_some(blob)
    }

    pub fn put(&mut self, name: &str, blob: &[u8]) {
        let hex: String = blob.iter().map(|b| format!("{b:02x}")).collect();
        self.save(name, Value::String(hex));
    }

    pub fn get_value(&self, name: &str) -> Option<&Value> {
        self.data.get(name)
    }

    pub fn put_value(&mut self, name: &str, value: Value) {
        self.save(name, value);
    }

    fn save(&mut self, name: &str, value: Value) {
        self.data.insert(name.into(), value);
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let _ = fs::write(&self.path, Value::Object(self.data.clone()).to_string());
    }
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

// ------------------------------------------------------------------------- keyboard

#[derive(Clone, Debug, PartialEq)]
pub struct Lighting {
    pub effect: u8,
    pub brightness: u8,
    pub speed: u8,
    pub colorful: bool,
    pub color: [u8; 3],
    pub side: u8,
    /// Minutes of inactivity before the backlight sleeps; 0 = never.
    pub sleep: f64,
    /// Shown in the preview only; set on the keyboard (see docs/PROTOCOL.md):
    /// side-light colour index, and the light bar above the arrow keys
    /// (mode like the side lights, colour index, brightness 0-4).
    pub side_color: u8,
    pub bar: u8,
    pub bar_color: u8,
    pub bar_brightness: u8,
}

impl Default for Lighting {
    fn default() -> Self {
        Lighting {
            effect: 1,
            brightness: 4,
            speed: 3,
            colorful: false,
            color: [255, 255, 255],
            side: 1,
            sleep: 5.0,
            side_color: 0,
            bar: 0,
            bar_color: 0,
            bar_brightness: 4,
        }
    }
}

pub const CFG_SIDE_COLOR: usize = 19;
pub const CFG_BAR: usize = 28;
pub const CFG_BAR_COLOR: usize = 29;
pub const CFG_BAR_BRIGHTNESS: usize = 30;

/// The keyboard's own colours, as used by colour indexes: red, green, blue,
/// yellow, magenta, cyan, white.
pub fn standard_color(index: u8) -> [u8; 3] {
    let o = 3 * (index as usize % 7);
    [STD_COLORS[o], STD_COLORS[o + 1], STD_COLORS[o + 2]]
}

pub struct Keyboard {
    link: Mutex<Box<dyn Link>>,
    cache: Mutex<Cache>,
    pub model: String,
    pub label: &'static str,
    pub live_fps: f64,
    pub live_few_colors: bool,
    can_read_blocks: bool,
}

impl Keyboard {
    pub fn new(link: Box<dyn Link>, model: String, cache: Cache) -> Keyboard {
        Keyboard {
            label: link.label(),
            live_fps: link.live_fps(),
            live_few_colors: link.live_few_colors(),
            can_read_blocks: link.can_read_blocks(),
            link: Mutex::new(link),
            cache: Mutex::new(cache),
            model,
        }
    }

    pub fn description(&self) -> String {
        format!("{} · {}", self.model, self.label)
    }

    pub fn wired(&self) -> bool {
        self.label == "USB cable"
    }

    pub fn alive(&self) -> bool {
        match self.link.try_lock() {
            Ok(link) => link.alive(),
            Err(_) => true, // busy with a transfer, so it's there
        }
    }

    /// One live frame (music sync). Not stored in the keyboard.
    pub fn send_live(&self, colors: &Frame) -> Result<()> {
        self.link.lock().unwrap().send_live(colors)
    }

    // ------------------------------------------------------------ safe access
    //
    // Rules that keep the keyboard's stored settings intact:
    // * every block read is checked (config_ok / palette_ok) before it's used;
    // * nothing that fails the check is ever written, and unchanged blocks
    //   aren't rewritten;
    // * after a write the settings are read back, and if they came out damaged
    //   the previous ones are put back straight away;
    // * one program talks to the keyboard at a time (DeviceLock).

    fn read_config(&self, link: &mut dyn Link) -> Result<Vec<u8>> {
        for attempt in 0..4u64 {
            let cfg = match link.read_config() {
                Ok(c) => Some(c),
                Err(Error::Device(_)) if attempt == 0 => None, // e.g. a reply to an earlier request
                Err(e) => return Err(e),
            };
            if let Some(cfg) = cfg.filter(|c| config_ok(c)) {
                self.cache.lock().unwrap().put("good_config", &cfg);
                return Ok(cfg);
            }
            pause(250 * (attempt + 1)); // may be a stale reply; ask again
        }
        Err(Error::Damaged(DAMAGED.into()))
    }

    /// The palette stored in the keyboard, or None if it can't be read (2.4G)
    /// or looks damaged.
    fn device_palette(&self, link: &mut dyn Link) -> Result<Option<Vec<u8>>> {
        if !link.can_read_blocks() {
            return Ok(None);
        }
        for attempt in 0..3u64 {
            let pal = link.read_palette()?;
            if palette_ok(&pal) {
                self.cache.lock().unwrap().put("palette", &pal);
                return Ok(Some(pal));
            }
            pause(250 * (attempt + 1));
        }
        Ok(None)
    }

    fn good_config(&self) -> Vec<u8> {
        match self.cache.lock().unwrap().get("good_config", CFG_LEN) {
            Some(c) if config_ok(&c) => c,
            _ => DEFAULT_CONFIG.to_vec(),
        }
    }

    fn good_palette(&self) -> Vec<u8> {
        match self.cache.lock().unwrap().get("palette", PAL_LEN) {
            Some(p) if palette_ok(&p) => p,
            _ => default_palette(),
        }
    }

    /// Write what changed, then make sure the keyboard's settings are still
    /// intact; if not, put the previous ones back.
    fn store(
        &self,
        link: &mut dyn Link,
        mut cfg: Vec<u8>,
        before_cfg: &[u8],
        pal: Option<&[u8]>,
        before_pal: Option<&[u8]>,
        keys: Option<&[u8]>,
    ) -> Result<()> {
        cfg[CFG_CUSTOM] = (cfg[CFG_EFFECT] == PER_KEY) as u8;
        if !config_ok(&cfg) || pal.is_some_and(|p| !palette_ok(p)) {
            return Err(Error::Device("Not writing: the new settings don't look valid.".into()));
        }
        let write_pal = pal.is_some_and(|p| before_pal != Some(p));
        if keys.is_none() && !write_pal && cfg == before_cfg {
            return Ok(()); // nothing changed
        }
        if let Some(keys) = keys {
            link.write_keys(&cfg, keys)?;
        } else if write_pal {
            link.write_lighting(&cfg, pal.unwrap())?;
        } else {
            link.write_config(&cfg)?;
        }
        let check = self.read_config(link).and_then(|_| {
            if write_pal && link.can_read_blocks() && self.device_palette(link)?.is_none() {
                return Err(Error::Damaged("palette".into()));
            }
            Ok(())
        });
        match check {
            Err(Error::Damaged(_)) => {
                pause(1000);
                let pal = before_pal.map(|p| p.to_vec()).unwrap_or_else(|| self.good_palette());
                self.put_back(link, before_cfg, &pal)?;
                return Err(Error::Device(
                    "The keyboard didn't store that change properly, so its previous settings were \
                     put back. Try again in a moment."
                        .into(),
                ));
            }
            Err(e) => return Err(e),
            Ok(()) => {}
        }
        if let Some(pal) = pal {
            self.cache.lock().unwrap().put("palette", pal);
        }
        Ok(())
    }

    fn put_back(&self, link: &mut dyn Link, cfg: &[u8], pal: &[u8]) -> Result<()> {
        let mut cfg = cfg.to_vec();
        cfg[CFG_CUSTOM] = (cfg[CFG_EFFECT] == PER_KEY) as u8;
        link.write_lighting(&cfg, pal)?;
        self.read_config(link).map(|_| ()) // Damaged if even that didn't take
    }

    /// Put back this keyboard's last good settings, or DEFAULT_CONFIG if none
    /// were saved on this computer. Returns which one was used.
    pub fn repair(&self) -> Result<&'static str> {
        let saved = matches!(self.cache.lock().unwrap().get("good_config", CFG_LEN), Some(c) if config_ok(&c));
        let _lock = DeviceLock::acquire()?;
        let mut link = self.link.lock().unwrap();
        let (cfg, pal) = (self.good_config(), self.good_palette());
        self.put_back(link.as_mut(), &cfg, &pal)?;
        Ok(if saved { "last good settings" } else { "default settings" })
    }

    // ------------------------------------------------------------ operations

    pub fn read_lighting(&self) -> Result<Lighting> {
        let (cfg, pal) = {
            let _lock = DeviceLock::acquire()?;
            let mut link = self.link.lock().unwrap();
            let cfg = self.read_config(link.as_mut())?;
            let pal = match self.device_palette(link.as_mut())? {
                Some(p) => p,
                None => self.good_palette(),
            };
            (cfg, pal)
        };
        let mut state = Lighting {
            effect: cfg[CFG_EFFECT],
            side: cfg[CFG_SIDE],
            sleep: cfg[CFG_SLEEP] as f64 / 2.0,
            side_color: cfg[CFG_SIDE_COLOR],
            bar: cfg[CFG_BAR],
            bar_color: cfg[CFG_BAR_COLOR],
            bar_brightness: cfg[CFG_BAR_BRIGHTNESS].min(4),
            ..Lighting::default()
        };
        if (1..=18).contains(&state.effect) {
            let p = param_offset(state.effect);
            state.brightness = cfg[p].min(4);
            state.speed = (cfg[p + 1] >> 4).min(4);
            let slot = cfg[p + 1] & 0x0F;
            state.colorful = slot >= COLORFUL;
            let o = slot_offset(state.effect, if state.colorful { 0 } else { slot as usize });
            if pal[o..o + 3].iter().any(|&c| c != 0) {
                state.color = [pal[o], pal[o + 1], pal[o + 2]];
            }
        }
        Ok(state)
    }

    pub fn apply_lighting(&self, state: &Lighting) -> Result<()> {
        let _lock = DeviceLock::acquire()?;
        let mut link = self.link.lock().unwrap();
        let before_cfg = self.read_config(link.as_mut())?;
        // None over 2.4G (can't be read) or if damaged: then it is always sent.
        let before_pal = self.device_palette(link.as_mut())?;
        let mut cfg = before_cfg.clone();
        let mut pal = before_pal.clone().unwrap_or_else(|| self.good_palette());
        let info = effect_info(state.effect);
        cfg[CFG_EFFECT] = state.effect;
        cfg[CFG_COLOR_MODE] = 1;
        cfg[CFG_SIDE] = state.side;
        if (1..=18).contains(&state.effect) {
            let p = param_offset(state.effect);
            cfg[p] = state.brightness.min(4);
            let speed = if info.speed { state.speed.min(4) } else { cfg[p + 1] >> 4 };
            let mut slot = cfg[p + 1] & 0x0F;
            if info.color {
                slot = if state.colorful { COLORFUL } else { 0 };
                if !state.colorful {
                    let o = slot_offset(state.effect, 0);
                    pal[o..o + 3].copy_from_slice(&state.color);
                }
            }
            cfg[p + 1] = speed << 4 | slot;
        }
        self.store(link.as_mut(), cfg, &before_cfg, Some(&pal), before_pal.as_deref(), None)
    }

    /// Battery over 2.4G, or None (USB cable / no reply / busy). The keyboard
    /// only reports its level over 2.4G while the cable is unplugged, so the
    /// last real reading is remembered (see last_battery).
    pub fn read_battery(&self) -> Option<Battery> {
        let _lock = DeviceLock::try_acquire(Duration::from_millis(500))?; // busy: next time
        let battery = self.link.lock().unwrap().read_battery();
        if let Some(Battery { percent: Some(p), .. }) = battery {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
            self.cache.lock().unwrap().put_value("battery", json!({"percent": p, "time": now}));
        }
        battery
    }

    /// (percent, unix time) of the last 2.4G reading.
    pub fn last_battery(&self) -> Option<(i64, f64)> {
        let cache = self.cache.lock().unwrap();
        let v = cache.get_value("battery")?;
        Some((v.get("percent")?.as_f64()? as i64, v.get("time")?.as_f64()?))
    }

    /// Idle minutes before the backlight turns off; 0 = never. Stored in the
    /// keyboard (one write), so it also applies without this app.
    pub fn set_sleep(&self, minutes: f64) -> Result<()> {
        let _lock = DeviceLock::acquire()?;
        let mut link = self.link.lock().unwrap();
        let before = self.read_config(link.as_mut())?;
        let mut cfg = before.clone();
        cfg[CFG_SLEEP] = (minutes * 2.0).round_ties_even().clamp(0.0, 255.0) as u8;
        self.store(link.as_mut(), cfg, &before, None, None, None)
    }

    pub fn read_keys(&self) -> Result<Frame> {
        let keys = if self.can_read_blocks {
            let keys = {
                let _lock = DeviceLock::acquire()?;
                self.link.lock().unwrap().read_keys()?
            };
            self.cache.lock().unwrap().put("keys", &keys);
            keys
        } else {
            self.cache.lock().unwrap().get("keys", KEYS_LEN).unwrap_or_else(|| vec![0; KEYS_LEN])
        };
        Ok(std::array::from_fn(|i| [keys[i], keys[PLANE + i], keys[2 * PLANE + i]]))
    }

    pub fn apply_keys(&self, colors: &Frame) -> Result<()> {
        let mut keys = vec![0u8; KEYS_LEN];
        for (i, [r, g, b]) in colors.iter().enumerate() {
            keys[i] = *r;
            keys[PLANE + i] = *g;
            keys[2 * PLANE + i] = *b;
        }
        {
            let _lock = DeviceLock::acquire()?;
            let mut link = self.link.lock().unwrap();
            let before = self.read_config(link.as_mut())?;
            let mut cfg = before.clone();
            cfg[CFG_EFFECT] = PER_KEY;
            cfg[CFG_COLOR_MODE] = 1;
            self.store(link.as_mut(), cfg, &before, None, None, Some(&keys))?;
        }
        self.cache.lock().unwrap().put("keys", &keys);
        Ok(())
    }
}

/// Open the keyboard over USB if possible, otherwise over the 2.4G receiver.
pub fn connect() -> Result<Keyboard> {
    #[cfg(feature = "fake-keyboard")]
    if std::env::var_os("AULA_F87_FAKE").is_some() {
        return Ok(crate::fake::keyboard());
    }
    let _lock = DeviceLock::acquire()?;
    let mut problems: Vec<String> = vec![];
    type Opener = fn(Hidraw) -> Box<dyn Link>;
    let kinds: [((u32, u32), u8, Opener); 2] = [
        (WIRED_ID, 0x06, |d| Box::new(WiredLink::new(d))),
        (DONGLE_ID, 0x13, |d| Box::new(DongleLink::new(d))),
    ];
    for (usb_id, report_id, open) in kinds {
        for path in hidraw::candidates(usb_id, report_id) {
            let result = Hidraw::open(&path).and_then(|dev| {
                let mut link = open(dev);
                let mut model = link.probe()?;
                let mut cache = Cache::load();
                if link.can_read_blocks() {
                    cache.put_value("model", Value::String(model.clone()));
                } else if let Some(known) = cache.get_value("model").and_then(|v| v.as_str()) {
                    model = known.to_string(); // the 2.4G receiver doesn't report the model
                }
                Ok(Keyboard::new(link, model, cache))
            });
            match result {
                Ok(kb) => return Ok(kb),
                Err(e) => problems.push(e.to_string()),
            }
        }
    }
    // Prefer the most helpful message (permission / wake-up hints come last).
    Err(Error::Device(problems.pop().unwrap_or_else(|| {
        "No AULA F87 found. Plug in the USB cable or the 2.4G receiver.".into()
    })))
}
