import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// Type text on the PlayStation, e.g. a long password from the password
// manager: pasted here, sent as key presses for the PlayStation's keyboard
// language. The bridge must be off (the keyboard is the desktop's then).
Card {
    id: card

    // (Depends on the language too.)
    readonly property string missing: AppController.psLayout !== "" ? AppController.unsupportedChars(field.text) : ""
    readonly property var layouts: JSON.parse(AppController.layoutsJson)
    readonly property bool canType: field.text !== "" && missing === "" && !AppController.typingBusy
                                    && !AppController.active && !AppController.pending && AppController.connected

    title: I18n.tr("Type text on the PlayStation")
    icon: "keyboard"
    subtitle: I18n.tr("For a long password or a message: paste it here, put the cursor in the PlayStation's text field, then \"Type\".")
    help: I18n.tr("The text is typed as key presses, as on a real keyboard, so the keyboard language set on the PlayStation must match the one chosen here. The text is not stored or logged. The bridge must be off while typing.")

    RowLayout {
        Layout.fillWidth: true
        spacing: 8
        Controls.TextField {
            id: field
            Layout.fillWidth: true
            Layout.minimumWidth: 60
            implicitWidth: 160
            placeholderText: I18n.tr("Text to type")
            echoMode: hide.checked ? TextInput.Password : TextInput.Normal
            enabled: !AppController.typingBusy
            onAccepted: if (card.canType) AppController.typeText(field.text)
            Accessible.name: I18n.tr("Text to type")
        }
        AppButton {
            flat: true
            icon.name: "eye"
            icon.color: Style.text
            checkable: true
            checked: !hide.checked
            Accessible.name: I18n.tr("Show the text")
            Controls.ToolTip.visible: hovered
            Controls.ToolTip.text: checked ? I18n.tr("Hide the text") : I18n.tr("Show the text")
            onClicked: hide.checked = !checked
        }
        AppButton {
            // Only the icon in a narrow window.
            text: card.width > 480 ? I18n.tr("Paste") : ""
            Controls.ToolTip.visible: hovered && text === ""
            Controls.ToolTip.text: I18n.tr("Paste")
            icon.name: "download"
            icon.color: Style.text
            enabled: !AppController.typingBusy
            onClicked: {
                field.selectAll();
                field.paste();
            }
            Accessible.description: I18n.tr("Replaces the text with the clipboard")
        }
    }

    // Hidden by default: it is often a password.
    QtObject {
        id: hide
        property bool checked: true
    }

    // Side by side when there is room, else one below the other.
    GridLayout {
        id: langRow
        Layout.fillWidth: true
        columns: card.width > 560 ? 4 : 1
        columnSpacing: 8
        rowSpacing: 6
        Controls.Label {
            Layout.fillWidth: langRow.columns === 1
            wrapMode: Text.WordWrap
            text: I18n.tr("Keyboard language on the PlayStation")
            color: Style.text
        }
        Controls.ComboBox {
            Layout.preferredWidth: 180
            model: card.layouts.map(l => l.name)
            currentIndex: card.layouts.findIndex(l => l.id === AppController.psLayout)
            onActivated: i => AppController.choosePsLayout(card.layouts[i].id)
            Accessible.name: I18n.tr("Keyboard language on the PlayStation")
        }
        Item {
            Layout.fillWidth: true
            visible: langRow.columns > 1
        }
        RowLayout {
            spacing: 8
            AppButton {
                visible: !AppController.typingBusy
                text: I18n.tr("Type")
                highlighted: true
                enabled: card.canType
                onClicked: AppController.typeText(field.text)
            }
            AppButton {
                visible: AppController.typingBusy
                text: I18n.tr("Stop")
                onClicked: AppController.cancelTyping()
            }
        }
    }

    Controls.ProgressBar {
        Layout.fillWidth: true
        visible: AppController.typingBusy
        value: AppController.typingProgress
    }

    Controls.Label {
        Layout.fillWidth: true
        visible: card.missing !== ""
        wrapMode: Text.WordWrap
        color: Style.bad
        text: I18n.trf("This keyboard language has no key for: %1", card.missing)
    }
    Controls.Label {
        Layout.fillWidth: true
        visible: field.text !== "" && (AppController.active || AppController.pending)
        wrapMode: Text.WordWrap
        color: Style.warn
        text: I18n.tr("Switch the bridge off first: while it is on, the keyboard goes to the PlayStation directly.")
    }
    Controls.Label {
        Layout.fillWidth: true
        visible: AppController.typingMessage !== "" && !AppController.typingBusy
        wrapMode: Text.WordWrap
        color: Style.stateColor(AppController.typingLevel)
        text: AppController.typingMessage
    }
}
