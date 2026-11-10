import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import PS5HIDBridge

Kirigami.ScrollablePage {
    title: "Auslöser"

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Kirigami.Heading { level: 2; text: "Tasten und Maustasten" }
        Controls.Label {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            text: "„Kombination“ löst aus, sobald alle Tasten gedrückt sind. „Halten“ schaltet die Steuerung nur ein, solange die Tasten gedrückt bleiben. „Zweimal tippen“ reagiert auf eine einzelne Taste, die schnell zweimal gedrückt wird."
        }
        TriggerEditor {
            Layout.fillWidth: true
        }

        Kirigami.Separator { Layout.fillWidth: true }

        Kirigami.Heading { level: 2; text: "Fensterregel" }
        Controls.Label {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            text: AppController.windowRuleAvailable
                  ? "Die Steuerung schaltet sich automatisch ein, solange ein Fenster aktiv ist, dessen Name oder Programm diesen Text enthält (z. B. „chiaki“). Leer lassen zum Abschalten."
                  : "Die Fensterregel braucht KDE Plasma (KWin). Auf diesem Desktop ist sie nicht verfügbar."
        }
        RowLayout {
            enabled: AppController.windowRuleAvailable
            Controls.TextField {
                id: ruleField
                Layout.fillWidth: true
                text: AppController.windowRule
                placeholderText: "chiaki"
            }
            Controls.Button {
                text: "Übernehmen"
                onClicked: AppController.setWindowRuleText(ruleField.text)
            }
        }

        Kirigami.Separator { Layout.fillWidth: true }

        Kirigami.Heading { level: 2; text: "Not-Aus" }
        Controls.Label {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            text: "Strg+Alt+Shift+Esc gibt Maus und Tastatur immer sofort frei und lässt auf der PS5 alle Tasten los. Diese Kombination lässt sich nicht ändern."
        }
    }
}
