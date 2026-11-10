import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import PS5HIDBridge

Kirigami.ScrollablePage {
    title: "Mouse"

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Controls.Label {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            text: "The default is the raw mouse values (sensitivity 1, no acceleration). Many PS5 games have their own settings; adjusting there is usually better."
        }

        Kirigami.FormLayout {
            Layout.fillWidth: true

            RowLayout {
                Kirigami.FormData.label: "Sensitivity:"
                Controls.Slider {
                    id: sens
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 14
                    from: 0.25
                    to: 4
                    stepSize: 0.05
                    value: AppController.sensitivity
                }
                Controls.Label { text: sens.value.toFixed(2) + "×" }
            }
            RowLayout {
                Kirigami.FormData.label: "Acceleration:"
                Controls.Slider {
                    id: accel
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 14
                    from: 0
                    to: 0.5
                    stepSize: 0.01
                    value: AppController.accel
                }
                Controls.Label { text: accel.value === 0 ? "off" : accel.value.toFixed(2) }
            }
        }

        CurvePlot {
            Layout.fillWidth: true
            sensitivity: sens.value
            accel: accel.value
        }

        RowLayout {
            Controls.Button {
                text: "Apply"
                highlighted: true
                onClicked: AppController.setMouse(sens.value, accel.value)
            }
            Controls.Button {
                text: "Raw values"
                onClicked: {
                    sens.value = 1;
                    accel.value = 0;
                    AppController.setMouse(1, 0);
                }
            }
        }
    }
}
