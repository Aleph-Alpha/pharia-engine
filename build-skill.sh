#!/bin/bash
set -x #echo on

cargo install wasm-tools
cargo build -p greet-skill --target wasm32-wasi --release
wasm-tools component new ./target/wasm32-wasi/release/greet_skill.wasm -o ./skills/greet_skill.wasm --adapt ./wasi_snapshot_preview1.reactor-21.0.1.wasm
