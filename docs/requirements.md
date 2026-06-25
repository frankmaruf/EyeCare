# EyeBreak — Project Requirements & Research

> **Working title:** *EyeBreak* (alternatives: *BlinkGuard*, *20-20-20*, *LookAway*, *RestEyes*)
> **Document type:** Requirements specification + technical research
> **Platforms:** Linux, Windows, macOS
> **Stack:** Tauri (v2) — Rust backend + web frontend
> **Status:** Draft v0.3 — added §4.12 / §7.9 floating mini-timer widget

---

## 1. Purpose

I work on my laptop for long stretches because my job requires it. Continuously staring at a screen strains the eyes, so I want a small cross-platform helper that reminds me to take regular breaks and look away from the screen — based on the **20-20-20 rule** — with reminders that are adjustable and, when needed, impossible to ignore.

## 2. Background — the 20-20-20 rule

The rule eye-care professionals recommend for screen use:

> **Every 20 minutes, look at something about 20 feet (6 meters) away for at least 20 seconds.**

This relaxes the eye's focusing muscles and encourages blinking, reducing digital eye strain. The interval (20 min) and the break length (20 sec) are defaults — the app must let me change both, since I may prefer a 10-minute cycle or a longer break.

## 3. Goals & Non-Goals

**Goals**
- Remind me to take eye breaks on a configurable schedule.
- Make reminders escalate from gentle to forced (always-on-top) when I ignore them.
- Let me control how the reminder window sits relative to other windows (above / below / normal — like a z-index).
- Launch automatically at system startup (toggleable).
- Run on Linux, Windows, and macOS from one Tauri codebase.
- Be lightweight and stay out of the way until it's time to break.

**Non-Goals (for v1)**
- Cloud sync or accounts.
- Detailed analytics dashboards.
- Mobile (Android/iOS) versions.
- Wrist/RSI-specific coaching (eyes are the focus; basic posture nudges are in scope).

## 4. Functional Requirements

### 4.1 Timer / interval engine
- Run a repeating timer for the **work interval** (default 20 min, adjustable).
- When the interval elapses, trigger a **break** of configurable **break length** (default 20 sec, adjustable).
- Support pausing/resuming and skipping the current cycle.
- Optionally detect idle time (no keyboard/mouse activity) and pause the timer so I'm not reminded to rest when I'm already away. *(configurable)*
- Timer should run in the Rust backend (reliable, precise, survives webview reloads).

### 4.2 Pre-break warning (heads-up)
- Show a short **"break in N seconds"** warning before a break begins (default 30 s, adjustable, 0 = off), so a forced overlay never slams up mid-sentence.
- The warning is gentle (small banner/notification) and lets me finish a thought or postpone before the break starts.

### 4.3 Break notification — escalation levels
The reminder should have configurable intensity:

1. **Gentle** — a non-intrusive notification / small banner I can ignore.
2. **Standard** — a visible reminder window that appears but doesn't steal focus.
3. **Forced** — once the time limit is hit/crossed, a window **forcefully comes over all other open software**, optionally fullscreen, and stays until the break is done. A *Skip / Postpone* button may be shown or hidden depending on settings.

Which level is used (and whether forced mode is allowed) must be configurable.

### 4.4 Snooze / postpone behavior
- A **postpone** action delays the break by a configurable **snooze duration** (default 3 min).
- A configurable **maximum number of postpones** (default 2; 0 = unlimited) prevents skipping breaks indefinitely — after the cap, the break is enforced.
- **Skip** (skip this cycle entirely) and **postpone** (delay a few minutes) are distinct actions; either can be hidden via settings for a stricter mode.

### 4.5 Window layering (z-order control)
Like CSS `z-index`, I want to choose where the app's window sits:
- **Always on top** — above all other windows.
- **Always below** — sits beneath other windows (e.g. near desktop level), useful for a passive countdown widget.
- **Normal** — behaves like a regular window.

This must be a setting I can change at runtime. *(See §7.2 — "always below" needs platform-specific work in Tauri.)*

### 4.6 Forced overlay / alarm
- When the timer crosses the limit and forced mode is on, show a frameless full-screen always-on-top overlay covering the **active monitor or all monitors** *(configurable)*.
- Optional sound **alert/alarm** when the break starts *(toggleable, with volume and sound choice)*.
- Optional ability to require the full break length before the overlay can be dismissed.
- Disable close/minimize during a required break; re-enable (or show Skip/Postpone) based on settings.

### 4.7 Break-end signal
- When the break finishes, optionally show a brief "break over" notification and/or a gentle chime so I know I can return to work. *(toggleable)*

### 4.8 Global keyboard shortcuts
- System-wide hotkeys (work even when the app isn't focused) for: **pause/resume**, **skip break**, **take a break now**, and **postpone**.
- Shortcuts are configurable and can be disabled.

### 4.9 Startup / autostart
- Option to launch the app automatically when the laptop boots / user logs in.
- This setting must be a toggle the user can turn on/off from within the app.

### 4.10 Settings & persistence
All settings persist across restarts (stored locally as JSON). Includes everything in the §6 table. Add:
- **Import / export settings** — back up or move config between machines via a settings file.

### 4.11 Tray / menu bar
- Live in the system tray (Windows/Linux) and menu bar (macOS).
- Quick access: pause/resume, skip break, take a break now, postpone, open settings, quit.
- Show time remaining until next break.

### 4.12 Floating mini-timer widget (minimized / always-visible mode)
When I minimize the main window (or send it to the tray), I want the option of a small **floating desktop widget** that stays on screen and shows the countdown to my next break — glanceable, out of the way, and good-looking. The tray tooltip already shows the time, but a visible widget is faster to read and feels more alive.

- **When it shows** *(configurable)*: `off` · `only when the main window is minimized/hidden` (default) · `always on top of the desktop`.
- **What it shows**: time remaining until the next break (and, during a break, the break countdown), with an optional progress ring and a small status dot (working / break / paused).
- **Frameless & floating**: no title bar; transparent or solid background; sits above other windows. Its stacking obeys the same z-order control as §4.5 (above / normal / below).
- **Draggable**: grab it anywhere to move it; it **remembers its position** across restarts. Optional snap to a screen corner/edge.
- **Adjustable size**: width and height (or a single scale slider), with sensible min/max and a live preview — so it can be a tiny pill or a larger dial.
- **Adjustable shape**: **round (circle)**, **squircle (rounded square, adjustable corner radius)**, or **square** — switchable at runtime.
- **Adjustable look**: opacity/translucency, accent color / follows the app theme, light/dark. Honors **reduce motion** (§5) — no pulsing/animated ring when that's on.
- **Interactions**: single-click restores the main window; right-click opens a mini menu (pause/resume, take a break now, skip, postpone, open settings, quit); optional double-click action.
- **Passive (click-through) mode** *(optional)*: let clicks pass through to whatever is beneath, turning the widget into a pure display that never gets in the way.
- **Auto-hide during a forced break overlay** so the two don't overlap.
- Persisted like every other setting (size, shape, opacity, position, mode) — see §6.

*(Platform feasibility — transparency, rounded corners, click-through and "always below" each have per-OS caveats; see §7.9.)*

## 5. Non-Functional Requirements
- **Lightweight:** low CPU and memory while idle; small install size (a strength of Tauri).
- **Responsive:** reminder must fire within ~1 second of the scheduled time.
- **Reliable:** timer must survive sleep/wake (recompute on resume).
- **Single instance:** only one copy runs at a time (launching again focuses the running app).
- **Respect the OS:** honor system **Do-Not-Disturb / Focus** mode — suppress non-forced reminders when the OS is in DND. *(configurable)*
- **Accessible:** readable fonts, high-contrast option, keyboard-dismissible, and a **"reduce motion"** option that disables animations (important since some break visuals/exercises animate).
- **Privacy:** fully local; no telemetry without explicit opt-in.
- **Updatable:** support in-app/auto updates so fixes ship easily.

## 6. Settings reference (summary table)

Every row below is a runtime-adjustable setting (changed in-app, persisted to JSON, applied without restart). The **Feasibility** marker shows how hard it is to deliver the *underlying behavior* across platforms:

- ✅ **Pure app logic / standard API** — fully adjustable and reliable on Windows, macOS, and all Linux setups.
- ⚠️ **Adjustable, but the behavior has a platform caveat** — works on Windows/macOS/X11; degraded or needs extra work on Wayland or needs an OS permission. See §7.2 / §7.7 / §7.8.

| Setting | Range / values | Default | Feasibility & platform notes |
|---|---|---|---|
| Work interval | 1–120 min | 20 | ✅ App logic |
| Break length | 5–600 sec | 20 | ✅ App logic |
| Pre-break warning | 0–120 sec (0 = off) | 30 | ✅ App logic + notification |
| Escalation level | gentle / standard / forced | standard | ⚠️ "forced" overlay is limited on Wayland (§7.2, §7.8); gentle/standard ✅ everywhere |
| Allow forced mode | on / off | on | ✅ Toggle is trivial; the forced overlay it enables is ⚠️ on Wayland |
| Snooze / postpone duration | 1–60 min | 3 | ✅ App logic |
| Max postpones | 0–10 (0 = unlimited) | 2 | ✅ App logic |
| Window layering | above / below / normal | normal | ⚠️ above/normal native in Tauri; "below" needs per-OS code; always-on-top restricted on Wayland (§7.2, §7.8) |
| Overlay scope | active / all monitors | active | ⚠️ multi-monitor fullscreen overlay reliable on X11; varies on Wayland |
| Sound alert (start) | on/off · volume 0–100 · sound file | off | ✅ Audio plays in the webview (Linux uses PulseAudio/PipeWire) |
| Break-end signal | off / notification / chime / both | both | ✅ Notifications need a notification daemon on Linux (present on GNOME/KDE/Xfce) |
| Require full break | on / off | off | ✅ App logic |
| Skip button | shown / hidden | shown | ✅ UI |
| Postpone button | shown / hidden | shown | ✅ UI |
| Idle pause | on/off · threshold 30–600 sec | on | ⚠️ idle detection solid on Windows/macOS/X11; limited on Wayland (§7.5, §7.8) |
| Respect OS Do-Not-Disturb | on / off | on | ⚠️ *reading* OS DND state is inconsistent across OSes; if unavailable, fall back to in-app Work hours (§9.9) |
| Reduce motion | on / off | off | ✅ App setting (can also follow the OS "reduce motion" preference) |
| Global shortcuts | on/off · per-action key bindings | on | ⚠️ X11 ✅; Wayland restricts global hotkeys (needs the desktop's global-shortcuts portal); macOS needs Accessibility permission (§7.8) |
| Work hours | start–end time (or off) | off | ✅ App logic |
| Autostart on boot | on / off | off | ✅ `tauri-plugin-autostart` (Linux uses XDG autostart — works on Kali/Ubuntu/most DEs) |
| Floating widget mode | off / when-minimized / always | when-minimized | ✅ separate always-on-top window (§4.12, §7.9) |
| Widget shape | round / squircle / square (+ corner radius) | squircle | ✅ CSS `border-radius` |
| Widget size | 80–320 px or scale 50–200% | 140 px | ✅ `setSize` |
| Widget background | solid / translucent / transparent | solid | ⚠️ transparency has macOS/Wayland/Nvidia caveats (§7.9); **solid always ✅** |
| Widget opacity | 20–100% | 95 | ✅ when solid/translucent; ⚠️ with full transparency (§7.9) |
| Widget click-through | on / off | off | ✅ `setIgnoreCursorEvents` (whole-widget; per-region deferred) |
| Widget position | remembered x/y · corner snap | bottom-right | ✅ persisted + `tauri-plugin-positioner` |
| Widget z-order | above / normal / below | above | ⚠️ "below" needs per-OS code, restricted on Wayland (§7.2) |
| *Eye-health extras (see §9)* | various per feature | — | ✅ mostly app logic (blink, exercises, long breaks, hydration, tips, stats). ⚠️ exceptions tied to hardware/OS: warm-screen filter, ambient-brightness/dark-room (§9.4–9.5) |

### 6.1 Adjustability summary
The **majority of settings are pure app logic and are fully adjustable on every target** (all timing values, escalation choice, snooze/postpone, buttons, sounds, reduce-motion, work hours, and most eye-health extras). The ⚠️ rows are still adjustable — the *setting* always works — but their effect depends on the OS/desktop: **forced overlay, "always below" layering, multi-monitor overlay, idle pause, OS-DND reading, and global shortcuts**. Each has a defined fallback so the app degrades gracefully rather than breaking (detailed in §7.2, §7.5, §7.7, §7.8). Practical implication: on Windows, macOS, and **X11-based Linux** every setting is fully functional; on **Wayland** the ⚠️ behaviors fall back.

## 7. Technical stack & research (Tauri)

### 7.1 Stack
- **Tauri v2** — Rust backend + web frontend, small binaries, low memory.
- **Frontend:** any lightweight web framework (Svelte / vanilla / React) — keep it minimal for footprint.
- **Core timer & state in Rust** (e.g. a `tokio` interval task) for precision and reliability; emit events to the frontend for UI updates.

**Tauri plugins / APIs to use**
- `tauri-plugin-autostart` — launch at login on all three OSes (§4.9).
- `tauri-plugin-notification` — native notifications for gentle level, pre-break warning, break-end (§4.2, §4.7).
- `tauri-plugin-store` — persist settings as JSON; supports import/export (§4.10).
- `tauri-plugin-single-instance` — enforce one running copy (§5).
- `tauri-plugin-global-shortcut` — system-wide hotkeys (§4.8).
- `tauri-plugin-positioner` — place a corner countdown widget.
- `tauri-plugin-updater` — auto-updates (§5).
- **System tray** — built into Tauri v2 (`TrayIconBuilder`) for the tray/menu-bar icon (§4.11).
- **Overlay windows** — `WebviewWindowBuilder`, one window per monitor for "all monitors"; configure with `set_fullscreen(true)`, `set_decorations(false)`, `set_always_on_top(true)`, `set_skip_taskbar(true)` (§4.6).

### 7.2 Window layering (the "z-index") per platform
The OS, not CSS, controls real window stacking. Tauri exposes `set_always_on_top(bool)`, which covers **above** and **normal** cleanly. There is **no first-class "always below"** API — achieving a below-everything window needs platform-specific code via a raw window handle:

- **Windows:** `SetWindowPos` with `HWND_TOPMOST` (above) or `HWND_BOTTOM` (below); `WS_EX_NOACTIVATE` to avoid stealing focus. True desktop-level "below everything" is the wallpaper/`WorkerW` trick (advanced).
- **macOS:** set `NSWindow.level` — `.screenSaver` (very high, for forced overlays), `.floating`, `.normal`, or a value near `kCGDesktopWindowLevel` for "always below".
- **Linux (X11):** EWMH hints `_NET_WM_STATE_ABOVE` / `_NET_WM_STATE_BELOW`. Behavior depends on the window manager.
- **Linux (Wayland):** locked down for security. Persistent/forced overlays generally need the **wlr-layer-shell** protocol — supported by wlroots-based compositors but **not** GNOME's Mutter. Riskiest platform; test early (see §7.7).

> **Recommendation:** ship **above / normal** in v1 (native Tauri). Treat **"always below"** as a later, platform-specific add-on — or drop it if not worth the effort.

### 7.3 Forced overlay
Implement as a frameless, always-on-top, fullscreen window per target display, shown when the break begins. Disable close/minimize during a required break; re-enable (or show Skip/Postpone) based on settings. Auto-suppress when screen-sharing/presenting (§9.9).

### 7.4 Autostart
Use **`tauri-plugin-autostart`**, which handles all three platforms. Under the hood: Windows registry `HKCU\...\CurrentVersion\Run`; macOS Login Items / `LaunchAgents`; Linux XDG autostart `~/.config/autostart/<app>.desktop`. Expose as a single in-app toggle.

### 7.5 Sleep/wake & idle
- Recompute the timer on resume from sleep so it doesn't fire a backlog of breaks.
- For idle detection, use a cross-platform Rust crate (e.g. `user-idle`) that reads seconds-since-last-input. Note Wayland idle support is limited — have a fallback.

### 7.6 Reference projects (study for behavior, regardless of their stack)
- **Stretchly** — open-source cross-platform break reminder; closest match to this spec. Great for UX patterns (pre-break warning, strict mode, multi-monitor).
- **SafeEyes** — 20-20-20 app with eye-exercise screens and idle handling.
- **Workrave** — mature break/RSI tool.
- **EyeLeo** / **Time Out** — Windows / macOS break reminders.

### 7.7 Risks & things to validate early
- **Wayland forced overlay:** a fullscreen always-on-top overlay is **not guaranteed** on Wayland (esp. GNOME/Mutter). Validate on target distros up front; provide a graceful fallback (maximized normal window + notification) where layer-shell isn't available.
- **"Always below" mode:** no native Tauri API; needs per-OS work. Consider deferring.
- **Idle detection on Wayland:** limited; design a fallback path.
- **Presentation/screen-share suppression (§9.9):** detecting this reliably across OSes is non-trivial; an acceptable approximation is "suppress when another app is fullscreen."
- **macOS permissions:** notifications (and accessibility, if needed for global shortcuts/idle) require user grants — request them and handle denial gracefully.
- **Distribution/signing:** macOS notarization and code signing; Linux packaging as AppImage/.deb/.rpm (see §7.8 for distro/desktop coverage); Windows installer signing to avoid SmartScreen warnings.

### 7.8 Linux distribution & desktop-environment support
**Compatibility is decided by the display server (X11 vs Wayland) and desktop environment — not by the distro name.** The same Tauri binary runs on Kali, Ubuntu, Debian, Fedora, Arch, Mint, etc. as long as dependencies are met. So "make it work on Kali, Ubuntu, and others" is mainly a packaging + X11/Wayland-handling problem, not a per-distro one.

- **Runtime dependency:** Tauri v2 on Linux needs **WebKitGTK** (`libwebkit2gtk-4.1`), GTK, and `libayatana-appindicator` for the tray. These are standard apt packages on Debian-based distros (Kali, Ubuntu).
- **Packaging (covers "Kali, Ubuntu, and others"):**
  - **`.deb`** → Debian-based distros, which include **both Kali and Ubuntu** (plus Mint, Pop!_OS, etc.).
  - **AppImage** → one portable file that runs across most distros with no install — the best single artifact for "and others". *Recommended primary Linux build.*
  - **`.rpm`** → Fedora / RHEL / openSUSE; **Flatpak** (optional) → broad sandboxed reach via Flathub.
- **Per-environment reality:**
  - **Kali Linux** — default is **Xfce on X11** → best case: forced overlay, idle pause, and global shortcuts all work reliably. (Kali GNOME/KDE images follow those DEs.)
  - **Ubuntu** — **GNOME on Wayland by default** (since 22.04), but an **X11 session is selectable at the login screen**. On Wayland the ⚠️ behaviors degrade; the X11 session restores full functionality. Detect the session via `XDG_SESSION_TYPE` and adapt at runtime.
  - **GNOME tray caveat** — stock GNOME hides AppIndicator tray icons without the **AppIndicator/KStatusNotifier** extension. Ubuntu ships it by default; vanilla GNOME may not. Provide a fallback (open a normal window) when no tray is available.
  - **KDE Plasma / Xfce** — tray and X11 behaviors generally work out of the box.
- **Wayland mitigations:** detect Wayland and (a) fall back the forced overlay to a maximized borderless window + urgent notification, (b) route global shortcuts through the XDG **GlobalShortcuts portal** where supported, (c) use an idle fallback / idle-notify portal.
- **Suggested Linux test matrix:** Kali (Xfce/X11), Ubuntu (GNOME/**Wayland** and GNOME/**X11**), Fedora (GNOME/Wayland), and one KDE Plasma distro — covering both display servers and the main desktops before release.

### 7.9 Floating widget — Tauri implementation & cross-platform research
The widget (§4.12) is a **second `WebviewWindow`** (label `widget`) created alongside the main window with `transparent: true`, `decorations: false`, `alwaysOnTop: true`, `skipTaskbar: true`. The countdown UI is the same web stack as the rest of the app; shape and size are just CSS plus window sizing — so most of it is pure app logic. The sensitive part is **transparency**, which is where the per-OS caveats live.

- **Shape** — pure CSS on a transparent window: `border-radius: 50%` = circle, e.g. `22px` = squircle, `0` = square. Bind the radius to a setting → adjustable at runtime. ✅ everywhere (given transparency works, below).
- **Size (adjustable W/H)** — store width/height (or a scale factor) and apply with `WebviewWindow.setSize()` / builder `inner_size`, clamped to a min/max. Live preview = call `setSize` as the slider moves. ✅ app logic.
- **Drag + remembered position** — a `data-tauri-drag-region` element moves the frameless window; persist x/y on the window `moved` event and restore on launch. `tauri-plugin-positioner` (already listed in §7.1) provides corner/edge snapping. ✅.
- **Z-order (above / normal / below)** — reuses §4.5 / §7.2: `set_always_on_top(true)` for *above* (✅ native); *below* needs the per-OS raw-handle work and is restricted on Wayland.
- **Click-through (passive mode)** — `WebviewWindow.setIgnoreCursorEvents(true)`. Tauri exposes **no native per-region hit-testing**, so click-through is **whole-widget** in v1 (fully interactive *or* fully passive); selective pass-through (clicks only through the transparent gaps) would need a ~60 fps cursor-position poll that toggles the flag — deferred.
- **Window effects (optional polish)** — `set_effects()` / `EffectsBuilder` gives acrylic/mica blur on Windows and vibrancy on macOS (both require a transparent window).

**Transparency — the platform-sensitive part (validate early):**
- **macOS** — transparency needs the **`macOSPrivateApi: true`** flag in `tauri.conf.json`. Shadows are always disabled for transparent windows. Known bug: a transparent webview window can **lose transparency in the packaged `.dmg`** (tauri-apps/tauri #13415) — test the bundled app, not just `dev`.
- **Windows** — transparency works; Windows 11 draws native rounded corners on an undecorated window when `border: true`; acrylic/mica via `set_effects`.
- **Linux / X11** — works with a running compositor.
- **Linux / Wayland** — needs the compositor's compositing (✅ KDE/KWin — this dev box — and GNOME/Mutter). **CSS shadows that fall outside the window get clipped** (no reliable Wayland frame-extents), so draw any glow on an *inner padded* element rather than outside the window edge.
- ⚠️ **Nvidia + transparent window** — a known class of crashes/visual artifacts (GBM "Error 71", black/ghosted corners; tauri-apps/tauri #14924). **Mitigation / fallback:** ship an **"opaque widget" mode** — a solid rounded background instead of a transparent one — which sidesteps every transparency edge case above while still giving the round/square shapes and adjustable size. `WEBKIT_DISABLE_DMABUF_RENDERER=1` is an additional escape hatch.

> **Recommendation:** default the widget to an **opaque/translucent rounded** look that is **draggable and interactive** (not click-through). Treat *full transparency*, *click-through*, and *"always below"* as opt-in "advanced" toggles — those are exactly the three modes with cross-platform caveats. This delivers a beautiful round/square widget on every target in v1, with the fancier modes degrading gracefully where unsupported.

## 8. UI / UX outline
- **Tray/menu-bar icon** with countdown and quick menu.
- **Floating widget:** small frameless always-on-top countdown shown when minimized — round / squircle / square, adjustable size, draggable, remembers its position; click to restore, right-click for the quick menu (§4.12).
- **Settings window:** grouped sections — *Timing*, *Reminders*, *Window & Display*, *Widget*, *Sound*, *Shortcuts*, *Eye health*, *Startup*.
- **Pre-break warning:** small banner/notification with a postpone option.
- **Break overlay:** large, calm message ("Look ~20 feet away — relax your eyes"), countdown, optional Skip/Postpone, optional progress ring.
- **Notifications:** native OS notifications for gentle level and break-end.

## 9. Additional eye-health features (suggestions)

Extra features beyond the core 20-20-20 reminder that would make this a more complete eye-health helper. Each is optional and toggleable so the app stays minimal for those who want only the basics.

### 9.1 Blink reminders
We blink far less while staring at screens, and incomplete blinking is a leading cause of dry, tired eyes. A subtle periodic prompt (a brief icon flash or "blink fully" cue, e.g. every few minutes) encourages complete blinks. *Configurable frequency; can be silent/visual-only.*

### 9.2 Guided micro eye-exercises during breaks
Instead of only "look away", optionally show a short animated guide during the break: near-far focus shifts, figure-eight tracing, slow eye rolls, and **palming** (cupping warm palms over closed eyes). Helps relax focusing muscles. *Enable/disable and pick which exercises appear; respects "reduce motion".*

### 9.3 Two break types — micro vs long
- **Micro-break** (eyes): short, every ~20 min — the core look-away.
- **Long/rest break** (body + eyes): longer, every ~60 min — stand up, walk, look out a window at distance.
Separating them addresses both eye strain and the fact that sitting too long is its own problem. *Both intervals and lengths adjustable.*

### 9.4 Blue-light / warm-screen reminder (evening wind-down)
A reminder (or simple toggle/guidance) to enable a warm color temperature / blue-light filter in the evening, since blue light contributes to eye fatigue and can disrupt sleep. The app can detect evening hours and nudge, or link to the OS night-mode (Windows Night Light, macOS Night Shift, GNOME Night Light). *Avoid duplicating OS features — a reminder + deep link is enough for v1.*

### 9.5 Ambient brightness & dark-room warning
Screen brightness should roughly match the surroundings; using a bright screen in a dark room strains the eyes. If a light sensor is available, suggest matching brightness; otherwise, a gentle reminder during evening hours not to use the screen in the dark. *Optional.*

### 9.6 Screen-distance & posture reminder
Occasional reminder to keep the screen about an arm's length away (~50–70 cm) with the top of the screen near eye level. Correct distance reduces both eye strain and neck strain. *Low-frequency, dismissible.*

### 9.7 Hydration reminder
Dehydration worsens dry eyes. An optional periodic water reminder pairs naturally with break reminders. *Off by default; adjustable interval.*

### 9.8 Rotating eye-care tips
On the break screen, show a short rotating tip (proper lighting, the 20-20-20 rule itself, blinking, annual eye check-ups, screen positioning). Lightweight education that reinforces good habits. *Toggleable.*

### 9.9 Work-hours, Do-Not-Disturb & presentation suppression *(important)*
- Only run reminders during configured **work hours**.
- A **snooze / focus** mode to pause reminders during meetings or deep-focus sessions.
- **Auto-suppress forced overlays when screen-sharing or presenting** (detect fullscreen apps / presentation mode) so a break screen never pops up mid–video call or demo.
This is partly usability, but it directly affects whether you'll keep the app enabled long-term — and a forced overlay during a screen share would be a real problem. *Strongly recommended for v1.0.*

### 9.10 Local habit summary & streaks
A simple, private (local-only, no cloud) view of breaks taken, breaks skipped, and a daily streak. Gentle accountability builds the habit without nagging or tracking you online. *Opt-in; no telemetry.*

### 9.11 Calming break visuals
Optionally display a calm nature image or a "look out your window" prompt on the break screen to encourage genuine distance focusing and a mental reset. *Toggleable; ships with a few bundled images, user can add their own.*

> **Suggested extra settings (beyond §6):** blink reminder (on/off + interval), eye-exercise guide (on/off + selection), long-break interval/length, evening warm-screen reminder (on/off), dark-room warning (on/off), posture reminder (on/off + interval), hydration reminder (on/off + interval), eye-care tips (on/off), work hours (start/end), DND/focus toggle, suppress-on-presentation (on/off), habit stats (on/off).

## 10. Suggested roadmap

| Phase | Scope |
|---|---|
| **MVP** | Adjustable work/break timer, pre-break warning, standard reminder window, tray icon, settings persistence, single-instance |
| **v1.0** | Forced always-on-top overlay, window layering (above/normal), snooze + max-postpones, sound alerts + break-end signal, global shortcuts, autostart toggle, work-hours + presentation suppression (§9.9), auto-update |
| **v1.1** | Idle pause, multi-monitor overlay, sleep/wake handling, respect OS DND, high-contrast/reduce-motion, blink reminders (§9.1), micro vs long breaks (§9.3) |
| **v1.2** | **Floating mini-timer widget (§4.12) — opaque/rounded, draggable, adjustable size & shape (round/squircle/square), remembered position**, guided eye-exercises (§9.2), eye-care tips (§9.8), hydration reminder (§9.7), evening warm-screen + dark-room nudges (§9.4–9.5), local habit stats (§9.10), settings import/export |
| **v1.3** | "Always below" window mode (platform-specific), **advanced widget modes (full transparency, click-through, always-below — §7.9)**, posture reminder (§9.6), calming break visuals (§9.11), themes, localization |

## 11. Open questions
- Should forced mode ever be fully un-skippable, or always allow an emergency exit?
- Is Wayland "force over everything" support a hard requirement, or acceptable to degrade gracefully where unsupported?
- Is the "always below" widget mode worth the platform-specific effort, or cut it?
- Single-monitor only for v1, or multi-monitor from the start?
- Floating widget (§4.12): default to **opaque** (dodges the Nvidia/Wayland transparency bugs, §7.9) or transparent-by-default? Show it on the primary monitor only, or on whichever monitor the cursor/active window is on?
- Will this be open-source? (Affects license choice and distribution.)
- Distribution targets: `.msi`/`.exe`, `.dmg`, `.AppImage`/`.deb` — which first?

---

*Next step: scaffold the Tauri v2 app and build the MVP (timer + pre-break warning + standard reminder + settings persistence) before tackling the forced-overlay, autostart, and Wayland-specific work.*
