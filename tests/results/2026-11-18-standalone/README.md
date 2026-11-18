# Standalone Pumpkin verification

The local server was switched from Multiversion routing to direct offline
Pumpkin on 127.0.0.1:25565, with the gateway on :8081 accepting only protocol
777. Multiversion.wasm is absent from the active plugins directory; proxy
forwarding is disabled. The former setup is backed up under
`/home/thomass/pumpkin-server/backups/standalone-20261118-024923/`.

See [summary.json](summary.json), per-transport JSON/logs and
[environment.json](environment.json) for results and binary hashes. Three
simulated protocol-777 clients ran concurrently (Eagler, Gaius tunnel and
vanilla TCP through the gateway); each required login, configuration, chunks,
teleport acknowledgement, its own chat echo and a keepalive reply. A protocol
755 offer was rejected with an exact-777 mismatch reply. Graphical play is
not established by these probes.

The test service remains running as `pumpkinmc-standalone-test.service` for
manual verification. Use the Gaius 26.3 build with an offline profile and
Direct Connect 127.0.0.1:25565, or a protocol-777 Eagler client using
ws://127.0.0.1:8081/. Older game protocols need translation and will be rejected.
