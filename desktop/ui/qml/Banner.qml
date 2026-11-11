import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// A message across the page: kind "info", "ok", "warn" or "error", with
// optional buttons on the right.
Rectangle {
    id: banner

    property string kind: "info"
    property string text
    property alias actions: actionRow.data
    readonly property string level: kind === "error" ? "bad" : kind === "info" ? "accent" : kind

    Layout.fillWidth: true
    implicitHeight: row.implicitHeight + 20
    radius: Style.radius
    color: Style.tint(Style.stateColor(level), 0.12)
    border.color: Style.tint(Style.stateColor(level), 0.45)

    RowLayout {
        id: row
        anchors.fill: parent
        anchors.margins: 10
        anchors.leftMargin: 14
        spacing: 10

        AppIcon {
            name: (banner.kind === "ok" ? "check" : banner.kind === "error" ? "error"
                               : banner.kind === "warn" ? "warning" : "help")
            color: Style.stateColor(banner.level)
            implicitWidth: 18
            implicitHeight: 18
        }
        Controls.Label {
            Layout.fillWidth: true
            text: banner.text
            color: Style.text
            wrapMode: Text.WordWrap
        }
        RowLayout {
            id: actionRow
            spacing: 6
        }
    }
}
