import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// How long before the PlayStation's poll a mouse report is armed, against
// the 1 ms frame: the fill is the lead, the mark its lower bound.
ColumnLayout {
    id: meter
    property int lead: 0
    property int floor: 0
    readonly property int frame: 1000
    spacing: 6

    Item {
        Layout.fillWidth: true
        implicitHeight: 14
        Rectangle {
            id: track
            anchors.verticalCenter: parent.verticalCenter
            width: parent.width
            height: 8
            radius: 4
            color: Style.tint(Style.series[0], 0.18)
        }
        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            width: Math.max(8, Math.min(1, meter.lead / meter.frame) * track.width)
            height: 8
            radius: 4
            color: Style.series[0]
        }
        Rectangle {
            visible: meter.floor > 0
            x: Math.min(1, meter.floor / meter.frame) * track.width - 1
            width: 2
            height: 14
            color: Style.text
        }
        MouseArea {
            id: hover
            anchors.fill: parent
            hoverEnabled: true
        }
        Controls.ToolTip.visible: hover.containsMouse
        Controls.ToolTip.text: `Armed ${meter.lead} µs before the poll, at least ${meter.floor} µs (of a 1000 µs frame)`
    }
    RowLayout {
        Layout.fillWidth: true
        Controls.Label {
            text: `armed ${meter.lead} µs before the poll · at least ${meter.floor} µs (mark)`
            color: Style.muted
            Layout.fillWidth: true
        }
        Controls.Label {
            text: "1 ms frame"
            color: Style.muted
        }
    }
}
