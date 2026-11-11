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
        const actions = { "toggle": "toggle", "on": "on", "off": "off", "pause": "pause", "release-all": "release all" };
        const kinds = { "combo": "", "hold": " (hold)", "double-tap": " (double tap)" };
        let parts = triggers.map(t => t.keys + kinds[t.kind] + (t.kind === "hold" ? "" : ": " + actions[t.action]));
        if (AppController.windowRuleEnabled && AppController.windowRule !== "")
            parts.push("automatically while \"" + AppController.windowRule + "\" is active");
        return parts.length ? parts.join(" · ") : "no triggers set up";
    }

    ColumnLayout {
        spacing: Kirigami.Units.largeSpacing

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            type: Kirigami.MessageType.Error
            text: AppController.lastError
            visible: AppController.lastError !== ""
            // Not showCloseButton: it hides the message for good (the
            // binding above would be gone and later errors not shown).
            actions: [
                Kirigami.Action {
                    icon.name: "dialog-close"
                    text: "Dismiss"
                    onTriggered: AppController.lastError = ""
                }
            ]
        }
        Kirigami.InlineMessage {
            Layout.fillWidth: true
            type: Kirigami.MessageType.Warning
            text: AppController.devicesBlocked + " selected device interface(s) without access. Set it up under \"Devices\"."
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
                        text: "Bridge"
                        enabled: AppController.connected
                        checked: AppController.active || AppController.pending
                        onToggled: AppController.setOn(checked)
                    }
                }
                Controls.Label {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: AppController.active
                          ? "Mouse and keyboard go to the PS5. The desktop gets no input."
                          : "Mouse and keyboard belong to the desktop."
                }
                Controls.Label {
                    Layout.fillWidth: true
                    visible: AppController.active
                    font.bold: true
                    text: "To the PS5: " + (AppController.liveText !== "" ? AppController.liveText : "no key")
                }
            }
        }

        Kirigami.FormLayout {
            Layout.fillWidth: true

            Controls.Label {
                Kirigami.FormData.label: "Board:"
                text: AppController.boardAddress === "" ? "not set up (see \"Board\")"
                      : (AppController.connected ? "connected" : "not reachable") + " (" + AppController.boardAddress + ")"
            }
            Controls.Label {
                Kirigami.FormData.label: "PS5:"
                text: AppController.boardUsb !== "" ? AppController.boardUsb : "unknown"
            }
            Controls.Label {
                Kirigami.FormData.label: "Latency:"
                text: AppController.rttMs >= 0
                      ? AppController.rttMs.toFixed(2) + " ms round trip (median " + AppController.rttP50.toFixed(2)
                        + " ms, 95 % below " + AppController.rttP95.toFixed(2) + " ms)"
                      : "–"
            }
            Controls.Label {
                Kirigami.FormData.label: "Input → network:"
                text: (AppController.inputP50 >= 0
                       ? AppController.inputP50.toFixed(2) + " ms median, 95 % below " + AppController.inputP95.toFixed(2)
                         + " ms, max " + AppController.inputMax.toFixed(2) + " ms"
                       : "– (measured while the bridge is on)")
                      + (AppController.connected && !AppController.realtime
                         ? " · normal priority (real-time not available, see README)" : "")
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
            Controls.Label {
                Kirigami.FormData.label: "Packets:"
                text: AppController.packetsPerSecond + " per second"
            }
            Controls.Label {
                Kirigami.FormData.label: "Devices:"
                text: AppController.devicesOpen + " interface(s) open"
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
            text: "Toggle: " + page.triggerSummary()
        }
        Controls.Label {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            text: "Emergency stop (always releases everything): Ctrl+Alt+Shift+Esc"
        }

        RowLayout {
            Controls.Button {
                text: "Release all keys"
                enabled: AppController.connected
                onClicked: AppController.releaseAll()
            }
            Controls.Button {
                text: AppController.paused ? "Resume" : "Pause"
                enabled: AppController.connected
                onClicked: AppController.pause()
            }
            Controls.Button {
                text: "Reconnect"
                onClicked: AppController.reconnect()
            }
        }
    }
}
