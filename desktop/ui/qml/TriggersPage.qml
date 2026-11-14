import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

AppPage {
    title: "Triggers"
    subtitle: "How you switch between the desktop and the PlayStation. Changes are saved right away."

    Component.onCompleted: AppController.refreshDevices()

    // Triggers and the emergency stop work on every keyboard the app may
    // read, also on keyboards that are not selected (those are never
    // forwarded).
    Banner {
        kind: "warn"
        visible: AppController.otherKeyboardsBlocked > 0
        text: "Triggers and the emergency stop work on every keyboard, also on ones that are not selected. "
              + AppController.otherKeyboardsBlocked + " keyboard(s) cannot be read yet; the app sets up access once (administrator password)."
        actions: AppButton {
            text: AppController.accessBusy ? "Setting up …" : "Set up access"
            enabled: !AppController.accessBusy
            onClicked: AppController.installAccess()
        }
    }
    Banner {
        kind: "error"
        visible: AppController.accessMessage !== ""
        text: AppController.accessMessage
    }

    Card {
        title: "Keys and mouse buttons"
        icon: "bolt"
        help: "Combination: fires as soon as all keys are pressed. Hold: the bridge is on only while the keys stay pressed. Double tap: a single key pressed twice quickly."
        TriggerEditor {
            Layout.fillWidth: true
        }
    }

    Card {
        title: "Window rule"
        icon: "eye"
        subtitle: AppController.windowRuleAvailable
                  ? "On by itself while a matching window is active (e.g. the Remote Play window)."
                  : "Needs KDE Plasma (KWin); not available on this desktop."
        help: "The bridge switches on while a window is active whose title, program or desktop file contains this text, and off when it loses the focus. \"chiaki-ng\" matches the Remote Play window."
        actions: AppSwitch {
            enabled: AppController.windowRuleAvailable
            checked: AppController.windowRuleEnabled
            onToggled: AppController.setWindowRuleOn(checked)
            Accessible.name: "Window rule"
        }

        Controls.TextField {
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
        title: "Emergency stop"
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
                text: "Always releases mouse and keyboard and every key on the PlayStation."
                color: Style.muted
                wrapMode: Text.WordWrap
            }
        }
    }
}
