#!/usr/bin/env python3
"""Integration tests: drive the board simulator (firmware/host/sim_main.c)
over UDP and check the HID reports it would send to the PS5.

Usage: test_sim.py --sim PATH/TO/pshb-sim
"""

from __future__ import annotations

import argparse
import os
import queue
import re
import socket
import subprocess
import sys
import threading
import time
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "protocol", "tools"))
import pshb_proto as proto  # noqa: E402

SIM = None
SESSION = 0x1122334455667788

REPORT_RE = {
    "KBD": re.compile(r"KBD mods=(?P<mods>[0-9a-f]{2}) keys=(?P<keys>(?:[0-9a-f]{2} ){5}[0-9a-f]{2})"),
    "MOUSE": re.compile(r"MOUSE buttons=(?P<buttons>[0-9a-f]{2}) dx=(?P<dx>-?\d+) dy=(?P<dy>-?\d+) "
                        r"wheel=(?P<wheel>-?\d+) pan=(?P<pan>-?\d+)"),
}


class Sim:
    """Runs the simulator and collects its report lines."""

    def __init__(self, *extra: str):
        self.proc = subprocess.Popen([SIM, "--port", "0", "--session", f"{SESSION:x}", *extra],
                                     stdout=subprocess.PIPE, text=True)
        self.lines: queue.Queue[str] = queue.Queue()
        ready = self.proc.stdout.readline()
        m = re.match(r"READY (\d+)", ready)
        if not m:
            raise RuntimeError(f"simulator did not start: {ready!r}")
        self.port = int(m.group(1))
        threading.Thread(target=self._reader, daemon=True).start()
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.settimeout(0.5)
        self.counter = 1000

    def _reader(self):
        for line in self.proc.stdout:
            self.lines.put(line.strip())

    def close(self):
        self.proc.terminate()
        self.proc.wait(timeout=5)
        self.sock.close()

    # ---- sending

    def raw(self, data: bytes):
        self.sock.sendto(data, ("127.0.0.1", self.port))

    def send(self, ptype: int, payload: bytes):
        self.counter += 1
        self.raw(proto.plain_packet(ptype, self.counter, payload))

    def send_input(self, **kw):
        self.send(proto.TYPE_INPUT, proto.Input(session_id=SESSION, **kw).encode())

    def ping(self) -> proto.Pong | None:
        self.ping_seq = self.counter + 1  # send() uses this value as packet counter
        self.send(proto.TYPE_PING, proto.Ping(12345, self.ping_seq).encode())
        try:
            data, _ = self.sock.recvfrom(2048)
        except socket.timeout:
            return None
        ptype, _, payload = proto.parse_plain(data)
        assert ptype == proto.TYPE_PONG
        return proto.Pong.decode(payload)

    # ---- reports

    def reports(self, kind: str, settle: float = 0.15) -> list[dict]:
        """All report lines of one kind that arrive within `settle` seconds."""
        out = []
        deadline = time.monotonic() + settle
        while True:
            left = deadline - time.monotonic()
            if left <= 0:
                break
            try:
                line = self.lines.get(timeout=left)
            except queue.Empty:
                break
            m = REPORT_RE.get(kind) and REPORT_RE[kind].fullmatch(line)
            if m:
                out.append(m.groupdict())
        return out

    def drain(self):
        self.reports("", settle=0.05)


class PlaintextTests(unittest.TestCase):
    def setUp(self):
        self.sim = Sim("--plaintext")

    def tearDown(self):
        self.sim.close()

    def test_ping_returns_pong(self):
        pong = self.sim.ping()
        self.assertIsNotNone(pong)
        self.assertEqual(pong.seq, self.sim.ping_seq)
        self.assertEqual(pong.t_app_us, 12345)
        self.assertEqual(pong.session_id, SESSION)
        self.assertTrue(pong.status & proto.STATUS_USB_MOUNTED)
        self.assertTrue(pong.status & proto.STATUS_PLAINTEXT)
        self.assertEqual(pong.rx_ok, 1)

    def test_keyboard_state(self):
        self.sim.send_input(mods=0x02, keys=[0x04, 0x1D])
        kbd = self.sim.reports("KBD")
        self.assertEqual(kbd[-1], {"mods": "02", "keys": "04 1d 00 00 00 00"})
        self.sim.send_input()
        kbd = self.sim.reports("KBD")
        self.assertEqual(kbd[-1], {"mods": "00", "keys": "00 00 00 00 00 00"})

    def test_motion_is_not_lost(self):
        for _ in range(10):
            self.sim.send_input(dx=7, dy=-3, wheel=1)
        mouse = self.sim.reports("MOUSE", settle=0.3)
        self.assertEqual(sum(int(m["dx"]) for m in mouse), 70)
        self.assertEqual(sum(int(m["dy"]) for m in mouse), -30)
        self.assertEqual(sum(int(m["wheel"]) for m in mouse), 10)

    def test_buttons(self):
        self.sim.send_input(buttons=0x11)
        mouse = self.sim.reports("MOUSE")
        self.assertEqual(mouse[-1]["buttons"], "11")

    def test_release_all(self):
        self.sim.send_input(mods=0x01, keys=[0x04], buttons=0x01)
        self.sim.drain()
        self.sim.send(proto.TYPE_INPUT, proto.Input(flags=proto.FLAG_RELEASE_ALL, keys=[0x05]).encode())
        self.assertEqual(self.sim.reports("KBD")[-1], {"mods": "00", "keys": "00 00 00 00 00 00"})

    def test_malformed_packets_are_counted(self):
        self.sim.raw(b"\x00")
        self.sim.raw(proto.plain_packet(proto.TYPE_INPUT, 1, b"\x00" * 5))
        self.sim.raw(proto.plain_packet(0x77, 2, b""))
        self.sim.raw(proto.plain_packet(proto.TYPE_PONG, 3, b"\x00" * proto.PONG.size))
        pong = self.sim.ping()
        self.assertEqual(pong.rx_malformed, 4)
        self.assertEqual(pong.rx_ok, 1)
        self.assertEqual(self.sim.reports("KBD"), [])


class FilterTests(unittest.TestCase):
    def test_disallowed_sender_is_ignored(self):
        sim = Sim("--plaintext", "--allow", "10.9.9.9")
        try:
            sim.send_input(keys=[0x04])
            self.assertIsNone(sim.ping())
            self.assertEqual(sim.reports("KBD"), [])
        finally:
            sim.close()

    def test_plaintext_rejected_by_default(self):
        sim = Sim()
        try:
            sim.send_input(keys=[0x04])
            self.assertIsNone(sim.ping())
            self.assertEqual(sim.reports("KBD"), [])
        finally:
            sim.close()


def main() -> int:
    global SIM
    ap = argparse.ArgumentParser()
    ap.add_argument("--sim", required=True)
    args, rest = ap.parse_known_args()
    SIM = os.path.abspath(args.sim)
    prog = unittest.main(argv=[sys.argv[0], *rest], exit=False, verbosity=2)
    return 0 if prog.result.wasSuccessful() else 1


if __name__ == "__main__":
    sys.exit(main())
