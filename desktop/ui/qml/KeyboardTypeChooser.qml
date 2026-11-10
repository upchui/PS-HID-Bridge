import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import PS5HIDBridge

// Physical keyboard type (ISO or ANSI). The board sends key positions, so the
// PS5 layout must match the keyboard anyway; the type only decides which code
// the key next to Enter sends.
ColumnLayout {
    spacing: Kirigami.Units.smallSpacing

    Kirigami.Heading { level: 3; text: "Keyboard type" }
    Flow {
        Layout.fillWidth: true
        spacing: Kirigami.Units.largeSpacing
        Controls.RadioButton {
            text: "ISO (105 keys, e.g. German, UK, French)"
            checked: AppController.isoKeyboard
            onToggled: if (checked) AppController.setKeyboardType(true)
        }
        Controls.RadioButton {
            text: "ANSI (104 keys, e.g. US)"
            checked: !AppController.isoKeyboard
            onToggled: if (checked) AppController.setKeyboardType(false)
        }
    }
    Controls.Label {
        Layout.fillWidth: true
        wrapMode: Text.WordWrap
        opacity: 0.8
        text: "The PS5 receives key positions, not characters: set its keyboard language to the layout printed on your keyboard. ISO keyboards have an L-shaped Enter key and an extra key next to the left Shift."
    }
}
