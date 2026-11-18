# 1.17+ compatibility evidence

A negotiated protocol is not a supported graphical client. This matrix keeps
local client artifacts, simulated network probes and graphical acceptance
separate. Dates in the evidence files are UTC (the 2026-11-18 run occurred on
2026-11-17 in America/Los_Angeles).

## Client inventory

[clients.json](clients.json) records exact SHA-256 hashes and byte sizes of
available local HTML builds. The expected protocol comes from the advertised
Minecraft version; it has not been observed on the wire for these candidates.
Upstream provenance and Eagler handshake versions remain to be captured.
Do not redistribute the HTML files with this repository.

| Local build | Expected protocol | Transport | Current result |
| --- | --- | --- | --- |
| Eaglercraft 26.2 0.3-dev (`eagler26.2.html`) | 776 | Eagler `/`, unconfirmed | Pending graphical verification |
| Gaius 1.21.11 portable v0.4.0-pre6 | 774 | `/tunnel` | Pending graphical verification |
| Gaius 26.2 portable v0.4.0-pre6 | 776 | `/tunnel` | Pending graphical verification |
| Gaius 26.3 portable v0.4.0-c263 | 777 | `/tunnel` | Prior human play test documented in the project README; exact-file test date not recorded |

The current session could not reach a browser host. No new graphical result or
supported-version claim is inferred from these artifacts.

## Recorded network results

[2026-11-18 mixed run](../results/2026-11-18-mixed-clients/summary.json) ran four distinct offline
identities simultaneously against the existing local gateway on port 8081:

| Simulated client | Protocol | Result | Evidence |
| --- | --- | --- | --- |
| Eagler v5 1.17 | 755 | Pass | [JSON](../results/2026-11-18-mixed-clients/eagler-755.json) |
| Eagler v5 26.3 | 777 | Pass | [JSON](../results/2026-11-18-mixed-clients/eagler-777.json) |
| Gaius tunnel 26.3 | 777 | Pass | [JSON](../results/2026-11-18-mixed-clients/gaius-777.json) |
| Vanilla TCP through gateway | 777 | Pass | [JSON](../results/2026-11-18-mixed-clients/vanilla-777.json) |

Every case required login, teleport acknowledgement, chunk delivery, its own
chat marker echo and a keepalive response. The 26.3 cases also completed
configuration/registry exchange. This verifies concurrent simulated sessions;
it is not sustained load, a graphical Java client, inventory/block/entity
interaction, skins, or reconnect acceptance. Responding to a teleport or
keepalive demonstrates the client reply was sent, not a separate server-side
acknowledgement of that reply.

The existing test installation is documented as Pumpkin 0.2.2+26.3-26.60 with
Multiversion 0.6.1, routing `:8081 -> :25565 -> :25566`. The available startup
log identifies that Pumpkin release; these probes do not query the running
translation plugin's release, so its exact binary/version remains unconfirmed
for this new run. The running gateway was not rebuilt/reloaded for the probes.

Separate negotiation checks passed: offering `755 776 777` selected `777`,
offering `755 776` selected `776`, and offering `754` returned a mismatch
advertising `755–777`.

## Sustained sessions and reconnects

The [two-minute run](../results/2026-11-18-mixed-sustained/README.md)
passed four concurrent simulated clients (Eagler 755, Eagler 777, Gaius tunnel
777 and vanilla TCP 777). Each remained in play for 120 seconds and answered
seven keepalives. All four then reconnected with the same identities and
passed their smoke checks again. A separate follow-up health/fuzz phase passed
four MOTD queries and ten malformed-input attempts covering all six vectors.
Graphical acceptance, longer soak measurements and fuzzing during mixed play
remain pending.

## Repeat the checks

From `rust/`, with the documented Multiversion test installation running:

```sh
python3 tests/probe_compatibility.py --output-dir /tmp/compatibility-new-run
python3 tests/probe_1_17.py --handshake-only --protocol 777 --offer 755 776 777
python3 tests/probe_1_17.py --handshake-only --protocol 776 --offer 755 776
python3 tests/probe_1_17.py --expect-mismatch --offer 754
```

The runner writes per-case JSON and logs plus an aggregate pass/fail summary.
It exits nonzero if any case fails or exceeds its deadline, and requires a new
output directory to preserve earlier evidence. The 1.17 probe only implements
protocol 755 play packets; other versions are permitted for negotiation checks.
Use `probe_26_3.py --transport vanilla` for the raw TCP gateway regression.
Add `--sustain --seconds 120 --reconnect` to the mixed runner to repeat the
recorded sustained/reconnect checks. The [testing guide](../../docs/TESTING.md)
explains deadlines, backend-port assumptions, result formats and the separate
health/fuzz phase.

## Graphical acceptance checklist

For each exact artifact hash, record the test date, observed game protocol,
Eagler handshake version (or not applicable for tunnel), Pumpkin release,
translation binary/version, transport, and evidence for every check:

- Offline login, world rendering and chunk loading.
- Movement and teleports, chat, inventory, breaking and placing blocks.
- Entity interaction and keepalives over an extended session.
- Disconnect and reconnect with the same identity.
- Skin exchange and optional UI features where advertised.
- Concurrent older-client, 26.3 Eagler, Gaius and vanilla regression sessions.

Mark individual checks pass/fail/pending and retain failure reasons and logs.
A complete candidate result is required before publishing support for that build.
