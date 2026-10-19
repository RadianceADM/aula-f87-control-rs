# Rust vs Python: music sync latency

**Question:** would rewriting [AULA F87 Control](https://github.com/RadianceADM/aula-f87-control)
in Rust make music sync react faster?

**Answer: no.** The Rust version uses far less CPU and memory and starts much faster. But the
time from sound to lit keys is the same, about 18 ms on average after the audio arrives. That
time is set by the keyboard's frame rate, not by the language.

Measured on 2026-10-19. The Python version is v0.4.1 of the app, compared with v0.1.0 of this
port.

## What was compared

This repo ports the whole music-sync path of the Python app (`aula_f87/music.py`) to Rust, line
by line:

- **Audio capture:** the same `pw-record` command, read through a pipe in 1024-frame blocks.
- **Analysis:** all three meters (full spectrum, Bass, Vocals with the stereo-centre mask) and
  the spectral-flux beat detector, with the same constants.
- **Rendering:** all four styles, including the reduced palette used over 2.4G.
- **Output:** live frames over the cable (feature report `0x08`, 30 fps) and the 2.4G receiver
  (`0x88` stream, 20 fps, 10 ms between fragments).
- **Lock file:** the same one the Python app uses, so the two never use the keyboard at the same
  time.

The GUI, top-bar icon, settings, lighting and per-key editing, sleep timer and battery aren't
ported. They don't affect latency.

## 1. Same output

Both versions were fed the same audio, and every LED value of every frame was compared.
Command: `python3 bench/parity.py AUDIO.raw`.

- **8 s of real music**, captured from this computer. The recording isn't in the repo because
  it's someone else's song. Results are in the table below.
- **10 s synthetic mix** (`bench/audio/mix-10s.raw`, made by `bench/make_audio.py`): kick,
  bass line, a centred melody and panned hi-hats and chords. All 4,800 frames, over every row of
  the table below, were identical.

| Style | Follow | Link | Frames | Identical frames | Values that differ | Largest difference |
|---|---|---|---|---|---|---|
| bars | all | cable | 239 | 238 | 2 | 1 |
| bars | all | 2.4G | 239 | 239 | 0 | 0 |
| bars | bass | cable / 2.4G | 239 | 239 | 0 | 0 |
| bars | vocals | cable / 2.4G | 239 | 239 | 0 | 0 |
| bars | beat | cable / 2.4G | 239 | 239 | 0 | 0 |
| bars | bass, vocals, beat | cable / 2.4G | 239 | 239 | 0 | 0 |
| meter | all | cable | 239 | 238 | 1 | 1 |
| rainbow | all | cable | 239 | 238 | 1 | 1 |
| meter, rainbow, pulse | all | 2.4G | 239 | 239 | 0 | 0 |
| pulse | all | cable | 239 | 239 | 0 | 0 |

Out of 3,824 frames, 4 values differ, each by 1 step out of 255, from floating-point rounding.
The two versions do the same work, so the timings below compare like with like.

## 2. Processing time per frame

This measures analysis plus rendering of one frame. Command: `aula-f87-control bench` /
`bench/py_frames.py bench`.

Real music, 2,390 frames, two runs, with the Python app's music sync running in the
background:

| Follow | Rust median | Python median | Rust p99 | Python p99 |
|---|---|---|---|---|
| All | 0.06 ms | 1.5–1.7 ms | 0.2–3.1 ms | 7.8–9.5 ms |
| Bass | 0.10 ms | 2.2 ms | 0.1 ms | 6.3 ms |
| Vocals | 0.14 ms | 2.4 ms | 3.8 ms | 11.9 ms |
| Beat | 0.06 ms | 1.6 ms | 3.1 ms | 9.4 ms |
| Bass + Vocals + Beat | 0.18–0.19 ms | 3.2–3.5 ms | 3.2–4.3 ms | 11.1–13.5 ms |

Synthetic mix, 2,400 frames, quieter machine:

| Follow | Rust median | Python median | Rust p99 | Python p99 |
|---|---|---|---|---|
| All | 0.05 ms | 1.58 ms | 0.06 ms | 2.3 ms |
| Bass + Vocals + Beat | 0.17 ms | 3.56 ms | 0.23 ms | 6.3 ms |

Rust is 20–30 times faster here. But a frame is sent every 33 ms over the cable, and every 50 ms over 2.4G, so Python
also finishes with plenty of time to spare. The p99 spikes in both columns are the machine
being busy. The Python app was running music sync in the background during the tests.

## 3. Real-time latency

Each version ran its real music-sync loop for about 32 seconds per run. The runs alternated
between Rust and Python, two rounds each, 60 tone bursts per version.

- `bench/feeder.py` stood in for `pw-record` and wrote audio into the pipe in real time, in
  5.3 ms chunks. The audio was silence with a tone burst about once a second, at a random
  phase against the frame clock.
- Frames went to a log instead of the keyboard, with timestamps.
- **Latency** is the time from the burst being written into the pipe to the first frame with
  lit keys.
- CPU and memory are for the music-sync process only, read from `/proc`.

Command: `python3 bench/live.py 30 2`.

| Measure | Rust | Python |
|---|---|---|
| Bursts detected | 60/60 | 60/60 |
| **Latency, mean** | **17.9 ms** | **18.4 ms** |
| Latency, median | 19.7 ms | 19.1 ms |
| Latency, 10th–90th percentile | 2.6–30.3 ms | 3.2–33.4 ms |
| Latency, min–max | 0.3–31.9 ms | 1.9–36.5 ms |
| Frame interval, mean | 33.6 ms | 34.4 ms |
| Frame interval, stdev (jitter) | 1.0 ms | 2.4 ms |
| Frame interval, max | 59.8 ms | 61.7 ms |
| Frames per second (target 30) | 29.8 | 29.1 |
| **CPU (steady)** | **0.4 % of one core** | **5.5 % of one core** |
| **Peak memory** | **3.9 MB** | **31.2 MB** |
| **Launch to first frame** | **16 ms** | **298 ms** |

The mean latency differs by 0.5 ms. With 60 bursts, the noise in that mean is about ±1.2 ms,
so this is no measurable difference. Latency is spread evenly from 0 to 33 ms because a burst
waits for the next frame tick. That wait is half a frame on average, about 16.7 ms at 30 fps.
Python's fixed extra cost shows at the low end: its fastest reaction was 1.9 ms against 0.3 ms
for Rust.

For comparison, the installed Python app (GTK window, music sync on) used **3.1 % of one
core and 60 MB** over 20 seconds. Its own `pw-record` used about 0 %.

## Where the delay comes from

From sound playing to keys lighting, roughly:

| Stage | Delay | Changes with Rust? |
|---|---|---|
| PipeWire capture (`--latency 20ms`) | ~20 ms | No, same `pw-record` |
| Waiting for the next frame tick (30 fps cable / 20 fps 2.4G) | 0–33 ms (avg 17) / 0–50 ms (avg 25) | No, the keyboard sets the rate |
| 2.4G only: 10 ms between stream fragments | 10–40 ms | No, shorter gaps stutter |
| Analysis window (2048 samples; 4096 for Bass) | the newest audio is weighted least; ~10–40 ms before a soft onset registers | No, it's the algorithm |
| **Python vs Rust processing** | **~1.5–3 ms vs ~0.1–0.2 ms** | **Yes, saves ~1.5–3 ms** |
| USB transfer + keyboard LED update | a few ms | No, same kernel call |

The total is roughly 50–100 ms over the cable and more over 2.4G. Rust removes about 2 ms of
it, which you can't see.

## What would actually cut latency

These work the same in either language:

1. **A higher frame rate over the cable.** 30 fps is what was tested as stall-free. If the
   keyboard takes 60 fps, the average wait drops from 17 ms to 8 ms. At 60 fps, Python would
   use about 11 % of a core and Rust about 1 %. This is where Rust starts to help with cost,
   not with latency.
2. **A lower capture latency,** for example `--latency 10ms` for `pw-record`. This saves about 10 ms,
   at a small CPU cost.
3. **A shorter window for Beat and Bass** (2048 instead of 4096, or an onset detector working on
   the newest samples). This reacts faster to hits, but gives coarser bass resolution.

## Conclusion

| | Rust | Python |
|---|---|---|
| Latency (what you see) | same | same |
| CPU while syncing | ~13× less | |
| Memory | ~8× less | |
| Start-up | ~20× faster | |
| Already done | music sync only | everything: GUI, top bar, Windows app, installers, PyPI |

A full rewrite would mean redoing the GTK window, top-bar icon, Windows app, installers and
packages. It wouldn't make music sync react faster. It would lower CPU use, which matters on a
laptop battery or at higher frame rates.

A middle path, if the CPU saving is wanted: keep the Python app, and have it start this Rust
binary for music sync. The frames and the lock file are already compatible.

## Not measured yet

- **On the real keyboard.** The lock file was held by the running app's music sync during the
  tests, and this port respects it. To try it, stop music sync in the app, then run
  `target/release/aula-f87-control music` (Ctrl+C stops it). The USB transfer is the same kernel
  call in both versions.
- **Light output itself.** Measuring when the LEDs actually change needs a camera or a light
  sensor. That part is identical for both versions anyway.

## Test machine

Intel Core i5-8265U (4 cores / 8 threads), Ubuntu 26.04.1 LTS, Linux 7.0. Python 3.14.4 with
NumPy 2.3.5 (float32 FFT); Rust 1.96.1, release build with LTO; `realfft` 3.5. The Rust binary
is 1.8 MB.

## Reproduce

```sh
cargo build --release
python3 bench/make_audio.py bench/audio/mix-10s.raw         # or any pw-record --raw capture
python3 bench/parity.py bench/audio/mix-10s.raw             # same output?
target/release/aula-f87-control bench bench/audio/mix-10s.raw --repeat 10
python3 bench/py_frames.py bench bench/audio/mix-10s.raw --repeat 10
python3 bench/live.py 30 2                                  # real-time loop, ~4 minutes
```

The Python scripts expect the Python app's source next to this repo, in `../aula-f87-control`. To use
another location, set `AULA_F87_PYTHON_SRC=/path/to/aula-f87-control`.
