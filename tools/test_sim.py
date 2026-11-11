#!/usr/bin/env python3
"""Integration tests: drive the board simulator (firmware/host/sim_main.c)
over UDP and check the HID reports it would send to the PS5.

Usage: test_sim.py --sim PATH/TO/pshb-sim
"""

from __future__ import annotations

import argparse
import json
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
# The version the simulator must report: MAJOR.MINOR.<commit count> (from
# make, or computed the same way).
VERSION = os.environ.get("PSHB_VERSION") or subprocess.run(
    ["bash", os.path.join(os.path.dirname(os.path.abspath(__file__)), "version.sh")],
    check=True, capture_output=True, text=True).stdout.strip()

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
        m = re.match(r"READY (\d+) HTTP (\d+)", ready)
        if not m:
            raise RuntimeError(f"simulator did not start: {ready!r}")
        self.port = int(m.group(1))
        self.http_port = int(m.group(2))
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

    def reports_all(self, settle: float = 0.3) -> dict[str, list[dict]]:
        """KBD and MOUSE reports arriving within `settle` seconds, in one pass."""
        out = {kind: [] for kind in REPORT_RE}
        deadline = time.monotonic() + settle
        while (left := deadline - time.monotonic()) > 0:
            try:
                line = self.lines.get(timeout=left)
            except queue.Empty:
                break
            for kind, rx in REPORT_RE.items():
                if m := rx.fullmatch(line):
                    out[kind].append(m.groupdict())
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
        self.assertEqual(pong.fw_version_str, VERSION)

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
        self.assertIn(f"session {SESSION:016x}", r.stdout)
        self.assertIn("USB connected", r.stdout)

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

    def test_replay_script_end_to_end(self):
        """evdev-like events -> engine (hotkey, grab rules) -> link -> C core -> HID reports."""
        import tempfile
        script = """
            tap A                 # inactive: must not arrive
            combo Ctrl+Alt+P      # activate (after all keys are released)
            wait 30
            key LEFTSHIFT down
            tap H
            key LEFTSHIFT up
            move 30 -10
            wheel 1
            button BTN_LEFT down
            button BTN_LEFT up
            wait 30
            combo Ctrl+Alt+P      # deactivate
            wait 30
            tap B                 # inactive again
        """
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as f:
            f.write(script)
        try:
            r = self.cli("replay", f.name, "--config", f.name + ".none.toml")
        finally:
            os.unlink(f.name)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("Bridge ON", r.stdout)
        self.assertIn("Bridge OFF", r.stdout)
        rep = self.sim.reports_all()
        kbd, mouse = rep["KBD"], rep["MOUSE"]
        keys = [k["keys"].split()[0] for k in kbd]
        self.assertIn({"mods": "02", "keys": "0b 00 00 00 00 00"}, kbd)  # Shift+H
        for code in ("04", "05", "13"):  # A, B (inactive) and P (trigger key)
            self.assertNotIn(code, keys)
        self.assertEqual(kbd[-1], {"mods": "00", "keys": "00 00 00 00 00 00"})
        self.assertEqual(sum(int(m["dx"]) for m in mouse), 30)
        self.assertEqual(sum(int(m["dy"]) for m in mouse), -10)
        self.assertEqual(sum(int(m["wheel"]) for m in mouse), 1)
        self.assertIn("01", [m["buttons"] for m in mouse])  # the click arrived
        self.assertEqual(mouse[-1]["buttons"], "00")

    def test_wrong_key(self):
        r = self.cli("ping", "--count", "1", key=bytes(32))
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("No answer", r.stderr)


class FilterTests(unittest.TestCase):
    def test_disallowed_sender_is_ignored(self):
        sim = Sim("--plaintext", "--allow", "10.9.9.9")
        try:
            sim.send_input(keys=[0x04])
            self.assertIsNone(sim.ping())
            self.assertEqual(sim.reports("KBD"), [])
        finally:
            sim.close()

    def test_allowed_subnet_option(self):
        sim = Sim("--plaintext", "--allow", "127.0.0.0/8")
        try:
            self.assertIsNotNone(sim.ping())
        finally:
            sim.close()
        sim = Sim("--plaintext", "--allow", "10.0.0.0/8")
        try:
            self.assertIsNone(sim.ping())
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


class Web:
    """Minimal HTTP client for the board API (manual cookie handling)."""

    def __init__(self, port: int):
        self.base = f"http://127.0.0.1:{port}"
        self.cookie = ""

    def req(self, method: str, path: str, body=None, csrf: bool = True):
        import urllib.error
        import urllib.request
        data = None if body is None else json.dumps(body).encode()
        r = urllib.request.Request(self.base + path, data=data, method=method)
        if data is not None:
            r.add_header("Content-Type", "application/json")
        if csrf and method == "POST":
            r.add_header("X-PSHB", "1")
        if self.cookie:
            r.add_header("Cookie", self.cookie)
        try:
            with urllib.request.urlopen(r, timeout=5) as resp:
                status, headers, raw = resp.status, resp.headers, resp.read()
        except urllib.error.HTTPError as e:
            status, headers, raw = e.code, e.headers, e.read()
        for c in headers.get_all("Set-Cookie") or []:
            name_value = c.split(";", 1)[0]
            self.cookie = "" if name_value.endswith("=") else name_value
        if headers.get("Content-Encoding") == "gzip":
            import gzip
            raw = gzip.decompress(raw)
        try:
            payload = json.loads(raw) if raw else {}
        except ValueError:
            payload = raw
        return status, payload, headers

    def upload(self, path: str, data: bytes, crc: int | None):
        import urllib.error
        import urllib.request
        r = urllib.request.Request(self.base + path, data=data, method="POST")
        r.add_header("X-PSHB", "1")
        r.add_header("Content-Type", "application/octet-stream")
        if crc is not None:
            r.add_header("X-PSHB-CRC32", f"{crc:x}")
        if self.cookie:
            r.add_header("Cookie", self.cookie)
        try:
            with urllib.request.urlopen(r, timeout=10) as resp:
                return resp.status, json.loads(resp.read() or b"{}")
        except urllib.error.HTTPError as e:
            return e.code, json.loads(e.read() or b"{}")
        except (ConnectionResetError, BrokenPipeError):
            return 0, {}

    def login(self, password: str):
        return self.req("POST", "/api/login", {"password": password})


NEW_PW = "SicheresPasswort1"


def make_uf2(blocks: int, abs_block: bool = True) -> bytes:
    """Synthetic UF2 for the RP2350 (plus the SDK's RP2350-E10 absolute block)."""
    import struct

    def block(addr, no, total, family, fill):
        payload = bytes((fill + i) & 0xFF for i in range(256)).ljust(476, b"\0")
        return struct.pack("<8I", 0x0A324655, 0x9E5D5157, 0x2000, addr, 256, no, total, family) + payload + \
            struct.pack("<I", 0x0AB16F30)

    out = b""
    if abs_block:
        out += block(0x10FFFF00, 0, 1, 0xE48BFF57, 0xEE)
    for i in range(blocks):
        out += block(0x10000000 + 256 * i, i, blocks, 0xE48BFF59, i)
    return out


class WebTests(unittest.TestCase):
    def setUp(self):
        import tempfile
        self.tmp = tempfile.mkdtemp()
        self.settings = os.path.join(self.tmp, "flash.bin")
        self.sim = Sim("--http-port", "0", "--settings", self.settings)
        self.web = Web(self.sim.http_port)

    def tearDown(self):
        self.sim.close()
        import shutil
        shutil.rmtree(self.tmp, ignore_errors=True)

    def setup_password(self):
        self.assertEqual(self.web.login("ps5-hid-bridge")[0], 200)
        self.assertEqual(self.web.req("POST", "/api/password", {"old": "ps5-hid-bridge", "new": NEW_PW})[0], 200)

    def test_static_files_and_headers(self):
        status, body, headers = self.web.req("GET", "/")
        self.assertEqual(status, 200)
        self.assertIn(b"PS5 HID Bridge", body)
        self.assertIn("default-src 'self'", headers["Content-Security-Policy"])
        self.assertEqual(headers["X-Frame-Options"], "DENY")
        self.assertEqual(self.web.req("GET", "/app.js")[0], 200)
        self.assertEqual(self.web.req("GET", "/nope")[0], 404)

    def test_login_and_forced_password_change(self):
        st = self.web.req("GET", "/api/state")[1]
        self.assertEqual(st, {"logged_in": False, "must_change_password": True})
        self.assertEqual(self.web.req("GET", "/api/status")[0], 401)
        self.assertEqual(self.web.req("POST", "/api/login", {"password": "ps5-hid-bridge"}, csrf=False)[0], 403)
        self.assertEqual(self.web.login("falsch")[0], 401)
        status, r, _ = self.web.login("ps5-hid-bridge")
        self.assertEqual(status, 200)
        self.assertTrue(r["must_change_password"])
        self.assertTrue(self.web.cookie.startswith("pshb_session="))
        # Nothing but the password change works yet.
        self.assertEqual(self.web.req("GET", "/api/status")[0], 403)
        self.assertEqual(self.web.req("POST", "/api/password", {"old": "ps5-hid-bridge", "new": "kurz"})[0], 400)
        self.assertEqual(self.web.req("POST", "/api/password", {"old": "ps5-hid-bridge", "new": "ps5-hid-bridge"})[0], 400)
        self.assertEqual(self.web.req("POST", "/api/password", {"old": "x", "new": NEW_PW})[0], 403)
        self.assertEqual(self.web.req("POST", "/api/password", {"old": "ps5-hid-bridge", "new": NEW_PW})[0], 200)
        status, st, _ = self.web.req("GET", "/api/status")
        self.assertEqual(status, 200)
        self.assertFalse(st["psk_set"])
        self.assertEqual(st["udp_port"], 45045)
        self.assertEqual(st["firmware"], VERSION)
        # Latency diagnostics are part of the status (the simulator has no USB timing).
        for k in ("lat_usb_count", "lat_usb_p50_us", "lat_usb_p95_us", "lat_usb_max_us",
                  "lat_rx_p50_us", "lat_rx_max_us", "mouse_waits"):
            self.assertEqual(st[k], 0, k)
        # Logout, then only the new password works.
        self.assertEqual(self.web.req("POST", "/api/logout", {})[0], 200)
        self.assertEqual(self.web.req("GET", "/api/status")[0], 401)
        self.assertEqual(self.web.login("ps5-hid-bridge")[0], 401)
        self.assertEqual(self.web.login(NEW_PW)[0], 200)

    def test_login_lockout(self):
        for _ in range(3):
            self.assertEqual(self.web.login("falsch")[0], 401)
        status, r, _ = self.web.login("ps5-hid-bridge")
        self.assertEqual(status, 429)
        self.assertIn("wait", r["error"])

    def test_generated_psk_is_used_by_the_input_path(self):
        self.setup_password()
        status, r, _ = self.web.req("POST", "/api/psk/generate", {})
        self.assertEqual(status, 200)
        key = bytes.fromhex(r["psk"])
        self.sim.key = key
        pong = self.sim.ping()
        self.assertIsNotNone(pong)
        self.sim.key = bytes(32)
        self.assertIsNone(self.sim.ping())
        self.assertTrue(self.web.req("GET", "/api/status")[1]["psk_set"])

    def test_allowed_sender(self):
        self.setup_password()
        self.web.req("POST", "/api/psk", {"psk": KEY.hex()})
        self.sim.key = KEY
        self.assertEqual(self.web.req("POST", "/api/allowed-sender", {"ip": "10.1.2.3"})[0], 200)
        self.assertIsNone(self.sim.ping())
        self.assertEqual(self.web.req("POST", "/api/allowed-sender", {"ip": "127.0.0.1"})[0], 200)
        self.assertIsNotNone(self.sim.ping())
        self.assertEqual(self.web.req("POST", "/api/allowed-sender", {"ip": "999.1.1.1"})[0], 400)
        self.assertEqual(self.web.req("GET", "/api/status")[1]["allowed_sender"], "127.0.0.1")

    def test_allowed_subnet(self):
        self.setup_password()
        self.web.req("POST", "/api/psk", {"psk": KEY.hex()})
        self.sim.key = KEY
        for bad in ("10.0.0.0/33", "10.0.0.0/", "10.0.0.0/8x", "10.0.0.0/-1", "10.0.0.0/8/8", "0.0.0.0/8"):
            self.assertEqual(self.web.req("POST", "/api/allowed-sender", {"ip": bad})[0], 400, bad)
        self.assertEqual(self.web.req("POST", "/api/allowed-sender", {"ip": "10.0.0.0/8"})[0], 200)
        self.assertIsNone(self.sim.ping())
        # Host bits are cleared: 127.0.0.9/8 is stored as 127.0.0.0/8.
        self.assertEqual(self.web.req("POST", "/api/allowed-sender", {"ip": "127.0.0.9/8"})[0], 200)
        self.assertEqual(self.web.req("GET", "/api/status")[1]["allowed_sender"], "127.0.0.0/8")
        self.assertIsNotNone(self.sim.ping())
        # /32 is a single address, /0 means any sender.
        self.assertEqual(self.web.req("POST", "/api/allowed-sender", {"ip": "127.0.0.1/32"})[0], 200)
        self.assertEqual(self.web.req("GET", "/api/status")[1]["allowed_sender"], "127.0.0.1")
        self.assertEqual(self.web.req("POST", "/api/allowed-sender", {"ip": "10.1.2.3/0"})[0], 200)
        self.assertEqual(self.web.req("GET", "/api/status")[1]["allowed_sender"], "")
        self.assertIsNotNone(self.sim.ping())

    def test_network_settings_validation_and_reboot(self):
        self.setup_password()
        bad = {"mode": "static", "ip": "192.168.1.50", "mask": "255.0.255.0", "gateway": "", "dns": "",
               "udp_port": 45045}
        self.assertEqual(self.web.req("POST", "/api/network", bad)[0], 400)
        self.assertEqual(self.web.req("POST", "/api/network", {**bad, "mask": "255.255.255.0", "udp_port": 0})[0], 400)
        good = {**bad, "mask": "255.255.255.0", "gateway": "192.168.1.1", "udp_port": 45046}
        status, r, _ = self.web.req("POST", "/api/network", good)
        self.assertEqual((status, r), (200, {"ok": True, "reboot": True}))
        self.assertTrue(self.sim.wait_line("REBOOT", timeout=2))
        # Sessions do not survive a reboot.
        self.assertEqual(self.web.req("GET", "/api/status")[0], 401)
        self.assertEqual(self.web.login(NEW_PW)[0], 200)
        st = self.web.req("GET", "/api/status")[1]
        self.assertEqual((st["ip_mode"], st["static_ip"], st["udp_port"]), ("static", "192.168.1.50", 45046))

    def test_factory_reset(self):
        self.setup_password()
        self.web.req("POST", "/api/psk/generate", {})
        self.assertEqual(self.web.req("POST", "/api/factory-reset", {})[0], 200)
        self.assertTrue(self.sim.wait_line("REBOOT", timeout=2))
        status, r, _ = self.web.login("ps5-hid-bridge")
        self.assertEqual(status, 200)
        self.assertTrue(r["must_change_password"])

    def test_firmware_upload(self):
        import zlib
        self.setup_password()
        uf2 = make_uf2(100)
        status, r = self.web.upload("/api/ota", uf2, zlib.crc32(uf2))
        self.assertEqual((status, r), (200, {"ok": True, "reboot": True}))
        self.assertTrue(self.sim.wait_line("OTA OK 100", timeout=2))
        self.assertTrue(self.sim.wait_line("REBOOT", timeout=2))

    def test_firmware_upload_rejects_bad_files(self):
        import zlib
        self.setup_password()
        uf2 = make_uf2(10)
        status, r = self.web.upload("/api/ota", uf2, zlib.crc32(uf2) ^ 1)
        self.assertEqual(status, 400)
        self.assertIn("Checksum", r["error"])
        junk = bytes(range(256)) * 4
        status, r = self.web.upload("/api/ota", junk, zlib.crc32(junk))
        self.assertEqual(status, 400)
        self.assertIn("UF2", r["error"])
        status, r = self.web.upload("/api/ota", uf2, None)
        self.assertEqual(status, 400)
        self.assertFalse(self.sim.wait_line("REBOOT", timeout=0.3))

    def test_only_one_firmware_upload_at_a_time(self):
        import zlib
        self.setup_password()
        uf2 = make_uf2(100)
        # First upload: headers and part of the body, connection kept open.
        first = socket.create_connection(("127.0.0.1", self.sim.http_port), timeout=5)
        head = (f"POST /api/ota HTTP/1.1\r\nHost: x\r\nX-PSHB: 1\r\nCookie: {self.web.cookie}\r\n"
                f"X-PSHB-CRC32: {zlib.crc32(uf2):x}\r\nContent-Length: {len(uf2)}\r\n\r\n")
        first.sendall(head.encode() + uf2[:4096])
        self.assertTrue(self.sim.wait_line(f"OTA BEGIN {len(uf2)}", timeout=2))
        status, r = self.web.upload("/api/ota", uf2, zlib.crc32(uf2))
        self.assertEqual(status, 409)
        self.assertIn("in progress", r["error"])
        # The interrupted upload is aborted when its connection goes away.
        first.close()
        time.sleep(0.2)
        status, r = self.web.upload("/api/ota", uf2, zlib.crc32(uf2))
        self.assertEqual((status, r), (200, {"ok": True, "reboot": True}))

    def test_udp_port_rules(self):
        self.setup_password()
        body = {"mode": "dhcp", "udp_port": 68}
        self.assertEqual(self.web.req("POST", "/api/network", body)[0], 400)
        body["udp_port"] = 67
        self.assertEqual(self.web.req("POST", "/api/network", body)[0], 400)
        # "12abc" is not 12
        import urllib.error
        import urllib.request
        req = urllib.request.Request(self.web.base + "/api/network", data=b'{"mode":"dhcp","udp_port":12abc}',
                                     method="POST", headers={"X-PSHB": "1", "Cookie": self.web.cookie,
                                                             "Content-Type": "application/json"})
        with self.assertRaises(urllib.error.HTTPError) as e:
            urllib.request.urlopen(req, timeout=5)
        self.assertEqual(e.exception.code, 400)
        # 80 is a TCP port: no conflict for UDP input.
        self.assertEqual(self.web.req("POST", "/api/network", {"mode": "dhcp", "udp_port": 80})[0], 200)

    def test_firmware_upload_needs_login(self):
        import zlib
        uf2 = make_uf2(4)
        status, _ = self.web.upload("/api/ota", uf2, zlib.crc32(uf2))
        self.assertIn(status, (0, 401))
        self.assertFalse(self.sim.wait_line(f"OTA BEGIN {len(uf2)}", timeout=0.3))

    def test_settings_survive_restart(self):
        self.setup_password()
        self.sim.close()
        self.sim = Sim("--http-port", "0", "--settings", self.settings)
        self.web = Web(self.sim.http_port)
        self.assertEqual(self.web.login("ps5-hid-bridge")[0], 401)
        self.assertEqual(self.web.login(NEW_PW)[0], 200)


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
