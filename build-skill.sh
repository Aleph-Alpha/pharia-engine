#!/bin/bash
set -x #echo on

cargo install wasm-tools
cargo build -p greet-skill --target wasm32-wasi
wasm-tools component new ./target/wasm32-wasi/debug/greet_skill.wasm -o ./skills/greet_skill.wasm --adapt ./wasi_snapshot_preview1.reactor-21.0.1.wasm