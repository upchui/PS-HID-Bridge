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
        // Too narrow for name, pill and buttons side by side: pill and
        // buttons go below the name.
        readonly property bool narrow: width < 460
        Layout.fillWidth: true
        implicitHeight: vlayout.implicitHeight + 20
        radius: Style.radius
        color: Style.surfaceAlt
        GridLayout {
            id: vlayout
            anchors.fill: parent
            anchors.margins: 10
            anchors.leftMargin: 14
            columns: vrow.narrow ? 2 : 3
            columnSpacing: 12
            rowSpacing: 8
            AppIcon {
                Layout.row: 0
                Layout.column: 0
                name: vrow.icon
                color: Style.muted
                implicitWidth: 20
                implicitHeight: 20
            }
            ColumnLayout {
                Layout.row: 0
                Layout.column: 1
                Layout.fillWidth: true
                spacing: 0
                Controls.Label {
                    Layout.fillWidth: true
                    text: vrow.name
                    color: Style.text
                    font.weight: Font.DemiBold
                    elide: Text.ElideRight
                }
                Controls.Label {
                    Layout.fillWidth: true
                    text: vrow.detail
                    color: Style.muted
                    wrapMode: Text.WordWrap
                }
            }
            RowLayout {
                Layout.row: vrow.narrow ? 1 : 0
                Layout.column: vrow.narrow ? 1 : 2
                spacing: 8
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
                    + (AppController.appUpdateAvailable && !AppController.canUpdateApp
                       ? " · not an AppImage: download the new version from the release page" : "")
            showPill: AppController.latestVersion !== ""
            pillLevel: AppController.appUpdateAvailable ? "accent" : "ok"
            pillText: AppController.appUpdated ? "restart to use it"
                      : AppController.appUpdateAvailable ? AppController.latestVersion + " available" : "up to date"
            AppButton {
                text: AppController.appUpdated ? "Restart now" : "Update"
                highlighted: true
                // Not an AppImage: nothing to update here (the line above says why).
                visible: (AppController.appUpdateAvailable && AppController.canUpdateApp) || AppController.appUpdated
                enabled: !AppController.updateBusy
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
            // In a wrapper: a disabled button gets no hover, the wrapper does
            // (the tooltip says why it is disabled).
            Item {
                visible: AppController.boardUpdateAvailable
                implicitWidth: boardUpdate.implicitWidth
                implicitHeight: boardUpdate.implicitHeight
                AppButton {
                    id: boardUpdate
                    anchors.fill: parent
                    text: "Update"
                    highlighted: true
                    enabled: !AppController.updateBusy && !AppController.active
                    onClicked: AppController.updateBoard()
                }
                HoverHandler {
                    id: boardUpdateHover
                }
                Controls.ToolTip.visible: boardUpdateHover.hovered && AppController.active
                Controls.ToolTip.text: "Switch the bridge off first: the board restarts."
            }
        }

        Banner {
            kind: "warn"
            visible: AppController.boardNoPartitionTable
            text: "The board has no partition table: an update overwrites the running firmware and there is no previous version to fall back to. Install it once over USB (\"First installation\"); key and settings stay."
            actions: AppButton {
                text: "Install via USB"
                onClicked: usbToggle.checked = true
            }
        }

        Banner {
            kind: AppController.updateLevel === "warn" ? "warn" : AppController.updateLevel === "ok" ? "ok" : "info"
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
