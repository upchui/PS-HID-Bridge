import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// One entry of the sidebar.
Controls.AbstractButton {
    id: item

    property string iconName
    property bool current: false

    Layout.fillWidth: true
    implicitHeight: row.implicitHeight + 14
    hoverEnabled: true

    background: Rectangle {
        radius: Style.radius
        color: item.current ? Style.tint(Style.accent, 0.16)
             : item.hovered ? Style.tint(Style.text, 0.06) : "transparent"
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
    }
}
