# Mixed-client smoke run

Pass: 2026-11-18 UTC (2026-11-17 America/Los_Angeles). Four simulated offline
identities concurrently passed login, teleport acknowledgement, chunk delivery,
their own unique chat marker echo and a keepalive reply. The 26.3 cases also
completed configuration with 32 registries.

| Case | Protocol | Result |
| --- | --- | --- |
| Eagler v5 | 755 | Pass |
| Eagler v5 | 777 | Pass |
| Gaius tunnel | 777 | Pass |
| Vanilla TCP through gateway | 777 | Pass |

See [summary.json](summary.json) and per-case JSON/logs. This run used the
existing gateway/translation/Pumpkin route; the gateway was not rebuilt and the
exact running translator release/binary was not re-established. It did not
verify graphical clients, sustained sessions, reconnects or concurrent fuzzing.
The [subsequent run](../2026-11-18-mixed-sustained/README.md) added sustained
sessions, reconnects and a separate follow-up listener-health/fuzz phase.

Repeat from the repository root with the documented test server running:

```sh
python3 rust/tests/probe_compatibility.py --output-dir tests/results/NEW-SMOKE-RUN
```

Use a new output directory and retain the exact environment and component
versions with future results. See the [testing guide](../../../docs/TESTING.md).
