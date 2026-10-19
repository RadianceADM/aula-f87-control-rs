"""Score the Vocals follow mode on bench/vocal_mix.py's mix.

python3 bench/vocal_eval.py MIX.raw TRUTH.csv [BINARY]

* contrast: how much brighter the board is while the voice sings than in the
  gaps (drums, 808 and pad keep playing): mean level singing / not singing.
* pitch: correlation between where the lights are (level-weighted column) and
  the sung pitch, over singing frames. 0 = no relation.
* jitter: mean frame-to-frame change of the levels while singing (lower = smoother).
"""
import subprocess
import sys

import numpy as np

mix, truth = sys.argv[1], sys.argv[2]
binary = sys.argv[3] if len(sys.argv) > 3 else "target/release/aula-f87-control"
out = subprocess.run([binary, "levels-file", mix, "--follow", "vocals"], capture_output=True, text=True, check=True)
data = np.array([[float(x) for x in line.split(",")] for line in out.stdout.split()])
levels = data[:, :18]
t = np.genfromtxt(truth, delimiter=",", names=True)
m = min(len(levels), len(t))
levels, sing, f0 = levels[:m], t["singing"][:m].astype(bool), t["f0"][:m]
mean = levels.mean(axis=1)
contrast = mean[sing].mean() / max(mean[~sing].mean(), 1e-6)
cols = np.arange(18)
w = levels.sum(axis=1)
centre = (levels * cols).sum(axis=1) / np.maximum(w, 1e-9)
ok = sing & (w > 0.3)
pitch = np.corrcoef(centre[ok], np.log(f0[ok]))[0, 1] if ok.sum() > 10 else float("nan")
jitter = np.abs(np.diff(levels, axis=0)).mean(axis=1)[sing[1:] & sing[:-1]].mean()
print(f"contrast {contrast:.2f}  pitch {pitch:.2f}  jitter {jitter:.3f}  "
      f"(singing mean {mean[sing].mean():.3f}, gaps mean {mean[~sing].mean():.3f})")
