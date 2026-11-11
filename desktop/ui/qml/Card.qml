import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// A rounded panel with an optional title row (icon, title, subtitle, help,
// buttons on the right) and content below.
Rectangle {
    id: card

    property string title
    property string subtitle
    property string icon
    property string help
    property color iconColor: Style.accent
    default property alias content: body.data
    property alias actions: actionRow.data
    property int padding: Style.pad

    Layout.fillWidth: true
    implicitWidth: layout.implicitWidth + 2 * padding
    implicitHeight: layout.implicitHeight + 2 * padding
    color: Style.surface
    radius: Style.radius
    border.color: Style.border
    border.width: 1

    ColumnLayout {
        id: layout
        anchors.fill: parent
        anchors.margins: card.padding
        spacing: Style.gap

        RowLayout {
            Layout.fillWidth: true
            visible: card.title !== "" || actionRow.children.length > 0
            spacing: 10

            AppIcon {
                visible: card.icon !== ""
                name: card.icon
                color: card.iconColor
                implicitWidth: 20
                implicitHeight: 20
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0
                RowLayout {
                    spacing: 6
                    Controls.Label {
                        text: card.title
                        color: Style.text
                        font.pointSize: Style.fontSize * 1.1
                        font.weight: Font.DemiBold
                    }
                    HelpHint {
                        visible: card.help !== ""
                        text: card.help
                    }
                }
                Controls.Label {
                    Layout.fillWidth: true
                    visible: card.subtitle !== ""
                    text: card.subtitle
                    color: Style.muted
                    wrapMode: Text.WordWrap
                }
            }
            Item {
                // keeps the buttons on the right also without a subtitle
                Layout.fillWidth: card.subtitle === ""
            }
            RowLayout {
                id: actionRow
                spacing: 8
            }
        }

        ColumnLayout {
            id: body
            Layout.fillWidth: true
            spacing: 10
        }
    }
}
