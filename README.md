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

Install the `.deb` (shows up in the app menu):

```bash
sudo apt install ./src-tauri/target/release/bundle/deb/EyeBreak_0.1.0_amd64.deb
```

## Auto-update (release flow)

The app is wired with `tauri-plugin-updater` and a **"Check for updates"** button (Settings → Updates). It is **inert until you publish signed releases** — and it updates **AppImage / Windows / macOS** bundles only; an apt-installed `.deb` is updated by your package manager, not by the in-app updater.

To make it live:

1. **Keep the signing key safe.** A keypair was generated at `~/.tauri/eyebreak.key` (private) / `.key.pub` (public). The public key is already in `tauri.conf.json` → `plugins.updater.pubkey`. Never commit the private key.
2. **Point the endpoint at your repo.** `tauri.conf.json` → `plugins.updater.endpoints` currently uses a placeholder GitHub URL — change `khulnatech/eyebreak` to your real repo.
3. **Build signed artifacts:**
   ```bash
   export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/eyebreak.key)"
   export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""      # set if the key has a password
   npm run tauri build -- --bundles appimage          # + nsis / dmg on their OSes
   ```
   This emits `*.AppImage`, `*.AppImage.sig`, and a `latest.json` next to the bundle.
4. **Publish** the bundle, its `.sig`, and `latest.json` to the GitHub release the endpoint points at. The app then sees the new version and can download + restart.

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
