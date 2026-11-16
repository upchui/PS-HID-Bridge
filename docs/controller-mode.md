# Controller mode (in development)

Many PS5 games do not support mouse and keyboard. In the controller mode the board presents itself to the PlayStation as a DualSense, and the app turns mouse and keyboard into controller input. The PlayStation checks controllers cryptographically. A real DualSense on a second USB port of the board is meant to answer that check; the board passes it through.

**Status: test images only, not usable for playing.** This first step finds out whether a PS5 accepts the board as a DualSense this way at all. The mapping in the app comes only if it does.

## Will it work?

Unknown:

- **No public success.** No project has shown a PS5 accepting a cloned DualSense whose check is answered by an ordinary DualSense.
  - The closest attempt, [a Raspberry Pi 5 relay](https://github.com/0xThreadz/dualsense-ps5-usb-research), passed every packet through, but the check never completed.
  - Commercial adapters need special dongles for it. [ReaSnow](https://www.reasnow.com/download/ReaSnowApp/workarounds%20for%20PS5.pdf) says outright that a PS5 controller cannot be the authentication controller.
- **What is known to work** is a different identity: a *licensed PS4 peripheral* with a licensed PS4 controller as the authenticator ([jfedor2/paaas](https://github.com/jfedor2/paaas), GP2040-CE). It works only in PS5 games that support such peripherals.
- **No audio:** the DualSense's speaker, microphone and the haptics that run over audio cannot be passed through. The USB host on the board (Pico-PIO-USB) has no isochronous transfers. Rumble, the light bar and the adaptive triggers (HID output reports) can.
- **Full speed only:** the board talks to the PlayStation at USB full speed; a real DualSense connects at high speed.
- **Updates and detection:** a PlayStation system update can break the passthrough at any time, as it did for commercial adapters. Online games can detect such adapters (anti-cheat) and ban accounts. Use it at your own risk.

## Wiring

A USB-A socket with four wires on the W5500-EVB-Pico2; the DualSense plugs into it with a USB-A to USB-C data cable.

| Signal | Board | Wire (first build) |
|---|---|---|
| 5 V (VBUS) | pin 40 (VBUS) | red |
| GND | pin 3 (GND) | black |
| D+ | pin 4 (GP2) | blue |
| D− | pin 5 (GP3) | white |

Wire colours differ between sockets (D+ is often green): check D+ and D− with a multimeter.

Recommended:
- 22 Ω in series on D+ and D− (Pico-PIO-USB).
- 15 kΩ from D+ and from D− to GND (device detection).
- At least 100 µF between VBUS and GND at the socket.

The socket's 5 V comes straight from the board's USB port, which the PlayStation supplies; there is no fuse in between. A charging DualSense draws up to about 1 A. The test images log the supply voltage (VSYS) so a drop shows up.

GP2/GP3 are used for nothing else. The timing image's measuring pins moved to GP6/GP7 for this.

## Test images

| File | What it does |
|---|---|
| `ps-hid-bridge-usbhosttest.uf2` | **Wiring test** at the PC, without the PlayStation. It describes whatever is plugged into the socket: speed, IDs, descriptors, for a DualSense also some feature reports and its sticks and buttons. It also logs the supply voltage and which data line is pulled up. |
| `ps-hid-bridge-dspassthrough.uf2` | **Passthrough test:** a real DualSense in the socket, the board on the PlayStation. The board shows the PlayStation the DualSense's own descriptors and passes through every class request (its authentication included), its input reports and the PlayStation's output reports (rumble, light, triggers). |

Install them like the HID test images:
- **By hand:** hold BOOTSEL, connect the board to the PC, copy the file onto the drive `RP2350`.
- **From the app:** "Firmware & updates" → "Install via USB" → "Choose file …".

To return, install `ps-hid-bridge.uf2` ("Firmware only"); key, password and network settings stay.

## The log

The test images do not process input from the app; they log everything:
- **UART0** (GP0/GP1, 115200 baud).
- **Over the network**, with the board's stored network settings. On the PC:

```sh
tools/boardlog.py 192.168.1.50                # print the log (from the board's start on)
tools/boardlog.py 192.168.1.50 -o ds.log      # also save it
tools/boardlog.py 192.168.1.50 -c status      # send a command
```

- **Timestamps and restarts:** every line carries the seconds since start. The log survives a restart by the watchdog (it restarts the board when the USB host hangs for more than 2.5 s).
- **Commands:**
  - Wiring test: `status`, `swap` (restarts with D+ and D− swapped, for a socket wired the other way round), `reboot`.
  - Passthrough test: `status`, `pattern on|off`, `variant a|b`, `reboot`.
    - `pattern on`: the left stick circles. This proves the PlayStation accepts input the DualSense did not send.
    - `variant a`: the PlayStation sees only the DualSense's HID interface (the default).
    - `variant b`: it sees all four interfaces; audio is accepted and dropped. Saved in the settings; the board restarts.

## Checklists

### 1. Wiring test (at the PC)

1. Multimeter: pin 4 to the socket's D+, pin 5 to D−; no short between VBUS and GND.
2. Install `ps-hid-bridge-usbhosttest.uf2`. Leave the board on the PC (power) with the network cable plugged in, and start `tools/boardlog.py`.
3. Plug in the DualSense. Expected:
   - `line: D+ (GP2) high: a full-speed device is attached`, then `054c:0ce6, full speed (DualSense)`.
   - A configuration descriptor of 227 bytes with 4 interfaces, and an HID report descriptor of 273 bytes.
   - The feature reports 0x05, 0x09 and 0x20.
   - `ds:` lines that change when you move the sticks and press buttons. The LED is steady.
   - `D- (GP3) high` means D+ and D− are swapped: fix the wiring, or send `swap`.
4. Unplug and plug in again a few times. Watch the `power:` lines while the DualSense charges; VSYS should stay above 4.3 V.

### 2. Passthrough test (on the PlayStation)

1. Install `ps-hid-bridge-dspassthrough.uf2`. Board on the PlayStation (its USB port, as usual), DualSense in the socket, network cable plugged in. Start `tools/boardlog.py`.
2. Switch the PlayStation on, press the PS button, move through the menus.
3. Play a native PS5 game for more than 15 minutes; the PlayStation checks controllers again after about 8 minutes.
4. Rest mode and waking up; restarting the PlayStation.
5. `pattern on` in the game: the left stick must circle.
6. If variant a fails (not recognised, or dropped after a while), try `variant b`.

**Success:** a native PS5 game for more than 15 minutes without the controller dropping, also after rest mode and a restart. If it fails, the next step stops here; the log (`ctl` lines: every request the PlayStation sent and how the DualSense answered) shows where.
