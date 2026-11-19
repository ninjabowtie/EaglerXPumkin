# Notification controls: local implementation and host validation

The local changes add dismissal by badge ID, four original Eagler priorities
(low, normal, higher, highest), and silent alerts. Both static welcome messages
and dynamic alerts now share a structured encoder instead of patching offsets
in a previously encoded packet. Defaults preserve existing welcome behavior.

`eaglernotifyhide <username|*> {"id":"maintenance"}` dismisses the same dynamic
badge ID previously shown by `eaglernotify`. Both commands use permission
`eaglerx-pumpkin:command.notify`, defaulting to operator level 2. Both share
capability gating, bounded send queues, target checks and the dynamic feature's
opt-in switch. Static welcome badges are separate from these command IDs.

## Important fixes discovered by running the real WASI host

- The earlier local notification command used permission namespace
  `eaglerxpumpkin`. Pumpkin rejected plugin initialization because the namespace
  must match `eaglerx-pumpkin`. Corrected both registration and documentation.
- The original command argument used a normal single-word parser for `*`.
  Pumpkin rejected unquoted broadcasts. Both commands now register an explicit
  literal `*` branch alongside the username argument.

These failures were not detected by compilation or native runtime tests.
`tests/probe_notifications.py` now reproduces the complete host command path.
It creates an isolated world/backend/gateway on loopback, uses only its own
plugin permission cache, and stops only its child server. The installed server
and its plugin/config remain unchanged. An optional external backend is also
supported, with the isolated-backend non-operator check explicitly skipped.

## Passing checks

- 68 Rust tests: codec, gateway/session/unit tests and real-socket integration.
  Covers every priority/silent flag combination, timestamp layout, matching
  show/hide UUIDs, invalid requests and target/capability/send-queue handling.
- 13 Python regressions, native and WASI Clippy with warnings denied, formatting,
  diff whitespace checks and locked WASI release build.
- Actual WASI plugin loaded in isolated Pumpkin: capability negotiation, targeted
  highest-priority silent show, matching hide packet, broadcast update preserving
  UUID, and rejection of both commands from a non-operator player.
  See [host-result.json](host-result.json), [host-console.log](host-console.log),
  and [rust-tests.log](rust-tests.log).
- Isolated child server shut down normally after the successful run.

Reproduce from `rust/` with a new directory:

```sh
./build.sh
python3 tests/probe_notifications.py --pumpkin-binary /home/thomass/pumpkin-server/pumpkin-server --output-dir /tmp/NEW-notification-test
```

The successful run used the existing local Pumpkin executable with SHA256
recorded below. The plugin artifact hash is in `host-result.json`.
No graphical client was launched; visual rendering, animation and notification
sound remain unverified. This is a functional smoke test, not a capacity/soak test.
Implementation changes and validation evidence are included with this report.

## Reference protocol

Packet shapes and priority values were checked against upstream
[show packet](https://github.com/lax1dude/eaglerxserver/blob/main/protocol-game/src/main/java/net/lax1dude/eaglercraft/v1_8/socket/protocol/pkt/server/SPacketNotifBadgeShowV4EAG.java),
[hide packet](https://github.com/lax1dude/eaglerxserver/blob/main/protocol-game/src/main/java/net/lax1dude/eaglercraft/v1_8/socket/protocol/pkt/server/SPacketNotifBadgeHideV4EAG.java),
and the V5 message registration in
[GamePluginMessageProtocol](https://github.com/lax1dude/eaglerxserver/blob/main/protocol-game/src/main/java/net/lax1dude/eaglercraft/v1_8/socket/protocol/GamePluginMessageProtocol.java).
9882aa350ee8674d12870ee7c9fa6eed387101c32c34a5f3071a9040b1c29fd3  /home/thomass/pumpkin-server/pumpkin-server
