#!/usr/bin/env python3
"""Send test traffic to a PS5 HID Bridge board (or the simulator).

Examples:
  udp_test.py --psk-file psk.txt 192.168.1.50 ping
  udp_test.py --plain 192.168.1.50 ping
  udp_test.py --plain 192.168.1.50 pattern
  udp_test.py --plain 192.168.1.50 flood --rate 1000 --seconds 5
"""

from __future__ import annotations

import argparse
import os
import socket
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "protocol", "tools"))
import pshb_proto as proto  # noqa: E402


class Client:
    def __init__(self, host: str, port: int, key: bytes | None):
        self.key = key  # None = plaintext debug protocol
        self.addr = (host, port)
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.settimeout(0.5)
        # Counter starts at the current time in microseconds so it keeps
        # increasing across restarts of this tool.
        self.counter = time.time_ns() // 1000
        self.session_id = 0
        self.ping_seq = 0

    def _next(self) -> int:
        self.counter += 1
        return self.counter

    def send(self, ptype: int, payload: bytes) -> None:
        if self.key is None:
            pkt = proto.plain_packet(ptype, self._next(), payload)
        else:
            pkt = proto.seal(self.key, proto.DIR_APP_TO_BOARD, ptype, self._next(), payload)
        self.sock.sendto(pkt, self.addr)

    def _open(self, data: bytes) -> tuple[int, int, bytes]:
        if self.key is None:
            return proto.parse_plain(data)
        return proto.open_packet(self.key, proto.DIR_BOARD_TO_APP, data)

    def send_input(self, **kw) -> None:
        self.send(proto.TYPE_INPUT, proto.Input(session_id=self.session_id, **kw).encode())

    def ping(self) -> tuple[proto.Pong, float] | None:
        self.ping_seq += 1
        t0 = time.monotonic_ns() // 1000
        self.send(proto.TYPE_PING, proto.Ping(t0, self.ping_seq).encode())
        deadline = time.monotonic() + 0.5
        while time.monotonic() < deadline:
            try:
                data, _ = self.sock.recvfrom(2048)
            except socket.timeout:
                return None
            try:
                ptype, _, payload = self._open(data)
            except ValueError:
                continue
            if ptype != proto.TYPE_PONG:
                continue
            pong = proto.Pong.decode(payload)
            self.session_id = pong.session_id
            if pong.seq == self.ping_seq:
                rtt_ms = (time.monotonic_ns() // 1000 - pong.t_app_us) / 1000
                self.session_id = pong.session_id
                return pong, rtt_ms
        return None


def describe(p: proto.Pong) -> str:
    flags = []
    if p.status & proto.STATUS_USB_MOUNTED:
        flags.append("USB verbunden")
    if p.status & proto.STATUS_USB_SUSPENDED:
        flags.append("USB im Ruhemodus")
    if p.status & proto.STATUS_FAILSAFE:
        flags.append("Failsafe aktiv")
    if p.status & proto.STATUS_PLAINTEXT:
        flags.append("UNVERSCHLÜSSELT")
    return (f"Firmware {p.fw_version_str}, Laufzeit {p.uptime_ms / 1000:.1f} s, "
            f"Session {p.session_id:016x}, {', '.join(flags) or 'USB getrennt'}\n"
            f"  Pakete: ok {p.rx_ok}, Signatur falsch {p.rx_auth_fail}, Replay {p.rx_replay}, "
            f"gefiltert {p.rx_filtered}, fehlerhaft {p.rx_malformed}")


def cmd_ping(c: Client, args) -> int:
    lost = 0
    last = None
    for _ in range(args.count):
        r = c.ping()
        if r is None:
            lost += 1
            print("Zeitüberschreitung")
        else:
            last, rtt = r
            print(f"Antwort seq={last.seq} Laufzeit={rtt:.2f} ms")
        time.sleep(args.interval)
    if last:
        print(describe(last))
    return 1 if lost == args.count else 0


def cmd_pattern(c: Client, args) -> int:
    if c.ping() is None:
        print("Keine Antwort vom Board.")
        return 1
    print("Maus fährt ein Quadrat ...")
    for direction in [(1, 0), (0, 1), (-1, 0), (0, -1)]:
        for _ in range(100):
            c.send_input(dx=direction[0] * 3, dy=direction[1] * 3)
            time.sleep(0.005)
    print("Tippe 'hallo' ...")
    usages = {"h": 0x0B, "a": 0x04, "l": 0x0F, "o": 0x12}
    for ch in "hallo":
        c.send_input(keys=[usages[ch]])
        time.sleep(0.03)
        c.send_input()
        time.sleep(0.03)
    c.send(proto.TYPE_INPUT, proto.Input(session_id=c.session_id, flags=proto.FLAG_RELEASE_ALL).encode())
    print("Fertig.")
    return 0


def cmd_flood(c: Client, args) -> int:
    before = c.ping()
    if before is None:
        print("Keine Antwort vom Board.")
        return 1
    total = int(args.rate * args.seconds)
    interval = 1.0 / args.rate
    start = time.perf_counter()
    for i in range(total):
        # +1/-1 alternating: the cursor stays where it is.
        c.send_input(dx=1 if i % 2 == 0 else -1)
        target = start + (i + 1) * interval
        while time.perf_counter() < target:
            pass
    elapsed = time.perf_counter() - start
    time.sleep(0.2)
    after = c.ping()
    if after is None:
        print("Keine Antwort nach dem Senden.")
        return 1
    received = after[0].rx_ok - before[0].rx_ok - 1  # minus the second ping
    loss = total - received
    print(f"{total} Pakete in {elapsed:.2f} s gesendet ({total / elapsed:.0f}/s), "
          f"{received} angekommen, Verlust {loss} ({100 * loss / total:.2f} %)")
    return 0 if loss == 0 else 1


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("host")
    ap.add_argument("--port", type=int, default=proto.UDP_PORT_DEFAULT)
    ap.add_argument("--plain", action="store_true", help="unverschlüsselte Debug-Pakete (Debug-Firmware)")
    ap.add_argument("--psk", help="Pre-Shared Key (64 Hex-Zeichen)")
    ap.add_argument("--psk-file", help="Datei mit dem Pre-Shared Key")
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("ping")
    p.add_argument("--count", type=int, default=5)
    p.add_argument("--interval", type=float, default=0.2)
    sub.add_parser("pattern")
    p = sub.add_parser("flood")
    p.add_argument("--rate", type=float, default=1000)
    p.add_argument("--seconds", type=float, default=5)
    args = ap.parse_args()

    key = None
    if not args.plain:
        text = args.psk or (open(args.psk_file).read() if args.psk_file else None)
        if not text:
            ap.error("--psk, --psk-file oder --plain angeben")
        key = bytes.fromhex("".join(text.split()))
        if len(key) != proto.KEY_LEN:
            ap.error("der Schlüssel muss 64 Hex-Zeichen haben")
    c = Client(args.host, args.port, key)
    return {"ping": cmd_ping, "pattern": cmd_pattern, "flood": cmd_flood}[args.cmd](c, args)


if __name__ == "__main__":
    sys.exit(main())
