//! PS5 HID Bridge desktop application (Qt 6 / Kirigami via cxx-qt).
//!
//! `--self-test` loads the whole QML scene, then quits; any QML warning is
//! fatal in that mode, so a missing module in the AppImage fails the test.

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QString, QUrl};

const MAIN_QML: &str = "qrc:/qt/qml/PS5HIDBridge/qml/Main.qml";

fn main() {
    let self_test = std::env::args().any(|a| a == "--self-test");

    // SAFETY: still single-threaded; Qt reads these during startup.
    unsafe {
        if std::env::var_os("QT_QUICK_CONTROLS_STYLE").is_none() {
            std::env::set_var("QT_QUICK_CONTROLS_STYLE", "Fusion");
        }
        if self_test {
            std::env::set_var("QT_FATAL_WARNINGS", "1");
        }
    }

    let mut app = QGuiApplication::new();
    let mut engine = QQmlApplicationEngine::new();

    if let Some(app) = app.as_mut() {
        app.set_application_name(&QString::from("ps5-hid-bridge"));
        app.set_application_version(&QString::from(pshb_core::VERSION));
    }

    if let Some(mut engine) = engine.as_mut() {
        engine
            .as_mut()
            .on_object_creation_failed(|_, url| {
                eprintln!("QML konnte nicht geladen werden: {url}");
                std::process::exit(1);
            })
            .release();
        engine.load(&QUrl::from(MAIN_QML));
    }

    let code = app.as_mut().map(|app| app.exec()).unwrap_or(1);
    std::process::exit(code);
}
