use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
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
