import QtQuick
import PSHIDBridge

// Start document for `make ui-shots` (through PSHB_STARTUP_QML): the window
// at a chosen width and theme, and with --demo-state busy the AppController
// filled with worst-case values -- every pill, warning and banner at once,
// long names, big numbers -- so the screenshots show whatever does not fit.
// The app runs with --screenshot: no service, nothing grabbed, and with no
// board in the settings nothing is contacted. Values set here stay (only a
// running service would overwrite them).
//
//   --demo-width N         window width in px (default 1008)
//   --demo-theme T         light or dark (default light)
//   --demo-state S         busy or plain (plain: as after a fresh start)
//   --demo-lang L          en or de (default en); with de the demo texts
//                          made in Rust (status, messages) are German too
Main {
    function demoArg(name, fallback) {
        const a = Qt.application.arguments;
        const i = a.indexOf(name);
        return i >= 0 && i + 1 < a.length ? a[i + 1] : fallback;
    }

    width: Number(demoArg("--demo-width", "1008"))

    Component.onCompleted: {
        // First: switching the language sets the status text again.
        AppController.setAppLanguage(demoArg("--demo-lang", "en"));
        AppController.appearance = demoArg("--demo-theme", "light");
        if (demoArg("--demo-state", "busy") !== "busy")
            return;
        // A text the app makes in Rust, in the chosen language.
        const de = demoArg("--demo-lang", "en") === "de";
        const t = (en, german) => de ? german : en;
        const C = AppController;
        C.setupDone = true;
        C.connected = true;
        C.active = true;
        C.statusText = t("Please release all keys on the keyboard first",
                         "Bitte zuerst alle Tasten auf der Tastatur loslassen");
        C.boardAddress = "192.168.178.123:45045";
        C.boardName = "Wohnzimmer PS5 – Gästezimmer";
        C.boardMac = "02:68:9d:96:c3:3d";
        C.renameLevel = "ok";
        C.renameMessage = t("Renamed. In your router and the browser the board is now called wohnzimmer-ps5-gaestezimmer.",
                            "Umbenannt. Im Router und im Browser heißt das Board jetzt wohnzimmer-ps5-gaestezimmer.");
        C.foundBoardsJson = JSON.stringify([
            { name: "Wohnzimmer PS5 – Gästezimmer", address: "192.168.178.123:45045", ip: "192.168.178.123",
              mac: "02:68:9d:96:c3:3d", firmware: "1.0.123", key_set: true, current: true },
            { name: "ps-hid-bridge-a1b2c3", address: "192.168.178.234:45099", ip: "192.168.178.234",
              mac: "02:11:22:a1:b2:c3", firmware: "1.0.99", key_set: false, current: false }
        ]);
        C.boardFirmware = "1.0.123";
        C.searching = true;
        C.boardUsb = "PlayStation in rest mode";
        C.pskSet = true;
        C.rttMs = 0.912;
        C.rttP50 = 0.516;
        C.rttP95 = 1.874;
        C.upP50 = 0.185;
        C.upP95 = 0.733;
        const hist = [];
        for (let i = 0; i < 120; i++)
            hist.push(0.4 + 0.1 * Math.sin(i / 3) + (i % 17 === 0 ? 1.2 : 0));
        C.rttHistoryJson = JSON.stringify(hist);
        C.upHistoryJson = JSON.stringify(hist.map(v => v / 2));
        C.inputP50 = 0.043;
        C.inputP95 = 0.12;
        C.inputMax = 16.234;
        C.packetsPerSecond = 1;
        C.realtime = false;
        C.cpuWakeState = "not_permitted";
        C.devicesOpen = 1;
        C.devicesBlocked = 0;
        C.inputResyncs = 12345;
        C.liveText = t("Ctrl + Shift + Alt + AltGr + right Super + W + A + D · mouse: left, right, middle",
                       "Strg + Umschalt + Alt + AltGr + Super rechts + W + A + D · Maus: links, rechts, Mitte");
        C.liveChipsJson = JSON.stringify(de
            ? ["Strg", "Umschalt", "Alt", "AltGr", "Super rechts", "W", "A", "D", "Maus: links, rechts, Mitte"]
            : ["Ctrl", "Shift", "Alt", "AltGr", "right Super", "W", "A", "D", "mouse: left, right, middle"]);
        C.liveMouseButtons = 7;
        C.liveDevice = "Micro-Star INT'L CO., LTD. MSI VERSA PRO WIRELESS Gaming Mouse";
        C.adapterIface = "enp0s20f0u4u1c2";
        C.adapterMode = "delaying";
        C.adapterDelayMs = 0.4;
        // The cause comes from pshb-core or the system: English in both.
        C.lastError = t("Window rule and window profiles not available: ", "Fensterregel und Fensterprofile nicht verfügbar: ")
                      + "the KWin script could not be loaded (org.kde.KWin.Scripting: no such object)";
        C.saveError = t("Settings could not be saved: /home/someone/.config/ps-hid-bridge/config.toml: No space left on device (os error 28). Changes apply only until the app closes.",
                        "Die Einstellungen konnten nicht gespeichert werden: /home/someone/.config/ps-hid-bridge/config.toml: No space left on device (os error 28). Änderungen gelten nur, bis die App geschlossen wird.");
        C.counterError = "No space left on device (os error 28)";
        C.latestVersion = "1.10.123";
        // The newest release's changelog, fetched by the update check: a
        // version newer than this app on top (entries are English in both).
        const log = JSON.parse(C.changelogJson);
        if (log.releases) {
            log.releases.unshift({
                version: "1.10.123", tag: "v1.10.123", build: false, date: "2026-12-24", compare: "v1.0.99...v1.10.123",
                changes: [{
                    area: "Desktop app", group: "app", hash: "0123abc", date: "2026-12-24",
                    items: ["A change in the newest version with a long title that has to wrap in a narrow window"],
                    details: [{ bullet: true, text: "What it does, why, and how it was tested, as the commit says." }]
                }]
            });
            C.changelogJson = JSON.stringify(log);
        }
        C.appUpdateAvailable = true;
        C.boardUpdateAvailable = true;
        C.canUpdateApp = false;
        C.updateLevel = "warn";
        C.updateStatus = t("Board update failed: the board did not answer within 30 s (HTTP 192.168.178.123:80)",
                           "Board-Update fehlgeschlagen: Das Board hat nicht innerhalb von 30 s geantwortet (HTTP 192.168.178.123:80)");
        C.accessMessage = t("Rule installed, but no access yet: unplug and replug the devices, or log out and in again.",
                            "Regel installiert, aber noch kein Zugriff: Steck die Geräte aus und wieder ein, oder melde dich ab und wieder an.");
        C.otherKeyboardsBlocked = 3;
        C.windowRuleAvailable = true;
        C.windowBackend = "KDE Plasma";
        C.installOffer = "install";
        C.installFolder = "/home/someone/Downloads";
        C.windowRuleEnabled = true;
        C.windowRule = "chiaki-ng, org.chiaki.chiaki-ng, PlayStation Remote Play";
        C.triggersJson = JSON.stringify([
            { kind: "combo", keys: "Ctrl+Alt+Shift+F12", action: "toggle", window_ms: 300 },
            { kind: "double-tap", keys: "RIGHTCTRL", action: "release-all", window_ms: 300 },
            { kind: "hold", keys: "Mouse5+Mouse4+LEFTMETA", action: "pause", window_ms: 300 }
        ]);
        C.latencySetupMessage = t("The administrator password was not entered (polkit): nothing was changed.",
                                  "Das Administratorpasswort wurde nicht eingegeben (polkit): Es wurde nichts geändert.");
        C.clockMessage = t("The board restarts with the new clock speeds; it is back in a few seconds.",
                           "Das Board startet mit den neuen Taktfrequenzen neu; es ist in ein paar Sekunden zurück.");
        C.usbStatus = t("No board in BOOTSEL mode found: hold the BOOTSEL button while plugging it in.",
                        "Kein Board im BOOTSEL-Modus gefunden: Halte die BOOTSEL-Taste beim Einstecken gedrückt.");
        C.boardInfoJson = JSON.stringify(boardInfo());
    }

    function stage(p50) {
        return { n: 12345678, min: Math.round(p50 / 3), p50: p50, p99: p50 * 4, max: p50 * 40 };
    }

    function boardInfo() {
        const diag = {
            rounds: 9876543, datagrams: 12345678, core0_spi: 98765432, cr_busy: 12,
            mouse_inputs: 8765432, mouse_waits: 765432, mouse_fetches: 87654321, mouse_misses: 123456,
            mouse_lead_us: 169, mouse_host_skips: 4321, mouse_floor_us: 163,
            crypto_min_ns: 9800, crypto_avg_ns: 10450, crypto_max_ns: 98765, crypto_fast: true,
            crypto_helped: 7654321, crypto_alone: 123456, spi_hz: 37500000, bus_timeouts: 3
        };
        const stages = { ready: 172, ready1: 98, detect: 6, bus: 21, spi: 38, handle: 61, usb: 610, gap: 1000, armed: 169 };
        for (const k in stages) {
            const s = stage(stages[k]);
            for (const f in s)
                diag[k + "_" + f] = s[f];
        }
        return {
            status: {
                firmware: "1.0.123", ip: "192.168.178.123", mask: "255.255.255.0", gateway: "192.168.178.1",
                dns: "192.168.178.1", mac: "02:11:22:33:44:55", ip_mode: "static", udp_port: 45045,
                allowed_sender: "192.168.178.0/24", partition: "B", uptime_ms: 3456789012,
                usb_mounted: true, usb_suspended: true,
                rx_ok: 123456789, rx_auth_fail: 12345, rx_replay: 2345, rx_filtered: 345, rx_malformed: 45
            },
            diag: diag,
            clock: { cpu_mhz: 200, spi_fast: true, cpu_hz: 150000000, spi_hz: 25000000,
                     cpu_fell_back: true, spi_fast_failed: true }
        };
    }
}
