//! Gets messages from background threads (keyboard jobs, music sync) to the
//! app on the UI thread, and gives it timers.
//!
//! Reactor's senders only work on the UI thread, so background threads push
//! onto an inbox and post a window message to a hidden message-only window
//! made on the UI thread. Its window procedure runs inside WinUI's own message
//! loop and hands each message to the app's sender. The same window gets the
//! app's timers (SetTimer) and the "show yourself" note from a second copy.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, FindWindowExW, HWND_MESSAGE, KillTimer, PostMessageW, RegisterClassW, SetTimer,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_TIMER, WNDCLASSW,
};
use windows::core::{PCWSTR, w};

use crate::app::Msg;

const CLASS: PCWSTR = w!("AulaF87ControlBridge");
const WM_INBOX: u32 = WM_APP + 1;
const WM_SHOW: u32 = WM_APP + 2;

static INBOX: Mutex<VecDeque<Msg>> = Mutex::new(VecDeque::new());
static WINDOW: AtomicIsize = AtomicIsize::new(0);

thread_local! {
    static SINK: RefCell<Option<Box<dyn Fn(Msg)>>> = RefCell::new(None);
}

fn hwnd() -> Option<HWND> {
    match WINDOW.load(Ordering::SeqCst) {
        0 => None,
        h => Some(HWND(h as *mut _)),
    }
}

/// Call once, on the UI thread, with the app's sender.
pub fn install(sink: impl Fn(Msg) + 'static) {
    SINK.with(|s| *s.borrow_mut() = Some(Box::new(sink)));
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let class = WNDCLASSW { lpfnWndProc: Some(window_proc), hInstance: instance.into(), lpszClassName: CLASS, ..Default::default() };
        RegisterClassW(&class);
        if let Ok(h) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS,
            w!("AULA F87 Control"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance.into()),
            None,
        ) {
            WINDOW.store(h.0 as isize, Ordering::SeqCst);
        }
    }
    // Anything posted before the window existed.
    if let Some(h) = hwnd() {
        unsafe {
            let _ = PostMessageW(Some(h), WM_INBOX, WPARAM(0), LPARAM(0));
        }
    }
}

/// From any thread.
pub fn post(msg: Msg) {
    INBOX.lock().unwrap().push_back(msg);
    if let Some(h) = hwnd() {
        unsafe {
            let _ = PostMessageW(Some(h), WM_INBOX, WPARAM(0), LPARAM(0));
        }
    }
}

/// (Re)starts timer `id`; it fires `Msg::Timer(id)` every `ms` until killed.
pub fn set_timer(id: usize, ms: u32) {
    if let Some(h) = hwnd() {
        unsafe {
            SetTimer(Some(h), id, ms, None);
        }
    }
}

pub fn kill_timer(id: usize) {
    if let Some(h) = hwnd() {
        unsafe {
            let _ = KillTimer(Some(h), id);
        }
    }
}

/// If another copy is already running, ask it to show its window. Returns true
/// when one was found (this copy should then exit).
pub fn wake_running_copy() -> bool {
    unsafe {
        match FindWindowExW(Some(HWND_MESSAGE), None, CLASS, PCWSTR::null()) {
            Ok(h) if !h.is_invalid() => {
                let _ = PostMessageW(Some(h), WM_SHOW, WPARAM(0), LPARAM(0));
                true
            }
            _ => false,
        }
    }
}

fn deliver(msg: Msg) {
    SINK.with(|s| {
        if let Some(sink) = s.borrow().as_ref() {
            sink(msg);
        }
    });
}

extern "system" fn window_proc(h: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_INBOX => {
            loop {
                let next = INBOX.lock().unwrap().pop_front();
                match next {
                    Some(m) => deliver(m),
                    None => break,
                }
            }
            LRESULT(0)
        }
        WM_TIMER => {
            deliver(Msg::Timer(wparam.0));
            LRESULT(0)
        }
        WM_SHOW => {
            deliver(Msg::ShowWindow);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(h, msg, wparam, lparam) },
    }
}
