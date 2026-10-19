"""Stand-in for pw-record: writes 48 kHz stereo s16 audio to stdout in real time,
silence with a tone burst every ~1 s, and logs when each burst starts.

python3 bench/feeder.py T0_LOG BURSTS SEED
"""
import os
import random
import sys
import time

import numpy as np

RATE, CHUNK = 48000, 256          # 256 frames = 5.3 ms per write
log_path, bursts, seed = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
rng = random.Random(seed)
t = np.arange(int(0.3 * RATE)) / RATE
tone = 0.5 * (np.sin(2 * np.pi * 60 * t) + np.sin(2 * np.pi * 1000 * t)) / 2
burst = (np.repeat(tone[:, None], 2, axis=1) * 32767).astype("<i2")
silence = np.zeros((CHUNK, 2), "<i2")

plan = [("silence", int(1.5 * RATE / CHUNK))]
for _ in range(bursts):
    gap = 0.7 + rng.uniform(0, 1 / 30)                   # random phase against the frame clock
    plan += [("burst", None), ("silence", int(gap * RATE / CHUNK))]
plan.append(("silence", int(0.5 * RATE / CHUNK)))

out = sys.stdout.buffer
fd = out.fileno()
start, sent = time.monotonic(), 0
t0s = []

def write(block):
    global sent
    due = start + sent / RATE
    delay = due - time.monotonic()
    if delay > 0:
        time.sleep(delay)
    os.write(fd, block.tobytes())
    sent += len(block)

try:
    for kind, n in plan:
        if kind == "silence":
            for _ in range(n):
                write(silence)
        else:
            for i in range(0, len(burst), CHUNK):
                if i == 0:
                    due = start + sent / RATE
                    while time.monotonic() < due:
                        time.sleep(max(0.0, due - time.monotonic()))
                    t0s.append(time.monotonic_ns())
                write(burst[i:i + CHUNK])
except BrokenPipeError:
    pass
with open(log_path, "w") as f:
    f.write("\n".join(map(str, t0s)) + "\n")
