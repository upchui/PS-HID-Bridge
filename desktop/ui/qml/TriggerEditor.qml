import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import PS5HIDBridge

// Edit the list of triggers; "Aufnehmen" records a combination by pressing it.
ColumnLayout {
    id: editor
    spacing: Kirigami.Units.smallSpacing

    property var items: []
    property int recordingIndex: -1
    property string message: ""
    property bool messageIsError: false

    readonly property var kindValues: ["combo", "hold", "double-tap"]
    readonly property var kindTexts: ["Kombination", "Halten", "Zweimal tippen"]
    readonly property var actionValues: ["toggle", "on", "off", "pause", "release-all"]
    readonly property var actionTexts: ["Umschalten", "Einschalten", "Ausschalten", "Pausieren", "Alles loslassen"]

    function reload() {
        items = JSON.parse(AppController.triggersJson);
    }

    function setField(index, key, value) {
        let copy = items.slice();
        copy[index] = Object.assign({}, copy[index]);
        copy[index][key] = value;
        items = copy;
    }

    function save() {
        const err = AppController.setTriggers(JSON.stringify(items));
        messageIsError = err !== "";
        message = err !== "" ? err : "Gespeichert.";
        return err === "";
    }

    Component.onCompleted: reload()
    Component.onDestruction: if (recordingIndex >= 0) AppController.recordTrigger(false)

    Connections {
        target: AppController
        function onRecordedChanged() {
            if (editor.recordingIndex >= 0 && AppController.recorded !== "") {
                editor.setField(editor.recordingIndex, "keys", AppController.recorded);
                editor.recordingIndex = -1;
            }
        }
    }

    Repeater {
        model: editor.items
        delegate: RowLayout {
            id: row
            required property var modelData
            required property int index
            Layout.fillWidth: true

            Controls.ComboBox {
                model: editor.kindTexts
                currentIndex: editor.kindValues.indexOf(row.modelData.kind)
                onActivated: idx => editor.setField(row.index, "kind", editor.kindValues[idx])
            }
            Controls.TextField {
                Layout.fillWidth: true
                Layout.minimumWidth: Kirigami.Units.gridUnit * 8
                text: row.modelData.keys
                placeholderText: "z. B. Strg+Alt+P oder Maus5"
                enabled: editor.recordingIndex !== row.index
                onEditingFinished: if (text !== row.modelData.keys) editor.setField(row.index, "keys", text)
            }
            Controls.Button {
                text: editor.recordingIndex === row.index ? "Drücken …" : "Aufnehmen"
                highlighted: editor.recordingIndex === row.index
                onClicked: {
                    if (editor.recordingIndex === row.index) {
                        AppController.recordTrigger(false);
                        editor.recordingIndex = -1;
                    } else {
                        editor.recordingIndex = row.index;
                        AppController.recordTrigger(true);
                    }
                }
            }
            Controls.ComboBox {
                model: editor.actionTexts
                visible: row.modelData.kind !== "hold"
                currentIndex: editor.actionValues.indexOf(row.modelData.action)
                onActivated: idx => editor.setField(row.index, "action", editor.actionValues[idx])
            }
            Controls.Button {
                text: "Entfernen"
                onClicked: {
                    let copy = editor.items.slice();
                    copy.splice(row.index, 1);
                    editor.items = copy;
                }
            }
        }
    }

    Controls.Label {
        Layout.fillWidth: true
        wrapMode: Text.WordWrap
        visible: editor.recordingIndex >= 0
        text: "Jetzt die gewünschte Tastenkombination oder Maustaste drücken und wieder loslassen. Die Steuerung ist dabei aus."
    }

    RowLayout {
        Controls.Button {
            text: "Auslöser hinzufügen"
            onClicked: editor.items = editor.items.concat([{ "kind": "combo", "keys": "", "action": "toggle", "window_ms": 300 }])
        }
        Controls.Button {
            text: "Speichern"
            highlighted: true
            onClicked: editor.save()
        }
    }

    Kirigami.InlineMessage {
        Layout.fillWidth: true
        visible: editor.message !== ""
        type: editor.messageIsError ? Kirigami.MessageType.Error : Kirigami.MessageType.Positive
        text: editor.message
    }
}
