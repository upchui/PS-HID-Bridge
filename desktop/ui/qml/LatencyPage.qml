import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import PSHIDBridge

// The board's own measurements of the input path (as under "Latency of the
// input path" in its web interface), its status and its clock speeds. Read
// with the key (signed requests), every 2 seconds while the page is open.
AppPage {
    id: page
    title: "Latency"
    subtitle: "Measured by the board itself, from the network chip's interrupt to the report the PlayStation fetches."

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

    function stage(key, label) {
        return { label: label, n: d[key + "_n"] || 0, min: d[key + "_min"] || 0, p50: d[key + "_p50"] || 0,
                 p99: d[key + "_p99"] || 0, max: d[key + "_max"] || 0 };
    }
    function us(v) {
        return v === undefined ? "–" : Math.round(v) + " µs";
    }
    function ms(v) {
        return v === undefined ? "–" : (v / 1000).toFixed(2) + " ms";
    }
    function pct(a, b) {
        return b > 0 ? (100 * a / b).toFixed(2) + " %" : "–";
    }
    function uptime(msecs) {
        let s = Math.floor(msecs / 1000);
        const days = Math.floor(s / 86400); s %= 86400;
        const h = Math.floor(s / 3600); s %= 3600;
        const m = Math.floor(s / 60); s %= 60;
        return days ? `${days} d ${h} h` : h ? `${h} h ${m} min` : `${m} min ${s} s`;
    }

    Component.onCompleted: AppController.refreshBoardInfo()
    Timer {
        interval: 2000
        repeat: true
        running: true
        onTriggered: AppController.refreshBoardInfo()
    }

    actions: [
        AppButton {
            text: "Start over"
            icon.name: "reconnect"
            flat: true
            enabled: page.have
            onClicked: AppController.resetBoardDiag()
        }
    ]

    Banner {
        kind: "warn"
        visible: AppController.boardInfoError !== ""
        text: "The board did not answer: " + AppController.boardInfoError
    }
    Banner {
        kind: "info"
        visible: !page.have && AppController.boardInfoError === ""
        text: AppController.boardAddress === "" ? "Set up the board first." : "Asking the board …"
    }

    // ---- the headline: packet -> report ready
    Card {
        visible: page.have
        title: "Packet → report ready"
        icon: "latency"
        help: "From the network chip's interrupt for a packet until its input is in the state the next USB report carries. Median: half of the packets were faster."
        RowLayout {
            spacing: 24
            Controls.Label {
                text: page.d.ready_n ? page.us(page.d.ready_p50) : "–"
                font.pointSize: Style.fontSize * 3.2
                font.weight: Font.DemiBold
                color: Style.text
            }
            ColumnLayout {
                spacing: 2
                Controls.Label {
                    text: page.d.ready_n ? `99 % below ${page.us(page.d.ready_p99)} · slowest ${page.us(page.d.ready_max)}` : "no input yet"
                    color: Style.muted
                }
                Controls.Label {
                    text: page.d.ready1_n
                        ? `not queued behind another packet: median ${page.us(page.d.ready1_p50)}, 99 % below ${page.us(page.d.ready1_p99)}`
                        : ""
                    color: Style.muted
                }
                Controls.Label {
                    text: page.d.ready_n ? `${page.d.ready_n} packets since the start or the last reset` : ""
                    color: Style.muted
                }
            }
        }
    }

    GridLayout {
        visible: page.have
        Layout.fillWidth: true
        columns: page.width > 760 ? 4 : 2
        columnSpacing: Style.gap
        rowSpacing: Style.gap
        StatTile {
            icon: "gamepad"
            label: "PlayStation fetch"
            value: page.d.usb_n ? page.ms(page.d.usb_p50) : "–"
            detail: page.d.usb_n ? `median · 99 % below ${page.ms(page.d.usb_p99)}` : "no reports yet"
        }
        StatTile {
            icon: "network"
            label: "Bus wait"
            value: page.d.bus_n ? page.us(page.d.bus_max) : "–"
            detail: "longest wait for the SPI bus"
        }
        StatTile {
            icon: "target"
            label: "Missed polls"
            level: (page.d.mouse_misses || 0) > 0 ? "warn" : ""
            value: page.d.mouse_fetches ? String(page.d.mouse_misses) : "–"
            detail: page.d.mouse_fetches ? `of ${page.d.mouse_fetches} reports (${page.pct(page.d.mouse_misses, page.d.mouse_fetches)})` : ""
        }
        StatTile {
            icon: "key"
            label: "Decryption"
            value: page.d.crypto_avg_ns ? (page.d.crypto_avg_ns / 1000).toFixed(1) + " µs" : "–"
            detail: page.d.crypto_fast ? "optimised for this CPU, both cores" : "portable code"
        }
    }

    // ---- where the time goes
    Card {
        visible: page.have && (page.d.ready_n || 0) > 0
        title: "Where the time goes"
        subtitle: "A typical packet: the median of every stage."
        icon: "status"
        help: "Noticed: interrupt until the main loop sees it. Network chip: waiting for the SPI bus. Reading: the datagram over SPI, while both cores already compute its keystream. Checking: tag check, decryption and applying the input."
        StageBar {
            Layout.fillWidth: true
            parts: [
                { name: "noticed", value: page.d.detect_p50 || 0 },
                { name: "network chip free", value: page.d.bus_p50 || 0 },
                { name: "reading + keystream", value: page.d.spi_p50 || 0 },
                { name: "checking + applying", value: page.d.handle_p50 || 0 }
            ]
        }
    }

    // ---- spread per stage
    Card {
        visible: page.have && (page.d.ready_n || 0) > 0
        title: "Spread per stage"
        subtitle: "Bar: fastest to 99 %. Dot: median. Mark: slowest (▸ beyond the scale)."
        icon: "latency"
        RangeChart {
            Layout.fillWidth: true
            rows: [
                page.stage("ready", "Packet → report ready"),
                page.stage("ready1", "… not queued behind another"),
                page.stage("detect", "… noticed by the board"),
                page.stage("bus", "… network chip free"),
                page.stage("spi", "… reading + keystream"),
                page.stage("handle", "… checking + applying")
            ].filter(r => r.n > 0)
        }
    }

    // ---- mouse reports and the PlayStation
    Card {
        visible: page.have && (page.d.mouse_fetches || 0) > 0
        title: "Mouse reports and the PlayStation"
        subtitle: "The PlayStation fetches a report once per millisecond; the board fills it shortly before."
        icon: "mouse"
        help: "Missed poll: motion was waiting in time, yet its report went out a frame later because the board was busy. Skipped by the PlayStation: the report was ready, the PlayStation fetched it a frame later; the board cannot change that and does not react to it."
        RangeChart {
            Layout.fillWidth: true
            rows: [
                page.stage("usb", "Packet → fetched"),
                page.stage("armed", "Report filled → fetched")
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
            text: `Missed polls: ${page.d.mouse_misses} of ${page.d.mouse_fetches} (${page.pct(page.d.mouse_misses, page.d.mouse_fetches)}) · `
                + `skipped by the PlayStation: ${page.d.mouse_host_skips} (${page.pct(page.d.mouse_host_skips, page.d.mouse_fetches)}) · `
                + `motion that waited for a report already filled: ${page.pct(page.d.mouse_waits, page.d.mouse_inputs)}`
        }
    }

    // ---- clock speed
    Card {
        visible: page.have
        title: "Clock speed"
        subtitle: page.clk.cpu_hz ? `now: processor ${(page.clk.cpu_hz / 1e6).toFixed(0)} MHz, network chip ${(page.clk.spi_hz / 1e6).toFixed(1)} MHz` : ""
        icon: "bolt"
        help: "Faster clocks shorten the input path by a few microseconds (about 52 → 40 µs from packet to report ready). 200 MHz is beyond the processor's rating: if the board does not run stably, it restarts by itself within 10 s and goes back to 150 MHz. The fast SPI clock is checked at every start."

        Banner {
            kind: "warn"
            visible: page.clk.cpu_fell_back === true
            text: "200 MHz did not run stably on this board, so it runs at 150 MHz again."
        }
        Banner {
            kind: "warn"
            visible: page.clk.spi_fast_failed === true
            text: "The fast SPI clock failed its self-test; the network chip runs at the standard clock."
        }
        GridLayout {
            columns: 2
            columnSpacing: 16
            rowSpacing: 10
            Controls.Label { text: "Processor"; color: Style.text }
            Segmented {
                id: cpuChoice
                value: String(page.clk.cpu_mhz || 150)
                model: [{ value: "150", text: "150 MHz (rated)" }, { value: "200", text: "200 MHz (overclocked)" }]
                onActivated: v => value = v
            }
            Controls.Label { text: "Network chip (SPI)"; color: Style.text }
            Segmented {
                id: spiChoice
                value: page.clk.spi_fast ? "fast" : "standard"
                model: [{ value: "standard", text: "Standard (25 MHz)" }, { value: "fast", text: "Fast (33–37.5 MHz)" }]
                onActivated: v => value = v
            }
        }
        RowLayout {
            spacing: 12
            AppButton {
                text: "Apply"
                highlighted: true
                enabled: !AppController.clockBusy && !AppController.active
                        && (cpuChoice.value !== String(page.clk.cpu_mhz) || (spiChoice.value === "fast") !== page.clk.spi_fast)
                onClicked: AppController.setBoardClock(Number(cpuChoice.value), spiChoice.value === "fast")
            }
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                color: Style.muted
                text: AppController.active ? "Switch the bridge off first: the board restarts." : AppController.clockMessage
            }
        }
    }

    // ---- details, as in the web interface
    Card {
        visible: page.have
        title: "Details"
        icon: "board"
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 6
            Repeater {
                model: page.have ? [
                    ["Firmware", `${page.st.firmware} · slot ${page.st.partition} · up ${page.uptime(page.st.uptime_ms)}`],
                    ["PlayStation", page.st.usb_mounted ? (page.st.usb_suspended ? "connected, in rest mode" : "connected") : "not connected"],
                    ["Address", `${page.st.ip} / ${page.st.mask} (${page.st.ip_mode === "static" ? "static" : "DHCP"})`],
                    ["Gateway / DNS", `${page.st.gateway} / ${page.st.dns}`],
                    ["MAC address", page.st.mac],
                    ["UDP port", String(page.st.udp_port)],
                    ["Sender filter", page.st.allowed_sender || "any sender"],
                    ["Packets", `${page.st.rx_ok} ok · ${page.st.rx_auth_fail} bad signature · ${page.st.rx_replay} replayed · ${page.st.rx_filtered} unknown sender · ${page.st.rx_malformed} malformed`],
                    ["Receive rounds", `${page.d.datagrams} packets in ${page.d.rounds} rounds · ${page.d.core0_spi} SPI transfers`],
                    ["Keystream on core 1", `${page.d.crypto_helped} packets · ${page.d.crypto_alone} on core 0 alone`],
                    ["Mouse input that waited", `${page.d.mouse_waits} of ${page.d.mouse_inputs} (${page.pct(page.d.mouse_waits, page.d.mouse_inputs)})`]
                ] : []
                delegate: RowLayout {
                    required property var modelData
                    Layout.fillWidth: true
                    spacing: 16
                    Controls.Label {
                        Layout.preferredWidth: 190
                        Layout.alignment: Qt.AlignTop
                        text: modelData[0]
                        color: Style.muted
                    }
                    Controls.Label {
                        Layout.fillWidth: true
                        text: modelData[1] === undefined ? "–" : String(modelData[1])
                        color: Style.text
                        wrapMode: Text.WordWrap
                    }
                }
            }
        }
    }
}
