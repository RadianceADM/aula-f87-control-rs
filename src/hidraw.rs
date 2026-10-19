//! Raw HID access through Linux hidraw (ioctl, read and write). No extra libraries.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use crate::device::Error;

fn ioc_rw(nr: u64, size: usize) -> u64 {
    (3 << 30) | ((size as u64) << 16) | ((b'H' as u64) << 8) | nr
}

pub struct Hidraw {
    file: File,
    path: PathBuf,
}

impl Hidraw {
    pub fn open(path: &Path) -> Result<Hidraw, Error> {
        match OpenOptions::new().read(true).write(true).custom_flags(libc::O_NONBLOCK).open(path) {
            Ok(file) => Ok(Hidraw { file, path: path.to_path_buf() }),
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => Err(Error::Device(format!(
                "Permission denied on {}. Install the udev rule (see README), then unplug and \
                 replug the keyboard.",
                path.display()
            ))),
            Err(e) => Err(Error::Io(e)),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// HIDIOCSFEATURE
    pub fn set_feature(&self, data: &[u8]) -> io::Result<()> {
        let r = unsafe { libc::ioctl(self.file.as_raw_fd(), ioc_rw(0x06, data.len()) as _, data.as_ptr()) };
        if r < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    /// HIDIOCGFEATURE
    pub fn get_feature(&self, report_id: u8, size: usize) -> io::Result<Vec<u8>> {
        let mut buf = vec![0u8; size];
        buf[0] = report_id;
        let r = unsafe { libc::ioctl(self.file.as_raw_fd(), ioc_rw(0x07, size) as _, buf.as_mut_ptr()) };
        if r < 0 {
            return Err(io::Error::last_os_error());
        }
        buf.truncate(r as usize);
        Ok(buf)
    }

    pub fn write(&self, data: &[u8]) -> io::Result<()> {
        (&self.file).write_all(data)
    }

    /// One input report, or None if nothing arrives within `timeout` seconds.
    pub fn read(&self, timeout: f64) -> Option<Vec<u8>> {
        let mut pfd = libc::pollfd { fd: self.file.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let ms = (timeout.max(0.0) * 1000.0).ceil() as libc::c_int;
        if unsafe { libc::poll(&mut pfd, 1, ms) } <= 0 {
            return None;
        }
        let mut buf = [0u8; 64];
        match (&self.file).read(&mut buf) {
            Ok(n) if n > 0 => Some(buf[..n].to_vec()),
            _ => None,
        }
    }

    pub fn alive(&self) -> bool {
        self.path.exists()
    }
}

/// hidraw nodes of this USB device whose report descriptor declares the
/// report ID we need (`85 id`).
pub fn candidates(usb_id: (u32, u32), report_id: u8) -> Vec<PathBuf> {
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
        if parts.len() == 3
            && (parts[1], parts[2]) == usb_id
            && desc.windows(2).any(|w| w == [0x85, report_id])
            && dev.exists()
        {
            found.push(dev);
        }
    }
    found
}
