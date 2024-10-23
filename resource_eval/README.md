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
The compiled skill does nothing else but greet a Person and optionally may consume some memory or waste some time.

For the Rust skill you enter the respective directory and build it for the wasm platform.
Note, that we cannot use the SDK as we live in the Pharia Kernel repo and it is impossible to not use a `Cargo.toml`
in the parent folders.

```shell
cd sample_rs
cargo build --target wasm32-wasip2 --release
cd ..
cp ../target/wasm32-wasip2/release/sample_rs.wasm .
```

Finally, we need to copy the generated file `sample_rs` from the parent target folder to the current folder.

## Concept

We do a series of skill deployments, accesses and other Pharia Kernel operations,
which can be described by a simple command language stored in files with a `.cmds` ending.
Each line starting with a character is tried to be interpreted as a command.
The first word should be a valid command.
All subsequent arguments delimited by whitespace are arguments to this command.
Commands are:

* `add_py`: Adds a python skill, makes a copy of `sample_py.wasm` and numbers the skills
  incrementally, the first skill added is `sample_py1.wasm`.
* `skills`: Lists all the skills.
* `execute_skill sample_py1 Alice 100 20`: Executes a skill with the name `sample_py1` and
  greets `Alice`, while doing so 100 KBytes of memory and 20 ms of wall time are wasted.
  The 20 is optional and defaults to 0, the 100 is optional and defaults to 0.
* `execute_all Alice 0 0`: Executes all skills that are listed with the given parameters.

You may put a number `n` before a command, which repeats command execution `n` times.

TODO: parallel repetition, parallel blocks

We capture the output of a run of the Pharia Kernel operations in a file such as `241022_162524.558_run.log`,
which means that the run was captured 2024 on October the 22nd at 16:25:24 and 558 ms.
The ending `_run.log` indicates that all information for evaluation is stored there, each line is a Json string.
These `_run.log` files can be stored and checked in to compare evaluations over time.
In the beginning of a `_run.log` file, there are always three lines.
First information about the machine the bench runs on, second about the binary, third about the cmds file.
Then for each Pharia Kernel operation there is a line with the `cmd` it executed, how long it `took` in ms.
In addition there is the resident set size (`rss`), which is the amount of memory in KB currently in RAM for the
running process, after the command and by how much it changed compared to before the command (`rss_diff`).
There is similar information about the virtual memory size (`vms`, `vms_diff`).
Note that `rss` may go down during a run if memory is swapped out. Unfortunately, there is no platform
independent (i.e. Linux, Mac, and Windows), way to log this accurately.
The last line observes the command `stop`, which happens automatically and may take a long time on a Mac.

The other log file ends with `_pk.log` and captures the Pharia Kernel log output as well as intermittent
resource consumption messages.
This log file is only used for error tracing and can safely be removed after a successful run as it is
not used for evaluation.

## Running

To run the evaluation with the commands file `bench.cmds` execute:

```shell
./main.py run -m bench.cmds
```

Alternatively, you can run the module

```shell
python3 -m resource_eval run -m bench.cmds
```

Two log files are automatically created for each run, one with `..._pk.log` and one with `...run.log`.
In addition progress is shown on screen.
With `-m` intermittent resource consumption messages are activated, which is helpful for long running
commands with many changes in resource consumption.

## Evaluation

Todo: Compare runs, graphics
