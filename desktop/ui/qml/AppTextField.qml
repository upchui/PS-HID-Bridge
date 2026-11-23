import QtQuick
import QtQuick.Controls as Controls
import PSHIDBridge

// Text field in the app's style: rounded like the buttons, a stronger
// border on hover, an accent border while typing in it.
Controls.TextField {
    id: field

    hoverEnabled: true
    leftPadding: 10
    rightPadding: 10
    topPadding: 6
    bottomPadding: 6
    color: enabled ? Style.text : Style.disabled
    placeholderTextColor: enabled ? Style.muted : Style.disabled
    selectionColor: Style.accent
    selectedTextColor: Style.accentText
    verticalAlignment: TextInput.AlignVCenter

    // A value longer than the field shows its beginning while it is not
    // being edited (the cursor would keep the end in view).
    function showStart() {
        if (!activeFocus)
            cursorPosition = 0;
    }
    onTextChanged: showStart()
    onActiveFocusChanged: showStart()
    Component.onCompleted: showStart()

    background: Rectangle {
        implicitWidth: 120
        implicitHeight: 32
        radius: 7
        color: field.enabled ? Style.field : Style.tint(Style.text, 0.04)
        border.width: field.activeFocus ? 2 : 1
        border.color: field.activeFocus ? Style.accent
                    : field.enabled && field.hovered ? Style.tint(Style.text, 0.35) : Style.border
    }
}
