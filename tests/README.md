# Test results and debugging

Store reproducible test evidence and debugging information here. Executable
Rust tests and live probe scripts remain in [`rust/tests`](../rust/tests).

## Layout

- [`results/`](results/): one folder per test run, containing JSON summaries,
  logs, screenshots and the configuration needed to reproduce the run.
- [`compatibility/`](compatibility/README.md): client build inventory, artifact
  hashes, version matrix and graphical acceptance checklist.
- [`captures/`](captures/): small packet captures or protocol traces, with notes
  describing the client, protocol, direction and capture method.
- [`debug/`](debug/): issue investigations, reproduction steps and failure notes.
- [`templates/run.md`](templates/run.md): copy into a run folder as `README.md`.

Use names such as `results/2026-11-18-mixed-clients/` or
`debug/2026-11-18-login-disconnect.md`. Include UTC timestamps, exact component
versions and client hashes when available. Record unknown values explicitly.
Keep failed runs as well as successful ones and link related issues or fixes.

## Record a compatibility run

With Pumpkin, Multiversion and the gateway running, from the repository root:

```sh
python3 rust/tests/probe_compatibility.py \
  --output-dir tests/results/YYYY-MM-DD-mixed-clients
```

For sustained sessions followed by reconnect checks:

```sh
python3 rust/tests/probe_compatibility.py --sustain --seconds 120 --reconnect \
  --output-dir tests/results/YYYY-MM-DD-mixed-sustained
```

See the [recorded sustained run](results/2026-11-18-mixed-sustained/README.md).
Choose a new folder name for each run. The runner saves a summary and per-client
JSON/log files. See the [recorded mixed-client run](results/2026-11-18-mixed-clients/summary.json)
and [compatibility notes](compatibility/README.md) for the current evidence and limits.

Commit useful, reviewed evidence. Remove credentials, forwarding secrets,
private chat and personal addresses before committing logs or captures. Keep
large raw dumps and downloaded client/server binaries outside Git; record their
location, size and SHA-256 hash in the run notes instead.
