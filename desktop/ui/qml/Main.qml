import QtQuick
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami

Kirigami.ApplicationWindow {
    id: root

    title: "PS5 HID Bridge"
    width: Kirigami.Units.gridUnit * 45
    height: Kirigami.Units.gridUnit * 30

    pageStack.initialPage: Kirigami.Page {
        title: "Status"

        Kirigami.PlaceholderMessage {
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.gridUnit * 4
            text: "PS5 HID Bridge"
            explanation: "Das Grundgerüst steht. Verbindung, Geräte und Auslöser folgen."
        }
    }

    Component.onCompleted: {
        if (Qt.application.arguments.indexOf("--self-test") >= 0) {
            Qt.callLater(Qt.quit)
        }
    }
}
