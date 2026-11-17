# EaglerXPumkin

A Rust WebSocket gateway for **Pumpkin Minecraft 26.3**, distributed as a
WASI Preview 2 plugin. Includes a native CLI for development.

Tested with **Pumpkin 0.2.2+26.3-26.60** and **Gaius 26.3 portable
v0.4.0-c263**: world loading, movement, text chat, breaking and placing blocks
work in the browser. The gateway also implements the Eagler handshake transport.

## Build

Install stable Rust and the WASI Preview 2 target, then run from the repository root:

```sh
rustup target add wasm32-wasip2
./rust/build.sh
```

Install `rust/dist/EaglerXPumkin.wasm` in Pumpkin's `plugins/` folder.
See [setup and configuration](rust/README.md) for offline-mode settings,
permissions, connection addresses and verification commands.

For the tested Gaius client, connect to **127.0.0.1:25565** using an offline
profile. Gaius automatically uses the plugin's WebSocket listener on port 8081.

## Project layout

- `rust/crates/eagler-ws`: incremental WebSocket framing and HTTP upgrades.
- `rust/crates/eaglerx-pumkin`: Pumpkin lifecycle, configuration, Eagler login,
  Minecraft compression, Gaius tunnel and shared networking runtime.
- `rust/tests`: live 26.3 protocol probes.
- [Roadmap](PUMPKIN_PORT_PLAN.md): current scope and future work.

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
Each release includes `EaglerXPumkin.wasm`, a zip with setup instructions,
license and example configuration, and `SHA256SUMS`. Beta and alpha releases
are marked as prereleases; stable releases are marked as latest.

The selected version is embedded in the plugin during the build. The workflow
creates a tag for the selected main commit without committing version bumps
back to `main`. Existing tags are never overwritten. If publishing fails after
creating a draft, its uploaded assets remain in that draft for inspection;
remove the incomplete draft/tag before retrying the same version.
Releases only run manually; pushing commits does not publish one.
