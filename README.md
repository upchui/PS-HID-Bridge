# PS HID Bridge

Control a PlayStation with the mouse and keyboard of a Linux PC while the picture comes in via Remote Play (chiaki-ng).

Remote Play only carries controller input. Mouse and keyboard therefore take a separate path through a small board on the PlayStation's USB port:

1. The desktop app reads mouse and keyboard via evdev and grabs them exclusively.
2. It encrypts the input (ChaCha20-Poly1305) and sends it over the LAN via UDP.
3. The board (WIZnet W5500-EVB-Pico2) verifies and decrypts it and passes it on to the PlayStation as USB HID reports.
4. The PlayStation sees an ordinary keyboard and mouse. The controller keeps working through Remote Play.

The packet format is described in [protocol/SPEC.md](protocol/SPEC.md). If no valid packet arrives for 250 ms, the board releases all keys (failsafe).

## Status

All parts are implemented and tested automatically: unit tests, shared protocol test vectors and a board simulator that runs the real firmware core on the PC. **Nothing has been checked on real hardware yet**; the checklist for that is under "Bring-up on real hardware" below.

| Part | Status |
|---|---|
| Firmware: USB HID (keyboard, mouse with 5 buttons, both wheels) | ✅ host tests |
| Firmware: network (W5500, DHCP/static IP), encrypted protocol, failsafe | ✅ simulator |
| Board web interface (password, key, network, update) | ✅ simulator |
| Firmware update over the network (A/B with fallback) | ✅ simulator; the boot ROM part can only be checked on hardware |
| Command line tool `pshb-cli` | ✅ |
| Desktop app (Kirigami, setup wizard, triggers, tray) | ✅ offscreen self-test |

## Download

Every commit on `main` is built and tested automatically and published as a pre-release on the [releases page](https://github.com/upchui/PS-HID-Bridge/releases), named after its version, e.g. `v0.9.16`. Each release contains the AppImage, the command line tool (`pshb-cli-x86_64-linux`), all firmware images and `SHA256SUMS`. The last part of the version is the number of commits, so every commit has a higher version than the one before; the app, `pshb-cli --version` and the firmware (web interface, "Status" page of the app) all show the same version.

## Quick start

1. **Get the files:** download them from the latest release, or build them yourself with `make submodules && make all` (see "Building"; the first run takes long because of Qt).
2. **Flash the board:** hold BOOTSEL and plug the board into the PC via USB; the drive `RP2350` appears. First copy `out/ps-hid-bridge-pt.uf2` onto it (partition table; the board comes back as a drive afterwards), then `out/ps-hid-bridge.uf2`. Or let the app do it (setup wizard or "Firmware & updates" → "Install via USB") or `pshb-cli flash` (see "Flashing over USB from the PC").
3. **Connect the board:** network cable, USB to the PlayStation. Look up the IP in your router (`ps-hid-bridge-XXXXXX`) and open it in the browser.
4. **Web interface:** log in with `ps-hid-bridge`, set your own password, generate a key under "Security".
5. **App:** start `out/PS-HID-Bridge-x86_64.AppImage`. The wizard asks for the board address and key, the devices, the keyboard type and the trigger.
6. **PlayStation:** set the keyboard language to the layout of your keyboard (see "Keyboard layout") and switch on USB power in rest mode in the power saving settings.

## Desktop app

`out/PS-HID-Bridge-x86_64.AppImage` runs on current Linux systems (built against glibc 2.34) under Wayland and X11. It follows the desktop's light or dark mode; the switch at the bottom of the sidebar picks light, automatic or dark. Changes are saved as soon as they are valid, there is no "Save" button.

- **Wizard** on first start: board (address, key, connection test; or first install the firmware over USB) → devices and keyboard type → trigger → live test that shows what reaches the PlayStation. "Run setup again" in the sidebar starts it again.
- **Status:** bridge on/off, the keys that currently go to the PlayStation, tiles for the board, the PlayStation (connected/rest mode), the round trip to the board (median, 95th percentile, history) and the path from input to network, plus "Release all keys", "Pause" and "Reconnect".
- **Input:** keyboards and mice, grouped per device, selected ones first; "Identify" marks the device you are typing on, "Set up access" appears for devices the user may not read. The keyboard type (ISO or ANSI), and mouse sensitivity and acceleration with a curve preview (the default is the raw values).
- **Triggers:** key combination, mouse button (`Mouse4`, `Mouse5`), *Hold* (bridge on only while pressed) and *Double tap*; each with the action Toggle, Switch on, Switch off, Pause or Release all. "Record" takes over the combination you press. Also the window rule (see below).
- **Board:** address and key, connection state, "Open web interface".
- **Firmware & updates:** installed and newest version of app and board firmware, "Check now", the automatic check (at start and every six hours), and "Install via USB" (see "Flashing over USB from the PC").
- **Tray icon:** grey = board not reachable, blue = ready, green = bridge on. A click toggles. A Plasma notification appears when the bridge switches.
- **Updates:** when a new release is out, the Status page shows a banner with "Update app" (the AppImage replaces itself, then "Restart now") and "Update board" (firmware over the network, signed with the key; the bridge must be off). See "Updates from the app and the command line".
- **Emergency stop:** `Ctrl+Alt+Shift+Esc` always releases mouse and keyboard at once and lets go of everything on the PlayStation. This combination cannot be switched off.

The bridge only switches on once all keys are released: if the devices were grabbed while a key is down, that key would stay stuck on the desktop. *Hold* triggers are the exception, because their key is down by definition: the desktop does not see that key (or any other key held at that moment) being released, so it considers it pressed until it is pressed again. A mouse button (`Mouse4`) is therefore a better hold key than a keyboard key. The emergency stop and "Switch off" end a hold until its key is released. If the app hangs, a watchdog releases the devices after one second.

Settings: `~/.config/ps-hid-bridge/config.toml`, key: `~/.config/ps-hid-bridge/psk` (readable by the user only). The project used to be called PS5 HID Bridge: settings, key and packet counter under the old name (`~/.config/ps5-hid-bridge`, `~/.local/state/ps5-hid-bridge`) move to the new folders on the first start, and "Set up access" replaces the old udev rule file.

### Keyboard layout

The board sends key **positions**, not characters, just like a real USB keyboard. The PlayStation turns them into characters with its own keyboard language setting. So:

- **On the PlayStation**, set the keyboard language in the keyboard settings to the layout printed on your keyboard, e.g. German for a German keyboard.
- **In the app**, choose the physical keyboard type under "Input" (or in the wizard):

| Type | Typical layouts | Recognisable by |
|---|---|---|
| ISO (default) | German, UK, French, Nordic … (105 keys) | L-shaped Enter key, extra key next to the left Shift |
| ANSI | US (104 keys) | wide single-row Enter key, backslash key above Enter |

The type only matters for the key next to Enter (`#'` on a German keyboard, `\|` on a US keyboard): ISO sends HID code 0x32, ANSI 0x31. Every other key is the same for both types.

With the command line tool: `pshb-cli config set --keyboard iso` or `--keyboard ansi`.

### Window rule

The window rule switches the bridge on by itself while a certain window has the focus, and off again when the focus moves elsewhere. It needs KDE Plasma (a small KWin script reports the active window).

- **Off by default.** Switch it on under "Triggers" → "Window rule".
- The pattern is matched against the window title, the program (window class) and the desktop file name, ignoring case. The default `chiaki-ng` matches the Remote Play window of chiaki-ng.
- "Pause" suspends the rule until the next window change; "Switch off" overrides it while the window keeps the focus.

With the command line tool (the setting is used by the desktop app): `pshb-cli config set --window-rule-enabled on --window-rule chiaki-ng`.

### Device access

A normal user may not read keyboard and mouse raw. In the wizard the app sets up a udev rule **only for the selected devices** (it asks once for the administrator password):

```
# /etc/udev/rules.d/70-ps-hid-bridge.rules
SUBSYSTEM=="input", KERNEL=="event*", ATTRS{id/vendor}=="046d", ATTRS{id/product}=="c52b", TAG+="uaccess"
```

By hand: `pshb-cli list-devices` shows vendor and product ID in the "ID" column; afterwards run `sudo udevadm control --reload && sudo udevadm trigger -s input`. The file name must start with a number below 73, otherwise `uaccess` has no effect. Alternative: `sudo usermod -aG input $USER` and log in again (access to all input devices).

## Command line tool

`out/pshb-cli` does the same without a user interface (and without the window rule).

```sh
pshb-cli list-devices                              # keyboards and mice with access status
pshb-cli config set --board 192.168.1.50 --device 046d:c52b --device "G Pro"
pshb-cli config set --keyboard ansi                # physical keyboard type (iso or ansi)
pshb-cli config set-psk <64 hex digits>            # store the key
pshb-cli config show                               # current settings
pshb-cli ping                                      # connection and board status
pshb-cli run                                       # forward; toggle with Ctrl+Alt+P
pshb-cli run --stats                               # also print the input latency every 5 s
pshb-cli update --check                            # installed and newest versions (also of the board)
pshb-cli update                                    # replace pshb-cli with the newest release
pshb-cli update --firmware                         # install the newest firmware on the board
pshb-cli update --firmware --file ps-hid-bridge.uf2  # ... or a file
pshb-cli flash                                     # first installation over USB (BOOTSEL), see below
pshb-cli flash --image erase                       # recovery images: firmware, hidtest, hidtest-mouse8, erase
```

Key names in combinations follow the **position on a US keyboard** (`Z` is the `Y` key on a German keyboard). Modifiers: `Ctrl`, `Alt`, `AltGr`, `Shift`, `Super`; mouse buttons: `Mouse4`, `Mouse5`, `MouseMiddle`. The German names of earlier versions (`Strg`, `Umschalt`, `Maus4` …) are still accepted.

`pshb-cli replay FILE` plays input from a text file through the same logic as real devices (`make test` uses this against the simulator):

```
combo Ctrl+Alt+P      # press and release the combination
wait 30               # milliseconds
key LEFTSHIFT down
tap H
key LEFTSHIFT up
move 30 -10           # mouse
wheel 1
button BTN_LEFT down
focus on              # window rule: the window has the focus
```

## Board

### Web interface

At `http://<board IP>/`, light or dark like the browser. The tab is part of the address (`#status`, `#network`, `#security`, `#system`), so a reload or a bookmark opens it directly.

- **Login:** factory password `ps-hid-bridge`. On the first login you must set your own password (8–64 characters). After three failed attempts the waiting time grows up to 64 seconds.
- **Status:** state of the PlayStation connection and input, tiles for network, firmware and slot, and latency, packet counters; more under "Details".
- **Network:** DHCP or static address, UDP port. Saving restarts the board.
- **Security:** generate a pre-shared key (shown exactly once) or enter an existing one, sender filter (one address such as `192.168.1.20` or a subnet such as `192.168.1.0/24`), change the password.
- **System:** firmware update, restart, factory reset.

The web interface uses unencrypted HTTP in the local network. The password protects against accidental changes and against other devices in the network, not against someone who can capture the network traffic. The input itself is always encrypted.

### Network and LED

The board gets an address via DHCP and registers as `ps-hid-bridge-XXXXXX` (last six digits of the MAC address). It receives input on UDP port 45045.

LED: blinks slowly = no USB host, blinks fast = USB present but no IP yet, steady = ready (flickers briefly on input).

### Firmware update over the network

Web interface → System → upload `ps-hid-bridge.uf2`. The board writes the new firmware into the second slot (A/B), checks checksum and content and starts it "on trial". Only when it has started by itself and reaches the network chip is it accepted permanently. If it does not start properly, the board returns to the previous version after a restart. Every new firmware carries a higher version (its last part is the number of commits); the boot ROM always starts the newer one.

This needs the partition table from the first installation (`ps-hid-bridge-pt.uf2`).

The System page shows when a newer firmware is available (your browser asks GitHub; the board itself cannot reach the internet) and links the file.

### Updates from the app and the command line

The desktop app and `pshb-cli update` install the newest release themselves (see "Command line tool"; for the app: section "Updates" on the board page):

- **Where from:** the releases of [upchui/PS-HID-Bridge](https://github.com/upchui/PS-HID-Bridge/releases), asked at `api.github.com` over HTTPS. Every download is checked against the release's `SHA256SUMS` before it is used.
- **App and CLI** replace their own file (written next to it, then renamed over it).
- **Board firmware** goes to the board over HTTP, signed with the pre-shared key instead of the web password: the board hands out a one-time challenge, and the app sends a MAC over the challenge and the whole file (see [SPEC.md](protocol/SPEC.md), "Firmware update with the key"). Then it waits until the board runs the new version; if the board returned to the old one, it says so.
- The app looks for updates at start and every six hours and only installs on a click; `pshb-cli config set --update-check off` (or the switch in the app) turns the check off.

### Flashing over USB from the PC

For the first installation or for recovery, the app ("Firmware & updates" → "Install via USB", also offered on the first page of the setup wizard) and `pshb-cli flash` do what is described under "Quick start" by hand: hold BOOTSEL while connecting the board to the PC via USB, and the board shows up as the drive `RP2350`. The program finds that drive through UDisks2 (the service file managers use; no root needed), mounts it if necessary, checks `INFO_UF2.TXT`, copies the files and waits for the board to take them:

- **First installation** (`--image first-install`, the default): partition table, then — after the board came back as a drive — the firmware.
- **Firmware only**, the **HID test images** and **Erase everything** (`--image firmware`, `hidtest`, `hidtest-mouse8`, `erase`): see "Other images". Erasing asks first (in the CLI unless `--yes`).
- The files come from the newest release (checked against `SHA256SUMS`); "Choose file …" in the app or `--file X.uf2` uses your own file instead (for the first installation it replaces the firmware, the partition table still comes from the release). Only UF2 files for the RP2350 are accepted.

If the board is not found: use a USB cable with data lines, hold BOOTSEL *while* plugging in, and check that `udisks2` runs (`systemctl status udisks2`).

### Factory reset and recovery

- **Factory reset:** in the web interface, or hold the BOOTSEL button for 5 seconds while the board is running. Password, key and network settings are erased.
- **Reflashing always works:** hold BOOTSEL while plugging in and copy a UF2 file.
- **Erase everything:** copy `out/ps-hid-bridge-nuke.uf2` via BOOTSEL. It erases the whole flash and returns to the drive; then install partition table and firmware again.

### Other images

The app and `pshb-cli flash` can put the firmware, the HID test images and the erase image on the board over USB (see above).

| File | Purpose |
|---|---|
| `ps-hid-bridge.uf2` | normal firmware |
| `ps-hid-bridge-pt.uf2` | partition table (once, before the first firmware) |
| `ps-hid-bridge-hidtest.uf2` | no network: 5 s after plugging in, the mouse moves in a square |
| `ps-hid-bridge-hidtest-mouse8.uf2` | the same with 8-bit mouse axes, in case a host dislikes the 16-bit descriptor |
| `ps-hid-bridge-plain.uf2` | **bring-up only:** also accepts unencrypted packets |
| `ps-hid-bridge-rollbacktest.uf2` | **testing only:** never confirms itself after an update, the board must return to the previous version |
| `ps-hid-bridge-nuke.uf2` | erases the whole flash |

Console: UART0 (GP0/GP1, 115200 baud). The USB ID `1209:0001` is the pid.codes test PID and meant for private use only.

## Latency

The input path takes about 1–2 ms from a key press or mouse movement to the PlayStation; the video path (PlayStation encoder → network → chiaki-ng → display) takes tens of milliseconds. The input path is built to stay short even when the PC is busy:

- **App:** one thread waits on all selected devices, runs the trigger logic and sends the packet itself, without handing the event to another thread. It runs with real-time priority (through rtkit, which desktops with PipeWire usually have, or directly if the user may use `SCHED_FIFO`, e.g. via a `realtime` group). If neither works, the status page says "normal priority".
- **Network:** packets are marked DSCP EF, so Wi-Fi sends them in the voice queue. A cable is still better: connect the PC and the PlayStation by cable and the board to the same switch.
- **Board:** core 0 handles nothing but USB and the input socket; network management and the web interface run on core 1 and use the SPI bus as little as possible.

Measuring:

- App, status page: "Input → network" (kernel time of the event until the packet is sent), median, 95 % and maximum. On the command line: `pshb-cli run --stats`.
- Board, web interface (Status): "Latency" with the time from a packet to the report fetched by the PlayStation, the receive and decrypt time, and how often mouse motion had to wait for the previous report.
- `tools/latency_bench.py --cli out/pshb-cli --vendor VVVV --product PPPP [--load 16]` measures the app from outside: it creates a virtual mouse via `/dev/uinput`, plays the board and reports percentiles, optionally with busy processes on every core. It needs write access to `/dev/uinput` and read access to the virtual device (`--vendor`/`--product` of a device that your udev rule covers). Typical on a desktop with real-time priority: median about 0.1 ms, 99 % below 0.3 ms, also with every core busy.

## Bring-up on real hardware

These points cannot be checked without a board. The order is chosen so that each step builds on the previous one:

1. `picotool info -a` (BOOTSEL mode): RP2350A, 2 MB flash.
2. `ps-hid-bridge-hidtest.uf2` on the PC: `lsusb -v` (1209:0001, two HID interfaces), `evtest` (the mouse moves in a square), polling rate 1000 Hz (`usbhid-dump` / usbmon).
3. The same image on the PlayStation: mouse in the menu and in a game with mouse support, also during a running Remote Play session (Aimlabs). If the mouse does not react: try `-hidtest-mouse8.uf2`.
4. Partition table + `ps-hid-bridge-plain.uf2`: DHCP, host name in the router, `make udp-test IP=… ARGS='--plain flood --rate 1000 --seconds 10'` without losses; test SPI clock 37.5 MHz (`PSHB_W5500_SPI_HZ`, i.e. clk_peri / 4).
5. Waiting time on core 0 while the web interface loads (GPIO pulse + logic analyser, or the maximum of "Receive and decrypt" on the Latency card): should stay below 0.5 ms.
6. Normal firmware, key via the web interface, connect the app; check the failsafe (pull the network cable while a key is pressed → released after 250 ms).
7. Update over the network with `ps-hid-bridge.uf2` (slot changes from A to B), then with `ps-hid-bridge-rollbacktest.uf2`: this image deliberately never confirms itself; after about 17 seconds the board must restart by itself with the previous version (check slot and firmware in the web interface).
8. Settings survive several updates (they live outside the firmware slots).
9. Latency card in the web interface while moving the mouse in a game: "Receive and decrypt" should stay well below 0.1 ms; "Packet → report fetched" around 0.5–1 ms. Compare with a build using `make firmware FW_CMAKE_ARGS=-DPSHB_W5500_FAST_RX=OFF` (plain ioLibrary receive). Note "Mouse motion that had to wait": a high count means reports should rather be filled right before the PlayStation fetches them instead of as early as possible — that can save up to 1 ms with continuous motion. Also check after the PlayStation wakes from rest mode that no old keystrokes arrive, and that logging in to the web interface works (it needs the larger core 1 stack).
10. First installation from the app ("Firmware & updates" → "Install via USB" → "First installation") on a board that still has no firmware, and "Erase everything" followed by another first installation.
11. Firmware update from the app: bridge off, "Update board" on the Status page (or `pshb-cli update --firmware --file out/ps-hid-bridge.uf2`). The board restarts and the app then shows the new firmware version. A wrong key must be refused ("Signature does not match").

## Building

Everything is built in Docker. The host only needs `docker`, `make` and `git`.

```sh
make submodules        # Pico SDK, TinyUSB, ioLibrary, Monocypher (pinned, not recursive)
make firmware          # -> out/*.uf2
make desktop-cli       # -> out/pshb-cli
make desktop           # -> out/PS-HID-Bridge-x86_64.AppImage
make test              # unit tests, test vectors, simulator, Rust tests
make all               # everything, including the AppImage self-test (tests first)
make -j2 -O all        # the same, two steps at a time (faster)
```

The first `make desktop` builds Qt 6.12 and Kirigami from source and therefore takes long (1–3 hours depending on the machine). After that everything is in the Docker cache. The number of parallel Qt jobs can be set with `make desktop QT_JOBS=4`; qtdeclarative needs about 2 GB of RAM per job.

All versions are kept in [versions.env](versions.env); downloads are checked by SHA-256. `make check-pins` checks that the submodules are at the tags listed there (and that `desktop/rust-toolchain.toml` matches `RUST_VERSION`).

For development builds without the web interface a key can be compiled in: `make firmware DEV_PSK=<64 hex digits>`. It only applies as long as no key is set in the web interface.

### Continuous integration

GitHub Actions builds on GitHub-hosted runners with the same Makefile:

- [`release.yml`](.github/workflows/release.yml) runs on every push to `main`: `make -j2 -O all` (every test, firmware, CLI, AppImage, AppImage self-test), then it publishes the pre-release `v<major>.<minor>.<commit count>`. Compiled Rust dependencies are cached per `Cargo.lock`. A release whose tag already exists for another commit (after a force-push) is refused.
- The toolchain images are stored in the GitHub Container Registry (`ghcr.io/<owner>/pshb-*`), tagged with a hash of `versions.env` and `docker/` (`make print-toolchain-tag`). Only when that hash changes does [`toolchain.yml`](.github/workflows/toolchain.yml) rebuild them; with Qt this takes a few hours, a normal release run takes a few minutes.
- `make PREBUILT=1 FW_IMAGE=… RUST_IMAGE=… DESKTOP_IMAGE=… APPTEST_IMAGE=… all` is what CI runs: the image targets only check that the pulled images exist.
- The version is `MAJOR.MINOR` from the file [`VERSION`](VERSION) plus the number of commits ([`tools/version.sh`](tools/version.sh), `make print-version`). Firmware, app, CLI and metainfo get it at build time; for a bigger step (e.g. `1.0`) only `VERSION` is edited.

## Testing without a board

`make sim` starts a board simulator: input on UDP port 45045, web interface at http://localhost:8080. It uses the same core as the firmware and prints text lines instead of USB reports (`KBD …`, `MOUSE …`); its settings are kept in `.cache/sim/flash.bin`. Connect the app or `pshb-cli` with the address `127.0.0.1`.

Test traffic to a board or to the simulator:

```sh
make udp-test IP=192.168.1.50 ARGS='--psk-file psk.txt ping'
make udp-test IP=192.168.1.50 ARGS='--psk-file psk.txt pattern'   # mouse square, types "hello"
make udp-test IP=192.168.1.50 ARGS='--psk-file psk.txt flood --rate 1000 --seconds 5'
```

Grabbing and releasing the devices is tested against virtual devices (uinput) that look like a keyboard converter and a gaming mouse with several event nodes each: the bridge service is switched on and off in many ways (Ctrl+Alt+P split over two nodes, hot-plug and service restart while on, a device held by another program), and after each step a second descriptor checks whether a node is still grabbed. It needs write access to `/dev/uinput`, and read access to the virtual devices: they use the IDs of the udev rule written by "Set up access" (default `feed:6536,0db0:1fc6`, otherwise `PSHB_GRAB_TEST_IDS=vvvv:pppp,vvvv:pppp`). Close the app first, it would pick up the virtual devices too. Runs on the host, not in Docker:

```sh
cd desktop && cargo test -p pshb-core --test grab_release -- --ignored --test-threads=1
```

## Directories

| Path | Content |
|---|---|
| `firmware/src/core/` | portable firmware core (protocol, input, settings, web server, update), also runs on the PC |
| `firmware/src/pico/` | RP2350 specific: USB, W5500, flash, boot ROM |
| `firmware/src/web/` | the board's web interface (embedded) |
| `firmware/host/` | host build: unit tests and board simulator |
| `desktop/core/` | Rust core (evdev, triggers, crypto, UDP) and `pshb-cli` |
| `desktop/ui/` | Kirigami user interface (QML, cxx-qt), tray, window rule |
| `protocol/` | packet format and shared test vectors |
| `docker/` | build environments |
| `tools/` | test tools (UDP test, simulator integration tests, latency benchmark), version and release helpers |
| `.github/workflows/` | CI: toolchain images and automatic releases |
