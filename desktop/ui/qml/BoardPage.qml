import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// Connection to the board: address and key.
AppPage {
    id: page
    title: I18n.tr("Board")
    subtitle: I18n.tr("The small board at the PlayStation's USB port, reached over the network.")

    property string error: ""
    property bool saved: false

    // Not set up yet: look for the board right away.

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
        text: AppController.connected ? I18n.tr("Connected.")
              : AppController.lastError !== "" ? I18n.trf("Saved. %1", AppController.lastError)
              : I18n.tr("Saved. Connecting …")
    }

    Card {
        title: I18n.tr("Connection")
        icon: "network"
        help: I18n.tr("The app finds boards in this network with current firmware; otherwise your router lists the board as ps-hid-bridge-… with its address. The key is created in the board's web interface under \"Security\".")
        actions: Pill {
            readonly property bool connecting: AppController.connecting
            level: AppController.boardAddress === "" || connecting ? "" : AppController.connected ? "ok" : "bad"
            text: AppController.boardAddress === "" ? I18n.tr("not set up") : AppController.connected ? I18n.tr("connected")
                  : connecting ? I18n.tr("connecting …") : I18n.tr("not reachable")
        }

        BoardFinder {
            id: finder
            Layout.fillWidth: true
            selectedAddress: address.text.trim()
            onPicked: a => address.text = a
            // The only board found, while nothing is entered yet: take it.
            onBoardsChanged: if (boards.length === 1 && address.text.trim() === "")
                                 address.text = boards[0].address
        }

        GridLayout {
            Layout.fillWidth: true
            columns: 3
            columnSpacing: 12
            rowSpacing: 8
            Controls.Label {
                text: I18n.tr("Address")
                color: Style.text
            }
            AppTextField {
                id: address
                Layout.fillWidth: true
                text: AppController.boardAddress
                placeholderText: I18n.tr("e.g. 192.168.1.50")
            }
            Item {
                implicitWidth: showKey.implicitWidth
            }
            Controls.Label {
                text: I18n.tr("Key")
                color: Style.text
            }
            AppTextField {
                id: psk
                Layout.fillWidth: true
                echoMode: showKey.checked ? TextInput.Normal : TextInput.Password
                placeholderText: AppController.pskSet ? I18n.tr("stored – fill in only to change it")
                                 : I18n.tr("64 hex digits from the web interface")
            }
            AppButton {
                id: showKey
                checkable: true
                flat: true
                icon.name: "eye"
                icon.color: checked ? Style.accent : Style.muted
                Accessible.name: I18n.tr("Show key")
                Controls.ToolTip.visible: hovered
                Controls.ToolTip.text: checked ? I18n.tr("Hide key") : I18n.tr("Show key")
            }
        }

        // Wraps in a narrow window (the German texts are long).
        Flow {
            Layout.fillWidth: true
            spacing: 8
            AppButton {
                text: I18n.tr("Save and connect")
                highlighted: true
                onClicked: {
                    page.error = AppController.saveBoard(address.text, psk.text);
                    page.saved = page.error === "";
                    if (page.saved)
                        psk.text = "";
                }
            }
            AppButton {
                text: I18n.tr("Open web interface")
                icon.name: "external"
                icon.color: Style.text
                enabled: AppController.boardAddress !== ""
                onClicked: Qt.openUrlExternally("http://" + page.host() + "/")
            }
        }

        PairBox {
            Layout.fillWidth: true
            address: address.text
            // The board in the field, also when set up under its name (the list has addresses).
            keySet: finder.boards.some(b => (b.address === address.text.trim()
                                             || (b.current && address.text.trim() === AppController.boardAddress))
                                            && b.key_set)
        }
    }

    Card {
        id: nameCard
        title: I18n.tr("Name")
        icon: "board"
        help: I18n.tr("Shown in the app's search and in the board's web interface. Your router and the browser use a network name made from it (letters, digits and dashes); the board keeps answering to its old name ps-hid-bridge-… too.")

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
            AppTextField {
                id: nameField
                Layout.fillWidth: true
                text: AppController.boardName
                maximumLength: 31
                placeholderText: AppController.connected ? I18n.tr("e.g. Living room") : I18n.tr("Connect to the board first")
                enabled: AppController.connected && !AppController.renameBusy
                onAccepted: if (renameButton.enabled) nameCard.rename()
            }
            AppButton {
                id: renameButton
                text: AppController.renameBusy ? I18n.tr("Renaming …") : I18n.tr("Rename")
                enabled: AppController.connected && !AppController.renameBusy
                         && nameField.text.trim() !== AppController.boardName
                onClicked: nameCard.rename()
            }
        }
        Controls.Label {
            Layout.fillWidth: true
            visible: AppController.connected
            text: I18n.trf("In your router and the browser: %1",
                           AppController.networkNameFor(nameField.text, nameCard.defaultName !== "" ? nameCard.defaultName : "ps-hid-bridge-…"))
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
        title: I18n.tr("Details")
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
                    [I18n.tr("Firmware"), AppController.boardFirmware !== "" ? AppController.boardFirmware : "–"],
                    ["PlayStation", AppController.boardUsb !== "" ? Style.playStation(AppController.boardUsb) : "–"],
                    [I18n.tr("Round trip"), AppController.rttMs >= 0 ? Style.us(AppController.rttMs * 1000) : "–"],
                    [I18n.tr("Desktop app"), AppController.version]
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
