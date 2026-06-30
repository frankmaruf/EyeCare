// Linux system tray via StatusNotifierItem (ksni, over D-Bus). No gtk, so it
// keeps the process light. Menu clicks happen on ksni's own thread, so they're
// forwarded as `Action`s over a channel; the main (Slint) thread drains them
// and touches UI/state there.

use std::sync::mpsc::Sender;

#[derive(Clone, Copy)]
pub enum Action {
    Open,
    Settings,
    Pause,
    Take,
    Skip,
    ToggleWidget,
    Quit,
}

pub struct EyeTray {
    tx: Sender<Action>,
    /// live countdown shown in the tray tooltip (§4.11)
    pub status: String,
}

impl EyeTray {
    fn send(&self, a: Action) {
        let _ = self.tx.send(a);
    }
}

impl ksni::Tray for EyeTray {
    fn id(&self) -> String {
        "us.frankmaruf.eyecare-native".into()
    }

    fn title(&self) -> String {
        "EyeCare".into()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "EyeCare".into(),
            description: self.status.clone(),
            icon_name: String::new(),
            icon_pixmap: Vec::new(),
        }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        icon_pixmap()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(Action::Open); // left-click opens the dashboard
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        vec![
            StandardItem {
                label: "Open EyeCare".into(),
                activate: Box::new(|t: &mut EyeTray| t.send(Action::Open)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Open settings".into(),
                activate: Box::new(|t: &mut EyeTray| t.send(Action::Settings)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Pause / Resume".into(),
                activate: Box::new(|t: &mut EyeTray| t.send(Action::Pause)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Take a break now".into(),
                activate: Box::new(|t: &mut EyeTray| t.send(Action::Take)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Skip break".into(),
                activate: Box::new(|t: &mut EyeTray| t.send(Action::Skip)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Hide / show widget".into(),
                activate: Box::new(|t: &mut EyeTray| t.send(Action::ToggleWidget)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|t: &mut EyeTray| t.send(Action::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Decode the embedded icon into the ARGB pixmap ksni expects.
fn icon_pixmap() -> Vec<ksni::Icon> {
    let bytes: &[u8] = include_bytes!("../ui/icon.png");
    let decoder = png::Decoder::new(bytes);
    let Ok(mut reader) = decoder.read_info() else {
        return Vec::new();
    };
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let Ok(info) = reader.next_frame(&mut buf) else {
        return Vec::new();
    };
    // Expect RGBA8; convert to ARGB byte order.
    if info.color_type != png::ColorType::Rgba {
        return Vec::new();
    }
    let mut argb = Vec::with_capacity(buf.len());
    for px in buf[..info.buffer_size()].chunks_exact(4) {
        argb.extend_from_slice(&[px[3], px[0], px[1], px[2]]);
    }
    vec![ksni::Icon {
        width: info.width as i32,
        height: info.height as i32,
        data: argb,
    }]
}

/// Start the tray on its own thread. Returns a handle that must be kept alive
/// for the tray to stay registered.
pub fn spawn(tx: Sender<Action>) -> ksni::Handle<EyeTray> {
    let service = ksni::TrayService::new(EyeTray {
        tx,
        status: String::new(),
    });
    let handle = service.handle();
    service.spawn();
    handle
}
