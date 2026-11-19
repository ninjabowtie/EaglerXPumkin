# Local codebase sweep

Reviewed the Rust gateway, WebSocket codec, Minecraft framing/compression,
session negotiation, skin/UI/notification helpers, configuration/address policy,
plugin/native entrypoints, and supporting probes/release tooling against
`03eb276`. Fixes and validation evidence are included with this report. No installed server plugin,
config, world or service was replaced/restarted. Live plugin checks used an
isolated Pumpkin child server/world under `/tmp` and shut down normally.

## Fixes and optimizations

1. **Receive-buffer copying:** both decoders drained the front of a Vec after
   every packet, shifting all remaining packets. Coalesced batches incurred
   quadratic copying. Both now advance a cursor, clear fully consumed buffers,
   and compact partial tails only when new bytes are pushed. Minecraft compressed
   bodies are decoded directly from receive storage, avoiding an intermediate
   copy; uncompressed encoding avoids a redundant intermediate allocation.
2. **Send queues under backpressure:** appending after partial writes moved the
   entire unread suffix. Queues now use a ring buffer, retain FIFO ordering,
   and preserve existing size and per-flush I/O limits. Tests cover wrapped
   storage, partial/interrupted writes, appends and multi-poll budgets.
3. **Malformed compression:** empty packet bodies and uncompressed packets at
   compression threshold zero were accepted. Both are rejected. Outgoing
   compression-envelope sizes are now checked against the same wire-size bound
   enforced by decoding. Explicit zlib completion, checksum, output-size and
   trailing-data validation are retained without the intermediate body copy.
   Truncated/trailing zlib cases were already rejected in the baseline; the
   newly reproduced failures were empty packets, threshold zero and encode size.
4. **Disconnect encoding:** truncating encoded JSON at 65535 bytes could produce
   malformed JSON or invalid UTF-8. Error reasons are now bounded at a UTF-8
   boundary before logging/JSON serialization. Even worst-case escaping fits the
   protocol. The older short-string path is also truncated at a UTF-8 boundary.
5. **MOTD max age:** expiration waited for the next animation interval. With an
   interval of 100 ms and max age 250 ms, a frame at 200 ms delayed closure until
   300 ms. Absolute expiration is now checked independently of the frame timer.
6. **Idle timers:** successful writes at the end of a poll (including MOTD
   frames) did not refresh activity and could be followed by an immediate idle
   disconnect. Every successful flush now counts as activity.
7. **Tunnel closure:** a tunnel close control followed by more frames in the
   same read continued processing those frames, potentially forwarding data
   after closure or discarding the queued close response on an error. Processing
   now stops once closure begins for all transport branches.
8. **Configuration:** reject backend port zero, unsupported exact game protocols
   below 755, and static MOTDs over 2048 UTF-8 bytes (matching animation-frame
   limits). Explicit protocol ranges still override the unused exact field.
   These are intentional validation changes for previously accepted bad inputs.
9. **Skin allocation:** preset skin/cape responses reserved full custom-texture
   capacity (about 12 KiB / 1 KiB) for tiny replies. Allocation now follows actual
   response type; custom skin capacity also includes the largest request-ID header.
10. **Interrupted HTTP reads:** the blocking WebSocket handshake helper aborted
    on `Interrupted`, unlike the other I/O loops. It now retries.
11. **Native diagnostics:** the CLI never installed a tracing subscriber, hiding
    its existing periodic diagnostics and warnings. It now logs to stderr at
    info by default and honors `RUST_LOG` (invalid/missing filters fall back to
    info). A native-only dependency provides formatting/filtering; the WASI
    plugin still uses Pumpkin logging and its artifact hash is unaffected by
    this native-only addition.

## Before/after evidence

The new portable regression tests were copied into a temporary snapshot of the
original committed production modules. **Nine cases failed there**: interrupted
handshake reads, three compression/configuration-size cases, configuration
validation, final-flush activity, tunnel closure, long disconnect JSON and MOTD
max age. A separate native executable test also failed against the baseline
because periodic diagnostics never appeared. **All ten regressions pass with
the fixes.** The ring-buffer-specific representation test was omitted from the
old Vec implementation. See [baseline-regressions.log](baseline-regressions.log)
and [baseline-cli.log](baseline-cli.log).

Release-mode measurement of three batches of 10000 coalesced 64-byte payloads:

| Codec | Original | Updated |
| --- | ---: | ---: |
| WebSocket | 253.77 ms | 1.91 ms |
| Minecraft | 239.45 ms | 0.67 ms |

These are large pre-buffered batches and include correctness assertions. They
isolate quadratic buffer shifting; normal socket reads are smaller. **They are
not network-throughput, gameplay-latency or capacity claims.** The manual
benchmarks are ignored in the normal suite and have no timing assertions.
[Before](benchmark-before.log), [after](benchmark-after.log).

## Validation

- **82 Rust tests pass**, with two manual benchmarks ignored by default.
  [Rust log](rust-tests.log). All existing 755–777 protocol/compression regressions
  remain covered. Added cases exercise split/coalesced reads, wrapped/partial
  writes, malformed compression, UTF-8/JSON limits, timers, close ordering and
  a real 600000-byte raw TCP response after the client half-closes its request.
- **Both manual coalesced-buffer benchmarks pass** in release mode.
- **13 Python regressions pass**. [Python log](python-tests.log).
- **Native and WASI Clippy with warnings denied, formatting and whitespace checks
  pass.** [Native Clippy](clippy-native.log), [WASI Clippy](clippy-wasi.log).
- **Locked WASI release build passes.** [Build log](wasm-build.log).
- **Actual rebuilt WASI artifact passes isolated Pumpkin-host verification**:
  Eagler capability negotiation and configuration exchange, targeted notification,
  silent/highest-priority flags, dismissal, broadcast replacement and rejection
  of non-operator command execution. The checked artifact hash matches the final
  local `dist/EaglerXPumpkin.wasm`.
  [Host result](host-result.json), [host console](host-console.log).
- Tool versions, artifact/backend hashes and reproduction details are in
  [environment.json](environment.json).

## Reproduction and limits

From `rust/` with Rust tools on PATH and loopback sockets permitted:

```sh
cargo test --locked --workspace --target aarch64-unknown-linux-gnu
cargo clippy --locked --workspace --all-targets --target aarch64-unknown-linux-gnu -- -D warnings
cargo clippy --locked --workspace --lib --target wasm32-wasip2 -- -D warnings
cargo fmt --all -- --check
python3 -m unittest discover -s tests -p 'test_*.py'
./build.sh
cargo test --release --workspace --lib --target aarch64-unknown-linux-gnu coalesced_batch_benchmark -- --ignored --nocapture
python3 tests/probe_notifications.py --pumpkin-binary /path/to/pumpkin-server --output-dir /tmp/NEW-sweep-host
```

The first unit-only attempts without loopback permission failed on the socket
harness as expected; complete validation above ran with that access. No graphical
client, new long-duration mixed-client soak, native WASI-host profiler or full
coverage-guided fuzz campaign was run. Existing loopback backend connection setup
remains bounded synchronous I/O; this sweep does not introduce a new async socket
architecture or claim to eliminate all scheduler costs.
