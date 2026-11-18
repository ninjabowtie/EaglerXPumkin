#!/usr/bin/env python3
"""Sustained multi-client soak and fuzz run for the real gateway.

Spawns concurrent Eagler v5 and Gaius /tunnel players acting randomly and
periodic MOTD query clients (which double as listener health checks), plus
fuzz connections serving malformed input. Exits nonzero if a health check
or MOTD query fails, a worker is disconnected unexpectedly, or a fuzz
vector leaves its connection open.

Requires a running offline-mode Pumpkin + gateway. No third-party
dependencies. The protocol probe supplements (and does not replace) the
browser play test; this script supplements it with duration, concurrency
and malformed-input coverage.
"""
import argparse
import collections
from datetime import datetime, timezone
import json
import socket
import struct
import sys
import threading
import time

import probe_26_3 as probe

LOCK = threading.Lock()


class Stats:
    def __init__(self):
        self.counters = collections.Counter()
        self.errors = []
        self.health_ok = 0

    def add(self, kind, count=1):
        with LOCK:
            self.counters[kind] += count

    def fail(self, message):
        with LOCK:
            self.errors.append(message)

    def snapshot(self):
        with LOCK:
            return dict(self.counters), list(self.errors)


def recv_until_closed(client, max_seconds=5.0):
    """Read frames until the gateway closes the connection. Returns the
    (text_frames, binary_frames, closed) tuple."""
    text = binary = 0
    closed = False
    deadline = time.monotonic() + max_seconds
    try:
        while time.monotonic() < deadline:
            client.sock.settimeout(max(0.001, deadline - time.monotonic()))
            opcode, _ = client.receive_frame()
            if opcode == 1:
                text += 1
            elif opcode == 2:
                binary += 1
    except socket.timeout:
        pass  # A silent open socket is not evidence that the peer closed it.
    except (EOFError, ConnectionError, OSError):
        closed = True
    return text, binary, closed


def motd_worker(host, port, deadline, stats, interval):
    """Poll the MOTD query path; counts animation frames per query."""
    while time.monotonic() < deadline:
        try:
            client = probe.Client(host, port)
            client.send(b"Accept: MOTD", 1)
            frames, _binary, closed = recv_until_closed(client)
            client.sock.close()
            if not closed:
                stats.fail("MOTD query connection was never closed by the gateway")
            if not frames:
                stats.fail("MOTD query returned no text frames")
            stats.add("motd_queries")
            stats.add("motd_frames", frames)
        except Exception as exc:  # noqa: BLE001 - soak must survive anything
            stats.fail(f"motd query failed: {exc}")
            stats.add("motd_failures")
        time.sleep(interval)


def worker(index, host, port, deadline, stats):
    transport = "gaius" if index % 2 else "eagler"
    name = f"Soak{index:02d}"
    try:
        client = (probe.GaiusClient if transport == "gaius" else probe.Client)(host, port)
        client.login(name)
        stats.add("joined")
        stats.add(f"joined_{transport}")
    except Exception as exc:  # noqa: BLE001
        stats.fail(f"{name} login failed: {exc}")
        return
    client.sock.settimeout(0.5)
    last_position = last_chat = time.monotonic()
    try:
        while time.monotonic() < deadline:
            try:
                packet = client.receive()
                stats.add("minecraft_bytes", len(packet))
                kind, offset = probe.read_vi(packet)
                if kind == 45:  # keepalive
                    client.send(b"\x1c" + packet[offset:])
                    stats.add("keepalive")
                elif kind == 32:  # play disconnect
                    stats.fail(f"{name} was disconnected by the server")
                    return
                elif kind in (33, 66, 124):
                    stats.add("chat_inbound")
            except socket.timeout:
                pass
            except (EOFError, ConnectionError, OSError) as exc:
                stats.fail(f"{name} disconnected unexpectedly: {exc}")
                return
            now = time.monotonic()
            if now - last_position >= 1.0:
                client.send(b"\x1e" + struct.pack(">dddB", index, 64.0, 0.0, 0))
                stats.add("positions")
                last_position = now
            if now - last_chat >= 5.0:
                stamp = int(time.time() * 1000)
                client.send(b"\x09" + probe.text(f"soak {name} {stamp}")
                            + struct.pack(">qq", stamp, 0) + b"\x00" * 6)
                stats.add("chat_outbound")
                last_chat = now
    finally:
        try:
            client.sock.close()
        except OSError:
            pass
        stats.add("finished")


def fuzz_round(host, port, stats, choice=None):
    """One malformed connection. The gateway must close it and stay healthy."""
    upgrade = (
        b"GET / HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\n"
        b"Connection: Upgrade\r\nSec-WebSocket-Version: 13\r\n"
        b"Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
    )
    choice = int(time.time() * 1000) % 6 if choice is None else choice
    sock = None
    try:
        sock = socket.create_connection((host, port), timeout=5)
        sock.settimeout(5)
        if choice == 0:
            sock.sendall(b"\x00\x01\x02\x03garbage-not-http\r\n\r\n")  # raw garbage
        else:
            sock.sendall(upgrade)
            if choice == 1:
                _send_masked(sock, 1, b"\xff\xfe")  # invalid UTF-8 text query
            elif choice == 2:
                _send_masked(sock, 1, b"Accept: MOTD")
                # Then an oversized declared frame length.
                sock.sendall(bytes([0x82, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]))
            elif choice == 3:
                # Eagler handshake with an invalid version count.
                _send_masked(sock, 2, b"\x01\x02\x00\x20\x00\x05\x00\x01\x01x\x01x\x00\x00")
            elif choice == 4:
                _send_masked(sock, 1, b"Accept: garbage")  # unsupported query
            else:
                _send_masked(sock, 2, b"\x08")  # out-of-order login packet
        received, closed = _drain(sock)
        if not closed:
            stats.fail(f"fuzz vector {choice} left its connection open")
        stats.add("fuzz_rounds")
        stats.add("fuzz_bytes", received)
        stats.add(f"fuzz_vector_{choice}")
    except socket.timeout:
        stats.fail(f"fuzz vector {choice} timed out without a peer close")
    except (ConnectionError, OSError) as exc:
        if sock is None:
            stats.fail(f"fuzz vector {choice} could not connect: {exc}")
        else:
            stats.add("fuzz_rounds")
            stats.add("fuzz_resets")
            stats.add(f"fuzz_vector_{choice}")
    finally:
        if sock is not None:
            sock.close()


def _send_masked(sock, opcode, payload):
    mask = bytes(range(4))
    header = bytes([0x80 | opcode])
    if len(payload) < 126:
        header += bytes([128 | len(payload)])
    else:
        header += b"\xfe" + struct.pack(">H", len(payload))
    masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
    sock.sendall(header + mask + masked)


def _drain(sock, max_seconds=3.0):
    """Drain raw rejection bytes until EOF/reset; a timeout is a failed close.

    Input can contain an HTTP upgrade followed by WebSocket frames. Reading
    raw bytes avoids interpreting HTTP headers as WebSocket packet lengths.
    """
    received = 0
    deadline = time.monotonic() + max_seconds
    try:
        while time.monotonic() < deadline:
            sock.settimeout(max(0.001, deadline - time.monotonic()))
            data = sock.recv(65536)
            if not data:
                return received, True
            received += len(data)
    except socket.timeout:
        pass
    except (ConnectionError, OSError):
        return received, True
    return received, False


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8081)
    parser.add_argument("--clients", type=int, default=5)
    parser.add_argument("--fuzz-clients", type=int, default=2)
    parser.add_argument("--seconds", type=int, default=60)
    parser.add_argument("--motd-interval", type=float, default=5.0)
    parser.add_argument("--json-output", help="save final counters and failures as JSON")
    args = parser.parse_args()
    if args.seconds <= 0 or args.motd_interval <= 0 or args.clients < 0 or args.fuzz_clients < 0:
        parser.error("durations must be positive and client counts nonnegative")
    started_utc = datetime.now(timezone.utc).isoformat()

    stats = Stats()
    deadline = time.monotonic() + args.seconds
    threads = []
    for index in range(args.clients):
        thread = threading.Thread(
            target=worker,
            args=(index, args.host, args.port, deadline, stats),
            daemon=True,
        )
        thread.start()
        threads.append(thread)
    for _ in range(args.fuzz_clients):
        thread = threading.Thread(
            target=_fuzz_loop,
            args=(args.host, args.port, deadline, stats),
            daemon=True,
        )
        thread.start()
        threads.append(thread)
    motd_thread = threading.Thread(
        target=motd_worker,
        args=(args.host, args.port, deadline, stats, args.motd_interval),
        daemon=True,
    )
    motd_thread.start()
    threads.append(motd_thread)

    started = time.monotonic()
    while time.monotonic() < deadline:
        time.sleep(min(10, max(0, deadline - time.monotonic())))
        counters, errors = stats.snapshot()
        print(f"soak {time.monotonic() - started:.0f}s: {json.dumps(counters)}")
        for error in errors[-3:]:
            print(f"  error: {error}", file=sys.stderr)

    for thread in threads:
        thread.join(timeout=args.seconds)
    counters, errors = stats.snapshot()
    result = dict(started_utc=started_utc, finished_utc=datetime.now(timezone.utc).isoformat(),
                  requested_seconds=args.seconds, clients=args.clients,
                  fuzz_clients=args.fuzz_clients, host=args.host, port=args.port,
                  final=counters, errors=errors, passed=not errors)
    print(json.dumps(result, indent=2))
    if args.json_output:
        from pathlib import Path
        Path(args.json_output).write_text(json.dumps(result, indent=2) + "\n")
    if errors:
        print(f"soak FAILED with {len(errors)} errors", file=sys.stderr)
        sys.exit(1)
    print("soak passed")


def _fuzz_loop(host, port, deadline, stats):
    choice = 0
    while time.monotonic() < deadline:
        fuzz_round(host, port, stats, choice)
        choice = (choice + 1) % 6
        time.sleep(2.0)


if __name__ == "__main__":
    main()
