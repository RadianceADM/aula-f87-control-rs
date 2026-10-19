//! Audio capture: runs `pw-record` (the same command as the Python app) and
//! keeps the most recent BASS_FFT_SIZE stereo samples.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::analyzer::BASS_FFT_SIZE;

pub const BLOCK: usize = 1024; // frames per read, like the Python app

pub fn pw_record_command() -> Vec<String> {
    [
        "pw-record",
        "-P",
        "{ stream.capture.sink=true node.name=aula-f87-music node.description=\"AULA F87 music sync\" }",
        "--raw", "--rate", "48000", "--channels", "2", "--format", "s16",
        "--latency", "20ms", "-",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Ring buffer of the latest samples.
pub struct Ring {
    data: Vec<[f32; 2]>,
    pos: usize, // index of the oldest sample
}

impl Default for Ring {
    fn default() -> Self {
        Ring { data: vec![[0.0; 2]; BASS_FFT_SIZE], pos: 0 }
    }
}

impl Ring {
    pub fn push(&mut self, sample: [f32; 2]) {
        self.data[self.pos] = sample;
        self.pos = (self.pos + 1) % self.data.len();
    }

    /// Oldest first.
    pub fn copy_into(&self, out: &mut [[f32; 2]]) {
        let (newer, older) = self.data.split_at(self.pos);
        out[..older.len()].copy_from_slice(older);
        out[older.len()..].copy_from_slice(newer);
    }
}

pub struct Capture {
    ring: Arc<Mutex<Ring>>,
    alive: Arc<AtomicBool>,
    child: Child,
}

impl Capture {
    pub fn start(cmd: &[String]) -> std::io::Result<Capture> {
        let mut child = Command::new(&cmd[0])
            .args(&cmd[1..])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut stdout = child.stdout.take().expect("piped");
        let ring = Arc::new(Mutex::new(Ring::default()));
        let alive = Arc::new(AtomicBool::new(true));
        let (r, a) = (ring.clone(), alive.clone());
        thread::spawn(move || {
            let mut buf = [0u8; BLOCK * 4];
            let mut pending: Vec<u8> = Vec::with_capacity(BLOCK * 8);
            loop {
                match stdout.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        pending.extend_from_slice(&buf[..n]);
                        let usable = pending.len() - pending.len() % 4;
                        let mut ring = r.lock().unwrap();
                        for f in pending[..usable].chunks_exact(4) {
                            let l = i16::from_le_bytes([f[0], f[1]]) as f32 / 32768.0;
                            let rr = i16::from_le_bytes([f[2], f[3]]) as f32 / 32768.0;
                            ring.push([l, rr]);
                        }
                        drop(ring);
                        pending.drain(..usable);
                    }
                }
            }
            a.store(false, Ordering::SeqCst);
        });
        Ok(Capture { ring, alive, child })
    }

    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    pub fn samples(&self, out: &mut [[f32; 2]]) {
        self.ring.lock().unwrap().copy_into(out);
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
