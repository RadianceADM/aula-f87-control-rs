//! aula-f87-rs: the music-sync part of AULA F87 Control, in Rust.
//!
//!   aula-f87-rs music [--style bars|meter|rainbow|pulse] [--follow all|bass,vocals,beat]
//!                     [--sensitivity 0.5-2] [--brightness 0.1-1] [--seconds N]
//!                     [--output keyboard|log:FILE] [--fps N] [--few-colors]
//!                     [--capture-cmd "program args..."]
//!   aula-f87-rs render-file AUDIO.raw [same style options] [--fps N] [--few-colors]
//!   aula-f87-rs bench AUDIO.raw [--repeat N] [same style options]
//!
//! AUDIO.raw is 48 kHz stereo s16le, as written by `pw-record --raw`.

mod analyzer;
mod capture;
mod hid;
mod layout;
mod render;

use std::fs::File;
use std::io::{BufWriter, Write};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use analyzer::{Analyzer, Follow, BASS_FFT_SIZE, RATE};
use capture::{Capture, Ring};
use hid::{DeviceLock, Link};
use render::{render, Frame, KeyBands, STYLE_NAMES};

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

fn monotonic_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

struct Options {
    style: usize,
    follow: Follow,
    sensitivity: f64,
    brightness: f64,
    seconds: Option<f64>,
    output: Option<String>,
    fps: Option<f64>,
    few_colors: bool,
    capture_cmd: Option<Vec<String>>,
    repeat: usize,
    file: Option<String>,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        style: 0,
        follow: Follow::default(),
        sensitivity: 1.0,
        brightness: 1.0,
        seconds: None,
        output: None,
        fps: None,
        few_colors: false,
        capture_cmd: None,
        repeat: 1,
        file: None,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || it.next().cloned().ok_or(format!("{arg} needs a value"));
        let num = |v: String| v.parse::<f64>().map_err(|_| format!("bad number: {v}"));
        match arg.as_str() {
            "--style" => {
                let v = value()?;
                o.style = STYLE_NAMES.iter().position(|s| *s == v).ok_or(format!("unknown style {v}"))?;
            }
            "--follow" => {
                for part in value()?.split(',').map(|p| p.trim().to_lowercase()) {
                    match part.as_str() {
                        "all" | "" => {}
                        "bass" => o.follow.bass = true,
                        "vocals" => o.follow.vocals = true,
                        "beat" => o.follow.beat = true,
                        other => return Err(format!("unknown --follow value {other}")),
                    }
                }
            }
            "--sensitivity" => o.sensitivity = num(value()?)?.clamp(0.5, 2.0),
            "--brightness" => o.brightness = num(value()?)?.clamp(0.1, 1.0),
            "--seconds" => o.seconds = Some(num(value()?)?),
            "--output" => o.output = Some(value()?),
            "--fps" => o.fps = Some(num(value()?)?),
            "--few-colors" => o.few_colors = true,
            "--repeat" => o.repeat = num(value()?)? as usize,
            "--capture-cmd" => {
                o.capture_cmd = Some(value()?.split_whitespace().map(String::from).collect())
            }
            f if !f.starts_with('-') && o.file.is_none() => o.file = Some(f.to_string()),
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(o)
}

enum Output {
    Keyboard(Link, #[allow(dead_code)] DeviceLock), // the lock is held while streaming
    Log(BufWriter<File>),
}

fn music(o: Options) -> Result<(), String> {
    let mut output = match o.output.as_deref() {
        None | Some("keyboard") => {
            // Same lock as the Python app: never stream while it talks to the keyboard.
            let lock = DeviceLock::acquire(Duration::from_secs(3))
                .ok_or("Another program is using the keyboard right now.")?;
            Output::Keyboard(Link::connect()?, lock)
        }
        Some(spec) => {
            let path = spec.strip_prefix("log:").ok_or(format!("unknown --output {spec}"))?;
            Output::Log(BufWriter::new(File::create(path).map_err(|e| e.to_string())?))
        }
    };
    let (fps, few_colors) = match &output {
        Output::Keyboard(link, _) => (o.fps.unwrap_or(link.fps()), link.few_colors()),
        Output::Log(_) => (o.fps.unwrap_or(30.0), o.few_colors),
    };
    let cmd = o.capture_cmd.clone().unwrap_or_else(capture::pw_record_command);
    let capture = Capture::start(&cmd).map_err(|e| format!("couldn't start {}: {e}", cmd[0]))?;
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
    }
    if let Output::Keyboard(link, _) = &output {
        eprintln!("Music sync on F87 · {}. Press Ctrl+C to stop.", link.label());
    }

    let mut analyzer = Analyzer::new();
    let bands = KeyBands::new();
    let mut samples = vec![[0f32; 2]; BASS_FFT_SIZE];
    let period = Duration::from_secs_f64(1.0 / fps);
    let start = Instant::now();
    let mut last = start;
    let mut frames = 0u64;
    let mut result = Ok(());
    while !STOP.load(Ordering::SeqCst) {
        if o.seconds.is_some_and(|s| start.elapsed().as_secs_f64() >= s) {
            break;
        }
        if !capture.alive() {
            result = Err("Audio capture stopped.".to_string());
            break;
        }
        let now = Instant::now();
        capture.samples(&mut samples);
        analyzer.update(&samples, (now - last).as_secs_f64(), o.sensitivity, o.follow);
        last = now;
        let colors = render(&analyzer, &bands, o.style, o.brightness, few_colors, (now - start).as_secs_f64());
        let sent = match &mut output {
            Output::Keyboard(link, _) => link.send_live(&colors).map_err(|e| e.to_string()),
            Output::Log(w) => {
                let lit = colors.iter().filter(|c| c.iter().any(|&v| v > 0)).count();
                writeln!(w, "{} {}", monotonic_ns(), lit).map_err(|e| e.to_string())
            }
        };
        if let Err(e) = sent {
            result = Err(e);
            break;
        }
        frames += 1;
        if let Some(rest) = period.checked_sub(now.elapsed()) {
            std::thread::sleep(rest);
        }
    }
    if let Output::Log(w) = &mut output {
        let _ = w.flush();
    }
    let elapsed = start.elapsed().as_secs_f64();
    eprintln!("Stopped after {elapsed:.0}s ({:.1} frames/s).", frames as f64 / elapsed.max(0.001));
    result
}

fn read_raw(path: &str) -> Result<Vec<[f32; 2]>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    Ok(bytes
        .chunks_exact(4)
        .map(|f| {
            [
                i16::from_le_bytes([f[0], f[1]]) as f32 / 32768.0,
                i16::from_le_bytes([f[2], f[3]]) as f32 / 32768.0,
            ]
        })
        .collect())
}

/// Offline: push 1/fps of audio per frame, analyse, render. Returns frames and
/// the time each update+render took.
fn run_file(o: &Options, audio: &[[f32; 2]], keep: bool) -> (Vec<Frame>, Vec<f64>) {
    let fps = o.fps.unwrap_or(30.0);
    let hop = (RATE / fps) as usize;
    let mut analyzer = Analyzer::new();
    let bands = KeyBands::new();
    let mut ring = Ring::default();
    let mut samples = vec![[0f32; 2]; BASS_FFT_SIZE];
    let (mut frames, mut times) = (vec![], vec![]);
    for (i, chunk) in audio.chunks_exact(hop).enumerate() {
        for s in chunk {
            ring.push(*s);
        }
        let t = Instant::now();
        ring.copy_into(&mut samples);
        analyzer.update(&samples, 1.0 / fps, o.sensitivity, o.follow);
        let colors = render(&analyzer, &bands, o.style, o.brightness, o.few_colors, i as f64 / fps);
        times.push(t.elapsed().as_secs_f64());
        if keep {
            frames.push(colors);
        }
    }
    (frames, times)
}

fn stats(times: &mut [f64]) -> String {
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pick = |q: f64| times[((times.len() - 1) as f64 * q) as usize] * 1e3;
    let mean = times.iter().sum::<f64>() / times.len() as f64 * 1e3;
    format!("frames {} median {:.3} ms p99 {:.3} ms mean {:.3} ms max {:.3} ms",
            times.len(), pick(0.5), pick(0.99), mean, times[times.len() - 1] * 1e3)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        eprintln!("usage: aula-f87-rs music|render-file|bench ... (see src/main.rs)");
        return ExitCode::from(2);
    };
    let o = match parse(&args[1..]) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let result = match cmd.as_str() {
        "music" => music(o),
        "render-file" | "bench" => (|| {
            let audio = read_raw(o.file.as_deref().ok_or("give an audio file")?)?;
            if cmd == "render-file" {
                let (frames, mut times) = run_file(&o, &audio, true);
                let mut out = BufWriter::new(std::io::stdout().lock());
                for f in &frames {
                    let hex: String = f.iter().flatten().map(|b| format!("{b:02x}")).collect();
                    writeln!(out, "{hex}").map_err(|e| e.to_string())?;
                }
                eprintln!("{}", stats(&mut times));
            } else {
                let mut all = vec![];
                for _ in 0..o.repeat.max(1) {
                    all.extend(run_file(&o, &audio, false).1);
                }
                println!("rust {}", stats(&mut all));
            }
            Ok(())
        })(),
        other => Err(format!("unknown command {other}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}
