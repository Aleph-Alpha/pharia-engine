cd greet-go
go generate
tinygo build -target=wasi -o main.wasm main.go
wasm-tools component embed --world skill ../wit main.wasm -o main.embed.wasm
wasm-tools component new main.embed.wasm --adapt ../wasi_snapshot_preview1.reactor-22.0.0.wasm -o main.component.wasm
wasm-tools validate main.component.wasm --features component-model
cp main.component.wasm ../skills/greet-go.wasm
cd ../
