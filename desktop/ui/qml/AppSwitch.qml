import QtQuick
import QtQuick.Controls as Controls
import PSHIDBridge

// On/off switch in the app's style (a rounded track with a knob).
Controls.Switch {
    id: control

    property bool big: false

    hoverEnabled: true
    padding: 0
    spacing: 8

    indicator: Rectangle {
        implicitWidth: control.big ? 54 : 40
        implicitHeight: control.big ? 30 : 22
        x: control.leftPadding
        y: control.topPadding + (control.availableHeight - height) / 2
        radius: height / 2
        color: !control.enabled ? Style.tint(Style.text, 0.08)
             : control.checked ? (control.hovered ? Qt.lighter(Style.accent, 1.08) : Style.accent)
             : control.hovered ? Style.tint(Style.text, 0.24) : Style.tint(Style.text, 0.18)
        border.color: control.visualFocus ? Style.accent : "transparent"

        Rectangle {
            readonly property int inset: 3
            width: parent.height - 2 * inset
            height: width
            radius: width / 2
            y: inset
            x: control.checked ? parent.width - width - inset : inset
            color: control.enabled ? "#ffffff" : Qt.rgba(1, 1, 1, Style.dark ? 0.3 : 0.8)
            Behavior on x {
                NumberAnimation { duration: 120; easing.type: Easing.OutCubic }
            }
        }
    }

    contentItem: Controls.Label {
        leftPadding: control.text !== "" ? control.indicator.width + control.spacing : control.indicator.width
        text: control.text
        color: control.enabled ? Style.text : Style.disabled
        verticalAlignment: Text.AlignVCenter
    }
}
