# PS5 HID Bridge

Control a PS5 with the mouse and keyboard of a Linux PC while the picture comes in via Remote Play (chiaki-ng).

Remote Play only carries controller input. Mouse and keyboard therefore take a separate path through a small board on the PS5's USB port:

1. The desktop app reads mouse and keyboard via evdev and grabs them exclusively.
2. It encrypts the input (ChaCha20-Poly1305) and sends it over the LAN via UDP.
3. The board (WIZnet W5500-EVB-Pico2) verifies and decrypts it and passes it on to the PS5 as USB HID reports.
4. The PS5 sees an ordinary keyboard and mouse. The controller keeps working through Remote Play.

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
2. **Flash the board:** hold BOOTSEL and plug the board into the PC via USB; the drive `RP2350` appears. First copy `out/ps5-hid-bridge-pt.uf2` onto it (partition table; the board comes back as a drive afterwards), then `out/ps5-hid-bridge.uf2`.
3. **Connect the board:** network cable, USB to the PS5. Look up the IP in your router (`ps5-hid-bridge-XXXXXX`) and open it in the browser.
4. **Web interface:** log in with `ps5-hid-bridge`, set your own password, generate a key under "Security".
5. **App:** start `out/PS5-HID-Bridge-x86_64.AppImage`. The wizard asks for the board address and key, the devices, the keyboard type and the trigger.
6. **PS5:** set the keyboard language to the layout of your keyboard (see "Keyboard layout") and switch on USB power in rest mode in the power saving settings.

## Desktop app

`out/PS5-HID-Bridge-x86_64.AppImage` runs on current Linux systems (built against glibc 2.34) under Wayland and X11.

- **Wizard** on first start: board (address, key, connection test) → select devices and set up access, keyboard type → trigger → live test that shows what reaches the PS5.
- **Status:** bridge on/off, connection, PS5 connected/rest mode, latency to the board with median and 95th percentile plus history, packets per second.
- **Devices:** keyboards and mice, grouped per device. "Press a key to identify" marks the device you are typing on. Below the list: the keyboard type (ISO or ANSI).
- **Triggers:** key combination, mouse button (`Mouse4`, `Mouse5`), *Hold* (bridge on only while pressed) and *Double tap*; each with the action Toggle, Switch on, Switch off, Pause or Release all. "Record" takes over the combination you press.
- **Window rule** (KDE Plasma, off by default): see below.
- **Mouse:** sensitivity and acceleration with a curve preview; the default is the raw values.
- **Tray icon:** grey = board not reachable, blue = ready, green = bridge on. A click toggles. A Plasma notification appears when the bridge switches.
- **Emergency stop:** `Ctrl+Alt+Shift+Esc` always releases mouse and keyboard at once and lets go of everything on the PS5. This combination cannot be switched off.

The bridge only switches on once all keys are released: if the devices were grabbed while a key is down, that key would stay stuck on the desktop. If the app hangs, a watchdog releases the devices after one second.

Settings: `~/.config/ps5-hid-bridge/config.toml`, key: `~/.config/ps5-hid-bridge/psk` (readable by the user only).

### Keyboard layout

The board sends key **positions**, not characters, just like a real USB keyboard. The PS5 turns them into characters with its own keyboard language setting. So:

- **On the PS5**, set the keyboard language in the keyboard settings to the layout printed on your keyboard, e.g. German for a German keyboard.
- **In the app**, choose the physical keyboard type under "Devices" (or in the wizard):

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
# /etc/udev/rules.d/70-ps5-hid-bridge.rules
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

At `http://<board IP>/`:

- **Login:** factory password `ps5-hid-bridge`. On the first login you must set your own password (8–64 characters). After three failed attempts the waiting time grows up to 64 seconds.
- **Status:** connection to the PS5, network, firmware and slot, packet counters.
- **Network:** DHCP or static address, UDP port. Saving restarts the board.
- **Security:** generate a pre-shared key (shown exactly once) or enter an existing one, sender filter (one address such as `192.168.1.20` or a subnet such as `192.168.1.0/24`), change the password.
- **System:** firmware update, restart, factory reset.

The web interface uses unencrypted HTTP in the local network. The password protects against accidental changes and against other devices in the network, not against someone who can capture the network traffic. The input itself is always encrypted.

### Network and LED

The board gets an address via DHCP and registers as `ps5-hid-bridge-XXXXXX` (last six digits of the MAC address). It receives input on UDP port 45045.

LED: blinks slowly = no USB host, blinks fast = USB present but no IP yet, steady = ready (flickers briefly on input).

### Firmware update over the network

Web interface → System → upload `ps5-hid-bridge.uf2`. The board writes the new firmware into the second slot (A/B), checks checksum and content and starts it "on trial". Only when it has started by itself and reaches the network chip is it accepted permanently. If it does not start properly, the board returns to the previous version after a restart. Every new firmware carries a higher version (its last part is the number of commits); the boot ROM always starts the newer one.

This needs the partition table from the first installation (`ps5-hid-bridge-pt.uf2`).

### Factory reset and recovery

- **Factory reset:** in the web interface, or hold the BOOTSEL button for 5 seconds while the board is running. Password, key and network settings are erased.
- **Reflashing always works:** hold BOOTSEL while plugging in and copy a UF2 file.
- **Erase everything:** copy `out/ps5-hid-bridge-nuke.uf2` via BOOTSEL. It erases the whole flash and returns to the drive; then install partition table and firmware again.

### Other images

| File | Purpose |
|---|---|
| `ps5-hid-bridge.uf2` | normal firmware |
| `ps5-hid-bridge-pt.uf2` | partition table (once, before the first firmware) |
| `ps5-hid-bridge-hidtest.uf2` | no network: 5 s after plugging in, the mouse moves in a square |
| `ps5-hid-bridge-hidtest-mouse8.uf2` | the same with 8-bit mouse axes, in case a host dislikes the 16-bit descriptor |
| `ps5-hid-bridge-plain.uf2` | **bring-up only:** also accepts unencrypted packets |
| `ps5-hid-bridge-rollbacktest.uf2` | **testing only:** never confirms itself after an update, the board must return to the previous version |
| `ps5-hid-bridge-nuke.uf2` | erases the whole flash |

Console: UART0 (GP0/GP1, 115200 baud). The USB ID `1209:0001` is the pid.codes test PID and meant for private use only.

## Latency

The input path takes about 1–2 ms from a key press or mouse movement to the PS5; the video path (PS5 encoder → network → chiaki-ng → display) takes tens of milliseconds. The input path is built to stay short even when the PC is busy:

- **App:** one thread waits on all selected devices, runs the trigger logic and sends the packet itself, without handing the event to another thread. It runs with real-time priority (through rtkit, which desktops with PipeWire usually have, or directly if the user may use `SCHED_FIFO`, e.g. via a `realtime` group). If neither works, the status page says "normal priority".
- **Network:** packets are marked DSCP EF, so Wi-Fi sends them in the voice queue. A cable is still better: connect the PC and the PS5 by cable and the board to the same switch.
- **Board:** core 0 handles nothing but USB and the input socket; network management and the web interface run on core 1 and use the SPI bus as little as possible.

Measuring:

- App, status page: "Input → network" (kernel time of the event until the packet is sent), median, 95 % and maximum. On the command line: `pshb-cli run --stats`.
- Board, web interface (Status): "Latency" with the time from a packet to the report fetched by the PS5, the receive and decrypt time, and how often mouse motion had to wait for the previous report.
- `tools/latency_bench.py --cli out/pshb-cli --vendor VVVV --product PPPP [--load 16]` measures the app from outside: it creates a virtual mouse via `/dev/uinput`, plays the board and reports percentiles, optionally with busy processes on every core. It needs write access to `/dev/uinput` and read access to the virtual device (`--vendor`/`--product` of a device that your udev rule covers). Typical on a desktop with real-time priority: median about 0.1 ms, 99 % below 0.3 ms, also with every core busy.

## Bring-up on real hardware

These points cannot be checked without a board. The order is chosen so that each step builds on the previous one:

1. `picotool info -a` (BOOTSEL mode): RP2350A, 2 MB flash.
2. `ps5-hid-bridge-hidtest.uf2` on the PC: `lsusb -v` (1209:0001, two HID interfaces), `evtest` (the mouse moves in a square), polling rate 1000 Hz (`usbhid-dump` / usbmon).
3. The same image on the PS5: mouse in the menu and in a game with mouse support, also during a running Remote Play session (Aimlabs). If the mouse does not react: try `-hidtest-mouse8.uf2`.
4. Partition table + `ps5-hid-bridge-plain.uf2`: DHCP, host name in the router, `make udp-test IP=… ARGS='--plain flood --rate 1000 --seconds 10'` without losses; test SPI clock 37.5 MHz (`PSHB_W5500_SPI_HZ`, i.e. clk_peri / 4).
5. Waiting time on core 0 while the web interface loads (GPIO pulse + logic analyser, or the maximum of "Receive and decrypt" on the Latency card): should stay below 0.5 ms.
6. Normal firmware, key via the web interface, connect the app; check the failsafe (pull the network cable while a key is pressed → released after 250 ms).
7. Update over the network with `ps5-hid-bridge.uf2` (slot changes from A to B), then with `ps5-hid-bridge-rollbacktest.uf2`: this image deliberately never confirms itself; after about 17 seconds the board must restart by itself with the previous version (check slot and firmware in the web interface).
8. Settings survive several updates (they live outside the firmware slots).
9. Latency card in the web interface while moving the mouse in a game: "Receive and decrypt" should stay well below 0.1 ms; "Packet → report fetched" around 0.5–1 ms. Compare with a build using `make firmware FW_CMAKE_ARGS=-DPSHB_W5500_FAST_RX=OFF` (plain ioLibrary receive). Note "Fetch time within the USB frame" and "Mouse motion that had to wait": they decide whether reports should be filled right before the PS5 fetches them (filled at the start of the frame instead of as early as possible) — that can save up to 1 ms with continuous motion.

## Building

Everything is built in Docker. The host only needs `docker`, `make` and `git`.

```sh
make submodules        # Pico SDK, TinyUSB, ioLibrary, Monocypher (pinned, not recursive)
make firmware          # -> out/*.uf2
make desktop-cli       # -> out/pshb-cli
make desktop           # -> out/PS5-HID-Bridge-x86_64.AppImage
make test              # unit tests, test vectors, simulator, Rust tests
make all               # everything, including the AppImage self-test
```

The first `make desktop` builds Qt 6.12 and Kirigami from source and therefore takes long (1–3 hours depending on the machine). After that everything is in the Docker cache. The number of parallel Qt jobs can be set with `make desktop QT_JOBS=4`; qtdeclarative needs about 2 GB of RAM per job.

All versions are kept in [versions.env](versions.env); downloads are checked by SHA-256. `make check-pins` checks that the submodules are at the tags listed there.

For development builds without the web interface a key can be compiled in: `make firmware DEV_PSK=<64 hex digits>`. It only applies as long as no key is set in the web interface.

### Continuous integration

GitHub Actions builds on GitHub-hosted runners with the same Makefile:

- [`release.yml`](.github/workflows/release.yml) runs on every push to `main`: `make all` (firmware, CLI, AppImage, every test, AppImage self-test), then it publishes the pre-release `v<major>.<minor>.<commit count>`.
- The toolchain images are stored in the GitHub Container Registry (`ghcr.io/<owner>/pshb-*`), tagged with a hash of `versions.env` and `docker/` (`make print-toolchain-tag`). Only when that hash changes does [`toolchain.yml`](.github/workflows/toolchain.yml) rebuild them; with Qt this takes a few hours, a normal release run takes about 20 minutes.
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
