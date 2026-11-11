import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// Versions and updates of app and board, and installing over USB.
AppPage {
    id: page

    component VersionRow: Rectangle {
        id: vrow
        property string icon
        property string name
        property string detail
        property bool showPill
        property string pillLevel
        property string pillText
        default property alias buttons: buttonRow.data
        Layout.fillWidth: true
        implicitHeight: vlayout.implicitHeight + 20
        radius: Style.radius
        color: Style.surfaceAlt
        RowLayout {
            id: vlayout
            anchors.fill: parent
            anchors.margins: 10
            anchors.leftMargin: 14
            spacing: 12
            AppIcon {
                name: vrow.icon
                color: Style.muted
                implicitWidth: 20
                implicitHeight: 20
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0
                Controls.Label {
                    text: vrow.name
                    color: Style.text
                    font.weight: Font.DemiBold
                }
                Controls.Label {
                    text: vrow.detail
                    color: Style.muted
                }
            }
            Pill {
                visible: vrow.showPill
                level: vrow.pillLevel
                text: vrow.pillText
            }
            RowLayout {
                id: buttonRow
                spacing: 6
            }
        }
    }
    title: "Firmware & updates"
    subtitle: "New versions come from GitHub; every download is checked before it is used."

    Card {
        title: "Versions"
        icon: "update"
        actions: AppButton {
            text: "Check now"
            icon.name: "update"
            icon.color: Style.text
            enabled: !AppController.updateBusy
            onClicked: AppController.checkUpdates()
        }

        VersionRow {
            icon: "status"
            name: "Desktop app"
            detail: "installed " + AppController.version
            showPill: AppController.latestVersion !== ""
            pillLevel: AppController.appUpdateAvailable ? "accent" : "ok"
            pillText: AppController.appUpdated ? "restart to use it"
                      : AppController.appUpdateAvailable ? AppController.latestVersion + " available" : "up to date"
            AppButton {
                text: AppController.appUpdated ? "Restart now" : "Update"
                highlighted: true
                visible: AppController.appUpdateAvailable || AppController.appUpdated
                enabled: !AppController.updateBusy && (AppController.appUpdated || AppController.canUpdateApp)
                onClicked: AppController.appUpdated ? AppController.restartApp() : AppController.updateApp()
            }
        }
        VersionRow {
            icon: "board"
            name: "Board firmware"
            detail: AppController.boardFirmware !== "" ? "installed " + AppController.boardFirmware : "unknown (board not connected)"
            showPill: AppController.latestVersion !== "" && AppController.boardFirmware !== ""
            pillLevel: AppController.boardUpdateAvailable ? "accent" : "ok"
            pillText: AppController.boardUpdateAvailable ? AppController.latestVersion + " available" : "up to date"
            AppButton {
                text: "Update"
                highlighted: true
                visible: AppController.boardUpdateAvailable
                enabled: !AppController.updateBusy && !AppController.active
                onClicked: AppController.updateBoard()
                Controls.ToolTip.visible: hovered && AppController.active
                Controls.ToolTip.text: "Switch the bridge off first: the board restarts."
            }
        }

        Banner {
            kind: AppController.updateStatus.indexOf("failed") >= 0 || AppController.updateStatus.indexOf("not") >= 0 ? "warn" : "info"
            visible: AppController.updateStatus !== ""
            text: AppController.updateStatus
        }

        RowLayout {
            spacing: 10
            AppSwitch {
                checked: AppController.updateCheck
                onToggled: AppController.enableUpdateCheck(checked)
                Accessible.name: "Look for updates automatically"
            }
            Controls.Label {
                Layout.fillWidth: true
                text: "Look for updates at start and every 6 hours" + (AppController.latestVersion !== "" ? " · newest " + AppController.latestVersion : "")
                color: Style.muted
                wrapMode: Text.WordWrap
            }
        }
    }

    Card {
        title: "Install via USB"
        icon: "usb"
        subtitle: "First installation or recovery, with the board connected to this PC."
        actions: AppButton {
            id: usbToggle
            text: checked ? "Hide" : "Open"
            checkable: true
        }

        // Loaded on demand (it watches for the board); always in the
        // self-test, so it is checked too.
        Loader {
            Layout.fillWidth: true
            active: usbToggle.checked || Qt.application.arguments.indexOf("--self-test") >= 0
                    || Qt.application.arguments.indexOf("--screenshot") >= 0
            visible: active
            sourceComponent: UsbInstall {}
        }
    }
}
