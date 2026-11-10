import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import PS5HIDBridge

Kirigami.ScrollablePage {
    id: page
    title: "Einrichtung – Schritt " + (step + 1) + " von " + titles.length

    signal finished()

    property int step: 0
    readonly property var titles: ["Willkommen", "Board", "Geräte", "Auslöser", "Test"]
    property string boardError: ""

    footer: Controls.ToolBar {
        contentItem: RowLayout {
            Controls.Label {
                Layout.fillWidth: true
                leftPadding: Kirigami.Units.largeSpacing
                text: page.titles[page.step]
                font.bold: true
            }
            Controls.Button {
                text: "Zurück"
                enabled: page.step > 0
                onClicked: page.step--
            }
            Controls.Button {
                text: page.step === page.titles.length - 1 ? "Fertig" : "Weiter"
                highlighted: true
                onClicked: {
                    if (page.step === 3 && !triggers.save())
                        return;
                    if (page.step === page.titles.length - 1) {
                        AppController.finishSetup();
                        page.finished();
                    } else {
                        page.step++;
                    }
                }
            }
        }
    }

    StackLayout {
        currentIndex: page.step

        // 0 -- Willkommen
        ColumnLayout {
            spacing: Kirigami.Units.largeSpacing
            Kirigami.Heading { text: "PS5 HID Bridge einrichten" }
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                text: "Mit dieser App steuerst du die PS5 mit Maus und Tastatur dieses Rechners, während das Bild per Remote Play kommt. Dafür brauchst du:"
            }
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                textFormat: Text.StyledText
                text: "• das Board mit aktueller Firmware, per USB an der PS5 und per Kabel im Netzwerk<br>"
                      + "• den Pre-Shared Key aus der Weboberfläche des Boards (Sicherheit → Neuen Schlüssel erzeugen)<br>"
                      + "• auf der PS5: Tastaturlayout <b>Deutsch</b> und USB-Stromversorgung im Ruhemodus eingeschaltet"
            }
        }

        // 1 -- Board
        ColumnLayout {
            spacing: Kirigami.Units.largeSpacing
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                text: "Adresse des Boards (steht im Router als ps5-hid-bridge-…) und den Schlüssel aus seiner Weboberfläche eintragen."
            }
            Kirigami.FormLayout {
                Layout.fillWidth: true
                Controls.TextField {
                    id: address
                    Kirigami.FormData.label: "Adresse:"
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 16
                    text: AppController.boardAddress
                    placeholderText: "z. B. 192.168.1.50"
                }
                Controls.TextField {
                    id: psk
                    Kirigami.FormData.label: "Pre-Shared Key:"
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 24
                    echoMode: TextInput.Password
                    placeholderText: AppController.pskSet ? "gespeichert – nur zum Ändern ausfüllen" : "64 Hex-Zeichen"
                }
            }
            Controls.Button {
                text: "Verbinden"
                highlighted: true
                onClicked: page.boardError = AppController.saveBoard(address.text, psk.text)
            }
            Kirigami.InlineMessage {
                Layout.fillWidth: true
                visible: page.boardError !== ""
                type: Kirigami.MessageType.Error
                text: page.boardError
            }
            Kirigami.InlineMessage {
                Layout.fillWidth: true
                visible: page.boardError === "" && AppController.boardAddress !== ""
                type: AppController.connected ? Kirigami.MessageType.Positive : Kirigami.MessageType.Information
                text: AppController.connected
                      ? "Verbunden. " + AppController.boardUsb + ", Firmware " + AppController.boardFirmware + "."
                      : "Warte auf Antwort vom Board … Stimmen Adresse und Schlüssel?"
            }
        }

        // 2 -- Geräte
        ColumnLayout {
            spacing: Kirigami.Units.largeSpacing
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                text: "Tastatur und Maus auswählen, die an die PS5 gehen sollen. Danach fragt die App einmal nach dem Administrator-Passwort, um Lesezugriff auf genau diese Geräte einzurichten."
            }
            DeviceList {
                Layout.fillWidth: true
            }
        }

        // 3 -- Auslöser
        ColumnLayout {
            spacing: Kirigami.Units.largeSpacing
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                text: "Womit schaltest du zwischen Desktop und PS5 um? Voreingestellt ist Strg+Alt+P. Weitere Arten (Maustaste, Halten, Fensterregel) gibt es später unter „Auslöser“."
            }
            TriggerEditor {
                id: triggers
                Layout.fillWidth: true
            }
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                text: "Not-Aus, immer verfügbar: Strg+Alt+Shift+Esc."
            }
        }

        // 4 -- Test
        ColumnLayout {
            spacing: Kirigami.Units.largeSpacing
            Controls.Label {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                text: "Jetzt den Auslöser drücken und wieder loslassen. Wenn die Steuerung an ist, siehst du hier, was bei der PS5 ankommt. Mit dem Auslöser wieder ausschalten."
            }
            Kirigami.FormLayout {
                Layout.fillWidth: true
                Controls.Label {
                    Kirigami.FormData.label: "Status:"
                    text: AppController.statusText
                    font.bold: true
                }
                Controls.Label {
                    Kirigami.FormData.label: "An die PS5:"
                    text: AppController.active ? (AppController.liveText !== "" ? AppController.liveText : "keine Taste") : "–"
                }
                Controls.Label {
                    Kirigami.FormData.label: "PS5:"
                    text: AppController.boardUsb !== "" ? AppController.boardUsb : "–"
                }
                Controls.Label {
                    Kirigami.FormData.label: "Laufzeit:"
                    text: AppController.rttMs >= 0 ? AppController.rttMs.toFixed(2) + " ms" : "–"
                }
            }
        }
    }

    onStepChanged: if (step === 2) AppController.refreshDevices()
}
