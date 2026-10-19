//! Make the backlight follow whatever the computer is playing.
//!
//! Audio comes from the default output's monitor ("what you hear") through
//! PipeWire's `pw-record`, so it works with any player. Frames are sent as live,
//! unsaved frames; the keyboard returns to its stored lighting by itself shortly
//! after they stop.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::analyzer::{Analyzer, Follow, BASS_FFT_SIZE};
use crate::capture::{self, Capture};
use crate::device::{DeviceLock, Error, Frame, Keyboard};
use crate::layout::PLANE;
use crate::render::{render, KeyBands};

#[derive(Clone, Copy, Debug)]
pub struct MusicParams {
    pub style: usize,
    pub sensitivity: f64,
    pub brightness: f64,
    pub follow: Follow,
}

impl Default for MusicParams {
    fn default() -> Self {
        MusicParams { style: 0, sensitivity: 1.0, brightness: 1.0, follow: Follow::default() }
    }
}

/// Why music sync stopped. `device` is true when talking to the keyboard
/// failed (e.g. it was unplugged), as opposed to audio problems.
#[derive(Clone, Debug)]
pub struct MusicError {
    pub message: String,
    pub device: bool,
}

impl std::fmt::Display for MusicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Where frames go: the keyboard, or (for measurements) a log of
/// "monotonic_ns lit_keys" lines.
pub enum Sink {
    Keyboard(Arc<Keyboard>),
    Log(BufWriter<File>, f64, bool), // writer, fps, few colours
}

pub fn monotonic_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

/// Background loop: capture -> analyse -> render -> send live frames.
pub struct MusicSync {
    pub params: Arc<Mutex<MusicParams>>,
    pub frames: Arc<AtomicU64>,
    /// The latest frame, for the on-screen preview.
    pub latest: Arc<Mutex<Frame>>,
    error: Arc<Mutex<Option<MusicError>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MusicSync {
    pub fn start(sink: Sink, params: MusicParams, capture_cmd: Option<Vec<String>>) -> Result<MusicSync, MusicError> {
        // Held while streaming, so nothing writes the keyboard's settings
        // (another copy of the app, the command line) in the middle of it.
        let lock = match &sink {
            Sink::Keyboard(_) => Some(DeviceLock::try_acquire(Duration::from_secs(3)).ok_or(MusicError {
                message: "Another program is using the keyboard right now.".into(),
                device: false,
            })?),
            Sink::Log(..) => None,
        };
        let cmd = capture_cmd.unwrap_or_else(capture::pw_record_command);
        let capture = Capture::start(&cmd).map_err(|e| MusicError {
            message: if e.kind() == std::io::ErrorKind::NotFound && cmd[0] == "pw-record" {
                "pw-record was not found. Install it with: sudo apt install pipewire-bin".into()
            } else {
                format!("Couldn't start {}: {e}", cmd[0])
            },
            device: false,
        })?;
        let params = Arc::new(Mutex::new(params));
        let frames = Arc::new(AtomicU64::new(0));
        let latest = Arc::new(Mutex::new([[0u8; 3]; PLANE]));
        let error = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let (p, f, l, e, s) = (params.clone(), frames.clone(), latest.clone(), error.clone(), stop.clone());
        let thread = thread::spawn(move || {
            let _lock = lock;
            if let Err(err) = run(sink, capture, &p, &f, &l, &s) {
                *e.lock().unwrap() = Some(err);
            }
        });
        Ok(MusicSync { params, frames, latest, error, stop, thread: Some(thread) })
    }

    pub fn running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished()) && !self.stop.load(Ordering::SeqCst)
    }

    /// Set when the loop stopped by itself.
    pub fn error(&self) -> Option<MusicError> {
        self.error.lock().unwrap().clone()
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for MusicSync {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run(
    mut sink: Sink,
    capture: Capture,
    params: &Mutex<MusicParams>,
    frames: &AtomicU64,
    latest: &Mutex<Frame>,
    stop: &AtomicBool,
) -> Result<(), MusicError> {
    let (fps, few_colors) = match &sink {
        Sink::Keyboard(kb) => (kb.live_fps, kb.live_few_colors),
        Sink::Log(_, fps, few) => (*fps, *few),
    };
    let mut analyzer = Analyzer::new();
    let bands = KeyBands::new();
    let mut samples = vec![[0f32; 2]; BASS_FFT_SIZE];
    let period = Duration::from_secs_f64(1.0 / fps);
    let start = Instant::now();
    let mut last = start;
    while !stop.load(Ordering::SeqCst) {
        if !capture.alive() {
            return Err(MusicError { message: "Audio capture stopped.".into(), device: false });
        }
        let now = Instant::now();
        let p = *params.lock().unwrap();
        capture.samples(&mut samples);
        analyzer.update(&samples, (now - last).as_secs_f64(), p.sensitivity, p.follow);
        last = now;
        let colors = render(&analyzer, &bands, p.style, p.brightness, few_colors, (now - start).as_secs_f64());
        match &mut sink {
            Sink::Keyboard(kb) => kb.send_live(&colors).map_err(|e| MusicError {
                device: matches!(e, Error::Io(_)),
                message: e.to_string(),
            })?,
            Sink::Log(w, ..) => {
                let lit = colors.iter().filter(|c| c.iter().any(|&v| v > 0)).count();
                let _ = writeln!(w, "{} {}", monotonic_ns(), lit);
            }
        }
        *latest.lock().unwrap() = colors;
        frames.fetch_add(1, Ordering::SeqCst);
        if let Some(rest) = period.checked_sub(now.elapsed()) {
            // sleep in short steps so stop() is quick
            let until = Instant::now() + rest;
            while !stop.load(Ordering::SeqCst) && Instant::now() < until {
                thread::sleep(until.saturating_duration_since(Instant::now()).min(Duration::from_millis(20)));
            }
        }
    }
    if let Sink::Log(w, ..) = &mut sink {
        let _ = w.flush();
    }
    Ok(())
}
