import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// Spread of one measure per row on one shared axis: a bar from the fastest
// to the 99th percentile, a dot at the median, a mark at the slowest (an
// arrow at the axis end if it lies beyond). The figures stand to the right
// (they are also the table of the chart); hover shows everything.
// Narrow: name and figures on one line, the bar below them at full width.
// rows: [{label, n, min, p50, p99, max}] in µs.
ColumnLayout {
    id: chart
    property var rows: []
    readonly property bool narrow: width < 560
    readonly property int labelWidth: Math.min(210, Math.round(width * 0.3))
    readonly property int valuesWidth: 170
    // The plot's width (the axis below has the same).
    readonly property real plotWidth: narrow ? width : Math.max(0, width - labelWidth - valuesWidth - 24)
    // Axis steps of 1, 2 or 5 × 10^n, about four of them (two on a short axis).
    readonly property real upper: Math.max(1, ...rows.map(r => r.p99)) * 1.15
    readonly property real step: niceStep(upper / (plotWidth < 240 ? 2 : 4))
    readonly property real scaleMax: Math.ceil(upper / step) * step
    readonly property var ticks: {
        const t = [];
        for (let v = 0; v <= scaleMax + step / 2; v += step)
            t.push(v);
        return t;
    }
    spacing: 0

    function niceStep(v) {
        const mag = Math.pow(10, Math.floor(Math.log10(v)));
        for (const s of [1, 2, 5]) {
            if (s * mag >= v)
                return s * mag;
        }
        return 10 * mag;
    }
    function fmt(us) {
        return Style.us(us);
    }
    // Axis labels in one unit: all in ms once the scale reaches 1 ms.
    function tick(us) {
        return scaleMax >= 1000 ? Style.ms(us / 1000) : Style.us(us);
    }
    function pos(v, w) {
        return Math.min(1, Math.max(0, v / scaleMax)) * w;
    }

    Controls.Label {
        Layout.fillWidth: true
        Layout.bottomMargin: 2
        text: I18n.tr("median · 99\u00a0% · slowest")
        color: Style.muted
        font.pointSize: Style.fontSize * 0.85
        horizontalAlignment: Text.AlignRight
    }

    Repeater {
        model: chart.rows
        delegate: GridLayout {
            id: row
            required property var modelData
            Layout.fillWidth: true
            columns: chart.narrow ? 2 : 3
            columnSpacing: 12
            rowSpacing: 0

            Controls.Label {
                Layout.row: 0
                Layout.column: 0
                Layout.fillWidth: chart.narrow
                Layout.preferredWidth: chart.narrow ? -1 : chart.labelWidth
                text: row.modelData.label
                color: Style.text
                elide: Text.ElideRight
            }
            Item {
                id: plot
                Layout.row: chart.narrow ? 1 : 0
                Layout.column: chart.narrow ? 0 : 1
                Layout.columnSpan: chart.narrow ? 2 : 1
                Layout.fillWidth: true
                Layout.preferredHeight: 28
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
                Controls.ToolTip.text: I18n.trf("%1: median %2, 99\u00a0% below %3, fastest %4, slowest %5 (%6)", row.modelData.label,
                                                chart.fmt(row.modelData.p50), chart.fmt(row.modelData.p99), chart.fmt(row.modelData.min),
                                                chart.fmt(row.modelData.max), Style.plural(row.modelData.n, I18n.tr("time"), I18n.tr("times")))
            }
            Controls.Label {
                Layout.row: 0
                Layout.column: chart.narrow ? 1 : 2
                Layout.preferredWidth: chart.narrow ? -1 : chart.valuesWidth
                Layout.alignment: Qt.AlignRight | Qt.AlignVCenter
                text: `${chart.fmt(row.modelData.p50)} · ${chart.fmt(row.modelData.p99)} · ${chart.fmt(row.modelData.max)}`
                color: Style.muted
                font.features: { "tnum": 1 }
                horizontalAlignment: Text.AlignRight
            }
        }
    }

    Item {  // axis, under the plots (as wide: margins for the columns beside them)
        id: axis
        Layout.fillWidth: true
        Layout.leftMargin: chart.narrow ? 0 : chart.labelWidth + 12
        Layout.rightMargin: chart.narrow ? 0 : chart.valuesWidth + 12
        implicitHeight: 18
        Repeater {
            model: chart.ticks
            delegate: Controls.Label {
                required property real modelData
                x: Math.min(axis.width - width, Math.max(0, chart.pos(modelData, axis.width) - width / 2))
                text: chart.tick(modelData)
                color: Style.muted
                font.pointSize: Style.fontSize * 0.85
                font.features: { "tnum": 1 }
            }
        }
    }
}
