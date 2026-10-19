//! Live frames to the keyboard over Linux hidraw, plus the lock file shared
//! with the Python app (so the two never talk to the keyboard at once).

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::layout::PLANE;
use crate::render::Frame;

const WIRED_ID: (u32, u32) = (0x258A, 0x010C);
const DONGLE_ID: (u32, u32) = (0x3554, 0xFA09);
const REPORT_SIZE: usize = 520;
const TRANSIENT: [i32; 4] = [libc::EPROTO, libc::EPIPE, libc::ETIMEDOUT, libc::EAGAIN];

fn ioc_rw(nr: u64, size: usize) -> u64 {
    (3 << 30) | ((size as u64) << 16) | ((b'H' as u64) << 8) | nr
}

fn set_feature(f: &File, data: &[u8]) -> io::Result<()> {
    // HIDIOCSFEATURE
    let r = unsafe { libc::ioctl(f.as_raw_fd(), ioc_rw(0x06, data.len()) as _, data.as_ptr()) };
    if r < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

fn get_feature(f: &File, report_id: u8, size: usize) -> io::Result<Vec<u8>> {
    // HIDIOCGFEATURE
    let mut buf = vec![0u8; size];
    buf[0] = report_id;
    let r = unsafe { libc::ioctl(f.as_raw_fd(), ioc_rw(0x07, size) as _, buf.as_mut_ptr()) };
    if r < 0 {
        return Err(io::Error::last_os_error());
    }
    buf.truncate(r as usize);
    Ok(buf)
}

fn is_transient(e: &io::Error) -> bool {
    e.raw_os_error().is_some_and(|c| TRANSIENT.contains(&c))
}

/// hidraw nodes of this USB device whose report descriptor declares the report ID.
fn candidates(usb_id: (u32, u32), report_id: u8) -> Vec<PathBuf> {
    let mut nodes: Vec<_> = match fs::read_dir("/sys/class/hidraw") {
        Ok(d) => d.filter_map(|e| e.ok()).map(|e| e.path()).collect(),
        Err(_) => return vec![],
    };
    nodes.sort();
    let mut found = vec![];
    for node in nodes {
        let (Ok(uevent), Ok(desc)) = (
            fs::read_to_string(node.join("device/uevent")),
            fs::read(node.join("device/report_descriptor")),
        ) else {
            continue;
        };
        let Some(id) = uevent.lines().find_map(|l| l.strip_prefix("HID_ID=")) else { continue };
        let parts: Vec<u32> = id.split(':').filter_map(|p| u32::from_str_radix(p, 16).ok()).collect();
        let dev = PathBuf::from("/dev").join(node.file_name().unwrap());
        if parts.len() == 3 && (parts[1], parts[2]) == usb_id
            && desc.windows(2).any(|w| w == [0x85, report_id]) && dev.exists()
        {
            found.push(dev);
        }
    }
    found
}

fn open(path: &PathBuf) -> io::Result<File> {
    OpenOptions::new().read(true).write(true).custom_flags(libc::O_NONBLOCK).open(path)
}

pub enum Link {
    Wired(File),
    Dongle(File),
}

impl Link {
    pub fn label(&self) -> &'static str {
        match self {
            Link::Wired(_) => "USB cable",
            Link::Dongle(_) => "2.4G receiver",
        }
    }

    pub fn fps(&self) -> f64 {
        match self {
            Link::Wired(_) => 30.0,
            Link::Dongle(_) => 20.0,
        }
    }

    pub fn few_colors(&self) -> bool {
        matches!(self, Link::Dongle(_))
    }

    /// The cable if it answers, otherwise the 2.4G receiver.
    pub fn connect() -> Result<Link, String> {
        let mut problems = vec![];
        for path in candidates(WIRED_ID, 0x06) {
            match open(&path).and_then(|f| probe_wired(&f).map(|_| f)) {
                Ok(f) => return Ok(Link::Wired(f)),
                Err(e) => problems.push(format!("{}: {e}", path.display())),
            }
        }
        for path in candidates(DONGLE_ID, 0x13) {
            match open(&path).and_then(|f| probe_dongle(&f).map(|_| f)) {
                Ok(f) => return Ok(Link::Dongle(f)),
                Err(e) => problems.push(format!("{}: {e}", path.display())),
            }
        }
        Err(problems.pop().unwrap_or_else(|| {
            "No AULA F87 found. Plug in the USB cable or the 2.4G receiver.".into()
        }))
    }

    /// Show a frame without storing it (wired cmd 0x08, 2.4G cmd 0x88).
    pub fn send_live(&mut self, colors: &Frame) -> io::Result<()> {
        match self {
            Link::Wired(f) => {
                let mut buf = [0u8; REPORT_SIZE];
                buf[..8].copy_from_slice(&[0x06, 0x08, 0, 0, 1, 0, 0x7A, 0x01]);
                for (led, rgb) in colors.iter().enumerate().take(PLANE) {
                    buf[8 + 3 * led..11 + 3 * led].copy_from_slice(rgb);
                }
                match set_feature(f, &buf) {
                    Err(e) if !is_transient(&e) => Err(e),
                    _ => Ok(()), // a stalled frame is simply dropped
                }
            }
            Link::Dongle(f) => {
                for frame in dongle_frames(colors) {
                    f.write_all(&frame)?;
                    sleep(Duration::from_millis(10)); // 2 ms stutters over 2.4G
                }
                drain(f);
                Ok(())
            }
        }
    }
}

fn probe_wired(f: &File) -> io::Result<()> {
    let mut req = [0u8; REPORT_SIZE];
    req[..7].copy_from_slice(&[0x06, 0x82, 0x01, 0x00, 0x01, 0x00, 0x06]);
    let mut last = io::Error::other("no answer");
    for attempt in 0..2 {
        let r = set_feature(f, &req).and_then(|_| {
            sleep(Duration::from_millis(10));
            get_feature(f, 0x06, REPORT_SIZE)
        });
        match r {
            Ok(resp) if resp.len() >= 14 && resp[1] == 0x82 => return Ok(()),
            Ok(_) => last = io::Error::other("unexpected reply"),
            Err(e) => last = e,
        }
        sleep(Duration::from_millis(50 * (attempt + 1)));
    }
    Err(last)
}

fn frame(cmd: u8, sub: u8, seq: u8, payload: &[u8]) -> [u8; 20] {
    let mut f = [0u8; 20];
    f[..4].copy_from_slice(&[0x13, cmd, sub, seq]);
    f[4..4 + payload.len()].copy_from_slice(payload);
    f[19] = f[..19].iter().fold(0u8, |s, b| s.wrapping_add(*b));
    f
}

fn drain(mut f: &File) {
    let mut buf = [0u8; 64];
    while f.read(&mut buf).is_ok_and(|n| n > 0) {}
}

fn probe_dongle(f: &File) -> io::Result<()> {
    drain(f);
    let mut w = f;
    w.write_all(&frame(0x44, 0x01, 0, &[]))?;
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut buf = [0u8; 64];
    let mut r = f;
    while Instant::now() < deadline {
        if let Ok(n) = r.read(&mut buf) {
            if n >= 20 && buf[..3] == [0x13, 0x44, 0x0A] {
                sleep(Duration::from_millis(100));
                drain(f);
                return Ok(());
            }
        }
        sleep(Duration::from_millis(2));
    }
    Err(io::Error::other(
        "The 2.4G receiver is plugged in but the keyboard is not answering. Press a key to wake it.",
    ))
}

/// The OEM "audio dance" stream: colour groups `R G B count led...`,
/// biggest groups first, at most 14 fragments of 14 bytes.
pub fn dongle_frames(colors: &Frame) -> Vec<[u8; 20]> {
    let mut groups: Vec<([u8; 3], Vec<u8>)> = vec![];
    for (led, rgb) in colors.iter().enumerate().take(PLANE) {
        if rgb.iter().any(|&c| c != 0) {
            match groups.iter_mut().find(|(c, _)| c == rgb) {
                Some((_, leds)) => leds.push(led as u8),
                None => groups.push((*rgb, vec![led as u8])),
            }
        }
    }
    groups.sort_by_key(|(_, leds)| std::cmp::Reverse(leds.len())); // stable, like sorted()
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

/// The lock file used by the Python app (device.DeviceLock).
pub struct DeviceLock {
    _file: File,
}

impl DeviceLock {
    pub fn path() -> PathBuf {
        let base = match std::env::var("XDG_RUNTIME_DIR") {
            Ok(d) if !d.is_empty() => PathBuf::from(d),
            _ => std::env::var("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|_| {
                PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache")
            }),
        };
        base.join("aula-f87-control.lock")
    }

    pub fn acquire(timeout: Duration) -> Option<DeviceLock> {
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
}
