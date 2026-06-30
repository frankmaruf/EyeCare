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

/// Tell the WM to keep this X11 window out of the taskbar + pager (so the widget
/// lives only in the tray, not the task switcher). `xid` is the Xlib window.
///
/// Sets it two ways so it sticks whether or not the window is mapped yet:
/// appends the atoms to the `_NET_WM_STATE` property (read by the WM at map time,
/// Append so it doesn't clobber `_NET_WM_STATE_ABOVE`) AND sends the live
/// ClientMessage (honored once mapped).
#[cfg(target_os = "linux")]
pub fn x11_skip_taskbar(xid: u32) {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        AtomEnum, ClientMessageEvent, ConnectionExt, EventMask, PropMode, CLIENT_MESSAGE_EVENT,
    };
    use x11rb::wrapper::ConnectionExt as _; // change_property32
    let _ = (|| -> Option<()> {
        let (conn, screen) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots[screen].root;
        let wm_state = conn.intern_atom(false, b"_NET_WM_STATE").ok()?.reply().ok()?.atom;
        let skip_tb = conn
            .intern_atom(false, b"_NET_WM_STATE_SKIP_TASKBAR").ok()?.reply().ok()?.atom;
        let skip_pg = conn
            .intern_atom(false, b"_NET_WM_STATE_SKIP_PAGER").ok()?.reply().ok()?.atom;
        // (a) append to the property (pre-map case)
        conn.change_property32(
            PropMode::APPEND,
            xid,
            wm_state,
            AtomEnum::ATOM,
            &[skip_tb, skip_pg],
        )
        .ok()?;
        // (b) live ClientMessage (already-mapped case)
        for atom in [skip_tb, skip_pg] {
            let ev = ClientMessageEvent {
                response_type: CLIENT_MESSAGE_EVENT,
                format: 32,
                sequence: 0,
                window: xid,
                type_: wm_state,
                data: [1, atom, 0, 1, 0].into(), // _NET_WM_STATE_ADD, source=app
            };
            conn.send_event(
                false,
                root,
                EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
                ev,
            )
            .ok()?;
        }
        conn.flush().ok()?;
        Some(())
    })();
}

#[cfg(not(target_os = "linux"))]
pub fn x11_skip_taskbar(_xid: u32) {}


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
