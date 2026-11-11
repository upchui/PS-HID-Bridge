import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import PSHIDBridge

Kirigami.ScrollablePage {
    id: page
    title: "Setup – step " + (step + 1) + " of " + titles.length

    signal finished()

    property int step: 0
    readonly property var titles: ["Welcome", "Board", "Devices", "Trigger", "Test"]
    property string boardError: ""

    footer: Controls.ToolBar {
        contentItem: RowLayout {
            Controls.Label {
                Layout.fillWidth: true
                leftPadding: Kirigami.Units.largeSpacing
                text: page.titles[page.step]
                font.bold: true
            }
            Controls.Button {
                text: "Back"
                enabled: page.step > 0
                onClicked: page.step--
            }
            Controls.Button {
                text: page.step === page.titles.length - 1 ? "Finish" : "Next"
                highlighted: true
                onClicked: {
                    if (page.step === 3 && !triggers.save())
                        return;
                    if (page.step === page.titles.length - 1) {
                        AppController.finishSetup();
                        page.finished();
                    } else {
                        page.step++;
                    }
                }
            }
        }
    }

    StackLayout {
        currentIndex: page.step

        // 0 -- Welcome
        ColumnLayout {
            spacing: Kirigami.Units.largeSpacing
            Kirigami.Heading { text: "Set up PS HID Bridge" }
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                text: "This app lets you control the PlayStation with the mouse and keyboard of this computer while the picture comes in via Remote Play. You need:"
            }
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                textFormat: Text.StyledText
                text: "• the board with current firmware, connected to the PlayStation via USB and to the network via cable<br>"
                      + "• the pre-shared key from the board's web interface (Security → Generate new key)<br>"
                      + "• on the PlayStation: the keyboard language of <b>your keyboard</b> (e.g. German) and USB power in rest mode switched on"
            }
        // Flashing over USB: loaded on demand (it watches for the board);
        // always in the self-test, so the page is checked too.
        Controls.Button {
            id: usbToggle
            text: usbToggle.checked ? "Hide USB installation" : "No firmware on the board yet? Install it via USB …"
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

        // 1 -- Board
        ColumnLayout {
            spacing: Kirigami.Units.largeSpacing
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                text: "Enter the board address (your router lists it as ps-hid-bridge-…) and the key from its web interface."
            }
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
                    placeholderText: AppController.pskSet ? "stored – fill in only to change it" : "64 hex digits"
                }
            }
            Controls.Button {
                text: "Connect"
                highlighted: true
                onClicked: page.boardError = AppController.saveBoard(address.text, psk.text)
            }
            Kirigami.InlineMessage {
                Layout.fillWidth: true
                visible: page.boardError !== ""
                type: Kirigami.MessageType.Error
                text: page.boardError
            }
            Kirigami.InlineMessage {
                Layout.fillWidth: true
                visible: page.boardError === "" && AppController.boardAddress !== ""
                type: AppController.connected ? Kirigami.MessageType.Positive : Kirigami.MessageType.Information
                text: AppController.connected
                      ? "Connected. " + AppController.boardUsb + ", firmware " + AppController.boardFirmware + "."
                      : AppController.lastError !== "" ? AppController.lastError
                      : "Waiting for an answer from the board … Are address and key correct?"
            }
        }

        // 2 -- Devices
        ColumnLayout {
            spacing: Kirigami.Units.largeSpacing
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                text: "Select the keyboard and mouse that should go to the PlayStation. The app then asks once for the administrator password to set up read access to exactly these devices."
            }
            DeviceList {
                Layout.fillWidth: true
            }
            KeyboardTypeChooser {
                Layout.fillWidth: true
            }
        }

        // 3 -- Trigger
        ColumnLayout {
            spacing: Kirigami.Units.largeSpacing
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                text: "How do you want to switch between desktop and PlayStation? The default is Ctrl+Alt+P. More kinds (mouse button, hold, window rule) are available later under \"Triggers\"."
            }
            TriggerEditor {
                id: triggers
                Layout.fillWidth: true
            }
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                text: "Emergency stop, always available: Ctrl+Alt+Shift+Esc."
            }
        }

        // 4 -- Test
        ColumnLayout {
            spacing: Kirigami.Units.largeSpacing
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                text: "Now press the trigger and release it. While the bridge is on, you see here what reaches the PlayStation. Press the trigger again to switch it off."
            }
            Kirigami.FormLayout {
                Layout.fillWidth: true
                Controls.Label {
                    Kirigami.FormData.label: "Status:"
                    text: AppController.statusText
                    font.bold: true
                }
                Controls.Label {
                    Kirigami.FormData.label: "To the PlayStation:"
                    text: AppController.active ? (AppController.liveText !== "" ? AppController.liveText : "no key") : "–"
                }
                Controls.Label {
                    Kirigami.FormData.label: "PlayStation:"
                    text: AppController.boardUsb !== "" ? AppController.boardUsb : "–"
                }
                Controls.Label {
                    Kirigami.FormData.label: "Latency:"
                    text: AppController.rttMs >= 0 ? AppController.rttMs.toFixed(2) + " ms" : "–"
                }
            }
        }
    }

    onStepChanged: if (step === 2) AppController.refreshDevices()
}
