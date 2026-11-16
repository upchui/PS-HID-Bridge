#!/usr/bin/env python3
"""Measure the board's input path under load (docs/latency.md).

Plays the app: sends encrypted INPUT packets at 1000/s (mouse moving by
one count back and forth, so the cursor stays where it is), optionally
loads the web interface or uploads to the upload sink of the timing image
at the same time, then reads the board's own measurements. The board must
be connected to a USB host that polls it (PlayStation, PC); switch the
bridge in the app off meanwhile.

Scenarios (each starts with fresh statistics):
  S0  idle: no packets (core 0's SPI traffic without input)
  S1  mouse
  S2  mouse + web interface (pages, scripts, status in a loop)
  S3  mouse + continuous upload to /api/bench/sink (timing image only)

Password and key are read from files (never pass them on the command line):
  tools/fw_bench.py 10.14.0.21 --password-file PW --psk-file KEY \\
      [--install out/ps-hid-bridge-timing.uf2] [--scenarios S0,S1,S2,S3] \\
      [--seconds 30] [--label step1] [--out out/bench]
"""

from __future__ import annotations

import argparse
import http.client
import json
import multiprocessing as mp
import os
import random
import socket
import struct
import sys
import time
import zlib

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "protocol", "tools"))
import pshb_proto as proto  # noqa: E402

RATE = 1000  # packets per second, like a 1000 Hz mouse through the app


# ---------------------------------------------------------------- web interface


class Web:
    """The board's HTTP API (one request per connection, as the board serves it)."""

    def __init__(self, host: str, password: str):
        self.host = host
        self.password = password
        self.cookie = ""

    def req(self, method: str, path: str, body: bytes | None = None, headers: dict | None = None,
            timeout: float = 10.0) -> tuple[int, dict | bytes]:
        h = {"Connection": "close"}
        if method == "POST":
            h["X-PSHB"] = "1"
        if self.cookie:
            h["Cookie"] = self.cookie
        h.update(headers or {})
        c = http.client.HTTPConnection(self.host, 80, timeout=timeout)
        try:
            c.request(method, path, body=body, headers=h)
            r = c.getresponse()
            raw = r.read()
            for v in r.headers.get_all("Set-Cookie") or []:
                nv = v.split(";", 1)[0]
                self.cookie = "" if nv.endswith("=") else nv
            if r.headers.get("Content-Type", "").startswith("application/json"):
                return r.status, json.loads(raw or b"{}")
            return r.status, raw
        finally:
            c.close()

    def login(self) -> None:
        status, r = self.req("POST", "/api/login", json.dumps({"password": self.password}).encode(),
                             {"Content-Type": "application/json"})
        if status != 200:
            raise SystemExit(f"Login failed: {status} {r}")

    def get(self, path: str) -> dict:
        status, r = self.req("GET", path)
        if status == 401:
            self.login()
            status, r = self.req("GET", path)
        if status != 200:
            raise RuntimeError(f"GET {path}: {status} {r}")
        return r

    def post(self, path: str, obj: dict | None = None) -> tuple[int, dict | bytes]:
        body = json.dumps(obj or {}).encode()
        status, r = self.req("POST", path, body, {"Content-Type": "application/json"})
        if status == 401:
            self.login()
            status, r = self.req("POST", path, body, {"Content-Type": "application/json"})
        return status, r

    def wait_up(self, timeout: float = 60) -> None:
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            try:
                if self.req("GET", "/api/state", timeout=2)[0] == 200:
                    self.cookie = ""
                    self.login()
                    return
            except OSError:
                pass
            time.sleep(0.5)
        raise SystemExit("The board did not come back.")

    def reboot(self) -> None:
        self.post("/api/reboot")
        time.sleep(2)
        self.wait_up()

    def install(self, path: str) -> dict:
        data = open(path, "rb").read()
        before = self.get("/api/status")
        print(f"Installing {os.path.basename(path)} ({len(data) // 1024} KiB), "
              f"running {before['firmware']} in slot {before.get('partition')} ...", flush=True)
        status, r = self.req("POST", "/api/ota", data,
                             {"Content-Type": "application/octet-stream",
                              "X-PSHB-CRC32": f"{zlib.crc32(data):x}"}, timeout=120)
        if status != 200:
            raise SystemExit(f"Upload failed: {status} {r}")
        time.sleep(3)
        self.wait_up(90)
        after = self.get("/api/status")
        print(f"Now running {after['firmware']} in slot {after.get('partition')}.", flush=True)
        return after


# ---------------------------------------------------------------- load processes


def flood(host: str, port: int, key: bytes, seconds: float, out: mp.Queue) -> None:
    """INPUT packets at RATE per second; reports what was sent and the board's answers."""
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.settimeout(0.5)
    sock.connect((host, port))
    counter = 0

    def next_counter() -> int:
        nonlocal counter
        counter = max(counter + 1, time.time_ns() // 1000)  # as the app does
        return counter

    def ping() -> proto.Pong | None:
        seq = random.randrange(1 << 31)
        sock.send(proto.seal(key, proto.DIR_APP_TO_BOARD, proto.TYPE_PING, next_counter(),
                             proto.Ping(time.monotonic_ns() // 1000, seq).encode()))
        end = time.monotonic() + 1
        while time.monotonic() < end:
            try:
                data = sock.recv(2048)
                ptype, _, payload = proto.open_packet(key, proto.DIR_BOARD_TO_APP, data)
            except (socket.timeout, ValueError):
                continue
            if ptype == proto.TYPE_PONG:
                p = proto.Pong.decode(payload)
                if p.seq == seq:
                    return p
        return None

    # After a restart the board first has to find our MAC address (ARP);
    # over WLAN that can take longer than its ARP timeout: try a few times.
    before = None
    for _ in range(8):
        before = ping()
        if before:
            break
    if before is None:
        out.put({"error": "no answer from the board (key, address?)"})
        return
    sent = 0
    if seconds > 0:
        total = int(RATE * seconds)
        start = time.perf_counter()
        for i in range(total):
            payload = proto.Input(session_id=before.session_id, dx=1 if i % 2 == 0 else -1).encode()
            sock.send(proto.seal(key, proto.DIR_APP_TO_BOARD, proto.TYPE_INPUT, next_counter(), payload))
            sent += 1
            target = start + (i + 1) / RATE
            while time.perf_counter() < target:
                pass
        elapsed = time.perf_counter() - start
    else:
        elapsed = 0.0
    time.sleep(0.4)  # longer than the failsafe (250 ms)
    after = ping() or ping()
    res = {"sent": sent, "seconds": elapsed}
    if after:
        res["arrived"] = after.rx_ok - before.rx_ok - 1  # minus the second ping
        res["auth_fail"] = after.rx_auth_fail - before.rx_auth_fail
        res["replay"] = after.rx_replay - before.rx_replay
        res["failsafe_after_stop"] = bool(after.status & proto.STATUS_FAILSAFE)
        res["firmware"] = after.fw_version_str
    out.put(res)


def web_load(host: str, cookie: str, stop: mp.Event, out: mp.Queue) -> None:
    """The web interface in a loop: what a browser on the status page fetches."""
    w = Web(host, "")
    w.cookie = cookie
    n = bytes_ = errors = 0
    paths = ["/", "/app.js", "/style.css", "/api/status"]
    while not stop.is_set():
        for p in paths:
            try:
                status, r = w.req("GET", p, timeout=5)
                n += 1
                bytes_ += len(r) if isinstance(r, bytes) else len(json.dumps(r))
                if status != 200:
                    errors += 1
            except OSError:
                errors += 1
    out.put({"requests": n, "bytes": bytes_, "errors": errors})


def upload_load(host: str, cookie: str, stop: mp.Event, out: mp.Queue) -> None:
    """Uploads to the sink back to back: network side of a firmware upload."""
    w = Web(host, "")
    w.cookie = cookie
    body = os.urandom(256 * 1024)
    total = errors = 0
    start = time.monotonic()
    while not stop.is_set():
        try:
            status, _ = w.req("POST", "/api/bench/sink", body,
                              {"Content-Type": "application/octet-stream"}, timeout=30)
            if status == 200:
                total += len(body)
            else:
                errors += 1
                time.sleep(0.5)
        except OSError:
            errors += 1
    secs = time.monotonic() - start
    out.put({"bytes": total, "seconds": secs, "kib_per_s": total / 1024 / secs if secs else 0,
             "errors": errors})


# ---------------------------------------------------------------- scenarios


def reset_stats(w: Web, has_diag: bool) -> None:
    if has_diag:
        status, r = w.post("/api/diag/reset")
        if status != 200:
            raise SystemExit(f"Reset failed: {status} {r}")
    else:
        w.reboot()  # older firmware: statistics start at boot
        time.sleep(3)  # USB enumeration with the host
    time.sleep(0.5)


def run(args, w: Web, key: bytes, name: str, has_diag: bool) -> dict:
    mouse = name != "S0"
    web = name == "S2"
    upload = name == "S3"
    reset_stats(w, has_diag)
    q: mp.Queue = mp.Queue()
    stop = mp.Event()
    procs = []
    if web:
        procs.append(mp.Process(target=web_load, args=(args.host, w.cookie, stop, q)))
    if upload:
        procs.append(mp.Process(target=upload_load, args=(args.host, w.cookie, stop, q)))
    for p in procs:
        p.start()
    time.sleep(0.5 if procs else 0)
    if mouse:
        f = mp.Process(target=flood, args=(args.host, args.port, key, args.seconds, q))
        f.start()
        f.join()
        flood_res = q.get()
    else:
        time.sleep(args.seconds)
        flood_res = {"sent": 0}
    stop.set()
    load = [q.get() for _ in procs]
    for p in procs:
        p.join()
    res = {"scenario": name, "flood": flood_res, "seconds": args.seconds}
    if load:
        res["load"] = load[0]
    res["status"] = w.get("/api/status")
    if has_diag:
        res["diag"] = w.get("/api/diag")
    return res


# ---------------------------------------------------------------- report

STAGES = [
    ("ready", "packet -> report ready (INTn -> applied)"),
    ("ready1", "  ... first datagram of a round (not queued)"),
    ("detect", "  INTn -> noticed by core 0"),
    ("bus", "  core 0 waits for the bus"),
    ("spi", "  SPI: read the datagram"),
    ("handle", "  checks + decryption + apply"),
    ("usb", "packet -> report fetched by the host"),
]


def report(results: list[dict]) -> None:
    for r in results:
        print(f"\n== {r['scenario']} ==")
        f = r["flood"]
        if f.get("error"):
            print("  " + f["error"])
            continue
        if f.get("sent"):
            print(f"  packets: {f['sent']} sent in {f['seconds']:.1f} s, {f.get('arrived')} arrived "
                  f"(replay {f.get('replay')}, bad signature {f.get('auth_fail')}), "
                  f"failsafe after stop: {f.get('failsafe_after_stop')}")
        if "load" in r:
            print(f"  load: {r['load']}")
        d = r.get("diag")
        s = r["status"]
        if d:
            print(f"  {'':44} {'n':>7} {'min':>6} {'median':>7} {'p99':>6} {'max':>6}  (µs)")
            for k, label in STAGES:
                if d.get(f"{k}_n"):
                    print(f"  {label:44} {d[k + '_n']:7} {d[k + '_min']:6} {d[k + '_p50']:7} "
                          f"{d[k + '_p99']:6} {d[k + '_max']:6}")
            fetches = d.get("mouse_fetches", 0)
            misses = d.get("mouse_misses", 0)
            if fetches:
                print(f"  missed polls: {misses} of {fetches} ({100 * misses / fetches:.3f} %), "
                      f"lead {d.get('mouse_lead_us')} µs")
            if d.get("rounds"):
                print(f"  {d['datagrams']} datagrams in {d['rounds']} receive rounds "
                      f"({d['datagrams'] / d['rounds']:.2f} per round)")
            print(f"  SPI transactions by core 0: {d.get('core0_spi')} "
                  f"({d.get('core0_spi', 0) / r.get('seconds', 30):.0f}/s), SPI {d.get('spi_hz', 0) / 1e6:.1f} MHz, "
                  f"command register busy {d.get('cr_busy')}x")
            if d.get("crypto_avg_ns"):
                print(f"  decrypting one INPUT packet: {d['crypto_avg_ns'] / 1000:.1f} µs on average "
                      f"(min {d['crypto_min_ns'] / 1000:.0f}, max {d['crypto_max_ns'] / 1000:.0f}), "
                      f"{'optimised' if d.get('crypto_fast') else 'C'} ChaCha20")
        else:
            print(f"  receive and decrypt (noticed -> applied): median {s.get('lat_rx_p50_us')} µs, "
                  f"max {s.get('lat_rx_max_us')} µs")
            if s.get("lat_usb_count"):
                print(f"  packet -> report fetched: median {s['lat_usb_p50_us']} µs, "
                      f"95 % {s['lat_usb_p95_us']} µs, max {s['lat_usb_max_us']} µs "
                      f"({s['lat_usb_count']} reports)")
            mi = s.get("mouse_inputs", 0)
            print(f"  mouse: {mi} inputs, {s.get('mouse_waits')} waited, {s.get('mouse_misses')} "
                  f"fetches missed, lead {s.get('mouse_lead_us')} µs")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("host")
    ap.add_argument("--port", type=int, default=proto.UDP_PORT_DEFAULT)
    ap.add_argument("--password-file")
    ap.add_argument("--psk-file")
    ap.add_argument("--install", help="UF2 to install over the network first")
    ap.add_argument("--scenarios", default="S0,S1,S2,S3")
    ap.add_argument("--seconds", type=float, default=30)
    ap.add_argument("--label", default="bench")
    ap.add_argument("--out", default="out/bench")
    ap.add_argument("--report", help="only print the report of an earlier run (JSON file)")
    args = ap.parse_args()
    if args.report:
        report(json.load(open(args.report)))
        return 0

    password = open(args.password_file).read().strip()
    key = bytes.fromhex(open(args.psk_file).read().strip())
    w = Web(args.host, password)
    w.login()
    if args.install:
        w.install(args.install)
    try:
        w.get("/api/diag")
        has_diag = True
    except RuntimeError:
        has_diag = False
    scenarios = [s.strip() for s in args.scenarios.split(",") if s.strip()]
    if not has_diag:
        scenarios = [s for s in scenarios if s in ("S1", "S2")]  # what older firmware can show
    results = []
    for name in scenarios:
        print(f"Running {name} ({args.seconds:.0f} s) ...", flush=True)
        results.append(run(args, w, key, name, has_diag))
    report(results)
    os.makedirs(args.out, exist_ok=True)
    path = os.path.join(args.out, f"{args.label}.json")
    with open(path, "w") as f:
        json.dump(results, f, indent=1)
    print(f"\nSaved to {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
