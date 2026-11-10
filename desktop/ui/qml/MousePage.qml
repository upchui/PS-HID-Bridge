import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import PS5HIDBridge

Kirigami.ScrollablePage {
    title: "Maus"

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Controls.Label {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            text: "Standard sind die rohen Werte der Maus (Empfindlichkeit 1, keine Beschleunigung). Viele Spiele auf der PS5 haben eigene Einstellungen; dort anzupassen ist meist besser."
        }

        Kirigami.FormLayout {
            Layout.fillWidth: true

            RowLayout {
                Kirigami.FormData.label: "Empfindlichkeit:"
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
                Kirigami.FormData.label: "Beschleunigung:"
                Controls.Slider {
                    id: accel
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 14
                    from: 0
                    to: 0.5
                    stepSize: 0.01
                    value: AppController.accel
                }
                Controls.Label { text: accel.value === 0 ? "aus" : accel.value.toFixed(2) }
            }
        }

        CurvePlot {
            Layout.fillWidth: true
            sensitivity: sens.value
            accel: accel.value
        }

        RowLayout {
            Controls.Button {
                text: "Übernehmen"
                highlighted: true
                onClicked: AppController.setMouse(sens.value, accel.value)
            }
            Controls.Button {
                text: "Rohe Werte"
                onClicked: {
                    sens.value = 1;
                    accel.value = 0;
                    AppController.setMouse(1, 0);
                }
            }
        }
    }
}
