# Evaluating Pharia Kernel

Memory requirements of Pharia Kernel can be rather high, if Python is used as a runtime environment for skills.
This is mainly because right now (Oct/24), the Python interpreter and all dependencies are not shared in the WASM runtime.
To track changes, we evaluate memory usage when installing and accessing many skills written in Python.

## Preparation

Ensure that `uv` is installed (independent of project) or install as described on the [UI-Website](https://docs.astral.sh/uv/).
On Mac and Linux do as follows:

```shell
curl -LsSf https://astral.sh/uv/install.sh | sh
```

Make sure that the environment variables are set in the file `.env`, see `.env.example` for guidance.
We need for installation already the jfrog credentials, as we need `pharia-kernel-sdk-py`.
You can export these as follows:

```shell
set -a; source .env
```

Create a `.venv` and sync the dependencies as follows:

```shell
uv venv
source .venv/bin/activate
uv sync
```

Compile the `sample_py` skill module to a WASM file.

```shell
pharia-skill build sample_py
```

which should provide you with a file `sample_py.wasm`.
For the evaluation of the resource consumption we copy from this file.

## Concept

We do a series of skill deployments, accesses etc. which can be described by
a simple command language. Each command starts in a line with a valid command.
All subsequent arguments delimited by whitespace are arguments to this command.
Commands are:

* `add_py`: adds a python skill, makes a copy of `sample.wasm` and numbers the skills
  incrementally, the first skill added is `sample1.wasm`.
* `list_skills': lists all the skills
* `execute skill sample1 Alice 100 20`: executes a skill with the name `sample1` and
  greets `Alice`, while doing so 100 KBytes memory and 20 ms all time are wasted.
  The 20 is optional and defaults to 0, the 100 is optional and defaults to 0.
* `execute_all Alice 0 0`: executes all skills that are listed with the given Parameters

As a special you may put a number `n` before a command, which repeats command execution
`n` times.

TODO: parallel repetition, parallel blocks

We capture the output of a run in file `241022_162524.558_run.log`, which means that the
run was captured 2024 on October the 22nd at 16:25:24 and 558 ms. The ending `_run.log` 
indicated that all information for evaluation is stored there, each line is a Json string.
These files can be stored and checked in to compare evaluations over time.
The other log file ends with `_pk.log` and captures the Pharia Kernel output. If all 
works well it can be safely removed.

## Running

To run the evaluation with the commands file `bench.cmds` do:

```shell
./main.py run bench.cms
```
