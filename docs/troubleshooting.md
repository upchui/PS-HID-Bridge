# Troubleshooting

Problems and where to look. For setting up and using PS HID Bridge, see the [README](../README.md). In an emergency, `Ctrl+Alt+Shift+Esc` always releases mouse and keyboard at once and lets go of everything on the PlayStation.

## A mouse button stays pressed

The status page shows what goes to the PlayStation and which device reports it ("from …"); a mouse button held for more than 3 s gets a hint. The app forwards what the kernel reports, so first find out where the button is held:

1. **Kernel:** with the button stuck, `sudo evtest --query /dev/input/eventN EV_KEY BTN_LEFT` (exit code 10 = held; the node is in `pshb-cli list-devices`). Held there although you let go: the mouse, its receiver or the USB path never delivered the release, not the app.
2. **Mouse/receiver:** `sudo cat /dev/hidrawN | xxd -c 12` shows the raw reports (first byte = buttons). A wireless receiver that sends only the release edge when you click the stuck button again (no press before it) had lost the earlier release itself. Seen with an MSI VERSA PRO WIRELESS: a release was lost when another button changed within a few hundred milliseconds.
3. **App:** held in the app but not in the kernel is a bug: please report it with the output of `journalctl --user | grep "PS HID Bridge"`.

What helps with a receiver that loses releases: the mouse wired, the receiver on a USB extension close to the mouse and away from other 2.4 GHz receivers, USB 3 ports and docks, the newest mouse and receiver firmware (vendor tool), another polling rate. To free a stuck button without clicking each one: "Release all keys" on the status page, or bind the trigger action *Release all* to a key; it lets go of everything on the PlayStation and ignores the stuck button until its next real release.

## The board is not found over USB

Use a USB cable with data lines, hold BOOTSEL *while* plugging in, and check that `udisks2` runs (`systemctl status udisks2`). See [Flashing over USB from the PC](advanced-setup.md#flashing-over-usb-from-the-pc).

## Wrong characters on the PlayStation

The board sends key positions, not characters; the PlayStation turns them into characters with its own keyboard language setting. Set it to the layout printed on your keyboard, and choose the keyboard type (ISO or ANSI) in the app under "Input". See [Keyboard layout](../README.md#keyboard-layout).

## The app cannot read mouse or keyboard

A normal user may not read keyboard and mouse raw. "Set up access" (in the wizard, and on the Input and Triggers pages) sets up a udev rule once with the administrator password; `pshb-cli list-devices` shows which devices can be read. See [Device access](../README.md#device-access).

## The status page says "normal priority"

The app did not get real-time priority: it asks rtkit, which desktops with PipeWire usually have, or sets it directly if the user may use `SCHED_FIFO` (e.g. via a `realtime` group). Input still works, with more variance under load. See [App](latency-pc.md#app).

## The status page warns about the network adapter

A USB network adapter with the `cdc_ncm` driver holds every outgoing packet back to bundle it with others, up to about 1.2 ms. "Set up" (administrator password, once) lets the app switch that off while it runs; "Undo" on the Latency page takes it back. See [USB network adapters](latency-pc.md#usb-network-adapters).

## The board went back to the previous firmware after an update

A new firmware starts "on trial" and is only accepted once it has started by itself and reaches the network chip; otherwise the board returns to the previous version after a restart, and the app says so. A board without the partition table from the first installation cannot fall back. See [Firmware update over the network](advanced-setup.md#firmware-update-over-the-network).
