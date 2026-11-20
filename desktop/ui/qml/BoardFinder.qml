import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// Boards in the network, from the search that runs all the time (its own
// thread, round after round): one row per board that answers; a tap takes
// its address. A turning symbol shows that the search goes on. Boards with
// older firmware do not answer and are entered by hand.
ColumnLayout {
    id: finder
    spacing: 8

    // A board was tapped: its address for the address field.
    signal picked(string address)
    // The address in the page's field: that board's row is marked.
    property string selectedAddress: ""
    // The page has been open long enough for every board to have answered
    // (an empty list then means: none answers, not: still looking).
    property bool settled: false

    readonly property var boards: JSON.parse(AppController.foundBoardsJson)

    Timer {
        interval: 3000
        running: true
        onTriggered: finder.settled = true
    }

    RowLayout {
        Layout.fillWidth: true
        spacing: 8
        // Turns while the search runs.
        AppIcon {
            id: spinner
            name: "reconnect"
            color: Style.accent
            implicitWidth: 16
            implicitHeight: 16
            visible: AppController.searching
            RotationAnimation on rotation {
                running: spinner.visible
                from: 0
                to: 360
                duration: 1200
                loops: Animation.Infinite
            }
        }
        Controls.Label {
            Layout.fillWidth: true
            text: !AppController.searching ? I18n.tr("The search in the network is not running.")
                  : finder.boards.length === 1 ? I18n.tr("Searching the network · 1 board")
                  : finder.boards.length > 1 ? I18n.trf("Searching the network · %1 boards – pick yours", finder.boards.length)
                  : I18n.tr("Searching the network …")
            color: Style.muted
            wrapMode: Text.WordWrap
        }
    }

    Controls.Label {
        Layout.fillWidth: true
        visible: finder.settled && AppController.searching && finder.boards.length === 0
        text: I18n.tr("No board answers. Is it plugged in and connected to this network? Boards with older "
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
