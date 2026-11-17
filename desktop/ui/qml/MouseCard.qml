import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// Mouse settings of one profile: sensitivity, acceleration, the vertical
// axis and the wheel.
Card {
    id: card

    // The profile (0 = "Standard") and its settings ({sensitivity, accel,
    // invert_y, y_ratio, wheel_speed}).
    property int index: 0
    property var profile: ({})

    readonly property bool raw: sens.value === 1 && accel.value === 0 && !invertY.checked
                                && yRatio.value === 1 && wheel.value === 1

    function apply() {
        AppController.setProfileMouse(card.index, sens.value, accel.value, invertY.checked, yRatio.value, wheel.value);
    }
    // The controls show the profile's values: at the start, for another
    // profile, and after a save (moving a slider ends its binding).
    function sync() {
        sens.value = card.profile.sensitivity ?? 1;
        accel.value = card.profile.accel ?? 0;
        yRatio.value = card.profile.y_ratio ?? 1;
        wheel.value = card.profile.wheel_speed ?? 1;
        invertY.checked = card.profile.invert_y ?? false;
    }
    onProfileChanged: if (!applySoon.running) sync()
    Component.onCompleted: sync()
    // A slider let go just before the card goes: still saved.
    Component.onDestruction: if (applySoon.running) applySoon.triggered()

    title: I18n.tr("Mouse")
    icon: "mouse"
    help: I18n.tr("The default is the raw mouse values (sensitivity 1, no acceleration). Many PlayStation games have their own settings; adjusting there is usually better.")
    actions: AppButton {
        text: I18n.tr("Raw values")
        flat: true
        enabled: !card.raw
        onClicked: {
            sens.value = 1;
            accel.value = 0;
            invertY.checked = false;
            yRatio.value = 1;
            wheel.value = 1;
            card.apply();
        }
    }

    // Applied shortly after a slider stops moving.
    Timer {
        id: applySoon
        interval: 250
        onTriggered: card.apply()
    }

    GridLayout {
        Layout.fillWidth: true
        columns: 3
        columnSpacing: 12
        Controls.Label {
            text: I18n.tr("Sensitivity")
            color: Style.text
        }
        Controls.Slider {
            id: sens
            Layout.fillWidth: true
            // The range the app accepts (also set with pshb-cli).
            from: 0.1
            to: 8
            stepSize: 0.05
            snapMode: Controls.Slider.SnapAlways  // exactly 1.00x again: raw input
            onMoved: applySoon.restart()
        }
        Controls.Label {
            Layout.preferredWidth: 56
            text: sens.value.toFixed(2) + "×"
            color: Style.muted
            horizontalAlignment: Text.AlignRight
        }
        Controls.Label {
            text: I18n.tr("Acceleration")
            color: Style.text
        }
        Controls.Slider {
            id: accel
            Layout.fillWidth: true
            from: 0
            to: 1
            stepSize: 0.01
            snapMode: Controls.Slider.SnapAlways
            onMoved: applySoon.restart()
        }
        Controls.Label {
            Layout.preferredWidth: 56
            text: accel.value === 0 ? I18n.tr("off") : accel.value.toFixed(2)
            color: Style.muted
            horizontalAlignment: Text.AlignRight
        }
        Controls.Label {
            text: I18n.tr("Vertical speed")
            color: Style.text
        }
        Controls.Slider {
            id: yRatio
            Layout.fillWidth: true
            from: 0.25
            to: 4
            stepSize: 0.05
            snapMode: Controls.Slider.SnapAlways
            onMoved: applySoon.restart()
            Accessible.name: I18n.tr("Vertical speed, relative to the horizontal one")
        }
        Controls.Label {
            Layout.preferredWidth: 56
            text: yRatio.value.toFixed(2) + "×"
            color: Style.muted
            horizontalAlignment: Text.AlignRight
        }
        Controls.Label {
            text: I18n.tr("Wheel")
            color: Style.text
        }
        Controls.Slider {
            id: wheel
            Layout.fillWidth: true
            from: 0.25
            to: 4
            stepSize: 0.05
            snapMode: Controls.Slider.SnapAlways
            onMoved: applySoon.restart()
            Accessible.name: I18n.tr("Wheel speed")
        }
        Controls.Label {
            Layout.preferredWidth: 56
            text: wheel.value.toFixed(2) + "×"
            color: Style.muted
            horizontalAlignment: Text.AlignRight
        }
    }

    RowLayout {
        Layout.fillWidth: true
        spacing: 12
        AppSwitch {
            id: invertY
            onToggled: card.apply()
            Accessible.name: I18n.tr("Invert the vertical axis")
        }
        Controls.Label {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            color: Style.text
            text: I18n.tr("Invert the vertical axis: moving the mouse forward moves down")
        }
    }

    CurvePlot {
        Layout.fillWidth: true
        implicitHeight: 150
        sensitivity: sens.value
        accel: accel.value
    }

    Controls.Label {
        Layout.fillWidth: true
        wrapMode: Text.WordWrap
        color: Style.muted
        font.pointSize: Style.fontSize * 0.9
        text: I18n.tr("The vertical speed, the inverted axis and the wheel apply to touchpads too.")
    }
}
