#!/bin/sh
# Run the same gateway outside Pumpkin for development. The WASM plugin and
# this CLI must not bind the same listener port at the same time.
set -eu
cd "$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
task_cargo=$(command -v cargo || true)
if [ -z "$task_cargo" ]; then task_cargo="$HOME/.cargo/bin/cargo"; fi
task_host=$("$task_cargo" -vV | sed -n 's/^host: //p')
exec "$task_cargo" run --locked --release --target "$task_host" -p eaglerx-pumpkin --bin eaglerx-pumpkin -- "${1:-eagler.toml}"
