#!/usr/bin/env python3
"""Exercise the real 26.3 gateway: Eagler login, config, chunks, chat and keepalive.
Requires a running offline-mode Pumpkin + gateway. No third-party dependencies.
This protocol probe supplements (and does not replace) the browser play test.
"""
import argparse
import collections
import json
import os
import socket
import struct
import time
import hashlib
import uuid
import zlib


def vi(n):
    out = bytearray()
    while True:
        b = n & 127
        n >>= 7
        out.append(b | (128 if n else 0))
        if not n:
            return bytes(out)


def read_vi(data, offset=0):
    n = 0
    for i in range(5):
        b = data[offset + i]
        n |= (b & 127) << (7 * i)
        if not b & 128:
            return n, offset + i + 1
    raise ValueError("invalid VarInt")


def text(s):
    b = s.encode()
    return vi(len(b)) + b


def ascii_text(s):
    b = s.encode("ascii")
    return bytes([len(b)]) + b


class Client:
    path = "/"

    def __init__(self, host, port):
        self.sock = socket.create_connection((host, port), timeout=5)
        self.sock.settimeout(5)
        self.sock.sendall((
            f"GET {self.path} HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\n"
            "Connection: Upgrade\r\nSec-WebSocket-Version: 13\r\n"
            "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
        ).encode())
        response = bytearray()
        while not response.endswith(b"\r\n\r\n"):
            response.extend(self.exact(1))
        assert response.startswith(b"HTTP/1.1 101"), response
        assert b"s3pPLMBiTxaQ9kYGzzhZRbK+xOo=" in response

    def exact(self, n):
        data = bytearray()
        while len(data) < n:
            part = self.sock.recv(n - len(data))
            if not part:
                raise EOFError("gateway disconnected")
            data.extend(part)
        return bytes(data)

    def send(self, data, opcode=2):
        mask = os.urandom(4)
        header = bytes([0x80 | opcode])
        if len(data) < 126:
            header += bytes([128 | len(data)])
        elif len(data) < 65536:
            header += b"\xfe" + struct.pack(">H", len(data))
        else:
            header += b"\xff" + struct.pack(">Q", len(data))
        self.sock.sendall(header + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(data)))

    def receive_frame(self):
        while True:
            op, n = self.exact(2)
            assert not n & 128, "server frames must not be masked"
            n &= 127
            if n == 126:
                n = struct.unpack(">H", self.exact(2))[0]
            elif n == 127:
                n = struct.unpack(">Q", self.exact(8))[0]
            data = self.exact(n)
            if op & 15 == 9:
                self.send(data, 10)
                continue
            if op & 15 == 8:
                raise EOFError(f"gateway closed WebSocket: {data!r}")
            return op & 15, data

    def receive(self):
        opcode, data = self.receive_frame()
        assert opcode == 2, (opcode, data[:200])
        return data

    def login(self, name):
        self.send(b"\x01\x02" + struct.pack(">HHHH", 1, 5, 1, 777)
                  + ascii_text("PortProbe") + ascii_text("1") + b"\x00\x00")
        version = self.receive()
        assert version[:5] == b"\x02\x00\x05\x03\x09", version
        self.send(b"\x04" + ascii_text(name) + ascii_text("default") + b"\x00\x00\x00\x00\x00")
        allow = self.receive()
        assert allow[0] == 5, allow
        self.send(b"\x08")
        assert self.receive() == b"\x09"
        flag = self.receive()
        assert flag[:2] == b"\xee\x21", flag  # disable URL skin lookup
        return self.configure()

    def configure(self):
        self.send(b"\x00" + text("en_us") + b"\x04\x00\x01\x7f\x01\x00\x01")
        registries = 0
        while True:
            p = self.receive()
            packet, offset = read_vi(p)
            if packet == 15:
                self.send(b"\x07\x00")  # request full registry entries
            elif packet == 7:
                registries += 1
            elif packet == 3:
                self.send(b"\x03")
                return registries
            elif packet == 4:
                self.send(b"\x04" + p[offset:])
            elif packet == 5:
                self.send(b"\x05" + p[offset:])
            elif packet == 20:
                self.send(b"\x09")
            elif packet == 2:
                raise RuntimeError(f"configuration disconnected: {p!r}")


class GaiusClient(Client):
    """Gaius /tunnel: binary WebSocket messages carry a normal MC TCP stream."""
    path = "/tunnel"

    def __init__(self, host, port):
        self.input = bytearray()
        self.compression = None
        super().__init__(host, port)
        Client.send(self, json.dumps({"type": "connect", "host": host, "port": 25565}).encode(), 1)
        opcode, payload = self.receive_frame()
        connected = json.loads(payload)
        assert opcode == 1 and connected == {"type": "connected", "host": host, "port": 25565}, connected

    def send(self, data, opcode=2):
        if opcode != 2:
            return Client.send(self, data, opcode)
        if self.compression is None:
            framed = data
        elif len(data) >= self.compression:
            framed = vi(len(data)) + zlib.compress(data)
        else:
            framed = b"\x00" + data
        self.send_stream(vi(len(framed)) + framed)

    def send_stream(self, data):
        Client.send(self, data)

    def receive_stream(self):
        return Client.receive(self)

    def receive(self):
        while True:
            try:
                n, offset = read_vi(self.input)
            except IndexError:
                n, offset = -1, 0
            if n >= 0 and len(self.input) >= n + offset:
                data = bytes(self.input[offset:n+offset])
                del self.input[:n+offset]
                if self.compression is not None:
                    size, offset = read_vi(data)
                    data = data[offset:]
                    if size:
                        data = zlib.decompress(data)
                        assert len(data) == size
                return data
            self.input.extend(self.receive_stream())

    def login(self, name):
        self.send(b"\x00" + vi(777) + text("localhost") + struct.pack(">H", 25565) + b"\x02")
        offline_id = uuid.UUID(bytes=hashlib.md5(f"OfflinePlayer:{name}".encode()).digest(), version=3)
        self.send(b"\x00" + text(name) + offline_id.bytes)
        while True:
            packet = self.receive()
            kind, offset = read_vi(packet)
            if kind == 3:
                self.compression, _ = read_vi(packet, offset)
            elif kind == 2:
                self.send(b"\x03")
                return self.configure()
            else:
                raise RuntimeError(f"Unexpected vanilla login packet: {packet!r}")


class VanillaClient(GaiusClient):
    """Vanilla TCP stream through the gateway's raw passthrough listener."""

    def __init__(self, host, port):
        self.input = bytearray()
        self.compression = None
        self.sock = socket.create_connection((host, port), timeout=5)
        self.sock.settimeout(5)

    def send_stream(self, data):
        self.sock.sendall(data)

    def receive_stream(self):
        data = self.sock.recv(65536)
        if not data:
            raise EOFError("gateway disconnected")
        return data


def receive_before(client, deadline):
    """Stop at the run deadline; earlier socket failures still fail the probe."""
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        return None
    client.sock.settimeout(remaining)
    try:
        return client.receive()
    except socket.timeout:
        if time.monotonic() >= deadline:
            return None
        raise


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8081)
    parser.add_argument("--seconds", type=int, default=20)
    parser.add_argument("--name", default="PortProbe")
    parser.add_argument("--sustain", action="store_true",
                        help="keep the session alive for the full --seconds after login")
    parser.add_argument("--transport", choices=("eagler", "gaius", "vanilla"), default="eagler")
    parser.add_argument("--json-output", help="write successful play result as JSON")
    args = parser.parse_args()
    if args.seconds <= 0:
        parser.error("--seconds must be positive")
    client = {"eagler": Client, "gaius": GaiusClient, "vanilla": VanillaClient}[args.transport](args.host, args.port)
    try:
        registries = client.login(args.name)
        counts = collections.Counter()
        chunks = bytes_received = keepalive_replies = 0
        joined = positioned = chat_seen = keepalive = False
        marker = f"gateway-probe-{args.name}-{time.time_ns()}"
        started = time.monotonic()
        deadline = started + args.seconds
        while time.monotonic() < deadline:
            p = receive_before(client, deadline)
            if p is None:
                break
            bytes_received += len(p)
            packet, offset = read_vi(p)
            counts[packet] += 1
            if marker.encode() in p:
                print("chat marker received in packet", packet)
                chat_seen = True
            if packet == 50:
                joined = True
            elif packet == 73:
                teleport, pos_offset = read_vi(p, offset)
                x, y, z = struct.unpack_from(">ddd", p, pos_offset)
                yaw, pitch = struct.unpack_from(">ff", p, pos_offset + 48)
                client.send(b"\x00" + vi(teleport) + struct.pack(">dddff", x, y, z, yaw, pitch))
                client.send(b"\x2c")  # player loaded
                client.send(b"\x1e" + struct.pack(">dddB", x + 0.125, y, z, 1))
                positioned = True
                client.send(b"\x09" + text(marker) + struct.pack(">qq", int(time.time() * 1000), 0)
                            + b"\x00\x00\x00\x00\x00\x00")
            elif packet == 46:
                chunks += 1
            elif packet == 11:  # chunk batch finished
                client.send(b"\x0b" + struct.pack(">f", 20.0))
            elif packet == 45:
                client.send(b"\x1c" + p[offset:])
                keepalive = True
                keepalive_replies += 1
            elif packet in (33, 66, 124):
                chat_seen |= marker.encode() in p
            elif packet == 32:  # disconnect
                raise RuntimeError(f"play disconnected: {p!r}")
            if not args.sustain and joined and positioned and chunks and chat_seen and keepalive:
                break
        result = dict(joined=joined, position_acknowledged=positioned, chat_echo=chat_seen,
                      keepalive_roundtrip=keepalive, chunks=chunks, registries=registries,
                      minecraft_bytes=bytes_received, transport=args.transport,
                      sustained=args.sustain, requested_seconds=args.seconds,
                      play_seconds=round(time.monotonic() - started, 3),
                      keepalive_replies=keepalive_replies, total_packets=sum(counts.values()))
        print(json.dumps(result, indent=2))
        assert joined and positioned and chunks and chat_seen and keepalive, result
        if args.json_output:
            from pathlib import Path
            Path(args.json_output).write_text(json.dumps(result, indent=2) + "\n")
    finally:
        client.sock.close()


if __name__ == "__main__":
    main()
