//! Raw HID access on Windows through the Win32 HID API (the same calls hidapi
//! makes). Each top-level collection of the keyboard is its own device path;
//! the vendor collections carry our reports. Same interface as `hidraw.rs`.

use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use windows::core::PCWSTR;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_Device_Interface_ListW, CM_Get_Device_Interface_List_SizeW, CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CR_SUCCESS,
};
use windows::Win32::Devices::HumanInterfaceDevice::{
    HidD_FreePreparsedData, HidD_GetAttributes, HidD_GetFeature, HidD_GetPreparsedData, HidD_SetFeature, HidP_GetCaps,
    GUID_DEVINTERFACE_HID, HIDD_ATTRIBUTES, HIDP_CAPS, HIDP_STATUS_SUCCESS, PHIDP_PREPARSED_DATA,
};
use windows::Win32::Foundation::{CloseHandle, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows::Win32::System::Threading::{CreateEventW, ResetEvent, WaitForSingleObject};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

use crate::device::Error;

/// A handle that may move between threads (the keyboard is used from a worker).
struct Owned(HANDLE);
unsafe impl Send for Owned {}
unsafe impl Sync for Owned {}

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

/// An OVERLAPPED with its event, boxed so its address stays put during I/O.
struct Ov {
    ov: Box<OVERLAPPED>,
    event: Owned,
}
unsafe impl Send for Ov {}

impl Ov {
    fn new() -> io::Result<Ov> {
        let event = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.map_err(win_err)?;
        let mut ov = Box::new(OVERLAPPED::default());
        ov.hEvent = event;
        Ok(Ov { ov, event: Owned(event) })
    }
}

fn win_err(e: windows::core::Error) -> io::Error {
    let code = e.code().0 as u32;
    if code & 0xFFFF_0000 == 0x8007_0000 {
        io::Error::from_raw_os_error((code & 0xFFFF) as i32)
    } else {
        io::Error::other(e.message())
    }
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn open_raw(path: &Path, access: u32, overlapped: bool) -> io::Result<Owned> {
    let w = wide(path);
    let flags = if overlapped { FILE_FLAG_OVERLAPPED } else { FILE_FLAGS_AND_ATTRIBUTES(0) };
    let h = unsafe {
        CreateFileW(PCWSTR(w.as_ptr()), access, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, flags, None)
    }
    .map_err(win_err)?;
    Ok(Owned(h))
}

/// Usage page and report lengths of one collection.
#[derive(Clone, Copy, Debug, Default)]
struct Caps {
    vid: u16,
    pid: u16,
    usage_page: u16,
    input_len: usize,
    output_len: usize,
    feature_len: usize,
}

fn caps_of(h: HANDLE) -> Option<Caps> {
    let mut attrs = HIDD_ATTRIBUTES { Size: std::mem::size_of::<HIDD_ATTRIBUTES>() as u32, ..Default::default() };
    if !unsafe { HidD_GetAttributes(h, &mut attrs) } {
        return None;
    }
    let mut pp = PHIDP_PREPARSED_DATA::default();
    if !unsafe { HidD_GetPreparsedData(h, &mut pp) } {
        return None;
    }
    let mut caps = HIDP_CAPS::default();
    let status = unsafe { HidP_GetCaps(pp, &mut caps) };
    unsafe {
        let _ = HidD_FreePreparsedData(pp);
    }
    if status != HIDP_STATUS_SUCCESS {
        return None;
    }
    Some(Caps {
        vid: attrs.VendorID,
        pid: attrs.ProductID,
        usage_page: caps.UsagePage,
        input_len: caps.InputReportByteLength as usize,
        output_len: caps.OutputReportByteLength as usize,
        feature_len: caps.FeatureReportByteLength as usize,
    })
}

/// Device paths of every HID collection present.
fn interfaces() -> Vec<PathBuf> {
    let mut len = 0u32;
    let r = unsafe {
        CM_Get_Device_Interface_List_SizeW(&mut len, &GUID_DEVINTERFACE_HID, PCWSTR::null(), CM_GET_DEVICE_INTERFACE_LIST_PRESENT)
    };
    if r != CR_SUCCESS || len == 0 {
        return vec![];
    }
    let mut buf = vec![0u16; len as usize];
    let r = unsafe {
        CM_Get_Device_Interface_ListW(&GUID_DEVINTERFACE_HID, PCWSTR::null(), &mut buf, CM_GET_DEVICE_INTERFACE_LIST_PRESENT)
    };
    if r != CR_SUCCESS {
        return vec![];
    }
    buf.split(|&c| c == 0).filter(|s| !s.is_empty()).map(|s| PathBuf::from(OsString::from_wide(s))).collect()
}

pub struct Hidraw {
    dev: Owned,
    path: PathBuf,
    caps: Caps,
    read_ov: Mutex<Ov>,
    write_ov: Mutex<Ov>,
}

impl Hidraw {
    pub fn open(path: &Path) -> Result<Hidraw, Error> {
        let dev = match open_raw(path, GENERIC_READ.0 | GENERIC_WRITE.0, true) {
            Ok(d) => d,
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                return Err(Error::Device(
                    "Windows didn't let this app open the keyboard. Close AULA's own software if it is \
                     running, then try again."
                        .into(),
                ))
            }
            Err(e) => return Err(Error::Io(e)),
        };
        let caps = caps_of(dev.0).ok_or_else(|| Error::Device("Couldn't read the keyboard's HID description.".into()))?;
        Ok(Hidraw {
            dev,
            path: path.to_path_buf(),
            caps,
            read_ov: Mutex::new(Ov::new()?),
            write_ov: Mutex::new(Ov::new()?),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn padded(data: &[u8], len: usize) -> Vec<u8> {
        let mut buf = data.to_vec();
        if buf.len() < len {
            buf.resize(len, 0);
        }
        buf
    }

    /// HidD_SetFeature; the buffer is padded to the collection's feature length.
    pub fn set_feature(&self, data: &[u8]) -> io::Result<()> {
        let buf = Self::padded(data, self.caps.feature_len);
        if unsafe { HidD_SetFeature(self.dev.0, buf.as_ptr() as *const _, buf.len() as u32) } {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    /// HidD_GetFeature; the reply starts with the report ID, like hidraw's.
    pub fn get_feature(&self, report_id: u8, size: usize) -> io::Result<Vec<u8>> {
        let mut buf = vec![0u8; self.caps.feature_len.max(size)];
        buf[0] = report_id;
        if !unsafe { HidD_GetFeature(self.dev.0, buf.as_mut_ptr() as *mut _, buf.len() as u32) } {
            return Err(io::Error::last_os_error());
        }
        buf.truncate(size);
        Ok(buf)
    }

    /// One output report (padded to the collection's output length).
    pub fn write(&self, data: &[u8]) -> io::Result<()> {
        let buf = Self::padded(data, self.caps.output_len);
        let mut guard = self.write_ov.lock().unwrap();
        let ov = &mut *guard;
        unsafe {
            let _ = ResetEvent(ov.event.0);
        }
        let started = unsafe { WriteFile(self.dev.0, Some(&buf), None, Some(&mut *ov.ov)) };
        if let Err(e) = started {
            if e.code() != ERROR_IO_PENDING.to_hresult() {
                return Err(win_err(e));
            }
            if unsafe { WaitForSingleObject(ov.event.0, 1000) } != WAIT_OBJECT_0 {
                unsafe {
                    let _ = CancelIoEx(self.dev.0, Some(&*ov.ov));
                }
                let mut n = 0u32;
                let _ = unsafe { GetOverlappedResult(self.dev.0, &*ov.ov, &mut n, true) };
                return Err(io::Error::from(io::ErrorKind::TimedOut));
            }
        }
        let mut n = 0u32;
        unsafe { GetOverlappedResult(self.dev.0, &*ov.ov, &mut n, false) }.map_err(win_err)?;
        Ok(())
    }

    /// One input report, or None if nothing arrives within `timeout` seconds.
    pub fn read(&self, timeout: f64) -> Option<Vec<u8>> {
        let len = if self.caps.input_len > 0 { self.caps.input_len } else { 64 };
        let mut buf = vec![0u8; len];
        let mut guard = self.read_ov.lock().unwrap();
        let ov = &mut *guard;
        unsafe {
            let _ = ResetEvent(ov.event.0);
        }
        let started = unsafe { ReadFile(self.dev.0, Some(&mut buf), None, Some(&mut *ov.ov)) };
        if let Err(e) = started {
            if e.code() != ERROR_IO_PENDING.to_hresult() {
                return None;
            }
            let ms = (timeout.max(0.0) * 1000.0).ceil() as u32;
            if unsafe { WaitForSingleObject(ov.event.0, ms) } != WAIT_OBJECT_0 {
                // Nothing yet: cancel, but keep a report that landed meanwhile.
                unsafe {
                    let _ = CancelIoEx(self.dev.0, Some(&*ov.ov));
                }
            }
        }
        let mut n = 0u32;
        match unsafe { GetOverlappedResult(self.dev.0, &*ov.ov, &mut n, true) } {
            Ok(()) if n > 0 => {
                buf.truncate(n as usize);
                Some(buf)
            }
            _ => None,
        }
    }

    /// Still plugged in: the collection's path can still be opened.
    pub fn alive(&self) -> bool {
        open_raw(&self.path, 0, false).is_ok()
    }
}

/// Vendor collections of this USB device that can carry `report_id`
/// (0x06: 520-byte feature reports over the cable; 0x13: 20-byte output
/// reports over the 2.4G receiver), preferred usage pages first.
pub fn candidates(usb_id: (u32, u32), report_id: u8) -> Vec<PathBuf> {
    let preferred: &[u16] = if report_id == 0x06 { &[0xFF00] } else { &[0xFF02, 0xFF04] };
    let fits = |c: &Caps| if report_id == 0x06 { c.feature_len >= 520 } else { c.output_len >= 20 };
    let mut found: Vec<(PathBuf, Caps)> = vec![];
    for path in interfaces() {
        let Ok(h) = open_raw(&path, 0, false) else { continue };
        let Some(caps) = caps_of(h.0) else { continue };
        if (caps.vid as u32, caps.pid as u32) == usb_id && caps.usage_page >= 0xFF00 {
            found.push((path, caps));
        }
    }
    // Collections whose report lengths fit come first; the rest stay as a fallback.
    found.sort_by_key(|(_, c)| {
        let rank = preferred.iter().position(|&p| p == c.usage_page).unwrap_or(preferred.len());
        (!fits(c), rank)
    });
    found.into_iter().map(|(p, _)| p).collect()
}

/// Everything known about the keyboard's collections, for Debugging Information.
pub fn describe(usb_id: (u32, u32)) -> Vec<String> {
    let mut out = vec![];
    for path in interfaces() {
        let Ok(h) = open_raw(&path, 0, false) else { continue };
        let Some(c) = caps_of(h.0) else { continue };
        if (c.vid as u32, c.pid as u32) == usb_id {
            out.push(format!(
                "page {:04X} in {} out {} feature {}",
                c.usage_page, c.input_len, c.output_len, c.feature_len
            ));
        }
    }
    out
}
