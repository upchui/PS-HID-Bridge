import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// Keyboards and mice, grouped per physical device (vendor:product); the
// selected ones first.
ColumnLayout {
    id: list
    spacing: 8

    readonly property var groups: {
        const g = JSON.parse(AppController.devicesJson);
        return g.filter(d => d.selected).concat(g.filter(d => !d.selected));
    }
    readonly property int selectedBlocked: groups.filter(g => g.selected && !g.accessible).length
    readonly property int selectedCount: groups.filter(g => g.selected).length
    property bool identify: false

    onIdentifyChanged: AppController.setIdentify(identify)
    Component.onDestruction: if (identify) AppController.setIdentify(false)

    Banner {
        kind: "warn"
        visible: list.selectedBlocked > 0
        text: list.selectedBlocked + " selected device(s) cannot be read yet. The app sets up access once (administrator password)."
        actions: AppButton {
            text: AppController.accessBusy ? "Setting up …" : "Set up access"
            enabled: !AppController.accessBusy
            onClicked: AppController.installAccess()
        }
    }
    Banner {
        kind: "error"
        visible: AppController.accessMessage !== ""
        text: AppController.accessMessage
    }

    RowLayout {
        Layout.fillWidth: true
        spacing: 8
        Controls.Label {
            Layout.fillWidth: true
            text: list.selectedCount + " of " + list.groups.length + " selected"
            color: Style.muted
        }
        AppButton {
            text: list.identify ? "Press a key …" : "Identify"
            icon.name: "target"
            icon.color: !enabled ? Style.disabled : list.identify ? Style.accentText : Style.text
            checkable: true
            checked: list.identify
            highlighted: list.identify
            enabled: list.selectedCount > 0 && list.selectedBlocked === 0
            onToggled: list.identify = checked
            Controls.ToolTip.visible: hovered
            Controls.ToolTip.text: "Marks the selected device you are typing on."
        }
        AppButton {
            text: "Refresh"
            icon.name: "update"
            icon.color: Style.text
            onClicked: AppController.refreshDevices()
        }
    }

    Controls.Label {
        Layout.fillWidth: true
        visible: list.groups.length === 0
        text: "No keyboard and no mouse found."
        color: Style.muted
    }

    Repeater {
        model: list.groups
        delegate: Rectangle {
            id: row
            required property var modelData
            readonly property bool identified: modelData.paths.indexOf(AppController.identifiedPath) >= 0

            Layout.fillWidth: true
            implicitHeight: rowLayout.implicitHeight + 16
            radius: Style.radius
            color: identified ? Style.tint(Style.accent, 0.14)
                 : hover.hovered ? Style.tint(Style.text, 0.04) : Style.surfaceAlt
            border.color: identified ? Style.accent : modelData.selected ? Style.tint(Style.accent, 0.4) : "transparent"

            HoverHandler { id: hover }
            TapHandler {
                onTapped: AppController.setDeviceSelected(row.modelData.id, !row.modelData.selected)
            }

            RowLayout {
                id: rowLayout
                anchors.fill: parent
                anchors.margins: 8
                anchors.leftMargin: 12
                spacing: 12

                AppIcon {
                    name: (row.modelData.kind === "keyboard" ? "keyboard"
                                       : row.modelData.kind === "mouse" ? "mouse" : "input")
                    color: row.modelData.selected ? Style.accent : Style.muted
                    implicitWidth: 22
                    implicitHeight: 22
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    Controls.Label {
                        Layout.fillWidth: true
                        text: row.modelData.name
                        color: Style.text
                        font.weight: row.modelData.selected ? Font.DemiBold : Font.Normal
                        elide: Text.ElideRight
                    }
                    Controls.Label {
                        Layout.fillWidth: true
                        text: row.modelData.kind + " · " + row.modelData.id
                              + (row.modelData.nodes > 1 ? " · " + row.modelData.nodes + " interfaces" : "")
                        color: Style.muted
                        font.pointSize: Style.fontSize * 0.9
                        elide: Text.ElideRight
                    }
                }
                Pill {
                    visible: row.identified
                    level: "accent"
                    text: "input"
                }
                Pill {
                    visible: !row.modelData.accessible
                    level: row.modelData.selected ? "warn" : ""
                    text: "no access"
                }
                AppSwitch {
                    checked: row.modelData.selected
                    onToggled: AppController.setDeviceSelected(row.modelData.id, checked)
                    Accessible.name: row.modelData.name
                }
            }
        }
    }
}
