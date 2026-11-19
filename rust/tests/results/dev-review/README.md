# Dev review and regression testing

Reviewed `dev` at `b74eb991fbb3e44e990eebe8311c06d3b0ee9487` against
`origin/main` at `c2d902241f231c5c2915a85441eba2aae046c58b` on 2026-11-18
America/Los_Angeles (2026-11-19 UTC). Remote branch tips were checked and matched.
The initial worktree was clean. Fixes and evidence are included with this report.

## Findings fixed

1. **High: static MOTD icons used an incompatible wire format.** The new code
   placed a PNG URL/data URI in `data.icon`, including the entire ~998 KB project
   PNG as ~1.3 MB of base64. Eagler expects a boolean and a following binary
   WebSocket message containing 16384 bytes (64x64 RGBA). Existing tests asserted
   the incorrect response. Replacing that assertion with the protocol contract
   reproduced the failure before the fix. The gateway now sends JSON, raw icon
   pixels, then the close frame. The default logo is resized at development time,
   embedded from a crate-local asset and decoded once. No-icon queries suppress
   the binary frame. This also removes the build dependency on an image outside
   the Rust workspace/crate.
2. **Medium: icon overrides accepted unusable values.** The former validation
   accepted arbitrary ASCII, invalid images and URLs, with no server-side image
   fetching or decoding. Overrides now accept a valid non-animated 64x64 PNG
   data URI, validate base64/PNG content at startup and convert to RGBA. URLs are
   rejected with an actionable error; this is an intentional config change from
   the unworking URL setting on dev. Image dimensions and decoder allocations
   are bounded. Tests cover grayscale, RGB, RGBA, palette transparency, 16-bit
   channels, corrupt/truncated PNGs, oversized dimensions and APNG rejection.
3. **Medium: benchmark results were mislabeled.** The benchmark prebuilt its
   input and timed decoding only, while claiming encode/decode round trips and
   coverage of the largest supported message (it used 256 KiB; the actual limit
   is 16 MiB). It now decodes a masked client frame and encodes the server reply
   inside each iteration, checks content, and reports payload throughput and
   average iteration time. Documentation distinguishes these codec measurements
   from network latency/backend throughput and retracts the earlier labels.

The icon protocol was checked against upstream primary sources:
[MOTDConnectionWrapper.java](https://github.com/lax1dude/eaglerxserver/blob/main/core/src/main/java/net/lax1dude/eaglercraft/backend/server/base/query/MOTDConnectionWrapper.java)
and
[ServerIconLoader.java](https://github.com/lax1dude/eaglerxserver/blob/main/core/src/main/java/net/lax1dude/eaglercraft/backend/server/base/ServerIconLoader.java).
Downloaded source SHA256 values are recorded in [environment.json](environment.json).

## Validation

- **59 Rust tests pass**: 9 codec tests, 44 gateway unit tests and 6 real-socket
  integration tests. [Log](rust-tests.log). Includes all existing 755–777
  handshake/compression regressions plus the new image/protocol cases.
- **13 Python tests pass**: release inputs, deadlines and close assertions.
  [Log](python-tests.log).
- **Formatting, Clippy with warnings denied and locked WASI release build pass**.
  [Clippy](clippy.log), [build](wasm-build.log). Artifact:
  `rust/dist/EaglerXPumpkin.wasm`; hashes are in `environment.json`.
- **Four simultaneous simulated clients pass 120 seconds and reconnects**:
  Eagler 755, Eagler 777, Gaius tunnel 777 and vanilla TCP 777. All receive
  chunks, acknowledge teleport, receive their chat echo and send seven
  keepalive replies. All four reconnect smoke checks also pass.
  [Summary](mixed/summary.json), with per-case JSON/logs in [mixed/](mixed/).
- **Concurrent malformed-input/listener checks pass**: 60 attempts, ten per
  vector, plus 24 successful MOTD queries, with no errors, running during the
  sustained client sessions. [Results](listener-health.json), [log](listener-health.log).
- **64 MOTD burst queries pass at concurrency 8**: 32 icons and 32 no-icon
  requests; validates boolean flag, JSON size, binary-frame size/order and close.
  [Results](motd-burst.json).
- Embedded image regeneration with `scripts/prepare_motd_icon.py` reproduces
  the checked-in asset byte for byte (Pillow required only for regeneration).

The release-mode codec benchmark passed; this run measured approximately
621–1174 MiB/s of payload and 0.1–212.9 microseconds per iteration across
64 B, 1 KiB, 64 KiB and 256 KiB. These short microbenchmarks include correctness
assertions and are environment-sensitive. [Raw output](codec-benchmark.log).

## Reproduction and limits

Commands from `rust/` (Rust tools must be on PATH):

```sh
cargo test --locked --workspace --target aarch64-unknown-linux-gnu
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --target aarch64-unknown-linux-gnu -- -D warnings
python3 -m unittest discover -s tests -p 'test_*.py'
./build.sh
cargo test --release -p eagler-ws --target aarch64-unknown-linux-gnu frame_codec_micro_benchmark -- --nocapture
```

The live run used the freshly built native CLI on loopback `:18082` pointing to
existing Multiversion `:25565` and Pumpkin `:25566`, with protocols 755–777 and
`connections_per_minute = 0`. This prevented the fuzz worker from exhausting the
shared source-address budget; the separate real-socket rate-limit test passed.
The installed gateway on `:8081` was left running. The temporary CLI was stopped
after testing. Exact installed backend/translator versions were not re-established.

```sh
python3 tests/probe_compatibility.py --port 18082 --sustain --seconds 120 --reconnect --output-dir NEW_DIR/mixed
# Run concurrently:
python3 tests/soak_26_3.py --port 18082 --clients 0 --fuzz-clients 1 --seconds 120 --motd-interval 5 --json-output NEW_DIR/listener-health.json
```

Socket tests need loopback access; their initial sandbox permission failures
were resolved by rerunning with that access. Native and WASI share the runtime,
but the rebuilt WASI artifact was compiled only, not installed/reloaded in the
Pumpkin host. No graphical Eagler client was launched: actual icon rendering,
world rendering and interactive gameplay remain unverified by this review.
Two-minute sessions do not establish long-duration stability or capacity.
