//! Audio -> one 0..1 level per key column, plus a beat pulse.
//!
//! A line-by-line port of `Analyzer` / `_BandMeter` in `aula_f87/music.py`
//! (v0.4.1). Spectra are single precision, like NumPy 2's float32 FFT, so both
//! versions produce (nearly) the same frames from the same audio.

use std::sync::Arc;

use realfft::num_complex::Complex32;
use realfft::{RealFftPlanner, RealToComplex};

pub const RATE: f64 = 48000.0;
pub const FFT_SIZE: usize = 2048; // main analysis window (~43 ms)
pub const BASS_FFT_SIZE: usize = 4096; // longer window: finer steps for the bass range
pub const BANDS: usize = 18; // roughly one band per key column
const LOW_HZ: f64 = 40.0;
const HIGH_HZ: f64 = 16000.0;

pub type Levels = [f64; BANDS];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Follow {
    pub bass: bool,
    pub vocals: bool,
    pub beat: bool,
}

impl Follow {
    pub fn any(&self) -> bool {
        self.bass || self.vocals || self.beat
    }
}

/// `np.hanning(n)`, computed the same way, stored as f32.
fn hanning(n: usize) -> Vec<f32> {
    (0..n)
        .map(|k| {
            let m = (1 + 2 * k) as f64 - n as f64; // arange(1 - M, M, 2)
            (0.5 + 0.5 * (std::f64::consts::PI * m / (n - 1) as f64).cos()) as f32
        })
        .collect()
}

/// `np.geomspace(lo, hi, num)`.
fn geomspace(lo: f64, hi: f64, num: usize) -> Vec<f64> {
    let (a, b) = (lo.log10(), hi.log10());
    let step = (b - a) / (num - 1) as f64;
    let mut v: Vec<f64> = (0..num).map(|i| 10f64.powf(i as f64 * step + a)).collect();
    v[0] = lo;
    v[num - 1] = hi;
    v
}

/// `np.fft.rfftfreq(n, 1 / RATE)`.
fn rfftfreq(n: usize) -> Vec<f64> {
    let val = 1.0 / (n as f64 * (1.0 / RATE));
    (0..=n / 2).map(|k| k as f64 * val).collect()
}

struct BandMeter {
    slices: Vec<(usize, usize)>,
    tilt_db: Levels,
    band_ref: Levels,
    levels: Levels,
}

impl BandMeter {
    const SPAN_DB: f64 = 24.0;
    const RANGE_DB: f64 = 40.0;
    const REF_FALL_DB_PER_S: f64 = 6.0;
    const FALL_PER_S: f64 = 2.2;
    const FLOOR_DB: f64 = -100.0;

    fn new(fft_size: usize, low_hz: f64, high_hz: f64, tilt: bool) -> Self {
        let freqs = rfftfreq(fft_size);
        let edges = geomspace(low_hz, high_hz, BANDS + 1);
        let search = |x: f64| freqs.partition_point(|&f| f < x); // searchsorted, side="left"
        let mut slices = Vec::with_capacity(BANDS);
        let mut tilt_db = [0.0; BANDS];
        for i in 0..BANDS {
            let a = search(edges[i]).min(freqs.len() - 1);
            let b = (a + 1).max(search(edges[i + 1]));
            slices.push((a, b));
            if tilt {
                tilt_db[i] = 3.0 * ((edges[i] * edges[i + 1]).sqrt() / 1000.0).log2();
            }
        }
        BandMeter {
            slices,
            tilt_db,
            band_ref: [Self::FLOOR_DB + Self::SPAN_DB; BANDS],
            levels: [0.0; BANDS],
        }
    }

    fn band_db(&self, spectrum: &[f32]) -> Levels {
        let mut db = [0.0; BANDS];
        for (i, &(a, b)) in self.slices.iter().enumerate() {
            let part = &spectrum[a..b];
            let mean = part.iter().map(|v| v * v).sum::<f32>() / part.len() as f32;
            db[i] = (20.0f32 * (mean.sqrt() + 1e-12f32).log10()) as f64 + self.tilt_db[i];
        }
        db
    }

    /// spectrum: magnitudes, or None for silence.
    fn update(&mut self, spectrum: Option<&[f32]>, dt: f64, sensitivity: f64) -> Levels {
        let mut target = [0.0; BANDS];
        if let Some(spectrum) = spectrum {
            let db = self.band_db(spectrum);
            let span = Self::SPAN_DB * sensitivity;
            for i in 0..BANDS {
                self.band_ref[i] = db[i].max(self.band_ref[i] - Self::REF_FALL_DB_PER_S * dt);
            }
            let loudest = self.band_ref.iter().cloned().fold(f64::MIN, f64::max);
            let floor = (loudest - Self::RANGE_DB).max(Self::FLOOR_DB + span);
            for i in 0..BANDS {
                self.band_ref[i] = self.band_ref[i].max(floor);
                target[i] = ((db[i] - (self.band_ref[i] - span)) / span).clamp(0.0, 1.0);
            }
        }
        for i in 0..BANDS {
            self.levels[i] = target[i].max(self.levels[i] - Self::FALL_PER_S * dt);
        }
        self.levels
    }
}

struct Fft {
    plan: Arc<dyn RealToComplex<f32>>,
    input: Vec<f32>,
    output: Vec<Complex32>,
}

impl Fft {
    fn new(planner: &mut RealFftPlanner<f32>, n: usize) -> Self {
        let plan = planner.plan_fft_forward(n);
        Fft { input: plan.make_input_vec(), output: plan.make_output_vec(), plan }
    }

    /// rfft(samples * window)
    fn run(&mut self, samples: &[f32], window: &[f32]) -> &[Complex32] {
        for ((dst, s), w) in self.input.iter_mut().zip(samples).zip(window) {
            *dst = s * w;
        }
        self.plan.process(&mut self.input, &mut self.output).expect("fft sizes match");
        &self.output
    }
}

pub struct Analyzer {
    window: Vec<f32>,
    bass_window: Vec<f32>,
    full: BandMeter,
    bass: BandMeter,
    vocals: BandMeter,
    prev_low_db: Option<[f64; 4]>,
    flux_avg: f64,
    beat: f64,
    cross: Option<(Vec<Complex32>, Vec<f32>, Vec<f32>)>, // smoothed L*conj(R), |L|^2, |R|^2
    pub levels: Levels,
    pub pulse: f64,
    fft: Fft,
    fft_l: Fft,
    fft_r: Fft,
    bass_fft: Fft,
    mid_all: Vec<f32>,
    left: Vec<f32>,
    right: Vec<f32>,
}

const PULSE_FALL_PER_S: f64 = 3.0;
const CENTRE_SMOOTH_S: f64 = 0.25;
const BEAT_BANDS: usize = 4;

/// The beat lights the board from the middle outwards.
fn burst(i: usize) -> f64 {
    let half = (BANDS - 1) as f64 / 2.0;
    1.0 - 0.45 * (i as f64 - half).abs() / half
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer {
    pub fn new() -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        Analyzer {
            window: hanning(FFT_SIZE),
            bass_window: hanning(BASS_FFT_SIZE),
            full: BandMeter::new(FFT_SIZE, LOW_HZ, HIGH_HZ, true),
            bass: BandMeter::new(BASS_FFT_SIZE, 30.0, 300.0, false),
            vocals: BandMeter::new(FFT_SIZE, 150.0, 5000.0, true),
            prev_low_db: None,
            flux_avg: 0.0,
            beat: 0.0,
            cross: None,
            levels: [0.0; BANDS],
            pulse: 0.0,
            fft: Fft::new(&mut planner, FFT_SIZE),
            fft_l: Fft::new(&mut planner, FFT_SIZE),
            fft_r: Fft::new(&mut planner, FFT_SIZE),
            bass_fft: Fft::new(&mut planner, BASS_FFT_SIZE),
            mid_all: vec![0.0; BASS_FFT_SIZE],
            left: vec![0.0; BASS_FFT_SIZE],
            right: vec![0.0; BASS_FFT_SIZE],
        }
    }

    /// samples: the last BASS_FFT_SIZE stereo samples, oldest first.
    pub fn update(&mut self, samples: &[[f32; 2]], dt: f64, sensitivity: f64, follow: Follow) {
        assert_eq!(samples.len(), BASS_FFT_SIZE);
        for (i, s) in samples.iter().enumerate() {
            self.left[i] = s[0];
            self.right[i] = s[1];
            self.mid_all[i] = (s[0] + s[1]) * 0.5;
        }
        let tail = BASS_FFT_SIZE - FFT_SIZE;
        let mid = &self.mid_all[tail..];
        let rms = (mid.iter().map(|v| v * v).sum::<f32>() / FFT_SIZE as f32).sqrt();
        let silent = (rms as f64) < 10f64.powf(-70.0 / 20.0);
        let norm = (FFT_SIZE / 4) as f32;

        // Whole spectrum: drives "all" and the beat detector.
        let magnitude: Vec<f32> = if silent {
            Vec::new()
        } else {
            self.fft.run(mid, &self.window).iter().map(|c| c.norm()).collect()
        };
        let spectrum: Option<Vec<f32>> =
            (!silent).then(|| magnitude.iter().map(|m| m / norm).collect());
        let full = self.full.update(spectrum.as_deref(), dt, sensitivity);

        // Beat: sudden rises in the low bands (spectral flux) against their recent average.
        let mut hit = 0.0;
        if let Some(spectrum) = &spectrum {
            let db = self.full.band_db(spectrum);
            let low = [db[0], db[1], db[2], db[3]];
            if let Some(prev) = self.prev_low_db {
                let flux: f64 = (0..BEAT_BANDS).map(|i| (low[i] - prev[i]).max(0.0)).sum();
                hit = ((flux - 1.5 * self.flux_avg - 3.0) / 12.0).clamp(0.0, 1.0);
                self.flux_avg += (flux - self.flux_avg) * (dt / 1.0).min(1.0);
            }
            self.prev_low_db = Some(low);
        } else {
            self.prev_low_db = None;
        }
        self.beat = hit.max(self.beat - PULSE_FALL_PER_S * dt);

        if !follow.any() {
            self.levels = full;
            self.pulse = self.beat;
            return;
        }

        let mut levels = [0.0f64; BANDS];
        let mut pulse = f64::MIN;
        if follow.bass {
            let bass_spec: Option<Vec<f32>> = (!silent).then(|| {
                let n = (BASS_FFT_SIZE / 4) as f32;
                self.bass_fft.run(&self.mid_all, &self.bass_window).iter().map(|c| c.norm() / n).collect()
            });
            let part = self.bass.update(bass_spec.as_deref(), dt, sensitivity);
            let mut sorted = part;
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
            pulse = pulse.max(sorted[BANDS - 4..].iter().sum::<f64>() / 4.0);
            merge(&mut levels, &part);
        }
        if follow.vocals {
            let vocal_spec: Option<Vec<f32>> = if silent {
                None
            } else {
                // Keep what sits in the centre: per frequency, L and R must be
                // correlated over time (coherence) and about equally loud.
                let spec_l = self.fft_l.run(&self.left[tail..], &self.window);
                let spec_r = self.fft_r.run(&self.right[tail..], &self.window);
                let lr: Vec<Complex32> = spec_l.iter().zip(spec_r).map(|(l, r)| l * r.conj()).collect();
                let ll: Vec<f32> = spec_l.iter().map(|c| c.norm().powi(2)).collect();
                let rr: Vec<f32> = spec_r.iter().map(|c| c.norm().powi(2)).collect();
                match &mut self.cross {
                    None => self.cross = Some((lr, ll, rr)),
                    Some((olr, oll, orr)) => {
                        let k = (dt / CENTRE_SMOOTH_S).min(1.0) as f32;
                        for i in 0..olr.len() {
                            let d = (lr[i] - olr[i]) * k;
                            olr[i] += d;
                            oll[i] += (ll[i] - oll[i]) * k;
                            orr[i] += (rr[i] - orr[i]) * k;
                        }
                    }
                }
                let (lr, ll, rr) = self.cross.as_ref().unwrap();
                Some(
                    (0..magnitude.len())
                        .map(|i| {
                            let coherence = lr[i].norm().powi(2) / (ll[i] * rr[i] + 1e-20);
                            let balance = 2.0 * (ll[i] * rr[i]).sqrt() / (ll[i] + rr[i] + 1e-20);
                            let mask = (coherence * balance).clamp(0.0, 1.0).powi(2);
                            magnitude[i] * mask / norm
                        })
                        .collect(),
                )
            };
            let part = self.vocals.update(vocal_spec.as_deref(), dt, sensitivity);
            pulse = pulse.max(part.iter().sum::<f64>() / BANDS as f64);
            merge(&mut levels, &part);
        }
        if follow.beat {
            let part: Levels = std::array::from_fn(|i| self.beat * burst(i));
            pulse = pulse.max(self.beat);
            merge(&mut levels, &part);
        }
        self.levels = levels;
        self.pulse = pulse;
    }
}

fn merge(into: &mut Levels, part: &Levels) {
    for (a, b) in into.iter_mut().zip(part) {
        *a = a.max(*b);
    }
}
