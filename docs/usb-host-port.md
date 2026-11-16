# USB host port on two GPIOs (not used by the firmware)

The board's own USB port is a USB device: it is the keyboard and mouse the PlayStation sees. A second USB port, where the board is the **host** (for a controller, a keyboard or any other USB device), can be added on two GPIO pins and driven in software by [Pico-PIO-USB](https://github.com/sekigon-gonnoc/Pico-PIO-USB).

The firmware does not use such a port. In November 2026 one was built and tested for a "controller mode": the board presents a DualSense to a PS5, and a real DualSense on the second port answers the console's authentication. That was stopped after the passthrough worked in the PS5's menus. This page keeps everything learned, so it can be picked up again. The code is kept under the git tag [`controller-mode-experiment`](https://github.com/upchui/PS-HID-Bridge/releases/tag/controller-mode-experiment) (see [The code](#the-code)).

- [Hardware](#hardware)
- [The DualSense on this port](#the-dualsense-on-this-port)
- [The PS5 and a passed-through DualSense](#the-ps5-and-a-passed-through-dualsense)
- [How the test firmware was built](#how-the-test-firmware-was-built)
- [Open ends](#open-ends)
- [The code](#the-code)

## Hardware

### Parts

- A USB-A socket (panel mount or breakout board) with four wires.
- Recommended, though the first build worked without them:
  - 2 × 22 Ω in series in D+ and D−;
  - 2 × 15 kΩ from D+ and from D− to GND (the pull-downs a USB host has);
  - at least 100 µF between 5 V and GND at the socket;
  - a resettable fuse of about 1 A in the 5 V line.

### Wiring

| Signal | Board (W5500-EVB-Pico2) | Wire on the first build |
|---|---|---|
| 5 V (VBUS) | pin 40 (VBUS) | red |
| GND | pin 3 (GND) | black |
| D+ | pin 4 (GP2) | blue |
| D− | pin 5 (GP3) | white |

- **D− next to D+.** Pico-PIO-USB needs D− on the GPIO right after D+ (`PIO_USB_PINOUT_DPDM`) or right before it (`PIO_USB_PINOUT_DMDP`). A socket wired the other way round therefore works in software too, with D+ = GP3.
- **Check the colours.** Wire colours differ between sockets (D+ is often green). On a USB-A socket the contacts are numbered 1 = 5 V, 2 = D−, 3 = D+, 4 = GND; 5 V and GND are the two outer ones.
- **Before the first use, check with a multimeter:**
  - pin 4 to the socket's D+ and pin 5 to D−;
  - no connection between pins 4 and 5 (they are neighbours on the header);
  - no short between 5 V and GND.
- **Other pins in use:** UART0 GP0/GP1, the W5500 GP16–GP21, the LED GP25. GP23/24/29 are board-internal: GP24 senses VBUS, GP29 reads VSYS/3 on ADC 3. Nothing else uses GP2/GP3.

### Power

- **Source:** The socket's 5 V comes straight from the board's USB connector. Pin 40 is VBUS, before the diode (BAT60A) to VSYS. Whatever powers the board powers the device too, with no switch in between. A measurement at the socket showed 4.9 V.
- **Draw:** A charging DualSense draws up to about 1 A (rated 5 V / 1.5 A).
- **What was seen:**
  - **No attach at first.** The board was on a PC port behind a USB dock (VSYS 4.34 V, so about 4.6 V at the socket). The DualSense neither charged (no orange light) nor attached: D+ stayed low. It was connected to the PC over Bluetooth at the same time. On that same port a phone attached and enumerated at once.
  - **Attached later:** after the board moved to a laptop port directly (VSYS 4.8–5.0 V), after the DualSense had been on the PC by cable once.
  - **Also on the PS5:** there it worked even at VSYS 4.25 V.
  - **Cause unclear:** low voltage, the Bluetooth connection or the controller's state.
  - **For the next time:** start with a solid supply, with the controller not connected over Bluetooth to anything, and switched off before plugging it in.
- **Measure it yourself:** The board can measure VSYS itself: `adc_gpio_init(29)`, input 3, millivolts = raw × 3 × 3300 / 4096.

### Conflict with the timing image

`ps-hid-bridge-timing.uf2` uses GP2/GP3 as measuring outputs: it would drive the socket's data lines. Do not install it while a device is plugged into the socket. Or move its pins to free ones first: `PSHB_TIMING_PIN_RX` / `PSHB_TIMING_PIN_FETCH` in `firmware/include/pshb_config.h`, e.g. GP6/GP7. The experiment did that; update the README and [latency.md](latency.md) along with it.

### Chip

The board's RP2350 reports chip revision 3 (`rp2350_chip_version()`), which means A3 or A4.

- **RP2350-E9 erratum:** A2 and A3 have it, A4 fixed it. An input pad with its pull-down on and nothing driving it can float up to about 2.2 V.
- **Not seen here:** with the pull-downs on and nothing attached, both lines read low.
- **Pico-PIO-USB's workaround** (drain the input before reading the line state) only covers revision 2. The experiment widened it to revision 3 in a patched copy (see [Build](#build)).

## The DualSense on this port

### Descriptors (as read over full speed)

- **Device:**
  - 054C:0CE6, bcdUSB 2.00, bMaxPacketSize0 64, bcdDevice 1.00.
  - Strings: manufacturer "Sony Interactive Entertainment" and product "DualSense Wireless Controller"; no serial number string.
  - The DualSense supports high speed. Through Pico-PIO-USB, and towards the PS5 from the board, everything runs at full speed.
- **Configuration:** 227 bytes, self-powered, 500 mA, four interfaces:

| Interface | Content | Endpoint (alternate setting 1) |
|---|---|---|
| 0 | audio control (UAC1) | — |
| 1 | speaker / haptics: 4 channels, 16 bit, 48 kHz | 0x01 OUT, isochronous, 392 bytes, every 1 ms |
| 2 | microphone: 2 channels, 16 bit, 48 kHz | 0x82 IN, isochronous, 196 bytes, every 1 ms |
| 3 | HID, report descriptor 289 bytes | 0x84 IN and 0x03 OUT, interrupt, 64 bytes, every 4 ms |

- **Report descriptor:** 289 bytes, a little more than the 273 that older dumps show (controller firmware). TinyUSB's host drops report descriptors larger than `CFG_TUH_ENUMERATION_BUFSIZE` (default 256): set it to 512.

Reports declared in it:

| Report | Size (after the ID) | Known use |
|---|---|---|
| input 0x01 | 63 | sticks, triggers, buttons, motion, touch |
| output 0x02 | 47 | rumble, light bar, player LEDs, trigger effects |
| feature 0x05 | 40 | calibration |
| feature 0x08 | 47 | the PS5 writes `08 11 00 …` (17 bytes) now and then |
| feature 0x09 | 19 | pairing: the controller's and the paired host's Bluetooth addresses |
| feature 0x0A | 26 | pairing (sets the host and link key) |
| feature 0x0B | 41 | like 0x09; read by the PS5 (38 bytes) |
| feature 0x0C | 41 | — |
| feature 0x20 | 63 | firmware info (build date as text, versions) |
| feature 0x21, 0x22 | 4, 63 | — |
| feature 0x80 / 0x81 | 63 / 63 | command / answer: the PS5 writes `80 01 13 …` and reads back the serial number as text |
| feature 0x82–0x85 | 9, 63, 63, 2 | — |
| feature 0xA0, 0xE0 | 1, 63 | — |
| feature 0xF0 / 0xF1 / 0xF2 | 63 / 63 / 15 | authentication: challenge / answer / status (see below) |
| feature 0xF4 / 0xF5 | 63 / 3 | further authentication reports, not used by the PS5 here |

<details>
<summary>Configuration descriptor (227 bytes)</summary>

```
09 02 e3 00 04 01 00 c0 fa 09 04 00 00 00 01 01
00 00 0a 24 01 00 01 49 00 02 01 02 0c 24 02 01
01 01 06 04 33 00 00 00 0c 24 06 02 01 01 03 00
00 00 00 00 09 24 03 03 01 03 04 02 00 0c 24 02
04 02 04 03 01 00 00 00 00 09 24 06 05 04 01 03
00 00 09 24 03 06 01 01 01 05 00 09 04 01 00 00
01 02 00 00 09 04 01 01 01 01 02 00 00 07 24 01
01 01 01 00 0b 24 02 01 04 02 10 01 80 bb 00 09
05 01 09 88 01 01 00 00 07 25 01 01 00 00 00 09
04 02 00 00 01 02 00 00 09 04 02 01 01 01 02 00
00 07 24 01 06 01 01 00 0b 24 02 01 02 02 10 01
80 bb 00 09 05 82 05 c4 00 01 00 00 07 25 01 00
00 00 00 09 04 03 00 02 03 00 00 00 09 21 11 01
00 01 22 21 01 07 05 84 03 40 00 04 07 05 03 03
40 00 04
```
</details>

<details>
<summary>HID report descriptor (289 bytes)</summary>

```
05 01 09 05 a1 01 85 01 09 30 09 31 09 32 09 35
09 33 09 34 15 00 26 ff 00 75 08 95 06 81 02 06
00 ff 09 20 95 01 81 02 05 01 09 39 15 00 25 07
35 00 46 3b 01 65 14 75 04 95 01 81 42 65 00 05
09 19 01 29 0f 15 00 25 01 75 01 95 0f 81 02 06
00 ff 09 21 95 0d 81 02 06 00 ff 09 22 15 00 26
ff 00 75 08 95 34 81 02 85 02 09 23 95 2f 91 02
85 05 09 33 95 28 b1 02 85 08 09 34 95 2f b1 02
85 09 09 24 95 13 b1 02 85 0a 09 25 95 1a b1 02
85 0b 09 41 95 29 b1 02 85 0c 09 42 95 29 b1 02
85 20 09 26 95 3f b1 02 85 21 09 27 95 04 b1 02
85 22 09 40 95 3f b1 02 85 80 09 28 95 3f b1 02
85 81 09 29 95 3f b1 02 85 82 09 2a 95 09 b1 02
85 83 09 2b 95 3f b1 02 85 84 09 2c 95 3f b1 02
85 85 09 2d 95 02 b1 02 85 a0 09 2e 95 01 b1 02
85 e0 09 2f 95 3f b1 02 85 f0 09 30 95 3f b1 02
85 f1 09 31 95 3f b1 02 85 f2 09 32 95 0f b1 02
85 f4 09 35 95 3f b1 02 85 f5 09 36 95 03 b1 02
c0
```
</details>

### Input report 0x01 (64 bytes with the ID, every 4 ms)

Checked on the board with sticks, triggers and buttons; the rest follows Linux `hid-playstation.c`.

| Byte | Content |
|---|---|
| 0 | report ID 0x01 |
| 1–4 | left stick X, Y; right stick X, Y (0–255, rest about 128) |
| 5–6 | L2, R2 (0–255) |
| 7 | sequence number |
| 8 | bits 0–3 D-pad (0 = up, clockwise, 8 = released); bit 4 square, 5 cross, 6 circle, 7 triangle |
| 9 | bit 0 L1, 1 R1, 2 L2, 3 R2, 4 create, 5 options, 6 L3, 7 R3 |
| 10 | bit 0 PS, 1 touchpad click, 2 mute |
| 16–27 | gyro and accelerometer (3 × 16 bit each) |
| 28–31 | sensor timestamp |
| 33–40 | two touch points |

There is no CRC over USB; that is Bluetooth only.

### Output report 0x02 (47 bytes after the ID)

The test firmware sent the following, and the controller answered with a green light bar, the middle player LED and a short rumble. Offsets are after the report ID, as in `hid-playstation.c`.

| Byte | Value | Meaning |
|---|---|---|
| 0 | 0x03 | valid flags 0: compatible vibration, haptics select |
| 1 | 0x14 | valid flags 1: light bar (0x04), player LEDs (0x10) |
| 2, 3 | 0x80 | right and left motor (0 = off) |
| 38 | 0x02 | valid flags 2: light bar setup |
| 41 | 0x02 | light bar setup: end the blue start-up glow |
| 43 | 0x04 | player LEDs: the middle one |
| 44–46 | 0x00 0xC0 0x40 | light bar red, green, blue |

## The PS5 and a passed-through DualSense

The board showed the PS5 the DualSense's own descriptors and passed every class request through to the real controller. The test ran on a PS5 with Settings → Accessories → Controllers → Communication method set to **Use USB cable**. With "Bluetooth" a DualSense plugged in by USB is paired and then used over Bluetooth, past the board.

### What the PS5 asks, in this order

1. **Descriptors:** device ×2, string 0 ×2, strings 1 and 2 ×2 each, configuration ×3, then `SET_CONFIGURATION`.
2. **Audio requests,** only when all four interfaces are shown:
   - `GET_MIN`, `GET_MAX` and `GET_RES` of the volume on feature units 5 and 2, and `SET_CUR` of the volume;
   - `SET_INTERFACE` 1 and 2 to alternate setting 1;
   - `SET_CUR` of the sampling rate on endpoint 0x01 (3 bytes). The real controller refused this one, because its own speaker interface had stayed at alternate setting 0. The PS5 went on regardless.
3. **HID:** `GET_DESCRIPTOR` report (289), `SET_IDLE` 0, `GET_REPORT` feature 0x05 (41), 0x20 (64) and 0x0B (38); `SET_REPORT` 0x80 (`80 01 13 …`) and `GET_REPORT` 0x81.
4. **Now and then** `SET_REPORT` 0x08 (`08 11 00 …`, 17 bytes).
5. **Authentication rounds** (below) when the controller is to be used, e.g. after the PS button. They also come after a replug.
6. **After it accepted the controller:**
   - `SET_REPORT` 0x80 and ten `GET_REPORT` 0x81, one per second;
   - output reports 0x02, about one per second;
   - audio to the speaker endpoint, about 580 KB/s, which the board dropped.

Every request passed through the board took 2–4 ms there and back, over Pico-PIO-USB at full speed.

### The authentication

The structure as observed. The values are left out: they identify the console and the controller.

- **`SET_REPORT` 0xF0, 4 × 64 bytes (the challenge).**
  - Byte 1 is 0x01, byte 2 the round number, byte 3 the page (0–3).
  - Page 0: `02 00 00 00`, then 16 bytes new in every round, then 8 bytes that stay.
  - Pages 1 and 2: 60 bytes each, the same in every round (the console's side, presumably a key or certificate).
  - Page 3: 12 new bytes, the rest zero.
- **`GET_REPORT` 0xF2, 16 bytes (the status):** `f2 00 <round> 12 00 …`. Byte 2 is the round whose answer is ready; read too early, it still shows the previous one.
- **`GET_REPORT` 0xF1, 4 × 64 bytes (the answer).** Byte 2 is the round, byte 3 the page.
  - Page 0: 16 bytes new in every round, then `10 00 03 00`, then at byte 36 a 16-character text identifier of the controller and 8 more bytes.
  - Pages 1 and 2: the same in every round (the controller's key or certificate).
  - Page 3: 8 fixed and 16 new bytes, the rest zero.
- **Rounds:**
  - **The first ones are quick.** The SETs come 12 ms apart, the status 30 ms after the last SET, answer page 0 15 ms later. Then the PS5 started over, usually reading only answer page 0, and the status still showed an older round.
  - **Later ones are slow.** One request per second, all four answer pages read, a new round every ~13 s for as long as the controller was not accepted.
- **Result:**
  - **HID interface only:** never accepted: about 15 rounds in a little over 3 minutes, and no output reports.
  - **All four interfaces:** accepted after a single round, and no further round in the following ~3 minutes (the end of the log). Then output reports, audio, and the controller worked in the menus.
  - A stick pattern was then written into the forwarded input reports for 8 seconds (the left stick circling). Whether the menu followed it was not confirmed before the test stopped. That the PS5 takes input the DualSense did not send is therefore still open.
- **Not seen,** because the test stopped before: the re-check that third-party controllers report after about 8 minutes, a game, rest mode, a PS5 restart.
- **Feature reports 0xF4/0xF5:** never sent by the PS5. An older DualSense was reported to use them (GIMX issue 672).

## How the test firmware was built

### Build

- **Pico-PIO-USB 0.7.3** as a submodule `firmware/lib/pico-pio-usb`, pinned in `versions.env` (`PICO_PIO_USB_TAG`) and checked by `tools/check-pins.sh`.
  - Its API matches what TinyUSB 0.18.0's driver `src/portable/raspberrypi/pio_usb/hcd_pio_usb.c` calls; TinyUSB pins Pico-PIO-USB 0.6.1 itself.
  - Changing `versions.env` changes the CI toolchain tag (`tools/toolchain-tag.sh` hashes the whole file): the first CI run on main rebuilds the toolchain images, Qt included.
- **CMake:** set `PICO_PIO_USB_PATH` **before** `pico_sdk_init.cmake`; TinyUSB's `hw/bsp/rp2040/family.cmake` then builds the target `tinyusb_pico_pio_usb`. The experiment pointed it at a copy made at configure time, with the E9 check widened from `chip_version <= 2` to `<= 3` (build fails if that line changes upstream).
- **Image targets:** the test images linked `tinyusb_host` and `tinyusb_pico_pio_usb` (plus `tinyusb_device` for the passthrough), and only the sources they needed:
  - core: `settings.c`, `crc32.c`, `uf2_writer.c` and the new `logbuf.c`;
  - pico: `clock.c`, `flash_store.c`, `w5500_port.c`, `ota.c` and the new `netlog.c`.

  They were OTA images (TBYB, hashed, versioned, like the firmware; a function `pshb_make_ota()`).
- **`tusb_config.h`** for the test images:
  - `CFG_TUSB_RHPORT1_MODE (OPT_MODE_HOST | OPT_MODE_FULL_SPEED)` and `CFG_TUH_RPI_PIO_USB 1`;
  - `CFG_TUH_DEVICE_MAX 1`, `CFG_TUH_HUB 0`, `CFG_TUH_HID 4`;
  - `CFG_TUH_ENUMERATION_BUFSIZE 512`, HID endpoint buffers 64.

  The device stays on root port 0.

### Running it

- **Core 1 runs only the host.** Pico-PIO-USB has a 1 ms SOF interrupt (hardware alarm 2) that busy-waits during transfers. `tuh_configure(1, TUH_CFGID_RPI_PIO_USB_CONFIGURATION, &cfg)` and `tuh_init(1)` must run on core 1, followed by a loop of `tuh_task()`.
- **Core 1's stack:** 8 KB rather than the default 2 KB. The blocking `tuh_descriptor_get_*_sync()` calls nest `tuh_task()`. Call them from that loop, not from inside a callback.
- **Configuration:** `pin_dp = 2`, `pinout = PIO_USB_PINOUT_DPDM`.
  - `tx_ch`: Pico-PIO-USB claims its DMA channel by number (`dma_claim_mask`, default 0). Pass one the W5500 left free: initialise the W5500 first, then `dma_claim_unused_channel()` and unclaim it again before handing it over.
  - It takes PIO0 (all 32 instructions) and state machines 0–2.
- **Pin reads:** Pico-PIO-USB inverts the pins' inputs (`gpio_set_inover`). While it runs, `gpio_get()` reads the opposite of the line: both reading 1 means an idle bus. Draining a pin for E9 while it runs would disturb its receiver.
- **TinyUSB host calls used:**
  - `tuh_mount_cb`, `tuh_hid_mount_cb` (gives the report descriptor);
  - `tuh_hid_receive_report` and `tuh_hid_report_received_cb`;
  - `tuh_hid_get_report` / `tuh_hid_set_report` for feature reports;
  - `tuh_hid_send_report` for output reports (it puts the report ID first);
  - `tuh_control_xfer` for any relayed control request.
- **Speed:** It ran at the standard 150 MHz. Upstream TinyUSB soak-tests RP2350 at that clock; 200 MHz was not tried.

### Testing without BOOTSEL

The board's BOOTSEL button did not work. The test images were installed over the network like a firmware update: `pshb-cli update --firmware --file out/<image>.uf2`.

- **They run on trial.** The boot ROM starts them on trial; as long as they never confirm themselves, any reset or power cut returns to the normal firmware. That makes them safe to try. `pshb-cli` then waits a minute for a web interface the test image does not have and reports "did not come back"; that is expected.
- **Starting itself again** (a mode switch, a hung core 1): `rom_reboot(REBOOT2_FLAG_REBOOT_TYPE_FLASH_UPDATE, …)` with `XIP_BASE` plus the running slot's start. The start is in the QMI address translation, `qmi_hw->atrans[0]` base × 4096. This was `ota_restart_on_trial()`.
- **Returning to the normal firmware:** `watchdog_reboot()` only arms the watchdog. The main loop must stop feeding it afterwards, or nothing happens.
- **What survives such a restart:**
  - The boot ROM clears the **watchdog scratch registers** on that reboot, so modes kept there were lost.
  - **Uninitialised RAM** (`__uninitialized_ram`) keeps its contents, with a magic word to tell it from power-on garbage.
- **Installing another test image** needs the normal firmware first: `back` (return), then the next update.

### Logging over the network

Without a UART adapter, the test images logged over the network:

- **`netlog.c`:** an extra stdio driver (`stdio_set_driver_enabled`). Every `printf` line gets a timestamp and goes into a 48 KB ring buffer (`logbuf.c`, host-tested) in uninitialised RAM, so the log of a hang survived the restart.
  - **UART0:** the buffer is written there without waiting.
  - **UDP port 45046:** a datagram "sub" makes the board send everything it still has, then each new line; "ka" keeps that up; any other text is a command. The board also brought up the W5500 with the stored network settings (DHCP or static) for this.
- **`tools/boardlog.py <ip> [-o file] [-c cmd]`** subscribes, prints and sends commands.

### The two test images

- **`ps-hid-bridge-usbhosttest` (wiring test):**
  - It checks the pins before the host starts: levels with pull-downs, after draining (E9), and whether driving one line moves the other (a bridge between pins 4 and 5).
  - It then enumerates whatever is plugged in and logs speed, IDs, descriptors and strings, the feature reports 0x05/0x09/0x20, the sticks and buttons (with a dead band against jitter), reports per second and VSYS.
  - It sends the output report above, so the controller lights up and rumbles.
  - Commands: `watch` (no host, only watch D+/D−), `host`, `swap`, `restart`, `back`.
- **`ps-hid-bridge-dspassthrough` (passthrough test):**
  - **Identity:** Core 1 reads the DualSense's device, configuration and string descriptors and its report descriptor, then publishes them to core 0. The device side to the PS5 starts (`tud_init(0)`) only after that, and disconnects when the DualSense is pulled.
  - **Class driver:** Core 0 serves those descriptors with a class driver of its own (`usbd_app_driver_get_cb`, `CFG_TUD_HID 0`):
    - **IN class requests:** the driver returns `true` from the SETUP stage without data. Once the controller answered, the main loop calls `tud_control_xfer()`; until then the PS5's data stage waits. After 500 ms, or if the controller refused, both control endpoints are stalled with `dcd_edpt_stall()`.
    - **A newer SETUP drops the pending answer.** `tud_event_hook_cb()` counts `DCD_EVENT_SETUP_RECEIVED`.
    - **OUT class requests** are acknowledged after their data stage, and passed on in order.
  - **Queues:** core 0 → core 1 carries the requests; core 1 → core 0 the answers. Input reports go latest-wins, output reports through a queue.
  - **Variant a:** only the HID interface, renumbered to 0 (`ds_desc.c`, host-tested); requests to it are mapped back to the controller's interface 3.
  - **Variant b:** all four interfaces.
    - The audio streaming interfaces' isochronous endpoints are set aside in the driver's `open()` (`usbd_edpt_iso_alloc`) and switched on at `SET_INTERFACE` (`usbd_edpt_iso_activate`).
    - Speaker data is taken and dropped; the microphone sends zeros.
    - The variant was a settings byte (taken from the reserved ones).
  - **Commands:** `pattern on|off` (the left stick circles), `variant a|b`, `status`, `restart`, `back`.

## Open ends

- **Untested:** a game for more than 15 minutes, the re-check after about 8 minutes, rest mode, a PS5 restart, a system update.
- **Haptics and speaker:** Pico-PIO-USB has no isochronous transfers, so the console's audio stream cannot reach the controller. That is the haptics of PS5 games and the speaker. It is unknown whether the PS5 sends HID rumble at all when it streams haptics.
- **Sampling rate:** The `SET_CUR` of the sampling rate failed at the controller. Either answer it on the board, or pass `SET_INTERFACE` on to the controller as well.
- **For an actual controller mode:**
  - the board would put its own input into the forwarded reports and keep the sequence number and timestamp plausible (first check: does the stick pattern move the menu?);
  - the app would compute that input;
  - see the original plan in the tag's `docs/controller-mode.md`.
- **Online games** can detect adapters (anti-cheat), and a PS5 system update can break the passthrough, as happened to commercial adapters.

## The code

Everything is in the git tag **[`controller-mode-experiment`](https://github.com/upchui/PS-HID-Bridge/releases/tag/controller-mode-experiment)** (10 commits on top of main as of November 2026; the branch itself was deleted):

```sh
git show controller-mode-experiment --stat          # what is in it
git switch -c controller-mode controller-mode-experiment
```

The files:

| Path | Content |
|---|---|
| `firmware/src/pico/netlog.c`, `tools/boardlog.py` | the network log |
| `firmware/src/pico/usbhost_test.c` | the wiring test |
| `firmware/src/pico/dspass.c` | the passthrough |
| `firmware/src/core/logbuf.c`, `ds_desc.c` | portable parts, with tests in `firmware/tests/` |
| `docs/controller-mode.md` | the test plan and checklists |

On GitHub: <https://github.com/upchui/PS-HID-Bridge/releases/tag/controller-mode-experiment>
