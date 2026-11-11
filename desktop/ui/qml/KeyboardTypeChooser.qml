import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// Physical keyboard type (ISO or ANSI). The board sends key positions, so the
// PlayStation layout must match the keyboard anyway; the type only decides
// which code the key next to Enter sends.
RowLayout {
    spacing: 10

    Segmented {
        value: AppController.isoKeyboard ? "iso" : "ansi"
        model: [
            { value: "iso", text: "ISO · 105 keys" },
            { value: "ansi", text: "ANSI · 104 keys" }
        ]
        onActivated: v => AppController.setKeyboardType(v === "iso")
    }
    Controls.Label {
        text: AppController.isoKeyboard ? "e.g. German, UK, French" : "e.g. US"
        color: Style.muted
    }
    HelpHint {
        text: "ISO keyboards have an L-shaped Enter key and an extra key next to the left Shift. The PlayStation receives key positions, not characters: set its keyboard language to the layout printed on your keyboard."
    }
}
