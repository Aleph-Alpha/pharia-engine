# Evaluating Pharia Kernel

## Findings Memory Consumption

An almost empty Python Skill, but at least with a native `pydantic` dependency consumes as
compiled Wasm file around 50 MB. Successively adding such skills to Pharia Kernel increases
memory consumption first by about 800 MB, second by about 300 MB and each following copy
by about 150 MB.

On a Mac paging starts at about 20 skills.

On a Mac (32 GB, Mac Pro 3) shuting down a running Pharia Kernel instance can take a very long time.
Even for a mere 5 skills it may take 6 seconds, whereas on a PC (16 GB, 5700G) it takes only 120 ms.

## Findings Execution Performance

Todo: no big deal
