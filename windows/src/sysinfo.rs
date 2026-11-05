//! The Windows version, for Debugging Information.

use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW};
use windows::core::{HSTRING, PCWSTR, w};

const KEY: PCWSTR = w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion");

fn text(name: &str) -> Option<String> {
    let name = HSTRING::from(name);
    let mut buf = [0u16; 256];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, KEY, &name, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len))
    };
    if r.is_err() {
        return None;
    }
    let n = (len as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..n]))
}

fn number(name: &str) -> Option<u32> {
    let name = HSTRING::from(name);
    let mut value = 0u32;
    let mut len = 4u32;
    let r = unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, KEY, &name, RRF_RT_REG_DWORD, None, Some(&mut value as *mut u32 as *mut _), Some(&mut len))
    };
    r.is_ok().then_some(value)
}

pub fn windows_version() -> String {
    let build: u32 = text("CurrentBuild").and_then(|b| b.parse().ok()).unwrap_or(0);
    // Windows 11 still says "Windows 10" in ProductName.
    let product = text("ProductName").unwrap_or_else(|| "Windows".into());
    let product = if build >= 22000 { product.replace("Windows 10", "Windows 11") } else { product };
    let display = text("DisplayVersion").unwrap_or_default();
    let ubr = number("UBR").map(|u| format!(".{u}")).unwrap_or_default();
    format!("{product} {display} (build {build}{ubr})")
}
