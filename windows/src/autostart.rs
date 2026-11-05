//! Start at login: a value under HKCU\...\Run that starts the app in the tray.
//! Like the Linux app, it turns itself on the first time the app runs.

use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegDeleteValueW, RegOpenKeyExW,
    RegQueryValueExW, RegSetValueExW,
};
use windows::core::{PCWSTR, w};

const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const NAME: PCWSTR = w!("AULA F87 Control");

fn open(access: windows::Win32::System::Registry::REG_SAM_FLAGS) -> Option<HKEY> {
    let mut key = HKEY::default();
    let r = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, RUN, None, access, &mut key) };
    (r == ERROR_SUCCESS).then_some(key)
}

fn command() -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    format!("\"{}\" --background", exe.display())
}

pub fn is_enabled() -> bool {
    let Some(key) = open(KEY_READ) else { return false };
    let r = unsafe { RegQueryValueExW(key, NAME, None, None, None, None) };
    unsafe {
        let _ = RegCloseKey(key);
    }
    r == ERROR_SUCCESS
}

pub fn enable() -> bool {
    let Some(key) = open(KEY_SET_VALUE) else { return false };
    let data: Vec<u16> = std::ffi::OsStr::new(&command()).encode_wide().chain(Some(0)).collect();
    let bytes = unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2) };
    let r = unsafe { RegSetValueExW(key, NAME, None, REG_SZ, Some(bytes)) };
    unsafe {
        let _ = RegCloseKey(key);
    }
    r == ERROR_SUCCESS
}

pub fn disable() -> bool {
    let Some(key) = open(KEY_SET_VALUE) else { return false };
    let r = unsafe { RegDeleteValueW(key, NAME) };
    unsafe {
        let _ = RegCloseKey(key);
    }
    r == ERROR_SUCCESS
}

fn marker() -> PathBuf {
    aula_f87::device::config_dir().join("autostart-offered")
}

/// First run: turn start-at-login on (the user can turn it off in the tray menu).
pub fn enable_on_first_run() {
    let m = marker();
    if m.exists() {
        return;
    }
    if enable() {
        let _ = std::fs::create_dir_all(m.parent().unwrap());
        let _ = std::fs::write(&m, b"1\n");
    }
}

/// Keep the Run value pointing at this copy (after an update moved the exe).
pub fn refresh() {
    if is_enabled() {
        enable();
    }
}
