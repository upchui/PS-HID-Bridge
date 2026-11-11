import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// A page: title (and subtitle) with optional buttons on the right, the
// content scrolls below, an optional footer stays at the bottom.
Item {
    id: page
    anchors.fill: parent

    property string title
    property string subtitle
    default property alias content: column.data
    property alias actions: actionRow.data
    property alias footer: footerRow.data

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        RowLayout {
            Layout.fillWidth: true
            Layout.maximumWidth: Style.maxWidth + 2 * Style.pad
            Layout.alignment: Qt.AlignHCenter
            Layout.margins: Style.pad
            Layout.bottomMargin: Style.gap
            spacing: Style.gap

            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2
                Controls.Label {
                    Layout.fillWidth: true
                    text: page.title
                    color: Style.text
                    font.pointSize: Style.fontSize * 1.6
                    font.weight: Font.DemiBold
                    elide: Text.ElideRight
                }
                Controls.Label {
                    Layout.fillWidth: true
                    visible: page.subtitle !== ""
                    text: page.subtitle
                    color: Style.muted
                    wrapMode: Text.WordWrap
                }
            }
            RowLayout {
                id: actionRow
                spacing: 8
            }
        }

        Controls.ScrollView {
            id: scroll
            Layout.fillWidth: true
            Layout.fillHeight: true
            contentWidth: availableWidth
            clip: true

            Item {
                width: scroll.availableWidth
                implicitHeight: column.implicitHeight + Style.pad

                ColumnLayout {
                    id: column
                    width: Math.min(parent.width - 2 * Style.pad, Style.maxWidth)
                    x: (parent.width - width) / 2
                    spacing: Style.gap
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            visible: footerRow.children.length > 0
            implicitHeight: footerRow.implicitHeight + 2 * Style.gap
            color: Style.surface
            Rectangle {
                width: parent.width
                height: 1
                color: Style.border
            }
            RowLayout {
                id: footerRow
                anchors.fill: parent
                anchors.margins: Style.gap
                anchors.leftMargin: Style.pad
                anchors.rightMargin: Style.pad
                spacing: 8
            }
        }
    }
}
