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

Copy `rust/dist/EaglerXPumpkin.wasm` to your Pumpkin server's `plugins/` folder.
Start Pumpkin and grant the plugin's requested permissions:
`network.tcp.bind`, `network.tcp.connect`, `fs.read.data`, `fs.write.data`.
The plugin creates `plugins/data/eaglerx-pumpkin/eagler.toml` on first load.
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

`connections_per_minute` (default `30`, `0` disables) limits new connections per
source address over a rolling minute. `max_connections` remains the global cap;
connections above either limit are accepted and closed immediately. Tune both
for public deployments and load tests.

`ip_allowlist` and `ip_blocklist` accept exact addresses or CIDR ranges (IPv4 or
IPv6). When the allowlist is non-empty, only matching addresses may connect; the
blocklist always takes precedence. Rules are checked before the rate limiter, so
blocked attempts do not consume an address's connection budget. Rejected
attempts are counted separately in diagnostics.

For the tested Gaius HTML, use an offline profile, then Multiplayer -> Direct
Connect -> **127.0.0.1:25565**. Gaius automatically tries
`ws://127.0.0.1:8081/tunnel`. Its `directPluginPort` URL option can select a
custom gateway port. Enter the Minecraft backend port in Gaius, not port 8081.
For Eaglercraft, use **ws://127.0.0.1:8081/** with a protocol 777 client.

## Diagnostics

Every `diagnostics_interval_secs` (default `60`, `0` disables) the gateway logs
one `Eagler gateway:` line with active and playing counts, lifetime peaks,
accepted/closed/refused/filtered/rate-limited totals, and throughput since the
previous report. Rejected connections and failed sessions also log the peer address at
debug/warn level. The counters are cumulative and reset only on plugin reload;
use the periodic throughput figures for sustained-load measurements rather than
wall-clock timers in the client.

## Skins

Eagler v5 players send `skin_v1`/`skin_v2`/`cape_v1` profile data during login.
The gateway decodes presets and custom 64x64 textures into a bounded in-memory
registry keyed by the player's UUID. Requests for a player with no profile data
receive the same deterministic Steve/Alex fallback as EaglerXServer. Injected
`GetOtherSkin` (0x01), `GetOtherCape` (0x02) and `GetOtherTextures` (0x03)
messages, including `0xFF` multi-packet batches, are answered locally and are
never forwarded to Pumpkin. The gateway also sends a V5 client-state flag that
disables URL skin lookups, because it does not fetch remote images. Voice,
webview and server-info injected messages are dropped for the same reason.

Entries are removed when the last session for that UUID disconnects; skins are
not persisted, since only connected players need textures.

## Pause menu and join notifications

Two optional V5 client features are available through static config:

```toml
pause_menu_button = "Discord"
pause_menu_url = "https://example.com/invite"
notification_title = "Welcome"
notification_body = "Have fun!"
notification_source = "EaglerXPumpkin"
notification_hide_after_secs = 10
notification_expire_after_secs = 3600
```

`pause_menu_button` adds a button to the pause menu that opens
`pause_menu_url` in a new browser tab. Set both fields or leave both empty. The
notification fields show one text notification after the player joins the
world; the badge reuses a stable UUID so a reconnect replaces the previous
message instead of stacking it. Notifications require at least one of the three
text fields.

Both features are advertised during the V5 capability exchange only when they
are configured and the client supports them, so clients without V5 support are
unaffected. Only static content is implemented; per-event notifications and
message channels driven by the backend remain future work.

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
malformed lengths, Eagler login ordering, skin profile parsing and injected
skin responses, and per-address rate limiting. This is not a throughput
benchmark.

## Remaining port work

26.3 is the current target. Eaglercraft uses offline accounts, so the offline
username login is the intended model and no password/registration flow is
planned. Older Minecraft clients require a translation layer; Pumpkin's version
enum alone does not provide that. The adaptive packet batcher's single-flush
coalescing is inherent in the poll-driven send queue, and the wire format keeps
one packet body per frame as in the reference. Dynamic backend-driven
notifications, message channels, EaglerWeb, rewind and full Java feature parity
remain outside this working first release. V5 skins
are implemented in memory for connected players; there is no persistent skin
cache or URL skin fetching. Static pause-menu links and join notifications are
available when configured. The gateway does not advertise optional Eagler v5
capabilities it cannot provide.

Eaglercraft voice chat (IVMP) is intentionally out of scope. Voice chat will be
handled by a separate plugin selected by the project owner.
