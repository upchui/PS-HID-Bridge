import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// One entry of the sidebar, with an optional marker on the right: a
// number (`badge`, e.g. of warnings) or a dot, in the colour of `level`.
Controls.AbstractButton {
    id: item

    property string iconName
    property bool current: false
    property string badge: ""
    property bool dot: false
    property string level: "accent"
    /// What the marker means, for screen readers and as a tooltip.
    property string note: ""

    Layout.fillWidth: true
    implicitHeight: row.implicitHeight + 14
    hoverEnabled: true
    Accessible.description: note
    Controls.ToolTip.visible: hovered && note !== ""
    Controls.ToolTip.text: note
    Controls.ToolTip.delay: 500

    background: Rectangle {
        radius: Style.radius
        color: item.current ? Style.tint(Style.accent, 0.16)
             : item.pressed ? Style.tint(Style.text, 0.1)
             : item.hovered ? Style.tint(Style.text, 0.06) : "transparent"
        // Keyboard focus.
        Rectangle {
            anchors.fill: parent
            anchors.margins: -1
            radius: parent.radius + 1
            color: "transparent"
            border.color: Style.accent
            border.width: 2
            visible: item.visualFocus
        }
    }
    contentItem: RowLayout {
        id: row
        spacing: 10
        AppIcon {
            Layout.leftMargin: 10
            name: item.iconName
            color: item.current ? Style.accent : Style.muted
            implicitWidth: 18
            implicitHeight: 18
        }
        Controls.Label {
            Layout.fillWidth: true
            text: item.text
            color: item.current ? Style.text : Style.muted
            font.weight: item.current ? Font.DemiBold : Font.Normal
            elide: Text.ElideRight
        }
        Pill {
            Layout.rightMargin: 8
            visible: item.badge !== ""
            text: item.badge
            level: item.level
        }
        Rectangle {
            Layout.rightMargin: 12
            visible: item.dot && item.badge === ""
            implicitWidth: 8
            implicitHeight: 8
            radius: 4
            color: Style.stateColor(item.level)
        }
    }
}
