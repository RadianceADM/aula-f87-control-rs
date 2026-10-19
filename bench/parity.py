"""Feed the same audio to both versions and compare every LED of every frame."""
import itertools
import subprocess
import sys

AUDIO = sys.argv[1]
RUST = "target/release/aula-f87-control"
rows = []
for style, follow, few in itertools.product(("bars", "meter", "rainbow", "pulse"),
                                            ("all", "bass", "vocals", "beat", "bass,vocals,beat"),
                                            (False, True)):
    if style != "bars" and follow != "all":
        continue  # styles only change the colouring; follow modes are tested with bars
    opts = ["--style", style, "--follow", follow] + (["--few-colors"] if few else [])
    rs = subprocess.run([RUST, "render-file", AUDIO, *opts], capture_output=True, text=True, check=True)
    py = subprocess.run([sys.executable, "bench/py_frames.py", "render", AUDIO, *opts],
                        capture_output=True, text=True, check=True)
    a, b = rs.stdout.split(), py.stdout.split()
    assert len(a) == len(b), (len(a), len(b))
    same_frames = sum(x == y for x, y in zip(a, b))
    diffs = [abs(p - q) for x, y in zip(a, b) if x != y
             for p, q in zip(bytes.fromhex(x), bytes.fromhex(y)) if p != q]
    lit = sum(any(bytes.fromhex(x)) for x in b)
    rows.append((style, follow, "2.4G" if few else "cable", len(a), lit, same_frames,
                 len(diffs), max(diffs, default=0)))
print("| Style | Follow | Link | Frames | Frames lit | Identical frames | Differing values | Largest difference |")
print("|---|---|---|---|---|---|---|---|")
for r in rows:
    print("| " + " | ".join(str(v) for v in r) + " |")
