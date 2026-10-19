# AULA F87 Control: Rust music sync (experiment)

This is a Rust port of the music-sync part of
[AULA F87 Control](https://github.com/RadianceADM/aula-f87-control). It was written to answer
one question: would a Rust rewrite make the keyboard react to music faster?

**Result:** the latency is the same. Rust uses about 13× less CPU and 8× less memory, and it
starts about 20× faster. The full numbers, the method and what would actually reduce latency are in
**[docs/COMPARISON.md](docs/COMPARISON.md)**.

For everyday use, install the Python app. It has the window, the top-bar icon, per-key colors,
the sleep timer, battery and Windows support.

## What's here

- `aula-f87-rs music`: music sync for the AULA F87 / F87 Pro, over the USB cable or the 2.4G
  receiver (Linux, hidraw). It analyses audio exactly like the Python app (v0.4.1); a
  frame-by-frame comparison is in the doc.
- `aula-f87-rs render-file` and `aula-f87-rs bench`: run the analysis offline on a recording,
  for comparisons.
- `bench/`: the scripts that produced the numbers, run against the Python app's own code.

## Build and run

```sh
cargo build --release
target/release/aula-f87-rs music --style bars --follow bass,beat    # Ctrl+C stops
```

| Option | Values |
|---|---|
| `--style` | `bars` (default), `meter`, `rainbow`, `pulse` |
| `--follow` | `all` (default), or any of `bass`, `vocals`, `beat`, comma-separated |
| `--sensitivity` | 0.5–2 (default 1) |
| `--brightness` | 0.1–1 (default 1) |
| `--seconds` | stop after this long |
| `--output log:FILE` | write frame timestamps to a file instead of the keyboard |
| `--capture-cmd "…"` | read audio from another program instead of `pw-record` |

Needs `pw-record` (`sudo apt install pipewire-bin`) and permission to use the keyboard's
hidraw device. The Python app's udev rule (`70-aula-f87.rules`) covers this.

It uses the same lock file as the Python app. If the app's music sync is running, this program
says so and exits instead of fighting over the keyboard.

## License

GPL-3.0-or-later, like the Python app it's ported from.
