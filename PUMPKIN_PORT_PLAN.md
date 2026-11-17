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

See [setup and verification](rust/README.md) for commands and tested scope.

## Implemented architecture

- Build `EaglerXPumkin.wasm` for `wasm32-wasip2` with the pinned Pumpkin v0.1 API.
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
- Log periodic diagnostics: active/playing counts, lifetime and interval peaks,
  accepted/closed/refused/rate-limited totals and socket throughput per second.
- Cancel scheduled polling and close sockets on plugin unload.

The v0.1 host requires WASI Preview 2. `network.loopback` restricts binds as well
as connections, so the public listener does not request that permission.
The plugin itself validates that backend destinations are loopback addresses.

## Completion against the original plan

The original plan targeted full parity with the Java plugin. Checklist items are
marked done only for the 26.3 gateway; annotations record scope changes.

### Phase 0 — spike

- [x] WASM plugin skeleton with metadata, permissions and `on_load` lifecycle.
- [x] TCP listener and accept path through WASI sockets.
- [x] Hand-rolled WebSocket framing over nonblocking streams.
- [x] Loopback relay to `127.0.0.1:25565`, verified with the Gaius browser.
- [x] Idle/keepalive behaviour verified by the live probes. Per-connection async
  tasks were replaced by one scheduler-driven poll loop.
- [ ] Record throughput/latency micro-benchmark numbers. Periodic counters now
  exist, but no benchmark run has been recorded.

### Phase 1 — core connect and play

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

### Phase 2 — full Eagler protocol layer

- [x] Skin pipeline: Eagler v5 `skin_v1`/`skin_v2`/`cape_v1` profile data,
  custom 64x64 uploads, deterministic Steve/Alex fallback, and an in-memory
  registry answering injected `GetOtherSkin`/`GetOtherCape`/`GetOtherTextures`
  requests. URL skin lookups are explicitly disabled instead of fetched, and the
  registry is bounded to online players so no persistent store is needed.
- [ ] Adaptive packet batching (the original three-mode batcher).
- [ ] Pause menu extensions, notifications and message channels.
- [ ] MOTD animations.
- [ ] IP filtering, whitelisting hooks and ban masks.
- [ ] Voice chat (IVMP). Not planned for this plugin; a separate plugin will
  provide it.
- [ ] Password/account registration. Not applicable: Eaglercraft uses offline
  accounts, so the offline username model is the intended behaviour.

### Phase 3 — ecosystem addons

- [ ] EaglerWeb file hosting. Deferred; not needed for the first release.
- [ ] EaglerXRewind 1.5.2 support. Deferred with older-client support.
- [ ] Supervisor, telemetry, update checker and marketplace metadata. Not
  planned for this port.

### Testing strategy

- [x] Live probes for both transports (Eagler v5 handshake and Gaius tunnel).
- [x] Unit coverage for framing, compression, login ordering, limits and rate
  limiting.
- [ ] Golden protocol captures replayed from Java-plugin traffic.
- [ ] Scripted browser end-to-end client.
- [ ] Vanilla Java client regression while the Eagler listener is active.
- [ ] Sustained multi-client soak and fuzz run.
- [ ] Tracked Pumpkin-release × Eagler-client version matrix.

## Next work

- Expand client compatibility testing beyond the verified Gaius 26.3 browser
  and the Eagler v5 protocol probe.
- Run sustained-load measurements with the periodic counters and tune
  `connections_per_minute`, `max_connections` and the diagnostics interval for
  public deployments.
- Add automated coverage for the untested paths above: golden captures, a
  scripted browser join, a vanilla-client regression and a multi-client soak.
- Port optional Eagler features as needed: notifications and pause menu
  extensions. Advertise capabilities only when their implementation is
  available.
- Evaluate older Minecraft client support separately. Pumpkin's old version
  enum variants do not provide translation; older clients need a translation
  layer and their own end-to-end validation.

## Out of scope

- Eaglercraft voice chat (IVMP); the project owner will use a separate plugin.
- Password authentication and account registration. Eaglercraft clients use
  offline accounts; there is no login/register/change-password flow to port.
- Bukkit/Paper, BungeeCord and Velocity platform adapters, NMS reflection,
  Java stubs, Gradle builds and proxy/backend RPC modules.
- Legacy Java addons and their build artifacts. Reference code is available
  in Git history and in the upstream EaglerXServer project.
