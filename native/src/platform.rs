// Platform integration helpers that stay out of main.rs: fullscreen/presentation
// detection (§9.9), taskbar/WM-state control, and sound playback (§4.6).
// Everything degrades to a safe no-op when the platform bits aren't available
// (the doc's "graceful fallback").
//
// All X11 helpers share ONE cached connection with pre-interned atoms (see
// `xconn`) — they're called from timers (every 150ms–1s), and opening a fresh
// X connection + re-interning atoms per call was several needless round trips
// each tick.

#[cfg(target_os = "linux")]
mod xconn {
    use std::cell::RefCell;

    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{Atom, ConnectionExt};
    use x11rb::rust_connection::RustConnection;

    pub struct Ctx {
        pub conn: RustConnection,
        pub root: u32,
        pub net_active: Atom,
        pub wm_state: Atom,
        pub fullscreen: Atom,
        pub hidden: Atom,
        pub skip_tb: Atom,
        pub skip_pg: Atom,
    }

    thread_local! {
        static CTX: RefCell<Option<Ctx>> = RefCell::new(None);
    }

    fn connect() -> Option<Ctx> {
        let (conn, screen) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots[screen].root;
        let intern = |name: &[u8]| -> Option<Atom> {
            Some(conn.intern_atom(false, name).ok()?.reply().ok()?.atom)
        };
        Some(Ctx {
            root,
            net_active: intern(b"_NET_ACTIVE_WINDOW")?,
            wm_state: intern(b"_NET_WM_STATE")?,
            fullscreen: intern(b"_NET_WM_STATE_FULLSCREEN")?,
            hidden: intern(b"_NET_WM_STATE_HIDDEN")?,
            skip_tb: intern(b"_NET_WM_STATE_SKIP_TASKBAR")?,
            skip_pg: intern(b"_NET_WM_STATE_SKIP_PAGER")?,
            conn,
        })
    }

    /// Run `f` against the cached connection, (re)connecting on demand. A `None`
    /// from `f` is treated as a broken/raced connection: drop it and reconnect on
    /// the next call. Stray events/errors are drained so they can't accumulate.
    pub fn with<R>(f: impl FnOnce(&Ctx) -> Option<R>) -> Option<R> {
        CTX.with(|cell| {
            let mut slot = cell.borrow_mut();
            if slot.is_none() {
                *slot = connect();
            }
            let result = slot.as_ref().and_then(f);
            if let Some(ctx) = slot.as_ref() {
                while let Ok(Some(_)) = ctx.conn.poll_for_event() {}
            }
            if result.is_none() {
                *slot = None;
            }
            result
        })
    }
}

/// Is some *other* application currently fullscreen? Used to hush non-forced
/// reminders and hide the widget during a screen-share / presentation / video.
#[cfg(target_os = "linux")]
pub fn another_app_fullscreen() -> bool {
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
    xconn::with(|c| {
        let win = c
            .conn
            .get_property(false, c.root, c.net_active, AtomEnum::WINDOW, 0, 1)
            .ok()?
            .reply()
            .ok()?
            .value32()?
            .next()?;
        if win == 0 {
            return Some(false);
        }
        let st = c
            .conn
            .get_property(false, win, c.wm_state, AtomEnum::ATOM, 0, 64)
            .ok()?
            .reply()
            .ok()?;
        Some(st.value32().map(|mut it| it.any(|a| a == c.fullscreen)).unwrap_or(false))
    })
    .unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
pub fn another_app_fullscreen() -> bool {
    false
}

/// Send the live _NET_WM_STATE ADD ClientMessages for skip-taskbar/pager
/// (honored once the window is mapped; idempotent if already set).
#[cfg(target_os = "linux")]
fn send_skip_state(c: &xconn::Ctx, xid: u32) -> Option<()> {
    use x11rb::protocol::xproto::{
        ClientMessageEvent, ConnectionExt, EventMask, CLIENT_MESSAGE_EVENT,
    };
    for atom in [c.skip_tb, c.skip_pg] {
        let ev = ClientMessageEvent {
            response_type: CLIENT_MESSAGE_EVENT,
            format: 32,
            sequence: 0,
            window: xid,
            type_: c.wm_state,
            data: [1, atom, 0, 1, 0].into(), // _NET_WM_STATE_ADD, source=app
        };
        c.conn
            .send_event(
                false,
                c.root,
                EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
                ev,
            )
            .ok()?;
    }
    use x11rb::connection::Connection;
    c.conn.flush().ok()?;
    Some(())
}

/// Tell the WM to keep this X11 window out of the taskbar + pager (so it lives
/// only in the tray, not the task switcher). Sets it two ways so it sticks
/// whether or not the window is mapped yet: appends the atoms to the
/// `_NET_WM_STATE` property (read by the WM at map time; Append so it doesn't
/// clobber `_NET_WM_STATE_ABOVE`) AND sends the live ClientMessage.
#[cfg(target_os = "linux")]
pub fn x11_skip_taskbar(xid: u32) {
    use x11rb::protocol::xproto::{AtomEnum, PropMode};
    use x11rb::wrapper::ConnectionExt as _;
    xconn::with(|c| {
        c.conn
            .change_property32(PropMode::APPEND, xid, c.wm_state, AtomEnum::ATOM, &[
                c.skip_tb, c.skip_pg,
            ])
            .ok()?;
        send_skip_state(c, xid)
    });
}

#[cfg(not(target_os = "linux"))]
pub fn x11_skip_taskbar(_xid: u32) {}

/// Re-assert skip-taskbar/pager on an already-mapped window via the live
/// ClientMessage only. Used to re-add skip-taskbar whenever KWin drops it while
/// mapping the dashboard.
#[cfg(target_os = "linux")]
pub fn x11_skip_taskbar_msg(xid: u32) {
    xconn::with(|c| send_skip_state(c, xid));
}

#[cfg(not(target_os = "linux"))]
pub fn x11_skip_taskbar_msg(_xid: u32) {}

/// One read of a window's `_NET_WM_STATE` → (is it minimized, has skip-taskbar),
/// so the watcher can both catch minimize (HIDDEN) and re-add skip-taskbar in a
/// single X round-trip. None if the state couldn't be read.
#[cfg(target_os = "linux")]
pub fn x11_window_flags(xid: u32) -> Option<(bool, bool)> {
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
    xconn::with(|c| {
        let st = c
            .conn
            .get_property(false, xid, c.wm_state, AtomEnum::ATOM, 0, 64)
            .ok()?
            .reply()
            .ok()?;
        let atoms: Vec<u32> = st.value32().map(|it| it.collect()).unwrap_or_default();
        Some((atoms.contains(&c.hidden), atoms.contains(&c.skip_tb)))
    })
}

#[cfg(not(target_os = "linux"))]
pub fn x11_window_flags(_xid: u32) -> Option<(bool, bool)> {
    None
}

/// Play a short completion chime at the given volume (0–100). Best-effort via a
/// system player; silently does nothing if none is present. The child is reaped
/// on a throwaway thread — dropping it unwaited would leave a zombie per chime.
#[cfg(target_os = "linux")]
pub fn play_sound(volume: u32) {
    let pct = volume.min(100);
    // canberra event sound first (respects the desktop's sound theme)
    let child = std::process::Command::new("canberra-gtk-play")
        .args(["-i", "complete", "-V", &(pct as f32 / 100.0).to_string()])
        .spawn()
        .or_else(|_| {
            // fall back to paplay of the freedesktop sound with a PulseAudio volume
            let vol = (pct as f32 / 100.0 * 65536.0) as u32;
            std::process::Command::new("paplay")
                .arg(format!("--volume={vol}"))
                .arg("/usr/share/sounds/freedesktop/stereo/complete.oga")
                .spawn()
        });
    if let Ok(mut child) = child {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

#[cfg(not(target_os = "linux"))]
pub fn play_sound(_volume: u32) {}
