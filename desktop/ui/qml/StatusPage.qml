import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

AppPage {
    id: page
    // Show another page (Main.qml): "What's new" opens the changelog.
    signal openPage(string name)
    // Every latency in µs (most are well below a millisecond), from 1 ms on in ms.
    function us(ms) {
        return Style.us(ms * 1000);
    }
    title: I18n.tr("Status")

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
    // The mouse buttons have not changed for 30 s while one is held: if the
    // user is not holding it, the mouse or its wireless receiver lost the
    // release. Counts from the last change of the mouse buttons only: keys
    // pressed meanwhile (playing on with the keyboard) do not restart it.
    property bool mouseHeldLong: false

    Timer {
        id: mouseHeldTimer
        interval: 30000
        onTriggered: page.mouseHeldLong = true
    }
    Connections {
        target: AppController
        function onLiveMouseButtonsChanged() {
            page.mouseHeldLong = false;
            if (AppController.liveMouseButtons !== 0)
                mouseHeldTimer.restart();
            else
                mouseHeldTimer.stop();
        }
    }

    // What is held, as chips: ["Shift", "A", "mouse: left"] (in the app's language).
    readonly property var liveChips: JSON.parse(AppController.liveChipsJson)

    function triggerChips() {
        const actions = { "toggle": I18n.tr("toggle"), "on": I18n.tr("on"), "off": I18n.tr("off"), "pause": I18n.tr("pause"),
                          "release-all": I18n.tr("release all"), "next-profile": I18n.tr("next profile") };
        const actionText = a => a.startsWith("profile:") ? I18n.trf("profile %1", a.substring(8).trim()) : actions[a];
        let chips = triggers.filter(t => t.keys !== "").map(t => ({
            keys: t.keys,
            what: t.kind === "hold" ? I18n.tr("hold")
                  : t.kind === "double-tap" ? I18n.trf("double tap: %1", actionText(t.action)) : actionText(t.action)
        }));
        if (AppController.windowRuleEnabled && AppController.windowRule !== "")
            chips.push({ keys: AppController.windowRule, what: I18n.tr("window active") });
        return chips;
    }

    // ------------------------------------------------------------ banners
    InstallBanner {}
    // Once after an update: what changed.
    Banner {
        kind: "ok"
        visible: AppController.whatsNew !== ""
        text: I18n.trf("Updated to version %1.", AppController.whatsNew)
        actions: [
            AppButton {
                text: I18n.tr("What's new")
                highlighted: true
                onClicked: {
                    AppController.dismissWhatsNew();
                    page.openPage("changelog");
                }
            },
            AppButton {
                text: I18n.tr("Dismiss")
                flat: true
                onClicked: AppController.dismissWhatsNew()
            }
        ]
    }
    Banner {
        // Green for the offer and progress, yellow when something failed.
        kind: AppController.updateStatus !== "" && AppController.updateLevel === "warn" ? "warn" : "ok"
        visible: AppController.appUpdateAvailable || AppController.boardUpdateAvailable
                 || AppController.appUpdated || AppController.updateBusy
        text: AppController.updateStatus !== "" ? AppController.updateStatus
              : AppController.appUpdateAvailable && AppController.boardUpdateAvailable
              ? I18n.trf("Version %1 is available for app and board.", AppController.latestVersion)
              : AppController.appUpdateAvailable ? I18n.trf("Version %1 is available for the app.", AppController.latestVersion)
              : I18n.trf("Version %1 is available for the board.", AppController.latestVersion)
        // One installation at a time: app, board over the network, board over
        // USB; the app restarts by itself after its update ("Restart now"
        // only when that failed).
        actions: [
            AppButton {
                text: I18n.tr("Update app")
                visible: AppController.appUpdateAvailable && AppController.canUpdateApp
                enabled: !AppController.updateBusy && !AppController.usbBusy
                onClicked: AppController.updateApp()
            },
            AppButton {
                text: I18n.tr("Restart now")
                visible: AppController.appUpdated
                enabled: !AppController.updateBusy && !AppController.usbBusy
                onClicked: AppController.restartApp()
            },
            AppButton {
                text: I18n.tr("Update board")
                visible: AppController.boardUpdateAvailable
                enabled: !AppController.updateBusy && !AppController.usbBusy && !AppController.active
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
                        text: AppController.active && AppController.devicesOpen === 0
                              ? I18n.tr("No keyboard or mouse selected: nothing goes to the PlayStation (Input page).")
                              : AppController.active ? I18n.tr("Mouse and keyboard go to the PlayStation.")
                              : AppController.connected ? I18n.tr("Mouse and keyboard belong to the desktop.")
                              : AppController.boardAddress === "" ? I18n.tr("Set up the board first.")
                              : I18n.tr("Waiting for the board …")
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
                    Accessible.name: I18n.tr("Bridge")

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
                    text: I18n.tr("To the PlayStation:")
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
                    text: I18n.tr("no key")
                    color: Style.muted
                    height: 26
                    verticalAlignment: Text.AlignVCenter
                }
                Controls.Label {
                    visible: page.liveChips.length > 0 && AppController.liveDevice !== ""
                    // At most as wide as the row, so a long device name is cut
                    // off with "…" instead of running out of the box.
                    width: Math.min(implicitWidth, parent.width)
                    text: I18n.trf("from %1", AppController.liveDevice)
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
                text: I18n.tr("A mouse button has been held for 30 seconds. If you are not pressing it, the mouse or its wireless receiver lost the release: click that button once, or use \"Release all keys\" (also available as a trigger action).")
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
            label: I18n.tr("Board")
            level: AppController.boardAddress === "" ? "warn" : AppController.connected ? "ok" : "bad"
            value: AppController.boardAddress === "" ? I18n.tr("Not set up")
                   : AppController.connected ? I18n.tr("Connected") : I18n.tr("Not reachable")
            detail: AppController.boardAddress === "" ? I18n.tr("See \"Board\".")
                    : AppController.boardFirmware !== "" ? I18n.trf("%1 · firmware %2", AppController.boardAddress, AppController.boardFirmware)
                    : AppController.boardAddress
        }
        StatTile {
            icon: "gamepad"
            label: "PlayStation"
            level: AppController.boardUsb === "PlayStation connected" ? "ok"
                   : AppController.boardUsb === "" ? "" : "warn"
            value: Style.playStation(AppController.boardUsb)
            detail: I18n.tr("USB connection of the board")
        }
        StatTile {
            id: wayTile
            // The way out (what input takes): measured with the board's time
            // stamps, else estimated as half the round trip (older firmware).
            readonly property bool oneWay: AppController.upP50 >= 0
            readonly property bool roundTrip: AppController.rttP50 >= 0
            icon: "latency"
            label: I18n.tr("Network latency to the board")
            value: oneWay ? page.us(AppController.upP50)
                   : roundTrip ? "≈ " + page.us(AppController.rttP50 / 2) : "–"
            detail: oneWay && roundTrip
                    ? I18n.trf("PC → board, median · 95\u00a0% below %1 · round trip %2", page.us(AppController.upP95),
                               page.us(AppController.rttP50))
                    : oneWay ? I18n.trf("PC → board, median · 95\u00a0% below %1", page.us(AppController.upP95))
                    : roundTrip
                    ? I18n.trf("half the round trip (exact with newer board firmware) · round trip %1, 95\u00a0% below %2",
                               page.us(AppController.rttP50), page.us(AppController.rttP95))
                    : I18n.tr("measured while connected")

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
            label: I18n.tr("Input → network")
            value: AppController.inputP50 >= 0 ? page.us(AppController.inputP50) : "–"
            detail: AppController.inputP50 >= 0
                    ? I18n.trf("median · max %1 · %2", page.us(AppController.inputMax),
                               Style.plural(AppController.packetsPerSecond, I18n.tr("packet/s"), I18n.tr("packets/s")))
                    : I18n.tr("measured while the bridge is on")

            // A Flow, not a row: the pills wrap to the next line instead of
            // running out of the tile.
            Flow {
                Layout.fillWidth: true
                spacing: 6
                Pill {
                    visible: AppController.connected
                    level: AppController.realtime ? "ok" : "warn"
                    text: AppController.realtime ? I18n.tr("real-time priority") : I18n.tr("normal priority")
                    help: AppController.realtime ? ""
                          : I18n.tr("Real-time priority is not available (rtkit or a realtime group is missing, see README). Input still works, with more variance under load.")
                }
                Pill {
                    visible: AppController.cpuWakeState === "held" || AppController.cpuWakeState === "not_permitted"
                    level: AppController.cpuWakeState === "held" ? "ok" : "warn"
                    text: AppController.cpuWakeState === "held" ? I18n.tr("CPU kept awake") : I18n.tr("CPU may sleep deeply")
                    help: AppController.cpuWakeState !== "not_permitted" ? ""
                          : I18n.tr("While the bridge is on, the app keeps the processor out of deep sleep states, so no input has to wait for a core to wake up first (now and then a few hundred microseconds). This user may not do that yet: set it up on the Latency page (asks for the administrator password).")
                }
                Pill {
                    level: AppController.devicesBlocked > 0 ? "warn" : AppController.devicesOpen > 0 ? "accent" : ""
                    text: AppController.devicesBlocked > 0 ? I18n.trf("%1 without access", Style.count(AppController.devicesBlocked))
                          : Style.plural(AppController.devicesOpen, I18n.tr("device interface"), I18n.tr("device interfaces"))
                }
                Pill {
                    visible: AppController.inputResyncs > 0
                    level: "warn"
                    text: I18n.trf("%1× input corrected", Style.count(AppController.inputResyncs))
                    help: I18n.tr("The kernel dropped input events because the app fell behind for a moment (or a release went missing). The app compared with the real key state and released or pressed what was lost, so no key stays stuck. If this count rises often, set up real-time priority (see README).")
                }
            }
        }
        StatTile {
            id: psTile
            readonly property bool complete: page.pcUs >= 0 && page.netUs >= 0 && page.fetchUs >= 0
            Layout.columnSpan: tiles.columns
            icon: "gamepad"
            label: I18n.tr("Input → PlayStation")
            value: complete ? "≈ " + Style.us(page.pcUs + page.netUs + page.fetchUs) : "–"
            detail: complete ? I18n.tr("typical: from the input reaching this PC to the PlayStation fetching the report")
                    : I18n.tr("complete while the bridge is on (and the board answers)")
            // Only with every part known: a bar of some parts would look
            // like the whole. Each part keeps its colour.
            StageBar {
                Layout.fillWidth: true
                Layout.topMargin: 4
                visible: psTile.complete
                parts: [
                    { name: I18n.tr("this PC"), value: page.pcUs, series: 0 },
                    { name: AppController.upP50 >= 0 ? I18n.tr("network") : I18n.tr("network (≈ half the round trip)"), value: page.netUs,
                      series: 1 },
                    { name: I18n.tr("board"), value: page.boardUs, series: 2 },
                    { name: I18n.tr("waiting for the PlayStation's poll"), value: page.waitUs, series: 3 }
                ]
            }
            help: I18n.tr("Medians of each part, added up. This PC: input event to packet sent (last 1000 packets). Network: PC → board, measured with the board's time stamps (last 120 answers: about 24 s while this page is open, it pings five times a second). Board: packet arrived to report ready. Waiting: until the PlayStation fetches the report; it polls once per millisecond. The board's parts count since it started or since \"Start over\" on the Latency page. Not included: the time until this PC fetches the report from the mouse or keyboard (their own USB polling).")
        }
    }

    // ------------------------------------------------------------ actions
    Card {
        title: I18n.tr("Controls")
        icon: "bolt"

        Flow {
            Layout.fillWidth: true
            spacing: 8
            AppButton {
                text: I18n.tr("Release all keys")
                icon.name: "release"
                icon.color: enabled ? Style.text : Style.disabled
                enabled: AppController.connected || AppController.active
                onClicked: AppController.releaseAll()
            }
            AppButton {
                text: AppController.paused ? I18n.tr("Resume") : I18n.tr("Pause")
                icon.name: (AppController.paused ? "play" : "pause")
                icon.color: enabled ? Style.text : Style.disabled
                enabled: AppController.connected || AppController.active
                onClicked: AppController.pause()
            }
            AppButton {
                text: I18n.tr("Reconnect")
                icon.name: "reconnect"
                icon.color: Style.text
                onClicked: AppController.reconnect()
            }
        }

        // The profile in use, and a quick choice (with more than one).
        RowLayout {
            readonly property var names: JSON.parse(AppController.profilesJson).map(p => p.name)
            visible: names.length > 1
            Layout.fillWidth: true
            spacing: 8
            Controls.Label {
                text: I18n.tr("Profile")
                color: Style.text
            }
            Controls.ComboBox {
                model: parent.names
                currentIndex: parent.names.indexOf(AppController.activeProfile)
                onActivated: i => AppController.chooseProfile(i)
                Accessible.name: I18n.tr("Profile")
            }
            Controls.Label {
                Layout.fillWidth: true
                visible: AppController.effectiveProfile !== "" && AppController.effectiveProfile !== AppController.activeProfile
                wrapMode: Text.WordWrap
                color: Style.muted
                text: I18n.trf("Now \"%1\", for the window that has the focus.", AppController.effectiveProfile)
            }
        }

        Flow {
            Layout.fillWidth: true
            spacing: 6
            Repeater {
                model: page.triggerChips().concat([{ keys: "Ctrl+Alt+Shift+Esc", what: I18n.tr("emergency stop") }])
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

    TypeTextCard {}
}
