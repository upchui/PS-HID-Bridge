import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// The list of triggers, saved automatically once every entry has keys.
// "Record" takes over a combination by pressing it.
ColumnLayout {
    id: editor
    spacing: 10

    property var items: []
    property int recordingIndex: -1
    property string error: ""

    readonly property var kinds: [
        { value: "combo", text: "Combination" },
        { value: "hold", text: "Hold" },
        { value: "double-tap", text: "Double tap" }
    ]
    readonly property var actionValues: ["toggle", "on", "off", "pause", "release-all"]
    readonly property var actionTexts: ["Toggle", "Switch on", "Switch off", "Pause", "Release all"]

    function reload() {
        items = JSON.parse(AppController.triggersJson);
    }

    function setField(index, key, value) {
        let copy = items.slice();
        copy[index] = Object.assign({}, copy[index]);
        copy[index][key] = value;
        items = copy;
        saveSoon.restart();
    }

    /// Store the triggers; false (and `error`) if one is incomplete or invalid.
    function save() {
        saveSoon.stop();
        if (items.some(t => t.keys.trim() === "")) {
            error = "Enter or record the keys of every trigger.";
            return false;
        }
        error = AppController.setTriggers(JSON.stringify(items));
        return error === "";
    }

    Timer {
        id: saveSoon
        interval: 600
        onTriggered: editor.save()
    }

    Component.onCompleted: reload()
    Component.onDestruction: {
        if (recordingIndex >= 0)
            AppController.recordTrigger(false);
        if (saveSoon.running)
            save();
    }

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
        delegate: Rectangle {
            id: row
            required property var modelData
            required property int index
            readonly property bool recording: editor.recordingIndex === index

            Layout.fillWidth: true
            implicitHeight: rowCol.implicitHeight + 24
            radius: Style.radius
            color: Style.surfaceAlt
            border.color: recording ? Style.accent : modelData.keys.trim() === "" ? Style.tint(Style.warn, 0.6) : Style.border

            ColumnLayout {
                id: rowCol
                anchors.fill: parent
                anchors.margins: 12
                spacing: 10

                RowLayout {
                    Layout.fillWidth: true
                    Segmented {
                        value: row.modelData.kind
                        model: editor.kinds
                        onActivated: v => editor.setField(row.index, "kind", v)
                    }
                    Item { Layout.fillWidth: true }
                    AppButton {
                        flat: true
                        icon.name: "trash"
                        icon.color: Style.muted
                        Accessible.name: "Remove trigger"
                        Controls.ToolTip.visible: hovered
                        Controls.ToolTip.text: "Remove"
                        onClicked: {
                            let copy = editor.items.slice();
                            copy.splice(row.index, 1);
                            editor.items = copy;
                            saveSoon.restart();
                        }
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 8
                    Controls.TextField {
                        Layout.fillWidth: true
                        text: row.recording ? "" : row.modelData.keys
                        placeholderText: row.recording ? "Press the keys or the mouse button, then release …"
                                         : "e.g. Ctrl+Alt+P or Mouse5"
                        enabled: !row.recording
                        onEditingFinished: if (text !== row.modelData.keys) editor.setField(row.index, "keys", text)
                    }
                    AppButton {
                        text: row.recording ? "Stop" : "Record"
                        icon.name: "record"
                        icon.color: row.recording ? Style.accentText : Style.bad
                        highlighted: row.recording
                        onClicked: {
                            if (row.recording) {
                                AppController.recordTrigger(false);
                                editor.recordingIndex = -1;
                            } else {
                                editor.recordingIndex = row.index;
                                AppController.recordTrigger(true);
                            }
                        }
                    }
                }

                RowLayout {
                    spacing: 8
                    Controls.Label {
                        text: row.modelData.kind === "hold" ? "The bridge is on while these keys are held." : "Action"
                        color: Style.muted
                    }
                    Controls.ComboBox {
                        visible: row.modelData.kind !== "hold"
                        model: editor.actionTexts
                        currentIndex: editor.actionValues.indexOf(row.modelData.action)
                        onActivated: idx => editor.setField(row.index, "action", editor.actionValues[idx])
                    }
                }
            }
        }
    }

    AppButton {
        text: "Add trigger"
        icon.name: "plus"
        icon.color: Style.text
        onClicked: editor.items = editor.items.concat([{ "kind": "combo", "keys": "", "action": "toggle", "window_ms": 300 }])
    }

    Banner {
        kind: "warn"
        visible: editor.error !== ""
        text: editor.error
    }
}
