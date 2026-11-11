import QtQuick
import QtQuick.Controls as Controls
import PSHIDBridge

// A small "?" that explains something on hover (or click).
AppIcon {
    id: hint

    property string text

    name: "help"
    color: hover.hovered || tip.visible ? Style.accent : Style.muted
    implicitWidth: 16
    implicitHeight: 16

    HoverHandler {
        id: hover
        cursorShape: Qt.WhatsThisCursor
    }
    TapHandler {
        onTapped: tip.visible = !tip.visible
    }
    Controls.ToolTip {
        id: tip
        visible: hover.hovered
        delay: 300
        text: hint.text
        width: Math.min(implicitWidth, 360)
        contentItem: Controls.Label {
            text: tip.text
            wrapMode: Text.WordWrap
            color: Style.text
        }
        background: Rectangle {
            color: Style.surfaceAlt
            border.color: Style.border
            radius: 6
        }
    }
}
