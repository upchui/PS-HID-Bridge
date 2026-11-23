import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// Remapped keys and buttons of one profile: a key, mouse button or the
// like reaches the PlayStation as another one, or not at all. Triggers and
// the emergency stop still see the real keys.
Card {
    id: card

    // The profile (0 = "Standard") and its list [{from, to}].
    property int index: 0
    property var remap: []

    property string error: ""
    // The row and field ("from" or "to") a recording goes to (-1: none).
    property int recordingRow: -1
    property string recordingField: ""

    // Choices besides keys (key names are typed or recorded).
    readonly property var presets: [
        { value: "none", text: I18n.tr("Nothing (switched off)") },
        { value: "BTN_LEFT", text: I18n.tr("Left click") },
        { value: "BTN_RIGHT", text: I18n.tr("Right click") },
        { value: "MouseMiddle", text: I18n.tr("Middle click") },
        { value: "Mouse4", text: I18n.tr("Mouse button 4 (back)") },
        { value: "Mouse5", text: I18n.tr("Mouse button 5 (forward)") }
    ]

    function textOf(value) {
        const p = card.presets.find(x => x.value.toLowerCase() === String(value).toLowerCase());
        return p ? p.text : value;
    }

    function load() {
        rows.clear();
        for (const r of card.remap)
            rows.append({ from: r.from, to: r.to });
    }
    onRemapChanged: if (!saveSoon.running) load()
    onIndexChanged: {
        stopRecording();
        card.error = "";
        load();
    }
    Component.onCompleted: load()
    Component.onDestruction: {
        stopRecording();
        if (saveSoon.running)
            saveSoon.triggered();
    }

    function save() {
        let list = [];
        for (let i = 0; i < rows.count; i++) {
            const r = rows.get(i);
            if (r.from.trim() !== "")
                list.push({ from: r.from.trim(), to: r.to.trim() === "" ? "none" : r.to.trim() });
        }
        card.error = AppController.setProfileRemap(card.index, JSON.stringify(list));
    }

    function setField(row, field, value) {
        rows.setProperty(row, field, value);
        saveSoon.restart();
    }

    function stopRecording() {
        if (card.recordingRow >= 0)
            AppController.recordTrigger(false);
        card.recordingRow = -1;
    }

    function record(row, field) {
        if (card.recordingRow === row && card.recordingField === field) {
            stopRecording();
            return;
        }
        stopRecording();
        card.recordingRow = row;
        card.recordingField = field;
        AppController.recordTrigger(true);
    }

    Connections {
        target: AppController
        function onRecordedChanged() {
            if (card.recordingRow < 0 || AppController.recorded === "")
                return;
            // One key or button: the last part of what was pressed.
            const parts = AppController.recorded.split("+");
            card.setField(card.recordingRow, card.recordingField, parts[parts.length - 1]);
            card.recordingRow = -1;
        }
        function onRecordingChanged() {
            if (!AppController.recording)
                card.recordingRow = -1;
        }
    }

    Timer {
        id: saveSoon
        interval: 400
        onTriggered: card.save()
    }

    ListModel {
        id: rows
    }

    title: I18n.tr("Remapped keys")
    icon: "keyboard"
    subtitle: I18n.tr("A key or mouse button reaches the PlayStation as another one, e.g. a mouse side button as a key the game wants.")
    help: I18n.tr("Key names as on a US keyboard (e.g. F, SPACE, LEFTSHIFT, Ctrl for both Ctrl keys), or record them. The trigger keys and Ctrl+Alt+Shift+Esc keep working with the real keys. A key held while the profile changes keeps its old meaning until it is released.")

    Controls.Label {
        visible: rows.count === 0
        Layout.fillWidth: true
        wrapMode: Text.WordWrap
        color: Style.muted
        text: I18n.tr("No keys remapped: everything reaches the PlayStation as it is.")
    }

    Repeater {
        model: rows
        delegate: Rectangle {
            id: row
            required property int index
            required property string from
            required property string to

            Layout.fillWidth: true
            implicitHeight: rowLayout.implicitHeight + 16
            radius: Style.radius - 2
            color: Style.surfaceAlt
            border.color: Style.border

            GridLayout {
                id: rowLayout
                anchors.fill: parent
                anchors.margins: 8
                columns: width > 560 ? 4 : 1
                columnSpacing: 8
                rowSpacing: 6

                // From
                RowLayout {
                    Layout.fillWidth: true
                    spacing: 6
                    AppTextField {
                        Layout.fillWidth: true
                        visible: !(card.recordingRow === row.index && card.recordingField === "from")
                        text: row.from
                        placeholderText: I18n.tr("Key or button, e.g. Mouse4")
                        onEditingFinished: if (text !== row.from) card.setField(row.index, "from", text)
                        Accessible.name: I18n.tr("Key or button on the PC")
                    }
                    Controls.Label {
                        Layout.fillWidth: true
                        visible: card.recordingRow === row.index && card.recordingField === "from"
                        text: I18n.tr("Press the key or mouse button …")
                        color: Style.accent
                        wrapMode: Text.WordWrap
                    }
                    AppButton {
                        flat: true
                        icon.name: "record"
                        icon.color: Style.bad
                        checked: card.recordingRow === row.index && card.recordingField === "from"
                        Accessible.name: I18n.tr("Record")
                        Controls.ToolTip.visible: hovered
                        Controls.ToolTip.text: I18n.tr("Record")
                        onClicked: card.record(row.index, "from")
                    }
                }

                Controls.Label {
                    text: "→"
                    color: Style.muted
                    Layout.alignment: Qt.AlignHCenter
                }

                // To
                RowLayout {
                    Layout.fillWidth: true
                    spacing: 6
                    AppTextField {
                        id: toField
                        Layout.fillWidth: true
                        visible: !(card.recordingRow === row.index && card.recordingField === "to")
                        text: card.textOf(row.to)
                        placeholderText: I18n.tr("Key, e.g. F")
                        onEditingFinished: {
                            const p = card.presets.find(x => x.text === text);
                            const value = p ? p.value : text;
                            if (value !== row.to)
                                card.setField(row.index, "to", value);
                        }
                        Accessible.name: I18n.tr("Reaches the PlayStation as")
                    }
                    Controls.Label {
                        Layout.fillWidth: true
                        visible: card.recordingRow === row.index && card.recordingField === "to"
                        text: I18n.tr("Press the key or mouse button …")
                        color: Style.accent
                        wrapMode: Text.WordWrap
                    }
                    AppButton {
                        flat: true
                        icon.name: "record"
                        icon.color: Style.bad
                        checked: card.recordingRow === row.index && card.recordingField === "to"
                        Accessible.name: I18n.tr("Record")
                        Controls.ToolTip.visible: hovered
                        Controls.ToolTip.text: I18n.tr("Record")
                        onClicked: card.record(row.index, "to")
                    }
                    AppButton {
                        flat: true
                        text: "…"
                        Accessible.name: I18n.tr("Mouse buttons and off")
                        Controls.ToolTip.visible: hovered
                        Controls.ToolTip.text: I18n.tr("Mouse buttons and off")
                        onClicked: presetMenu.popup()
                        // On a card like AppComboBox's list.
                        Controls.Menu {
                            id: presetMenu
                            padding: 4
                            background: Rectangle {
                                implicitWidth: 200
                                radius: Style.radius
                                color: Style.surface
                                border.color: Style.border
                            }
                            Repeater {
                                model: card.presets
                                delegate: Controls.MenuItem {
                                    id: preset
                                    required property var modelData
                                    text: modelData.text
                                    hoverEnabled: true
                                    leftPadding: 10
                                    rightPadding: 10
                                    topPadding: 6
                                    bottomPadding: 6
                                    onTriggered: card.setField(row.index, "to", modelData.value)
                                    background: Rectangle {
                                        radius: 5
                                        color: preset.pressed ? Style.tint(Style.accent, 0.24)
                                             : preset.highlighted ? Style.tint(Style.accent, 0.14) : "transparent"
                                    }
                                    contentItem: Controls.Label {
                                        text: preset.text
                                        color: Style.text
                                        elide: Text.ElideRight
                                    }
                                }
                            }
                        }
                    }
                }

                AppButton {
                    flat: true
                    icon.name: "trash"
                    icon.color: Style.muted
                    Layout.alignment: Qt.AlignRight
                    Accessible.name: I18n.tr("Remove")
                    Controls.ToolTip.visible: hovered
                    Controls.ToolTip.text: I18n.tr("Remove")
                    onClicked: {
                        if (card.recordingRow === row.index)
                            card.stopRecording();
                        else if (card.recordingRow > row.index)
                            card.recordingRow -= 1;
                        rows.remove(row.index);
                        saveSoon.restart();
                    }
                }
            }
        }
    }

    AppButton {
        text: I18n.tr("Remap a key")
        icon.name: "plus"
        icon.color: Style.text
        onClicked: rows.append({ from: "", to: "" })
    }

    Banner {
        kind: "warn"
        visible: card.error !== ""
        text: card.error
    }
}
