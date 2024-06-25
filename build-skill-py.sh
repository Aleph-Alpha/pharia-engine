python3 -m venv .venv
source .venv/bin/activate
pip install componentize-py

componentize-py -d wit/skill.wit -w skill bindings greet-py
componentize-py -d wit/skill.wit -w skill componentize greet-py.app -o ./skills/greet-py.wasm
