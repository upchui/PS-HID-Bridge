import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import PS5HIDBridge

Kirigami.ScrollablePage {
    title: "Devices"

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Controls.Label {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            text: "These devices are forwarded to the PS5 while the bridge is on. Other devices stay with the desktop."
        }

        DeviceList {
            Layout.fillWidth: true
        }

        Kirigami.Separator { Layout.fillWidth: true }

        KeyboardTypeChooser {
            Layout.fillWidth: true
        }
    }

    Component.onCompleted: AppController.refreshDevices()
}
