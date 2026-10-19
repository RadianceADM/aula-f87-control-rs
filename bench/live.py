"""Real-time comparison: run each version's music-sync loop on the same live audio
stream (bench/feeder.py), and measure

* latency: tone burst written to the pipe -> first frame with lit keys sent,
* CPU use and memory of the music-sync process itself (read from /proc, so the
  feeder is not counted), and the time from launch to the first frame.

Runs alternate (Rust, Python, Rust, Python, ...) so background load hits both alike.

python3 bench/live.py [BURSTS_PER_RUN] [ROUNDS]
"""
import os
import statistics
import subprocess
import sys
import tempfile
import time

BURSTS = int(sys.argv[1]) if len(sys.argv) > 1 else 30
ROUNDS = int(sys.argv[2]) if len(sys.argv) > 2 else 2
SECONDS = 1.5 + BURSTS * (0.3 + 0.72) + 1.0
TICK = os.sysconf("SC_CLK_TCK")


def proc_sample(pid):
    """(monotonic s, own CPU seconds, peak RSS MB) of a running process."""
    stat = open(f"/proc/{pid}/stat").read().rsplit(")", 1)[1].split()
    cpu = (int(stat[11]) + int(stat[12])) / TICK          # utime + stime, not children
    hwm = next(int(line.split()[1]) for line in open(f"/proc/{pid}/status")
               if line.startswith("VmHWM"))
    return time.monotonic(), cpu, hwm / 1024
here = os.path.dirname(os.path.abspath(__file__))
tmp = tempfile.mkdtemp()
results = {}

runs = {"rust": [], "python": []}
for round_ in range(ROUNDS):
    for name in ("rust", "python"):
        t0_log = os.path.join(tmp, f"{name}{round_}.t0")
        frame_log = os.path.join(tmp, f"{name}{round_}.frames")
        feeder = f"{sys.executable} {here}/feeder.py {t0_log} {BURSTS} {round_}"
        if name == "rust":
            cmd = [os.path.join(here, "..", "target", "release", "aula-f87-control"), "music",
                   "--output", f"log:{frame_log}", "--seconds", str(SECONDS), "--capture-cmd", feeder]
        else:
            cmd = [sys.executable, os.path.join(here, "py_live.py"), frame_log, str(SECONDS),
                   *feeder.split()]
        spawned = time.monotonic_ns()
        proc = subprocess.Popen(cmd, stderr=subprocess.DEVNULL)
        samples = []
        while proc.poll() is None:
            try:
                samples.append(proc_sample(proc.pid))
            except (OSError, StopIteration, IndexError):
                pass
            time.sleep(0.25)
        time.sleep(0.5)  # let the feeder write its log
        steady = [x for x in samples if x[0] - samples[0][0] >= 3.0]
        cpu_pct = 100 * (steady[-1][1] - steady[0][1]) / (steady[-1][0] - steady[0][0])
        t0s = [int(x) for x in open(t0_log).read().split()]
        frames = [tuple(map(int, line.split())) for line in open(frame_log)]
        lat = []
        for t0 in t0s:
            hit = next((t for t, lit in frames if t >= t0 and lit > 0), None)
            if hit is not None:
                lat.append((hit - t0) / 1e6)
        runs[name].append({
            "lat": lat, "bursts": len(t0s),
            "gaps": [(b[0] - a[0]) / 1e6 for a, b in zip(frames, frames[1:])],
            "fps": len(frames) / ((frames[-1][0] - frames[0][0]) / 1e9),
            "cpu": cpu_pct, "mem": max(x[2] for x in samples),
            "first": (frames[0][0] - spawned) / 1e6,
        })

for name, rs in runs.items():
    lat = [v for r in rs for v in r["lat"]]
    gaps = [v for r in rs for v in r["gaps"]]
    results[name] = {
        "bursts detected": f"{len(lat)}/{sum(r['bursts'] for r in rs)}",
        "latency mean (ms)": statistics.mean(lat),
        "latency median (ms)": statistics.median(lat),
        "latency 10th-90th percentile (ms)": "%.1f-%.1f" % tuple(
            statistics.quantiles(lat, n=10)[i] for i in (0, 8)),
        "latency min-max (ms)": f"{min(lat):.1f}-{max(lat):.1f}",
        "frame interval mean (ms)": statistics.mean(gaps),
        "frame interval stdev (ms)": statistics.pstdev(gaps),
        "frame interval max (ms)": max(gaps),
        "frames/s": statistics.mean(r["fps"] for r in rs),
        "CPU, steady (% of one core)": statistics.mean(r["cpu"] for r in rs),
        "peak memory (MB)": max(r["mem"] for r in rs),
        "launch to first frame (ms)": statistics.mean(r["first"] for r in rs),
    }

keys = list(results["rust"])
print("| Measure | Rust | Python |")
print("|---|---|---|")
for k in keys:
    cells = [results[n][k] for n in ("rust", "python")]
    cells = [f"{c:.1f}" if isinstance(c, float) else str(c) for c in cells]
    print(f"| {k} | {cells[0]} | {cells[1]} |")
