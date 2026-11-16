# USB host port on two GPIOs (not used by the firmware)

The board's own USB port is a USB device: it is the keyboard and mouse the PlayStation sees. A second USB port, where the board is the **host** (for a controller, a keyboard or any other USB device), can be added on two GPIO pins and driven in software by [Pico-PIO-USB](https://github.com/sekigon-gonnoc/Pico-PIO-USB). The firmware does not use such a port at the moment. This page records how one was wired and tested, in case it is wanted later.

## Parts

- A USB-A socket (panel mount or breakout board) with four wires.
- Recommended, though the first build worked without them:
  - 2 × 22 Ω in series in D+ and D−;
  - 2 × 15 kΩ from D+ and from D− to GND (the pull-downs a USB host has);
  - at least 100 µF between 5 V and GND at the socket;
  - a resettable fuse of about 1 A in the 5 V line.

## Wiring

| Signal | Board (W5500-EVB-Pico2) | Wire on the first build |
|---|---|---|
| 5 V (VBUS) | pin 40 (VBUS) | red |
| GND | pin 3 (GND) | black |
| D+ | pin 4 (GP2) | blue |
| D− | pin 5 (GP3) | white |

- **D− on the next GPIO after D+.** Pico-PIO-USB needs D− on the GPIO right after D+ (or right before it, as a software option).
- **Check the colours.** Wire colours differ between sockets (D+ is often green). Measure with a multimeter. On a USB-A socket the contacts are numbered 1 = 5 V, 2 = D−, 3 = D+, 4 = GND; 5 V and GND are the two outer ones.
- **Before the first use, check:**
  - pin 4 to the socket's D+ and pin 5 to D−;
  - no connection between pins 4 and 5 (they are neighbours on the header);
  - no short between 5 V and GND.

## Power

- **Source:** The socket's 5 V comes straight from the board's USB connector (pin 40 is VBUS, before the diode to VSYS). Whatever powers the board powers the device too, with no switch in between.
- **A charging controller draws up to about 1 A.**
- **Some devices need the full 5 V.**
  - A DualSense neither charged nor connected with the board on a PC port behind a USB dock: the board measured VSYS 4.34 V, so about 4.6 V at the socket.
  - With the board directly on a laptop port (VSYS 5.0 V) it connected at once.
  - A phone worked both ways.
  - Measure the voltage at the socket, with the device plugged in; aim for at least 4.75 V.

## Conflict with the timing image

- **The timing image drives GP2/GP3.** `ps-hid-bridge-timing.uf2` uses them as measuring outputs, so it would drive the socket's data lines. Do not install it while a device is plugged into the socket. Or move its pins to free ones (`PSHB_TIMING_PIN_RX` / `PSHB_TIMING_PIN_FETCH` in `firmware/include/pshb_config.h`, e.g. GP6/GP7) and update the README.
- **Nothing else uses GP2/GP3.** UART0 is on GP0/GP1, the W5500 on GP16–GP21, the LED on GP25.

## If firmware for it is written

What the first test (November 2026) ran into:

- **Library and cores:**
  - Pico-PIO-USB 0.7.3 worked with TinyUSB 0.18.0's PIO-USB host driver (`CFG_TUH_RPI_PIO_USB`, root port 1, `pin_dp = 2`) at the standard 150 MHz.
  - It belongs alone on core 1: a 1 ms SOF interrupt that busy-waits during transfers.
- **Resources:**
  - It claims its DMA channel by number (default 0); pass one the W5500 left free.
  - It takes PIO0 (all 32 instructions) and hardware alarm 2.
- **Pin reads:** It inverts the pins' inputs (`gpio_set_inover`), so while it runs `gpio_get()` reads the opposite of the line.
- **RP2350-E9:** Its line-state workaround only covers chip revision 2. This board reports revision 3 (A3 or A4); it worked without external pull-downs.
- **Descriptor size:** TinyUSB's enumeration buffer (`CFG_TUH_ENUMERATION_BUFSIZE`) must hold the largest descriptor. A DualSense's HID report descriptor is 289 bytes; the default of 256 drops it.
- **Limits:** Full speed (12 Mbit/s) only, no isochronous transfers (USB audio).
- **Testing without BOOTSEL:** A test image installed over the network (`pshb-cli update --firmware --file …`) runs on trial: if it never confirms itself, any reset returns to the normal firmware.
  - The boot ROM clears the watchdog scratch registers when such an image restarts itself on trial.
  - Uninitialised RAM (`__uninitialized_ram`) survives that restart.

## What was tried

In November 2026 a DualSense was plugged in here and passed through to a PS5, its authentication included. The aim was a controller mode for games without mouse and keyboard support.

- **On the board:** The DualSense enumerated: full speed, 250 input reports per second; light bar, LEDs and rumble answered output reports.
- **The PS5's check:** The PS5 ran its authentication through the board, and the real DualSense answered: four `SET_REPORT 0xF0` (challenge), `GET_REPORT 0xF2` (status), four `GET_REPORT 0xF1` (answer).
- **HID interface only:** The PS5 rejected the answer and asked again about every 13 seconds.
- **All four interfaces:** It accepted the answer and used the controller. The four interfaces are audio control, speaker, microphone and HID; the audio was taken and dropped.
- **Stopped there,** before a test in a game.
