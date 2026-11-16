import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// The list of triggers, saved automatically: every complete one (with
// keys); one still without keys waits in the list until it has some.
// "Record" takes over a combination by pressing it.
// The rows come from a ListModel changed in place: editing a field does not
// rebuild the rows (focus stays where it is).
ColumnLayout {
    id: editor
    spacing: 10

    property int recordingIndex: -1
    property string error: ""

    readonly property var kinds: [
        { value: "combo", text: "Combination" },
        { value: "hold", text: "Hold" },
        { value: "double-tap", text: "Double tap" }
    ]
    readonly property var actionValues: ["toggle", "on", "off", "pause", "release-all"]
    readonly property var actionTexts: ["Toggle", "Switch on", "Switch off", "Pause", "Release all"]

    // kind, keys, action, window_ms, and `raw`: the whole entry as read
    // (settings a newer version wrote are kept).
    ListModel {
        id: triggerModel
    }

    function reload() {
        triggerModel.clear();
        for (const t of JSON.parse(AppController.triggersJson))
            triggerModel.append({ kind: t.kind, keys: t.keys, action: t.action, window_ms: t.window_ms,
                                  raw: JSON.stringify(t) });
    }

    function entries() {
        const all = [];
        for (let i = 0; i < triggerModel.count; i++) {
            const t = triggerModel.get(i);
            all.push(Object.assign(JSON.parse(t.raw), { kind: t.kind, keys: t.keys, action: t.action,
                                                        window_ms: t.window_ms }));
        }
        return all;
    }

    function setField(index, key, value) {
        triggerModel.setProperty(index, key, value);
        saveSoon.restart();
    }

    function stopRecording() {
        if (recordingIndex >= 0)
            AppController.recordTrigger(false);
        recordingIndex = -1;
    }

    /// Store every complete trigger; false (and `error`) if one has no keys
    /// yet (it stays here, unsaved) or the list is invalid.
    function save() {
        saveSoon.stop();
        const all = entries();
        const complete = all.filter(t => t.keys.trim() !== "");
        error = AppController.setTriggers(JSON.stringify(complete));
        if (error === "" && complete.length < all.length)
            error = "A trigger without keys is not saved yet: enter or record its keys.";
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
        model: triggerModel
        delegate: Rectangle {
            id: row
            required property int index
            required property string kind
            required property string keys
            required property string action
            readonly property bool recording: editor.recordingIndex === index

            Layout.fillWidth: true
            implicitHeight: rowCol.implicitHeight + 24
            radius: Style.radius
            color: Style.surfaceAlt
            border.color: recording ? Style.accent : keys.trim() === "" ? Style.tint(Style.warn, 0.6) : Style.border

            ColumnLayout {
                id: rowCol
                anchors.fill: parent
                anchors.margins: 12
                spacing: 10

                RowLayout {
                    Layout.fillWidth: true
                    Segmented {
                        value: row.kind
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
                            // Keep a running recording on its row: stop it if
                            // that row goes, follow it if an earlier one goes.
                            if (editor.recordingIndex === row.index) {
                                AppController.recordTrigger(false);
                                editor.recordingIndex = -1;
                            } else if (editor.recordingIndex > row.index) {
                                editor.recordingIndex -= 1;
                            }
                            triggerModel.remove(row.index);
                            saveSoon.restart();
                        }
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 8
                    Controls.TextField {
                        Layout.fillWidth: true
                        visible: !row.recording
                        text: row.keys
                        placeholderText: "e.g. Ctrl+Alt+P or Mouse5"
                        onEditingFinished: if (text !== row.keys) editor.setField(row.index, "keys", text)
                    }
                    // While recording: the instruction in full (a placeholder
                    // would be cut off in a narrow field).
                    Controls.Label {
                        Layout.fillWidth: true
                        visible: row.recording
                        text: "Press the keys or the mouse button, then release …"
                        color: Style.accent
                        wrapMode: Text.WordWrap
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
                        text: row.kind === "hold" ? "The bridge is on while these keys are held." : "Action"
                        color: Style.muted
                    }
                    Controls.ComboBox {
                        visible: row.kind !== "hold"
                        model: editor.actionTexts
                        currentIndex: editor.actionValues.indexOf(row.action)
                        onActivated: idx => editor.setField(row.index, "action", editor.actionValues[idx])
                    }
                }
                Controls.Label {
                    visible: row.kind === "hold"
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    color: Style.muted
                    font.pointSize: Style.fontSize * 0.9
                    text: "The desktop never sees this key let go and keeps it pressed afterwards. "
                          + "Use a key on a device that is not selected (e.g. a second keyboard), "
                          + "or one the desktop does not need."
                }
            }
        }
    }

    AppButton {
        text: "Add trigger"
        icon.name: "plus"
        icon.color: Style.text
        onClicked: triggerModel.append({ kind: "combo", keys: "", action: "toggle", window_ms: 300, raw: "{}" })
    }

    Banner {
        kind: "warn"
        visible: editor.error !== ""
        text: editor.error
    }
}
