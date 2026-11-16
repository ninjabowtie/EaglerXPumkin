# EaglerXPumkin → Pumpkin (Rust) Port Plan

> Implementation update (2026-11-16): the first working 26.3 gateway is in
> `rust/`; see `rust/README.md` for verified setup and scope. Stock Pumpkin
> 0.2.2+26.3-26.60 loaded the WASI P2 plugin successfully. Gaius 26.3 browser
> play was confirmed by the user (movement, chat, breaking/placing blocks).
> Important corrections to the original design below:
> - The pinned `pumpkin-plugin-api` crate exports ABI v0.1, not async v0.2.
>   It requires `wasm32-wasip2`; the implementation uses scheduler polling,
>   without guest threads.
> - `network.loopback` restricts all socket operations to loopback, including
>   binds. It is not requested for the public listener. Backend destinations
>   are restricted to loopback by the plugin config validator instead.
> - Eagler messages carry packet bodies; backend TCP lengths and compression
>   must be handled by the bridge. They cannot simply be copied unchanged.
>   Gaius uses a separate `/tunnel` protocol carrying the full TCP stream.
> - Stock Pumpkin currently accepts protocol 777. Older enum variants do not
>   establish support for older clients; those need a translation plugin.
> Feature parity excluding Eaglercraft voice remains the longer-term plan; this
> first release targets 26.3. Voice chat will be handled by a separate plugin,
> per the project owner’s direction (2026-11-16).


**Goal:** Re-implement this plugin so Eaglercraft clients can join **stock Pumpkin servers** (https://pumpkinmc.org/), with feature parity with the Java plugin as it works today on Paper, except for Eaglercraft voice chat, which is out of scope and will be handled by a separate plugin.

- **Target runtime:** unmodified Pumpkin server builds (which load **WASM plugins only** — verified in `crates/pumpkin/src/main.rs`, only `WasmPluginLoader` is registered).
- **Deliverable:** a Rust WASM plugin (`EaglerXPumkin.wasm`) plus a Rust workspace for the protocol implementation, packaged like any Pumpkin plugin and installable by dropping it in `plugins/` and accepting its permission prompts.
- **License:** lax1dude's permissive BSD-style license + Pumpkin's MIT/Apache plugin API. Keep attribution headers in ported files.

---

## 1. Verified constraints from the Pumpkin source

These facts were verified against the Pumpkin repository (master, Nov 2026) and drive the architecture:

| # | Fact | Source |
|---|------|--------|
| 1 | Release Pumpkin registers **only** `WasmPluginLoader`. Native `.so` plugins exist in core but are not wired into the shipped binary. | `crates/pumpkin/src/main.rs:169` |
| 2 | The plugin API has **no hook before the VarInt codec**. Eaglercraft's HTTP/WebSocket upgrade bytes would be discarded as a malformed Minecraft packet before any event fires. | `pumpkin-protocol` codec pipeline, `PacketReceivedEvent` docs |
| 3 | WASM plugins **can bind TCP listeners** via `wasi:sockets`, gated by permissions. | `crates/pumpkin-wasm-host/src/runtime.rs` → `socket_policy_for_permissions()`, `WasiCtxBuilder::allow_tcp()` |
| 4 | Permissions exist for: `network.tcp.bind`, `network.tcp.connect`, `network.loopback`, `network.udp.*`, `network.outbound`, `http.outbound`, `fs.read.data`, `fs.write.data`. | `crates/pumpkin-core/src/plugin/permissions.rs` |
| 5 | Plugins get a private data folder (`plugins/data/<name>`) for config + storage. | `pumpkin-plugin-api` docs |
| 6 | The async plugin ABI (`pumpkin:plugin@0.2.0`) supports long-running async tasks. | PR #3713 (async v0.2 plugin ABI) |
| 7 | Server status (MOTD) is normally served by Pumpkin's own listener; Eagler clients ping over the Eagler protocol instead, so the plugin answers those itself. | EaglerXServer behavior |

**Consequence:** the plugin cannot share port 25565 (dual-stack) the way the Java plugin does on Paper. Eaglercraft gets its **own port** (default **8081**), and the plugin forwards the resulting vanilla Minecraft byte stream over **loopback TCP** to `127.0.0.1:25565`, so Pumpkin sees a completely ordinary Java client. All MC-layer negotiation (compression, keepalives, plugin channels) happens transparently between Pumpkin and the Eagler client through the relay — the plugin only parses the Eagler layer plus the initial MC handshake packet.

---

## 2. Architecture

```
Eaglercraft browser client
        │  ws://host:8081  (wss:// via reverse proxy, same as today)
        ▼
┌──────────────────────────────────────────────────────┐
│  EaglerXPumkin.wasm  (Pumpkin WASM plugin)           │
│                                                      │
│  ┌─ Eagler listener (wasi:sockets TCP bind :8081) ─┐ │
│  │  • first-byte sniff: HTTP/WebSocket upgrade     │ │
│  │  • WebSocket framing (hand-rolled over WASI     │ │
│  │    streams; binary frames)                      │ │
│  │  • Eagler handshake v2–v5, auth, profile        │ │
│  │  • skins (cache + Mojang fetch via http)        │ │
│  │  • MOTD / server-list ping                      │ │
│  │  • adaptive packet batching                     │ │
│  └──────────────────┬──────────────────────────────┘ │
│                     │ vanilla MC byte stream         │
│                     │ (loopback TCP connect)         │
└─────────────────────┼────────────────────────────────┘
                      ▼
        127.0.0.1:25565 — Pumpkin's normal Java listener
        (offline-mode; sees a plain Java client)
```

### Key design decisions

1. **Embedded proxy, zero Pumpkin modifications.** The plugin never touches Pumpkin internals — it only uses WASI (sockets, filesystem, HTTP) and the plugin lifecycle. This means it works on every stock Pumpkin build and survives Pumpkin's internal churn.
2. **Relay, don't translate.** After the Eagler handshake completes, the client's Minecraft bytes are relayed as-is. The plugin parses only:
   - the client's initial MC handshake packet (to forward, rewriting the address field as configured for forwarding compatibility),
   - `SLoginStart`-adjacent bytes if needed to synchronize Eagler profile data.
   Everything else is opaque bytes in → bytes out.
3. **Handshake ownership.** The plugin performs the vanilla handshake + login-start *on behalf of* the Eagler client when the client's stream begins (exactly like EaglerXServer does at the Bungee/Velocity layer), then switches to pure relay.
4. **Everything Eagler-specific lives in the Eagler layer.** Skins, MOTD, pause menu, notifications are Eagler-protocol constructs between the plugin and the browser client; Pumpkin never sees them.

---

## 3. Scope: Java feature parity excluding Eaglercraft voice

Every feature the Java plugin provides on Paper is in scope, mapped to its new home:

### Phase 0 — Spike / feasibility (1–2 weeks)

> Goal: retire the two riskiest unknowns before committing to the full build.

- [ ] WASM plugin skeleton: `register_plugin!`, metadata, permissions requested, `on_load` lifecycle.
- [ ] Bind a TCP port via `wasi:sockets` from inside the plugin; accept a connection; echo bytes.
- [ ] Verify long-running per-connection async tasks (v0.2 ABI) and idle keep-alive behavior.
- [ ] Bare WebSocket echo (hand-rolled framing over WASI streams).
- [ ] Loopback TCP relay to 127.0.0.1:25565; connect a real vanilla client **through the plugin** and confirm join/play works end-to-end.
- [ ] Micro-benchmark throughput/latency through the WASM boundary (chunk streaming worst case).
- **Kill criteria:** if per-connection relay throughput can't stream a 12-chunk radius login comfortably, re-evaluate (fallback: native loader patch or standalone sidecar — see §7).

### Phase 1 — Core: connect and play (4–8 weeks)

- [ ] Eagler listener on configurable port (default 8081), HTTP upgrade detection.
- [ ] WebSocket framing: binary frames, close/ping/pong, 64k+ payload handling.
- [ ] Eaglercraft handshake: protocol versions v2–v5, brand packets, profile data, UUID derivation.
- [ ] Connection routing: Eagler → loopback TCP → Pumpkin; vanilla handshake/login-start injection.
- [ ] Disconnect propagation both directions (MC kick ↔ WS close + Eagler kick packet).
- [ ] Config file (`eagler.toml`) in the plugin data folder, mirroring the Java plugin's `eaglercraftx` settings where they still apply (port, MOTD, protocol version floor, forwarding address...).
- [ ] Static MOTD + server-list ping (Eagler ping path).
- [ ] Logging, connection limits, basic ratelimiting.
- **Exit criteria:** Eaglercraft 1.8/1.12+ browser clients join a stock offline-mode Pumpkin server, move, chat, break/place blocks.

### Phase 2 — Full Eagler protocol layer (4–8 weeks)

- [ ] **Skins:** full skin pipeline — profile skin packets, custom-skin uploads, SQLite skin cache → ported as a binary file store in the data folder (SQLite itself can't run in WASM; `redb`-style or custom append-only store), **pre-warming**: at plugin start, fetch recently-seen players' skins from Mojang sessionserver via `http.outbound`.
- **Out of scope — Eaglercraft voice:** do not port IVMP channels or voice signaling. Voice chat will use a separate plugin chosen by the project owner.
- [ ] **Password auth:** Eagler authentication handler (offline-mode account login/register/change-password flow) with hash storage in the data folder.
- [ ] **Adaptive packet batching:** port the 3-mode batcher (idle passthrough / ~2 ms burst buffering / 200 ms forced-flush timer) — critical here because every packet crosses the WASM boundary twice.
- [ ] **Pause menu / notifications / message channels:** Eagler protocol features (custom icons, update notifications, websocket keepalive, C17-style messages).
- [ ] **MOTD animations** (eaglermotd feature set) if configs exist.
- [ ] IP filtering, whitelisting hooks, connection ratelimits, ban masks (ported from core `base/` utilities).

### Phase 3 — Ecosystem addons (open-ended, same order as the Java repo)

- [ ] **EaglerWeb:** HTTP file hosting on an auxiliary port (or HTTP-only multiplexing on the Eagler port) — static site + template support.
- [ ] **EaglerXRewind:** 1.5.2 client support (protocol translation layer, port of `rewind_v1_5`).
- [ ] **Supervisor:** multi-server supervision/IPC — **only if** needed for proxy-style deployments; single-server installs don't use it.
- [ ] Telemetry, update checker, marketplace metadata.

### Explicitly dropped (no Pumpkin equivalent needed)

- `api-bukkit`, `api-bungee`, `api-velocity`, NMS reflection (`NmsNames`), stubs — Java platform glue.
- `backend-rpc-*` — proxy↔backend RPC machinery; the plugin *is* the endpoint now.
- ViaVersion/ViaBackwards dependency — Pumpkin handles multi-version Java clients natively (`JavaMinecraftVersion`).

---

## 4. Repository layout (proposed)

```
EaglerXPumkin/
├── PUMPKIN_PORT_PLAN.md          ← this file
└── rust/                         ← new Cargo workspace (Java tree stays until parity)
    ├── Cargo.toml
    ├── crates/
    │   ├── eagler-protocol/      # Eagler wire protocol: packets, serde, versions v2–v5
    │   ├── eagler-skins/         # skin store, Mojang fetch, cache
    │   ├── eagler-ws/            # WebSocket framing over WASI streams
    │   ├── eagler-config/        # TOML config
    │   └── eaglerx-pumkin/       # the plugin itself: listener, relay, lifecycle
    └── wasm/                     # build scripts, wit bindings, CI
```

- Target: `wasm32-wasip2` (matches Pumpkin's wasmtime component setup).
- WASI usage: `wasi:sockets` (TCP bind/connect, loopback), `wasi:io` streams, `wasi:filesystem` (data folder), `wasi:http` (Mojang skin API, update checks).
- Keep a per-commit CI job that builds against the current Pumpkin `pumpkin-plugin-api` release (`PLUGIN_API_VERSION` bumps require a rebuild — automate the version pin).

---

## 5. Risks and mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| `wasi:sockets` throughput across the WASM boundary | Chunk streaming latency | Phase 0 micro-benchmark; adaptive batching (already planned) cuts crossing count; kill criteria in Phase 0 |
| Hand-rolled WebSocket over WASI streams | Protocol bugs | Conformance tests against real Eaglercraft client builds + vanilla WS test suites |
| Pumpkin plugin API churn (`PLUGIN_API_VERSION = 6` today) | Rebuilds per release | CI matrix on Pumpkin releases; the plugin uses almost no server-facing API (lifecycle only), so breakage risk is minimal |
| Online-mode Pumpkin servers | Eaglercraft cannot perform Mojang auth | Document: offline mode required; Phase 2 password auth covers identity; optionally support forward-compat with future Pumpkin auth hooks |
| Port change (8081 vs dual-stack 25565) | Setup difference vs Java plugin | Document reverse-proxy setups (Caddy/nginx/Cloudflare/playit.gg) — unchanged from Java deployment |
| SQLite unavailable in WASM | Skin cache rewrite | Custom binary cache format; format compatible with `skin-cache` where practical, with a one-time importer |

---

## 6. Testing strategy

1. **Protocol conformance:** replay captures from the Java plugin's traffic (Eagler handshake and skin packets) as golden tests for `eagler-protocol`.
2. **End-to-end:** scripted Eaglercraft browser client (puppeteer against a local Eaglercraft build) joining a Pumpkin server via the plugin; assert join, movement sync, chat, skin display.
3. **Vanilla regression:** normal Java client joins on 25565 while the Eagler listener is active — zero interference.
4. **Soak:** 50 fake Eagler clients (protocol fuzzer from Phase 1 captures) for memory/leak checks; packet limiter behavior.
5. **Version matrix:** Pumpkin release builds × Eagler client builds tracked in CI notes.

---

## 7. Fallbacks (if Phase 0 spike fails)

- **Fallback A:** upstream a `NativePluginLoader` registration PR to Pumpkin; ship the same code as a native `.so` plugin (no WASM sandbox → tokio directly, no boundary cost).
- **Fallback B:** ship the identical Rust code as a standalone sidecar binary next to Pumpkin (same architecture, out-of-process) — architecturally identical, trivial to produce because the plugin already treats Pumpkin as an opaque TCP endpoint.

Both fallbacks reuse ≥95% of the ported code; the Phase 0 spike chooses the lane.

---

## 8. Rough effort

| Phase | Duration (solo) |
|-------|-----------------|
| 0 Spike | 1–2 weeks |
| 1 Core connect-and-play | 4–8 weeks |
| 2 Eagler layer (skins, auth, batching) | 4–8 weeks |
| 3 Addons (eaglerweb, rewind, supervisor) | open-ended |

Total relevant Java being ported: ~25k lines (from this repo) → estimated ~12–18k lines of Rust.

---

## 9. Immediate next actions

1. Create the `rust/` workspace and Phase 0 spike crate.
2. Build against the current Pumpkin release; confirm permission prompts for `network.tcp.bind` + `network.loopback` + `fs.write.data`.
3. Run the Phase 0 checklist; record benchmark numbers in this file.
4. Proceed to Phase 1 with the protocol crate, porting from `protocol-game` and `core/src/main/java/net/lax1dude/eaglercraft/backend/server/base/{pipeline,handshake}` as the reference implementation.
