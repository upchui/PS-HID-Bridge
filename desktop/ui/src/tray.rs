//! Tray icon (StatusNotifierItem via ksni). The icon is drawn in code so
//! the AppImage does not depend on an icon theme: grey = board not
//! reachable, blue = ready, green = bridge active.

use cxx_qt::CxxQtThread;
use ksni::blocking::TrayMethods;
use ksni::menu::StandardItem;

use crate::controller::qobject::AppController;

pub struct BridgeTray {
    pub active: bool,
    pub connected: bool,
    qt: CxxQtThread<AppController>,
}

pub fn spawn(qt: CxxQtThread<AppController>) -> Option<ksni::blocking::Handle<BridgeTray>> {
    let tray = BridgeTray {
        active: false,
        connected: false,
        qt,
    };
    match tray.spawn() {
        Ok(h) => Some(h),
        Err(e) => {
            eprintln!("Tray not available: {e}");
            None
        }
    }
}

/// Rounded square with a dot, ARGB32 big-endian as the SNI spec wants.
fn draw_icon(size: i32, fill: [u8; 3], dot: [u8; 3]) -> ksni::Icon {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    let r = size as f32 * 0.22;
    let c = size as f32 / 2.0;
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // distance outside the rounded rectangle [1, size-1]
            let dx = (fx - c).abs() - (c - 1.0 - r);
            let dy = (fy - c).abs() - (c - 1.0 - r);
            let outside = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() - r;
            let in_dot = ((fx - c).powi(2) + (fy - c).powi(2)).sqrt() < size as f32 * 0.2;
            let (a, rgb) = if outside > 0.5 {
                (0u8, [0, 0, 0])
            } else if in_dot {
                (255, dot)
            } else {
                (255, fill)
            };
            data.extend_from_slice(&[a, rgb[0], rgb[1], rgb[2]]);
        }
    }
    ksni::Icon {
        width: size,
        height: size,
        data,
    }
}

impl ksni::Tray for BridgeTray {
    const MENU_ON_ACTIVATE: bool = false;

    fn id(&self) -> String {
        "io.github.thealhu.ps5hidbridge".into()
    }

    fn title(&self) -> String {
        if self.active {
            "PS5 HID Bridge – bridge on".into()
        } else if self.connected {
            "PS5 HID Bridge – ready".into()
        } else {
            "PS5 HID Bridge – board not reachable".into()
        }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let fill = if self.active {
            [0x2e, 0x9d, 0x5b]
        } else if self.connected {
            [0x1d, 0x5b, 0xd8]
        } else {
            [0x7a, 0x82, 0x90]
        };
        [16, 22, 32, 48]
            .iter()
            .map(|&s| draw_icon(s, fill, [0xff, 0xff, 0xff]))
            .collect()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.qt.queue(|obj| obj.toggle());
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
            StandardItem {
                label: if self.active {
                    "Bridge off".into()
                } else {
                    "Bridge on".into()
                },
                activate: Box::new(|t: &mut Self| {
                    let _ = t.qt.queue(|obj| obj.toggle());
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Release all keys".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.qt.queue(|obj| obj.release_all());
                }),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.qt.queue(|obj| obj.quit_app());
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}
