//! PS HID Bridge desktop application (Qt 6 / Kirigami via cxx-qt).
//!
//! Options:
//!   --self-test            load every page offscreen, then quit (warnings are fatal)
//!   --screenshot DIR       write a PNG of every page into DIR, then quit

mod controller;
mod focus;
mod notify;
mod tray;

use cxx_qt_lib::{QByteArray, QGuiApplication, QQmlApplicationEngine, QString, QUrl};

// Instantiate the window through the module (resolves via the qmldir in the
// compiled-in resources, like QQmlApplicationEngine::loadFromModule).
const MAIN_QML: &str = "import PSHIDBridge\nMain {}\n";

/// Held while this instance runs (released early for a restart after an update).
static INSTANCE_LOCK: std::sync::Mutex<Option<std::fs::File>> = std::sync::Mutex::new(None);

/// One instance per user: a second one would fight over the devices, the
/// packet counter and the KWin script, and show a second tray icon.
fn single_instance() {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    match pshb_core::lockfile::exclusive(&dir.join("ps-hid-bridge.lock")) {
        Ok(lock) => *INSTANCE_LOCK.lock().unwrap() = Some(lock),
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
            eprintln!("PS HID Bridge is already running.");
            notify::send_now("PS HID Bridge is already running", "Open it from its tray icon.");
            std::process::exit(0);
        }
        Err(_) => {} // no lock possible: start anyway
    }
}

/// Let a new instance start (restart after an update).
pub fn release_instance_lock() {
    INSTANCE_LOCK.lock().unwrap().take();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let self_test = args.iter().any(|a| a == "--self-test");
    let screenshot = args.iter().any(|a| a == "--screenshot");
    if !self_test && !screenshot {
        single_instance();
    }

    // SAFETY: still single-threaded; Qt reads these during startup.
    unsafe {
        if std::env::var_os("QT_QUICK_CONTROLS_STYLE").is_none() {
            std::env::set_var("QT_QUICK_CONTROLS_STYLE", "Fusion");
        }
        if self_test || screenshot {
            if std::env::var_os("QT_QPA_PLATFORM").is_none() {
                std::env::set_var("QT_QPA_PLATFORM", "offscreen");
            }
            std::env::set_var("QT_QUICK_BACKEND", "software");
        }
        if self_test {
            std::env::set_var("QT_FATAL_WARNINGS", "1");
        }
    }

    // Register our QML module (QML files, AppController) with the engine.
    cxx_qt::init_qml_module!("PSHIDBridge");

    let mut app = QGuiApplication::new();
    let mut engine = QQmlApplicationEngine::new();

    if let Some(mut app) = app.as_mut() {
        app.as_mut().set_application_name(&QString::from("ps-hid-bridge"));
        app.as_mut()
            .set_application_version(&QString::from(pshb_core::VERSION));
        app.as_mut()
            .set_organization_name(&QString::from("ps-hid-bridge"));
    }
    QGuiApplication::set_desktop_file_name(&QString::from("io.github.thealhu.pshidbridge"));

    if let Some(mut engine) = engine.as_mut() {
        engine
            .as_mut()
            .on_object_creation_failed(|_, url| {
                eprintln!("Could not load QML: {url}");
                std::process::exit(1);
            })
            .release();
        // PSHB_STARTUP_QML: replace the startup document (debugging only).
        let startup = std::env::var("PSHB_STARTUP_QML").unwrap_or_else(|_| MAIN_QML.to_string());
        engine.load_data(
            &QByteArray::from(startup.as_str()),
            &QUrl::from("qrc:/ps-hid-bridge/startup.qml"),
        );
    }

    let code = app.as_mut().map(|app| app.exec()).unwrap_or(1);
    // Destroy the engine (and with it the AppController singleton) first: its
    // service releases every key on the board and ungrabs the devices.
    drop(engine);
    drop(app);
    std::process::exit(code);
}
