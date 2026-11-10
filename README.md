# PS5 HID Bridge

Mit Maus und Tastatur am Linux-Rechner die PS5 steuern, während das Bild per Remote Play (chiaki-ng) kommt.

Remote Play überträgt nur Controller-Eingaben. Darum läuft der Weg für Maus und Tastatur getrennt über ein eigenes Board am USB-Port der PS5:

1. Die Desktop-App liest Maus und Tastatur per evdev und greift sie exklusiv.
2. Sie verschlüsselt die Eingaben (ChaCha20-Poly1305) und schickt sie per UDP übers LAN.
3. Das Board (WIZnet W5500-EVB-Pico2) prüft und entschlüsselt sie und gibt sie als USB-HID-Report an die PS5 weiter.
4. Die PS5 sieht eine normale Tastatur-Maus-Kombination. Der Controller läuft weiter über Remote Play.

Das Paketformat steht in [protocol/SPEC.md](protocol/SPEC.md). Kommt 250 ms lang kein gültiges Paket, lässt das Board alle Tasten los (Failsafe).

## Stand

| Teil | Stand |
|---|---|
| Repository, Docker-Builds | ✅ |
| Firmware: USB-HID | ✅ (Host-Tests; Hardware-Test steht aus) |
| Firmware: Netzwerk, DHCP, UDP | ✅ (Simulator; Hardware-Test steht aus) |
| Protokoll mit Verschlüsselung | ✅ (C, Rust, Python gegen gemeinsame Testvektoren) |
| Kommandozeilen-Tool | ✅ |
| Weboberfläche des Boards | ✅ (im Simulator getestet) |
| Desktop-App (Kirigami) | Grundgerüst |
| Firmware-Update übers Netz | – |

## Bauen

Gebaut wird ausschließlich in Docker. Auf dem Rechner braucht es nur `docker`, `make` und `git`.

```sh
make submodules        # Pico SDK, TinyUSB, ioLibrary, Monocypher (gepinnt, nicht rekursiv)
make firmware          # -> out/ps5-hid-bridge.uf2
make desktop-cli       # -> out/pshb-cli
make desktop           # -> out/PS5-HID-Bridge-x86_64.AppImage
make test              # Unit-Tests, Protokoll-Testvektoren, Simulator
make all               # alles
```

Der erste `make desktop` baut Qt 6 und Kirigami aus dem Quellcode und dauert deshalb lange (je nach Rechner 1–3 Stunden). Danach liegt alles im Docker-Cache. Die Zahl der parallelen Qt-Jobs lässt sich mit `make desktop QT_JOBS=4` anpassen; qtdeclarative braucht pro Job etwa 2 GB RAM.

Alle Versionen stehen zentral in [versions.env](versions.env). `make check-pins` prüft, dass die Submodule auf den dort eingetragenen Tags stehen.

## Kommandozeilen-Tool

`out/pshb-cli` läuft ohne Oberfläche und reicht für den Alltag, bis die App fertig ist.

```sh
pshb-cli list-devices                              # Tastaturen und Mäuse anzeigen
pshb-cli config set --board 192.168.1.50 --device "K400" --device "G Pro"
pshb-cli config set-psk <64 Hex-Zeichen>           # Schlüssel speichern (Datei mit Rechten 0600)
pshb-cli ping                                      # Verbindung und Board-Status
pshb-cli run                                       # weiterleiten; Umschalten mit Strg+Alt+P
```

- **Umschalten:** Standard ist `Strg+Alt+P` (änderbar mit `config set --hotkey`). Die Steuerung geht erst an, wenn alle Tasten losgelassen sind; erst dann werden Maus und Tastatur exklusiv übernommen. Sonst bleiben auf dem Desktop Tasten hängen.
- **Not-Aus:** `Strg+Alt+Shift+Esc` gibt die Geräte immer frei und lässt auf der PS5 alles los. Diese Kombination lässt sich nicht abschalten.
- **Hängt das Programm**, gibt ein Watchdog die Geräte nach einer Sekunde frei. Beim Beenden (Strg+C, `kill`) wird ebenfalls alles losgelassen.
- Einstellungen liegen in `~/.config/ps5-hid-bridge/config.toml`, der Schlüssel in `~/.config/ps5-hid-bridge/psk`.
- Tastennamen in Kombinationen richten sich nach der **Position auf einer US-Tastatur** (`Z` ist auf der deutschen Tastatur das `Y`). Modifikatoren: `Strg`, `Alt`, `AltGr`, `Shift`, `Super`; Maustasten: `Maus4`, `Maus5`, `MausMitte`.

### Gerätezugriff

Ohne Zusatzrechte darf ein normaler Benutzer Tastatur und Maus nicht roh lesen. Zwei Möglichkeiten:

1. **udev-Regel nur für die gewählten Geräte** (empfohlen; die App richtet sie im Assistenten selbst ein). Hersteller- und Produktkennung zeigt `pshb-cli list-devices` in der Spalte „ID“:

   ```
   # /etc/udev/rules.d/70-ps5-hid-bridge.rules
   SUBSYSTEM=="input", KERNEL=="event*", ATTRS{id/vendor}=="046d", ATTRS{id/product}=="c52b", TAG+="uaccess"
   ```

   Danach `sudo udevadm control --reload && sudo udevadm trigger -s input`. Der Dateiname muss mit einer Zahl unter 73 beginnen, sonst greift `uaccess` nicht.
2. **Gruppe `input`:** `sudo usermod -aG input $USER`, danach neu anmelden. Das gibt Zugriff auf alle Eingabegeräte.

### Testskripte

`pshb-cli replay DATEI` spielt Eingaben aus einer Textdatei ab, durch dieselbe Logik wie echte Geräte. `make test` nutzt das gegen den Simulator.

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

## Verzeichnisse

| Pfad | Inhalt |
|---|---|
| `firmware/` | C-Firmware für das RP2350 (Pico SDK, TinyUSB, ioLibrary) |
| `firmware/src/core/` | portabler Kern, läuft auch auf dem PC (Tests, Simulator) |
| `firmware/host/` | Host-Build: Unit-Tests und Board-Simulator |
| `desktop/core/` | Rust-Kern (evdev, Krypto, UDP) und `pshb-cli` |
| `desktop/ui/` | Kirigami-Oberfläche (QML, cxx-qt) |
| `protocol/` | Paketformat und gemeinsame Testvektoren |
| `docker/` | Build-Umgebungen |
| `out/` | Build-Ergebnisse |

## Hardware

- WIZnet W5500-EVB-Pico2 (RP2350, W5500, RJ45), per USB-C → USB-A an der PS5.
- Damit das Board im Ruhemodus der PS5 aktiv bleibt, in den Energiespar-Einstellungen der PS5 die USB-Stromversorgung im Ruhemodus einschalten.
- Die PS5 muss auf **deutsches Tastaturlayout** gestellt sein. Das Board sendet Positionscodes, sonst sind z. B. Z und Y vertauscht.

### Erstinstallation

1. BOOTSEL gedrückt halten, Board per USB an den Rechner stecken. Es erscheint das Laufwerk `RP2350`.
2. `out/ps5-hid-bridge-pt.uf2` darauf kopieren (Partitionstabelle: zwei Firmware-Slots für Updates übers Netz und ein Bereich für die Einstellungen). Das Board startet danach wieder im BOOTSEL-Modus.
3. `out/ps5-hid-bridge.uf2` darauf kopieren.
4. Netzwerkkabel anschließen, die IP im Router nachsehen (`ps5-hid-bridge-XXXXXX`) und im Browser öffnen.

### Weboberfläche

Unter `http://<IP-des-Boards>/`:

- **Anmeldung:** Werkspasswort `ps5-hid-bridge`. Beim ersten Anmelden muss ein eigenes Passwort (8–64 Zeichen) gesetzt werden. Nach drei Fehlversuchen wächst die Wartezeit bis auf 64 Sekunden.
- **Status:** Verbindung zur PS5, Netzwerk, Firmware, Paketzähler.
- **Netzwerk:** DHCP oder feste Adresse, UDP-Port. Speichern startet das Board neu.
- **Sicherheit:** Pre-Shared Key erzeugen (wird genau einmal angezeigt, dann in der App eintragen) oder einen vorhandenen eintragen, Absenderfilter, Passwort ändern.
- **System:** Neustart, Werkseinstellungen.

**Werkseinstellungen ohne Weboberfläche:** BOOTSEL-Taste im laufenden Betrieb 5 Sekunden halten. Passwort, Schlüssel und Netzwerkeinstellungen werden gelöscht, das Board startet neu.

Die Weboberfläche läuft über unverschlüsseltes HTTP im lokalen Netz. Das Passwort schützt vor versehentlichen Änderungen und vor Geräten im selben Netz, nicht vor jemandem, der den Netzwerkverkehr mitschneiden kann.

### Netzwerk

Das Board holt sich per DHCP eine Adresse und meldet sich als `ps5-hid-bridge-XXXXXX` (die letzten sechs Stellen der MAC-Adresse). Unter diesem Namen steht es im Router bzw. in UniFi. Eingaben empfängt es auf UDP-Port 45045.

LED: blinkt langsam = kein USB-Host, blinkt schnell = USB da, aber noch keine IP, leuchtet = bereit (flackert kurz bei Eingaben).

### Ohne Board testen

`make sim` startet einen Board-Simulator: Eingaben auf UDP-Port 45045, Weboberfläche unter http://localhost:8080. Er nutzt denselben Kern wie die Firmware und gibt statt USB-Reports Textzeilen aus (`KBD …`, `MOUSE …`). Die Einstellungen bleiben in `.cache/sim/flash.bin` erhalten. `make test` prüft damit den ganzen Eingabeweg und die Weboberfläche automatisch.

Testverkehr an ein Board oder an den Simulator schicken:

```sh
make udp-test IP=192.168.1.50 ARGS='--psk-file psk.txt ping'
make udp-test IP=192.168.1.50 ARGS='--psk-file psk.txt pattern'   # Maus-Quadrat, tippt „hallo“
make udp-test IP=192.168.1.50 ARGS='--psk-file psk.txt flood --rate 1000 --seconds 5'
```

Bis die Weboberfläche fertig ist (Schritt 6), bekommt die Firmware den Schlüssel beim Bauen: `make firmware DEV_PSK=<64 Hex-Zeichen>`. Ohne Schlüssel lehnt sie alle Pakete ab.

Statt `--psk-file` geht auch `--plain`, aber nur mit dem Debug-Image `out/ps5-hid-bridge-plain.uf2`, das unverschlüsselte Pakete annimmt. Dieses Image nur zur Inbetriebnahme verwenden.

### Testimages ohne Netzwerk

| Datei | Zweck |
|---|---|
| `out/ps5-hid-bridge-hidtest.uf2` | 5 s nach dem Einstecken fährt die Maus langsam ein Quadrat (16-Bit-Maus). |
| `out/ps5-hid-bridge-hidtest-mouse8.uf2` | dasselbe mit 8-Bit-Mausachsen, falls ein Host den 16-Bit-Deskriptor nicht mag. |

Flashen: BOOTSEL gedrückt halten, Board an den Rechner stecken, UF2 auf das Laufwerk `RP2350` kopieren. Die LED blinkt, solange kein USB-Host das Board eingebunden hat, und leuchtet dauerhaft, sobald es eingebunden ist. Konsole: UART0 (GP0/GP1, 115200 Baud).

Die USB-Kennung `1209:0001` ist die Test-PID von pid.codes und nur für den privaten Gebrauch gedacht.
