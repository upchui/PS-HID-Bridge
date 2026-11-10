# PS5 HID Bridge

Mit Maus und Tastatur am Linux-Rechner die PS5 steuern, während das Bild per Remote Play (chiaki-ng) kommt.

Remote Play überträgt nur Controller-Eingaben. Darum läuft der Weg für Maus und Tastatur getrennt über ein eigenes Board am USB-Port der PS5:

1. Die Desktop-App liest Maus und Tastatur per evdev und greift sie exklusiv.
2. Sie verschlüsselt die Eingaben (ChaCha20-Poly1305) und schickt sie per UDP übers LAN.
3. Das Board (WIZnet W5500-EVB-Pico2) prüft und entschlüsselt sie und gibt sie als USB-HID-Report an die PS5 weiter.
4. Die PS5 sieht eine normale Tastatur-Maus-Kombination. Der Controller läuft weiter über Remote Play.

Das Paketformat steht in [protocol/SPEC.md](protocol/SPEC.md). Kommt 250 ms lang kein gültiges Paket, lässt das Board alle Tasten los (Failsafe).

## Stand

Alle Teile sind umgesetzt und automatisch getestet: Unit-Tests, gemeinsame Protokoll-Testvektoren und ein Board-Simulator, der den echten Firmware-Kern auf dem PC ausführt. **Auf echter Hardware ist noch nichts geprüft**; die Checkliste dafür steht unten unter „Inbetriebnahme“.

| Teil | Stand |
|---|---|
| Firmware: USB-HID (Tastatur, Maus mit 5 Tasten, beide Mausräder) | ✅ Host-Tests |
| Firmware: Netzwerk (W5500, DHCP/feste IP), verschlüsseltes Protokoll, Failsafe | ✅ Simulator |
| Weboberfläche des Boards (Passwort, Schlüssel, Netzwerk, Update) | ✅ Simulator |
| Firmware-Update übers Netz (A/B mit Rückfall) | ✅ Simulator; Bootrom-Teil nur auf Hardware prüfbar |
| Kommandozeilen-Tool `pshb-cli` | ✅ |
| Desktop-App (Kirigami, Assistent, Auslöser, Tray) | ✅ Offscreen-Selbsttest |

## Schnellstart

1. **Bauen:** `make submodules && make all` (siehe „Bauen“; der erste Lauf dauert wegen Qt lange).
2. **Board flashen:** BOOTSEL gedrückt halten, Board per USB an den Rechner stecken, es erscheint das Laufwerk `RP2350`. Erst `out/ps5-hid-bridge-pt.uf2` darauf kopieren (Partitionstabelle; das Board startet danach wieder als Laufwerk), dann `out/ps5-hid-bridge.uf2`.
3. **Board anschließen:** Netzwerkkabel, USB an die PS5. IP im Router nachsehen (`ps5-hid-bridge-XXXXXX`) und im Browser öffnen.
4. **Weboberfläche:** mit `ps5-hid-bridge` anmelden, eigenes Passwort setzen, unter „Sicherheit“ einen Schlüssel erzeugen.
5. **App:** `out/PS5-HID-Bridge-x86_64.AppImage` starten. Der Assistent fragt nach Board-Adresse und Schlüssel, nach den Geräten und nach dem Auslöser.
6. **PS5:** Tastaturlayout auf **Deutsch** stellen, in den Energiespar-Einstellungen die USB-Stromversorgung im Ruhemodus einschalten.

## Desktop-App

`out/PS5-HID-Bridge-x86_64.AppImage` läuft auf aktuellen Linux-Systemen (gebaut gegen glibc 2.34) unter Wayland und X11.

- **Assistent** beim ersten Start: Board (Adresse, Schlüssel, Verbindungstest) → Geräte auswählen und Zugriff einrichten → Auslöser → Live-Test, der zeigt, was bei der PS5 ankommt.
- **Status:** Steuerung an/aus, Verbindung, PS5 verbunden/Ruhemodus, Laufzeit zum Board mit Median und 95-%-Wert samt Verlauf, Pakete pro Sekunde.
- **Geräte:** Tastaturen und Mäuse, zusammengefasst je Gerät. „Taste drücken zum Erkennen“ markiert das Gerät, an dem gerade getippt wird.
- **Auslöser:** Tastenkombination, Maustaste (`Maus4`, `Maus5`), *Halten* (Steuerung nur, solange gedrückt) und *Zweimal tippen*; jeweils mit Aktion Umschalten, An, Aus, Pausieren oder Alles loslassen. „Aufnehmen“ übernimmt die gedrückte Kombination.
- **Fensterregel** (KDE Plasma): Steuerung automatisch an, solange ein Fenster aktiv ist, dessen Name oder Programm einen Text enthält (z. B. `chiaki`). „Pausieren“ setzt die Regel bis zum nächsten Fensterwechsel aus.
- **Maus:** Empfindlichkeit und Beschleunigung mit Kurvenvorschau; Standard sind die rohen Werte.
- **Tray-Symbol:** grau = Board nicht erreichbar, blau = bereit, grün = Steuerung an. Klick schaltet um. Beim Umschalten erscheint eine Plasma-Benachrichtigung.
- **Not-Aus:** `Strg+Alt+Shift+Esc` gibt Maus und Tastatur immer sofort frei und lässt auf der PS5 alles los. Diese Kombination lässt sich nicht abschalten.

Die Steuerung geht erst an, wenn alle Tasten losgelassen sind: Würden die Geräte übernommen, während eine Taste unten ist, bliebe sie auf dem Desktop hängen. Hängt die App, gibt ein Watchdog die Geräte nach einer Sekunde frei.

Einstellungen: `~/.config/ps5-hid-bridge/config.toml`, Schlüssel: `~/.config/ps5-hid-bridge/psk` (nur für den Benutzer lesbar).

### Gerätezugriff

Ein normaler Benutzer darf Tastatur und Maus nicht roh lesen. Die App richtet im Assistenten eine udev-Regel **nur für die gewählten Geräte** ein (fragt einmal nach dem Administrator-Passwort):

```
# /etc/udev/rules.d/70-ps5-hid-bridge.rules
SUBSYSTEM=="input", KERNEL=="event*", ATTRS{id/vendor}=="046d", ATTRS{id/product}=="c52b", TAG+="uaccess"
```

Von Hand: Hersteller- und Produktkennung zeigt `pshb-cli list-devices` in der Spalte „ID“; danach `sudo udevadm control --reload && sudo udevadm trigger -s input`. Der Dateiname muss mit einer Zahl unter 73 beginnen, sonst greift `uaccess` nicht. Alternative: `sudo usermod -aG input $USER` und neu anmelden (Zugriff auf alle Eingabegeräte).

## Kommandozeilen-Tool

`out/pshb-cli` macht dasselbe ohne Oberfläche (ohne Fensterregel).

```sh
pshb-cli list-devices                              # Tastaturen und Mäuse mit Zugriffsstatus
pshb-cli config set --board 192.168.1.50 --device 046d:c52b --device "G Pro"
pshb-cli config set-psk <64 Hex-Zeichen>           # Schlüssel speichern
pshb-cli ping                                      # Verbindung und Board-Status
pshb-cli run                                       # weiterleiten; Umschalten mit Strg+Alt+P
```

Tastennamen in Kombinationen richten sich nach der **Position auf einer US-Tastatur** (`Z` ist auf der deutschen Tastatur das `Y`). Modifikatoren: `Strg`, `Alt`, `AltGr`, `Shift`, `Super`; Maustasten: `Maus4`, `Maus5`, `MausMitte`.

`pshb-cli replay DATEI` spielt Eingaben aus einer Textdatei durch dieselbe Logik wie echte Geräte (`make test` nutzt das gegen den Simulator):

```
combo Strg+Alt+P      # Kombination drücken und loslassen
wait 30               # Millisekunden
key LEFTSHIFT down
tap H
key LEFTSHIFT up
move 30 -10           # Maus
wheel 1
button BTN_LEFT down
focus on              # Fensterregel: Fenster hat den Fokus
```

## Board

### Weboberfläche

Unter `http://<IP-des-Boards>/`:

- **Anmeldung:** Werkspasswort `ps5-hid-bridge`. Beim ersten Anmelden muss ein eigenes Passwort (8–64 Zeichen) gesetzt werden. Nach drei Fehlversuchen wächst die Wartezeit bis auf 64 Sekunden.
- **Status:** Verbindung zur PS5, Netzwerk, Firmware und Slot, Paketzähler.
- **Netzwerk:** DHCP oder feste Adresse, UDP-Port. Speichern startet das Board neu.
- **Sicherheit:** Pre-Shared Key erzeugen (wird genau einmal angezeigt) oder einen vorhandenen eintragen, Absenderfilter, Passwort ändern.
- **System:** Firmware-Update, Neustart, Werkseinstellungen.

Die Weboberfläche läuft über unverschlüsseltes HTTP im lokalen Netz. Das Passwort schützt vor versehentlichen Änderungen und vor anderen Geräten im Netz, nicht vor jemandem, der den Netzwerkverkehr mitschneiden kann. Die Eingaben selbst sind immer verschlüsselt.

### Netzwerk und LED

Das Board holt sich per DHCP eine Adresse und meldet sich als `ps5-hid-bridge-XXXXXX` (letzte sechs Stellen der MAC-Adresse). Eingaben empfängt es auf UDP-Port 45045.

LED: blinkt langsam = kein USB-Host, blinkt schnell = USB da, aber noch keine IP, leuchtet = bereit (flackert kurz bei Eingaben).

### Firmware-Update übers Netz

Weboberfläche → System → `ps5-hid-bridge.uf2` hochladen. Das Board schreibt die neue Firmware in den zweiten Speicherplatz (A/B), prüft Prüfsumme und Inhalt und startet sie „auf Probe“. Erst wenn sie selbst gestartet ist und den Netzwerkchip erreicht, wird sie dauerhaft übernommen. Startet sie nicht richtig, kehrt das Board nach einem Neustart zur vorherigen Version zurück. Jede neue Firmware trägt eine höhere Build-Nummer (Anzahl der Commits); der Bootrom startet immer die neuere.

Voraussetzung ist die Partitionstabelle aus der Erstinstallation (`ps5-hid-bridge-pt.uf2`).

### Werkseinstellungen und Wiederherstellung

- **Werkseinstellungen:** in der Weboberfläche oder BOOTSEL-Taste im laufenden Betrieb 5 Sekunden halten. Passwort, Schlüssel und Netzwerkeinstellungen werden gelöscht.
- **Neu flashen geht immer:** BOOTSEL beim Einstecken halten und UF2 kopieren.
- **Alles löschen:** `out/ps5-hid-bridge-nuke.uf2` per BOOTSEL kopieren. Es löscht den ganzen Flash und kehrt zum Laufwerk zurück; danach Partitionstabelle und Firmware neu installieren.

### Weitere Images

| Datei | Zweck |
|---|---|
| `ps5-hid-bridge.uf2` | normale Firmware |
| `ps5-hid-bridge-pt.uf2` | Partitionstabelle (einmalig vor der ersten Firmware) |
| `ps5-hid-bridge-hidtest.uf2` | ohne Netzwerk: 5 s nach dem Einstecken fährt die Maus ein Quadrat |
| `ps5-hid-bridge-hidtest-mouse8.uf2` | dasselbe mit 8-Bit-Mausachsen, falls ein Host den 16-Bit-Deskriptor nicht mag |
| `ps5-hid-bridge-plain.uf2` | **nur zur Inbetriebnahme:** nimmt zusätzlich unverschlüsselte Pakete an |
| `ps5-hid-bridge-rollbacktest.uf2` | **nur zum Testen:** bestätigt sich nach einem Update nie, das Board muss zur vorherigen Version zurückkehren |
| `ps5-hid-bridge-nuke.uf2` | löscht den ganzen Flash |

Konsole: UART0 (GP0/GP1, 115200 Baud). Die USB-Kennung `1209:0001` ist die Test-PID von pid.codes und nur für den privaten Gebrauch gedacht.

## Inbetriebnahme mit echter Hardware

Diese Punkte lassen sich ohne Board nicht prüfen. Reihenfolge so wählen, dass jeder Schritt auf dem vorigen aufbaut:

1. `picotool info -a` (BOOTSEL-Modus): RP2350A, 2 MB Flash.
2. `ps5-hid-bridge-hidtest.uf2` am PC: `lsusb -v` (1209:0001, zwei HID-Interfaces), `evtest` (Maus fährt ein Quadrat), Abfragerate 1000 Hz (`usbhid-dump` / usbmon).
3. Dasselbe Image an der PS5: Maus im Menü und in einem Spiel mit Maus-Unterstützung, auch während einer laufenden Remote-Play-Sitzung (Aimlabs). Falls die Maus nicht reagiert: `-hidtest-mouse8.uf2` probieren.
4. Partitionstabelle + `ps5-hid-bridge-plain.uf2`: DHCP, Hostname im Router, `make udp-test IP=… ARGS='--plain flood --rate 1000 --seconds 10'` ohne Verluste; SPI-Takt 37,5 MHz testen (`PSHB_W5500_SPI_HZ`).
5. Wartezeit auf Kern 0, während die Weboberfläche lädt (GPIO-Puls + Logikanalysator): soll unter 0,5 ms bleiben.
6. Normale Firmware, Schlüssel per Weboberfläche, App verbinden; Failsafe prüfen (Netzwerkkabel ziehen bei gedrückter Taste → nach 250 ms losgelassen).
7. Update übers Netz mit `ps5-hid-bridge.uf2` (Slot wechselt von A nach B), dann mit `ps5-hid-bridge-rollbacktest.uf2`: Dieses Image bestätigt sich absichtlich nie, nach etwa 17 Sekunden muss das Board von selbst mit der vorherigen Version neu starten (Slot und Firmware in der Weboberfläche prüfen).
8. Einstellungen bleiben nach mehreren Updates erhalten (liegen außerhalb der Firmware-Slots).

## Bauen

Gebaut wird ausschließlich in Docker. Auf dem Rechner braucht es nur `docker`, `make` und `git`.

```sh
make submodules        # Pico SDK, TinyUSB, ioLibrary, Monocypher (gepinnt, nicht rekursiv)
make firmware          # -> out/*.uf2
make desktop-cli       # -> out/pshb-cli
make desktop           # -> out/PS5-HID-Bridge-x86_64.AppImage
make test              # Unit-Tests, Testvektoren, Simulator, Rust-Tests
make all               # alles, inklusive AppImage-Selbsttest
```

Der erste `make desktop` baut Qt 6.12 und Kirigami aus dem Quellcode und dauert deshalb lange (je nach Rechner 1–3 Stunden). Danach liegt alles im Docker-Cache. Die Zahl der parallelen Qt-Jobs lässt sich mit `make desktop QT_JOBS=4` anpassen; qtdeclarative braucht pro Job etwa 2 GB RAM.

Alle Versionen stehen zentral in [versions.env](versions.env); Downloads werden per SHA-256 geprüft. `make check-pins` prüft, dass die Submodule auf den dort eingetragenen Tags stehen.

Für Entwicklungs-Builds ohne Weboberfläche lässt sich ein Schlüssel einkompilieren: `make firmware DEV_PSK=<64 Hex-Zeichen>`. Er gilt nur, solange in der Weboberfläche keiner gesetzt ist.

## Ohne Board testen

`make sim` startet einen Board-Simulator: Eingaben auf UDP-Port 45045, Weboberfläche unter http://localhost:8080. Er nutzt denselben Kern wie die Firmware und gibt statt USB-Reports Textzeilen aus (`KBD …`, `MOUSE …`); die Einstellungen bleiben in `.cache/sim/flash.bin` erhalten. App oder `pshb-cli` mit Adresse `127.0.0.1` verbinden.

Testverkehr an ein Board oder an den Simulator:

```sh
make udp-test IP=192.168.1.50 ARGS='--psk-file psk.txt ping'
make udp-test IP=192.168.1.50 ARGS='--psk-file psk.txt pattern'   # Maus-Quadrat, tippt „hallo“
make udp-test IP=192.168.1.50 ARGS='--psk-file psk.txt flood --rate 1000 --seconds 5'
```

## Verzeichnisse

| Pfad | Inhalt |
|---|---|
| `firmware/src/core/` | portabler Firmware-Kern (Protokoll, Eingaben, Einstellungen, Webserver, Update), läuft auch auf dem PC |
| `firmware/src/pico/` | RP2350-spezifisch: USB, W5500, Flash, Bootrom |
| `firmware/src/web/` | Weboberfläche des Boards (wird eingebettet) |
| `firmware/host/` | Host-Build: Unit-Tests und Board-Simulator |
| `desktop/core/` | Rust-Kern (evdev, Auslöser, Krypto, UDP) und `pshb-cli` |
| `desktop/ui/` | Kirigami-Oberfläche (QML, cxx-qt), Tray, Fensterregel |
| `protocol/` | Paketformat und gemeinsame Testvektoren |
| `docker/` | Build-Umgebungen |
| `tools/` | Testwerkzeuge (UDP-Test, Simulator-Integrationstests) |
