# Pumpkin 26.3 gateway

A runnable WASI Preview 2 plugin for stock Pumpkin, plus a native development
CLI using the same networking state machine. Legacy Java reference code is
available in Git history and in upstream EaglerXServer.

## Verified compatibility

Tested with **Pumpkin 0.2.2+26.3-26.60** (Minecraft protocol **777**) and
**Gaius 26.3 portable v0.4.0-c263**. A human browser play test confirmed world
loading, movement, chat, breaking and placing blocks.

Two transports share port 8081:

- `/tunnel`: Gaius connect/connected JSON control messages, binary frames carrying
  the complete vanilla Minecraft TCP stream, and flow pause/resume messages.
  The only destination is the configured loopback backend; this is not a
  general-purpose relay. The requested port must match `backend_port`.
- `/`: Eagler handshake negotiation, offline profile login, backend login
  injection, compression handling and packet body framing. Eagler v5 with
  protocol 777 passed the live protocol probe. A graphical Eagler client was
  not used for that path; the tested graphical client is Gaius, which uses
  `/tunnel` instead of the Eagler handshake.

Unmodified vanilla TCP connections on the gateway port are also relayed to the
same backend. Normal Java clients can still use Pumpkin's own port.

## Build and install

Install stable Rust (tested with 1.100.0) and the WASI target:

```sh
rustup target add wasm32-wasip2
./rust/build.sh
```

Copy `rust/dist/EaglerXPumkin.wasm` to your Pumpkin server's `plugins/` folder.
Start Pumpkin and grant the plugin's requested permissions:
`network.tcp.bind`, `network.tcp.connect`, `fs.read.data`, `fs.write.data`.
The plugin creates `plugins/data/eaglerx-pumkin/eagler.toml` on first load.
A listener or config error fails the plugin load instead of pretending to run.

Pumpkin's v0.1 plugin host uses WASI Preview 2. Do not change the build to
`wasm32-wasip3` or enable guest threads: that combination is incompatible with
this pinned API. The runtime retains its listener and connections and polls
nonblocking I/O once per server tick. Unloading cancels its task and closes
all sockets.

## Server configuration and connecting

In Pumpkin's `pumpkin.toml`, configure the Java listener for offline access:

```toml
[networking.java]
address = "127.0.0.1:25565"
online_mode = false
encryption = false
```

Offline mode identifies players by their supplied username, without Mojang
account verification. The example binds the backend to loopback so browsers
enter through the gateway. Use TLS termination for public HTTPS-hosted clients.

The plugin defaults to `0.0.0.0:8081` -> `127.0.0.1:25565`.
See [eagler.example.toml](eagler.example.toml) for all settings. Ports must differ; the backend must
be a numeric loopback address. Restart/reload the plugin after changing config.
`server_address` is the Minecraft handshake hostname injected for Eagler clients;
Gaius sends its own Minecraft handshake unchanged.

For the tested Gaius HTML, use an offline profile, then Multiplayer -> Direct
Connect -> **127.0.0.1:25565**. Gaius automatically tries
`ws://127.0.0.1:8081/tunnel`. Its `directPluginPort` URL option can select a
custom gateway port. Enter the Minecraft backend port in Gaius, not port 8081.
For Eaglercraft, use **ws://127.0.0.1:8081/** with a protocol 777 client.

## Development and verification

For the native CLI, with Pumpkin already running and the WASM plugin disabled:

```sh
./rust/run-native.sh
```

It creates `rust/eagler.toml` on first run. You can supply a config path as the
first argument. Do not run the plugin and native CLI on the same listen port.

From `rust/`, run unit tests on the native host (override the default WASI target):

```sh
cargo test --workspace --target "$(rustc -vV | sed -n 's/^host: //p')"
```

With the real server and gateway running, from the repository root:

```sh
python3 rust/tests/probe_26_3.py --transport eagler
python3 rust/tests/probe_26_3.py --transport gaius
```

Both live probes completed configuration, teleport acknowledgement, chunk
streaming, returned chat and a keepalive round trip through the WASM plugin.
Each received 32 registries and 118 chunks (approximately 7.7 MB of Minecraft
packet bodies). Unit coverage includes partial TCP reads/writes, incremental
WebSocket fragmentation, 64 KiB+ frames, HTTP upgrades, compression transitions,
malformed lengths and Eagler login ordering. This is not a throughput benchmark.

## Remaining port work

26.3 is the current target. Older Minecraft clients require a translation layer;
Pumpkin's version enum alone does not provide that. Skin caching/custom skins,
password authentication, notifications, pause menu extensions, EaglerWeb,
rewind and full Java feature parity remain outside this working first release.
The gateway does not advertise optional Eagler v5 capabilities it cannot provide.

Eaglercraft voice chat (IVMP) is intentionally out of scope. Voice chat will be
handled by a separate plugin selected by the project owner.
