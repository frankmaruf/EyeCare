# EyeBreak

Cross-platform **20-20-20** eye-break reminder built with **Tauri v2** (Rust backend + TypeScript/Vite frontend). Runs on Linux, Windows, and macOS from one codebase.

> Every 20 minutes, look ~20 feet (6 m) away for 20 seconds — to reduce digital eye strain.

Full spec: [`docs/requirements.md`](docs/requirements.md).

## Status — MVP

Implemented in this milestone:

- **Rust timer engine** — authoritative `tokio` 1-second loop: work interval → break → repeat. Survives webview reloads.
- **Controls** — pause/resume, skip, "take a break now", postpone (with a max-postpones cap).
- **Pre-break warning** — native notification N seconds before a break.
- **Break reminder window** — calm look-away screen with a countdown and Skip/Postpone.
- **Escalation levels** — `gentle` (notification only) · `standard` (window) · `forced` (frameless fullscreen, always-on-top).
- **Settings** — adjustable timing/escalation/snooze, persisted as JSON in the OS app-config dir.
- **System tray** — quick menu (open, pause/resume, take break, skip, quit) + live countdown tooltip.
- **Single instance** — a second launch focuses the running copy.
- Closing the main window hides it to the tray (keeps running).

See [`docs/requirements.md`](docs/requirements.md) §10 for the roadmap beyond the MVP (forced-overlay polish, autostart, global shortcuts, idle pause, eye-health extras, Wayland fallbacks, installers).

## Prerequisites (Linux / Debian-based, incl. Kali)

```bash
sudo apt install -y libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev pkg-config
```

Plus a recent **Rust** toolchain and **Node.js**.

## Develop

```bash
npm install
npm run tauri dev
```

## Build (current platform)

```bash
npm run tauri build
```

Linux artifacts land in `src-tauri/target/release/bundle/` (`.deb`, AppImage, …). Build Windows/macOS artifacts on their own OS (or via CI) — Tauri does not cross-compile easily.

## Project layout

```
eyebreak/
├─ index.html            # webview shell (#break loads the break screen)
├─ src/                  # frontend (TypeScript + Vite)
│  ├─ main.ts            # dashboard + break views, talks to Rust
│  └─ styles.css
├─ src-tauri/            # Rust backend
│  ├─ src/lib.rs         # timer engine, commands, tray, windows
│  ├─ tauri.conf.json
│  └─ capabilities/
└─ docs/requirements.md  # full specification
```
