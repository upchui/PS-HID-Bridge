import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// Connection to the board: address and key.
AppPage {
    id: page
    title: "Board"
    subtitle: "The small board at the PlayStation's USB port, reached over the network."

    property string error: ""
    property bool saved: false

    function host() {
        const a = AppController.boardAddress;
        return a.indexOf(":") >= 0 ? a.substring(0, a.lastIndexOf(":")) : a;
    }

    Banner {
        kind: "error"
        visible: page.error !== ""
        text: page.error
    }
    Banner {
        kind: AppController.connected ? "ok" : "info"
        visible: page.saved && page.error === ""
        text: AppController.connected ? "Connected."
              : AppController.lastError !== "" ? "Saved. " + AppController.lastError
              : "Saved. Connecting …"
    }

    Card {
        title: "Connection"
        icon: "network"
        help: "Your router lists the board as ps-hid-bridge-… with its address. The key is created in the board's web interface under \"Security\"."
        actions: Pill {
            level: AppController.boardAddress === "" ? "" : AppController.connected ? "ok" : "bad"
            text: AppController.boardAddress === "" ? "not set up" : AppController.connected ? "connected" : "not reachable"
        }

        GridLayout {
            Layout.fillWidth: true
            columns: 3
            columnSpacing: 12
            rowSpacing: 8
            Controls.Label {
                text: "Address"
                color: Style.text
            }
            Controls.TextField {
                id: address
                Layout.fillWidth: true
                text: AppController.boardAddress
                placeholderText: "e.g. 192.168.1.50"
            }
            Item {
                implicitWidth: showKey.implicitWidth
            }
            Controls.Label {
                text: "Key"
                color: Style.text
            }
            Controls.TextField {
                id: psk
                Layout.fillWidth: true
                echoMode: showKey.checked ? TextInput.Normal : TextInput.Password
                placeholderText: AppController.pskSet ? "stored – fill in only to change it" : "64 hex digits from the web interface"
            }
            AppButton {
                id: showKey
                checkable: true
                flat: true
                icon.name: "eye"
                icon.color: checked ? Style.accent : Style.muted
                Accessible.name: "Show key"
                Controls.ToolTip.visible: hovered
                Controls.ToolTip.text: checked ? "Hide key" : "Show key"
            }
        }

        RowLayout {
            spacing: 8
            AppButton {
                text: "Save and connect"
                highlighted: true
                onClicked: {
                    page.error = AppController.saveBoard(address.text, psk.text);
                    page.saved = page.error === "";
                    if (page.saved)
                        psk.text = "";
                }
            }
            AppButton {
                text: "Open web interface"
                icon.name: "external"
                icon.color: Style.text
                enabled: AppController.boardAddress !== ""
                onClicked: Qt.openUrlExternally("http://" + page.host() + "/")
            }
        }
    }

    Card {
        title: "Details"
        icon: "board"
        GridLayout {
            Layout.fillWidth: true
            columns: 2
            columnSpacing: 24
            rowSpacing: 6
            Repeater {
                model: [
                    ["Firmware", AppController.boardFirmware !== "" ? AppController.boardFirmware : "–"],
                    ["PlayStation", AppController.boardUsb !== "" ? AppController.boardUsb : "–"],
                    ["Round trip", AppController.rttMs >= 0 ? AppController.rttMs.toFixed(2) + " ms" : "–"],
                    ["Desktop app", AppController.version]
                ]
                delegate: RowLayout {
                    required property var modelData
                    Layout.columnSpan: 1
                    spacing: 12
                    Controls.Label {
                        Layout.preferredWidth: 110
                        text: parent.modelData[0]
                        color: Style.muted
                    }
                    Controls.Label {
                        text: parent.modelData[1]
                        color: Style.text
                    }
                }
            }
        }
    }
}
