"""Write a 10 s synthetic stereo mix (48 kHz s16le) for the benchmarks:
kick drum at 120 BPM, a bass line, a sung-like melody in the centre, hi-hats and
chords panned to the sides. Free to share, unlike a recording of real music.

python3 bench/make_audio.py bench/audio/mix-10s.raw
"""
import sys

import numpy as np

RATE, SECONDS = 48000, 10.0
rng = np.random.default_rng(1)
t = np.arange(int(RATE * SECONDS)) / RATE
left = np.zeros_like(t)
right = np.zeros_like(t)

beat = 0.5  # 120 BPM
for start in np.arange(0, SECONDS, beat):
    n = slice(int(start * RATE), int(min(SECONDS, start + 0.25) * RATE))
    tt = t[n] - start
    kick = np.sin(2 * np.pi * (50 + 80 * np.exp(-tt * 30)) * tt) * np.exp(-tt * 12) * 0.6
    left[n] += kick
    right[n] += kick
    hat = slice(int((start + 0.25) * RATE), int(min(SECONDS, start + 0.29) * RATE))
    noise = rng.standard_normal(hat.stop - hat.start) * 0.08 * np.exp(-np.arange(hat.stop - hat.start) / 400)
    left[hat] += noise * 0.9
    right[hat] += noise * 0.2

notes = [55, 55, 65.4, 73.4, 82.4, 73.4, 65.4, 61.7]
for i, start in enumerate(np.arange(0, SECONDS, 1.0)):
    n = slice(int(start * RATE), int(min(SECONDS, start + 0.9) * RATE))
    bass = np.sin(2 * np.pi * notes[i % len(notes)] * t[n]) * 0.3
    left[n] += bass
    right[n] += bass

melody = [262, 294, 330, 392, 330, 294, 262, 220]
for i, start in enumerate(np.arange(0, SECONDS, 0.6)):
    n = slice(int(start * RATE), int(min(SECONDS, start + 0.5) * RATE))
    tt = t[n] - start
    f = melody[i % len(melody)] * (1 + 0.01 * np.sin(2 * np.pi * 5.5 * tt))
    phase = 2 * np.pi * np.cumsum(f) / RATE
    voice = sum(np.sin(k * phase) / k for k in range(1, 8)) * 0.12 * np.minimum(1, tt * 20)
    left[n] += voice
    right[n] += voice

for root, start in zip((196, 220, 175, 196), (0, 2.5, 5, 7.5)):
    n = slice(int(start * RATE), int(min(SECONDS, start + 2.4) * RATE))
    chord = sum(np.sin(2 * np.pi * root * r * t[n]) for r in (1, 1.26, 1.5)) * 0.05
    left[n] += chord * 0.2
    right[n] += chord

mix = np.stack([left, right], axis=1)
mix /= np.abs(mix).max() / 0.8
(mix * 32767).astype("<i2").tofile(sys.argv[1])
