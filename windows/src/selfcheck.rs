//! Checks that run without the window, for testing over SSH:
//!   --self-check          find the keyboard and read its settings (reads only)
//!   --loopback-test N     listen to the speaker for N seconds, print the level
//!   --music-test N        music sync to the keyboard for N seconds (live frames, nothing stored)

use std::time::{Duration, Instant};

use aula_f87::capture::Capture;
use aula_f87::device::{self, DONGLE_ID, WIRED_ID};
use aula_f87::effects::{battery_text, effect_info};
use aula_f87::music::{MusicParams, MusicSync, Sink};

fn seconds(args: &[String], i: usize) -> u64 {
    args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(5)
}

pub fn run(args: &[String]) -> Option<i32> {
    let i = args.iter().position(|a| a.starts_with("--self-check") || a.ends_with("-test") || a == "--version")?;
    Some(match args[i].as_str() {
        "--self-check" => self_check(),
        "--loopback-test" => loopback_test(seconds(args, i)),
        "--music-test" => music_test(seconds(args, i)),
        "--live-test" => live_test(),
        "--version" => {
            println!("{} {}", aula_f87::about::NAME, env!("CARGO_PKG_VERSION"));
            0
        }
        other => {
            println!("unknown option {other}");
            2
        }
    })
}

fn self_check() -> i32 {
    println!("{} {} (Windows)", aula_f87::about::NAME, env!("CARGO_PKG_VERSION"));
    println!("system: {}", crate::sysinfo::windows_version());
    for (name, id) in [("cable", WIRED_ID), ("receiver", DONGLE_ID)] {
        for c in aula_f87::hidraw::describe(id) {
            println!("hid {name}: {c}");
        }
        for p in aula_f87::hidraw::candidates(id, if name == "cable" { 0x06 } else { 0x13 }) {
            println!("candidate {name}: {}", p.display());
        }
    }
    let kb = match device::connect() {
        Ok(kb) => kb,
        Err(e) => {
            println!("connect: FAILED: {e}");
            return 1;
        }
    };
    println!("keyboard: {}", kb.description());
    match kb.read_lighting() {
        Ok(l) => println!(
            "lighting: effect {} ({}), brightness {}, speed {}, rainbow {}, color {:?}, side {}, sleep {} min",
            l.effect,
            effect_info(l.effect).name,
            l.brightness,
            l.speed,
            l.colorful,
            l.color,
            l.side,
            l.sleep
        ),
        Err(e) => {
            println!("read_lighting: FAILED: {e}");
            return 1;
        }
    }
    match kb.read_keys() {
        Ok(keys) => println!("per-key map: {} keys lit", keys.iter().filter(|c| c.iter().any(|&v| v > 0)).count()),
        Err(e) => println!("read_keys: FAILED: {e}"),
    }
    println!("battery: {}", battery_text(kb.read_battery().as_ref(), kb.wired(), kb.last_battery()));
    println!("alive: {}", kb.alive());
    0
}

fn loopback_test(secs: u64) -> i32 {
    let capture = match Capture::loopback() {
        Ok(c) => c,
        Err(e) => {
            println!("loopback: FAILED: {e}");
            return 1;
        }
    };
    let mut buf = vec![[0f32; 2]; aula_f87::analyzer::BASS_FFT_SIZE];
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(secs) {
        std::thread::sleep(Duration::from_secs(1));
        capture.samples(&mut buf);
        let rms = (buf.iter().map(|s| (s[0] * s[0] + s[1] * s[1]) as f64 / 2.0).sum::<f64>() / buf.len() as f64).sqrt();
        let peak = buf.iter().map(|s| s[0].abs().max(s[1].abs())).fold(0f32, f32::max);
        println!("t={:>2}s rms {:.4} peak {:.3} alive {}", start.elapsed().as_secs(), rms, peak, capture.alive());
    }
    0
}

fn music_test(secs: u64) -> i32 {
    let kb = match device::connect() {
        Ok(kb) => std::sync::Arc::new(kb),
        Err(e) => {
            println!("connect: FAILED: {e}");
            return 1;
        }
    };
    println!("keyboard: {}", kb.description());
    let mut sync = match MusicSync::start(Sink::Keyboard(kb), MusicParams::default(), None) {
        Ok(s) => s,
        Err(e) => {
            println!("music sync: FAILED: {e}");
            return 1;
        }
    };
    let start = Instant::now();
    let mut last = 0;
    while start.elapsed() < Duration::from_secs(secs) && sync.running() {
        std::thread::sleep(Duration::from_secs(1));
        let frames = sync.frames.load(std::sync::atomic::Ordering::SeqCst);
        let lit = sync.latest.lock().unwrap().iter().filter(|c| c.iter().any(|&v| v > 0)).count();
        println!("t={:>2}s frames/s {} lit keys {}", start.elapsed().as_secs(), frames - last, lit);
        last = frames;
    }
    let err = sync.error();
    sync.stop();
    match err {
        Some(e) => {
            println!("music sync stopped: {e}");
            1
        }
        None => 0,
    }
}

/// How long one live frame takes to send (30 dim frames, nothing stored).
fn live_test() -> i32 {
    let kb = match device::connect() {
        Ok(kb) => kb,
        Err(e) => {
            println!("connect: FAILED: {e}");
            return 1;
        }
    };
    let frame = [[40u8, 40, 40]; aula_f87::layout::PLANE];
    let mut times = vec![];
    for _ in 0..30 {
        let t = Instant::now();
        if let Err(e) = kb.send_live(&frame) {
            println!("send_live: FAILED: {e}");
            return 1;
        }
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let avg = times.iter().sum::<f64>() / times.len() as f64;
    println!("send_live ms: min {:.1} median {:.1} avg {:.1} max {:.1}", times[0], times[15], avg, times[29]);
    let t = Instant::now();
    std::thread::sleep(Duration::from_millis(5));
    println!("sleep(5ms) took {:.2} ms", t.elapsed().as_secs_f64() * 1000.0);
    0
}
