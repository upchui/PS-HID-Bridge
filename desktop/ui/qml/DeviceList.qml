import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import PS5HIDBridge

// Keyboards and mice, grouped per physical device (vendor:product).
ColumnLayout {
    id: list
    spacing: Kirigami.Units.smallSpacing

    readonly property var groups: JSON.parse(AppController.devicesJson)
    readonly property int selectedBlocked: groups.filter(g => g.selected && !g.accessible).length
    readonly property int selectedCount: groups.filter(g => g.selected).length
    property bool identify: false
    property string accessError: ""

    onIdentifyChanged: AppController.setIdentify(identify)
    Component.onDestruction: if (identify) AppController.setIdentify(false)

    Controls.Label {
        Layout.fillWidth: true
        wrapMode: Text.WordWrap
        visible: list.groups.length === 0
        text: "No keyboard and no mouse found."
    }

    Repeater {
        model: list.groups
        delegate: Controls.ItemDelegate {
            id: row
            required property var modelData
            Layout.fillWidth: true
            highlighted: modelData.paths.indexOf(AppController.identifiedPath) >= 0
            onClicked: AppController.setDeviceSelected(modelData.id, !modelData.selected)

            contentItem: RowLayout {
                Controls.CheckBox {
                    checked: row.modelData.selected
                    onToggled: AppController.setDeviceSelected(row.modelData.id, checked)
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    Controls.Label {
                        Layout.fillWidth: true
                        elide: Text.ElideRight
                        text: row.modelData.name
                        font.bold: row.highlighted
                    }
                    Controls.Label {
                        Layout.fillWidth: true
                        elide: Text.ElideRight
                        opacity: 0.7
                        font: Kirigami.Theme.smallFont
                        text: row.modelData.kind + " · " + row.modelData.id
                              + (row.modelData.nodes > 1 ? " · " + row.modelData.nodes + " interfaces" : "")
                              + (row.modelData.accessible ? "" : " · no access")
                    }
                }
                Controls.Label {
                    visible: row.highlighted
                    text: "← input detected"
                }
            }
        }
    }

    RowLayout {
        Layout.fillWidth: true
        Controls.Button {
            text: "Refresh list"
            onClicked: AppController.refreshDevices()
        }
        Controls.Switch {
            text: "Press a key to identify"
            checked: list.identify
            enabled: list.selectedCount > 0 && list.selectedBlocked === 0
            onToggled: list.identify = checked
        }
    }

    Kirigami.InlineMessage {
        Layout.fillWidth: true
        type: Kirigami.MessageType.Warning
        visible: list.selectedBlocked > 0
        text: list.selectedBlocked + " selected device(s) without read permission. The app sets up a udev rule for them (administrator password required)."
        actions: [
            Kirigami.Action {
                text: "Set up access"
                onTriggered: list.accessError = AppController.installAccess()
            }
        ]
    }
    Kirigami.InlineMessage {
        Layout.fillWidth: true
        type: Kirigami.MessageType.Error
        visible: list.accessError !== ""
        text: list.accessError
    }
}
