import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// Devices that go to the PlayStation, keyboard type and mouse settings.
AppPage {
    id: page
    title: "Input"
    subtitle: "Selected devices go to the PlayStation while the bridge is on; all others stay with the desktop."

    Component.onCompleted: AppController.refreshDevices()
    // A slider let go just before leaving the page: still saved.
    Component.onDestruction: {
        if (applyMouse.running)
            applyMouse.triggered();
        if (applyTouch.running)
            applyTouch.triggered();
    }

    Card {
        title: "Devices"
        icon: "input"
        DeviceList {
            Layout.fillWidth: true
        }
        RowLayout {
            visible: AppController.accessRuleInstalled
            Layout.fillWidth: true
            spacing: 12
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                color: Style.muted
                text: "Read access through /etc/udev/rules.d/70-ps-hid-bridge.rules. Removing it takes the access back: "
                      + "the app can no longer read keyboard and mouse until it is set up again (administrator password)."
            }
            AppButton {
                text: AppController.accessBusy ? "Removing …" : "Remove access"
                flat: true
                enabled: !AppController.accessBusy && !AppController.active && !AppController.pending
                onClicked: AppController.removeAccess()
                Accessible.description: "Removes the device access rule; asks for the administrator password"
            }
        }
    }

    Card {
        title: "Keyboard type"
        icon: "keyboard"
        KeyboardTypeChooser {}
    }

    Card {
        title: "Mouse"
        icon: "mouse"
        help: "The default is the raw mouse values (sensitivity 1, no acceleration). Many PlayStation games have their own settings; adjusting there is usually better."
        actions: AppButton {
            text: "Raw values"
            flat: true
            enabled: sens.value !== 1 || accel.value !== 0
            onClicked: {
                sens.value = 1;
                accel.value = 0;
                AppController.setMouse(1, 0);
            }
        }

        // Applied shortly after the slider stops moving.
        Timer {
            id: applyMouse
            interval: 250
            onTriggered: AppController.setMouse(sens.value, accel.value)
        }

        GridLayout {
            Layout.fillWidth: true
            columns: 3
            columnSpacing: 12
            Controls.Label {
                text: "Sensitivity"
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
                value: AppController.sensitivity
                onMoved: applyMouse.restart()
            }
            Controls.Label {
                Layout.preferredWidth: 56
                text: sens.value.toFixed(2) + "×"
                color: Style.muted
                horizontalAlignment: Text.AlignRight
            }
            Controls.Label {
                text: "Acceleration"
                color: Style.text
            }
            Controls.Slider {
                id: accel
                Layout.fillWidth: true
                from: 0
                to: 1
                stepSize: 0.01
                snapMode: Controls.Slider.SnapAlways
                value: AppController.accel
                onMoved: applyMouse.restart()
            }
            Controls.Label {
                Layout.preferredWidth: 56
                text: accel.value === 0 ? "off" : accel.value.toFixed(2)
                color: Style.muted
                horizontalAlignment: Text.AlignRight
            }
        }

        CurvePlot {
            Layout.fillWidth: true
            implicitHeight: 150
            sensitivity: sens.value
            accel: accel.value
        }
    }

    Card {
        title: "Touchpad and touchscreen"
        icon: "touchpad"
        subtitle: "One finger moves, two fingers scroll. A touchscreen works like a large touchpad: the PlayStation knows a mouse only."
        help: "Clicking a clickpad: left; with two fingers on it or in the bottom-right corner right, with three fingers middle. A thumb resting on the bottom of a clickpad does not move the pointer, so it can click while another finger moves. A tap is no click right after a key press (a palm on the touchpad while typing). The mouse sensitivity above applies too."
        actions: AppButton {
            text: "Defaults"
            flat: true
            enabled: touchSpeed.value !== 1 || !AppController.tapToClick || AppController.naturalScroll
            onClicked: {
                touchSpeed.value = 1;
                AppController.setTouchpad(1, true, false);
            }
        }

        // Applied shortly after the slider stops moving.
        Timer {
            id: applyTouch
            interval: 250
            onTriggered: AppController.setTouchpad(touchSpeed.value, AppController.tapToClick, AppController.naturalScroll)
        }

        GridLayout {
            Layout.fillWidth: true
            columns: 3
            columnSpacing: 12
            Controls.Label {
                text: "Speed"
                color: Style.text
            }
            Controls.Slider {
                id: touchSpeed
                Layout.fillWidth: true
                // The range the app accepts (also set with pshb-cli).
                from: 0.1
                to: 8
                stepSize: 0.05
                snapMode: Controls.Slider.SnapAlways
                value: AppController.touchpadSpeed
                onMoved: applyTouch.restart()
            }
            Controls.Label {
                Layout.preferredWidth: 56
                text: touchSpeed.value.toFixed(2) + "×"
                color: Style.muted
                horizontalAlignment: Text.AlignRight
            }
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: 12
            AppSwitch {
                checked: AppController.tapToClick
                onToggled: AppController.setTouchpad(touchSpeed.value, checked, AppController.naturalScroll)
                Accessible.name: "Tap to click"
            }
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                color: Style.text
                text: "Tap to click: one finger left, two right, three middle"
            }
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: 12
            AppSwitch {
                checked: AppController.naturalScroll
                onToggled: AppController.setTouchpad(touchSpeed.value, AppController.tapToClick, checked)
                Accessible.name: "Natural scrolling"
            }
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                color: Style.text
                text: "Natural scrolling: the content follows the fingers"
            }
        }
    }
}
