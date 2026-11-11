import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import PSHIDBridge

Kirigami.ScrollablePage {
    title: "Triggers"

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Kirigami.Heading { level: 2; text: "Keys and mouse buttons" }
        Controls.Label {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            text: "\"Combination\" fires as soon as all keys are pressed. \"Hold\" turns the bridge on only while the keys stay pressed. \"Double tap\" reacts to a single key pressed twice quickly."
        }
        TriggerEditor {
            Layout.fillWidth: true
        }

        Kirigami.Separator { Layout.fillWidth: true }

        RowLayout {
            Layout.fillWidth: true
            Kirigami.Heading {
                Layout.fillWidth: true
                level: 2
                text: "Window rule"
            }
            Controls.Switch {
                text: AppController.windowRuleEnabled ? "On" : "Off"
                enabled: AppController.windowRuleAvailable
                checked: AppController.windowRuleEnabled
                onToggled: AppController.setWindowRuleOn(checked)
            }
        }
        Controls.Label {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            text: AppController.windowRuleAvailable
                  ? "When switched on, the bridge turns on by itself while a window is active whose title, program or desktop file contains this text, and off again when it loses the focus. The default \"chiaki-ng\" matches the Remote Play window."
                  : "The window rule needs KDE Plasma (KWin). It is not available on this desktop."
        }
        RowLayout {
            enabled: AppController.windowRuleAvailable && AppController.windowRuleEnabled
            Controls.TextField {
                id: ruleField
                Layout.fillWidth: true
                text: AppController.windowRule
                placeholderText: "chiaki-ng"
            }
            Controls.Button {
                text: "Apply"
                enabled: ruleField.text.trim() !== "" && ruleField.text.trim() !== AppController.windowRule
                onClicked: AppController.setWindowRuleText(ruleField.text)
            }
        }

        Kirigami.Separator { Layout.fillWidth: true }

        Kirigami.Heading { level: 2; text: "Emergency stop" }
        Controls.Label {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            text: "Ctrl+Alt+Shift+Esc always releases mouse and keyboard at once and lets go of every key on the PlayStation. This combination cannot be changed."
        }
    }
}
