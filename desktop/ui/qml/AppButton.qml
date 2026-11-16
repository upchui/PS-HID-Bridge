import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// Button in the app's style: rounded; "highlighted" is filled with the
// accent colour, "flat" has no frame, checked (a toggle) gets an accent tint
// and an accent icon. An icon (icon.name, one of AppIcon's) is drawn in
// icon.color if set, else in the text colour.
Controls.Button {
    id: button

    readonly property bool primary: highlighted
    readonly property color foreground: !enabled ? Style.disabled : primary ? Style.accentText : Style.text

    hoverEnabled: true
    padding: 7
    leftPadding: text !== "" ? 14 : 8
    rightPadding: text !== "" ? 14 : 8

    background: Rectangle {
        implicitHeight: 32
        radius: 7
        color: !button.enabled ? (button.flat ? "transparent" : Style.tint(Style.text, 0.04))
             : button.primary ? (button.pressed ? Qt.darker(Style.accent, 1.15)
                                 : button.hovered ? Qt.lighter(Style.accent, 1.08) : Style.accent)
             : button.pressed ? Style.tint(Style.text, 0.14)
             : button.checked ? Style.tint(Style.accent, button.hovered ? 0.2 : 0.14)
             : button.hovered ? Style.tint(Style.text, 0.08)
             : button.flat ? "transparent" : Style.button
        border.color: button.primary || (button.flat && !button.checked) ? "transparent"
                      : button.checked ? Style.tint(Style.accent, 0.5) : Style.border
        border.width: 1
        // Keyboard focus.
        Rectangle {
            anchors.fill: parent
            anchors.margins: -3
            radius: parent.radius + 3
            color: "transparent"
            border.color: Style.accent
            border.width: 2
            visible: button.visualFocus
        }
    }

    contentItem: Item {
        implicitWidth: row.implicitWidth
        implicitHeight: row.implicitHeight
        RowLayout {
            id: row
            anchors.centerIn: parent
            spacing: 6
            AppIcon {
                visible: button.icon.name !== ""
                name: button.icon.name
                color: !button.enabled || button.primary ? button.foreground
                       : button.checked ? Style.accent
                       : button.icon.color.a > 0 ? button.icon.color : button.foreground
                implicitWidth: 16
                implicitHeight: 16
            }
            Controls.Label {
                visible: button.text !== ""
                text: button.text
                color: button.foreground
                font.weight: button.primary ? Font.DemiBold : Font.Medium
            }
        }
    }
}
