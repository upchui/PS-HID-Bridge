import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// Keyboards and mice, grouped per physical device (vendor:product); the
// selected ones first. The order is kept while the page is open (a row does
// not jump away when it is switched), and the rows are updated in place.
ColumnLayout {
    id: list
    spacing: 8

    readonly property var parsed: JSON.parse(AppController.devicesJson)
    // Ids in the order shown: selected first, set again when devices come
    // or go and on "Refresh".
    property var order: []
    readonly property var groups: {
        const at = id => {
            const i = order.indexOf(id);
            return i < 0 ? order.length : i;
        };
        return parsed.slice().sort((a, b) => at(a.id) - at(b.id));
    }
    readonly property int selectedBlocked: groups.filter(g => g.selected && !g.accessible).length
    readonly property int selectedCount: groups.filter(g => g.selected).length
    property bool identify: false

    function sortNow() {
        order = parsed.filter(d => d.selected).concat(parsed.filter(d => !d.selected)).map(d => d.id);
    }
    onParsedChanged: {
        const ids = parsed.map(d => d.id);
        if (ids.length !== order.length || ids.some(id => order.indexOf(id) < 0))
            sortNow();
    }
    Component.onCompleted: {
        sortNow();
        sync();
    }

    // The rows, changed in place where the devices are the same.
    ListModel {
        id: rows
    }
    function entry(d) {
        return { devId: d.id, name: d.name, kind: d.kind, nodes: d.nodes, accessible: d.accessible,
                 selected: d.selected, paths: JSON.stringify(d.paths) };
    }
    function sync() {
        const g = groups;
        let same = rows.count === g.length;
        for (let i = 0; same && i < g.length; i++)
            same = rows.get(i).devId === g[i].id;
        if (!same) {
            rows.clear();
            for (const d of g)
                rows.append(entry(d));
            return;
        }
        for (let i = 0; i < g.length; i++) {
            const e = entry(g[i]);
            for (const k in e) {
                if (rows.get(i)[k] !== e[k])
                    rows.setProperty(i, k, e[k]);
            }
        }
    }
    onGroupsChanged: sync()

    onIdentifyChanged: AppController.setIdentify(identify)
    // Nothing selected any more: nothing to identify (the button would stay
    // on, disabled).
    onSelectedCountChanged: if (selectedCount === 0) identify = false
    Component.onDestruction: if (identify) AppController.setIdentify(false)

    Banner {
        kind: "warn"
        visible: list.selectedBlocked > 0
        text: (list.selectedBlocked === 1 ? "1 selected device cannot" : list.selectedBlocked + " selected devices cannot")
              + " be read yet. The app sets up access once (administrator password)."
        actions: AppButton {
            text: AppController.accessBusy ? "Setting up …" : "Set up access"
            enabled: !AppController.accessBusy
            onClicked: AppController.installAccess()
        }
    }
    Banner {
        kind: "warn"
        visible: AppController.accessMessage !== ""
        text: AppController.accessMessage
    }

    RowLayout {
        Layout.fillWidth: true
        spacing: 8
        Controls.Label {
            Layout.fillWidth: true
            text: list.selectedCount + " of " + list.groups.length + " selected"
            color: Style.muted
            elide: Text.ElideRight
        }
        // In a wrapper: the tooltip also explains a disabled button.
        Item {
            implicitWidth: identifyButton.implicitWidth
            implicitHeight: identifyButton.implicitHeight
            AppButton {
                id: identifyButton
                anchors.fill: parent
                text: list.identify ? "Press a key …" : "Identify"
                icon.name: "target"
                icon.color: !enabled ? Style.disabled : list.identify ? Style.accentText : Style.text
                checkable: true
                checked: list.identify
                highlighted: list.identify
                enabled: list.selectedCount > 0 && list.selectedBlocked === 0
                onToggled: list.identify = checked
            }
            HoverHandler {
                id: identifyHover
            }
            Controls.ToolTip.visible: identifyHover.hovered
            Controls.ToolTip.text: list.selectedCount === 0 ? "Select a device first."
                                   : list.selectedBlocked > 0 ? "Set up access first (above)."
                                   : "Marks the selected device you are typing on."
        }
        AppButton {
            text: "Refresh"
            icon.name: "update"
            icon.color: Style.text
            onClicked: {
                AppController.refreshDevices();
                list.sortNow();
            }
        }
    }

    Controls.Label {
        Layout.fillWidth: true
        visible: list.groups.length === 0
        text: "No keyboard, mouse, touchpad or touchscreen found."
        color: Style.muted
    }

    Repeater {
        model: rows
        delegate: Rectangle {
            id: row
            required property string devId
            required property string name
            required property string kind
            required property int nodes
            required property bool accessible
            required property bool selected
            required property string paths
            readonly property bool identified: AppController.identifiedPath !== ""
                                               && JSON.parse(paths).indexOf(AppController.identifiedPath) >= 0

            Layout.fillWidth: true
            implicitHeight: rowLayout.implicitHeight + 16
            radius: Style.radius
            color: identified ? Style.tint(Style.accent, 0.14)
                 : hover.hovered ? Style.tint(Style.text, 0.04) : Style.surfaceAlt
            border.color: identified ? Style.accent : selected ? Style.tint(Style.accent, 0.4) : "transparent"

            HoverHandler { id: hover }
            TapHandler {
                onTapped: AppController.setDeviceSelected(row.devId, !row.selected)
            }

            RowLayout {
                id: rowLayout
                anchors.fill: parent
                anchors.margins: 8
                anchors.leftMargin: 12
                spacing: 12

                AppIcon {
                    name: row.kind === "keyboard" ? "keyboard" : row.kind === "mouse" ? "mouse"
                          : row.kind === "touchpad" || row.kind === "touchscreen" ? "touchpad" : "input"
                    color: row.selected ? Style.accent : Style.muted
                    implicitWidth: 22
                    implicitHeight: 22
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    Controls.Label {
                        Layout.fillWidth: true
                        text: row.name
                        color: Style.text
                        font.weight: row.selected ? Font.DemiBold : Font.Normal
                        elide: Text.ElideRight
                    }
                    Controls.Label {
                        Layout.fillWidth: true
                        text: row.kind + " · " + row.devId + (row.nodes > 1 ? " · " + row.nodes + " interfaces" : "")
                        color: Style.muted
                        font.pointSize: Style.fontSize * 0.9
                        elide: Text.ElideRight
                    }
                }
                Pill {
                    visible: row.identified
                    level: "accent"
                    text: "input"
                }
                Pill {
                    visible: !row.accessible
                    level: row.selected ? "warn" : ""
                    text: "no access"
                }
                AppSwitch {
                    checked: row.selected
                    onToggled: AppController.setDeviceSelected(row.devId, checked)
                    Accessible.name: row.name
                }
            }
        }
    }
}
