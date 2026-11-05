# AULA F87 Control for Windows

A native WinUI 3 app in Rust, on the same core as the Linux app (`../src`).

| File | What it does |
|---|---|
| `src/app.rs` | The app: keyboard connection, effects, music sync, the tray icon and its menu. It outlives the window. |
| `src/ui.rs` | The window: Lighting, Per-key colors, Music sync and About pages (windows-reactor components). |
| `src/keyview.rs` | The on-screen keyboard (the same drawing as the Linux app). |
| `src/bridge.rs` | Gets messages from background threads to the UI thread (a message-only window), timers, one running copy. |
| `src/worker.rs` | Keyboard I/O off the UI thread, one job at a time. |
| `src/autostart.rs` | Start at login (HKCU Run). |
| `src/selfcheck.rs` | `--self-check`, `--loopback-test N`, `--music-test N`, `--live-test`: checks without the window, for testing over SSH. |
| `installer.nsi` | The per-user installer (NSIS). |

The core's Windows parts are `../src/hidwin.rs` (Win32 HID API) and `../src/wasapi.rs` (WASAPI
loopback).

## Building

Build on Windows with the MSVC Rust toolchain (Rust 1.95 or newer) and the Windows SDK.

```powershell
powershell -File windows\fetch-reactor.ps1            # once: windows-rs at the pinned commit
powershell -File windows\build.ps1 -Test -Dist -Installer
```

- **windows-reactor from a pinned commit.** windows-reactor 0.100.0 on crates.io predates its
  `Application` and `NotifyIcon` (tray icon) support. `build.ps1` patches `windows-reactor` and
  `windows-reactor-setup` to a sparse checkout of windows-rs at commit `e9772320b`, made by
  `fetch-reactor.ps1`. Switch back to crates.io when a release includes them.
- **Self-contained.** `windows-reactor-setup` stages the Windows App Runtime next to the exe, so
  nothing else needs installing. The app folder is ~63 MB, the installer ~16 MB.
- **Everything on D:.** `build.ps1` keeps sources, target, the crate cache, the runtime download
  cache and temp files under `D:\aula` (`-Work` changes it).
- **The installer** uses NSIS (`makensis`).
