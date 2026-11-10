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
| Firmware: USB-HID | – |
| Firmware: Netzwerk, DHCP, UDP | – |
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

Die USB-Kennung `1209:0001` ist die Test-PID von pid.codes und nur für den privaten Gebrauch gedacht.
