# Pumpkin port roadmap

## Current target

A Rust WASM plugin for stock **Pumpkin Minecraft 26.3** (protocol **777**).
The native development CLI shares the plugin's networking runtime.

Verified on Pumpkin 0.2.2+26.3-26.60 with Gaius 26.3 portable v0.4.0-c263.
The browser play test confirmed world loading, movement, chat, block breaking
and placement. Both live transport probes also completed configuration,
chunk streaming, returned chat and keepalive exchanges.

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
- Cancel scheduled polling and close sockets on plugin unload.

The v0.1 host requires WASI Preview 2. `network.loopback` restricts binds as well
as connections, so the public listener does not request that permission.
The plugin itself validates that backend destinations are loopback addresses.

## Next work

- Expand client compatibility testing beyond the verified Gaius 26.3 browser
  and the Eagler v5 protocol probe.
- Improve diagnostics, connection rate limiting and sustained-load measurements.
- Port selected Eagler features as needed: skins, password authentication,
  notifications and pause menu extensions. Advertise capabilities only when
  their implementation is available.
- Evaluate older Minecraft client support separately. Pumpkin's old version
  enum variants do not provide translation; older clients need a translation
  layer and their own end-to-end validation.

## Out of scope

- Eaglercraft voice chat (IVMP); the project owner will use a separate plugin.
- Bukkit/Paper, BungeeCord and Velocity platform adapters, NMS reflection,
  Java stubs, Gradle builds and proxy/backend RPC modules.
- Legacy Java addons and their build artifacts. Reference code is available
  in Git history and in the upstream EaglerXServer project.
