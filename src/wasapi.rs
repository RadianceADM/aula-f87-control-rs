//! Windows audio capture: WASAPI loopback of the default speaker ("what you
//! hear"), so music sync follows any player. The stream is asked for 48 kHz
//! stereo float (Windows converts); if a device refuses that, its own mix
//! format is converted here. Follows the default speaker when it changes, and
//! feeds silence while nothing plays (loopback delivers no packets then) or
//! while there is no speaker at all.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
};
use windows::Win32::Media::KernelStreaming::{KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE};
use windows::Win32::Media::Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED};

use crate::analyzer::RATE;
use crate::capture::Ring;

const BUFFER_100NS: i64 = 2_000_000; // 200 ms shared buffer
const POLL: Duration = Duration::from_millis(10);
const SILENCE_AFTER: Duration = Duration::from_millis(40);

/// How the device's samples are laid out.
#[derive(Clone, Copy, Debug)]
enum Sample {
    F32,
    I16,
    I32,
}

#[derive(Clone, Copy, Debug)]
struct Layout {
    sample: Sample,
    channels: usize,
    rate: f64,
}

/// Turns device frames into 48 kHz stereo (nearest-sample resampling is
/// plenty for band levels).
struct Converter {
    layout: Layout,
    step: f64, // device frames per output frame
    pos: f64,
}

impl Converter {
    fn new(layout: Layout) -> Converter {
        Converter { layout, step: layout.rate / RATE, pos: 0.0 }
    }

    unsafe fn frame(&self, data: *const u8, i: usize) -> [f32; 2] {
        let ch = self.layout.channels;
        let get = |c: usize| -> f32 {
            let idx = i * ch + c.min(ch - 1);
            match self.layout.sample {
                Sample::F32 => *(data as *const f32).add(idx),
                Sample::I16 => *(data as *const i16).add(idx) as f32 / 32768.0,
                Sample::I32 => *(data as *const i32).add(idx) as f32 / 2_147_483_648.0,
            }
        };
        [get(0), get(1)]
    }

    unsafe fn push(&mut self, ring: &mut Ring, data: *const u8, frames: usize) {
        if (self.step - 1.0).abs() < 1e-9 {
            for i in 0..frames {
                ring.push(self.frame(data, i));
            }
            return;
        }
        while self.pos < frames as f64 {
            ring.push(self.frame(data, self.pos as usize));
            self.pos += self.step;
        }
        self.pos -= frames as f64;
    }
}

struct Stream {
    _client: IAudioClient,
    capture: IAudioCaptureClient,
    converter: Converter,
    id: String,
}

fn default_id(enumerator: &IMMDeviceEnumerator) -> Option<String> {
    unsafe {
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
        let id = device.GetId().ok()?;
        let s = id.to_string().ok();
        CoTaskMemFree(Some(id.0 as *const _));
        s
    }
}

fn layout_of(format: *const WAVEFORMATEX) -> Option<Layout> {
    let f = unsafe { &*format };
    let channels = f.nChannels as usize;
    if channels == 0 {
        return None;
    }
    let tag = f.wFormatTag as u32;
    let sample = if tag == WAVE_FORMAT_IEEE_FLOAT && f.wBitsPerSample == 32 {
        Sample::F32
    } else if tag == WAVE_FORMAT_EXTENSIBLE {
        let ext = unsafe { &*(format as *const WAVEFORMATEXTENSIBLE) };
        let sub = ext.SubFormat;
        match (sub, f.wBitsPerSample) {
            (s, 32) if s == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT => Sample::F32,
            (s, 16) if s == KSDATAFORMAT_SUBTYPE_PCM => Sample::I16,
            (s, 32) if s == KSDATAFORMAT_SUBTYPE_PCM => Sample::I32,
            _ => return None,
        }
    } else if tag == 1 && f.wBitsPerSample == 16 {
        Sample::I16
    } else {
        return None;
    };
    Some(Layout { sample, channels, rate: f.nSamplesPerSec as f64 })
}

fn open(enumerator: &IMMDeviceEnumerator) -> windows::core::Result<Stream> {
    unsafe {
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let id = {
            let raw = device.GetId()?;
            let s = raw.to_string().unwrap_or_default();
            CoTaskMemFree(Some(raw.0 as *const _));
            s
        };
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        let wanted = WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_IEEE_FLOAT as u16,
            nChannels: 2,
            nSamplesPerSec: RATE as u32,
            nAvgBytesPerSec: RATE as u32 * 8,
            nBlockAlign: 8,
            wBitsPerSample: 32,
            cbSize: 0,
        };
        let converted = client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            BUFFER_100NS,
            0,
            &wanted,
            None,
        );
        let (client, layout) = match converted {
            Ok(()) => (client, Layout { sample: Sample::F32, channels: 2, rate: RATE }),
            Err(_) => {
                // A fresh client in the device's own mix format.
                let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
                let mix = client.GetMixFormat()?;
                let layout = layout_of(mix);
                let r = client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, BUFFER_100NS, 0, mix, None);
                CoTaskMemFree(Some(mix as *const _));
                r?;
                let layout = layout.ok_or_else(|| windows::core::Error::from_hresult(windows::core::HRESULT(0x8889_0008u32 as i32)))?;
                (client, layout)
            }
        };
        let capture: IAudioCaptureClient = client.GetService()?;
        client.Start()?;
        Ok(Stream { _client: client, capture, converter: Converter::new(layout), id })
    }
}

/// Drain every waiting packet into the ring. Returns how many frames arrived.
fn drain(stream: &mut Stream, ring: &Mutex<Ring>) -> windows::core::Result<usize> {
    let mut total = 0;
    unsafe {
        let mut packet = stream.capture.GetNextPacketSize()?;
        while packet > 0 {
            let mut data: *mut u8 = std::ptr::null_mut();
            let mut frames = 0u32;
            let mut flags = 0u32;
            stream.capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
            {
                let mut ring = ring.lock().unwrap();
                if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                    let out = (frames as f64 / stream.converter.step) as usize;
                    for _ in 0..out {
                        ring.push([0.0; 2]);
                    }
                } else {
                    stream.converter.push(&mut ring, data, frames as usize);
                }
            }
            stream.capture.ReleaseBuffer(frames)?;
            total += frames as usize;
            packet = stream.capture.GetNextPacketSize()?;
        }
    }
    Ok(total)
}

/// Push silence for the time that passed without packets.
fn fill_silence(ring: &Mutex<Ring>, since: &mut Instant) {
    let frames = (since.elapsed().as_secs_f64() * RATE) as usize;
    if frames == 0 {
        return;
    }
    *since = Instant::now();
    let mut ring = ring.lock().unwrap();
    for _ in 0..frames.min(crate::analyzer::BASS_FFT_SIZE) {
        ring.push([0.0; 2]);
    }
}

/// The capture loop; runs on its own thread until `stop` is set.
pub fn run(ring: Arc<Mutex<Ring>>, stop: Arc<AtomicBool>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let enumerator: IMMDeviceEnumerator = match unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) } {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut stream: Option<Stream> = None;
    let mut last_audio = Instant::now();
    let mut silence_mark = Instant::now();
    let mut last_check = Instant::now();
    let mut retry_at = Instant::now();
    while !stop.load(Ordering::SeqCst) {
        thread::sleep(POLL);
        if stream.is_none() && Instant::now() >= retry_at {
            match open(&enumerator) {
                Ok(s) => stream = Some(s),
                Err(_) => retry_at = Instant::now() + Duration::from_secs(1), // no speaker right now
            }
        }
        let mut got = 0;
        if let Some(s) = stream.as_mut() {
            match drain(s, &ring) {
                Ok(n) => got = n,
                Err(_) => {
                    // e.g. AUDCLNT_E_DEVICE_INVALIDATED: the speaker went away
                    stream = None;
                    retry_at = Instant::now() + Duration::from_millis(300);
                }
            }
        }
        if got > 0 {
            last_audio = Instant::now();
            silence_mark = last_audio;
        } else if last_audio.elapsed() > SILENCE_AFTER {
            fill_silence(&ring, &mut silence_mark);
        }
        if last_check.elapsed() >= Duration::from_secs(1) {
            last_check = Instant::now();
            if let (Some(s), Some(id)) = (stream.as_ref(), default_id(&enumerator)) {
                if s.id != id {
                    stream = None; // the default speaker changed: follow it
                    retry_at = Instant::now();
                }
            }
        }
    }
}
