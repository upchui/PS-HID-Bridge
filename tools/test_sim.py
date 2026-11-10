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
CLI = None
SESSION = 0x1122334455667788

REPORT_RE = {
    "KBD": re.compile(r"KBD mods=(?P<mods>[0-9a-f]{2}) keys=(?P<keys>(?:[0-9a-f]{2} ){5}[0-9a-f]{2})"),
    "MOUSE": re.compile(r"MOUSE buttons=(?P<buttons>[0-9a-f]{2}) dx=(?P<dx>-?\d+) dy=(?P<dy>-?\d+) "
                        r"wheel=(?P<wheel>-?\d+) pan=(?P<pan>-?\d+)"),
}


KEY = bytes(range(32))


class Sim:
    """Runs the simulator and collects its report lines.

    With `key` set, packets are encrypted (protocol v1); otherwise the
    simulator must be started with --plaintext.
    """

    def __init__(self, *extra: str, key: bytes | None = None):
        self.key = key
        if key is not None:
            extra = ("--psk", key.hex(), *extra)
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

    def packet(self, ptype: int, payload: bytes) -> bytes:
        self.counter += 1
        if self.key is None:
            return proto.plain_packet(ptype, self.counter, payload)
        return proto.seal(self.key, proto.DIR_APP_TO_BOARD, ptype, self.counter, payload)

    def send(self, ptype: int, payload: bytes):
        self.raw(self.packet(ptype, payload))

    def receive_pong(self) -> proto.Pong | None:
        try:
            data, _ = self.sock.recvfrom(2048)
        except socket.timeout:
            return None
        if self.key is None:
            ptype, _, payload = proto.parse_plain(data)
        else:
            ptype, _, payload = proto.open_packet(self.key, proto.DIR_BOARD_TO_APP, data)
        assert ptype == proto.TYPE_PONG
        return proto.Pong.decode(payload)

    def send_input(self, **kw):
        self.send(proto.TYPE_INPUT, proto.Input(session_id=SESSION, **kw).encode())

    def ping(self) -> proto.Pong | None:
        self.ping_seq = self.counter + 1  # send() uses this value as packet counter
        self.send(proto.TYPE_PING, proto.Ping(12345, self.ping_seq).encode())
        return self.receive_pong()

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

    def wait_line(self, text: str, timeout: float) -> bool:
        deadline = time.monotonic() + timeout
        while (left := deadline - time.monotonic()) > 0:
            try:
                if self.lines.get(timeout=left) == text:
                    return True
            except queue.Empty:
                break
        return False


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


class EncryptedTests(unittest.TestCase):
    def setUp(self):
        self.sim = Sim(key=KEY)

    def tearDown(self):
        self.sim.close()

    def test_ping_and_input(self):
        pong = self.sim.ping()
        self.assertIsNotNone(pong)
        self.assertEqual(pong.session_id, SESSION)
        self.assertFalse(pong.status & proto.STATUS_PLAINTEXT)
        self.assertTrue(pong.status & proto.STATUS_FAILSAFE)  # nothing received yet
        self.sim.send_input(keys=[0x04], dx=5)
        self.assertEqual(self.sim.reports("KBD")[-1]["keys"], "04 00 00 00 00 00")
        pong = self.sim.ping()
        self.assertFalse(pong.status & proto.STATUS_FAILSAFE)
        self.assertEqual(pong.rx_ok, 3)

    def test_plaintext_is_rejected(self):
        self.sim.raw(proto.plain_packet(proto.TYPE_INPUT, 99, proto.Input(SESSION, keys=[4]).encode()))
        self.assertEqual(self.sim.reports("KBD"), [])
        self.assertEqual(self.sim.ping().rx_malformed, 1)

    def test_wrong_key_is_rejected(self):
        self.sim.counter += 1
        self.sim.raw(proto.seal(bytes(32), proto.DIR_APP_TO_BOARD, proto.TYPE_INPUT, self.sim.counter,
                                proto.Input(SESSION, keys=[4]).encode()))
        self.assertEqual(self.sim.reports("KBD"), [])
        self.assertEqual(self.sim.ping().rx_auth_fail, 1)

    def test_replay_is_rejected(self):
        pkt = self.sim.packet(proto.TYPE_INPUT, proto.Input(SESSION, dx=10).encode())
        self.sim.raw(pkt)
        self.sim.raw(pkt)
        mouse = self.sim.reports("MOUSE")
        self.assertEqual(sum(int(m["dx"]) for m in mouse), 10)
        self.assertEqual(self.sim.ping().rx_replay, 1)

    def test_wrong_session_gets_hint(self):
        self.sim.send(proto.TYPE_INPUT, proto.Input(0xDEAD, keys=[4]).encode())
        hint = self.sim.receive_pong()
        self.assertIsNotNone(hint)
        self.assertEqual((hint.t_app_us, hint.seq), (0, 0))
        self.assertEqual(hint.session_id, SESSION)
        self.assertEqual(self.sim.reports("KBD"), [])

    def test_failsafe_releases_keys(self):
        self.sim.send_input(keys=[0x04], buttons=0x01)
        self.assertEqual(self.sim.reports("KBD")[-1]["keys"], "04 00 00 00 00 00")
        self.assertTrue(self.sim.wait_line("FAILSAFE", timeout=1.0))
        kbd = self.sim.reports("KBD")
        self.assertEqual(kbd[-1]["keys"], "00 00 00 00 00 00")

    def test_heartbeat_keeps_keys_pressed(self):
        self.sim.send_input(keys=[0x04])
        for _ in range(12):  # 600 ms of heartbeats every 50 ms
            time.sleep(0.05)
            self.sim.send(proto.TYPE_HEARTBEAT, proto.Input(SESSION, keys=[0x04]).encode())
        self.assertFalse(self.sim.wait_line("FAILSAFE", timeout=0.05))
        self.assertTrue(self.sim.wait_line("FAILSAFE", timeout=1.0))


class InteropTests(unittest.TestCase):
    """Rust desktop core (pshb-cli) against the C firmware core (simulator)."""

    def setUp(self):
        if not os.environ.get("PSHB_CLI"):
            self.skipTest("no --cli given")
        self.sim = Sim(key=KEY)
        self.board = f"127.0.0.1:{self.sim.port}"

    def tearDown(self):
        self.sim.close()

    def cli(self, *args: str, key: bytes = KEY) -> subprocess.CompletedProcess:
        return subprocess.run([os.environ["PSHB_CLI"], args[0], "--board", self.board, "--psk", key.hex(),
                               *args[1:]], capture_output=True, text=True, timeout=20)

    def test_ping(self):
        r = self.cli("ping", "--count", "2")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn(f"Session {SESSION:016x}", r.stdout)
        self.assertIn("USB verbunden", r.stdout)

    def test_send_keys_and_motion(self):
        r = self.cli("send", "--keys", "04,1d", "--mods", "02", "--dx", "30", "--dy", "-7", "--hold-ms", "100")
        self.assertEqual(r.returncode, 0, r.stderr)
        kbd = self.sim.reports("KBD", settle=0.3)
        self.assertIn({"mods": "02", "keys": "04 1d 00 00 00 00"}, kbd)
        self.assertEqual(kbd[-1], {"mods": "00", "keys": "00 00 00 00 00 00"})  # released at the end

    def test_motion_sum(self):
        r = self.cli("send", "--dx", "40000", "--dy", "-3", "--hold-ms", "50")
        self.assertEqual(r.returncode, 0, r.stderr)
        mouse = self.sim.reports("MOUSE", settle=0.3)
        self.assertEqual(sum(int(m["dx"]) for m in mouse), 40000)
        self.assertEqual(sum(int(m["dy"]) for m in mouse), -3)

    def test_wrong_key(self):
        r = self.cli("ping", "--count", "1", key=bytes(32))
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("Keine Antwort", r.stderr)


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
    ap.add_argument("--cli", help="pshb-cli binary for the Rust/C interop tests")
    args, rest = ap.parse_known_args()
    SIM = os.path.abspath(args.sim)
    if args.cli:
        os.environ["PSHB_CLI"] = os.path.abspath(args.cli)
    prog = unittest.main(argv=[sys.argv[0], *rest], exit=False, verbosity=2)
    return 0 if prog.result.wasSuccessful() else 1


if __name__ == "__main__":
    sys.exit(main())
