import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import PSHIDBridge

// Flash the board over USB (BOOTSEL drive): first installation, firmware,
// test images, erase. Looks for the board while it is shown.
ColumnLayout {
    id: root
    spacing: Kirigami.Units.smallSpacing

    // Own UF2 file (URL); "" = files of the newest release.
    property string file: ""
    property bool confirmErase: false

    Component.onCompleted: AppController.usbWatch(true)
    Component.onDestruction: AppController.usbWatch(false)

    readonly property var images: [
        { value: "first-install", text: "First installation (partition table + firmware)" },
        { value: "firmware", text: "Firmware only" },
        { value: "hidtest", text: "HID test image (no network, mouse moves in a square)" },
        { value: "hidtest-mouse8", text: "HID test image with 8-bit mouse axes" },
        { value: "erase", text: "Erase everything (firmware and settings)" }
    ]

    Controls.Label {
        Layout.fillWidth: true
        wrapMode: Text.WordWrap
        text: "Hold the BOOTSEL button on the board and connect it to this PC via USB (not to the PlayStation). It then shows up as the drive \"RP2350\"."
    }
    Kirigami.InlineMessage {
        Layout.fillWidth: true
        visible: true
        type: AppController.usbBoardFound ? Kirigami.MessageType.Positive : Kirigami.MessageType.Information
        text: AppController.usbBoardFound ? "Board found (RP2350 boot drive)." : "Waiting for the board …"
    }

    Kirigami.FormLayout {
        Layout.fillWidth: true
        Controls.ComboBox {
            id: imageBox
            Kirigami.FormData.label: "Install:"
            Layout.fillWidth: true
            model: root.images
            textRole: "text"
            valueRole: "value"
            onActivated: root.confirmErase = false
        }
        RowLayout {
            Kirigami.FormData.label: "Files:"
            Controls.Label {
                Layout.fillWidth: true
                elide: Text.ElideMiddle
                text: root.file === "" ? "from the newest release (GitHub)"
                      : decodeURIComponent(root.file.replace(/^file:\/\//, ""))
            }
            Controls.Button {
                text: "Choose file …"
                onClicked: fileDialog.open()
            }
            Controls.Button {
                text: "Newest release"
                visible: root.file !== ""
                onClicked: root.file = ""
            }
        }
    }

    Kirigami.InlineMessage {
        Layout.fillWidth: true
        visible: root.confirmErase
        type: Kirigami.MessageType.Warning
        text: "This erases the board's firmware and settings."
        actions: [
            Kirigami.Action {
                text: "Erase"
                onTriggered: {
                    root.confirmErase = false;
                    AppController.usbFlash("erase", root.file);
                }
            },
            Kirigami.Action {
                text: "Cancel"
                onTriggered: root.confirmErase = false
            }
        ]
    }

    RowLayout {
        Controls.Button {
            text: "Install"
            highlighted: true
            enabled: !AppController.usbBusy
            onClicked: {
                if (imageBox.currentValue === "erase")
                    root.confirmErase = true;
                else
                    AppController.usbFlash(imageBox.currentValue, root.file);
            }
        }
        Controls.Button {
            text: "Cancel"
            visible: AppController.usbBusy
            onClicked: AppController.usbCancel()
        }
    }
    Controls.Label {
        Layout.fillWidth: true
        wrapMode: Text.WordWrap
        visible: AppController.usbStatus !== ""
        text: AppController.usbStatus
    }

    FileDialog {
        id: fileDialog
        title: "UF2 file for the board"
        nameFilters: ["UF2 files (*.uf2)"]
        onAccepted: root.file = selectedFile.toString()
    }
}
