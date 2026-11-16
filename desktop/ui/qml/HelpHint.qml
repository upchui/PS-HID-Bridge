import QtQuick
import QtQuick.Controls as Controls
import PSHIDBridge

// A small "?" that explains something on hover; a click keeps the
// explanation open until the next click.
AppIcon {
    id: hint

    property string text
    property bool pinned: false

    name: "help"
    color: hover.hovered || tip.visible ? Style.accent : Style.muted
    implicitWidth: 16
    implicitHeight: 16

    HoverHandler {
        id: hover
        cursorShape: Qt.WhatsThisCursor
    }
    TapHandler {
        onTapped: hint.pinned = !hint.pinned
    }
    Controls.ToolTip {
        id: tip
        visible: hover.hovered || hint.pinned
        onVisibleChanged: if (!visible) hint.pinned = false  // closed from outside (click elsewhere, Esc)
        delay: hint.pinned ? 0 : 300
        text: hint.text
        width: Math.min(implicitWidth, 360)
        // Centred over the "?" by its real width: the style centres by the
        // unwrapped text width, which puts a long text at the window's edge.
        x: (hint.width - width) / 2
        y: -height - 3
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
