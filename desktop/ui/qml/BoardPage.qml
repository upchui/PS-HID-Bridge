import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import PSHIDBridge

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
            text: AppController.connected ? "Connected."
                  : AppController.lastError !== "" ? "Saved. " + AppController.lastError
                  : "Saved. Connecting …"
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
                Kirigami.FormData.label: "PlayStation:"
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
            text: "The board's web interface creates the key under \"Security\". Your router shows the address (device name ps-hid-bridge-…)."
        }

        Kirigami.Separator { Layout.fillWidth: true }

        Kirigami.Heading {
            level: 2
            text: "Updates"
        }
        Kirigami.FormLayout {
            Layout.fillWidth: true
            Controls.Label {
                Kirigami.FormData.label: "Newest release:"
                text: AppController.latestVersion !== "" ? AppController.latestVersion : "–"
            }
            Controls.Switch {
                Kirigami.FormData.label: "Automatically:"
                text: "Look for updates at start and every 6 hours"
                checked: AppController.updateCheck
                onToggled: AppController.enableUpdateCheck(checked)
            }
        }
        RowLayout {
            Controls.Button {
                text: "Check now"
                enabled: !AppController.updateBusy
                onClicked: AppController.checkUpdates()
            }
            Controls.Button {
                text: AppController.appUpdated ? "Restart now" : "Update app"
                visible: AppController.appUpdateAvailable || AppController.appUpdated
                enabled: !AppController.updateBusy && (AppController.appUpdated || AppController.canUpdateApp)
                onClicked: AppController.appUpdated ? AppController.restartApp() : AppController.updateApp()
            }
            Controls.Button {
                text: "Update board firmware"
                visible: AppController.boardUpdateAvailable
                enabled: !AppController.updateBusy && !AppController.active
                onClicked: AppController.updateBoard()
            }
        }
        Controls.Label {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            visible: AppController.updateStatus !== ""
            text: AppController.updateStatus
        }
        Controls.Label {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            opacity: 0.8
            text: "Releases come from github.com/upchui/PS-HID-Bridge; every download is checked against the release's checksums. The board firmware is installed over the network, signed with the key"
                  + (AppController.active ? " (switch the bridge off first)." : ".")
        }

        Kirigami.Separator { Layout.fillWidth: true }

        Kirigami.Heading {
            level: 2
            text: "Install via USB"
        }
        Controls.Label {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            opacity: 0.8
            text: "For the first installation or for recovery: the board is connected to this PC instead of the network."
        }
        // Flashing over USB: loaded on demand (it watches for the board);
        // always in the self-test, so the page is checked too.
        Controls.Button {
            id: usbToggle
            text: usbToggle.checked ? "Hide USB installation" : "Install via USB …"
            checkable: true
            icon.name: "drive-removable-media-usb"
        }
        Loader {
            Layout.fillWidth: true
            active: usbToggle.checked || Qt.application.arguments.indexOf("--self-test") >= 0
                    || Qt.application.arguments.indexOf("--screenshot") >= 0
            visible: active
            sourceComponent: UsbInstall {}
        }
    }
}
