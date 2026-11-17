import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

AppPage {
    id: page
    // Every latency in µs (most are well below a millisecond), from 1 ms on in ms.
    function us(ms) {
        return Style.us(ms * 1000);
    }
    title: "Status"

    // Fast pings and the board's statistics only while someone looks: not
    // while the window is minimised, and with the bridge on not while it is
    // behind the game either (on the board they would add work next to the
    // input path: the web server and the SPI bus).
    readonly property bool shown: page.Window.window !== null
                                  && page.Window.window.visibility !== Window.Minimized
                                  && page.Window.window.visibility !== Window.Hidden
                                  && (page.Window.active || !AppController.active)
    onShownChanged: AppController.setLatencyView(shown)
    Component.onCompleted: AppController.setLatencyView(shown)
    Component.onDestruction: AppController.setLatencyView(false)

    // The board's own measurements (signed request, see LatencyPage.qml):
    // its part of the way to the PlayStation.
    // Not while the board is gone: its last answer would look current.
    readonly property var diag: {
        try {
            return AppController.boardInfoJson === "" || !AppController.connected ? ({})
                 : (JSON.parse(AppController.boardInfoJson).diag || ({}));
        } catch (e) {
            return ({});
        }
    }
    Timer {
        interval: 5000
        repeat: true
        running: page.shown && AppController.connected
        triggeredOnStart: true
        onTriggered: AppController.refreshBoardInfo()
    }

    // The way from input to PlayStation in parts (µs, -1 = not known yet):
    // this PC (input -> network), the network (PC -> board; half the round
    // trip with older board firmware), the board (packet -> report ready)
    // and the wait for the PlayStation's fetch (packet -> report fetched,
    // minus the board's part).
    readonly property int pcUs: AppController.inputP50 >= 0 ? Math.round(AppController.inputP50 * 1000) : -1
    readonly property int netUs: AppController.upP50 >= 0 ? Math.round(AppController.upP50 * 1000)
                                 : AppController.rttP50 >= 0 ? Math.round(AppController.rttP50 * 500) : -1
    readonly property int boardUs: diag.ready_n > 0 ? diag.ready_p50 : -1
    readonly property int fetchUs: diag.usb_n > 0 ? diag.usb_p50 : -1
    readonly property int waitUs: fetchUs >= 0 && boardUs >= 0 ? Math.max(0, fetchUs - boardUs) : -1

    readonly property var triggers: JSON.parse(AppController.triggersJson)
    readonly property string level: AppController.active ? "ok" : AppController.connected ? "accent" : ""
    // A mouse button has been held for a while: if the user is not holding
    // it, the mouse or its wireless receiver lost the release.
    property bool mouseHeldLong: false

    Timer {
        id: mouseHeldTimer
        interval: 3000
        onTriggered: page.mouseHeldLong = true
    }
    Connections {
        target: AppController
        function onLiveTextChanged() {
            page.mouseHeldLong = false;
            if (AppController.liveText.indexOf("mouse:") >= 0)
                mouseHeldTimer.restart();
            else
                mouseHeldTimer.stop();
        }
    }

    // "Shift + A · mouse: left" -> ["Shift", "A", "mouse: left"]; nothing
    // held ("no key", or "no key · mouse: left" without a key) is no chip.
    function keyChips(text) {
        if (text === "")
            return [];
        return text.split(" · ").map(p => p.split(" + ")).reduce((a, b) => a.concat(b), [])
                   .filter(c => c !== "no key");
    }
    readonly property var liveChips: keyChips(AppController.liveText)

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
        // Green for the offer and progress, yellow when something failed.
        kind: AppController.updateStatus !== "" && AppController.updateLevel === "warn" ? "warn" : "ok"
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
    Banner {
        kind: "warn"
        visible: AppController.adapterMode === "delaying"
        text: "The network adapter " + AppController.adapterIface + " holds every packet back up to about "
              + page.us(AppController.adapterDelayMs) + " before sending it, to bundle packets (USB adapter, cdc_ncm). "
              + "This comes on top of “Input → network” and shows in the round trip. "
              + "Set up once (administrator password); after that the adapter sends at once while the app runs and bundles again when it is closed, without a password."
              + (AppController.latencySetupMessage !== "" ? "\n" + AppController.latencySetupMessage : "")
        actions: AppButton {
            text: AppController.latencySetupBusy ? "Setting up …" : "Set up"
            enabled: !AppController.latencySetupBusy
            onClicked: AppController.setupLowLatency()
            Accessible.description: "Asks for the administrator password"
        }
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
                    Layout.alignment: Qt.AlignTop
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
                        Layout.fillWidth: true
                        text: AppController.statusText
                        color: Style.text
                        font.pointSize: Style.fontSize * 1.6
                        font.weight: Font.DemiBold
                        wrapMode: Text.WordWrap
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
                    id: bridgeSwitch
                    // Switching off always works: the devices may be
                    // grabbed while the board is gone.
                    enabled: AppController.connected || AppController.active || AppController.pending
                    checked: AppController.active || AppController.pending
                    onToggled: {
                        AppController.setOn(checked);
                        resync.restart();
                    }
                    big: true
                    Accessible.name: "Bridge"

                    // A click flips the switch at once, but the bridge may
                    // stay off (e.g. a device that cannot be grabbed), and
                    // then no property changes to update it: show the real
                    // state again shortly afterwards.
                    Timer {
                        id: resync
                        interval: 400
                        onTriggered: bridgeSwitch.checked = Qt.binding(
                            () => AppController.active || AppController.pending)
                    }
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
                    model: page.liveChips
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
                    visible: page.liveChips.length === 0
                    text: "no key"
                    color: Style.muted
                    height: 26
                    verticalAlignment: Text.AlignVCenter
                }
                Controls.Label {
                    visible: page.liveChips.length > 0 && AppController.liveDevice !== ""
                    // At most as wide as the row, so a long device name is cut
                    // off with "…" instead of running out of the box.
                    width: Math.min(implicitWidth, parent.width)
                    text: "from " + AppController.liveDevice
                    color: Style.muted
                    height: 26
                    verticalAlignment: Text.AlignVCenter
                    elide: Text.ElideRight
                }
            }
            Banner {
                Layout.fillWidth: true
                visible: AppController.active && page.mouseHeldLong
                kind: "warn"
                text: "A mouse button has been held for a few seconds. If you are not pressing it, the mouse or its wireless receiver lost the release: click that button once, or use \"Release all keys\" (also available as a trigger action)."
            }
        }
    }

    // ------------------------------------------------------------ tiles
    GridLayout {
        id: tiles
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
            value: Style.playStation(AppController.boardUsb)
            detail: "USB connection of the board"
        }
        StatTile {
            id: wayTile
            // The way out (what input takes): measured with the board's time
            // stamps, else estimated as half the round trip (older firmware).
            readonly property bool oneWay: AppController.upP50 >= 0
            readonly property bool roundTrip: AppController.rttP50 >= 0
            icon: "latency"
            label: "Network latency to the board"
            value: oneWay ? page.us(AppController.upP50)
                   : roundTrip ? "≈ " + page.us(AppController.rttP50 / 2) : "–"
            detail: oneWay
                    ? "PC → board, median · 95\u00a0% below " + page.us(AppController.upP95)
                      + (roundTrip ? " · round trip " + page.us(AppController.rttP50) : "")
                    : roundTrip
                    ? "half the round trip (exact with newer board firmware) · round trip " + page.us(AppController.rttP50)
                      + ", 95\u00a0% below " + page.us(AppController.rttP95)
                    : "measured while connected"

            Canvas {
                id: spark
                Layout.fillWidth: true
                Layout.topMargin: 4
                implicitHeight: 36
                visible: AppController.connected
                // The way out; without time stamps half of each round trip.
                readonly property var values: wayTile.oneWay ? JSON.parse(AppController.upHistoryJson)
                                              : JSON.parse(AppController.rttHistoryJson).map(v => v / 2)
                // Lowest top of the scale (ms): one-way times are often below 0.1 ms.
                readonly property real floor: 0.1
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
                    const max = Math.max(floor, Math.max.apply(null, v) * 1.2);
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
            value: AppController.inputP50 >= 0 ? page.us(AppController.inputP50) : "–"
            detail: AppController.inputP50 >= 0
                    ? "median · max " + page.us(AppController.inputMax) + " · "
                      + Style.plural(AppController.packetsPerSecond, "packet/s", "packets/s")
                    : "measured while the bridge is on"

            // A Flow, not a row: the pills wrap to the next line instead of
            // running out of the tile.
            Flow {
                Layout.fillWidth: true
                spacing: 6
                Pill {
                    visible: AppController.connected
                    level: AppController.realtime ? "ok" : "warn"
                    text: AppController.realtime ? "real-time priority" : "normal priority"
                    help: AppController.realtime ? ""
                          : "Real-time priority is not available (rtkit or a realtime group is missing, see README). Input still works, with more variance under load."
                }
                Pill {
                    visible: AppController.cpuWakeState === "held" || AppController.cpuWakeState === "not_permitted"
                    level: AppController.cpuWakeState === "held" ? "ok" : "warn"
                    text: AppController.cpuWakeState === "held" ? "CPU kept awake" : "CPU may sleep deeply"
                    help: AppController.cpuWakeState !== "not_permitted" ? ""
                          : "While the bridge is on, the app keeps the processor out of deep sleep states, so no input has to wait for a core to wake up first (now and then a few hundred microseconds). This user may not do that yet: set it up on the Latency page (asks for the administrator password)."
                }
                Pill {
                    level: AppController.devicesBlocked > 0 ? "warn" : AppController.devicesOpen > 0 ? "accent" : ""
                    text: AppController.devicesBlocked > 0 ? Style.count(AppController.devicesBlocked) + " without access"
                          : Style.plural(AppController.devicesOpen, "device interface", "device interfaces")
                }
                Pill {
                    visible: AppController.inputResyncs > 0
                    level: "warn"
                    text: Style.count(AppController.inputResyncs) + "× input corrected"
                    help: "The kernel dropped input events because the app fell behind for a moment (or a release went missing). The app compared with the real key state and released or pressed what was lost, so no key stays stuck. If this count rises often, set up real-time priority (see README)."
                }
            }
        }
        StatTile {
            id: psTile
            readonly property bool complete: page.pcUs >= 0 && page.netUs >= 0 && page.fetchUs >= 0
            Layout.columnSpan: tiles.columns
            icon: "gamepad"
            label: "Input → PlayStation"
            value: complete ? "≈ " + Style.us(page.pcUs + page.netUs + page.fetchUs) : "–"
            detail: complete ? "typical: from the input reaching this PC to the PlayStation fetching the report"
                    : "complete while the bridge is on (and the board answers)"
            // Only with every part known: a bar of some parts would look
            // like the whole. Each part keeps its colour.
            StageBar {
                Layout.fillWidth: true
                Layout.topMargin: 4
                visible: psTile.complete
                parts: [
                    { name: "this PC", value: page.pcUs, series: 0 },
                    { name: AppController.upP50 >= 0 ? "network" : "network (≈ half the round trip)", value: page.netUs, series: 1 },
                    { name: "board", value: page.boardUs, series: 2 },
                    { name: "waiting for the PlayStation's poll", value: page.waitUs, series: 3 }
                ]
            }
            help: "Medians of each part, added up. This PC: input event to packet sent (last 1000 packets). Network: PC → board, measured with the board's time stamps (last 120 answers: about 24 s while this page is open, it pings five times a second). Board: packet arrived to report ready. Waiting: until the PlayStation fetches the report; it polls once per millisecond. The board's parts count since it started or since \"Start over\" on the Latency page. Not included: the time until this PC fetches the report from the mouse or keyboard (their own USB polling)."
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
                enabled: AppController.connected || AppController.active
                onClicked: AppController.releaseAll()
            }
            AppButton {
                text: AppController.paused ? "Resume" : "Pause"
                icon.name: (AppController.paused ? "play" : "pause")
                icon.color: enabled ? Style.text : Style.disabled
                enabled: AppController.connected || AppController.active
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
                    // At most as wide as the card: a long window rule is cut
                    // off with "…" (placed by hand: a layout rounds widths
                    // down and would cut every text by a fraction).
                    width: Math.min(Math.ceil(keysText.implicitWidth + whatText.implicitWidth) + 22, parent.width)
                    implicitHeight: keysText.implicitHeight + 8
                    radius: 5
                    color: Style.surfaceAlt
                    border.color: Style.border
                    Controls.Label {
                        id: keysText
                        x: 8
                        width: Math.min(implicitWidth, chip.width - 22 - whatText.implicitWidth)
                        anchors.verticalCenter: parent.verticalCenter
                        text: chip.modelData.keys
                        color: Style.text
                        font.weight: Font.DemiBold
                        elide: Text.ElideRight
                    }
                    Controls.Label {
                        id: whatText
                        x: keysText.x + keysText.width + 6
                        anchors.verticalCenter: parent.verticalCenter
                        text: chip.modelData.what
                        color: Style.muted
                    }
                }
            }
        }
    }
}
