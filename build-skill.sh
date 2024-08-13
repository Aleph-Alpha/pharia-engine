#!/bin/bash
set -x #echo on

cargo install wasm-tools
cargo build -p greet-skill --target wasm32-wasi --release
wasm-tools component new ./target/wasm32-wasi/release/greet_skill.wasm -o ./skills/greet_skill.wasm --adapt ./wasi_snapshot_preview1.reactor-23.0.2.wasm

cargo build -p greet-skill-v0_1 --target wasm32-wasi --release
wasm-tools component new ./target/wasm32-wasi/release/greet_skill_v0_1.wasm -o ./skills/greet_skill_v0_1.wasm --adapt ./wasi_snapshot_preview1.reactor-23.0.2.wasm

cargo build -p greet-skill-v0_2 --target wasm32-wasi --release
wasm-tools component new ./target/wasm32-wasi/release/greet_skill_v0_2.wasm -o ./skills/greet_skill_v0_2.wasm --adapt ./wasi_snapshot_preview1.reactor-23.0.2.wasm
