import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import PS5HIDBridge

Kirigami.ScrollablePage {
    title: "Geräte"

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Controls.Label {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            text: "Diese Geräte werden an die PS5 weitergeleitet, solange die Steuerung an ist. Andere Geräte bleiben beim Desktop."
        }

        DeviceList {
            Layout.fillWidth: true
        }
    }

    Component.onCompleted: AppController.refreshDevices()
}
