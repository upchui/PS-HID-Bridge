import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import PSHIDBridge

// Flash the board over USB (BOOTSEL drive): first installation, firmware,
// test images, erase. Looks for the board while it is shown.
ColumnLayout {
    id: root
    spacing: 12

    // Own UF2 file (URL); "" = files of the newest release.
    property string file: ""
    property bool confirmErase: false

    Component.onCompleted: AppController.usbWatch(true)
    Component.onDestruction: AppController.usbWatch(false)

    readonly property var images: [
        { value: "first-install", text: I18n.tr("First installation (partition table + firmware)") },
        { value: "firmware", text: I18n.tr("Firmware only") },
        { value: "hidtest", text: I18n.tr("HID test image (no network, mouse moves in a square)") },
        { value: "hidtest-mouse8", text: I18n.tr("HID test image with 8-bit mouse axes") },
        { value: "erase", text: I18n.tr("Erase everything (firmware and settings)") }
    ]

    component Step: RowLayout {
        id: step
        property int number
        property string text
        default property alias extra: stepExtra.data
        Layout.fillWidth: true
        spacing: 12
        Rectangle {
            Layout.alignment: Qt.AlignTop
            implicitWidth: 26
            implicitHeight: 26
            radius: 13
            color: Style.tint(Style.accent, 0.15)
            Controls.Label {
                anchors.centerIn: parent
                text: step.number
                color: Style.accent
                font.weight: Font.Bold
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 8
            Controls.Label {
                Layout.fillWidth: true
                Layout.topMargin: 3
                text: step.text
                color: Style.text
                wrapMode: Text.WordWrap
            }
            ColumnLayout {
                id: stepExtra
                Layout.fillWidth: true
                spacing: 8
            }
        }
    }

    Step {
        number: 1
        text: I18n.tr("Hold BOOTSEL on the board and connect it to this PC via USB (not to the PlayStation).")
        Pill {
            level: AppController.usbBoardFound ? "ok" : ""
            icon: AppController.usbBoardFound ? "check" : "usb"
            text: AppController.usbBoardFound ? I18n.tr("Board found (drive RP2350)") : I18n.tr("Waiting for the board …")
        }
    }

    Step {
        number: 2
        text: I18n.tr("Choose what to install.")
        AppComboBox {
            id: imageBox
            Layout.fillWidth: true
            Layout.maximumWidth: 460
            model: root.images
            textRole: "text"
            valueRole: "value"
            onActivated: root.confirmErase = false
        }
        // The source on a line of its own (a chosen file's path needs the
        // room), the buttons below it.
        RowLayout {
            Layout.fillWidth: true
            spacing: 8
            AppIcon {
                name: (root.file === "" ? "download" : "folder")
                color: Style.muted
                implicitWidth: 16
                implicitHeight: 16
            }
            Controls.Label {
                Layout.fillWidth: true
                elide: Text.ElideMiddle
                color: Style.muted
                text: root.file === "" ? I18n.tr("Files from the newest release")
                      : decodeURIComponent(root.file.replace(/^file:\/\//, ""))
            }
        }
        Flow {
            Layout.fillWidth: true
            spacing: 8
            AppButton {
                text: I18n.tr("Choose file …")
                onClicked: fileDialog.open()
            }
            AppButton {
                text: I18n.tr("Newest release")
                visible: root.file !== ""
                onClicked: root.file = ""
            }
        }
    }

    Step {
        number: 3
        text: I18n.tr("Install. The board restarts by itself afterwards.")
        Banner {
            kind: "warn"
            visible: root.confirmErase
            text: I18n.tr("This erases the board's firmware and settings.")
            actions: [
                AppButton {
                    text: I18n.tr("Erase")
                    enabled: !AppController.updateBusy
                    onClicked: {
                        root.confirmErase = false;
                        AppController.usbFlash("erase", ""); // never an own file
                    }
                },
                AppButton {
                    text: I18n.tr("Cancel")
                    flat: true
                    onClicked: root.confirmErase = false
                }
            ]
        }
        RowLayout {
            spacing: 8
            AppButton {
                text: I18n.tr("Install")
                highlighted: true
                icon.name: "usb"
                icon.color: enabled ? Style.accent : Style.disabled
                // One installation at a time: not while the app or the
                // board's firmware is updated over the network.
                enabled: !AppController.usbBusy && !AppController.updateBusy
                onClicked: {
                    if (imageBox.currentValue === "erase")
                        root.confirmErase = true;
                    else
                        AppController.usbFlash(imageBox.currentValue, root.file);
                }
            }
            AppButton {
                text: I18n.tr("Cancel")
                visible: AppController.usbBusy
                onClicked: AppController.usbCancel()
            }
            Controls.BusyIndicator {
                visible: AppController.usbBusy
                running: visible
                implicitWidth: 24
                implicitHeight: 24
            }
        }
        Controls.Label {
            Layout.fillWidth: true
            visible: AppController.updateBusy && !AppController.usbBusy
            text: I18n.tr("An update over the network is running: install over USB when it is done.")
            color: Style.stateColor("warn")
            wrapMode: Text.WordWrap
        }
        Controls.Label {
            Layout.fillWidth: true
            visible: AppController.usbStatus !== ""
            text: AppController.usbStatus
            color: Style.text
            wrapMode: Text.WordWrap
        }
    }

    FileDialog {
        id: fileDialog
        title: I18n.tr("UF2 file for the board")
        nameFilters: [I18n.tr("UF2 files (*.uf2)")]
        onAccepted: root.file = selectedFile.toString()
    }
}
