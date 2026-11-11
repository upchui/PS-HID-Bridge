import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

AppPage {
    id: page
    title: "Status"

    Component.onCompleted: AppController.setLatencyView(true)
    Component.onDestruction: AppController.setLatencyView(false)

    readonly property var triggers: JSON.parse(AppController.triggersJson)
    readonly property string level: AppController.active ? "ok" : AppController.connected ? "accent" : ""

    // "Shift + A · mouse: left" -> ["Shift", "A", "mouse: left"]
    function keyChips(text) {
        if (text === "")
            return [];
        return text.split(" · ").map(p => p.split(" + ")).reduce((a, b) => a.concat(b), []);
    }

    function triggerChips() {
        const actions = { "toggle": "toggle", "on": "on", "off": "off", "pause": "pause", "release-all": "release all" };
        let chips = triggers.filter(t => t.keys !== "").map(t => ({
            keys: t.keys,
            what: t.kind === "hold" ? "hold" : (t.kind === "double-tap" ? "double tap: " : "") + actions[t.action]
        }));
        if (AppController.windowRuleEnabled && AppController.windowRule !== "")
            chips.push({ keys: AppController.windowRule, what: "window active" });
        return chips;
    }

    // ------------------------------------------------------------ banners
    Banner {
        kind: "error"
        visible: AppController.lastError !== ""
        text: AppController.lastError
        actions: AppButton {
            text: "Dismiss"
            flat: true
            onClicked: AppController.lastError = ""
        }
    }
    Banner {
        kind: "ok"
        visible: AppController.appUpdateAvailable || AppController.boardUpdateAvailable
                 || AppController.appUpdated || AppController.updateBusy
        text: AppController.updateStatus !== "" ? AppController.updateStatus
              : "Version " + AppController.latestVersion + " is available"
                + (AppController.appUpdateAvailable && AppController.boardUpdateAvailable ? " for app and board."
                   : AppController.appUpdateAvailable ? " for the app." : " for the board.")
        actions: [
            AppButton {
                text: "Update app"
                visible: AppController.appUpdateAvailable && AppController.canUpdateApp
                enabled: !AppController.updateBusy
                onClicked: AppController.updateApp()
            },
            AppButton {
                text: "Restart now"
                visible: AppController.appUpdated
                onClicked: AppController.restartApp()
            },
            AppButton {
                text: "Update board"
                visible: AppController.boardUpdateAvailable
                enabled: !AppController.updateBusy && !AppController.active
                onClicked: AppController.updateBoard()
            }
        ]
    }

    // ------------------------------------------------------------ hero
    Rectangle {
        Layout.fillWidth: true
        implicitHeight: hero.implicitHeight + 2 * Style.pad + 4
        radius: Style.radius + 2
        color: page.level === "" ? Style.surface : Style.tint(Style.stateColor(page.level), Style.dark ? 0.14 : 0.09)
        border.color: page.level === "" ? Style.border : Style.tint(Style.stateColor(page.level), 0.5)

        ColumnLayout {
            id: hero
            anchors.fill: parent
            anchors.margins: Style.pad + 2
            spacing: 10

            RowLayout {
                spacing: 14
                Rectangle {
                    implicitWidth: 48
                    implicitHeight: 48
                    radius: 24
                    color: page.level === "" ? Style.surfaceAlt : Style.stateColor(page.level)
                    AppIcon {
                        anchors.centerIn: parent
                        name: "power"
                        color: page.level === "" ? Style.muted : Style.accentText
                        implicitWidth: 26
                        implicitHeight: 26
                    }
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 2
                    Controls.Label {
                        text: AppController.statusText
                        color: Style.text
                        font.pointSize: Style.fontSize * 1.6
                        font.weight: Font.DemiBold
                    }
                    Controls.Label {
                        Layout.fillWidth: true
                        color: Style.muted
                        wrapMode: Text.WordWrap
                        text: AppController.active ? "Mouse and keyboard go to the PlayStation."
                              : AppController.connected ? "Mouse and keyboard belong to the desktop."
                              : AppController.boardAddress === "" ? "Set up the board first."
                              : "Waiting for the board …"
                    }
                }
                AppSwitch {
                    enabled: AppController.connected
                    checked: AppController.active || AppController.pending
                    onToggled: AppController.setOn(checked)
                    big: true
                    Accessible.name: "Bridge"
                }
            }

            Flow {
                Layout.fillWidth: true
                visible: AppController.active
                spacing: 6
                Controls.Label {
                    text: "To the PlayStation:"
                    color: Style.muted
                    height: 26
                    verticalAlignment: Text.AlignVCenter
                }
                Repeater {
                    model: page.keyChips(AppController.liveText)
                    delegate: Rectangle {
                        id: keyChip
                        required property string modelData
                        implicitWidth: keyLabel.implicitWidth + 16
                        implicitHeight: 26
                        radius: 5
                        color: Style.surface
                        border.color: Style.border
                        Controls.Label {
                            id: keyLabel
                            anchors.centerIn: parent
                            text: keyChip.modelData
                            color: Style.text
                            font.weight: Font.DemiBold
                        }
                    }
                }
                Controls.Label {
                    visible: AppController.liveText === ""
                    text: "no key"
                    color: Style.muted
                    height: 26
                    verticalAlignment: Text.AlignVCenter
                }
            }
        }
    }

    // ------------------------------------------------------------ tiles
    GridLayout {
        Layout.fillWidth: true
        columns: page.width > 760 ? 2 : 1
        columnSpacing: Style.gap
        rowSpacing: Style.gap

        StatTile {
            icon: "board"
            label: "Board"
            level: AppController.boardAddress === "" ? "warn" : AppController.connected ? "ok" : "bad"
            value: AppController.boardAddress === "" ? "Not set up"
                   : AppController.connected ? "Connected" : "Not reachable"
            detail: AppController.boardAddress === "" ? "See \"Board\"."
                    : AppController.boardAddress + (AppController.boardFirmware !== "" ? " · firmware " + AppController.boardFirmware : "")
        }
        StatTile {
            icon: "gamepad"
            label: "PlayStation"
            level: AppController.boardUsb === "PlayStation connected" ? "ok"
                   : AppController.boardUsb === "" ? "" : "warn"
            value: AppController.boardUsb === "" ? "Unknown"
                   : AppController.boardUsb === "PlayStation connected" ? "Connected"
                   : AppController.boardUsb === "PlayStation in rest mode" ? "Rest mode" : "Not connected"
            detail: "USB connection of the board"
        }
        StatTile {
            icon: "latency"
            label: "Round trip to the board"
            value: AppController.rttMs >= 0 ? AppController.rttP50.toFixed(2) + " ms" : "–"
            detail: AppController.rttMs >= 0 ? "median · 95 % below " + AppController.rttP95.toFixed(2) + " ms" : "measured while connected"

            Canvas {
                id: spark
                Layout.fillWidth: true
                Layout.topMargin: 4
                implicitHeight: 36
                visible: AppController.connected
                readonly property var values: JSON.parse(AppController.rttHistoryJson)
                readonly property color line: Style.accent
                onValuesChanged: requestPaint()
                onWidthChanged: requestPaint()
                onLineChanged: requestPaint()
                onPaint: {
                    const ctx = getContext("2d");
                    ctx.reset();
                    const v = values;
                    if (v.length < 2)
                        return;
                    const max = Math.max(1, Math.max.apply(null, v) * 1.2);
                    const x = i => (width - 2) * i / (v.length - 1) + 1;
                    const y = val => height - 2 - (height - 4) * val / max;
                    ctx.strokeStyle = line;
                    ctx.lineWidth = 1.5;
                    ctx.beginPath();
                    ctx.moveTo(x(0), y(v[0]));
                    for (let i = 1; i < v.length; i++)
                        ctx.lineTo(x(i), y(v[i]));
                    ctx.stroke();
                }
            }
        }
        StatTile {
            icon: "keyboard"
            label: "Input → network"
            value: AppController.inputP50 >= 0 ? AppController.inputP50.toFixed(2) + " ms" : "–"
            detail: AppController.inputP50 >= 0
                    ? "median · max " + AppController.inputMax.toFixed(2) + " ms · " + AppController.packetsPerSecond + " packets/s"
                    : "measured while the bridge is on"

            RowLayout {
                spacing: 6
                Pill {
                    visible: AppController.connected
                    level: AppController.realtime ? "ok" : "warn"
                    text: AppController.realtime ? "real-time priority" : "normal priority"
                }
                HelpHint {
                    visible: AppController.connected && !AppController.realtime
                    text: "Real-time priority is not available (rtkit or a realtime group is missing, see README). Input still works, with more variance under load."
                }
                Pill {
                    level: AppController.devicesBlocked > 0 ? "warn" : AppController.devicesOpen > 0 ? "accent" : ""
                    text: AppController.devicesBlocked > 0 ? AppController.devicesBlocked + " without access"
                          : AppController.devicesOpen + " device interface(s)"
                }
            }
        }
    }

    // ------------------------------------------------------------ actions
    Card {
        title: "Controls"
        icon: "bolt"

        Flow {
            Layout.fillWidth: true
            spacing: 8
            AppButton {
                text: "Release all keys"
                icon.name: "release"
                icon.color: enabled ? Style.text : Style.disabled
                enabled: AppController.connected
                onClicked: AppController.releaseAll()
            }
            AppButton {
                text: AppController.paused ? "Resume" : "Pause"
                icon.name: (AppController.paused ? "play" : "pause")
                icon.color: enabled ? Style.text : Style.disabled
                enabled: AppController.connected
                onClicked: AppController.pause()
            }
            AppButton {
                text: "Reconnect"
                icon.name: "reconnect"
                icon.color: Style.text
                onClicked: AppController.reconnect()
            }
        }

        Flow {
            Layout.fillWidth: true
            spacing: 6
            Repeater {
                model: page.triggerChips().concat([{ keys: "Ctrl+Alt+Shift+Esc", what: "emergency stop" }])
                delegate: Rectangle {
                    id: chip
                    required property var modelData
                    implicitWidth: chipRow.implicitWidth + 16
                    implicitHeight: chipRow.implicitHeight + 8
                    radius: 5
                    color: Style.surfaceAlt
                    border.color: Style.border
                    RowLayout {
                        id: chipRow
                        anchors.centerIn: parent
                        spacing: 6
                        Controls.Label {
                            text: chip.modelData.keys
                            color: Style.text
                            font.weight: Font.DemiBold
                        }
                        Controls.Label {
                            text: chip.modelData.what
                            color: Style.muted
                        }
                    }
                }
            }
        }
    }
}
