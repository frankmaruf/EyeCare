// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // On a Linux Wayland session, GTK's keep-above (always-on-top) is a no-op,
    // so the floating widget can't stay above other windows. Forcing the X11
    // backend (via XWayland) restores always-on-top through the compositor.
    // Honors an explicit GDK_BACKEND if the user set one.
    #[cfg(target_os = "linux")]
    {
        let is_wayland = std::env::var("XDG_SESSION_TYPE")
            .map(|t| t.eq_ignore_ascii_case("wayland"))
            .unwrap_or(false);
        if is_wayland && std::env::var_os("GDK_BACKEND").is_none() {
            std::env::set_var("GDK_BACKEND", "x11");
        }
    }

    eyebreak_lib::run()
}
