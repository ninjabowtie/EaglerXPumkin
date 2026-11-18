# Testing and debugging

Run the commands below from the repository root unless a command explicitly
changes directory. Executable tests and probe tools live in `rust/`; reviewed
results, logs, client inventories and debugging notes live in [`tests/`](../tests/README.md).

## Local checks without Pumpkin

The workspace defaults to `wasm32-wasip2`. Use the native host target to execute
Rust tests, including the integration tests that bind local loopback sockets:

```sh
cd rust
cargo fmt --all -- --check
cargo test --workspace --target "$(rustc -vV | sed -n 's/^host: //p')"
cargo clippy --workspace --all-targets \
  --target "$(rustc -vV | sed -n 's/^host: //p')" -- -D warnings
cd ..
python3 -m unittest discover -s rust/tests -p 'test_*.py'
```

The last pre-push validation passed 50 Rust tests and 13 Python tests, plus
formatting and Clippy, on the native aarch64 Linux host. Rust tests cover
WebSocket/TCP framing, compression, configuration, runtime limits and protocol
session behavior. Session regressions exercise every game protocol 755–777
with compression disabled and thresholds of 1 and 256 bytes: handshake/Login
Start injection, Login Acknowledged for 764+, profile-stage buffering and
bidirectional relay. Malformed requests, backend login failures and unsupported
versions are covered separately. These simulated packets do not establish
real-client gameplay compatibility for every admitted protocol.

Python tests cover release-input handling, deadline behavior, and rejection
checks that distinguish a peer close from a timeout. They need no running
server. The manual release workflow currently runs `test_release.py` and the
Rust checks; it does not launch the live probes or the full Python test set.

## Live test setup

Start the offline Pumpkin/gateway installation described in the
[Multiversion setup](../rust/README.md#multiversion-companion-setup). The recorded
mixed-version route is gateway `127.0.0.1:8081` → translation listener `:25565`
→ Pumpkin `:25566`, with client protocols 755–777 enabled in the gateway.
The 1.17 probe requires translation; standalone exact-777 mode cannot run it.

The probes default to `--host 127.0.0.1 --port 8081`. Gaius tunnel control and
vanilla login handshakes in the current probe tools use backend port 25565;
changing the gateway port does not change that backend-port assumption.
Use distinct `--name` values if launching individual probes concurrently.

## Individual protocol probes

```sh
python3 rust/tests/probe_1_17.py --seconds 30
python3 rust/tests/probe_26_3.py --transport eagler
python3 rust/tests/probe_26_3.py --transport gaius
python3 rust/tests/probe_26_3.py --transport vanilla
```

The 1.17 probe implements protocol 755 play packets only. The 26.3 probe uses
protocol 777 and supports Eagler packet bodies, Gaius `/tunnel` streams and
vanilla TCP through the gateway. The vanilla mode is a simulated TCP client,
not a launched graphical Java client.

Each probe requires login, teleport acknowledgement, chunks, its own unique
chat marker echo and a keepalive reply. The 26.3 modes also complete
configuration/registry exchange. `--json-output PATH` saves a successful play
result; the parent directory must exist. Failure details go to the process
output and the exit status is nonzero. These checks verify that client replies
were sent; they do not inspect a separate server acknowledgement of each reply.

Negotiation checks against the 755–777 configuration:

```sh
python3 rust/tests/probe_1_17.py --handshake-only --protocol 777 --offer 755 776 777
python3 rust/tests/probe_1_17.py --handshake-only --protocol 776 --offer 755 776
python3 rust/tests/probe_1_17.py --expect-mismatch --offer 754
```

`--protocol` specifies the expected selection; the probe fails on a different
selection. `--expect-mismatch` requires a version-mismatch response and stops
before login. Non-755 protocols are allowed for these negotiation-only checks.

## Mixed sessions, sustained play and reconnects

```sh
python3 rust/tests/probe_compatibility.py \
  --output-dir tests/results/NEW-SMOKE-RUN
python3 rust/tests/probe_compatibility.py --sustain --seconds 120 --reconnect \
  --output-dir tests/results/NEW-SUSTAINED-RUN
```

Choose a new directory for each invocation; the runner refuses an existing
output directory. It starts four distinct offline identities concurrently:
Eagler 755, Eagler 777, Gaius tunnel 777 and vanilla TCP 777. It saves per-case
JSON/log files and `summary.json`, and exits nonzero on any failed case or
process deadline. `--seconds` bounds each play phase after login.

Without `--sustain`, clients finish when all smoke assertions pass. With it,
they continue receiving packets and replying to keepalives for the full play
duration. `--reconnect` waits until all first sessions close, then repeats smoke
checks with the same identities and a 30-second play budget per case. Result
JSON includes measured play duration, packet counts and keepalive replies.

## Listener health and malformed input

After creating a result directory, record a separate listener check:

```sh
python3 rust/tests/soak_26_3.py --clients 0 --fuzz-clients 1 --seconds 20 \
  --motd-interval 5 --json-output tests/results/NEW-SUSTAINED-RUN/listener-health.json
```

The legacy soak tool also supports `--clients N` for concurrent simulated 26.3
Eagler/Gaius players. Its fuzz workers cycle through six malformed-input vectors;
inspect the `fuzz_vector_0` through `fuzz_vector_5` counters to confirm coverage.
A silent socket timeout is a failed peer-close check, and an MOTD query must
receive a text frame. Connection failures are not successful fuzz rejections.
It prints periodic/final counters and saves failures as well as successes when
`--json-output` is supplied. Its output directory must already exist.

The recorded listener phase ran after the sustained sessions, not during them.
The legacy tool's combined game-client/fuzz mode has not been validated by the
new recorded run. MOTD queries are observed for up to five seconds, so use the
static MOTD or an animation close age within that window for this health check.
Allow for the gateway's per-address rate limit when increasing fuzz/query load;
refused connections must not be mistaken for malformed-packet handling.

## Evidence and outstanding checks

- [Client inventory and compatibility matrix](../tests/compatibility/README.md).
- [Mixed smoke results](../tests/results/2026-11-18-mixed-clients/README.md).
- [120-second sessions, reconnects and follow-up health/fuzz results](../tests/results/2026-11-18-mixed-sustained/README.md).
- [Run template](../tests/templates/run.md) for exact versions, source/binary
  hashes, configuration, commands, timestamps, failures and limitations.

All four sustained cases passed 120 seconds and seven keepalive replies each;
all four reconnects passed. The follow-up passed four MOTD queries and ten
malformed attempts across all six vectors. The running gateway was not rebuilt
for these probes, and the exact running translator release/binary was not
re-established. Preserve those limits when citing the results.

Graphical verification of older clients remains open: browser access was
unavailable during these runs. World rendering, inventory, block/entity
interaction, skins and optional UI are not established by synthetic probes.
Longer measurements, fuzzing during mixed play, golden captures and a real
Java-client regression remain on the [roadmap](PUMPKIN_PORT_PLAN.md).
