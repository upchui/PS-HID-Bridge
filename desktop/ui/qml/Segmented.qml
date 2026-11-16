import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// A row of mutually exclusive choices: model [{value, text?, icon?, tip?}]
// (tip: a tooltip, e.g. for an option with only an icon).
Rectangle {
    id: seg

    property var model: []
    property string value
    signal activated(string value)

    implicitWidth: row.implicitWidth + 6
    implicitHeight: row.implicitHeight + 6
    radius: Style.radius
    color: Style.surfaceAlt
    border.color: Style.border

    RowLayout {
        id: row
        anchors.centerIn: parent
        spacing: 2

        Repeater {
            model: seg.model
            delegate: Controls.AbstractButton {
                id: option
                required property var modelData
                readonly property bool selected: seg.value === modelData.value
                horizontalPadding: 10
                verticalPadding: 5
                implicitWidth: implicitContentWidth + leftPadding + rightPadding
                implicitHeight: implicitContentHeight + topPadding + bottomPadding
                hoverEnabled: true
                Accessible.name: modelData.tip ?? modelData.text ?? modelData.value
                onClicked: if (!selected) seg.activated(modelData.value)
                Controls.ToolTip.visible: hovered && modelData.tip !== undefined
                Controls.ToolTip.text: modelData.tip ?? ""
                Controls.ToolTip.delay: 500

                background: Rectangle {
                    radius: Style.radius - 2
                    color: option.selected ? Style.surface
                         : option.pressed ? Style.tint(Style.text, 0.1)
                         : option.hovered ? Style.tint(Style.text, 0.05) : "transparent"
                    border.color: option.selected ? Style.border : "transparent"
        // Keyboard focus.
        Rectangle {
            anchors.fill: parent
            anchors.margins: -3
            radius: parent.radius + 3
            color: "transparent"
            border.color: Style.accent
            border.width: 2
            visible: option.visualFocus
        }
                }
                contentItem: RowLayout {
                    id: content
                    spacing: 6
                    AppIcon {
                        visible: option.modelData.icon !== undefined
                        name: option.modelData.icon ?? ""
                        color: option.selected ? Style.accent : Style.muted
                        implicitWidth: 16
                        implicitHeight: 16
                    }
                    Controls.Label {
                        visible: option.modelData.text !== undefined
                        text: option.modelData.text !== undefined ? option.modelData.text : ""
                        color: option.selected ? Style.text : Style.muted
                        font.weight: option.selected ? Font.DemiBold : Font.Normal
                    }
                }
            }
        }
    }
}
