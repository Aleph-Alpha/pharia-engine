# PhariaEngine

[![Aleph Alpha](https://img.shields.io/badge/aleph-alpha-212516.svg?labelColor=E3FF00&logo=data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAAAXNSR0IB2cksfwAAAARnQU1BAACxjwv8YQUAAAAgY0hSTQAAeiYAAICEAAD6AAAAgOgAAHUwAADqYAAAOpgAABdwnLpRPAAAAAZiS0dEAP8A/wD/oL2nkwAAAAlwSFlzAAAuIwAALiMBeKU/dgAAAAd0SU1FB+kKFA0JAyhHQSMAAAEzSURBVDjLjZO7SoNBEIU/UsUYjAErW8VOFLTKAxgbiYYgPoO+hm8Q9BG0NiB2VpGAdtrYSMoUwU2llfrbnIGTPxedZnfPXJidcwYmrQ5cAyXDSsLq/GHrQAIy4B4oAgXgTlhSzIQt6lwGugrOgAvhB8AQ6AHVXA41YAC0rN0nFfgBtoSvAmXdj5VTA+go+FsOgF29M6Cd6/bEfJ0YWrI/VhTYE/ZmyRVgJPwd2AvHkf37VNi5YSvCzgxroAkD3ACfum/qfM2x476PaL8wh1JvfW1WUBQ4NOE8TymwofPFKGyEc98Gk4AlS+wLf5gyxBTKDBq/TAthbaN4xzQwRmMIqamAskQDsC0xZcCjfbPlQnJZVsX/UPIFuDTqupL7mJRnLdOthlzUYs1dpvw6XwEL/1nnX8b7Wwgn81GqAAAAAElFTkSuQmCC)](https://aleph-alpha.com)
[![Build Status](https://github.com/aleph-alpha/pharia-engine/actions/workflows/ci.yml/badge.svg)](https://github.com/aleph-alpha/pharia-engine/actions)
[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![Contributions welcome](https://img.shields.io/badge/contributions-welcome-brightgreen.svg)](CONTRIBUTING.md)

PhariaEngine allows you to execute Cognitive Business Units called Skills. These Skills can be written in any language which compiles to WebAssembly (Wasm).
We provide a SDK and dedicated support for Python.
PhariaEngine handles the interaction between these Skills and drivers for functionality like inference and retrieval via the Cognitive System Interface (CSI).
Writing Skills for PhariaEngine is more constrained then shipping an end to end use case in a custom Docker container.
Yet these constraints allow us to make opinionated decisions for the Skill developer.
We strive to take away only the decisions and responsibilities a Skill developer may find "boring" (such as authentication, parallelization of inference calls).
In more technical terms, we aim to reduce the accidental complexity the Skill developer has to engage with.

> [!NOTE]
> Until now, PhariaEngine has been deployed as part of the PhariaAI product suite. Making PhariaEngine independent of PhariaAI a smooth experience is currently work in progress. Opensourcing this repository is one step towards that goal. This is the beginning of a journey and we are constantly improving.
>
> If you are excited about the PhariaEngine idea, please reach out or start contributing.

## Features

- **WebAssembly Skills**: Hosts AI methodology written in Python and makes it available via HTTP endpoints
- **Flexible Inference Backends**: Works with Aleph Alpha inference or any OpenAI-compatible backend
- **OpenTelemetry Tracing**: Integrated observability with support for OpenTelemetry backends like Langfuse and Langsmith, following GenAI semantic conventions
- **Model Context Protocol (MCP)**: Dynamic tool discovery and invocation through MCP servers with per-namespace configuration

## PhariaAI

The Engine is part of the [PhariaAI product suite](https://docs.aleph-alpha.com/products/pharia-ai/overview/).
However, it can also be configured to run standalone with any OpenAI-compatible inference backend.
If operated outside of PhariaAI, some features are not available.
These include RAG capabilities offered by the [DocumentIndex](https://docs.aleph-alpha.com/products/apis/pharia-search/aleph-alpha-document-index-api/), features like explainability that are offered by the [Aleph Alpha inference](https://docs.aleph-alpha.com/products/apis/pharia-inference/) and the integration with the [Pharia IAM service](https://docs.aleph-alpha.com/products/pharia-ai/pharia-os/user-management/).

## Setup

Next to the [Rust toolchain](https://www.rust-lang.org/tools/install), there are some prerequisites you will need to install once:

```sh
# We need the Wasm target to be able to compile the skills
rustup target add wasm32-wasip2
# We need wasm-tools to strip skill binaries
cargo install wasm-tools
# Create an .env file and set the missing values
cp .env.example .env
```

Now, you can run the Engine with

```sh
cargo run
```

### Testing

```sh
# Create an .env.test file and set the missing values
cp .env.test.example .env.test
# We recommend cargo nextest, but cargo test also works
cargo install cargo-nextest --locked
cargo nextest run --workspace --all-features
```

Note that some tests require access to a PhariaAI instance.

To deselect tests against OpenAI, run `cargo nextest run --features test_no_openai`.

### Formatting & Linting

```sh
cargo fmt -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
```

### Running the Container

Build the image with

```sh
podman build . --tag pharia-engine
```

Then, run the container with

```sh
podman run -v ./config.toml:/app/config.toml -p 8081:8081 --env-file .env pharia-engine
```

You can find more details on how to configure the Engine in the [OPERATING.md](./OPERATING.md).

#### MacOS

Podman on MacOS requires a separate virtual machine run by the user.
To compile PhariaEngine, at least 4 GiB of RAM are needed and 8 GiB are recommended. You set this up with

```sh
podman machine init
podman machine set --memory 8192
podman machine start
```

## Release

Releasing in this repository is automated with [release-pls](https://github.com/googleapis/release-please).

1. For every commit to the `main` branch, release-pls creates a release Pull Request.
2. Review the release Pull Request and add new changes, if necessary, by amending the commit directly.
3. Merge the release Pull Request and release-pls will release the updated packages.

## Usage Examples

The Engine organizes Skills in [namespaces](https://docs.aleph-alpha.com/products/pharia-ai/configuration/how-to-enable-custom-skill-development/#for-operators).
Each namespace can be configured to pull Skills from either a remote OCI registry or a local directory. Configuration examples can be found in the `.env.example` file.
The Engine comes configured with a `test-beta` namespace that hosts some hard-coded skills. You can invoke the `hello` skill with

```sh
curl -v 127.0.0.1:8081/v1/skills/test-beta/hello/message-stream \
-H 'Content-Type: application/json' \
-d '"Homer"'
```
