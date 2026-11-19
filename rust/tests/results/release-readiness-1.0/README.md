# 1.0.0 release readiness assessment

**Verdict: NO-GO for a stable 1.0.0 release at commit `7ab70a8`.**
The core gateway is a credible prerelease, with substantially improved automated
coverage. This assessment reproduced a lifecycle failure in the current candidate
and found important compatibility, sustained-load and release-delivery gaps.
Passing unit tests alone does not close those gaps.

Assessed scope: a Pumpkin 26.3 offline-mode WebSocket/Eagler gateway, with optional
Multiversion translation. Full Java EaglerXServer parity, voice, account
registration and legacy-client support are not assumed prerequisites for 1.0.
The public support contract must say which subset actually ships.

Assessment date: 2026-11-18 America/Los_Angeles / 2026-11-19 UTC.
This task changed no production source, installed plugin, service or server
configuration. Only local assessment artifacts were added. Nothing was published.

## Fresh verification

| Check | Result and limit |
| --- | --- |
| Native Rust suite | 82 pass; two manual benchmarks ignored |
| Python regressions | 13 pass |
| Formatting and native/WASI Clippy | Pass, warnings denied |
| Locked WASI release build | Pass |
| Fresh isolated Pumpkin host | Pass: negotiation, configuration, dynamic show/hide/broadcast, flags and non-operator rejection |
| Temporary `1.0.0` stamping and clean build | Pass in an archived checkout under `/tmp`; actual host logs `Loaded eaglerx-pumpkin (1.0.0)` and command checks pass |
| Notification-enabled unload/reload | **Fail on first reload**; listener absent |
| Notification-disabled lifecycle control | Three active-session unload/reload cycles pass |
| Current RustSec lockfile audit | No known vulnerabilities found across 90 locked dependencies; one maintenance warning |
| Release-package local links | Seven broken link occurrences, six distinct local targets |
| Graphical acceptance / extended candidate soak | Not run in this assessment; current evidence remains incomplete |

The candidate WASM hash is
`0eb0ed902d7f4895ffeacf1a304f78eb0b47950c5d9988077ba26b989e1866e3`.
The temporary version-stamped artifact hash is
`535290147e4fa67c566ebeca7361d14c73c669ffa7ab3c46304c15dc4b1bcc13`.
The host was Pumpkin 0.2.2+26.3-26.60; exact hashes and tool versions are in
[environment.json](environment.json). Local native checks ran on aarch64 Linux;
no fresh x86 host or graphical browser run is inferred from these results.

## Release blockers and material risks

### R1 — Confirmed lifecycle failure with notifications enabled: fix before stable

Reproduction against the exact candidate artifact in an isolated host:

1. Start with `dynamic_notifications = true` and connect a capable Eagler client.
2. Run `plugin unload eaglerx-pumpkin`; unload succeeds and the listener closes.
3. Run `plugin load EaglerXPumpkin.wasm` using a file in the test working directory.
4. Pumpkin logs: `Failed to initialize plugin eaglerx-pumpkin: Initialization failed: Permission eaglerx-pumpkin:command.notify is already registered`.
5. Pumpkin subsequently prints `Plugin EaglerXPumpkin.wasm loaded successfully`,
   but connecting to the gateway fails with connection refused.

The command adapter unconditionally registers the permission in
`src/notification_command.rs:22`. Permission state survives unload in this tested
host. The integration needs a host-compatible, idempotent registration/lifecycle
strategy; it must not silently swallow arbitrary permission errors.

The control with notifications disabled passes three unload/reload cycles,
including active-session closure and successful logins after rebind. That
isolates the observed failure to the enabled command/permission path, rather
than a universal inability to unload the runtime.

Evidence: [failure result](lifecycle-enabled-result.json),
[failure console](lifecycle-enabled-console.log),
[control result](lifecycle-disabled-result.json),
[control console](lifecycle-disabled-console.log).
The host's misleading success message is also an upstream integration concern.
Full server restart works in the fresh-start tests, but the documentation
currently tells admins to reload as well as restart. Merely leaving the option
false by default does not make the advertised enabled path stable.

### R2 — Real-client compatibility evidence is insufficient for the advertised scope

The strongest recorded graphical play result is Gaius 26.3 over `/tunnel`, on an
earlier gateway build. That is valuable but does not verify graphical Eagler
`/` gameplay on this release candidate. The user's successful icon check is
narrow icon-path evidence, not a complete gameplay acceptance run.

Protocol 755–777 unit regressions verify synthetic handshake/login/framing
behavior. Earlier simultaneous 755/777 Eagler, Gaius and raw-TCP probes verify
network flows, not inventory semantics, rendering, entity interaction or the
correct appearance of skins, capes, notifications and pause-menu controls.
The recorded compatibility inventory still marks graphical candidates pending.

Before stable, test and record exact client hashes, observed protocols,
Pumpkin/translator versions and artifact hashes. At minimum verify a genuine
Eagler client and the claimed Gaius path on the exact candidate: login, chunks,
movement/teleports, chat, inventory/containers, block/entity interaction,
keepalives and same-identity reconnects. Verify advertised optional UI and skins
visually. For cross-version support, verify the actual translated client paths;
otherwise label those combinations experimental and narrow the 1.0 claim.

### R3 — Candidate sustained-load, memory and scheduler behavior are not established

The recorded sustained mixed-client runs are 120 seconds with four simulated
clients. The later concurrent malformed-input run is useful, but it used an
older native gateway artifact. Neither it nor codec microbenchmarks establish
hours of stability or production capacity for the current WASI candidate.

Concrete reasons to require load evidence:

- Each send queue permits 16 MiB and there are two per connection. The aggregate
  *allowance* at the default 64-connection cap is 2 GiB for send queues alone;
  at the configurable 256 cap it is 8 GiB. These are theoretical allowances,
  not observed RSS, and exclude receive/fragments/decompression/host overhead.
  There is no plugin-wide queued-byte budget. Finite per-connection limits alone
  do not establish safe aggregate memory behavior.
- Backend connect uses a synchronous `connect_timeout(..., 20 ms)` inside polling.
  Processing multiple slow local connects can consume a meaningful fraction of
  the server tick budget. There is no measured candidate TPS/poll-latency envelope.
- The WASI task polls once per server tick and accepts one new connection per
  invocation. At 20 TPS that is at most about 20 accepts/second, including refused
  connections, before any host scheduling delay. Native CLI behavior is not a
  substitute for this host schedule.
- Slow readers, incomplete frames, fragmented input, compressed input, connection
  churn and backend failure/recovery have not been measured together under
  sustained load with a declared reference memory/TPS budget.

These are design risks and unverified capacity limits, not a claim that an OOM,
exploit or tick stall was reproduced here. Define a modest initial supported
concurrency and reference hardware, then measure the actual WASI artifact. A
reasonable release gate is at least a four-hour mixed workload with reconnects
and concurrent malformed-input/slow-client checks, followed by a longer RC soak.
Record peak/steady memory, tick rate, poll latency, disconnects and recovery.
Reduce limits or add a global memory budget if measurements require it.

### R4 — Distribution documentation and provenance need a release pass

The workflow copies only the Rust README, example config, license, WASM and
release metadata into the ZIP. The README links to absent repository docs,
tests and source files: seven local-link occurrences fail in that archive layout.
See [package-check.json](package-check.json). Ship the needed documents or use
version-specific repository URLs and test the downloaded archive from an empty
folder, not a source checkout.

Stable `release.json` records `github.sha` as `commit`, while the publish job
creates and tags a later version-bump commit. The tree is checked, which is good,
but the metadata's commit field does not identify the final tagged commit.
Clarify it as a base commit plus the stamped tree/version, or otherwise make
published provenance unambiguous. Beta/alpha tags also point at unstamped source
while their artifacts contain a stamped version; document how to reproduce them.

There are no dedicated upgrade/rollback notes for stricter validation: image URLs
are no longer accepted as `motd_icon`, static MOTDs are capped at 2048 bytes,
backend port zero and exact protocols below 755 are rejected. Existing absent
options retain defaults, but rollback to an older binary after adding new options
can fail because config parsing rejects unknown fields. Back up config as well
as WASM. Test permission-cache behavior for a changed artifact hash, invalid
config, occupied listen port, backend outage and recovery.

### R5 — Security and deployment contract needs explicit sign-off

Strengths: numeric loopback-only destinations, fixed tunnel backend port,
per-address/global connection limits, timeouts, bounded message/queue sizes,
compression validation, IP policy, opt-in notification commands and tested
non-operator rejection. The native CLI now emits its diagnostic logs.

Limits that must be understood before advertising public deployment:

- Offline names are not authenticated identities. TLS protects transport, not
  ownership of an offline username. Review operator/whitelist/ban policy through
  the chosen backend/translator route; an IP policy is not account authentication.
  This does not require adding the deliberately out-of-scope registration system.
- The default gateway bind is `0.0.0.0`. Public HTTPS clients need external TLS
  termination. The repository mentions this but lacks a worked, verified public
  deployment example and trusted-proxy/rate-limit guidance.
- Address filtering/rate limiting use the TCP peer address. Behind a reverse proxy,
  users share the proxy's address; the default 30/minute budget can become a
  shared bottleneck. Forwarded headers are not trusted by the implementation.
  Document edge enforcement and tuning rather than implying per-player IP limits.
- The hand-written WebSocket/parser/codec surface has useful regression tests and
  a small malformed-input suite, but no recorded independent conformance campaign
  or coverage-guided fuzz campaign. Known dependency audit results do not assess
  this application-level surface or the Pumpkin executable/translator.

The fresh RustSec audit reports zero known vulnerabilities and one
`RUSTSEC-2023-0089` unmaintained warning for `atomic-polyfill`, reached in the
lockfile via `heapless` / `tracing-serde-structured` / `pumpkin-plugin-api`.
The selected WASI graph does not include `atomic-polyfill`; the warning is not
an identified vulnerability in this plugin. Record/review it rather than equating
it with either a clean bill of security or a critical failure.
Audit database: 1533 advisories, commit
`472a4c9a785030434f15d7da84964d60fbe789ab`, updated 2026-11-18.
[Full audit](dependency-audit.json),
[official advisory](https://rustsec.org/advisories/RUSTSEC-2023-0089.html).

### R6 — Automation, support promises and licensing remain release decisions

Only a manual release workflow exists. It runs native Rust tests, native/WASI
Clippy and a WASI build, but only the release-input Python tests; it does not run
the full Python suite, a host load/reload smoke test, dependency audit or live
compatibility probes. Add a non-publishing PR/push quality gate and include the
host regression that found R1. Publishing can remain manual.

The compatibility/roadmap documents contain older test counts and partial feature
status. Freeze the supported Pumpkin release/API, client matrix, config/command
contract and upgrade policy. Specify what compatibility 1.0 promises; a version
bump itself does not establish that contract. Rust manifests also omit an explicit
minimum supported toolchain and licensing metadata; this is more important if
the public Rust APIs/crates are intended to be supported, not just a WASM artifact.

The local LICENSE and
[upstream LICENSE](https://github.com/lax1dude/eaglerxserver/blob/main/LICENSE)
contain an all-rights-reserved notice and warranty/liability disclaimer, without
an explicit reuse/distribution grant in those files. Attribution is present, but
these files alone do not settle the distribution permission question. Obtain
maintainer clarification/review for the port and shipped assets/dependencies
before treating licensing as a completed release checklist item. This assessment
does not determine infringement or declare distribution prohibited.

Finally, the remote `main` remains at `c2d9022`; the assessed fixes are on `dev`
`7ab70a8`. The release workflow requires `main`. Dispatching it on current `main`
would not release this assessed code. Integrate and revalidate the intended
release tree first; do not simply dispatch a stable version from the old branch.

## Missing features: which ones actually block 1.0?

| Area | Present | Missing / decision |
| --- | --- | --- |
| Core transports | Eagler negotiation/login, Gaius tunnel, raw TCP relay, compression | Real client acceptance and declared supported matrix are blockers; universal version support is not implied |
| 1.17+ translation | Range negotiation and protocol-aware login injection; Multiversion route documented | Translation remains external; real per-client compatibility is incomplete. Mark experimental or verify what 1.0 claims |
| Legacy clients | Pre-1.17 explicitly rejected/deferred | 1.8/1.12/Rewind are not blockers for a clearly scoped 26.3/1.17+ gateway; they block a full legacy-parity claim |
| MOTD | Static icon, custom 64x64 PNG data URI, text animation, no-icon handling | Per-frame icon animation, sprites, weighted/named chains and full cache-query subtypes are absent. Optional unless advertised as full EaglerMOTD parity |
| Skins/capes | Presets, custom profile textures, local V5 responses and fallback | URL fetching/persistent caches deliberately absent; visual acceptance of implemented paths still needed |
| Notifications | Target/broadcast show/hide, IDs, priorities, silent mode, lifetimes, operator permission | Enabled reload is broken. Custom badge icons, text colors/rich components and full backend RPC/message-channel parity are absent and can be deferred |
| Pause menu/server info | Static external-link button | Rich embedded server-info/webview and broader customization absent; optional |
| Identity policy | Address allow/block/rate limits; backend handles player policy | Gateway UUID-ban masks/player whitelist absent by design. Verify backend enforcement and explain offline identity limitations |
| Web/file hosting | Not implemented | EaglerWeb is deferred; not necessary for core gateway 1.0 |
| Voice/accounts | Deliberately outside this port's scope | Voice is delegated to another plugin; registration/password flow not planned. Do not make either a surprise release prerequisite |
| Operations | Periodic logs, native diagnostics, config validation and lifecycle hooks | Hot reload correctness, upgrade/rollback instructions and measured deployment envelope need completion |

A stable, explicitly limited gateway does not need every original Java addon.
Conversely, calling the release a full EaglerXServer port would make considerably
more of this missing functionality material. The scope must be settled before
choosing a stable version number.

## Recommended path to a release decision

1. Fix R1 and retain an automated enabled/disabled lifecycle test. Exercise cold
   start, active-client unload, three reload cycles, command re-registration and
   reconnects. Verify the listener and commands, not only the host success text.
2. Publish a precise support matrix and complete graphical acceptance on the
   candidate for the combinations included in the 1.0 promise. Leave other
   protocols experimental rather than inferring support from negotiation alone.
3. Complete current-WASI sustained/load/failure testing with an explicit supported
   concurrency and measured memory/TPS budget; run backend policy and public TLS/
   proxy deployment checks. Retain hashes for all participating components.
4. Fix archive documentation/provenance, provide upgrade and rollback notes,
   resolve the licensing question, update stale status docs, and add automated
   non-publishing quality/host checks. Keep release publication manual.
5. Merge the intended candidate into `main`, rerun checks against that exact tree,
   stage/version the artifacts, and perform a clean-install acceptance of the
   downloadable package. Reassess; do not use this assessment to certify a later
   untested commit automatically.

**Recommendation today:** continue as a beta/prerelease. After the concrete
lifecycle failure and the evidence/delivery gaps are closed, a narrowly scoped
1.0 is reasonable without adding all optional original-plugin features.

## Evidence and reproduction

- [Fresh Rust tests](rust-tests.log), [Python tests](python-tests.log),
  [native Clippy](clippy-native.log), [WASI Clippy](clippy-wasi.log), [build](build.log).
- [Fresh host](fresh-host-result.json), [fresh host console](fresh-host-console.log).
- [Stamped build](stamped-build.log), [stamped host result](stamped-host-result.json),
  [stamped host console](stamped-host-console.log).
- [Dependency audit](dependency-audit.json), [package inspection](package-check.json).
- [Notification-enabled lifecycle reproducer](lifecycle_probe_enabled.py),
  [disabled control](lifecycle_probe_disabled.py). These are assessment harnesses,
  derived from the existing live probe, not production changes.

From `rust/`, using fresh output directories and loopback permission:

```sh
cargo test --locked --workspace --target aarch64-unknown-linux-gnu
cargo clippy --locked --workspace --all-targets --target aarch64-unknown-linux-gnu -- -D warnings
cargo clippy --locked --workspace --lib --target wasm32-wasip2 -- -D warnings
cargo fmt --all -- --check
python3 -m unittest discover -s tests -p 'test_*.py'
./build.sh
python3 tests/probe_notifications.py --pumpkin-binary /path/to/pumpkin-server --output-dir /tmp/NEW-fresh-host
python3 tests/results/release-readiness-1.0/lifecycle_probe_enabled.py --pumpkin-binary /path/to/pumpkin-server --output-dir /tmp/NEW-lifecycle-on
python3 tests/results/release-readiness-1.0/lifecycle_probe_disabled.py --pumpkin-binary /path/to/pumpkin-server --output-dir /tmp/NEW-lifecycle-off
```

The enabled reproducer is expected to exit nonzero on this commit. Both lifecycle
harnesses stop their isolated child host in `finally`. The temporary 1.0-stamped
build was made from `git archive HEAD`, stamped with the repository's release
script and built offline/locked, without changing the checked-out Cargo files.
The audit used official cargo-audit 0.23.0 with its archive digest checked against
RustSec's release metadata before execution. No release workflow was dispatched.
