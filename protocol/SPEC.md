# PS5 HID Bridge – Protokoll v1

Dieses Dokument beschreibt die UDP-Pakete zwischen Desktop-App und Board. Es gibt drei Implementierungen:

- Firmware in C (`firmware/src/core/`)
- Desktop-Kern in Rust (`desktop/core/src/protocol.rs`)
- Python-Referenz (`protocol/tools/pshb_proto.py`)

Alle drei prüfen sich gegen die gemeinsamen Testvektoren in `protocol/vectors/`.

## Überblick

- Transport: UDP, Board lauscht auf Port **45045** (einstellbar).
- Jedes Paket ist mit **ChaCha20-Poly1305** (RFC 8439) und einem **Pre-Shared Key** (256 Bit) verschlüsselt und signiert.
- Alle Zahlen sind **Little Endian**.
- Pakete mit falscher Signatur, alter Sequenznummer, falscher Session oder fremder Absender-IP verwirft das Board stillschweigend und zählt sie nur mit.

## Paketaufbau

| Offset | Größe | Feld | Inhalt |
|---|---|---|---|
| 0 | 1 | Version | `0x01` |
| 1 | 1 | Typ | siehe unten |
| 2 | 8 | Zähler | u64, Sequenznummer des Absenders |
| 10 | n | Nutzdaten | verschlüsselt, Länge je nach Typ |
| 10+n | 16 | Tag | Poly1305 |

Die ersten 10 Bytes (Version, Typ, Zähler) sind unverschlüsselt und gehen als **Associated Data** in die Signatur ein. Sie lassen sich also nicht verändern, ohne dass die Prüfung fehlschlägt.

| Typ | Name | Richtung | Nutzdaten | Paketlänge |
|---|---|---|---|---|
| `0x01` | INPUT | App → Board | 23 B | 49 B |
| `0x02` | HEARTBEAT | App → Board | 23 B | 49 B |
| `0x03` | PING | App → Board | 12 B | 38 B |
| `0x04` | PONG | Board → App | 49 B | 75 B |

### Nonce

Die 12-Byte-Nonce wird aus Richtung und Zähler gebildet:

| Offset | Größe | Inhalt |
|---|---|---|
| 0 | 4 | Richtung, u32: `1` = App → Board, `2` = Board → App |
| 4 | 8 | Zähler aus dem Header, u64 |

Beide Richtungen nutzen denselben Schlüssel. Durch die Richtung in der Nonce können sich die Nonces von App und Board nie überschneiden.

**Eine Nonce darf mit demselben Schlüssel niemals zweimal verwendet werden.** Deshalb gilt:

- **App:** Zähler = `max(letzter + 1, aktuelle Unix-Zeit in µs, gespeicherte Untergrenze)`. Die Untergrenze wird regelmäßig auf `Zähler + 60 s` gesetzt und gespeichert. So bleibt der Zähler auch nach einem Neustart der App oder einer zurückgestellten Uhr eindeutig.
- **Board:** Zähler startet bei jedem Booten mit einem Zufallswert aus dem TRNG (auf 62 Bit begrenzt) und zählt dann hoch.

## Nutzdaten

### INPUT und HEARTBEAT (23 Bytes)

| Offset | Größe | Feld | Inhalt |
|---|---|---|---|
| 0 | 8 | session_id | u64, Session des Boards (aus PONG) |
| 8 | 1 | flags | Bit 0 = RELEASE_ALL: alles loslassen, übrige Felder ignorieren |
| 9 | 1 | mods | Modifikator-Bits wie im HID-Tastatur-Report (Bit 0 = Strg links … Bit 7 = GUI rechts) |
| 10 | 6 | keys | bis zu 6 gedrückte Tasten, HID-Usage-Page 0x07, unbenutzt = 0; bei mehr als 6 Tasten alle sechs = `0x01` (ErrorRollOver) |
| 16 | 1 | buttons | Maustasten: Bit 0 links, 1 rechts, 2 Mitte, 3 zurück, 4 vor |
| 17 | 2 | dx | i16, relative Mausbewegung |
| 19 | 2 | dy | i16 |
| 21 | 1 | wheel | i8, Mausrad vertikal |
| 22 | 1 | pan | i8, Mausrad horizontal |

- Tastatur und Maustasten sind **absoluter Zustand**: Jedes Paket enthält alle gerade gedrückten Tasten. Ein verlorenes Paket korrigiert sich mit dem nächsten.
- Bewegung und Mausrad sind **relativ**: Das Board addiert sie auf und gibt sie über die USB-Reports aus, ohne etwas zu verlieren.
- HEARTBEAT hat denselben Aufbau. Die App schickt ihn, solange die Steuerung aktiv ist und gerade nichts anderes gesendet wird (alle 50 ms). Er trägt den vollen Tastenzustand und `dx = dy = wheel = pan = 0`.

### PING (12 Bytes)

| Offset | Größe | Feld | Inhalt |
|---|---|---|---|
| 0 | 8 | t_app_us | u64, Sendezeit der App (beliebige monotone Uhr) |
| 8 | 4 | seq | u32, laufende Nummer |

PING braucht keine Session-ID. Die App erfährt die Session erst aus der Antwort.

### PONG (49 Bytes)

| Offset | Größe | Feld | Inhalt |
|---|---|---|---|
| 0 | 8 | t_app_us | Echo aus PING (0 bei Session-Hinweis, siehe unten) |
| 8 | 4 | seq | Echo aus PING |
| 12 | 8 | session_id | aktuelle Session des Boards |
| 20 | 4 | fw_version | `0x00MMmmpp` (Major, Minor, Patch) |
| 24 | 4 | uptime_ms | Laufzeit seit dem Start |
| 28 | 1 | status | Bit 0 USB verbunden, Bit 1 USB im Ruhemodus, Bit 2 Failsafe aktiv, Bit 7 unverschlüsselte Debug-Firmware |
| 29 | 4 | rx_ok | gültige Pakete |
| 33 | 4 | rx_auth_fail | falsche Signatur oder kein Schlüssel gesetzt |
| 37 | 4 | rx_replay | Zähler nicht größer als der letzte oder falsche Session |
| 41 | 4 | rx_filtered | fremde Absender-IP |
| 45 | 4 | rx_malformed | falsche Länge, Version oder unbekannter Typ |

## Ablauf im Board

Für jedes empfangene Paket prüft das Board in dieser Reihenfolge:

1. Länge mindestens Header + Tag, Version `0x01`, Typ INPUT, HEARTBEAT oder PING, Länge passt zum Typ. Sonst `rx_malformed`.
2. Absender-IP erlaubt (falls ein Filter eingestellt ist). Sonst `rx_filtered`.
3. `Zähler > letzter gültiger Zähler`. Sonst `rx_replay`. Diese Prüfung ist billig und kommt deshalb vor der Kryptografie.
4. Tag prüfen und entschlüsseln. Sonst `rx_auth_fail`.
5. Erst jetzt: `letzter gültiger Zähler = Zähler`. Ein gefälschtes Paket kann den Zähler also nicht hochtreiben.
6. INPUT/HEARTBEAT: `session_id` muss der aktuellen Session entsprechen. Sonst `rx_replay`; das Board schickt höchstens alle 100 ms ein PONG mit `t_app_us = seq = 0` als **Session-Hinweis** zurück, damit die App die neue Session sofort kennt.
7. PING: PONG an den Absender (IP und Port aus dem empfangenen Paket).

Die **Session-ID** erzeugt das Board bei jedem Start neu aus dem TRNG. Nach einem Neustart sind dadurch alle früher aufgezeichneten Eingabepakete wertlos, auch wenn der Zählerstand des Boards wieder bei null beginnt.

## Failsafe

- Kommt **250 ms** lang kein gültiges INPUT oder HEARTBEAT, lässt das Board alle Tasten und Maustasten los (Status-Bit 2).
- Dasselbe passiert, wenn die PS5 den USB-Port trennt oder in den Ruhemodus versetzt.
- Das nächste gültige Paket setzt den Zustand wieder.

## Ablauf in der App

1. PING senden, bis ein PONG kommt; daraus die `session_id` übernehmen.
2. Bei jeder Änderung ein INPUT senden, höchstens eines pro Millisekunde. Bewegungen innerhalb dieser Millisekunde werden zusammengefasst; was nicht in i16 passt, geht ins nächste Paket.
3. Solange die Steuerung aktiv ist: HEARTBEAT, wenn 50 ms lang nichts gesendet wurde.
4. Jede Sekunde PING (für die Latenzanzeige häufiger). Ändert sich die `session_id` in einem PONG, ab sofort die neue verwenden und den vollen Zustand erneut senden.
5. Beim Deaktivieren: INPUT mit `RELEASE_ALL`.

## Unverschlüsselter Debug-Modus (Version 0)

Nur das Debug-Image `ps5-hid-bridge-plain.uf2` und der Simulator mit `--plaintext` nehmen Pakete mit Version `0x00` an. Sie haben denselben Header und dieselben Nutzdaten, aber keine Verschlüsselung und keinen Tag. Zähler, Session und Absenderfilter werden in diesem Modus nicht geprüft. Antworten kommen ebenfalls als Version 0. Status-Bit 7 zeigt diesen Modus an.

## Testvektoren

`protocol/vectors/v1.json` erzeugt `protocol/tools/gen_vectors.py` mit festen Schlüsseln und Zählern:

- `aead_rfc8439`: der Testvektor aus RFC 8439, Abschnitt 2.8.2. Er beweist, dass die Bibliotheken das Standardverfahren umsetzen.
- `packets`: gültige Pakete aller Typen und Richtungen, mit den dekodierten Feldern.
- `invalid`: Pakete, die beim Öffnen scheitern müssen: falscher Tag, falscher Schlüssel, falsche Richtung, veränderter Header, zu kurz.

`make test` prüft, dass die eingecheckte Datei dem Generator entspricht und dass C, Rust und Python alle Vektoren identisch verarbeiten.
