"""A test mix for the Vocals follow mode, with the answer key.

Drums (kick, snare: centred), an 808 bass (centred), hi-hats (panned), a wide
stereo synth pad (decorrelated L/R) and a centred "voice": a harmonic tone with
formants, vibrato and syllables, singing in phrases with gaps. The voice moves
between pitches so pitch tracking can be checked.

python3 bench/vocal_mix.py OUT.raw TRUTH.csv     (48 kHz stereo s16le; truth per 1/30 s)
"""
import sys

import numpy as np

RATE, SECONDS, FPS = 48000, 24.0, 30
rng = np.random.default_rng(3)
n = int(RATE * SECONDS)
t = np.arange(n) / RATE
L = np.zeros(n)
R = np.zeros(n)


def add(sig, start, gain_l=1.0, gain_r=1.0):
    a = int(start * RATE)
    b = min(n, a + len(sig))
    L[a:b] += sig[: b - a] * gain_l
    R[a:b] += sig[: b - a] * gain_r


beat = 60 / 90  # 90 BPM, hip-hop tempo
for i, start in enumerate(np.arange(0, SECONDS, beat / 2)):
    tt = np.arange(int(0.35 * RATE)) / RATE
    if i % 4 == 0:   # kick
        add(np.sin(2 * np.pi * (45 + 90 * np.exp(-tt * 25)) * tt) * np.exp(-tt * 9) * 0.9, start)
    if i % 4 == 2:   # snare: tone + noise, centred
        snare = (np.sin(2 * np.pi * 190 * tt) * 0.4 + rng.standard_normal(len(tt)) * 0.5) * np.exp(-tt * 18)
        add(snare * 0.7, start)
    hh = rng.standard_normal(int(0.04 * RATE)) * np.exp(-np.arange(int(0.04 * RATE)) / 300) * 0.12
    add(np.convolve(hh, [1, -1], "same"), start + beat / 4, 0.9, 0.3)

for start in np.arange(0, SECONDS, 2 * beat):   # 808: long, slightly saturated
    tt = np.arange(int(1.2 * RATE)) / RATE
    f = 49.0 if int(start / (2 * beat)) % 2 == 0 else 41.2
    add(np.tanh(2.5 * np.sin(2 * np.pi * f * tt)) * np.exp(-tt * 1.5) * 0.35, start)

# wide pad: different phases left and right, 300-2000 Hz
for root, start in zip((220, 196, 262, 247) * 3, np.arange(0, SECONDS, 2.0)):
    tt = np.arange(int(2.0 * RATE)) / RATE
    env = np.minimum(1, tt * 4) * np.minimum(1, (2.0 - tt) * 4)
    for mult in (1, 1.5, 2, 3):
        add(np.sin(2 * np.pi * root * mult * tt) * env * 0.05, start, 1, 0)
        add(np.sin(2 * np.pi * root * mult * 1.003 * tt + 1.7) * env * 0.05, start, 0, 1)

# voice: phrases of 2.5 s with 1.5 s gaps; notes change every 0.5 s
truth_on = np.zeros(n, bool)
truth_f0 = np.zeros(n)
notes = [196, 220, 247, 294, 330, 294, 247, 220, 165, 147, 196, 262]
phase = 0.0
voice = np.zeros(n)
k = 0
for start in np.arange(0.5, SECONDS - 2.5, 4.0):
    for j in range(5):
        a, b = int((start + 0.5 * j) * RATE), int((start + 0.5 * j + 0.5) * RATE)
        f0 = notes[k % len(notes)]
        k += 1
        tt = np.arange(b - a) / RATE
        f = f0 * (1 + 0.012 * np.sin(2 * np.pi * 5.5 * tt))
        ph = 2 * np.pi * np.cumsum(f) / RATE
        syll = 0.55 + 0.45 * np.clip(np.sin(2 * np.pi * 5 * tt), 0, 1) ** 0.5   # syllables
        tone = np.zeros(b - a)
        for h in range(1, 25):
            fh = f0 * h
            formant = sum(g * np.exp(-((fh - fc) / bw) ** 2) for fc, bw, g in
                          ((700, 300, 1.0), (1200, 400, 0.6), (2600, 500, 0.35)))
            tone += np.sin(h * ph) * (formant + 0.02) / h ** 0.3
        env = np.minimum(1, tt * 30) * np.minimum(1, (len(tt) / RATE - tt) * 30)
        voice[a:b] = tone * syll * env * 0.09
        truth_on[a:b] = True
        truth_f0[a:b] = f0
L += voice
R += voice

mix = np.stack([L, R], axis=1)
mix /= np.abs(mix).max() / 0.9
(mix * 32767).astype("<i2").tofile(sys.argv[1])
hop = RATE // FPS
with open(sys.argv[2], "w") as fh:
    fh.write("frame,singing,f0\n")
    for i in range(n // hop):
        seg = slice(i * hop, (i + 1) * hop)
        fh.write(f"{i},{int(truth_on[seg].mean() > 0.5)},{truth_f0[seg].max():.1f}\n")
