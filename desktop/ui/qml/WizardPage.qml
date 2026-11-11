import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

AppPage {
    id: page
    title: "Set up PS HID Bridge"

    signal finished()

    property int step: 0
    readonly property var titles: ["Welcome", "Board", "Devices", "Trigger", "Test"]
    property string boardError: ""

    onStepChanged: if (step === 2) AppController.refreshDevices()

    footer: [
        Controls.Label {
            Layout.fillWidth: true
            text: "Step " + (page.step + 1) + " of " + page.titles.length
            color: Style.muted
        },
        AppButton {
            text: "Back"
            enabled: page.step > 0
            onClicked: page.step--
        },
        AppButton {
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
    ]

    // ------------------------------------------------------------ steps
    RowLayout {
        Layout.fillWidth: true
        Layout.bottomMargin: 4
        spacing: 0
        Repeater {
            model: page.titles
            delegate: RowLayout {
                id: stepItem
                required property string modelData
                required property int index
                readonly property bool done: index < page.step
                readonly property bool current: index === page.step
                Layout.fillWidth: index < page.titles.length - 1
                spacing: 8
                Rectangle {
                    implicitWidth: 28
                    implicitHeight: 28
                    radius: 14
                    color: stepItem.current || stepItem.done ? Style.accent : Style.surfaceAlt
                    border.color: stepItem.current || stepItem.done ? Style.accent : Style.border
                    AppIcon {
                        anchors.centerIn: parent
                        visible: stepItem.done
                        name: "check"
                        color: Style.accentText
                        implicitWidth: 16
                        implicitHeight: 16
                    }
                    Controls.Label {
                        anchors.centerIn: parent
                        visible: !stepItem.done
                        text: stepItem.index + 1
                        color: stepItem.current ? Style.accentText : Style.muted
                        font.weight: Font.DemiBold
                    }
                }
                Controls.Label {
                    text: stepItem.modelData
                    color: stepItem.current ? Style.text : Style.muted
                    font.weight: stepItem.current ? Font.DemiBold : Font.Normal
                }
                Rectangle {
                    visible: stepItem.index < page.titles.length - 1
                    Layout.fillWidth: true
                    Layout.leftMargin: 4
                    Layout.rightMargin: 8
                    implicitHeight: 2
                    color: stepItem.done ? Style.accent : Style.border
                }
            }
        }
    }

    StackLayout {
        Layout.fillWidth: true
        currentIndex: page.step

        // 0 -- Welcome
        ColumnLayout {
            spacing: Style.gap
            Card {
                title: "Welcome"
                icon: "gamepad"
                subtitle: "Control the PlayStation with this computer's mouse and keyboard while Remote Play shows the picture."

                Repeater {
                    model: [
                        ["board", "The board with firmware, at the PlayStation's USB port and connected to the network."],
                        ["key", "The key from the board's web interface (Security → Generate new key)."],
                        ["gamepad", "On the PlayStation: the keyboard language of your keyboard, and USB power in rest mode on."]
                    ]
                    delegate: RowLayout {
                        required property var modelData
                        Layout.fillWidth: true
                        spacing: 12
                        AppIcon {
                            name: (parent.modelData[0])
                            color: Style.accent
                            implicitWidth: 20
                            implicitHeight: 20
                        }
                        Controls.Label {
                            Layout.fillWidth: true
                            text: parent.modelData[1]
                            color: Style.text
                            wrapMode: Text.WordWrap
                        }
                    }
                }
            }
            Card {
                title: "No firmware on the board yet?"
                icon: "usb"
                subtitle: "Install it from here over USB."
                actions: AppButton {
                    id: usbToggle
                    text: checked ? "Hide" : "Install via USB"
                    checkable: true
                }
                // Loaded on demand (it watches for the board); always in
                // the self-test, so it is checked too.
                Loader {
                    Layout.fillWidth: true
                    active: usbToggle.checked || Qt.application.arguments.indexOf("--self-test") >= 0
                            || Qt.application.arguments.indexOf("--screenshot") >= 0
                    visible: active
                    sourceComponent: UsbInstall {}
                }
            }
        }

        // 1 -- Board
        ColumnLayout {
            spacing: Style.gap
            Card {
                title: "Board"
                icon: "network"
                subtitle: "Your router lists it as ps-hid-bridge-…; the key comes from its web interface."

                GridLayout {
                    Layout.fillWidth: true
                    columns: 2
                    columnSpacing: 12
                    rowSpacing: 8
                    Controls.Label {
                        text: "Address"
                        color: Style.text
                    }
                    Controls.TextField {
                        id: address
                        Layout.fillWidth: true
                        Layout.maximumWidth: 360
                        text: AppController.boardAddress
                        placeholderText: "e.g. 192.168.1.50"
                    }
                    Controls.Label {
                        text: "Key"
                        color: Style.text
                    }
                    Controls.TextField {
                        id: psk
                        Layout.fillWidth: true
                        echoMode: TextInput.Password
                        placeholderText: AppController.pskSet ? "stored – fill in only to change it" : "64 hex digits"
                    }
                }
                AppButton {
                    text: "Connect"
                    highlighted: true
                    onClicked: page.boardError = AppController.saveBoard(address.text, psk.text)
                }
                Banner {
                    kind: "error"
                    visible: page.boardError !== ""
                    text: page.boardError
                }
                Banner {
                    kind: AppController.connected ? "ok" : "info"
                    visible: page.boardError === "" && AppController.boardAddress !== ""
                    text: AppController.connected
                          ? "Connected. " + AppController.boardUsb + ", firmware " + AppController.boardFirmware + "."
                          : AppController.lastError !== "" ? AppController.lastError
                          : "Waiting for an answer from the board … Are address and key correct?"
                }
            }
        }

        // 2 -- Devices
        ColumnLayout {
            spacing: Style.gap
            Card {
                title: "Keyboard and mouse"
                icon: "input"
                subtitle: "Select what should go to the PlayStation."
                DeviceList {
                    Layout.fillWidth: true
                }
            }
            Card {
                title: "Keyboard type"
                icon: "keyboard"
                KeyboardTypeChooser {}
            }
        }

        // 3 -- Trigger
        ColumnLayout {
            spacing: Style.gap
            Card {
                title: "Switching"
                icon: "bolt"
                subtitle: "How you switch between desktop and PlayStation. Emergency stop, always: Ctrl+Alt+Shift+Esc."
                TriggerEditor {
                    id: triggers
                    Layout.fillWidth: true
                }
            }
        }

        // 4 -- Test
        ColumnLayout {
            spacing: Style.gap
            Card {
                title: "Try it"
                icon: "play"
                subtitle: "Press the trigger: you see here what reaches the PlayStation. Press it again to switch off."

                Controls.Label {
                    text: AppController.statusText
                    color: AppController.active ? Style.ok : Style.text
                    font.pointSize: Style.fontSize * 1.5
                    font.weight: Font.DemiBold
                }
                GridLayout {
                    columns: 2
                    columnSpacing: 16
                    rowSpacing: 6
                    Controls.Label { text: "To the PlayStation"; color: Style.muted }
                    Controls.Label {
                        text: AppController.active ? (AppController.liveText !== "" ? AppController.liveText : "no key") : "–"
                        color: Style.text
                        font.weight: Font.DemiBold
                    }
                    Controls.Label { text: "PlayStation"; color: Style.muted }
                    Controls.Label {
                        text: AppController.boardUsb !== "" ? AppController.boardUsb : "–"
                        color: Style.text
                    }
                    Controls.Label { text: "Round trip"; color: Style.muted }
                    Controls.Label {
                        text: AppController.rttMs >= 0 ? AppController.rttMs.toFixed(2) + " ms" : "–"
                        color: Style.text
                    }
                }
            }
        }
    }
}
