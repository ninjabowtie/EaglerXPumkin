# EaglerXPumpkin

A Rust WebSocket gateway for **Pumpkin Minecraft 26.3**, distributed as a
WASI Preview 2 plugin. Includes a native CLI for development.

Tested with **Pumpkin 0.2.2+26.3-26.60** and **Gaius 26.3 portable
v0.4.0-c263**: world loading, movement, text chat, breaking and placing blocks
work in the browser. The gateway also implements the Eagler handshake transport.

## Companion plugin: Multiversion

Install [Multiversion](https://market.pumpkinmc.org/plugin/s7e5Nz2f-multiversion) alongside
EaglerXPumpkin for the intended multi-version server setup. EaglerXPumpkin
relies on Multiversion for Minecraft protocol translation when accepting
1.17+ client protocols. EaglerXPumpkin handles WebSocket/Eagler transport,
negotiation and login; Multiversion translates the Minecraft stream for
Pumpkin.

See the [companion setup](rust/README.md#multiversion-companion-setup) for routing
and configuration. The gateway now negotiates client protocols inside the
configured range, but no 1.17+ Eaglercraft client has been verified graphically
yet; per-client testing and matrix entries remain open. The existing
same-version gateway can still run on its own.

## Client support status

The gateway negotiates **Eaglercraft clients for Minecraft 1.17 and newer**
when `min_minecraft_protocol`/`max_minecraft_protocol` are configured: it picks
the highest client-offered protocol in the range and injects the
version-correct login packet. This does not yet mean every 1.17+ client works
end to end: compatibility depends on Multiversion translating the client's
protocol for Pumpkin, and each client build still needs graphical
verification before being called supported. The currently verified graphical
scope remains the 26.3 setup above. See the
[1.17+ compatibility plan](docs/PUMPKIN_PORT_PLAN.md#planned-eaglercraft-117-client-support)
for translation requirements, milestones and acceptance checks.

## Build

Install stable Rust and the WASI Preview 2 target, then run from the repository root:

```sh
rustup target add wasm32-wasip2
./rust/build.sh
```

Install `rust/dist/EaglerXPumpkin.wasm` in Pumpkin's `plugins/` folder.
See [setup and configuration](rust/README.md) for offline-mode settings,
permissions, connection addresses and verification commands.

For the tested Gaius client, connect to **127.0.0.1:25565** using an offline
profile. Gaius automatically uses the plugin's WebSocket listener on port 8081.

## Project layout

- `rust/crates/eagler-ws`: incremental WebSocket framing and HTTP upgrades.
- `rust/crates/eaglerx-pumpkin`: Pumpkin lifecycle, configuration, Eagler login,
  Minecraft compression, Gaius tunnel and shared networking runtime.
- `rust/tests`: live 26.3 protocol probes.
- [Roadmap](docs/PUMPKIN_PORT_PLAN.md): current scope and future work.

## Attribution

The Eagler handshake implementation is ported from
[lax1dude's EaglerXServer](https://github.com/lax1dude/eaglerxserver).
Original copyright and license notices are retained in [LICENSE](LICENSE) and
ported source files. The legacy Java/Paper implementation remains available in
this repository's Git history; it is no longer part of the active build.

## Manual releases

On GitHub, open **Actions → Manual release → Run workflow**, select `main`,
enter a version, and choose **stable**, **beta**, or **alpha**.

| Version input | Channel | Release tag |
| --- | --- | --- |
| `0.2.0` | stable | `v0.2.0` |
| `0.2.0` | beta | `v0.2.0-beta.1` |
| `0.2.0-alpha.2` | alpha | `v0.2.0-alpha.2` |

The workflow runs tests and builds the platform-independent WASM plugin.
Each release includes `EaglerXPumpkin.wasm`, a zip with setup instructions,
license and example configuration, and `SHA256SUMS`. Beta and alpha releases
are marked as prereleases; stable releases are marked as latest.

The selected version is embedded in the plugin during the build. Stable
releases commit and push a `chore(release): vX.Y.Z` version bump, then tag that
commit. Beta and alpha releases tag the already-built commit and never write to
`main`. Existing tags and releases are never overwritten, and a failed publish
removes the tag it created.
Releases only run manually; pushing commits does not publish one.

**Made with the help of artificial intelligence**
