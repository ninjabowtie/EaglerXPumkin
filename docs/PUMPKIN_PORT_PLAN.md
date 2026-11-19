# Pumpkin port roadmap

## Current target

A Rust WASM plugin for stock **Pumpkin Minecraft 26.3** (protocol **777**).
The native development CLI shares the plugin's networking runtime.

Verified on Pumpkin 0.2.2+26.3-26.60 with Gaius 26.3 portable v0.4.0-c263.
The browser play test confirmed world loading, movement, chat, block breaking
and placement. Both live transport probes also completed configuration,
chunk streaming, returned chat and keepalive exchanges.

The original Phase 0 spike and Phase 1 core gateway are complete for this
target. Later phases are partially adopted; the checklist below records the
original plan's remaining work.

See [setup and configuration](../rust/README.md), the
[testing guide](TESTING.md) and [recorded evidence](../tests/README.md) for
commands, results and tested scope.

## Planned Eaglercraft 1.17+ client support

The next compatibility goal is Eaglercraft clients targeting **Minecraft 1.17
and newer**, while retaining the current Pumpkin 26.3 backend and its verified
connection paths. Protocol negotiation and version-correct login injection for
1.17+ are implemented; end-to-end client support still depends on the
Multiversion translation path and per-client verification, so individual
versions are not yet marked supported. This does not imply that a compatible
Eaglercraft build exists for every Minecraft release, or that every client at
or above 1.17 is supported automatically. Support will be recorded for exact
client builds, Minecraft protocol versions and Pumpkin releases after testing.
Clients below 1.17, including the original 1.8/1.12 targets and Rewind 1.5.2,
remain deferred separately.

### Compatibility design

- Treat the Eagler handshake version and the Minecraft game protocol as separate
  compatibility decisions. A successful WebSocket upgrade or Eagler handshake
  alone does not establish gameplay compatibility.
- Inventory available 1.17+ client builds and capture their handshake, login,
  profile and capability exchanges. Preserve the initial offline identity when
  nickname selection is disabled, and test empty nickname fields as well as
  explicit names. Advertise only implemented Eagler capabilities.
- Rely on [Multiversion](https://market.pumpkinmc.org/plugin/s7e5Nz2f-multiversion) as the
  companion plugin for Minecraft protocol translation. Install it alongside
  EaglerXPumpkin and route the gateway's loopback backend through its listener.
  EaglerXPumpkin owns WebSocket/Eagler transport, identity and negotiation;
  Multiversion owns translation to the Pumpkin backend. Maintain this dependency
  when implementing 1.17+ support and validate compatible plugin releases.
- Keep the client's selected protocol distinct from the backend protocol.
  Translation must cover packet IDs and payloads, registries, chunks, entity
  metadata, inventory, chat, and login/configuration state differences as
  applicable. Changing a protocol number or adding an enum variant is not
  sufficient.
- Adapt Eagler login injection, compression and play packet handling to the
  selected translation boundary. Validate `/tunnel` independently: its raw
  Minecraft stream still needs a compatible backend or translation path.
- Reject unsupported versions with an actionable compatibility message before
  attempting gameplay. Keep the existing loopback destination restriction,
  resource limits and address policies when introducing a translation route.

### Milestones and acceptance

- [ ] Inventory exact 1.17+ Eaglercraft builds and establish a version matrix:
  client/build, Minecraft protocol, Eagler handshake version, transport,
  Pumpkin release, translation component/version, test date and result.
  Started: the [compatibility matrix](../tests/compatibility/README.md)
  records SHA-256 hashes for local Eaglercraft 26.2 0.3-dev and Gaius
  1.21.11/26.2/26.3 builds. Observed game protocols, Eagler handshake versions,
  upstream provenance and graphical acceptance remain pending.
- [x] Select Multiversion as the companion translation dependency and document
  the loopback proxy setup. Local Multiversion 0.6.1 verification covered a
  simulated 1.17 (protocol 755) Eagler play flow through the proxy — join,
  teleport acknowledgement, keepalive round trip, chunk data and chat echo —
  plus negotiation checks: the highest client-offered protocol is selected
  (offering 755/776/777 selects 777, offering 755/776 selects 776) and
  out-of-range offers receive the mismatch packet naming the accepted range.
  The existing 26.3 Eagler/Gaius probes still pass through the proxy and were
  re-run against a standalone backend without Multiversion (exact-777
  configuration; its mismatch reply advertises protocol 777).
- [ ] Validate a real 1.17+ Eaglercraft client whose game protocol differs from
  the current backend; the TCP login check alone does not meet this milestone.
- [x] Implement version negotiation and routing with explicit supported-version
  checks, initial-identity regression coverage and clear disconnect reasons.
  The gateway selects the highest client-offered protocol inside the configured
  `min_minecraft_protocol`/`max_minecraft_protocol` range, injects the
  version-correct Login Start packet for each 1.17+ protocol generation
  (1.17-1.18.2, 1.19, 1.19.2, 1.19.3-1.20.1, 1.20.2+), sends Login
  Acknowledged for 1.20.2+ backends, and answers unsupported clients with a
  mismatch packet naming the accepted protocols. Translation routing is
  delegated to Multiversion via the loopback backend.
- [x] Add automated handshake and packet/state transition regressions for each
  admitted protocol, including malformed requests and unsupported versions.
  Unit tests exercise every protocol 755–777 with compression disabled and
  thresholds 1/256: backend handshake/Login Start injection, Login Acknowledged
  where applicable, profile-stage buffering and bidirectional packet relay.
  Malformed requests, excessive profiles, backend rejection/encryption and
  unsupported-version paths have regression coverage. These are simulated
  packet/state tests; graphical compatibility remains a separate milestone.
- [ ] Verify each candidate with a real graphical client: offline login, world
  and chunk loading, movement/teleports, chat, inventory, block breaking and
  placement, entity interaction, keepalives, disconnect and reconnect. Check
  skins and optional UI features only where advertised.
- [ ] Run mixed-client sessions and sustained tests alongside the existing
  26.3 probes; retain Gaius tunnel and vanilla TCP regression checks.
  Partial: the [recorded 2026-11-18 UTC run](../tests/results/2026-11-18-mixed-clients/summary.json)
  passed simultaneous simulated Eagler 755, Eagler 777, Gaius tunnel 777 and
  vanilla TCP 777 sessions through the existing gateway. All required login,
  teleport, chunks, chat echo and keepalive responses. Reusable probes and
  per-case evidence are in `tests/results/`. The subsequent
  [sustained run](../tests/results/2026-11-18-mixed-sustained/README.md) kept
  all four clients in play for 120 seconds each (seven keepalive replies per
  client) and passed same-identity reconnect checks for all four. A separate
  follow-up passed four MOTD queries and ten malformed-input attempts across
  six vectors. Real graphical clients, longer measurements and fuzzing during
  mixed play remain pending; the running translation plugin's exact
  binary/version was not re-established for these runs.
- [ ] Publish tested matrix entries and installation instructions before marking
  individual versions supported. Add subsequent 1.17+ protocols incrementally;
  do not label the whole version range supported based on one passing client.

## Implemented architecture

- Build `EaglerXPumpkin.wasm` for `wasm32-wasip2` with the pinned
  `pumpkin-plugin-api` 0.2.1.
- Poll a nonblocking listener and connection state machines from Pumpkin's
  scheduler. No guest threads or socket cloning are required.
- Listen on a separate WebSocket port (8081 by default) and connect only to the
  configured numeric loopback backend (127.0.0.1:25565 by default).
- `/tunnel` supports Gaius's JSON connect/connected and flow-control messages,
  carrying vanilla TCP bytes in binary WebSocket frames.
- `/` implements Eagler version negotiation and offline login. The bridge
  injects backend login packets, handles TCP packet lengths and compression,
  and delivers Minecraft packet bodies as WebSocket messages.
- Raw vanilla TCP connections on the gateway listener relay to the same backend.
- Keep partial reads, fragmented messages and pending writes across polls;
  enforce connection, buffer and handshake limits.
- Limit new connections per source address (default 30 per rolling minute),
  refuse overflow and the global cap immediately, and report peer addresses on
  failures.
- Log periodic diagnostics: active/playing counts, lifetime peaks,
  accepted/closed/refused/filtered/rate-limited totals and socket throughput
  per second.
- Cancel scheduled polling and close sockets on plugin unload.

The plugin host requires WASI Preview 2. `network.loopback` restricts binds as well
as connections, so the public listener does not request that permission.
The plugin itself validates that backend destinations are loopback addresses.

## Completion against the original plan

The original plan targeted full parity with the Java plugin. Checklist items are
marked done only for the 26.3 gateway; annotations record scope changes.

### Phase 0 - spike

- [x] WASM plugin skeleton with metadata, permissions and `on_load` lifecycle.
- [x] TCP listener and accept path through WASI sockets.
- [x] Hand-rolled WebSocket framing over nonblocking streams.
- [x] Loopback relay to `127.0.0.1:25565`, verified with the Gaius browser.
- [x] Idle/keepalive behaviour verified by the live probes. Per-connection async
  tasks were replaced by one scheduler-driven poll loop.
- [x] Record throughput/latency micro-benchmark numbers. Periodic counters now
  exist, and the frame codec micro-benchmark (`frame_codec_micro_benchmark` in
  `crates/eagler-ws`) now times masked-client decoding and server-reply encoding
  for payloads from 64 bytes through 256 KiB. The earlier 38-98 MiB/s and ~3 ms
  measurements timed decoding only, so they are not round-trip figures. See the
  [dev review](../rust/tests/results/dev-review/README.md) for corrected results;
  sustained-load performance characterization against a live backend remains open.

### Phase 1 - core connect and play

- [x] Configurable Eagler listener and HTTP upgrade detection.
- [x] Binary WebSocket frames, close/ping/pong and 64 KiB+ payload handling.
- [x] Eagler handshake v2–v5, profile exchange and offline UUID derivation.
- [x] Routing to the backend with vanilla handshake/login injection,
  compression and disconnect propagation.
- [x] TOML config in the plugin data folder.
- [x] Static MOTD and Eagler server-list ping.
- [x] Logging, connection limits and per-address rate limiting.
- [x] 26.3 browser join, movement, chat and block interaction.
- [ ] Original 1.8/1.12 client exit criterion. Only 26.3 is supported; older
  clients need a translation layer and their own validation.

### Phase 2 - full Eagler protocol layer

- [x] Skin pipeline: Eagler v5 `skin_v1`/`skin_v2`/`cape_v1` profile data,
  custom 64x64 uploads, deterministic Steve/Alex fallback, and an in-memory
  registry answering injected `GetOtherSkin`/`GetOtherCape`/`GetOtherTextures`
  requests. URL skin lookups are explicitly disabled instead of fetched, and the
  registry is bounded to online players so no persistent store is needed.
- [x] Adaptive packet batching (the original three-mode batcher). The
  applicable behaviour is already provided by the shared send-queue
  architecture: all frames queued during one poll are written back to back in
  as few nonblocking writes as the socket accepts, which is the same
  single-flush coalescing the Java batcher achieves. The timer modes (idle
  passthrough, 2 ms burst buffering, 200 ms forced flush) have no equivalent
  in the poll-driven runtime and no packet is held past the poll in which it
  was read. The reference keeps one packet body per binary WebSocket frame
  (see WebSocketEaglerFrameCodec: each buffered packet is written individually
  and wrapped 1:1), so the wire format is unchanged; merging several packet
  bodies into one frame would deviate from the reference and the live probe
  and needs a graphical Eagler client to validate before it could be
  considered.
- [x] Static pause-menu external link and join notification, advertised only
  when configured and supported by the client.
- [ ] Backend-driven notifications and message channels, including custom badge
  icons and per-event delivery.
- [x] MOTD animations. Text frames cycle on the query connection with the
  configured interval and are force-closed after the max age, matching
  EaglerMOTD's interval/close_socket_after mechanics and disabling the
  client-side cache while animated. Scoped port: per-frame server icons,
  sprite sheets, weighted message pools, named message chains and the
  cache-control query subtypes of the EaglerMOTD addon are not ported. Static
  server-list answers now announce the built-in EaglerXPumpkin server icon with
  a boolean and send its 64x64 RGBA pixels in a separate binary frame. `motd_icon`
  accepts a validated 64x64 PNG data URI override; image URLs are unsupported. The
  graphical client verified so far (Gaius) uses `/tunnel` and does not
  exercise the MOTD query path. Automated unit and real-socket integration
  tests cover text cycling and max-age closure; graphical MOTD verification
  remains pending.
- [x] IPv4/IPv6 allow and block lists with CIDR prefixes, evaluated before the
  connection rate limiter.
- [ ] Player whitelisting and UUID ban masks. Offline accounts make identity
  policy a backend concern; only address policy is enforced at the gateway.
- [ ] Voice chat (IVMP). Not planned for this plugin; a separate plugin will
  provide it.
- [ ] Password/account registration. Not applicable: Eaglercraft uses offline
  accounts, so the offline username model is the intended behaviour.

### Phase 3 - ecosystem addons

- [ ] EaglerWeb file hosting. Deferred; not needed for the first release.
- [ ] EaglerXRewind 1.5.2 support. Deferred separately from the planned 1.17+ work.
- [ ] Supervisor, telemetry, update checker and marketplace metadata. Not
  planned for this port.

### Testing strategy

- [x] Live simulated probes for Eagler v5, Gaius tunnel and raw vanilla TCP
  through the gateway, including a translated 1.17 Eagler play stream and
  same-identity reconnects. Recorded evidence lives in `tests/results/`.
- [x] Unit coverage for framing, compression, login ordering, limits and rate
  limiting, plus real-socket integration tests that drive the runtime poll
  loop end to end: HTTP upgrade, animated MOTD cycling, malformed-session
  resilience, connection rate limiting and listener health.
- [ ] Golden protocol captures replayed from Java-plugin traffic.
- [ ] Scripted browser end-to-end client.
- [ ] Vanilla Java client regression while the Eagler listener is active.
  Partial: simulated vanilla TCP 777 passed alongside Eagler/Gaius, sustained
  sessions and reconnects; a graphical Java client has not been verified.
- [ ] Sustained multi-client soak and fuzz run. The script
  rust/tests/soak_26_3.py is provided and was validated against the native
  gateway for its MOTD health-check and fuzz paths. A recorded 120-second
  four-client simulated run plus reconnects and a separate health/fuzz phase
  now exist in `tests/results/2026-11-18-mixed-sustained/`. A longer soak with
  simultaneous fuzzing and real graphical clients remains pending.
- [ ] Tracked Pumpkin-release × Eagler-client version matrix.

## Next work

- Expand client compatibility testing beyond the verified Gaius 26.3 browser
  and the Eagler v5 protocol probe.
- Run sustained-load measurements with the periodic counters and tune
  `connections_per_minute`, `max_connections` and the diagnostics interval for
  public deployments.
- Extend the recorded protocol smoke/sustained/reconnect checks with golden
  captures, a scripted browser join, a graphical vanilla Java client, and a
  longer mixed-client run with concurrent fuzzing. Keep exact component
  versions and binary hashes with the evidence.
- Port optional Eagler features as needed: notifications and pause menu
  extensions. Advertise capabilities only when their implementation is
  available.
- Deliver the [planned 1.17+ Eaglercraft compatibility work](#planned-eaglercraft-117-client-support)
  incrementally, relying on Multiversion and a tested client matrix.
  Pre-1.17 support remains deferred.

## Out of scope

- Eaglercraft voice chat (IVMP); the project owner will use a separate plugin.
- Password authentication and account registration. Eaglercraft clients use
  offline accounts; there is no login/register/change-password flow to port.
- Bukkit/Paper, BungeeCord and Velocity platform adapters, NMS reflection,
  Java stubs, Gradle builds and proxy/backend RPC modules.
- Legacy Java addons and their build artifacts. Reference code is available
  in Git history and in the upstream EaglerXServer project.
