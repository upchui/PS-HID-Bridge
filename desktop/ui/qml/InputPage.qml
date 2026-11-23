import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// Devices that go to the PlayStation, keyboard type, profiles (mouse
// settings, remapped keys) and the touchpad.
AppPage {
    id: page
    title: I18n.tr("Input")
    subtitle: I18n.tr("Selected devices go to the PlayStation while the bridge is on; all others stay with the desktop.")
    ownWarnings: ["access"]

    Component.onCompleted: AppController.refreshDevices()
    // A slider let go just before leaving the page: still saved.
    Component.onDestruction: {
        if (applyTouch.running)
            applyTouch.triggered();
    }

    Card {
        title: I18n.tr("Devices")
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
                text: I18n.tr("Read access through /etc/udev/rules.d/70-ps-hid-bridge.rules. Removing it takes the access back: "
                              + "the app can no longer read keyboard and mouse until it is set up again (administrator password).")
            }
            AppButton {
                text: AppController.accessBusy ? I18n.tr("Removing …") : I18n.tr("Remove access")
                flat: true
                enabled: !AppController.accessBusy && !AppController.active && !AppController.pending
                onClicked: AppController.removeAccess()
                Accessible.description: I18n.tr("Removes the device access rule; asks for the administrator password")
            }
        }
    }

    Card {
        title: I18n.tr("Keyboard type")
        icon: "keyboard"
        KeyboardTypeChooser {}
    }

    ProfileCard {
        id: profiles
    }

    MouseCard {
        index: profiles.index
        profile: profiles.profile
        subtitle: profiles.names.length > 1 ? I18n.trf("Profile \"%1\"", profiles.profile.name ?? "") : ""
    }

    RemapEditor {
        index: profiles.index
        remap: profiles.profile.remap ?? []
        subtitle: profiles.names.length > 1 ? I18n.trf("Profile \"%1\"", profiles.profile.name ?? "")
                                            : I18n.tr("A key or mouse button reaches the PlayStation as another one, e.g. a mouse side button as a key the game wants.")
    }

    Card {
        title: I18n.tr("Touchpad and touchscreen")
        icon: "touchpad"
        subtitle: I18n.tr("One finger moves, two fingers scroll. A touchscreen works like a large touchpad: the PlayStation knows a mouse only.")
        help: I18n.tr("Clicking a clickpad: left; with two fingers on it or in the bottom-right corner right, with three fingers middle. A thumb resting on the bottom of a clickpad does not move the pointer, so it can click while another finger moves. A tap is no click right after a key press (a palm on the touchpad while typing). The mouse sensitivity above applies too.")
        actions: AppButton {
            text: I18n.tr("Defaults")
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
                text: I18n.tr("Speed")
                color: Style.text
            }
            AppSlider {
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
                Accessible.name: I18n.tr("Tap to click")
            }
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                color: Style.text
                text: I18n.tr("Tap to click: one finger left, two right, three middle")
            }
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: 12
            AppSwitch {
                checked: AppController.naturalScroll
                onToggled: AppController.setTouchpad(touchSpeed.value, AppController.tapToClick, checked)
                Accessible.name: I18n.tr("Natural scrolling")
            }
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                color: Style.text
                text: I18n.tr("Natural scrolling: the content follows the fingers")
            }
        }
    }
}
