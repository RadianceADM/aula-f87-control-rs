"""The Python app's real music-sync loop (MusicSync) with a fake keyboard that logs
when each frame is sent. Capture runs CAPTURE_CMD instead of pw-record.

python3 bench/py_live.py FRAME_LOG SECONDS CAPTURE_CMD... [--python-src DIR]
"""
import os
import subprocess
import sys
import tempfile
import threading
import time

args = sys.argv[1:]
src = os.environ.get("AULA_F87_PYTHON_SRC", os.path.join(os.path.dirname(__file__), "..", "..", "aula-f87-control"))
if "--python-src" in args:
    i = args.index("--python-src")
    src = args[i + 1]
    del args[i:i + 2]
log_path, seconds, capture_cmd = args[0], float(args[1]), args[2:]
sys.path.insert(0, src)
from aula_f87 import device, music  # noqa: E402

# The fake keyboard needs no lock; use a private lock file so a running app isn't disturbed.
_init = device.DeviceLock.__init__
lock_dir = tempfile.mkdtemp()
def _private(self):
    _init(self)
    self.path = device.Path(lock_dir, "bench.lock")
device.DeviceLock.__init__ = _private

_popen = subprocess.Popen
music.subprocess.Popen = lambda _cmd, **kw: _popen(capture_cmd, **kw)

log = []
class FakeLink:
    LIVE_FPS = 30
    LIVE_FEW_COLORS = False
    def send_live(self, colors):
        log.append((time.monotonic_ns(), sum(1 for c in colors if any(c))))

sync = music.MusicSync(FakeLink())
done = threading.Event()
done.wait(seconds)
sync.stop()
with open(log_path, "w") as f:
    f.write("".join(f"{t} {lit}\n" for t, lit in log))
