# Sustained mixed-client run and reconnects

Pass: 2026-11-18 UTC (2026-11-17 America/Los_Angeles). Four simulated clients
ran concurrently for 120 seconds each, then disconnected and reconnected with
the same identities. All eight cases passed login, teleport acknowledgement,
chunk delivery, their own chat marker echo and keepalive replies.

| Case | Play duration | Keepalive replies | Chunks |
| --- | --- | --- | --- |
| Eagler v5 / 755 | 120.000 s | 7 | 118 |
| Eagler v5 / 777 | 120.001 s | 7 | 117 |
| Gaius tunnel / 777 | 120.001 s | 7 | 117 |
| Vanilla TCP through gateway / 777 | 120.000 s | 7 | 117 |

Every reconnect completed its smoke checks in approximately 15 seconds and
answered another keepalive. The 26.3 cases received 32 configuration registries.
See [summary.json](summary.json), per-case JSON/logs and
[environment.json](environment.json) for source hashes and setup limits.

## Reproduction

From the repository root, with the existing offline Pumpkin/Multiversion setup:

```sh
python3 rust/tests/probe_compatibility.py --sustain --seconds 120 --reconnect \
  --output-dir tests/results/NEW-RUN-NAME
python3 rust/tests/soak_26_3.py --clients 0 --fuzz-clients 1 --seconds 20 \
  --motd-interval 5 --json-output tests/results/NEW-RUN-NAME/listener-health.json
```

The separate follow-up listener check passed four MOTD queries and ten
malformed-input attempts spanning all six vectors; see
[listener-health.json](listener-health.json) and [log](listener-health.log).
The soak helper was corrected before this phase: a socket timeout no longer
counts as a successful peer closure, connection failures no longer count as
successful fuzz rejections, and an MOTD response must contain a text frame.

## Limits and follow-up

These are synthetic clients against the existing running gateway. Browser
access was unavailable. This run does not verify graphical world rendering,
inventory, block/entity interaction, skins or optional UI. It is not a capacity
benchmark or a long-duration production soak. The malformed-input check ran
after the sustained sessions, not concurrently. The translator's exact running
release/binary remains unknown. Keep graphical and exact-build support
milestones open until those checks are recorded.
