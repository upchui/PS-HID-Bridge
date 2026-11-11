import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// A tile with an icon, a label, a big value and a line below.
Rectangle {
    id: tile

    property string icon
    property string label
    property string value
    property string detail
    property string level: ""
    default property alias extra: extraColumn.data

    Layout.fillWidth: true
    Layout.fillHeight: true
    implicitHeight: col.implicitHeight + 2 * Style.pad
    color: Style.surface
    radius: Style.radius
    border.color: Style.border

    ColumnLayout {
        id: col
        anchors.fill: parent
        anchors.margins: Style.pad
        spacing: 4

        RowLayout {
            spacing: 8
            Rectangle {
                implicitWidth: 30
                implicitHeight: 30
                radius: 8
                color: Style.tint(Style.stateColor(tile.level === "" ? "accent" : tile.level), 0.15)
                AppIcon {
                    anchors.centerIn: parent
                    name: tile.icon
                    color: Style.stateColor(tile.level === "" ? "accent" : tile.level)
                    implicitWidth: 18
                    implicitHeight: 18
                }
            }
            Controls.Label {
                Layout.fillWidth: true
                text: tile.label
                color: Style.muted
                font.weight: Font.DemiBold
                elide: Text.ElideRight
            }
        }
        Controls.Label {
            Layout.fillWidth: true
            Layout.topMargin: 4
            text: tile.value
            color: tile.level === "bad" || tile.level === "warn" ? Style.stateColor(tile.level) : Style.text
            font.pointSize: Style.fontSize * 1.45
            font.weight: Font.DemiBold
            elide: Text.ElideRight
        }
        Controls.Label {
            Layout.fillWidth: true
            visible: tile.detail !== ""
            text: tile.detail
            color: Style.muted
            wrapMode: Text.WordWrap
        }
        ColumnLayout {
            id: extraColumn
            Layout.fillWidth: true
            spacing: 4
        }
        Item { Layout.fillHeight: true }
    }
}
