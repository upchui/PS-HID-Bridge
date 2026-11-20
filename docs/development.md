# Development

Test status, building, continuous integration, testing without a board, screenshots and the checks on real hardware. For installing and using PS HID Bridge, see the [README](../README.md).

## Status

**Version 1.0: works on real hardware** (WIZnet W5500-EVB-Pico2 on a PlayStation, desktop app on Linux). Besides that, every part is tested automatically on each commit: unit tests, shared protocol test vectors and a board simulator that runs the real firmware core on the PC. The checklist under [Checks on real hardware](#checks-on-real-hardware) below is the way to check a new board or a bigger change.

| Part | Status |
|---|---|
| Firmware: USB HID (keyboard, mouse with 5 buttons, both wheels) | ✅ real hardware, host tests |
| Firmware: network (W5500, DHCP/static IP), encrypted protocol, failsafe | ✅ real hardware, simulator |
| Board web interface (password, key, network, update) | ✅ real hardware, simulator |
| mDNS (`<name>.local`, DNS-SD) | simulator, host tests; real network still to check (checklist 12) |
| Firmware update over the network (A/B with fallback) | ✅ real hardware, simulator |
| Command line tool `pshb-cli` | ✅ |
| Desktop app (Kirigami, setup wizard, triggers, tray) | ✅ real hardware, offscreen self-test |
| Profiles, remapped keys, mouse options, window backends, typing text, pairing, diagnostics, German UI | unit tests, simulator, offscreen self-test in both languages; real hardware still to check (checklist 13–16) |

## Building

Everything is built in Docker. The host only needs `docker`, `make` and `git`.

```sh
make submodules        # Pico SDK, TinyUSB, ioLibrary, Monocypher (pinned, not recursive)
make firmware          # -> out/*.uf2
make desktop-cli       # -> out/pshb-cli
make desktop           # -> out/PS-HID-Bridge-x86_64.AppImage
make ARCH=aarch64 desktop-cli desktop   # ARM64: out/pshb-cli-aarch64, out/PS-HID-Bridge-aarch64.AppImage
make test              # unit tests, test vectors, simulator, Rust tests
make all               # everything, including the AppImage self-test (tests first)
make -j2 -O all        # the same, two steps at a time (faster)
```

The first `make desktop` builds Qt 6.12 and Kirigami from source and therefore takes long (1–3 hours depending on the machine). After that everything is in the Docker cache. The number of parallel Qt jobs can be set with `make desktop QT_JOBS=4`; qtdeclarative needs about 2 GB of RAM per job.

All versions are kept in [versions.env](../versions.env); downloads are checked by SHA-256. `make check-pins` checks that the submodules are at the tags listed there (and that `desktop/rust-toolchain.toml` matches `RUST_VERSION`).

### Building for ARM64 (aarch64)

`ARCH=aarch64` builds the desktop app and the CLI for ARM64 (`test-desktop`, `desktop-cli`, `desktop`, `test-appimage`, `ui-shots`); the firmware and its tests are the same for every PC and refuse it. The ARM images are called `pshb-rust-aarch64`, `pshb-desktop-aarch64` and `pshb-apptest-aarch64`, the Rust build has its own directory (`.cache/target/cargo-aarch64`).

- **On an ARM64 machine** it builds natively, as fast as on x86.
- **On an x86 PC** the ARM containers run under QEMU: install `qemu-user-static` and `qemu-user-static-binfmt` (Arch, CachyOS) or `qemu-user-static` and `binfmt-support` (Debian, Ubuntu); `docker buildx ls` then lists `linux/arm64`. Everything takes several times longer. `make ARCH=aarch64 image-rust test-desktop desktop-cli` (Rust tests and the CLI, no Qt) is fine; building Qt under emulation (`make ARCH=aarch64 image-desktop`) takes very long (likely more than ten hours). Use the images CI built instead: `make ARCH=aarch64 PREBUILT=1 DESKTOP_IMAGE=ghcr.io/upchui/pshb-desktop:<tag>-aarch64 APPTEST_IMAGE=ghcr.io/upchui/pshb-apptest:<tag>-aarch64 desktop test-appimage` (`<tag>`: `make print-toolchain-tag`; pull them first with `docker pull --platform linux/arm64 …`).

For development builds without the web interface a key can be compiled in: `make firmware DEV_PSK=<64 hex digits>`. It only applies as long as no key is set in the web interface.

### Continuous integration

GitHub Actions builds on GitHub-hosted runners with the same Makefile:

- [`release.yml`](../.github/workflows/release.yml) runs on every push to `main`: `make -j2 -O all` on x86_64 (every test, firmware, CLI, AppImage, AppImage self-test) and, at the same time on an ARM runner, `make ARCH=aarch64 … test-desktop desktop-cli desktop test-appimage`; when both pass it publishes the release with the files of both `v<major>.<minor>.<patch>` and marks it as the latest. The app finds it through `github.com/…/releases/latest` (no GitHub API, so no limit of 60 requests per hour). Compiled Rust dependencies are cached per `Cargo.lock`. A release whose tag already exists for another commit (after a force-push) is refused.
- The toolchain images are stored in the GitHub Container Registry (`ghcr.io/<owner>/pshb-*`), tagged with a hash of `versions.env` and `docker/` (`make print-toolchain-tag`); the ARM64 ones (desktop and AppImage test, built natively on an ARM runner) with `-aarch64` after it. Only when that hash changes does [`toolchain.yml`](../.github/workflows/toolchain.yml) rebuild them; with Qt this takes a few hours, a normal release run takes a few minutes. An image whose own inputs did not change (its Dockerfile, the scripts it copies, its values in `versions.env`: [`tools/image-inputs.sh`](../tools/image-inputs.sh)) is not built again: the workflow gives the existing image the new tag. So a change to the desktop images does not download the ARM toolchain for the firmware image, and a firmware change does not rebuild Qt; all images still share one tag.
- `make PREBUILT=1 FW_IMAGE=… RUST_IMAGE=… DESKTOP_IMAGE=… APPTEST_IMAGE=… all` is what CI runs: the image targets only check that the pulled images exist.
- The version is `MAJOR.MINOR` from the file [`VERSION`](../VERSION) plus the number of commits since `VERSION` was last changed ([`tools/version.sh`](../tools/version.sh), `make print-version`): the commit that raises `VERSION` is `X.Y.0`. Firmware, app, CLI and metainfo get it at build time; for a bigger step (e.g. `1.1`) only `VERSION` is edited. The boot ROM gets it as binary version `MAJOR*256+MINOR`.`PATCH`.
- The app's Changelog page is made from git when the app is built: [`tools/changelog.py`](../tools/changelog.py) turns the release tags (`v*`) and the commit subjects ("Area: outcome; outcome") into `.cache/build/changelog-<arch>.json`, which `desktop/ui/build.rs` compiles in (`PSHB_CHANGELOG`). Commits of the areas CI, Build, Tests and Tools are left out, so write the subject for the person using the app. `make test-changelog` checks it.

## Testing without a board

`make sim` starts a board simulator: input on UDP port 45045, web interface at http://localhost:8080. It uses the same core as the firmware and prints text lines instead of USB reports (`KBD …`, `MOUSE …`); its settings are kept in `.cache/sim/flash.bin`. Connect the app or `pshb-cli` with the address `127.0.0.1`. The app reaches the input side only: its board pages (Latency, clock, board update) use port 80, so against the simulator use `pshb-cli … --http-port 8080` for those.

Test traffic to a board or to the simulator:

```sh
make udp-test IP=192.168.1.50 ARGS='--psk-file psk.txt ping'
make udp-test IP=192.168.1.50 ARGS='--psk-file psk.txt pattern'   # mouse square, types "hello"
make udp-test IP=192.168.1.50 ARGS='--psk-file psk.txt flood --rate 1000 --seconds 5'
```

Grabbing and releasing the devices is tested against virtual devices (uinput) that look like a keyboard converter and a gaming mouse with several event nodes each: the bridge service is switched on and off in many ways (Ctrl+Alt+P split over two nodes, hot-plug and service restart while on, a device held by another program), and after each step a second descriptor checks whether a node is still grabbed. It needs write access to `/dev/uinput`, and read access to the virtual devices: they use the IDs of the udev rule written by "Set up access" (see [Device access](../README.md#device-access); default `feed:6536,0db0:1fc6`, otherwise `PSHB_GRAB_TEST_IDS=vvvv:pppp,vvvv:pppp`). Close the app first, it would pick up the virtual devices too. Runs on the host, not in Docker:

```sh
cd desktop && cargo test -p pshb-core --test grab_release -- --ignored --test-threads=1
```

The same goes for [`dropped_events.rs`](../desktop/core/tests/dropped_events.rs): a virtual mouse holds the left button while far more events arrive than the kernel queues for a reader that lags behind, and the release gets lost in the overflow; it must still reach the engine at once. Reading the key state (EVIOCGKEY) also deletes the key events still queued for the reader, so the tests also check that a press read before such a query cannot stay held, and that a click while another button is held (aiming and firing) is not lost.

```sh
cd desktop && cargo test -p pshb-core --test dropped_events -- --ignored --test-threads=1
```

## Screenshots

The pictures in [images/](images/) come from the app itself and from the board's web interface:

- `PS-HID-Bridge-x86_64.AppImage --screenshot DIR [--screenshot-height N]` writes a PNG of every page into `DIR` (offscreen, no board connection) and quits.
- With `--screenshot-live` as well, the app connects to the board first, switches the bridge on after 4 s and collects values on the status page for 25 s before it takes the pictures. It refuses to start next to a running app (packet counter). It grabs the devices selected in the settings, so point it at a copy of the settings (`XDG_CONFIG_HOME`) that selects only a virtual device with an ID no real device has. Moving that device (uinput) fills "Input → network" and "Input → PlayStation". The device needs `uaccess`: give it the keys 1–31 as well, then udev counts it as a keyboard and the rule from "Set up access" covers it.
- The web interface: any browser that can be scripted, e.g. Firefox with `--headless --marionette` and an empty profile. Leave out the "Details" card: it shows the MAC address.

To check the layout, `make ui-shots` takes every page at the window widths 612 (the minimum), 760, 1008 and 1400 px, light and dark, into `out/ui-shots/`. The start document [`tools/ui_demo.qml`](../tools/ui_demo.qml) fills the app with worst-case values: every pill, warning and banner at once, long device names, big numbers. It runs offscreen with an empty config, so it contacts no board and grabs nothing. `UI_STATES=plain` shows the app as after a fresh start; `UI_WIDTHS` and `UI_THEMES` pick fewer pictures.

## Checks on real hardware

For a new board or a bigger firmware change. The order is chosen so that each step builds on the previous one. The images are listed under [Other images](advanced-setup.md#other-images).

1. `picotool info -a` (BOOTSEL mode): RP2350A, 2 MB flash.
2. `ps-hid-bridge-hidtest.uf2` on the PC: `lsusb -v` (1209:0001, two HID interfaces), `evtest` (the mouse moves in a square), polling rate 1000 Hz (`usbhid-dump` / usbmon).
3. The same image on the PlayStation: mouse in the menu and in a game with mouse support, also during a running Remote Play session (Aimlabs). If the mouse does not react: try `-hidtest-mouse8.uf2`.
4. Partition table + `ps-hid-bridge-plain.uf2`: DHCP, host name in the router, `make udp-test IP=… ARGS='--plain flood --rate 1000 --seconds 10'` without losses; test SPI clock 37.5 MHz (`PSHB_W5500_SPI_HZ`, i.e. clk_peri / 4).
5. Waiting time on core 0 for the SPI bus while the web interface loads or a firmware upload runs ("… waiting for the network chip" under "Latency of the input path", or `tools/fw_bench.py`): should stay below 0.1 ms.
6. Normal firmware, key via the web interface, connect the app; check the failsafe (pull the network cable while a key is pressed → released after 250 ms).
7. Update over the network with `ps-hid-bridge.uf2` (slot changes from A to B), then with `ps-hid-bridge-rollbacktest.uf2`: this image deliberately never confirms itself; after about 17 seconds the board must restart by itself with the previous version (check slot and firmware in the web interface).
8. Settings survive several updates (they live outside the firmware slots).
9. "Latency of the input path" in the web interface while moving the mouse in a game: "Packet → report ready" around 52–56 µs (median), "Packet → fetched by the PlayStation" around 0.5–0.7 ms; "Missed polls" close to 0 and the lead around 100–150 µs ("Polls the PlayStation skipped" about 1 % is the host's doing). "Decrypting one packet" should say "optimised for this CPU". Compare with `-DPSHB_MOUSE_LATE_ARM=OFF` (reports filled right after the previous fetch, median about 1 ms). Also check after the PlayStation wakes from rest mode that no old keystrokes arrive, and that logging in to the web interface works (it needs the larger core 1 stack).
10. First installation from the app ("Firmware & updates" → "Install via USB" → "First installation") on a board that still has no firmware, and "Erase everything" followed by another first installation.
11. Firmware update from the app: bridge off, "Update board" on the Status page (or `pshb-cli update --firmware --file out/ps-hid-bridge.uf2`). The board restarts and the app then shows the new firmware version. A wrong key must be refused ("Signature does not match").
12. mDNS: `avahi-resolve -n <network name>.local` and `ping` from the PC, `http://<network name>.local` in the browser (also from a phone), `avahi-browse -rt _http._tcp` lists the board; after renaming it, the new name within a few seconds. With an IGMP-snooping switch: still found after 10 minutes. "Latency of the input path" unchanged while a phone keeps browsing `.local` names.
13. Remapped keys and profiles in a game: a mouse side button remapped to a key, a profile that follows chiaki-ng's window, a profile switch by trigger while a remapped key is held (it must not stay pressed).
14. Typing text into a PlayStation text field with its keyboard language set to German: `@`, umlauts, `é` (dead key), `^` alone; and with US English.
15. Pairing: "Get the key" on a board after a factory reset (asks for an own password first), then on a board that has a key; the browser stays logged in or can log in again.
16. Window rule on Hyprland, Sway or an X11 desktop besides KDE Plasma (the Triggers page names the backend).
17. The board gets another address while you play: give it another fixed address in the router (DHCP reservation), then pull its network cable for a moment and plug it back in (it renews its lease at once). With the bridge on and a key held, play goes on without a pause, the board's log says "told the app ... about the new address", and the app shows "Board at a new address". With older firmware on the board the app finds it within about 2 s. Then the name: once the app shows the board's `.local` name as its address (Board page), change the reservation again.

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
