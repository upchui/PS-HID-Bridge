import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// Pairing without copying the key: the app logs in to the board's web
// interface with its password, has the board make a new key and stores it.
// A board with its factory password needs an own one first.
ColumnLayout {
    id: box

    // The board's address (from the address field next to it).
    property string address
    // The board has a key already (from the search): a new one replaces it.
    property bool keySet: false

    spacing: 8

    Controls.Label {
        Layout.fillWidth: true
        wrapMode: Text.WordWrap
        color: Style.text
        text: I18n.tr("Or let the app get the key: enter the board's web password.")
    }
    // Side by side when there is room, else one below the other.
    GridLayout {
        Layout.fillWidth: true
        columns: box.width > 480 ? 2 : 1
        columnSpacing: 8
        rowSpacing: 6
        Controls.TextField {
            id: password
            Layout.fillWidth: true
            Layout.minimumWidth: 60
            Layout.preferredWidth: 160
            echoMode: TextInput.Password
            placeholderText: I18n.tr("Web password (on a new board: ps-hid-bridge)")
            enabled: !AppController.pairBusy
            onAccepted: getKey.clicked()
            Accessible.name: I18n.tr("The board's web password")
        }
        AppButton {
            id: getKey
            text: AppController.pairBusy ? I18n.tr("Getting the key …") : I18n.tr("Get the key")
            icon.name: "key"
            icon.color: Style.text
            enabled: !AppController.pairBusy && box.address.trim() !== "" && password.text !== ""
                     && (!AppController.pairNeedsNewPassword || (newPassword.text.length >= 8 && newPassword.text === repeat.text))
            onClicked: AppController.pairBoard(box.address, password.text,
                                               AppController.pairNeedsNewPassword ? newPassword.text : "")
        }
    }
    ColumnLayout {
        Layout.fillWidth: true
        visible: AppController.pairNeedsNewPassword
        spacing: 6
        Controls.TextField {
            id: newPassword
            Layout.fillWidth: true
            Layout.minimumWidth: 60
            Layout.preferredWidth: 160
            echoMode: TextInput.Password
            placeholderText: I18n.tr("Your own web password (8 to 64 characters)")
            Accessible.name: I18n.tr("New web password")
        }
        Controls.TextField {
            id: repeat
            Layout.fillWidth: true
            Layout.minimumWidth: 60
            Layout.preferredWidth: 160
            echoMode: TextInput.Password
            placeholderText: I18n.tr("The same again")
            Accessible.name: I18n.tr("New web password again")
        }
        Controls.Label {
            Layout.fillWidth: true
            visible: repeat.text !== "" && repeat.text !== newPassword.text
            color: Style.bad
            text: I18n.tr("The two passwords differ.")
        }
    }
    Controls.Label {
        Layout.fillWidth: true
        visible: box.keySet
        wrapMode: Text.WordWrap
        color: Style.muted
        font.pointSize: Style.fontSize * 0.9
        text: I18n.tr("The board has a key already: a new one replaces it, other PCs with the old key no longer get through.")
    }
    Controls.Label {
        Layout.fillWidth: true
        wrapMode: Text.WordWrap
        color: Style.muted
        font.pointSize: Style.fontSize * 0.9
        text: I18n.tr("Password and key go through your network unencrypted, as with the web interface in the browser: do this in your own network only.")
    }
    Controls.Label {
        Layout.fillWidth: true
        visible: AppController.pairMessage !== ""
        wrapMode: Text.WordWrap
        color: Style.stateColor(AppController.pairLevel)
        text: AppController.pairMessage
    }
}
