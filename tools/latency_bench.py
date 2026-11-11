#!/usr/bin/env python3
"""Measure the desktop side's input latency: from an input event to the UDP
packet leaving the app.

The script creates a virtual mouse via /dev/uinput, plays the board itself
(answers PINGs, records INPUT packets with kernel receive timestamps) and
runs `pshb-cli run` against it. It switches the bridge on with F24, then
moves the virtual mouse by one count at a time and measures how long each
movement takes to arrive as a packet. Real keyboards and mice are not
touched: only the virtual device is selected and grabbed.

Requirements: write access to /dev/uinput, and read access to the new
event node (e.g. a udev rule for the chosen --vendor/--product), Python
package "cryptography".

Usage:
  tools/latency_bench.py --cli out/pshb-cli --vendor VVVV --product PPPP [--count 2000] [--load 16]
"""

import argparse
import fcntl
import multiprocessing
import os
import queue
import random
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "protocol", "tools"))
import pshb_proto as proto  # noqa: E402

NAME = "pshb-latency-bench"
EV_SYN, EV_KEY, EV_REL = 0, 1, 2
BTN_LEFT, KEY_F24, REL_X, REL_Y = 0x110, 194, 0, 1
UI_SET_EVBIT, UI_SET_KEYBIT, UI_SET_RELBIT = 0x40045564, 0x40045565, 0x40045566
UI_DEV_SETUP, UI_DEV_CREATE, UI_DEV_DESTROY = 0x405C5503, 0x5501, 0x5502
SO_TIMESTAMPNS = 35
SESSION = 0x5E55_1011_BE4C_0001
KEY = bytes(range(32))


class VirtualMouse:
    def __init__(self, vendor: int, product: int):
        self.fd = os.open("/dev/uinput", os.O_WRONLY | os.O_NONBLOCK)
        for ev in (EV_SYN, EV_KEY, EV_REL):
            fcntl.ioctl(self.fd, UI_SET_EVBIT, ev)
        for key in (BTN_LEFT, KEY_F24):
            fcntl.ioctl(self.fd, UI_SET_KEYBIT, key)
        for rel in (REL_X, REL_Y):
            fcntl.ioctl(self.fd, UI_SET_RELBIT, rel)
        setup = struct.pack("<HHHH80sI", 0x03, vendor, product, 1, NAME.encode(), 0)  # BUS_USB
        fcntl.ioctl(self.fd, UI_DEV_SETUP, setup)
        fcntl.ioctl(self.fd, UI_DEV_CREATE)

    def emit(self, *events):
        data = b"".join(struct.pack("qqHHi", 0, 0, t, c, v) for t, c, v in events)
        data += struct.pack("qqHHi", 0, 0, EV_SYN, 0, 0)
        os.write(self.fd, data)

    def close(self):
        fcntl.ioctl(self.fd, UI_DEV_DESTROY)
        os.close(self.fd)


def find_event_node(timeout=3.0):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        for ev in os.listdir("/sys/class/input"):
            try:
                with open(f"/sys/class/input/{ev}/device/name") as f:
                    if f.read().strip() == NAME and ev.startswith("event"):
                        path = f"/dev/input/{ev}"
                        if os.access(path, os.R_OK):
                            return path
            except OSError:
                pass
        time.sleep(0.05)
    return None


class FakeBoard(threading.Thread):
    """Answers PINGs; puts (kernel receive time in ns, dx) of every INPUT packet into a queue."""

    def __init__(self):
        super().__init__(daemon=True)
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.bind(("127.0.0.1", 0))
        self.sock.setsockopt(socket.SOL_SOCKET, SO_TIMESTAMPNS, 1)
        self.inputs = queue.Queue()
        self.counter = 1000

    @property
    def port(self):
        return self.sock.getsockname()[1]

    def run(self):
        while True:
            data, anc, _, addr = self.sock.recvmsg(512, 256)
            t_ns = time.time_ns()
            for level, ctype, cdata in anc:
                if level == socket.SOL_SOCKET and ctype == SO_TIMESTAMPNS:
                    sec, nsec = struct.unpack("qq", cdata[:16])
                    t_ns = sec * 1_000_000_000 + nsec
            try:
                ptype, _, payload = proto.open_packet(KEY, proto.DIR_APP_TO_BOARD, data)
            except ValueError:
                continue
            if ptype == proto.TYPE_PING:
                t_app, seq = proto.PING.unpack(payload)
                self.counter += 1
                pong = proto.PONG.pack(t_app, seq, SESSION, 0x000900, 0, proto.STATUS_USB_MOUNTED, 0, 0, 0, 0, 0)
                self.sock.sendto(proto.seal(KEY, proto.DIR_BOARD_TO_APP, proto.TYPE_PONG, self.counter, pong), addr)
            elif ptype == proto.TYPE_INPUT:
                fields = proto.INPUT.unpack(payload)
                self.inputs.put((t_ns, fields[5]))


def burn():
    while True:
        pass


def percentile(v, q):
    return v[min(len(v) - 1, max(0, int(round(q * len(v))) - 1))]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--cli", required=True, help="pshb-cli binary to measure")
    ap.add_argument("--count", type=int, default=2000)
    ap.add_argument("--load", type=int, default=0, help="busy processes running during the measurement")
    ap.add_argument("--vendor", type=lambda s: int(s, 16), required=True,
                    help="USB vendor id (hex) for the virtual mouse; pick one your udev rule grants access to")
    ap.add_argument("--product", type=lambda s: int(s, 16), required=True, help="USB product id (hex)")
    ap.add_argument("--gap-ms", type=float, default=3.0, help="mean pause between movements")
    args = ap.parse_args()

    board = FakeBoard()
    board.start()
    mouse = VirtualMouse(args.vendor, args.product)
    procs, cli = [], None
    try:
        if not find_event_node():
            sys.exit(f"{NAME}: event node did not become readable (udev rule for "
                     f"{args.vendor:04x}:{args.product:04x}?)")
        home = tempfile.mkdtemp(prefix="pshb-bench-")
        env = dict(os.environ, XDG_CONFIG_HOME=home, XDG_STATE_HOME=home)
        cli = subprocess.Popen(
            [args.cli, "run", "--board", f"127.0.0.1:{board.port}", "--psk", KEY.hex(),
             "--device", NAME, "--hotkey", "F24", "--stats"],
            env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        lines = queue.Queue()
        threading.Thread(target=lambda: [lines.put(l) for l in cli.stdout], daemon=True).start()

        def wait_for(text, timeout=5.0):
            end = time.monotonic() + timeout
            while time.monotonic() < end:
                try:
                    line = lines.get(timeout=0.1)
                except queue.Empty:
                    continue
                if text in line:
                    return line
            sys.exit(f"pshb-cli did not print {text!r}")

        wait_for("Ready.")
        print(wait_for("Priority:").strip())
        mouse.emit((EV_KEY, KEY_F24, 1))
        mouse.emit((EV_KEY, KEY_F24, 0))
        wait_for("Bridge ON")
        time.sleep(0.2)
        while not board.inputs.empty():
            board.inputs.get()

        procs = [multiprocessing.Process(target=burn, daemon=True) for _ in range(args.load)]
        for p in procs:
            p.start()
        time.sleep(0.5 if procs else 0)

        samples, lost = [], 0
        for _ in range(args.count):
            time.sleep(random.uniform(0.5, 1.5) * args.gap_ms / 1000)
            t0 = time.time_ns()
            mouse.emit((EV_REL, REL_X, 1))
            try:
                t1, dx = board.inputs.get(timeout=0.5)
            except queue.Empty:
                lost += 1
                continue
            if dx != 1:
                lost += 1
                continue
            samples.append((t1 - t0) / 1000)
        for p in procs:
            p.terminate()

        mouse.emit((EV_KEY, KEY_F24, 1))
        mouse.emit((EV_KEY, KEY_F24, 0))
        wait_for("Bridge OFF")
    finally:
        for p in procs:
            p.terminate()
        if cli:
            cli.terminate()
            cli.wait(timeout=5)
        mouse.close()

    samples.sort()
    print(f"samples {len(samples)}, lost {lost}, load {args.load}")
    print("input event -> UDP packet (µs): "
          f"p50 {percentile(samples, 0.5):.0f}  p90 {percentile(samples, 0.9):.0f}  "
          f"p99 {percentile(samples, 0.99):.0f}  p99.9 {percentile(samples, 0.999):.0f}  max {samples[-1]:.0f}")


if __name__ == "__main__":
    main()
