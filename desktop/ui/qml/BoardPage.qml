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

    // Not set up yet: look for the board right away.
    Component.onCompleted: if (AppController.boardAddress === "") finder.search()

    // The address without a port, ready for a URL: "host:port" -> "host",
    // "[fe80::1]:45045" -> "[fe80::1]", a bare IPv6 address gets brackets.
    function host() {
        const a = AppController.boardAddress;
        if (a.startsWith("[")) {
            const end = a.indexOf("]");
            return end > 0 ? a.substring(0, end + 1) : a;
        }
        const colons = a.split(":").length - 1;
        if (colons > 1)
            return "[" + a + "]";
        return colons === 1 ? a.substring(0, a.indexOf(":")) : a;
    }

    Banner {
        kind: "error"
        visible: page.error !== ""
        text: page.error
    }
    Banner {
        kind: AppController.connected ? "ok" : AppController.lastError !== "" ? "warn" : "info"
        visible: page.saved && page.error === ""
        text: AppController.connected ? "Connected."
              : AppController.lastError !== "" ? "Saved. " + AppController.lastError
              : "Saved. Connecting …"
    }

    Card {
        title: "Connection"
        icon: "network"
        help: "The app finds boards in this network with current firmware; otherwise your router lists the board as ps-hid-bridge-… with its address. The key is created in the board's web interface under \"Security\"."
        actions: Pill {
            readonly property bool connecting: AppController.statusText === "Connecting …"
            level: AppController.boardAddress === "" || connecting ? "" : AppController.connected ? "ok" : "bad"
            text: AppController.boardAddress === "" ? "not set up" : AppController.connected ? "connected"
                  : connecting ? "connecting …" : "not reachable"
        }

        BoardFinder {
            id: finder
            Layout.fillWidth: true
            selectedAddress: address.text.trim()
            onPicked: a => address.text = a
            // The only board found, while nothing is entered yet: take it.
            onBoardsChanged: if (searched && boards.length === 1 && address.text.trim() === "")
                                 address.text = boards[0].address
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
        id: nameCard
        title: "Name"
        icon: "board"
        help: "Shown in the app's search and in the board's web interface. Your router and the browser use a network name made from it (letters, digits and dashes); the board keeps answering to its old name ps-hid-bridge-… too."

        // The board's default name, from its MAC address ("" = not known).
        readonly property string defaultName: {
            const m = AppController.boardMac.split(":");
            return m.length === 6 ? "ps-hid-bridge-" + m.slice(3).join("") : "";
        }
        function rename() {
            AppController.renameBoard(nameField.text);
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 8
            Controls.TextField {
                id: nameField
                Layout.fillWidth: true
                text: AppController.boardName
                maximumLength: 31
                placeholderText: AppController.connected ? "e.g. Living room" : "Connect to the board first"
                enabled: AppController.connected && !AppController.renameBusy
                onAccepted: if (renameButton.enabled) nameCard.rename()
            }
            AppButton {
                id: renameButton
                text: AppController.renameBusy ? "Renaming …" : "Rename"
                enabled: AppController.connected && !AppController.renameBusy
                         && nameField.text.trim() !== AppController.boardName
                onClicked: nameCard.rename()
            }
        }
        Controls.Label {
            Layout.fillWidth: true
            visible: AppController.connected
            text: "In your router and the browser: "
                  + AppController.networkNameFor(nameField.text, nameCard.defaultName !== "" ? nameCard.defaultName : "ps-hid-bridge-…")
            color: Style.muted
            wrapMode: Text.WordWrap
        }
        Banner {
            kind: AppController.renameLevel === "ok" ? "ok" : "warn"
            visible: AppController.renameMessage !== ""
            text: AppController.renameMessage
        }
    }

    Card {
        title: "Details"
        icon: "board"
        GridLayout {
            id: details
            Layout.fillWidth: true
            // Two pairs per line where they fit, else one.
            columns: width > 480 ? 2 : 1
            columnSpacing: 24
            rowSpacing: 6
            Repeater {
                model: [
                    ["Firmware", AppController.boardFirmware !== "" ? AppController.boardFirmware : "–"],
                    ["PlayStation", AppController.boardUsb !== "" ? Style.playStation(AppController.boardUsb) : "–"],
                    ["Round trip", AppController.rttMs >= 0 ? Style.us(AppController.rttMs * 1000) : "–"],
                    ["Desktop app", AppController.version]
                ]
                delegate: RowLayout {
                    required property var modelData
                    Layout.fillWidth: true
                    Layout.preferredWidth: details.columns > 1 ? (details.width - details.columnSpacing) / 2 : details.width
                    spacing: 12
                    Controls.Label {
                        Layout.preferredWidth: 110
                        text: parent.modelData[0]
                        color: Style.muted
                    }
                    Controls.Label {
                        Layout.fillWidth: true
                        text: parent.modelData[1]
                        color: Style.text
                        elide: Text.ElideRight
                    }
                }
            }
        }
    }
}
