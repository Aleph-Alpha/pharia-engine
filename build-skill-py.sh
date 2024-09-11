cargo install wasm-tools

python3 -m venv .venv
source .venv/bin/activate
pip install componentize-py --upgrade

componentize-py -d wit/skill@unversioned/skill.wit -w skill bindings greet-py
componentize-py -d wit/skill@unversioned/skill.wit -w skill componentize greet-py.app -o ./skills/greet-py.wasm
wasm-tools strip ./skills/greet-py.wasm -o ./skills/greet-py.wasm

componentize-py -d wit/skill@0.2/skill.wit -w skill bindings greet-py-v0_2
componentize-py -d wit/skill@0.2/skill.wit -w skill componentize greet-py-v0_2.app -o ./skills/greet-py-v0_2.wasm
wasm-tools strip ./skills/greet-py-v0_2.wasm -o ./skills/greet-py-v0_2.wasm
