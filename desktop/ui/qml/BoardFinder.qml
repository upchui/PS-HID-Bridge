import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// Boards in the network (AppController.discoverBoards): one row per board
// that answered the search; a tap takes its address. Boards with older
// firmware do not answer and are entered by hand.
ColumnLayout {
    id: finder
    spacing: 8

    // A board was tapped: its address for the address field.
    signal picked(string address)
    // The address in the page's field: that board's row is marked.
    property string selectedAddress: ""
    // A search ran while this page is open (an empty list then means: none
    // answered, not: not asked yet).
    property bool searched: false

    readonly property var boards: JSON.parse(AppController.foundBoardsJson)

    function search() {
        searched = true;
        AppController.discoverBoards();
    }

    RowLayout {
        Layout.fillWidth: true
        spacing: 8
        Controls.Label {
            Layout.fillWidth: true
            text: AppController.discoverBusy ? I18n.tr("Looking for boards in the network …")
                  : finder.boards.length === 1 ? I18n.tr("1 board in the network")
                  : finder.boards.length > 1 ? I18n.trf("%1 boards in the network – pick yours", finder.boards.length)
                  : I18n.tr("Find the board in this network.")
            color: Style.muted
            elide: Text.ElideRight
        }
        AppButton {
            text: AppController.discoverBusy ? I18n.tr("Searching …") : I18n.tr("Search")
            icon.name: "network"
            icon.color: enabled ? Style.text : Style.disabled
            enabled: !AppController.discoverBusy
            onClicked: finder.search()
        }
    }

    Controls.Label {
        Layout.fillWidth: true
        visible: finder.searched && !AppController.discoverBusy && finder.boards.length === 0
        text: I18n.tr("No board answered. Is it plugged in and connected to this network? Boards with older "
                      + "firmware do not answer the search: enter the address by hand (your router lists the "
                      + "board as ps-hid-bridge-…).")
        color: Style.muted
        wrapMode: Text.WordWrap
    }

    Repeater {
        model: finder.boards
        delegate: Rectangle {
            id: row
            required property var modelData
            // Also the board set up under its name (the list has addresses).
            readonly property bool chosen: modelData.address === finder.selectedAddress
                                           || (modelData.current && finder.selectedAddress === AppController.boardAddress)

            Layout.fillWidth: true
            implicitHeight: rowLayout.implicitHeight + 16
            radius: Style.radius
            color: hover.hovered ? Style.tint(Style.text, 0.04) : Style.surfaceAlt
            border.color: chosen ? Style.tint(Style.accent, 0.6) : "transparent"

            HoverHandler { id: hover }
            TapHandler {
                onTapped: finder.picked(row.modelData.address)
            }

            RowLayout {
                id: rowLayout
                anchors.fill: parent
                anchors.margins: 8
                anchors.leftMargin: 12
                spacing: 12

                AppIcon {
                    name: "board"
                    color: row.chosen ? Style.accent : Style.muted
                    implicitWidth: 22
                    implicitHeight: 22
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    Controls.Label {
                        Layout.fillWidth: true
                        text: row.modelData.name !== "" ? row.modelData.name : row.modelData.ip
                        color: Style.text
                        font.weight: row.chosen ? Font.DemiBold : Font.Normal
                        elide: Text.ElideRight
                    }
                    Controls.Label {
                        Layout.fillWidth: true
                        text: I18n.trf("%1 · firmware %2 · %3", row.modelData.address, row.modelData.firmware,
                                       row.modelData.mac)
                        color: Style.muted
                        font.pointSize: Style.fontSize * 0.9
                        elide: Text.ElideRight
                    }
                }
                Pill {
                    visible: !row.modelData.key_set
                    level: "warn"
                    text: I18n.tr("no key yet")
                }
                Pill {
                    visible: row.modelData.current
                    level: AppController.connected ? "ok" : ""
                    text: AppController.connected ? I18n.tr("connected") : I18n.tr("in use")
                }
                Pill {
                    visible: row.chosen && !row.modelData.current
                    level: "accent"
                    text: I18n.tr("selected")
                }
            }
        }
    }
}
