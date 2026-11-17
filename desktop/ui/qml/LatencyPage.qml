import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// The board's own measurements of the input path (as under "Latency of the
// input path" in its web interface), its status and its clock speeds. Read
// with the key (signed requests), every 2 seconds while the page is open.
AppPage {
    id: page
    title: I18n.tr("Latency")
    subtitle: I18n.tr("Measured by the board itself, from the network chip's interrupt to the report the PlayStation fetches; at the end, what this PC adds.")

    readonly property var info: {
        try {
            return AppController.boardInfoJson === "" ? null : JSON.parse(AppController.boardInfoJson);
        } catch (e) {
            return null;
        }
    }
    readonly property var d: info ? info.diag : ({})
    readonly property var st: info ? info.status : ({})
    readonly property var clk: info ? info.clock : ({})
    readonly property bool have: info !== null
    // The board did not answer the last request: what is shown is its last
    // answer (dimmed, see the banner).
    readonly property bool stale: have && AppController.boardInfoError !== ""
    // The clock choice before "Apply" ("" = what the board runs). It ends
    // when the board reports other speeds (after the restart, or a fall
    // back to 150 MHz), so the controls always come back to the truth.
    property string pendingCpu: ""
    property string pendingSpi: ""
    readonly property string boardClock: clk.cpu_mhz + "/" + clk.spi_fast
    onBoardClockChanged: { pendingCpu = ""; pendingSpi = ""; }

    function stage(key, label) {
        return { label: label, n: d[key + "_n"] || 0, min: d[key + "_min"] || 0, p50: d[key + "_p50"] || 0,
                 p99: d[key + "_p99"] || 0, max: d[key + "_max"] || 0 };
    }
    function us(v) {
        return v === undefined ? "–" : Style.us(v);
    }
    function pct(a, b) {
        return Style.percent(a, b);
    }
    function n(v) {
        return v === undefined ? "–" : Style.count(v);
    }
    function uptime(msecs) {
        let s = Math.floor(msecs / 1000);
        const days = Math.floor(s / 86400); s %= 86400;
        const h = Math.floor(s / 3600); s %= 3600;
        const m = Math.floor(s / 60); s %= 60;
        return days ? `${days} d ${h} h` : h ? `${h} h ${m} min` : `${m} min ${s} s`;
    }

    // Only while someone looks: not while the window is minimised, and with
    // the bridge on not while it is behind the game either -- the board's
    // web server would only add work next to the input path.
    Timer {
        interval: 2000
        repeat: true
        running: page.Window.window !== null && page.Window.window.visibility !== Window.Minimized
                 && page.Window.window.visibility !== Window.Hidden
                 && (page.Window.active || !AppController.active)
        triggeredOnStart: true
        onTriggered: AppController.refreshBoardInfo()
    }

    actions: [
        AppButton {
            text: I18n.tr("Start over")
            icon.name: "reconnect"
            flat: true
            enabled: page.have && !page.stale
            onClicked: AppController.resetBoardDiag()
        }
    ]

    Banner {
        kind: "warn"
        visible: AppController.boardInfoError !== ""
        text: page.have ? I18n.trf("The board did not answer: %1\nThe values below are its last answer.", AppController.boardInfoError)
              : I18n.trf("The board did not answer: %1", AppController.boardInfoError)
    }
    Banner {
        kind: "info"
        visible: !page.have && AppController.boardInfoError === ""
        text: AppController.boardAddress === "" ? I18n.tr("Set up the board first.") : I18n.tr("Asking the board …")
    }

    // ---- the headline: packet -> report ready
    Card {
        visible: page.have
        opacity: page.stale ? 0.5 : 1
        title: I18n.tr("Packet → report ready")
        icon: "latency"
        help: I18n.tr("From the network chip's interrupt for a packet until its input is in the state the next USB report carries. Median: half of the packets were faster.")
        // The lines go under the big number when the card is narrow.
        GridLayout {
            Layout.fillWidth: true
            columns: width > 560 ? 2 : 1
            columnSpacing: 24
            rowSpacing: 4
            Controls.Label {
                text: page.d.ready_n ? page.us(page.d.ready_p50) : "–"
                font.pointSize: Style.fontSize * 3.2
                font.weight: Font.DemiBold
                color: Style.text
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2
                Controls.Label {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: page.d.ready_n ? I18n.trf("99\u00a0% below %1 · slowest %2", page.us(page.d.ready_p99), page.us(page.d.ready_max))
                          : I18n.tr("no input yet")
                    color: Style.muted
                }
                Controls.Label {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    visible: text !== ""
                    text: page.d.ready1_n
                        ? I18n.trf("not queued behind another packet: median %1, 99\u00a0% below %2",
                                   page.us(page.d.ready1_p50), page.us(page.d.ready1_p99))
                        : ""
                    color: Style.muted
                }
                Controls.Label {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    visible: text !== ""
                    text: page.d.ready_n ? I18n.trf("%1 since the start or the last reset",
                                                    Style.plural(page.d.ready_n, I18n.tr("packet"), I18n.tr("packets")))
                          : ""
                    color: Style.muted
                }
            }
        }
    }

    GridLayout {
        visible: page.have
        opacity: page.stale ? 0.5 : 1
        Layout.fillWidth: true
        columns: page.width > 760 ? 4 : 2
        columnSpacing: Style.gap
        rowSpacing: Style.gap
        StatTile {
            icon: "gamepad"
            label: I18n.tr("PlayStation fetch")
            value: page.d.usb_n ? page.us(page.d.usb_p50) : "–"
            detail: page.d.usb_n ? I18n.trf("median · 99\u00a0% below %1", page.us(page.d.usb_p99)) : I18n.tr("no reports yet")
        }
        StatTile {
            icon: "network"
            label: I18n.tr("Bus wait")
            value: page.d.bus_n ? page.us(page.d.bus_max) : "–"
            detail: I18n.tr("longest wait for the SPI bus")
        }
        StatTile {
            icon: "target"
            label: I18n.tr("Missed polls")
            level: (page.d.mouse_misses || 0) > 0 ? "warn" : ""
            value: page.d.mouse_fetches ? page.n(page.d.mouse_misses) : "–"
            detail: page.d.mouse_fetches ? I18n.trf("of %1 (%2)", Style.plural(page.d.mouse_fetches, I18n.tr("report"), I18n.tr("reports")),
                                                    page.pct(page.d.mouse_misses, page.d.mouse_fetches))
                    : ""
        }
        StatTile {
            icon: "key"
            label: I18n.tr("Decryption")
            value: page.d.crypto_avg_ns ? (page.d.crypto_avg_ns / 1000).toFixed(1) + "\u00a0µs" : "–"
            detail: page.d.crypto_fast ? I18n.tr("optimised for this CPU, both cores") : I18n.tr("portable code")
        }
    }

    // ---- where the time goes
    Card {
        visible: page.have && (page.d.ready_n || 0) > 0
        opacity: page.stale ? 0.5 : 1
        title: I18n.tr("Where the time goes")
        subtitle: I18n.tr("A typical packet: the median of every stage.")
        icon: "status"
        help: I18n.tr("Noticed: interrupt until the main loop sees it. Network chip: waiting for the SPI bus. Reading: the datagram over SPI, while both cores already compute its keystream. Checking: tag check, decryption and applying the input.")
        StageBar {
            Layout.fillWidth: true
            parts: [
                { name: I18n.tr("noticed"), value: page.d.detect_p50 || 0 },
                { name: I18n.tr("network chip free"), value: page.d.bus_p50 || 0 },
                { name: I18n.tr("reading + keystream"), value: page.d.spi_p50 || 0 },
                { name: I18n.tr("checking + applying"), value: page.d.handle_p50 || 0 }
            ]
        }
    }

    // ---- spread per stage
    Card {
        visible: page.have && (page.d.ready_n || 0) > 0
        opacity: page.stale ? 0.5 : 1
        title: I18n.tr("Spread per stage")
        subtitle: I18n.tr("Bar: fastest to 99 %. Dot: median. Mark: slowest (▸ beyond the scale).")
        icon: "latency"
        RangeChart {
            Layout.fillWidth: true
            rows: [
                page.stage("ready", I18n.tr("Packet → report ready")),
                page.stage("ready1", I18n.tr("… not queued behind another")),
                page.stage("detect", I18n.tr("… noticed by the board")),
                page.stage("bus", I18n.tr("… network chip free")),
                page.stage("spi", I18n.tr("… reading + keystream")),
                page.stage("handle", I18n.tr("… checking + applying"))
            ].filter(r => r.n > 0)
        }
    }

    // ---- mouse reports and the PlayStation
    Card {
        visible: page.have && (page.d.mouse_fetches || 0) > 0
        opacity: page.stale ? 0.5 : 1
        title: I18n.tr("Mouse reports and the PlayStation")
        subtitle: I18n.tr("The PlayStation fetches a report once per millisecond; the board fills it shortly before.")
        icon: "mouse"
        help: I18n.tr("Missed poll: motion was waiting in time, yet its report went out a frame later because the board was busy. Skipped by the PlayStation: the report was ready, the PlayStation fetched it a frame later; the board cannot change that and does not react to it.")
        RangeChart {
            Layout.fillWidth: true
            rows: [
                page.stage("usb", I18n.tr("Packet → fetched")),
                page.stage("armed", I18n.tr("Report filled → fetched"))
            ].filter(r => r.n > 0)
        }
        LeadMeter {
            Layout.fillWidth: true
            Layout.topMargin: 8
            lead: page.d.mouse_lead_us || 0
            floor: page.d.mouse_floor_us || 0
        }
        Controls.Label {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            color: Style.muted
            text: I18n.trf("Missed polls: %1 of %2 (%3) · skipped by the PlayStation: %4 (%5) · "
                           + "motion that waited for a report already filled: %6",
                           page.n(page.d.mouse_misses), page.n(page.d.mouse_fetches),
                           page.pct(page.d.mouse_misses, page.d.mouse_fetches), page.n(page.d.mouse_host_skips),
                           page.pct(page.d.mouse_host_skips, page.d.mouse_fetches), page.pct(page.d.mouse_waits, page.d.mouse_inputs))
        }
    }

    // ---- clock speed
    Card {
        visible: page.have
        opacity: page.stale ? 0.5 : 1
        title: I18n.tr("Clock speed")
        subtitle: page.clk.cpu_hz ? I18n.trf("now: processor %1 MHz, network chip %2 MHz", (page.clk.cpu_hz / 1e6).toFixed(0),
                                             (page.clk.spi_hz / 1e6).toFixed(1))
                  : ""
        icon: "bolt"
        help: I18n.tr("Faster clocks shorten the input path by a few microseconds (about 52 → 40 µs from packet to report ready). 200 MHz is beyond the processor's rating: if the board does not run stably, it restarts by itself within 10 s and goes back to 150 MHz. The fast SPI clock is checked at every start.")

        Banner {
            kind: "warn"
            visible: page.clk.cpu_fell_back === true
            text: I18n.tr("200 MHz did not run stably on this board, so it runs at 150 MHz again.")
        }
        Banner {
            kind: "warn"
            visible: page.clk.spi_fast_failed === true
            text: I18n.tr("The fast SPI clock failed its self-test; the network chip runs at the standard clock.")
        }
        // Label and choice side by side, or the choice below its label when
        // the card is too narrow for both.
        GridLayout {
            Layout.fillWidth: true
            columns: width > 480 ? 2 : 1
            columnSpacing: 16
            rowSpacing: 10
            Controls.Label { text: I18n.tr("Processor"); color: Style.text }
            Segmented {
                id: cpuChoice
                value: page.pendingCpu !== "" ? page.pendingCpu : String(page.clk.cpu_mhz || 150)
                model: [{ value: "150", text: I18n.tr("150 MHz (rated)") }, { value: "200", text: I18n.tr("200 MHz (overclocked)") }]
                onActivated: v => page.pendingCpu = v
            }
            Controls.Label { text: I18n.tr("Network chip (SPI)"); color: Style.text }
            Segmented {
                id: spiChoice
                value: page.pendingSpi !== "" ? page.pendingSpi : page.clk.spi_fast ? "fast" : "standard"
                model: [{ value: "standard", text: I18n.tr("Standard (25 MHz)") }, { value: "fast", text: I18n.tr("Fast (33–37.5 MHz)") }]
                onActivated: v => page.pendingSpi = v
            }
        }
        RowLayout {
            spacing: 12
            AppButton {
                text: I18n.tr("Apply")
                highlighted: true
                enabled: !AppController.clockBusy && !AppController.active && !page.stale
                        && (cpuChoice.value !== String(page.clk.cpu_mhz) || (spiChoice.value === "fast") !== page.clk.spi_fast)
                onClicked: AppController.setBoardClock(Number(cpuChoice.value), spiChoice.value === "fast")
            }
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                color: Style.muted
                text: AppController.active ? I18n.tr("Switch the bridge off first: the board restarts.") : AppController.clockMessage
            }
        }
    }

    // ---- this PC: what happens before the packet reaches the network
    Card {
        title: I18n.tr("This PC")
        subtitle: AppController.inputP50 >= 0
                  ? I18n.trf("input → network: median %1 · max %2", Style.us(AppController.inputP50 * 1000),
                             Style.us(AppController.inputMax * 1000))
                  : I18n.tr("input → network is measured while the bridge is on")
        icon: "network"
        help: I18n.tr("Before the board sees a packet, this PC has to notice the input and send it. Two things in the system can add more than the board's whole input path: USB network adapters that hold packets back to bundle them (about 1 ms), and now and then a core that has to wake up from a deep sleep state first (a few hundred µs).")

        GridLayout {
            Layout.fillWidth: true
            columns: 2
            columnSpacing: 16
            rowSpacing: 10
            Controls.Label { text: I18n.tr("Network latency to the board"); color: Style.text }
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                color: Style.muted
                text: AppController.upP50 >= 0
                      ? I18n.trf("PC → board: median %1 · 95\u00a0% below %2 (measured with the board's time stamps)",
                                 Style.us(AppController.upP50 * 1000), Style.us(AppController.upP95 * 1000))
                      : AppController.rttP50 >= 0
                      ? I18n.trf("≈ %1, half the round trip (exact with newer board firmware)", Style.us(AppController.rttP50 * 500))
                      : "–"
            }
            Controls.Label { text: I18n.tr("Round trip"); color: Style.text }
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                color: Style.muted
                text: AppController.rttP50 >= 0
                      ? I18n.trf("median %1 · 95\u00a0% below %2 (there and back, with the board's processing)",
                                 Style.us(AppController.rttP50 * 1000), Style.us(AppController.rttP95 * 1000))
                      : "–"
            }
            Controls.Label { text: I18n.tr("Network adapter"); color: Style.text }
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                color: AppController.adapterMode === "delaying" ? Style.stateColor("warn") : Style.muted
                text: AppController.adapterMode === "delaying"
                      ? I18n.trf("%1 holds every packet back up to about %2 (USB adapter, cdc_ncm)", AppController.adapterIface,
                                 Style.us(AppController.adapterDelayMs * 1000))
                      : AppController.adapterMode === "managed"
                      ? (AppController.adapterAtOnceNow
                         ? I18n.trf("%1 sends at once while the app runs (bundles again when it is closed)", AppController.adapterIface)
                         : I18n.trf("%1 bundles packets (up to %2); the app switches it to “send at once”", AppController.adapterIface,
                                    Style.us(AppController.adapterDelayMs * 1000)))
                      : AppController.adapterMode === "fixed" ? I18n.trf("%1 sends at once (set outside the app)", AppController.adapterIface)
                      : AppController.connected ? I18n.tr("sends packets at once") : "–"
            }
            Controls.Label { text: I18n.tr("Processor sleep"); color: Style.text }
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                color: AppController.cpuWakeState === "not_permitted" ? Style.stateColor("warn") : Style.muted
                text: !AppController.keepCpuAwake ? I18n.tr("deep sleep states allowed (switched off below)")
                      : AppController.cpuWakeState === "held" ? I18n.tr("kept out of deep sleep states now")
                      : AppController.cpuWakeState === "ready" ? I18n.tr("kept out of deep sleep states while the bridge is on")
                      : AppController.cpuWakeState === "not_permitted" ? I18n.tr("may not be kept awake by this user yet")
                      : AppController.cpuWakeState === "unavailable" ? I18n.tr("cannot be kept awake on this system")
                      : "–"
            }
        }
        RowLayout {
            spacing: 12
            AppSwitch {
                checked: AppController.keepCpuAwake
                onToggled: AppController.enableKeepCpuAwake(checked)
                Accessible.name: I18n.tr("Keep the processor awake while the bridge is on")
            }
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                color: Style.muted
                text: I18n.tr("Keep the processor awake while the bridge is on: no input has to wait for a core to wake up from deep sleep (now and then a few hundred µs). The PC uses a few watts more meanwhile.")
            }
        }
        RowLayout {
            id: setupRow
            readonly property bool needed: AppController.adapterMode === "delaying" || AppController.cpuWakeState === "not_permitted"
            visible: needed || AppController.latencyRuleInstalled || AppController.latencySetupMessage !== ""
            spacing: 12
            AppButton {
                text: AppController.latencySetupBusy ? I18n.tr("Setting up …") : I18n.tr("Set up")
                highlighted: true
                visible: setupRow.needed
                enabled: !AppController.latencySetupBusy
                onClicked: AppController.setupLowLatency()
            }
            AppButton {
                text: AppController.latencySetupBusy && !setupRow.needed ? I18n.tr("Removing …") : I18n.tr("Undo")
                visible: AppController.latencyRuleInstalled
                enabled: !AppController.latencySetupBusy
                onClicked: AppController.removeLowLatency()
                Accessible.description: I18n.tr("Removes the low-latency rule; asks for the administrator password")
            }
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                color: Style.muted
                text: AppController.latencySetupMessage !== "" ? AppController.latencySetupMessage
                      : setupRow.needed ? I18n.tr("Asks for the administrator password once and installs a udev rule: this user may then switch the bundling of USB network adapters (the app turns it off while it runs and back on when it is closed, without a password) and, where needed, keep the processor awake.")
                      : I18n.tr("Set up through /etc/udev/rules.d/70-ps-hid-bridge-latency.rules. “Undo” removes it: the network adapter bundles packets again and the app no longer switches it (administrator password).")
            }
        }
    }

    // ---- details, as in the web interface
    Card {
        visible: page.have
        opacity: page.stale ? 0.5 : 1
        title: I18n.tr("Details")
        icon: "board"
        // One grid: the values line up from row to row; below each name
        // when the card is narrow.
        GridLayout {
            id: detailGrid
            Layout.fillWidth: true
            columns: width > 480 ? 2 : 1
            columnSpacing: 16
            rowSpacing: 6
            Repeater {
                model: page.have ? [
                    [I18n.tr("Firmware"), I18n.trf("%1 · slot %2 · up %3", page.st.firmware, page.st.partition, page.uptime(page.st.uptime_ms))],
                    ["PlayStation", page.st.usb_mounted ? (page.st.usb_suspended ? I18n.tr("connected, in rest mode") : I18n.tr("connected"))
                                    : I18n.tr("not connected")],
                    [I18n.tr("Address"), `${page.st.ip} / ${page.st.mask} (${page.st.ip_mode === "static" ? I18n.tr("static") : "DHCP"})`],
                    ["Gateway / DNS", `${page.st.gateway} / ${page.st.dns}`],
                    [I18n.tr("MAC address"), page.st.mac],
                    [I18n.tr("UDP port"), String(page.st.udp_port)],
                    [I18n.tr("Sender filter"), page.st.allowed_sender || I18n.tr("any sender")],
                    [I18n.tr("Packets"), I18n.trf("%1 ok · %2 bad signature · %3 replayed · %4 unknown sender · %5 malformed",
                                                  page.n(page.st.rx_ok), page.n(page.st.rx_auth_fail), page.n(page.st.rx_replay),
                                                  page.n(page.st.rx_filtered), page.n(page.st.rx_malformed))],
                    [I18n.tr("Receive rounds"), I18n.trf("%1 in %2 · %3 SPI transfers",
                                                         Style.plural(page.d.datagrams || 0, I18n.tr("packet"), I18n.tr("packets")),
                                                         Style.plural(page.d.rounds || 0, I18n.tr("round"), I18n.tr("rounds")),
                                                         page.n(page.d.core0_spi))],
                    [I18n.tr("Keystream on core 1"), I18n.trf("%1 · %2 on core 0 alone",
                                                              Style.plural(page.d.crypto_helped || 0, I18n.tr("packet"), I18n.tr("packets")),
                                                              page.n(page.d.crypto_alone))],
                    [I18n.tr("Mouse input that waited"), I18n.trf("%1 of %2 (%3)", page.n(page.d.mouse_waits), page.n(page.d.mouse_inputs),
                                                                  page.pct(page.d.mouse_waits, page.d.mouse_inputs))]
                ].reduce((all, row) => all.concat([{ name: true, text: row[0] }, { name: false, text: row[1] }]), []) : []
                delegate: Controls.Label {
                    required property var modelData
                    Layout.fillWidth: !modelData.name
                    Layout.alignment: Qt.AlignTop
                    Layout.topMargin: modelData.name && detailGrid.columns === 1 ? 4 : 0
                    text: modelData.text === undefined ? "–" : String(modelData.text)
                    color: modelData.name ? Style.muted : Style.text
                    wrapMode: Text.WordWrap
                }
            }
        }
    }
}
