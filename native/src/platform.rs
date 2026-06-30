// Platform integration helpers that stay out of main.rs: fullscreen/presentation
// detection (§9.9) and sound playback (§4.6). Everything degrades to a safe
// no-op when the platform bits aren't available (the doc's "graceful fallback").

/// Is some *other* application currently fullscreen? Used to hush non-forced
/// reminders and hide the widget during a screen-share / presentation / video.
#[cfg(target_os = "linux")]
pub fn another_app_fullscreen() -> bool {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
    (|| -> Option<bool> {
        let (conn, screen_num) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots[screen_num].root;
        let active = conn
            .intern_atom(false, b"_NET_ACTIVE_WINDOW").ok()?.reply().ok()?.atom;
        let state = conn
            .intern_atom(false, b"_NET_WM_STATE").ok()?.reply().ok()?.atom;
        let fullscreen = conn
            .intern_atom(false, b"_NET_WM_STATE_FULLSCREEN").ok()?.reply().ok()?.atom;
        let win = conn
            .get_property(false, root, active, AtomEnum::WINDOW, 0, 1).ok()?
            .reply().ok()?
            .value32()?.next()?;
        if win == 0 {
            return Some(false);
        }
        let st = conn
            .get_property(false, win, state, AtomEnum::ATOM, 0, 64).ok()?
            .reply().ok()?;
        Some(st.value32().map(|mut it| it.any(|a| a == fullscreen)).unwrap_or(false))
    })()
    .unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
pub fn another_app_fullscreen() -> bool {
    false
}

/// Play a short completion chime at the given volume (0–100). Best-effort via a
/// system player; silently does nothing if none is present.
#[cfg(target_os = "linux")]
pub fn play_sound(volume: u32) {
    let pct = volume.min(100);
    // canberra event sound first (respects the desktop's sound theme)
    if std::process::Command::new("canberra-gtk-play")
        .args(["-i", "complete", "-V", &(pct as f32 / 100.0).to_string()])
        .spawn()
        .is_ok()
    {
        return;
    }
    // fall back to paplay of the freedesktop sound with a PulseAudio volume
    let vol = (pct as f32 / 100.0 * 65536.0) as u32;
    let _ = std::process::Command::new("paplay")
        .arg(format!("--volume={vol}"))
        .arg("/usr/share/sounds/freedesktop/stereo/complete.oga")
        .spawn();
}

#[cfg(not(target_os = "linux"))]
pub fn play_sound(_volume: u32) {}
