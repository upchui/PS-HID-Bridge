import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import PS5HIDBridge

Kirigami.ScrollablePage {
    id: page
    title: "Status"

    Component.onCompleted: AppController.setLatencyView(true)
    Component.onDestruction: AppController.setLatencyView(false)

    readonly property var triggers: JSON.parse(AppController.triggersJson)

    function triggerSummary() {
        const actions = { "toggle": "umschalten", "on": "an", "off": "aus", "pause": "pausieren", "release-all": "alles loslassen" };
        const kinds = { "combo": "", "hold": " (halten)", "double-tap": " (zweimal tippen)" };
        let parts = triggers.map(t => t.keys + kinds[t.kind] + ": " + actions[t.action]);
        if (AppController.windowRule !== "")
            parts.push("automatisch, wenn „" + AppController.windowRule + "“ aktiv ist");
        return parts.length ? parts.join(" · ") : "keine Auslöser eingerichtet";
    }

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            type: Kirigami.MessageType.Error
            text: AppController.lastError
            visible: AppController.lastError !== ""
            showCloseButton: true
        }
        Kirigami.InlineMessage {
            Layout.fillWidth: true
            type: Kirigami.MessageType.Warning
            text: AppController.devicesBlocked + " gewählte Geräteschnittstelle(n) ohne Zugriff. Unter „Geräte“ einrichten."
            visible: AppController.devicesBlocked > 0
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: stateColumn.implicitHeight + Kirigami.Units.gridUnit * 2
            radius: Kirigami.Units.cornerRadius
            color: AppController.active ? Qt.rgba(0.18, 0.62, 0.36, 0.18)
                 : AppController.connected ? Qt.rgba(0.11, 0.36, 0.85, 0.12)
                 : Qt.rgba(0.5, 0.5, 0.5, 0.12)
            border.color: AppController.active ? "#2e9d5b" : AppController.connected ? "#1d5bd8" : "#7a8290"

            ColumnLayout {
                id: stateColumn
                anchors.fill: parent
                anchors.margins: Kirigami.Units.gridUnit

                RowLayout {
                    Layout.fillWidth: true
                    Kirigami.Heading {
                        Layout.fillWidth: true
                        level: 1
                        text: AppController.statusText
                    }
                    Controls.Switch {
                        text: "Steuerung"
                        enabled: AppController.connected
                        checked: AppController.active || AppController.pending
                        onToggled: AppController.setOn(checked)
                    }
                }
                Controls.Label {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: AppController.active
                          ? "Maus und Tastatur gehen an die PS5. Der Desktop bekommt keine Eingaben."
                          : "Maus und Tastatur gehören dem Desktop."
                }
                Controls.Label {
                    Layout.fillWidth: true
                    visible: AppController.active
                    font.bold: true
                    text: "An die PS5: " + (AppController.liveText !== "" ? AppController.liveText : "keine Taste")
                }
            }
        }

        Kirigami.FormLayout {
            Layout.fillWidth: true

            Controls.Label {
                Kirigami.FormData.label: "Board:"
                text: AppController.boardAddress === "" ? "nicht eingerichtet (unter „Board“)"
                      : (AppController.connected ? "verbunden" : "nicht erreichbar") + " (" + AppController.boardAddress + ")"
            }
            Controls.Label {
                Kirigami.FormData.label: "PS5:"
                text: AppController.boardUsb !== "" ? AppController.boardUsb : "unbekannt"
            }
            Controls.Label {
                Kirigami.FormData.label: "Laufzeit:"
                text: AppController.rttMs >= 0
                      ? AppController.rttMs.toFixed(2) + " ms hin und zurück (Median " + AppController.rttP50.toFixed(2)
                        + " ms, 95 % unter " + AppController.rttP95.toFixed(2) + " ms)"
                      : "–"
            }
            Controls.Label {
                Kirigami.FormData.label: "Pakete:"
                text: AppController.packetsPerSecond + " pro Sekunde"
            }
            Controls.Label {
                Kirigami.FormData.label: "Geräte:"
                text: AppController.devicesOpen + " Schnittstelle(n) aktiv"
            }
            Controls.Label {
                Kirigami.FormData.label: "Firmware:"
                text: AppController.boardFirmware !== "" ? AppController.boardFirmware : "–"
            }
        }

        Canvas {
            id: spark
            Layout.fillWidth: true
            implicitHeight: Kirigami.Units.gridUnit * 4
            visible: AppController.connected
            readonly property var values: JSON.parse(AppController.rttHistoryJson)
            onValuesChanged: requestPaint()
            onWidthChanged: requestPaint()
            onPaint: {
                const ctx = getContext("2d");
                ctx.reset();
                const v = values;
                if (v.length < 2)
                    return;
                const max = Math.max(1, Math.max.apply(null, v) * 1.2);
                const x = i => (width - 2) * i / (v.length - 1) + 1;
                const y = val => height - 2 - (height - 4) * val / max;
                ctx.strokeStyle = Kirigami.Theme.highlightColor;
                ctx.lineWidth = 1.5;
                ctx.beginPath();
                ctx.moveTo(x(0), y(v[0]));
                for (let i = 1; i < v.length; i++)
                    ctx.lineTo(x(i), y(v[i]));
                ctx.stroke();
                ctx.fillStyle = Kirigami.Theme.disabledTextColor;
                ctx.font = "10px sans-serif";
                ctx.fillText(max.toFixed(1) + " ms", 2, 10);
            }
        }

        Kirigami.Separator { Layout.fillWidth: true }

        Controls.Label {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            text: "Umschalten: " + page.triggerSummary()
        }
        Controls.Label {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            text: "Not-Aus (gibt immer alles frei): Strg+Alt+Shift+Esc"
        }

        RowLayout {
            Controls.Button {
                text: "Alle Tasten loslassen"
                enabled: AppController.connected
                onClicked: AppController.releaseAll()
            }
            Controls.Button {
                text: AppController.paused ? "Fortsetzen" : "Pausieren"
                enabled: AppController.connected
                onClicked: AppController.pause()
            }
            Controls.Button {
                text: "Neu verbinden"
                onClicked: AppController.reconnect()
            }
        }
    }
}
