# Eagler client investigation

The local test gateway rejected every protocol below 777 even with Multiversion
installed. Its active `plugins/data/eaglerx-pumpkin/eagler.toml` had neither
protocol bound, so its startup log correctly reported `MC protocols 777-777`.
This rejection happens before backend login or protocol translation.

The local config was backed up, then these fields were added and the idle
`pumpkinmc-test.service` was restarted:

```toml
min_minecraft_protocol = 755
max_minecraft_protocol = 777
```

The backend remains Multiversion :25565 forwarding to Pumpkin :25566.
No gateway binary or source-code change was needed for this configuration fix.
See [environment.json](environment.json) for binary hashes and versions.

## Results and limits

- [Before the fix](negotiation.json), synthetic Eagler v5 offers for 766,
  774 and 776 were rejected with the exact-777 mismatch; 777 was accepted.
- [After the fix](login-after-range-fix.json), all three older protocols
  completed negotiation, translated backend login and the Eagler finish exchange.
- [Identity login replays](identity-login-replays.json) also passed for all
  eight inventory entries' expected protocols, offering handshake versions
  3/4/5 and supplying an initial username. These are constructed packets,
  not executions of the HTML files. Repeating protocol 774 for five builds
  does not establish compatibility with each build's implementation.
- Eagler-only synthetic gameplay passed for [1.17](play-755.json) and
  [26.3](play-777.json): chunks, teleport reply, chat echo and keepalive;
  26.3 also completed registry/configuration exchange.
- Simultaneous [1.17](sustained-755.json) and [26.3](sustained-777.json)
  sessions each lasted 120 seconds and answered seven keepalives.
- Both sustained identities then passed reconnect smoke checks:
  [1.17](reconnect-755.json) and [26.3](reconnect-777.json).

Gaius was not used in these runs. Its tunnel results are separate from
Eaglercraft compatibility.

[clients.json](clients.json) inventories the eight exact files in
`/home/thomass/Downloads/eagler-clients/`, including SHA-256 hashes. The
1.21.11 files retain misleading 1.12 HTML titles, but their decoded WASM
contains 1.21.11 version strings. Static operator inspection of v4's hello
function found Eagler versions 3/4/5 and a protocol getter returning 774.
Its `mc47` incompatibility message is not evidence that it offers protocol 47.
Static presence or absence of a named V5 method is not a runtime test;
the 1.21.11 WASM omits the function-name section used by that name search.

**The HTML clients were not launched.** The computer-use tool reported
`The browser access host is unavailable`. Graphical login, rendering and
gameplay for these exact builds remain unverified. In particular, protocols
766/774/776 passed backend login checks, not full gameplay checks. The
configuration fix resolves the reproduced rejection, but these results do
not rule out additional real-client bugs after login.

## Repeat gameplay checks

From `rust/`, with the range and Multiversion route above active:

```sh
python3 tests/probe_1_17.py --sustain --seconds 120 --name OlderSoak
python3 tests/probe_26_3.py --transport eagler --sustain --seconds 120 --name CurrentSoak
```

Run the commands concurrently to repeat the paired sustained check. Re-run
without `--sustain` using the same names for reconnect smoke checks. These
commands execute synthetic Eagler protocol clients, not the supplied HTML.
