"""Python side of the comparison: the same offline loop as `aula-f87-rs render-file|bench`,
using the Python app's own analyzer and renderer (aula_f87/music.py).

python3 bench/py_frames.py render|bench AUDIO.raw [--style bars] [--follow all] [--fps 30]
                           [--few-colors] [--repeat N] [--python-src ../aula-f87-control]
"""
import argparse
import os
import sys
import time

import numpy as np

p = argparse.ArgumentParser()
p.add_argument("mode", choices=("render", "bench"))
p.add_argument("file")
p.add_argument("--style", default="bars", choices=("bars", "meter", "rainbow", "pulse"))
p.add_argument("--follow", default="all")
p.add_argument("--fps", type=float, default=30.0)
p.add_argument("--few-colors", action="store_true")
p.add_argument("--repeat", type=int, default=1)
p.add_argument("--sensitivity", type=float, default=1.0)
p.add_argument("--brightness", type=float, default=1.0)
p.add_argument("--python-src", default=os.environ.get("AULA_F87_PYTHON_SRC", os.path.join(os.path.dirname(__file__), "..", "..", "aula-f87-control")))
a = p.parse_args()
sys.path.insert(0, a.python_src)
from aula_f87 import music  # noqa: E402

follow = tuple(f for f in a.follow.split(",") if f and f != "all")
style = ("bars", "meter", "rainbow", "pulse").index(a.style)
audio = np.fromfile(a.file, dtype="<i2").reshape(-1, 2).astype(np.float32) / 32768.0
hop = int(48000 / a.fps)
times = []
out = sys.stdout if a.mode == "render" else None
for _ in range(a.repeat if a.mode == "bench" else 1):
    ring, analyzer = music._Ring(), music.Analyzer()
    for i in range(len(audio) // hop):
        ring.push(audio[i * hop:(i + 1) * hop])
        t = time.perf_counter()
        analyzer.update(ring.copy(), 1.0 / a.fps, a.sensitivity, follow)
        colors = music.render(analyzer, style, a.brightness, a.few_colors, i / a.fps)
        times.append(time.perf_counter() - t)
        if out:
            out.write(bytes(c for rgb in colors for c in rgb).hex() + "\n")
times.sort()
pick = lambda q: times[int((len(times) - 1) * q)] * 1e3  # noqa: E731
line = (f"frames {len(times)} median {pick(0.5):.3f} ms p99 {pick(0.99):.3f} ms "
        f"mean {sum(times) / len(times) * 1e3:.3f} ms max {times[-1] * 1e3:.3f} ms")
print(line if out is None else line, file=sys.stderr if out else sys.stdout) if out else print("python", line)
