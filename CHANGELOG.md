# Changelog

## v0.5.2 — 2026-10-20

- **Two lower music-sync sensitivity steps,** 0.35 and 0.2, for loud, dense music that lit the
  whole board even at the lowest setting. Below 0.5 the bars are cut shorter. On a hip-hop
  recording, 0.5 lit 49 of the 87 keys on average, 0.35 lights 33, and 0.2 lights 19. Settings
  from 0.5 up behave as before.

## v0.5.1 — 2026-10-20

- **Music sync: Vocals follows the singing voice.** It keeps what sits in the centre of the
  stereo mix and is harmonic, so drum hits, the 808 and wide instruments drop out. The keys
  glow while someone sings, centred on the column of the sung pitch, and go dark between
  vocal lines.
  - On a test mix with drums, an 808, a wide pad and a voice singing in phrases, the lights
    are about 3.6× brighter while the voice sings than in the gaps. Before, it was 1.8×. They
    now follow the melody: the correlation with the pitch is 0.76, where it was about 0.
  - On real hip-hop, the Vocals lights no longer move with the beat. The correlation with the
    drums fell from 0.20 to 0.03.
- **Smoother All bars.** They still rise at once, but fall smoothly and are slightly blended
  with their neighbours. Frame-to-frame flicker is halved, and the reaction time is the same.
  Bass and Beat are unchanged.
- **The preview shows the keyboard as it is.** It imitates the current effect as it moves
  (breathing, rainbow wave, raindrops, snake and so on). It also shows the light bar above the
  arrow keys and the side lights, and picks up changes made with Fn shortcuts.
- **Top-bar menu fixed.** Its items could show the wrong labels and clicks could hit the wrong
  item after the battery item appeared.
- Over 2.4G, the window shows the model name it learned over the cable (e.g. F87 Pro).

## v0.5.0 — 2026-10-20

- **Rewritten in Rust.** Same window, top-bar icon, features and settings as the Python app
  (v0.4.2), with a native GTK 4 / libadwaita front end.
  - Music sync uses about 13× less CPU and 8× less memory, and the app starts about 20× faster.
    The lights react just as quickly; see [docs/COMPARISON.md](docs/COMPARISON.md).
  - It reads and writes the same files (`~/.config/aula-f87-control`), so switching keeps your
    settings, and it uses the same lock file, so the two never use the keyboard at once.
- Installs with `cargo install aula-f87-control`.

## v0.4.2 — 2026-10-19

- **About window** with the version, links, and this changelog.
  - Linux: the main menu (☰) in the window, or **About** in the top-bar menu. The changelog is
    under *What's New*, and *Troubleshooting → Debugging Information* has details to paste
    into a bug report.
  - Windows / macOS: the **About** button at the top of the window.
- `aula-f87-control --version` prints the version.

## v0.4.1 — 2026-10-19

- **The keyboard's stored settings are protected.** On an F87 Pro, they once got damaged after
  the keyboard restarted in the middle of a write. The keyboard then stopped sending key presses
  and its side lights flashed. The app now prevents this:
  - Every settings block it reads is checked. Blocks that don't look like real settings are
    read again, and are never written back.
  - Only changed blocks are written, writes are spaced out, and the result is read back. If a
    write left damaged settings behind, the previous ones are put back right away.
  - Only one program talks to the keyboard at a time. The window, its music sync and the
    command line take turns, so settings are never written while music sync streams.
- **Repair.** If the keyboard's settings are damaged anyway, the app says so and offers
  **Repair**: in the window, in the top-bar menu, or with `aula-f87-control repair`. It writes
  back the last good settings this computer saw, or safe defaults.
- **Closing the window stops music sync,** so the keyboard goes back to its own lighting. It
  starts again when you open the window. The app still stays in the top bar.
- Quick slider moves send one update instead of one per step.
- Settings-safety tests run in CI on Linux and Windows.

## v0.4.0 — 2026-10-19

- **Music sync: choose what to follow.** Tick **All**, or any of **Bass**, **Vocals** and
  **Beat**, in the window, the top-bar menu (*Follow*) or the command line
  (`--follow bass,beat`). The choice is saved.
  - **Bass** looks only at 30–300 Hz, measured with a longer window for finer steps.
  - **Vocals** keeps 150 Hz–5 kHz from the centre of the stereo mix. It uses how alike the two
    channels are over time, so instruments spread to the sides drop out.
  - **Beat** finds kick drums from sudden rises in the low end, so a steady bass line no
    longer counts as a beat.
- Better beat detection for *Beat pulse* and the new *Beat* follow mode.
- The command line rejects an invalid `--follow` value before connecting to the keyboard.

## v0.3.2 — 2026-10-19

- **Packages.** The release workflow can publish to PyPI through trusted publishing, and
  there's an Arch Linux package (`packaging/aur/PKGBUILD`) with the udev rule, menu entry and
  icons.
- **Start at login is on by default for every install method,** including packages: the app
  turns it on the first time it runs. Turning it off is remembered, and updates keep that
  choice.
- **New banner** for link previews and the README.
- Fixed a confusing error when a keyboard was half-connected (hidraw device without a `/dev`
  node).

## v0.3.1 — 2026-10-19

- **App settings are saved now.** The app remembers whether music sync is on, its style,
  sensitivity and brightness, the last tab, the brush color and the window size, in
  `settings.json`. If music sync was on, it starts again with the app, including at login,
  and again when the keyboard comes back. Pressing **Stop** keeps it off.

## v0.3.0 — 2026-10-19

- **Top-bar icon (Linux).** A quick menu for music sync and its style, keep backlight on,
  colors, effects and brightness, plus opening the window, start at login and quitting. The
  battery % is shown next to the icon over 2.4G. It's built directly on D-Bus
  (StatusNotifierItem + dbusmenu), so it needs no extra packages and works with Ubuntu's
  AppIndicators extension.
- **Runs in the background.** Closing the window keeps the app, and music sync, running in
  the top bar. Use **Quit** in the menu to stop it.
- **Starts at login.** `install.sh` and the quick installer turn this on and start the app
  right away. Turn it off with *Start at login* in the menu or `./install.sh --no-autostart`.
  Re-installing restarts a running copy so updates take effect.
- **New app icon.**
- New `--background` option.

## v0.2.0 — 2026-10-19

First public release.

- **Lighting:** 14 built-in effects in any color or rainbow, with brightness, speed and the
  side lights effect.
- **Per-key colors:** paint keys on an on-screen keyboard and save them to the keyboard.
- **Music sync:** the keys follow whatever the computer plays. Styles are EQ bars, meter
  colors, rainbow and beat pulse. It uses live frames that are never saved to the keyboard.
- **Power:** battery level over 2.4G (the last reading is shown while on the cable), plus a
  *Keep backlight on* switch and a *Turn backlight off after* timer.
- **Apps:** a GTK 4 / libadwaita app on Linux and a Tkinter app on Windows and macOS. The
  command line has `status`, `set`, `sleep` and `music`.
- **Connections:** USB cable and 2.4G receiver. Linux uses hidraw and PipeWire. Windows and
  macOS use hidapi and soundcard.
- **Quick install scripts:** `scripts/install-linux.sh` and `scripts/install-windows.ps1`.
- **Protocol reference:** [docs/PROTOCOL.md](docs/PROTOCOL.md).
