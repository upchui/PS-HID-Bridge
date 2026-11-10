import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import PS5HIDBridge

Kirigami.ScrollablePage {
    id: page
    title: "Board"

    property string error: ""
    property bool saved: false

    function host() {
        const a = AppController.boardAddress;
        return a.indexOf(":") >= 0 ? a.substring(0, a.lastIndexOf(":")) : a;
    }

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Kirigami.FormLayout {
            Layout.fillWidth: true

            Controls.TextField {
                id: address
                Kirigami.FormData.label: "Address:"
                Layout.preferredWidth: Kirigami.Units.gridUnit * 16
                text: AppController.boardAddress
                placeholderText: "e.g. 192.168.1.50"
            }
            Controls.TextField {
                id: psk
                Kirigami.FormData.label: "Pre-shared key:"
                Layout.preferredWidth: Kirigami.Units.gridUnit * 24
                echoMode: TextInput.Password
                placeholderText: AppController.pskSet ? "stored – fill in only to change it" : "64 hex digits from the web interface"
            }
        }

        RowLayout {
            Controls.Button {
                text: "Save and connect"
                highlighted: true
                onClicked: {
                    page.error = AppController.saveBoard(address.text, psk.text);
                    page.saved = page.error === "";
                    if (page.saved)
                        psk.text = "";
                }
            }
            Controls.Button {
                text: "Open web interface"
                enabled: AppController.boardAddress !== ""
                onClicked: Qt.openUrlExternally("http://" + page.host() + "/")
            }
        }

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            visible: page.error !== ""
            type: Kirigami.MessageType.Error
            text: page.error
        }
        Kirigami.InlineMessage {
            Layout.fillWidth: true
            visible: page.saved && page.error === ""
            type: AppController.connected ? Kirigami.MessageType.Positive : Kirigami.MessageType.Information
            text: AppController.connected ? "Connected." : "Saved. Connecting …"
        }

        Kirigami.FormLayout {
            Layout.fillWidth: true
            Controls.Label {
                Kirigami.FormData.label: "Connection:"
                text: AppController.connected ? "connected" : "not reachable"
            }
            Controls.Label {
                Kirigami.FormData.label: "Firmware:"
                text: AppController.boardFirmware !== "" ? AppController.boardFirmware : "–"
            }
            Controls.Label {
                Kirigami.FormData.label: "PS5:"
                text: AppController.boardUsb !== "" ? AppController.boardUsb : "–"
            }
            Controls.Label {
                Kirigami.FormData.label: "Desktop app:"
                text: "Version " + AppController.version
            }
        }

        Controls.Label {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            opacity: 0.8
            text: "The board's web interface creates the key under \"Security\". Your router shows the address (device name ps5-hid-bridge-…)."
        }
    }
}
