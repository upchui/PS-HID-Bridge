import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// Profiles: mouse settings and remapped keys that apply together. The card
// picks the profile the cards below edit (`index`); "Use" makes it the one
// in effect. A profile with a window applies by itself while that window has
// the focus.
Card {
    id: card

    // The profile being edited (0 = "Standard").
    property int index: 0
    readonly property var profiles: JSON.parse(AppController.profilesJson)
    readonly property var profile: profiles[Math.min(index, profiles.length - 1)] ?? ({})
    readonly property var names: profiles.map(p => p.name)
    // Asking for a name: "" (no), "new" or "rename".
    property string naming: ""
    property string error: ""

    function startNaming(what) {
        card.error = "";
        card.naming = what;
        nameField.text = what === "rename" ? card.profile.name : "";
        nameField.forceActiveFocus();
    }

    function finishNaming() {
        const name = nameField.text.trim();
        const err = card.naming === "new" ? AppController.addProfile(name, card.index)
                                          : AppController.renameProfile(card.index, name);
        card.error = err;
        if (err !== "")
            return;
        if (card.naming === "new")
            card.index = card.profiles.length - 1;
        card.naming = "";
    }

    title: I18n.tr("Profiles")
    icon: "gamepad"
    subtitle: I18n.tr("A profile holds the mouse settings and the remapped keys below, e.g. one per game.")
    help: I18n.tr("Choose a profile by hand here, in the tray menu or with a trigger (Triggers page: \"Profile …\" or \"Next profile\"). A profile with a window applies by itself while that window has the focus, and the profile chosen by hand again when the focus moves on. Adding or removing a profile switches the bridge off.")

    RowLayout {
        Layout.fillWidth: true
        spacing: 8
        AppComboBox {
            id: picker
            Layout.fillWidth: true
            Layout.maximumWidth: 320
            model: card.names
            currentIndex: Math.min(card.index, card.names.length - 1)
            onActivated: i => {
                card.index = i;
                card.naming = "";
                card.error = "";
            }
            Accessible.name: I18n.tr("Profile to edit")
        }
        Pill {
            visible: card.profile.name === AppController.effectiveProfile && AppController.effectiveProfile !== ""
            level: "ok"
            text: I18n.tr("in use")
        }
        Item { Layout.fillWidth: true }
        AppButton {
            text: I18n.tr("Use")
            visible: card.profile.name !== AppController.activeProfile
            onClicked: AppController.chooseProfile(card.index)
            Accessible.description: I18n.tr("Use this profile from now on")
        }
        AppButton {
            flat: true
            icon.name: "plus"
            icon.color: Style.text
            Accessible.name: I18n.tr("New profile")
            Controls.ToolTip.visible: hovered
            Controls.ToolTip.text: I18n.tr("New profile (a copy of this one)")
            onClicked: card.startNaming("new")
        }
        AppButton {
            flat: true
            text: I18n.tr("Rename")
            visible: card.index > 0
            onClicked: card.startNaming("rename")
        }
        AppButton {
            flat: true
            icon.name: "trash"
            icon.color: Style.muted
            visible: card.index > 0
            Accessible.name: I18n.tr("Remove profile")
            Controls.ToolTip.visible: hovered
            Controls.ToolTip.text: I18n.tr("Remove profile")
            onClicked: confirmDelete.visible = true
        }
    }

    // Asking for a name (new profile, or another name).
    RowLayout {
        Layout.fillWidth: true
        visible: card.naming !== ""
        spacing: 8
        AppTextField {
            id: nameField
            Layout.fillWidth: true
            Layout.maximumWidth: 320
            placeholderText: card.naming === "new" ? I18n.tr("Name of the new profile, e.g. a game") : I18n.tr("New name")
            maximumLength: 40
            onAccepted: card.finishNaming()
        }
        AppButton {
            text: card.naming === "new" ? I18n.tr("Add") : I18n.tr("Rename")
            highlighted: true
            enabled: nameField.text.trim() !== ""
            onClicked: card.finishNaming()
        }
        AppButton {
            text: I18n.tr("Cancel")
            flat: true
            onClicked: {
                card.naming = "";
                card.error = "";
            }
        }
    }

    Banner {
        id: confirmDelete
        visible: false
        kind: "warn"
        text: I18n.trf("Remove the profile \"%1\"? Triggers that chose it go to the next profile instead.", card.profile.name ?? "")
        actions: [
            AppButton {
                text: I18n.tr("Remove")
                onClicked: {
                    confirmDelete.visible = false;
                    AppController.deleteProfile(card.index);
                    card.index = 0;
                }
            },
            AppButton {
                text: I18n.tr("Cancel")
                flat: true
                onClicked: confirmDelete.visible = false
            }
        ]
    }

    Banner {
        kind: "error"
        visible: card.error !== ""
        text: card.error
    }

    // The window this profile follows.
    RowLayout {
        Layout.fillWidth: true
        visible: card.index > 0
        spacing: 8
        Controls.Label {
            text: I18n.tr("Window")
            color: Style.text
        }
        AppTextField {
            id: windowField
            Layout.fillWidth: true
            text: card.profile.window ?? ""
            enabled: AppController.windowRuleAvailable
            placeholderText: I18n.tr("Only by hand (or e.g. chiaki-ng: while that window has the focus)")
            onEditingFinished: if (text.trim() !== (card.profile.window ?? "")) AppController.setProfileWindow(card.index, text)
            Accessible.name: I18n.tr("Window this profile follows")
        }
    }
    Controls.Label {
        Layout.fillWidth: true
        visible: card.index > 0 && !AppController.windowRuleAvailable
        wrapMode: Text.WordWrap
        color: Style.muted
        font.pointSize: Style.fontSize * 0.9
        text: I18n.trf("Following a window is not available here: %1.", AppController.windowBackend)
    }
    Controls.Label {
        Layout.fillWidth: true
        visible: card.index === 0
        wrapMode: Text.WordWrap
        color: Style.muted
        font.pointSize: Style.fontSize * 0.9
        text: I18n.tr("\"Standard\" applies whenever no other profile is chosen or asked for by its window.")
    }
}
