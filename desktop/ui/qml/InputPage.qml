import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// Devices that go to the PlayStation, keyboard type and mouse settings.
AppPage {
    id: page
    title: "Input"
    subtitle: "Selected devices go to the PlayStation while the bridge is on; all others stay with the desktop."

    Component.onCompleted: AppController.refreshDevices()

    Card {
        title: "Devices"
        icon: "input"
        DeviceList {
            Layout.fillWidth: true
        }
    }

    Card {
        title: "Keyboard type"
        icon: "keyboard"
        KeyboardTypeChooser {}
    }

    Card {
        title: "Mouse"
        icon: "mouse"
        help: "The default is the raw mouse values (sensitivity 1, no acceleration). Many PlayStation games have their own settings; adjusting there is usually better."
        actions: AppButton {
            text: "Raw values"
            flat: true
            enabled: sens.value !== 1 || accel.value !== 0
            onClicked: {
                sens.value = 1;
                accel.value = 0;
                AppController.setMouse(1, 0);
            }
        }

        // Applied shortly after the slider stops moving.
        Timer {
            id: applyMouse
            interval: 250
            onTriggered: AppController.setMouse(sens.value, accel.value)
        }

        GridLayout {
            Layout.fillWidth: true
            columns: 3
            columnSpacing: 12
            Controls.Label {
                text: "Sensitivity"
                color: Style.text
            }
            Controls.Slider {
                id: sens
                Layout.fillWidth: true
                from: 0.25
                to: 4
                stepSize: 0.05
                value: AppController.sensitivity
                onMoved: applyMouse.restart()
            }
            Controls.Label {
                Layout.preferredWidth: 56
                text: sens.value.toFixed(2) + "×"
                color: Style.muted
                horizontalAlignment: Text.AlignRight
            }
            Controls.Label {
                text: "Acceleration"
                color: Style.text
            }
            Controls.Slider {
                id: accel
                Layout.fillWidth: true
                from: 0
                to: 0.5
                stepSize: 0.01
                value: AppController.accel
                onMoved: applyMouse.restart()
            }
            Controls.Label {
                Layout.preferredWidth: 56
                text: accel.value === 0 ? "off" : accel.value.toFixed(2)
                color: Style.muted
                horizontalAlignment: Text.AlignRight
            }
        }

        CurvePlot {
            Layout.fillWidth: true
            implicitHeight: 150
            sensitivity: sens.value
            accel: accel.value
        }
    }
}
