import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// Spread of one measure per row on one shared axis: a bar from the fastest
// to the 99th percentile, a dot at the median, a mark at the slowest (an
// arrow at the axis end if it lies beyond). The figures stand to the right
// (they are also the table of the chart); hover shows everything.
// rows: [{label, n, min, p50, p99, max}] in µs.
ColumnLayout {
    id: chart
    property var rows: []
    property int labelWidth: 210
    readonly property int valuesWidth: 170
    readonly property real scaleMax: nice(Math.max(1, ...rows.map(r => r.p99)) * 1.15)
    readonly property var ticks: [0, 1, 2, 3, 4].map(i => scaleMax * i / 4)
    spacing: 0

    function nice(v) {
        const mag = Math.pow(10, Math.floor(Math.log10(v)));
        for (const s of [1, 2, 2.5, 5, 10]) {
            if (s * mag >= v)
                return s * mag;
        }
        return 10 * mag;
    }
    function fmt(us) {
        return Math.round(us) + " µs";
    }
    function pos(v, w) {
        return Math.min(1, Math.max(0, v / scaleMax)) * w;
    }

    RowLayout {
        Layout.fillWidth: true
        spacing: 12
        Item { Layout.preferredWidth: chart.labelWidth }
        Item { Layout.fillWidth: true }
        Controls.Label {
            Layout.preferredWidth: chart.valuesWidth
            text: "median · 99 % · slowest"
            color: Style.muted
            font.pointSize: Style.fontSize * 0.85
            horizontalAlignment: Text.AlignRight
        }
    }

    Repeater {
        model: chart.rows
        delegate: RowLayout {
            id: row
            required property var modelData
            Layout.fillWidth: true
            Layout.preferredHeight: 28
            spacing: 12

            Controls.Label {
                Layout.preferredWidth: chart.labelWidth
                text: row.modelData.label
                color: Style.text
                elide: Text.ElideRight
            }
            Item {
                id: plot
                Layout.fillWidth: true
                Layout.fillHeight: true
                readonly property bool beyond: row.modelData.max > chart.scaleMax

                Repeater {
                    model: chart.ticks
                    delegate: Rectangle {
                        required property real modelData
                        x: chart.pos(modelData, plot.width)
                        width: 1
                        height: plot.height
                        color: Style.grid
                    }
                }
                Rectangle {  // fastest .. 99 %
                    x: chart.pos(row.modelData.min, plot.width)
                    width: Math.max(4, chart.pos(row.modelData.p99, plot.width) - x)
                    height: 10
                    radius: 4
                    anchors.verticalCenter: parent.verticalCenter
                    color: Style.tint(Style.series[0], 0.55)
                }
                Rectangle {  // slowest
                    visible: !plot.beyond
                    x: chart.pos(row.modelData.max, plot.width) - 1
                    width: 2
                    height: 14
                    anchors.verticalCenter: parent.verticalCenter
                    color: Style.series[0]
                }
                Controls.Label {  // slowest beyond the axis
                    visible: plot.beyond
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    text: "▸"
                    color: Style.series[0]
                }
                Rectangle {  // median, with a ring in the surface colour
                    x: chart.pos(row.modelData.p50, plot.width) - width / 2
                    width: 12
                    height: 12
                    radius: 6
                    anchors.verticalCenter: parent.verticalCenter
                    color: Style.series[0]
                    border.color: Style.surface
                    border.width: 2
                }
                MouseArea {
                    id: hover
                    anchors.fill: parent
                    hoverEnabled: true
                }
                Controls.ToolTip.visible: hover.containsMouse
                Controls.ToolTip.text: `${row.modelData.label}: median ${chart.fmt(row.modelData.p50)}, 99 % below ${chart.fmt(row.modelData.p99)}, fastest ${chart.fmt(row.modelData.min)}, slowest ${chart.fmt(row.modelData.max)} (${row.modelData.n} times)`
            }
            Controls.Label {
                Layout.preferredWidth: chart.valuesWidth
                text: `${chart.fmt(row.modelData.p50)} · ${chart.fmt(row.modelData.p99)} · ${chart.fmt(row.modelData.max)}`
                color: Style.muted
                font.features: { "tnum": 1 }
                horizontalAlignment: Text.AlignRight
            }
        }
    }

    RowLayout {  // axis
        Layout.fillWidth: true
        spacing: 12
        Item { Layout.preferredWidth: chart.labelWidth }
        Item {
            id: axis
            Layout.fillWidth: true
            implicitHeight: 18
            Repeater {
                model: chart.ticks
                delegate: Controls.Label {
                    required property real modelData
                    required property int index
                    x: Math.min(axis.width - width, Math.max(0, chart.pos(modelData, axis.width) - width / 2))
                    text: chart.fmt(modelData)
                    color: Style.muted
                    font.pointSize: Style.fontSize * 0.85
                    font.features: { "tnum": 1 }
                }
            }
        }
        Item { Layout.preferredWidth: chart.valuesWidth }
    }
}
