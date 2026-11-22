import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import PSHIDBridge

// Versions and updates of app and board, installing over USB, and the
// diagnostics report.
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
    title: I18n.tr("Firmware & updates")
    subtitle: I18n.tr("New versions come from GitHub; every download is checked before it is used.")

    Card {
        title: I18n.tr("Versions")
        icon: "update"
        actions: AppButton {
            text: I18n.tr("Check now")
            icon.name: "update"
            icon.color: Style.text
            enabled: !AppController.updateBusy
            onClicked: AppController.checkUpdates()
        }

        VersionRow {
            icon: "status"
            name: I18n.tr("Desktop app")
            detail: AppController.appUpdateAvailable && !AppController.canUpdateApp
                    ? I18n.trf("installed %1 · not an AppImage: download the new version from the release page",
                               AppController.version)
                    : I18n.trf("installed %1", AppController.version)
            showPill: AppController.latestVersion !== ""
            pillLevel: AppController.appUpdateAvailable ? "accent" : "ok"
            pillText: AppController.appUpdated ? I18n.tr("restart to use it")
                      : AppController.appUpdateAvailable ? I18n.trf("%1 available", AppController.latestVersion)
                      : I18n.tr("up to date")
            AppButton {
                text: AppController.appUpdated ? I18n.tr("Restart now") : I18n.tr("Update")
                highlighted: true
                // Not an AppImage: nothing to update here (the line above says why).
                visible: (AppController.appUpdateAvailable && AppController.canUpdateApp) || AppController.appUpdated
                enabled: !AppController.updateBusy
                onClicked: AppController.appUpdated ? AppController.restartApp() : AppController.updateApp()
            }
        }
        VersionRow {
            icon: "board"
            name: I18n.tr("Board firmware")
            detail: AppController.boardFirmware !== "" ? I18n.trf("installed %1", AppController.boardFirmware)
                    : I18n.tr("unknown (board not connected)")
            showPill: AppController.latestVersion !== "" && AppController.boardFirmware !== ""
            pillLevel: AppController.boardUpdateAvailable ? "accent" : "ok"
            pillText: AppController.boardUpdateAvailable ? I18n.trf("%1 available", AppController.latestVersion)
                      : I18n.tr("up to date")
            // In a wrapper: a disabled button gets no hover, the wrapper does
            // (the tooltip says why it is disabled).
            Item {
                visible: AppController.boardUpdateAvailable
                implicitWidth: boardUpdate.implicitWidth
                implicitHeight: boardUpdate.implicitHeight
                AppButton {
                    id: boardUpdate
                    anchors.fill: parent
                    text: I18n.tr("Update")
                    highlighted: true
                    enabled: !AppController.updateBusy && !AppController.active
                    onClicked: AppController.updateBoard()
                }
                HoverHandler {
                    id: boardUpdateHover
                }
                Controls.ToolTip.visible: boardUpdateHover.hovered && AppController.active
                Controls.ToolTip.text: I18n.tr("Switch the bridge off first: the board restarts.")
            }
        }

        Banner {
            kind: "warn"
            visible: AppController.boardNoPartitionTable
            text: I18n.tr("The board has no partition table: an update overwrites the running firmware and there is no previous version to fall back to. Install it once over USB (\"First installation\"); key and settings stay.")
            actions: AppButton {
                text: I18n.tr("Install via USB")
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
                Accessible.name: I18n.tr("Look for updates automatically")
            }
            Controls.Label {
                Layout.fillWidth: true
                text: AppController.latestVersion !== ""
                      ? I18n.trf("Look for updates at start and every 5 minutes · newest %1", AppController.latestVersion)
                      : I18n.tr("Look for updates at start and every 5 minutes")
                color: Style.muted
                wrapMode: Text.WordWrap
            }
        }
    }

    Card {
        title: I18n.tr("Install via USB")
        icon: "usb"
        subtitle: I18n.tr("First installation or recovery, with the board connected to this PC.")
        actions: AppButton {
            id: usbToggle
            text: checked ? I18n.tr("Hide") : I18n.tr("Open")
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

    Card {
        title: I18n.tr("Diagnostics")
        icon: "help"
        subtitle: I18n.tr("One text file with what helps to find a problem, to attach to a bug report.")
        help: I18n.tr("Versions, system, desktop, the settings (without the key), the devices and their access, what the board says about itself, and the app's recent messages. Never the key or a text typed on the PlayStation. Look through it before you share it: it names your devices and the board's addresses.")

        RowLayout {
            Layout.fillWidth: true
            spacing: 12
            AppButton {
                text: AppController.diagBusy ? I18n.tr("Collecting …") : I18n.tr("Save diagnostics …")
                icon.name: "download"
                icon.color: Style.text
                enabled: !AppController.diagBusy
                onClicked: diagDialog.open()
            }
            Controls.Label {
                Layout.fillWidth: true
                visible: AppController.diagMessage !== ""
                wrapMode: Text.WrapAnywhere
                color: Style.stateColor(AppController.diagLevel)
                text: AppController.diagMessage
            }
        }

        FileDialog {
            id: diagDialog
            title: I18n.tr("Save diagnostics")
            fileMode: FileDialog.SaveFile
            nameFilters: [I18n.tr("Text files (*.md *.txt)")]
            onAccepted: AppController.saveDiagnostics(selectedFile.toString())
        }
    }

    // Before uninstalling (e.g. with Shelly, which cannot touch files of
    // root): take back what the app set up with the administrator password.
    Card {
        title: I18n.tr("Before uninstalling")
        icon: "trash"
        subtitle: I18n.tr("The app set up rules in the system with the administrator password. This removes them and takes back the access they gave; uninstall the app afterwards, e.g. in Shelly. Settings and key stay.")
        help: I18n.tr("Device access: /etc/udev/rules.d/70-ps-hid-bridge.rules lets you read keyboard and mouse. Low latency: /etc/udev/rules.d/70-ps-hid-bridge-latency.rules lets the app switch the network adapter's packet bundling and keep the processor awake. Settings and key stay in ~/.config/ps-hid-bridge; a reinstalled app finds them again.")

        Repeater {
            model: [
                { name: I18n.tr("Device access (udev rule)"), on: AppController.accessRuleInstalled },
                { name: I18n.tr("Low latency (udev rule)"), on: AppController.latencyRuleInstalled }
            ]
            delegate: RowLayout {
                required property var modelData
                Layout.fillWidth: true
                spacing: 10
                AppIcon {
                    name: modelData.on ? "check" : ""
                    color: Style.muted
                    implicitWidth: 16
                    implicitHeight: 16
                }
                Controls.Label {
                    Layout.fillWidth: true
                    text: modelData.name
                    color: modelData.on ? Style.text : Style.muted
                    wrapMode: Text.WordWrap
                }
                Controls.Label {
                    text: modelData.on ? I18n.tr("set up") : I18n.tr("not set up")
                    color: Style.muted
                }
            }
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 12
            AppButton {
                text: AppController.cleanupBusy ? I18n.tr("Removing …") : I18n.tr("Remove system changes")
                icon.name: "trash"
                icon.color: Style.text
                enabled: !AppController.cleanupBusy && !AppController.active && !AppController.pending
                         && (AppController.accessRuleInstalled || AppController.latencyRuleInstalled)
                onClicked: AppController.removeSystemChanges()
                Accessible.description: I18n.tr("Asks for the administrator password")
            }
            Controls.Label {
                Layout.fillWidth: true
                visible: AppController.active || AppController.pending
                wrapMode: Text.WordWrap
                color: Style.muted
                text: I18n.tr("Switch the bridge off first.")
            }
        }
        Controls.Label {
            Layout.fillWidth: true
            visible: AppController.cleanupMessage !== ""
            wrapMode: Text.WordWrap
            color: Style.stateColor(AppController.cleanupLevel)
            text: AppController.cleanupMessage
        }
    }
}
