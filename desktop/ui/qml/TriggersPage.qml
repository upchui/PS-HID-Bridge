import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

AppPage {
    title: I18n.tr("Triggers")
    subtitle: I18n.tr("How you switch between the desktop and the PlayStation. Changes are saved right away (a trigger once it has keys).")

    Component.onCompleted: AppController.refreshDevices()

    Card {
        title: I18n.tr("Keys and mouse buttons")
        icon: "bolt"
        help: I18n.tr("Combination: fires as soon as all keys are pressed. Hold: the bridge is on only while the keys stay pressed. Double tap: a single key pressed twice quickly.")
        TriggerEditor {
            Layout.fillWidth: true
        }
    }

    Card {
        title: I18n.tr("Window rule")
        icon: "eye"
        subtitle: AppController.windowRuleAvailable
                  ? I18n.trf("On by itself while a matching window is active (e.g. the Remote Play window). Windows from: %1.", AppController.windowBackend)
                  : I18n.trf("Not available here: %1. It works on KDE Plasma, Hyprland, Sway, i3, niri and X11 desktops.", AppController.windowBackend)
        help: I18n.tr("The bridge switches on while a window is active whose title, program or desktop file contains this text, and off when it loses the focus. \"chiaki-ng\" matches the Remote Play window.")
        actions: AppSwitch {
            enabled: AppController.windowRuleAvailable
            checked: AppController.windowRuleEnabled
            onToggled: AppController.setWindowRuleOn(checked)
            Accessible.name: I18n.tr("Window rule")
        }

        AppTextField {
            Layout.fillWidth: true
            enabled: AppController.windowRuleAvailable && AppController.windowRuleEnabled
            text: AppController.windowRule
            placeholderText: "chiaki-ng"
            onEditingFinished: {
                const t = text.trim();
                if (t !== "" && t !== AppController.windowRule)
                    AppController.setWindowRuleText(t);
            }
        }
    }

    Card {
        title: I18n.tr("Emergency stop")
        icon: "warning"
        iconColor: Style.warn
        RowLayout {
            spacing: 10
            Rectangle {
                implicitWidth: esc.implicitWidth + 16
                implicitHeight: esc.implicitHeight + 8
                radius: 5
                color: Style.surfaceAlt
                border.color: Style.border
                Controls.Label {
                    id: esc
                    anchors.centerIn: parent
                    text: "Ctrl+Alt+Shift+Esc"
                    color: Style.text
                    font.weight: Font.DemiBold
                }
            }
            Controls.Label {
                Layout.fillWidth: true
                text: I18n.tr("Always releases mouse and keyboard and every key on the PlayStation.")
                color: Style.muted
                wrapMode: Text.WordWrap
            }
        }
    }
}
