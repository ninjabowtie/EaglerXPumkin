#!/usr/bin/env python3
"""Exercise the real gateway with an Eagler v5 client negotiating a non-777
Minecraft protocol (default 755 = 1.17) through Multiversion translation.
Verifies: highest-protocol selection, version-correct login injection,
backend login success parsing, Eagler finish round trip and a translated
1.17-format play stream (join, teleport ack, chunks, chat echo, keepalive).
"""
import argparse
import collections
import json
import struct
import time
import hashlib

from probe_26_3 import Client as EaglerClient, vi, read_vi, text, ascii_text, receive_before


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=8081)
    ap.add_argument("--protocol", type=int, default=755)
    ap.add_argument("--offer", type=int, nargs="*", default=None,
                    help="protocols to offer (default: only --protocol)")
    ap.add_argument("--seconds", type=int, default=20)
    ap.add_argument("--name", default="OldClient")
    ap.add_argument("--sustain", action="store_true",
                    help="keep the session alive for the full --seconds after login")
    ap.add_argument("--handshake-only", action="store_true",
                    help="stop after negotiation (for selection/mismatch checks)")
    ap.add_argument("--expect-mismatch", action="store_true",
                    help="require the server to reject the offered protocols")
    ap.add_argument("--json-output", help="write successful play result as JSON")
    args = ap.parse_args()
    if not (args.handshake_only or args.expect_mismatch) and args.protocol != 755:
        ap.error("play packet checks are implemented only for protocol 755 (1.17)")
    if args.seconds <= 0:
        ap.error("--seconds must be positive")
    if not 1 <= len(args.name) <= 16 or not args.name.isascii() or not all(
            c.isalnum() or c == "_" for c in args.name):
        ap.error("--name must contain 1..16 ASCII letters, digits or underscores")
    offer = args.offer or [args.protocol]
    if any(not 0 <= protocol <= 65535 for protocol in offer):
        ap.error("offered protocols must fit unsigned 16-bit values")

    client = EaglerClient(args.host, args.port)
    try:
        # Handshake v5 offering the protocol(s) under test.
        packet = b"\x01\x02" + struct.pack(">HHH", 1, 5, len(offer))
        for p in offer:
            packet += struct.pack(">H", p)
        packet += ascii_text("OldProbe") + ascii_text("1") + b"\x00\x00"
        client.send(packet)
        version = client.receive()
        if args.expect_mismatch:
            assert version[:1] == b"\x03", f"expected mismatch, got {version[:7].hex()}"
            print(json.dumps(dict(offer=offer, mismatch=True, reply=version.hex()), indent=2))
            return
        want = b"\x02\x00\x05" + struct.pack(">H", args.protocol)
        assert version[:5] == want, (
            f"server selected wrong protocol: {version[:5].hex()} want {want.hex()}")
        if args.handshake_only:
            print(json.dumps(dict(offer=offer, selected=args.protocol), indent=2))
            return
        print(f"negotiated protocol {args.protocol} from offer {offer}")

        # The initial identity is empty, so the requested nickname is used.
        client.send(b"\x04" + ascii_text(args.name) + ascii_text("default")
                    + b"\x00\x00\x00\x00\x00")
        allow = client.receive()
        assert allow[0] == 5, allow[:40]
        uuid = allow[1 + 1 + len(args.name):][:16]
        offline_id = bytearray(hashlib.md5(f"OfflinePlayer:{args.name}".encode()).digest())
        offline_id[6] = (offline_id[6] & 15) | 0x30  # version 3, as the gateway does
        offline_id[8] = (offline_id[8] & 63) | 0x80
        assert uuid == bytes(offline_id), (uuid.hex(), bytes(offline_id).hex())
        print(f"allow login: uuid {uuid.hex()} matches offline identity")

        client.send(b"\x08")
        assert client.receive() == b"\x09", "expected finish login ack"
        print("Eagler finish acknowledged")

        # 1.17-format play stream through Multiversion.
        JOIN, CHUNK, TELEPORT, KEEPALIVE, DISCONNECT, CHAT = 0x26, 0x22, 0x38, 0x21, 0x1A, 0x0F
        counts = collections.Counter()
        dump_left = 5
        joined = positioned = keepalive = False
        chat_seen = chat_sent = False
        chunks = keepalive_replies = 0
        marker = f"oldprobe-{args.name}-{time.time_ns()}"
        started = time.monotonic()
        deadline = started + args.seconds
        while time.monotonic() < deadline:
            p = receive_before(client, deadline)
            if p is None:
                break
            packet_id, offset = read_vi(p)
            counts[packet_id] += 1
            if dump_left > 0:
                print(f"play packet 0x{packet_id:x} ({len(p)} bytes)")
                dump_left -= 1
            if packet_id == DISCONNECT:
                raise RuntimeError(f"play disconnect: {p[offset:offset+120]!r}")
            if packet_id == JOIN:
                joined = True
            elif packet_id == TELEPORT:
                # x,y,z (24) + yaw,pitch (8) + flags (1), then the teleport id
                teleport, off = read_vi(p, offset + 33)
                client.send(b"\x00" + vi(teleport))
                client.send(b"\x12" + p[offset:offset + 33])
                positioned = True
                if not chat_sent:
                    client.send(b"\x03" + text(marker))
                    chat_sent = True
            elif packet_id == KEEPALIVE:
                client.send(b"\x0f" + p[offset:])
                keepalive = True
                keepalive_replies += 1
            elif packet_id == CHUNK:
                chunks += 1
            elif packet_id == CHAT:
                chat_seen |= marker.encode() in p
            if not args.sustain and joined and positioned and keepalive and chunks and chat_seen:
                break
        print(f"packet counts: {dict(sorted(counts.items()))}")
        result = dict(protocol=args.protocol, joined=joined, positioned=positioned,
                      keepalive=keepalive, chat_echo=chat_seen, chunks=chunks,
                      total_packets=sum(counts.values()), sustained=args.sustain,
                      requested_seconds=args.seconds,
                      play_seconds=round(time.monotonic() - started, 3),
                      keepalive_replies=keepalive_replies)
        print(json.dumps(result, indent=2))
        assert joined, "no join game received"
        assert positioned, "no position/teleport round trip"
        assert keepalive, "no keepalive round trip"
        assert chunks, "no chunk data"
        assert chat_seen, "no chat marker echo"
        if args.json_output:
            from pathlib import Path
            Path(args.json_output).write_text(json.dumps(result, indent=2) + "\n")
    finally:
        client.sock.close()


if __name__ == "__main__":
    main()
