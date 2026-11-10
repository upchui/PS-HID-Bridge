# PS5 HID Bridge

Mit Maus und Tastatur am Linux-Rechner die PS5 steuern, während das Bild per Remote Play (chiaki-ng) kommt.

Remote Play überträgt nur Controller-Eingaben. Darum läuft der Weg für Maus und Tastatur getrennt über ein eigenes Board am USB-Port der PS5:

1. Die Desktop-App liest Maus und Tastatur per evdev und greift sie exklusiv.
2. Sie verschlüsselt die Eingaben (ChaCha20-Poly1305) und schickt sie per UDP übers LAN.
3. Das Board (WIZnet W5500-EVB-Pico2) prüft und entschlüsselt sie und gibt sie als USB-HID-Report an die PS5 weiter.
4. Die PS5 sieht eine normale Tastatur-Maus-Kombination. Der Controller läuft weiter über Remote Play.

Das Paketformat steht in [protocol/SPEC.md](protocol/SPEC.md).

## Stand

| Teil | Stand |
|---|---|
| Repository, Docker-Builds | ✅ |
| Firmware: USB-HID | ✅ (Host-Tests; Hardware-Test steht aus) |
| Firmware: Netzwerk, DHCP, UDP | ✅ (Simulator; Hardware-Test steht aus) |
| Protokoll mit Verschlüsselung | – |
| Kommandozeilen-Tool | – |
| Weboberfläche des Boards | – |
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

### Netzwerk

Das Board holt sich per DHCP eine Adresse und meldet sich als `ps5-hid-bridge-XXXXXX` (die letzten sechs Stellen der MAC-Adresse). Unter diesem Namen steht es im Router bzw. in UniFi. Eingaben empfängt es auf UDP-Port 45045.

LED: blinkt langsam = kein USB-Host, blinkt schnell = USB da, aber noch keine IP, leuchtet = bereit (flackert kurz bei Eingaben).

### Ohne Board testen

`make sim` startet einen Board-Simulator auf UDP-Port 45045. Er nutzt denselben Kern wie die Firmware und gibt statt USB-Reports Textzeilen aus (`KBD …`, `MOUSE …`). `make test` prüft damit den ganzen Eingabeweg automatisch.

Testverkehr an ein Board oder an den Simulator schicken:

```sh
make udp-test IP=192.168.1.50 ARGS='--plain ping'
make udp-test IP=192.168.1.50 ARGS='--plain pattern'        # Maus-Quadrat, tippt „hallo“
make udp-test IP=192.168.1.50 ARGS='--plain flood --rate 1000 --seconds 5'
```

`--plain` funktioniert nur mit dem Debug-Image `out/ps5-hid-bridge-plain.uf2`, das unverschlüsselte Pakete annimmt. Dieses Image nur zur Inbetriebnahme verwenden.

### Testimages ohne Netzwerk

| Datei | Zweck |
|---|---|
| `out/ps5-hid-bridge-hidtest.uf2` | 5 s nach dem Einstecken fährt die Maus langsam ein Quadrat (16-Bit-Maus). |
| `out/ps5-hid-bridge-hidtest-mouse8.uf2` | dasselbe mit 8-Bit-Mausachsen, falls ein Host den 16-Bit-Deskriptor nicht mag. |

Flashen: BOOTSEL gedrückt halten, Board an den Rechner stecken, UF2 auf das Laufwerk `RP2350` kopieren. Die LED blinkt, solange kein USB-Host das Board eingebunden hat, und leuchtet dauerhaft, sobald es eingebunden ist. Konsole: UART0 (GP0/GP1, 115200 Baud).

Die USB-Kennung `1209:0001` ist die Test-PID von pid.codes und nur für den privaten Gebrauch gedacht.
