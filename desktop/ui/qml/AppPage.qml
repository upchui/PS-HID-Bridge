import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// A page: title (and subtitle) with optional buttons on the right, the
// content scrolls below, an optional footer stays at the bottom. The
// warnings that concern the whole app (AppWarnings) come first.
Item {
    id: page
    anchors.fill: parent

    property string title
    property string subtitle
    default property alias content: column.data
    property alias actions: actionRow.data
    property alias footer: footerRow.data
    // The app-wide warnings on top (the wizard shows its own per step).
    property bool warnings: true
    // Those of them that belong to this page (keys of Warnings.list), in
    // full; the others are summed up in one line.
    property var ownWarnings: []

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        RowLayout {
            Layout.fillWidth: true
            // As wide as the content below (the margins come on top).
            Layout.maximumWidth: Style.maxWidth
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

        // Settings that could not be written: on every page, so it shows
        // where the change was made. It goes once a save works again.
        Banner {
            Layout.maximumWidth: Style.maxWidth
            Layout.alignment: Qt.AlignHCenter
            Layout.leftMargin: Style.pad
            Layout.rightMargin: Style.pad
            Layout.bottomMargin: Style.gap
            kind: "error"
            visible: AppController.saveError !== ""
            text: AppController.saveError
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

                    // Declared here, so before the page's own content.
                    AppWarnings {
                        id: appWarnings
                        Layout.fillWidth: true
                        visible: page.warnings && appWarnings.any
                        own: page.ownWarnings
                    }
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
