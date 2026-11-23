import QtQuick
import QtQuick.Controls as Controls
import PSHIDBridge

// Slider in the app's style: a thin rounded track, filled in the accent
// colour up to a round knob.
Controls.Slider {
    id: slider

    hoverEnabled: true
    padding: 4

    background: Item {
        implicitWidth: 160
        implicitHeight: 24
        x: slider.leftPadding
        y: slider.topPadding + (slider.availableHeight - height) / 2
        width: slider.availableWidth
        height: 4

        Rectangle {
            anchors.fill: parent
            radius: 2
            color: Style.tint(Style.text, slider.enabled ? 0.15 : 0.08)
        }
        Rectangle {
            width: slider.visualPosition * parent.width
            height: parent.height
            radius: 2
            color: slider.enabled ? Style.accent : Style.disabled
        }
    }

    handle: Rectangle {
        readonly property int size: 18
        x: slider.leftPadding + slider.visualPosition * (slider.availableWidth - size)
        y: slider.topPadding + (slider.availableHeight - size) / 2
        implicitWidth: size
        implicitHeight: size
        radius: size / 2
        color: Style.dark ? Style.surface : "#ffffff"
        border.width: 2
        border.color: slider.enabled ? Style.accent : Style.disabled

        // Hover and drag: a soft ring around the knob.
        Rectangle {
            anchors.centerIn: parent
            width: parent.width + 10
            height: width
            radius: width / 2
            z: -1
            color: Style.tint(Style.accent, slider.pressed ? 0.25 : 0.14)
            visible: slider.enabled && (slider.hovered || slider.pressed)
        }
        // Keyboard focus.
        Rectangle {
            anchors.centerIn: parent
            width: parent.width + 8
            height: width
            radius: width / 2
            color: "transparent"
            border.color: Style.accent
            border.width: 2
            visible: slider.visualFocus
        }
    }
}
