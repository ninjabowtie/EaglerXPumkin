#!/bin/sh
set -eu
cd "$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
task_cargo=$(command -v cargo || true)
if [ -z "$task_cargo" ]; then task_cargo="$HOME/.cargo/bin/cargo"; fi
"$task_cargo" build --locked --release --lib -p eaglerx-pumkin --target wasm32-wasip2
mkdir -p dist
cp target/wasm32-wasip2/release/eaglerx_pumkin.wasm dist/EaglerXPumkin.wasm
printf '%s\n' "Built $(pwd)/dist/EaglerXPumkin.wasm"
