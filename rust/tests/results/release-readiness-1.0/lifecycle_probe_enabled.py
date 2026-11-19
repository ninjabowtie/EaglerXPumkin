#!/usr/bin/env python3
"""Test WASI notification commands in an isolated Pumpkin host.

Uses the isolated host as its offline protocol-777 backend by default.
Optionally route through an existing backend with --backend-port.
Creates a NEW output directory with an isolated world/config, launches Pumpkin,
and stops only that child process. Does not replace installed plugins.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import socket
import struct
import subprocess
import time

import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from probe_26_3 import Client, ascii_text, read_vi, text


def unused_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def login(client, name):
    client.send(b"\x01\x02" + struct.pack(">HHHH", 1, 5, 1, 777)
                + ascii_text("NotificationProbe") + ascii_text("1") + b"\x00\x00")
    assert client.receive()[:5] == b"\x02\x00\x05\x03\x09"
    # Offer notification capability V0, no extended capabilities.
    client.send(b"\x04" + ascii_text(name) + ascii_text("default")
                + b"\x00\x00\x00\x08\x01\x00")
    allow = client.receive()
    assert allow[0] == 5, allow
    caps, offset = read_vi(allow, 2 + allow[1] + 16)
    assert caps == 8 and allow[offset:] == b"\x00\x00", allow
    client.send(b"\x08")
    assert client.receive() == b"\x09"
    assert client.receive()[:2] == b"\xee\x21"
    client.configure()


def receive_badge(client, opcode):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        client.sock.settimeout(max(0.01, deadline - time.monotonic()))
        packet = client.receive()
        if packet[:2] == bytes([0xee, opcode]):
            return packet
        kind, offset = read_vi(packet)
        if kind == 45:  # Keep the backend healthy while waiting.
            client.send(b"\x1c" + packet[offset:])
        elif kind == 32:
            raise AssertionError(f"backend disconnected: {packet!r}")
    raise AssertionError("notification packet deadline exceeded")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pumpkin-binary", type=Path, required=True)
    parser.add_argument("--artifact", type=Path, default=Path("dist/EaglerXPumpkin.wasm"))
    parser.add_argument("--backend-port", type=int)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    binary = args.pumpkin_binary.resolve(strict=True)
    artifact = args.artifact.resolve(strict=True)
    root = args.output_dir.resolve()
    root.mkdir(parents=True, exist_ok=False)
    plugins = root / "plugins"
    data = plugins / "data/eaglerx-pumpkin"
    data.mkdir(parents=True)
    shutil.copy2(artifact, plugins / "EaglerXPumpkin.wasm")
    digest = hashlib.sha256(artifact.read_bytes()).hexdigest()
    # This isolated host explicitly approves only the tested gateway's existing
    # four permissions. The installed server's permission cache is untouched.
    (plugins / "permission_cache.json").write_text(json.dumps({"entries": {digest: {
        "permissions_requested": ["network.tcp.bind", "network.tcp.connect", "fs.read.data", "fs.write.data"],
        "approved": True,
    }}}))
    port = unused_port()
    java_port = unused_port()
    backend_port = args.backend_port or java_port
    (data / "eagler.toml").write_text(
        f'listen_address = "127.0.0.1"\nlisten_port = {port}\nbackend_port = {backend_port}\n'
        'dynamic_notifications = true\nconnections_per_minute = 0\n')
    (root / "pumpkin.toml").write_text(
        f'[networking.java]\naddress = "127.0.0.1:{java_port}"\nonline_mode = false\nencryption = false\n'
        '[networking.bedrock]\nenabled = false\n[commands]\nuse_tty = false\n')
    result = {"passed": False, "artifact_sha256": digest, "gateway_port": port,
              "scope": "isolated WASI host, console commands and simulated notification-capable Eagler client"}
    client = None
    with (root / "console.log").open("w") as log:
        process = subprocess.Popen([str(binary)], cwd=root, stdin=subprocess.PIPE,
                                   stdout=log, stderr=subprocess.STDOUT, text=True)
        try:
            deadline = time.monotonic() + 60
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    raise RuntimeError("isolated Pumpkin exited during startup; see console.log")
                output = (root / "console.log").read_text()
                if "Failed to initialize plugin" in output:
                    raise RuntimeError("gateway plugin initialization failed; see console.log")
                if "Started server;" in output and "scheduler listener active" in output:
                    break
                time.sleep(0.25)
            else:
                raise RuntimeError("isolated Pumpkin startup deadline exceeded")
            client = Client("127.0.0.1", port)
            login(client, "BadgeProbe")

            def command(text):
                process.stdin.write(text + "\n")
                process.stdin.flush()

            command('eaglernotify BadgeProbe {"id":"test","title":"Alert","priority":"highest","silent":true}')
            show = receive_badge(client, 0x1f)
            offset = 18
            for _ in range(3):
                length, offset = read_vi(show, offset)
                offset += length
            assert show[offset + 6] == 7, "highest + silent flags incorrect"
            command('eaglernotifyhide BadgeProbe {"id":"test"}')
            hide = receive_badge(client, 0x20)
            assert hide == b"\xee\x20" + show[2:18], "show/hide UUID mismatch"
            command('eaglernotify * {"id":"test","title":"Updated","priority":"low"}')
            updated = receive_badge(client, 0x1f)
            assert updated[2:18] == show[2:18], "badge update changed identity"
            permission_checked = False
            if args.backend_port is None:
                # This player is a non-operator on the isolated backend. Both
                # commands must produce feedback without emitting badge packets.
                for denied in [
                    'eaglernotify * {"title":"Unauthorized"}',
                    'eaglernotifyhide * {"id":"test"}',
                ]:
                    client.send(b"\x07" + text(denied))
                    deadline = time.monotonic() + 2
                    feedback = False
                    while time.monotonic() < deadline:
                        client.sock.settimeout(max(0.01, deadline - time.monotonic()))
                        try:
                            packet = client.receive()
                        except socket.timeout:
                            break
                        assert packet[:2] not in [b"\xee\x1f", b"\xee\x20"], "non-operator command emitted badge"
                        kind, offset = read_vi(packet)
                        if kind == 124:
                            feedback = True
                        elif kind == 45:
                            client.send(b"\x1c" + packet[offset:])
                    assert feedback, "denied command produced no feedback"
                permission_checked = True
            # Additional release assessment: real unload/reload with active sessions.
            (root / "EaglerXPumpkin.wasm").symlink_to(plugins / "EaglerXPumpkin.wasm")
            def wait_log(since, phrase):
                end = time.monotonic() + 15
                while time.monotonic() < end:
                    recent = (root / "console.log").read_text()[since:]
                    if phrase in recent:
                        return
                    if "Failed to load plugin" in recent or "Failed to unload plugin" in recent:
                        raise AssertionError(recent)
                    time.sleep(0.1)
                raise AssertionError("host lifecycle deadline: " + phrase)
            for cycle in range(3):
                mark = len((root / "console.log").read_text())
                command("plugin unload eaglerx-pumpkin")
                wait_log(mark, "unloaded successfully")
                closed = False
                client.sock.settimeout(2)
                try:
                    while client.sock.recv(65536):
                        pass
                    closed = True
                except (ConnectionResetError, BrokenPipeError):
                    closed = True
                except socket.timeout:
                    pass
                assert closed, "active session survived plugin unload"
                client.sock.close()
                client = None
                try:
                    sock = socket.create_connection(("127.0.0.1", port), timeout=1)
                except ConnectionRefusedError:
                    pass
                else:
                    sock.close()
                    raise AssertionError("listener survived plugin unload")
                mark = len((root / "console.log").read_text())
                command("plugin load EaglerXPumpkin.wasm")
                wait_log(mark, "loaded successfully")
                client = Client("127.0.0.1", port)
                name = "BadgeLife" + str(cycle)
                login(client, name)
                command('eaglernotify ' + name + ' {"id":"lifecycle","title":"Reloaded"}')
                receive_badge(client, 0x1f)
            result.update(lifecycle_passed=True, lifecycle_cycles=3)
            result.update(passed=True, checks=["command registration", "capability negotiation",
                "targeted highest/silent alert", "dismissal by ID", "broadcast update preserves badge UUID"], non_operator_permissions_checked=permission_checked)
        except Exception as error:
            result["error"] = str(error)
            raise
        finally:
            if client:
                client.sock.close()
            if process.poll() is None:
                process.stdin.write("stop\n")
                process.stdin.flush()
                try:
                    process.wait(timeout=20)
                except subprocess.TimeoutExpired:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
            (root / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
