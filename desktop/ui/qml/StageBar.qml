import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// One whole split into its parts (a tall segmented bar): each part its own
// block with its value inside where it fits, a 2 px gap between blocks, the
// names in the legend below, everything on hover.
// parts: [{name, value}] (values in `unit`).
ColumnLayout {
    id: bar
    property var parts: []
    property string unit: "µs"
    readonly property real total: parts.reduce((a, p) => a + Math.max(0, p.value), 0)
    spacing: 10

    Item {
        id: track
        Layout.fillWidth: true
        implicitHeight: 28
        readonly property int gaps: Math.max(0, bar.parts.filter(p => p.value > 0).length - 1) * 2

        Row {
            anchors.fill: parent
            spacing: 2
            Repeater {
                model: bar.parts
                delegate: Rectangle {
                    id: block
                    required property var modelData
                    required property int index
                    readonly property real share: bar.total > 0 ? Math.max(0, modelData.value) / bar.total : 0
                    width: share * (track.width - track.gaps)
                    visible: modelData.value > 0
                    height: track.height
                    radius: 4
                    color: Style.series[index % Style.series.length]

                    TextMetrics {
                        id: fit
                        text: block.modelData.value + " " + bar.unit
                        font.pointSize: Style.fontSize * 0.9
                        font.weight: Font.DemiBold
                    }
                    Controls.Label {
                        anchors.left: parent.left
                        anchors.leftMargin: 8
                        anchors.verticalCenter: parent.verticalCenter
                        visible: fit.width + 16 <= block.width
                        text: fit.text
                        font: fit.font
                        color: Style.onSeries(block.index)
                    }
                    MouseArea {
                        id: hover
                        anchors.fill: parent
                        anchors.margins: -4
                        hoverEnabled: true
                    }
                    Controls.ToolTip.visible: hover.containsMouse
                    Controls.ToolTip.text: `${block.modelData.name}: ${block.modelData.value} ${bar.unit} (${Math.round(block.share * 100)} %)`
                    Accessible.name: Controls.ToolTip.text
                }
            }
        }
    }

    Flow {
        Layout.fillWidth: true
        spacing: 16
        Repeater {
            model: bar.parts
            delegate: Row {
                required property var modelData
                required property int index
                spacing: 6
                Rectangle {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 10
                    height: 10
                    radius: 2
                    color: Style.series[index % Style.series.length]
                }
                Controls.Label {
                    text: modelData.name
                    color: Style.muted
                }
            }
        }
    }
}
