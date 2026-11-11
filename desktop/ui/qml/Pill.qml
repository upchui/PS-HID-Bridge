import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// A small coloured label: state ("ok", "warn", "bad", "accent", or muted).
Rectangle {
    id: pill

    property string text
    property string level: ""
    property string icon: ""
    readonly property color tone: Style.stateColor(level)

    implicitWidth: row.implicitWidth + 16
    implicitHeight: row.implicitHeight + 6
    radius: height / 2
    color: Style.tint(tone, 0.15)

    RowLayout {
        id: row
        anchors.centerIn: parent
        spacing: 4
        AppIcon {
            visible: pill.icon !== ""
            name: pill.icon
            color: pill.tone
            implicitWidth: 14
            implicitHeight: 14
        }
        Controls.Label {
            text: pill.text
            color: pill.tone
            font.weight: Font.DemiBold
            font.pointSize: Style.fontSize * 0.9
        }
    }
}
