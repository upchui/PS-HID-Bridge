use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    // Link with lld: cxx-qt would otherwise get GNU gold, which emits two
    // .init_array sections -- the loader runs only one, so the static
    // initializers that register the QML module's resources never run.
    // (Only this binary: the CLI and the tests keep the default linker, so
    // all of them can share one target directory.)
    println!("cargo:rustc-link-arg-bins=-fuse-ld=lld");
    CxxQtBuilder::new_qml_module(QmlModule::new("PS5HIDBridge").qml_files([
        "qml/Main.qml",
        "qml/StatusPage.qml",
        "qml/DevicesPage.qml",
        "qml/TriggersPage.qml",
        "qml/MousePage.qml",
        "qml/BoardPage.qml",
        "qml/WizardPage.qml",
        "qml/DeviceList.qml",
        "qml/TriggerEditor.qml",
        "qml/CurvePlot.qml",
        "qml/KeyboardTypeChooser.qml",
    ]))
    .files(["src/controller.rs"])
    .qt_module("Quick")
    .qt_module("QuickControls2")
    .build();
}
