//! Command line: `aula-f87-control [gui|status|set|sleep|music|repair] ...`.
//! Without a command it opens the window.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::about;
use crate::analyzer::{Analyzer, Follow, BASS_FFT_SIZE, RATE};
use crate::capture::Ring;
use crate::device::{self, Error, Frame, Keyboard, Lighting};
use crate::effects::{battery_text, effect_info, effects, fmt_g, SIDE_MODES};
use crate::music::{MusicParams, MusicSync, Sink};
use crate::render::{render, KeyBands, STYLE_NAMES};

const HELP: &str = "usage: aula-f87-control [--background] [--version] {gui,status,set,sleep,music,repair,setup} ...

Lighting control for the AULA F87 / F87 Pro keyboard.

commands:
  gui                 open the window (default)
  status              show the current lighting
  set                 change the lighting
                        --effect NAME|NUMBER   e.g. static, breathing, 3
                        --color RRGGBB
                        --rainbow
                        --brightness 0-4
                        --speed 0-4
                        --side 0-4             0 off, 1 rainbow wave, 2 rainbow together,
                                               3 solid, 4 breathing
  sleep WHEN          when the backlight turns off: 'never' or idle minutes (1, 5, 10, 30)
  music               make the backlight follow what's playing (Ctrl+C stops)
                        --style bars|meter|rainbow|pulse
                        --sensitivity 0.2-2    higher = taller bars
                        --brightness 0.1-1
                        --follow all|bass,vocals,beat
                        --seconds N            stop automatically after this long
  repair              put back the keyboard's last good settings
                      (if it stopped typing or its lights flash)
  setup               add the app-menu entry, icons and start at login for this user
                      (after `cargo install`); --remove undoes it, --udev-rule prints the rule

options:
  --background        start hidden with only the top-bar icon (used at login)
  --version           print the version
  -h, --help          show this help";

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

fn usage(msg: &str) -> i32 {
    eprintln!("aula-f87-control: error: {msg}");
    2
}

fn effect_id(text: &str) -> Option<u8> {
    if let Ok(n) = text.parse::<u8>() {
        return Some(n);
    }
    let t = text.to_lowercase();
    effects().into_iter().find(|e| e.name.to_lowercase().starts_with(&t)).map(|e| e.id)
}

fn color(text: &str) -> Option<[u8; 3]> {
    let t = text.trim_start_matches('#');
    if t.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(t, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn level(text: &str) -> Option<u8> {
    text.parse::<u8>().ok().filter(|v| *v <= 4)
}

fn sleep_text(minutes: f64) -> String {
    if minutes == 0.0 { "never (backlight always on)".into() } else { format!("after {} min idle", fmt_g(minutes)) }
}

fn print_state(kb: &Keyboard, state: &Lighting) {
    let info = effect_info(state.effect);
    let color = if state.colorful {
        "rainbow".to_string()
    } else {
        format!("#{:02x}{:02x}{:02x}", state.color[0], state.color[1], state.color[2])
    };
    println!("Keyboard:   {}", kb.description());
    println!("Effect:     {} ({})", info.name, state.effect);
    println!("Color:      {color}");
    println!("Brightness: {}/4   Speed: {}/4", state.brightness, state.speed);
    let side = SIDE_MODES.get(state.side as usize).map(|s| s.to_string()).unwrap_or(state.side.to_string());
    println!("Side lights: {side}");
    println!("Sleep:      {}", sleep_text(state.sleep));
    let battery = kb.read_battery();
    println!("Battery:    {}", battery_text(battery.as_ref(), kb.wired(), kb.last_battery()));
}

struct MusicArgs {
    params: MusicParams,
    seconds: Option<f64>,
    output: Option<String>,
    fps: Option<f64>,
    few_colors: bool,
    capture_cmd: Option<Vec<String>>,
    repeat: usize,
    file: Option<String>,
}

fn parse_music(args: &[String]) -> Result<MusicArgs, String> {
    let mut m = MusicArgs {
        params: MusicParams::default(),
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
        let mut value = || it.next().cloned().ok_or(format!("argument {arg}: expected one argument"));
        let num = |v: String| v.parse::<f64>().map_err(|_| format!("invalid number: {v}"));
        match arg.as_str() {
            "--style" => {
                let v = value()?;
                m.params.style = STYLE_NAMES
                    .iter()
                    .position(|s| *s == v)
                    .ok_or(format!("argument --style: invalid choice: '{v}' (choose from bars, meter, rainbow, pulse)"))?;
            }
            "--follow" => {
                let mut f = Follow::default();
                let mut unknown = vec![];
                for part in value()?.split(',').map(|p| p.trim().to_lowercase()).filter(|p| !p.is_empty()) {
                    match part.as_str() {
                        "all" => {}
                        "bass" => f.bass = true,
                        "vocals" => f.vocals = true,
                        "beat" => f.beat = true,
                        _ => unknown.push(part),
                    }
                }
                if !unknown.is_empty() {
                    return Err(format!("Unknown --follow value: {} (use all, bass, vocals, beat)", unknown.join(", ")));
                }
                m.params.follow = f;
            }
            "--sensitivity" => m.params.sensitivity = num(value()?)?.clamp(0.2, 2.0),
            "--brightness" => m.params.brightness = num(value()?)?.clamp(0.1, 1.0),
            "--seconds" => m.seconds = Some(num(value()?)?),
            // for measurements (see bench/)
            "--output" => m.output = Some(value()?),
            "--fps" => m.fps = Some(num(value()?)?),
            "--few-colors" => m.few_colors = true,
            "--repeat" => m.repeat = num(value()?)? as usize,
            "--capture-cmd" => m.capture_cmd = Some(value()?.split_whitespace().map(String::from).collect()),
            f if !f.starts_with('-') && m.file.is_none() => m.file = Some(f.to_string()),
            other => return Err(format!("unrecognized arguments: {other}")),
        }
    }
    Ok(m)
}

pub fn main(args: Vec<String>) -> i32 {
    let mut background = false;
    let mut rest: Vec<String> = vec![];
    for a in args {
        match a.as_str() {
            "--background" => background = true,
            "--version" => {
                println!("aula-f87-control {}", about::VERSION);
                return 0;
            }
            "-h" | "--help" if rest.is_empty() => {
                println!("{HELP}");
                return 0;
            }
            _ => rest.push(a),
        }
    }
    let cmd = rest.first().cloned().unwrap_or_else(|| "gui".into());
    let opts = if rest.is_empty() { vec![] } else { rest[1..].to_vec() };
    if opts.iter().any(|a| a == "-h" || a == "--help") {
        println!("{HELP}");
        return 0;
    }
    match cmd.as_str() {
        "gui" => gui(background),
        "status" | "set" | "sleep" | "repair" => lighting_command(&cmd, &opts),
        "setup" if opts.iter().any(|o| o == "--udev-rule") => {
            print!("{}", crate::setup::UDEV_RULE);
            0
        }
        "setup" => match crate::setup::run(opts.iter().any(|o| o == "--remove")) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("Error: {e}");
                1
            }
        },
        "music" => match parse_music(&opts) {
            Ok(m) => music(m),
            Err(e) => usage(&e),
        },
        "render-file" | "bench" | "levels-file" => match parse_music(&opts) {
            Ok(m) => offline(&cmd, m),
            Err(e) => usage(&e),
        },
        other => usage(&format!("argument cmd: invalid choice: '{other}'")),
    }
}

#[cfg(feature = "gui")]
fn gui(background: bool) -> i32 {
    crate::gui::main(background)
}

#[cfg(not(feature = "gui"))]
fn gui(_background: bool) -> i32 {
    eprintln!("This build has no window (built without the \"gui\" feature). Use the commands; see --help.");
    1
}

fn lighting_command(cmd: &str, opts: &[String]) -> i32 {
    // Parse everything before touching the keyboard.
    let mut changes: Vec<Box<dyn Fn(&mut Lighting)>> = vec![];
    let mut sleep_minutes = None;
    if cmd == "sleep" {
        let Some(when) = opts.first() else { return usage("the following arguments are required: when") };
        sleep_minutes = Some(match when.to_lowercase().as_str() {
            "never" | "off" | "0" => 0.0,
            w => match w.parse::<f64>() {
                Ok(v) => v,
                Err(_) => {
                    eprintln!("Use 'never' or a number of minutes.");
                    return 2;
                }
            },
        });
    }
    if cmd == "set" {
        let mut it = opts.iter();
        while let Some(arg) = it.next() {
            if arg == "--rainbow" {
                continue; // applied last, see below
            }
            let Some(v) = it.next() else { return usage(&format!("argument {arg}: expected one argument")) };
            match arg.as_str() {
                "--effect" => match effect_id(v) {
                    Some(id) => changes.push(Box::new(move |s| s.effect = id)),
                    None => return usage(&format!("argument --effect: unknown effect '{v}'")),
                },
                "--color" => match color(v) {
                    Some(c) => changes.push(Box::new(move |s| {
                        s.color = c;
                        s.colorful = false;
                    })),
                    None => return usage("argument --color: use RRGGBB"),
                },
                "--brightness" => match level(v) {
                    Some(n) => changes.push(Box::new(move |s| s.brightness = n)),
                    None => return usage(&format!("argument --brightness: invalid choice: {v} (choose from 0-4)")),
                },
                "--speed" => match level(v) {
                    Some(n) => changes.push(Box::new(move |s| s.speed = n)),
                    None => return usage(&format!("argument --speed: invalid choice: {v} (choose from 0-4)")),
                },
                "--side" => match level(v) {
                    Some(n) => changes.push(Box::new(move |s| s.side = n)),
                    None => return usage(&format!("argument --side: invalid choice: {v} (choose from 0-4)")),
                },
                other => return usage(&format!("unrecognized arguments: {other}")),
            }
        }
        // --rainbow wins over --color, like the Python app
        if opts.iter().any(|a| a == "--rainbow") {
            changes.push(Box::new(|s| s.colorful = true));
        }
    }
    let kb = match device::connect() {
        Ok(kb) => kb,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let result = (|| -> device::Result<()> {
        if cmd == "repair" {
            let used = kb.repair()?;
            println!("Put back the keyboard's {used}.");
        }
        let mut state = kb.read_lighting()?;
        if let Some(minutes) = sleep_minutes {
            kb.set_sleep(minutes)?;
            state = kb.read_lighting()?;
        }
        if cmd == "set" {
            for change in &changes {
                change(&mut state);
            }
            kb.apply_lighting(&state)?;
            state = kb.read_lighting()?;
        }
        print_state(&kb, &state);
        Ok(())
    })();
    match result {
        Ok(()) => 0,
        Err(e @ Error::Damaged(_)) => {
            eprintln!("Error: {e}\nRun: aula-f87-control repair");
            1
        }
        Err(e) => {
            eprintln!("Error: {e}");
            1
        }
    }
}

fn music(m: MusicArgs) -> i32 {
    let sink = match m.output.as_deref() {
        None | Some("keyboard") => match device::connect() {
            Ok(kb) => Sink::Keyboard(Arc::new(kb)),
            Err(e) => {
                eprintln!("{e}");
                return 1;
            }
        },
        Some(spec) => {
            let Some(path) = spec.strip_prefix("log:") else { return usage(&format!("unknown --output {spec}")) };
            match File::create(path) {
                Ok(f) => Sink::Log(BufWriter::new(f), m.fps.unwrap_or(30.0), m.few_colors),
                Err(e) => {
                    eprintln!("{path}: {e}");
                    return 1;
                }
            }
        }
    };
    let description = match &sink {
        Sink::Keyboard(kb) => Some(kb.description()),
        Sink::Log(..) => None,
    };
    let mut sync = match MusicSync::start(sink, m.params, m.capture_cmd) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
    }
    if let Some(d) = description {
        println!("Music sync on {d}. Press Ctrl+C to stop.");
    }
    let start = Instant::now();
    while !STOP.load(Ordering::SeqCst) && sync.running() {
        if m.seconds.is_some_and(|s| start.elapsed().as_secs_f64() >= s) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let failure = sync.error();
    sync.stop();
    let elapsed = start.elapsed().as_secs_f64();
    let frames = sync.frames.load(Ordering::SeqCst);
    println!("Stopped after {elapsed:.0}s ({:.1} frames/s).", frames as f64 / elapsed.max(0.001));
    match failure {
        Some(e) => {
            eprintln!("Error: {e}");
            1
        }
        None => 0,
    }
}

// ------------------------------------------------------------------ measurements (bench/)

fn read_raw(path: &str) -> Result<Vec<[f32; 2]>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    Ok(bytes
        .chunks_exact(4)
        .map(|f| [i16::from_le_bytes([f[0], f[1]]) as f32 / 32768.0, i16::from_le_bytes([f[2], f[3]]) as f32 / 32768.0])
        .collect())
}

/// Offline: the analyser's column levels and pulse for every frame (for tuning).
fn levels_file(m: &MusicArgs, audio: &[[f32; 2]]) {
    let fps = m.fps.unwrap_or(30.0);
    let hop = (RATE / fps) as usize;
    let p = m.params;
    let mut analyzer = Analyzer::new();
    let mut ring = Ring::default();
    let mut samples = vec![[0f32; 2]; BASS_FFT_SIZE];
    let mut out = BufWriter::new(std::io::stdout().lock());
    for chunk in audio.chunks_exact(hop) {
        for s in chunk {
            ring.push(*s);
        }
        ring.copy_into(&mut samples);
        analyzer.update(&samples, 1.0 / fps, p.sensitivity, p.follow);
        let cells: Vec<String> = analyzer.levels.iter().chain([&analyzer.pulse]).map(|v| format!("{v:.4}")).collect();
        let _ = writeln!(out, "{}", cells.join(","));
    }
}

/// Offline: push 1/fps of audio per frame, analyse, render.
fn run_file(m: &MusicArgs, audio: &[[f32; 2]], keep: bool) -> (Vec<Frame>, Vec<f64>) {
    let fps = m.fps.unwrap_or(30.0);
    let hop = (RATE / fps) as usize;
    let p = m.params;
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
        analyzer.update(&samples, 1.0 / fps, p.sensitivity, p.follow);
        let colors = render(&analyzer, &bands, p.style, p.brightness, m.few_colors, i as f64 / fps);
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
    format!(
        "frames {} median {:.3} ms p99 {:.3} ms mean {:.3} ms max {:.3} ms",
        times.len(),
        pick(0.5),
        pick(0.99),
        mean,
        times[times.len() - 1] * 1e3
    )
}

fn offline(cmd: &str, m: MusicArgs) -> i32 {
    let audio = match m.file.as_deref().ok_or("give an audio file".to_string()).and_then(read_raw) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Error: {e}");
            return 1;
        }
    };
    if cmd == "levels-file" {
        levels_file(&m, &audio);
    } else if cmd == "render-file" {
        let (frames, mut times) = run_file(&m, &audio, true);
        let mut out = BufWriter::new(std::io::stdout().lock());
        for f in &frames {
            let hex: String = f.iter().flatten().map(|b| format!("{b:02x}")).collect();
            let _ = writeln!(out, "{hex}");
        }
        eprintln!("{}", stats(&mut times));
    } else {
        let mut all = vec![];
        for _ in 0..m.repeat.max(1) {
            all.extend(run_file(&m, &audio, false).1);
        }
        println!("rust {}", stats(&mut all));
    }
    0
}
