#!/usr/bin/env python3
"""Log of the controller-mode test images (firmware/src/pico/netlog.c).

Subscribes to the board's log over UDP and prints it: first everything the
board still has since it started (also from before a watchdog restart), then
every new line. Keeps the subscription alive and picks it up again after the
board restarts. Commands for the test image can be sent along.

Usage:
  tools/boardlog.py BOARD_IP                  # print until Ctrl+C
  tools/boardlog.py BOARD_IP -o ds.log        # also append to a file
  tools/boardlog.py BOARD_IP -c status        # send a command, keep printing
  tools/boardlog.py BOARD_IP -c swap --seconds 10
"""

import argparse
import select
import socket
import sys
import time

PORT = 45046
KEEPALIVE_S = 3.0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("board", help="address of the board")
    ap.add_argument("-o", "--output", help="also append the log to this file")
    ap.add_argument("-c", "--command", action="append", default=[], help="send a command (repeatable)")
    ap.add_argument("--seconds", type=float, default=0.0, help="stop after this long (0: until Ctrl+C)")
    args = ap.parse_args()

    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind(("", 0))
    dest = (args.board, PORT)
    sock.sendto(b"sub", dest)
    for cmd in args.command:
        sock.sendto(cmd.encode(), dest)
    out = open(args.output, "a", encoding="utf-8") if args.output else None

    start = last_ka = time.monotonic()
    try:
        while not args.seconds or time.monotonic() - start < args.seconds:
            now = time.monotonic()
            if now - last_ka >= KEEPALIVE_S:
                sock.sendto(b"ka", dest)
                last_ka = now
            ready, _, _ = select.select([sock], [], [], 0.5)
            if not ready:
                continue
            data, _ = sock.recvfrom(65535)
            text = data.decode("utf-8", "replace")
            sys.stdout.write(text)
            sys.stdout.flush()
            if out:
                out.write(text)
                out.flush()
    except KeyboardInterrupt:
        pass
    finally:
        if out:
            out.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
