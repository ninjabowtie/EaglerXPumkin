# Pumpkin 26.3 gateway

A runnable WASI Preview 2 plugin for stock Pumpkin, plus a native development
CLI using the same networking state machine. Legacy Java reference code is
available in Git history and in upstream EaglerXServer.

**Other client versions require [Multiversion](#multiversion-companion-setup).**
EaglerXPumpkin alone does not let clients on versions other than the server's
version join (26.3 for the tested server). It provides the gateway transport
and handshake handling, not Minecraft protocol translation.

## Verified compatibility

Tested with **Pumpkin 0.2.2+26.3-26.60** (Minecraft protocol **777**) and
**Gaius 26.3 portable v0.4.0-c263**. A human browser play test confirmed world
loading, movement, chat, breaking and placing blocks.
Gaius uses a separate tunnel transport; its graphical play test does not
establish graphical Eaglercraft compatibility.

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

## 1.17+ client negotiation

The gateway negotiates Minecraft versions with Eaglercraft clients: it selects
the highest client-offered protocol inside the configured
`min_minecraft_protocol`/`max_minecraft_protocol` range (see
[eagler.example.toml](eagler.example.toml)), injects the version-correct Login
Start packet for the tested protocol layouts from 1.17 through 26.3, and
rejects unsupported clients with a mismatch packet that names the accepted protocols. With both
fields unset, only `minecraft_protocol` (default 777, Minecraft 26.3) is
accepted, which keeps standalone installs on the verified setup.

Accepting 1.17+ clients on versions other than the server's still requires a
translation layer: the backend must
translate the client's protocol to Pumpkin's, which is the job of
[Multiversion](#multiversion-companion-setup) in this setup. Each exact client
build and protocol still needs verification with a real graphical client before
it can be called supported; see the
[roadmap](../docs/PUMPKIN_PORT_PLAN.md#planned-eaglercraft-117-client-support)
for the pending matrix entries. Eagler handshake versions and Minecraft game
protocols are separate compatibility decisions, and the raw `/tunnel`
transport carries whatever Minecraft stream the client sends.

## Build and install

From the repository root, install stable Rust (tested with 1.100.0) and the
WASI target:

```sh
rustup target add wasm32-wasip2
./rust/build.sh
```

Copy `rust/dist/EaglerXPumpkin.wasm` to your Pumpkin server's `plugins/` folder.
Start Pumpkin and grant the plugin's requested permissions:
`network.tcp.bind`, `network.tcp.connect`, `fs.read.data`, `fs.write.data`.
The plugin creates `plugins/data/eaglerx-pumpkin/eagler.toml` on first load.
A listener or config error fails the plugin load instead of pretending to run.

Pumpkin's plugin host uses WASI Preview 2. Do not change the build to
`wasm32-wasip3` or enable guest threads: that combination is incompatible with
the pinned `pumpkin-plugin-api` 0.2.1. The runtime retains its listener and
nonblocking I/O once per server tick. Unloading cancels its task and closes
all sockets.

## Multiversion companion setup

Use [Multiversion](https://market.pumpkinmc.org/plugin/s7e5Nz2f-multiversion) alongside
EaglerXPumpkin to let clients on versions other than the server's join.
Without Multiversion, clients must match the server version (26.3 in the tested
setup). 1.17+ Eaglercraft support across different server and client versions relies
on this plugin for Minecraft protocol translation rather than adding a separate
translator to EaglerXPumpkin. Install both WASM plugins in Pumpkin's
`plugins/` directory and grant each plugin its requested permissions.

Enable the client range in EaglerXPumpkin's `eagler.toml` by setting both
bounds together:

```toml
min_minecraft_protocol = 755   # Minecraft 1.17
max_minecraft_protocol = 777   # Minecraft 26.3
```

The gateway then accepts every client protocol in that inclusive range, selects
the highest one the client offers, and tells rejected clients the accepted
range. Omit both fields to accept only `minecraft_protocol`.
These bounds only control gateway negotiation; widening the range does not
translate protocols or allow other versions to join without Multiversion.

Installing Multiversion does not change an existing EaglerXPumpkin config.
If older clients are rejected with "Use a Minecraft 26.3 Eaglercraft client
(protocol 777)", check that both bounds above are set in the active
`plugins/data/eaglerx-pumpkin/eagler.toml`, then restart/reload the gateway.
Its startup log must report `MC protocols 755-777`, rather than `777-777`.
Also verify that `backend_port` points to Multiversion's listener, not directly
to Pumpkin. A successful version negotiation alone does not verify gameplay.
See the [Eagler-only investigation](tests/results/eagler-client-investigation/README.md)
for a reproduced exact-777 configuration failure and checks after enabling
the range, including their real-client testing limits.

The local test server uses this route:

```text
Eagler/Gaius WebSocket :8081 -> Multiversion 127.0.0.1:25565 -> Pumpkin 127.0.0.1:25566
Java TCP clients           -> Multiversion :25565         -> Pumpkin 127.0.0.1:25566
```

Keep EaglerXPumpkin's `backend_address = "127.0.0.1"` and `backend_port = 25565`.
In Multiversion's `plugins/data/multiversion/config.toml`, set
`backend.address = "127.0.0.1:25566"` and `auth.online_mode = false` for the
offline Eaglercraft test setup. In Pumpkin's `pumpkin.toml`:

- Set `[networking.java]` to `address = "127.0.0.1:25566"`,
  `online_mode = false` and `encryption = false`.
- Enable `[networking.proxy]` and `[networking.proxy.velocity]`.
- Match the Velocity `secret` to Multiversion's generated
  `backend.forwarding_secret`; use the value generated for your installation.
- Disable `[networking.java.compression]` when Multiversion's
  `backend.pumpkin_compression = false`, as in the local setup.

Follow Multiversion's generated `pumpkin-settings.txt` and startup diagnostics
for its installed version. Restart Pumpkin after changing these settings and
check that Multiversion accepts connections and its forwarding setup check
passes. Connect Java/Gaius clients using port 25565; the WebSocket address
remains `ws://127.0.0.1:8081/`.

Local verification with Multiversion 0.6.1 covered a simulated Minecraft 1.17
(protocol 755) Eagler play flow through the proxy — join, teleport
acknowledgement, keepalive round trip, chunk data and chat echo — plus
negotiation checks: the highest client-offered protocol is selected (offering
755/776/777 selects 777, offering 755/776 selects 776) and out-of-range offers
receive the mismatch packet. The existing 26.3 Eagler and Gaius probes still
pass through the proxy and were re-run against a standalone backend without
Multiversion (exact-777 configuration; its mismatch reply advertises protocol
777). This is not a real graphical 1.17 gameplay test: no 1.17+ Eaglercraft
client has been verified graphically yet; per-client matrix entries remain open.

## Compatibility milestone evidence

The [compatibility matrix](../tests/compatibility/README.md) inventories exact local
client artifacts and records a simultaneous simulated-client run: Eagler 1.17,
Eagler 26.3, Gaius tunnel 26.3, and vanilla TCP through the gateway. All four
passed login, chunk, teleport, chat and keepalive checks. A subsequent recorded
run kept all four connected for 120 seconds and verified reconnects with the
same identities. Graphical older-client acceptance and longer soak measurements
remain pending.

With the documented Multiversion test route running, from `rust/`:

```sh
python3 tests/probe_compatibility.py --output-dir /tmp/compatibility-new-run
```

For sustained sessions and reconnects, add `--sustain --seconds 120 --reconnect`.
The runner requires a new output directory, saves JSON results and logs, and
exits nonzero on a failed case. See the [testing guide](../docs/TESTING.md) for
all probe modes, recorded evidence and failure interpretation.

## Server configuration and connecting

For a standalone same-version setup without Multiversion, configure the Java
listener in Pumpkin's `pumpkin.toml` for offline access:

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
See [eagler.example.toml](eagler.example.toml) for all settings. The listener and
backend must not overlap on the same address/port; the backend must be a
numeric loopback address. Restart/reload the plugin after changing config.
`server_address` is the Minecraft handshake hostname injected for Eagler clients;
Gaius sends its own Minecraft handshake unchanged.

`connections_per_minute` (default `30`, `0` disables) limits new connections per
source address over a rolling minute. `max_connections` remains the global cap;
connections above either limit are accepted and closed immediately. Tune both
for public deployments and load tests.

`handshake_timeout_secs` (default `30`, range `1..3600`) sets the absolute
HTTP/Eagler login deadline; partial traffic does not extend it. Established
Gaius tunnels, raw TCP relays and MOTD animations are exempt from this deadline.
`idle_timeout_secs` (default `300`, range `1..86400`) closes any connection with
no traffic for that duration. Both settings must be positive; existing configs
retain the defaults. Restart/reload after changing them.

`ip_allowlist` and `ip_blocklist` accept exact addresses or CIDR ranges (IPv4 or
IPv6). When the allowlist is non-empty, only matching addresses may connect; the
blocklist always takes precedence. Rules are checked before the rate limiter, so
blocked attempts do not consume an address's connection budget. Rejected
attempts are counted separately in diagnostics.

For the tested Gaius HTML, use an offline profile, then Multiplayer -> Direct
Connect -> **127.0.0.1:25565**. Gaius automatically tries
`ws://127.0.0.1:8081/tunnel`. Its `directPluginPort` URL option can select a
custom gateway port. Enter the Minecraft backend port in Gaius, not port 8081.
For Eaglercraft, use **ws://127.0.0.1:8081/** with a protocol 777 client, or
with any client protocol inside the configured
`min_minecraft_protocol`/`max_minecraft_protocol` range when Multiversion is
installed and configured for protocol translation. Without Multiversion, use
a client matching the server version (26.3 / protocol 777 in the tested setup).

## Diagnostics

Every `diagnostics_interval_secs` (default `60`, `0` disables) the gateway logs
one `Eagler gateway:` line with active and playing counts, lifetime peaks,
accepted/closed/refused/filtered/rate-limited totals, and throughput since the
previous report. Rejected connections and failed sessions also log the peer address at
debug/warn level. The counters are cumulative and reset only on plugin reload;
use the periodic throughput figures for sustained-load measurements rather than
wall-clock timers in the client.

## MOTD and animations

The `motd` string answers Eagler server-list queries with up to two lines and
is sent once per query with `"cache": true`, after which the query connection
closes.

The answer also carries a server icon. By default it is the repository's
`EaglerXPumpkin.png`, resized to 64×64 and embedded in the gateway. The JSON
announces `"icon": true`, followed by a separate binary WebSocket frame of
16384 RGBA bytes, as required by Eagler. Static `MOTD.noicon` queries omit the
binary frame and announce `"icon": false`.

Set `motd_icon` to a base64 PNG data URI (`data:image/png;base64,...`) to
override the default. The PNG must be non-animated and exactly 64×64; the URI
must be at most 2097152 bytes. Invalid images and image URLs fail config
validation at startup. The gateway does not fetch remote images. Download
and resize an image before encoding it. Leave the field unset for the built-in
icon. Animated MOTD frames currently advertise no icon.

To regenerate the embedded icon after changing the project logo, run
`python3 scripts/prepare_motd_icon.py` from `rust/` (requires Pillow).

Set `motd_frames` to animate it instead. Each entry is one full frame in the
same `line1\nline2` format (up to 32 frames, 2048 bytes each). The query
connection then stays open: frames cycle every `motd_interval_ms`
(1..=60000) and the connection is force-closed after `motd_close_after_ms`
(1..=3600000, at least the interval), mirroring EaglerMOTD's interval and
`close_socket_after`. Animated frames are sent with `"cache": false` and the
reference's epoch-millisecond timestamp so browsers re-query the sequence
instead of caching a stale picture. The gateway sends at most one frame per
server tick, so intervals below ~50 ms collapse to tick granularity.

Animation is limited to cycling text frames: per-frame server icons, sprite
sheets, weighted message pools, named message chains and the cache-control
query subtypes of the EaglerMOTD addon are not ported.

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

Both features are advertised during V5 capability exchange only when configured
and supported by the client. Static welcome messages still work independently
of dynamic notifications.

### Dynamic notification commands

Set `dynamic_notifications = true` in `eagler.toml`, reload/restart the plugin,
and reconnect existing clients so they negotiate notification support. The
plugin registers these Pumpkin commands only when enabled:

```text
eaglernotify <exact_username|*> <notification JSON>
eaglernotifyhide <exact_username|*> {"id":"badge ID"}
```

From the server console (use `/` before the command in game):

```text
eaglernotify * {"id":"maintenance","title":"Server maintenance","body":"Restart in five minutes","source":"Admin","hide_after_secs":15,"expire_after_secs":300}
eaglernotify PlayerName {"id":"reward","title":"Quest complete!","body":"Your reward is ready."}
eaglernotify * {"id":"maintenance","title":"Maintenance postponed","body":"Keep playing!","priority":"higher","silent":true}
eaglernotifyhide * {"id":"maintenance"}
```

The permission node is `eaglerx-pumpkin:command.notify`, granted by default to
operators at level 2 or above. Console, command blocks and other server plugins
can invoke the command through Pumpkin's normal command dispatcher, subject to
its permission checks. This provides server-driven alerts without exposing a
new network listener or reading commands from clients.

Show JSON accepts `id`, `title`, `body`, `source`, `priority`, `silent`,
`hide_after_secs` and `expire_after_secs`; unknown fields are rejected.
Priority is `low`, `normal` (default), `higher` or `highest`. `silent` defaults
to false. These use the original Eagler protocol flags.
Dismiss JSON requires only `id` (same 1–64 byte ID used to show the badge),
rejects unknown fields, and is limited to 1024 bytes. It removes that badge from
the selected clients; dismissing an absent ID is harmless. It cannot dismiss
the separate static welcome badge. Defaults are ID `server`,
source `EaglerXPumpkin`, hide after 10 seconds and expire after 3600 seconds.
A title or body is required. Text is literal; JSON text components are not
interpreted. Reusing an ID replaces that badge for each recipient; different
IDs coexist. IDs are 1–64 UTF-8 bytes. Serialized text components must fit
255 bytes for title/source and 32767 bytes for body. The command JSON is capped
at 65536 bytes. Hide time must be positive and cannot exceed expiration.

Recipients are connected Eagler V5 sessions that negotiated notifications.
`*` broadcasts; usernames are exact and case-sensitive. Gaius tunnels, raw Java
connections, unsupported clients and closing connections are skipped. Full send
queues are skipped and logged. Feedback reports **queued connections**, not
client acknowledgements. Offline alerts are not retained or replayed on login.

Native embedders can call `Runtime::notify(target, &Notification)` using the
same validated request type in `eaglerx_pumpkin::notifications`. The native CLI
does not register Pumpkin commands. Native callers can also use
`Runtime::dismiss_notification(target, &DismissNotification)`. Custom badge
icons and arbitrary client/backend message channels remain future work.

For an isolated live WASI host test, from `rust/`:

```sh
python3 tests/probe_notifications.py --pumpkin-binary /path/to/pumpkin-server --output-dir /tmp/NEW-badge-test
```

This starts a separate Pumpkin world/backend and gateway on loopback ports,
checks console show/dismiss/broadcast commands with a simulated capable client,
checks non-operator rejection, and shuts the child server down. It needs a new
output directory and saves a console log plus `result.json`. It does not modify
the installed server. Optional `--backend-port` uses an existing offline 777
backend and skips the isolated-backend permission check. Graphical rendering
still requires a real Eagler client.

## Development and verification

For the native CLI, with Pumpkin already running and the WASM plugin disabled:

```sh
./rust/run-native.sh
```

It creates `rust/eagler.toml` on first run. You can supply a config path as the
first argument. Do not run the plugin and native CLI on the same listen port.

From `rust/`, run the Rust suite on the native host (override the default WASI target):

```sh
cargo test --workspace --target "$(rustc -vV | sed -n 's/^host: //p')"
```

This also runs the real-socket integration tests in
[crates/eaglerx-pumpkin/tests/gateway.rs](crates/eaglerx-pumpkin/tests/gateway.rs),
which exercise the HTTP upgrade, animated MOTD frame cycling with the max-age
close, static MOTD JSON/binary icon ordering, custom icons and no-icon queries, malformed-
session resilience and the connection rate limit through the runtime's poll
loop with a background poller thread. No Pumpkin backend is
required for those paths. Session tests cover every protocol 755–777 with
compression disabled or enabled at thresholds 1 and 256, including login
injection, Login Acknowledged, buffering and packet relay. The native suite
is covered by the validation results in [the dev review](tests/results/dev-review/README.md).

From the repository root, run the Python tests without a server:

```sh
python3 -m unittest discover -s rust/tests -p 'test_*.py'
```

The 13 tests cover release inputs, probe deadlines and peer-close assertions.
See [testing and debugging](../docs/TESTING.md) for formatting/Clippy checks and
the complete test-tool reference.

With the real server and gateway running, from the repository root:

```sh
python3 rust/tests/probe_26_3.py --transport eagler
python3 rust/tests/probe_26_3.py --transport gaius
python3 rust/tests/probe_26_3.py --transport vanilla
python3 rust/tests/probe_1_17.py --seconds 30
```

The original two 26.3 live probes completed configuration, teleport
acknowledgement, chunk streaming, returned chat and a keepalive round trip through the WASM plugin.
Each received 32 registries and 118 chunks (approximately 7.7 MB of Minecraft
packet bodies). Unit coverage includes partial TCP reads/writes, incremental
WebSocket fragmentation, 64 KiB+ frames, HTTP upgrades, compression transitions,
malformed lengths, Eagler login ordering, skin profile parsing and injected
skin responses, per-address rate limiting, and animated MOTD frame cycling
with the max-age close. The `eagler-ws` frame codec micro-benchmark
(`frame_codec_micro_benchmark`) measures masked-client decoding and server-reply
encoding per iteration for representative payloads (64 B through 256 KiB).
It reports payload throughput and mean codec time per iteration, excluding
sockets and backend work. Earlier recorded numbers timed decoding only and
should not be treated as encode/decode round-trip measurements.

For a sustained multi-client soak with fuzzing, with the real server and
gateway running, from the repository root:

```sh
python3 rust/tests/soak_26_3.py --clients 5 --fuzz-clients 2 --seconds 120
```

The soak supports concurrent simulated 26.3 Eagler and Gaius players sending
movement/chat, MOTD listener checks and six malformed-input vectors. The new
recorded health/fuzz phase used `--clients 0` after the sustained mixed run;
it did not validate simultaneous fuzzing and mixed-version gameplay. Timeouts
no longer count as successful peer closes, and MOTD queries must receive a
text frame. Use `--json-output PATH` to retain final counters and failures.
It exits nonzero on any health-check failure, unexpected disconnect or unclosed fuzz
connection, and prints periodic plus final counters. Use it together with the
periodic `Eagler gateway:` diagnostics lines when tuning the connection limits
for public deployments.

## Remaining port work

26.3 is the current target. Eaglercraft uses offline accounts, so the offline
username login is the intended model and no password/registration flow is
planned. The gateway negotiates 1.17+ client protocols when a protocol range is
configured, but gameplay compatibility for those versions still depends on a
translation layer (Multiversion) and per-client graphical verification;
Pumpkin's version enum alone does not provide translation. Pre-1.17 clients
remain deferred. The adaptive packet batcher's single-flush
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
