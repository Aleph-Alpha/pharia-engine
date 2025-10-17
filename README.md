# PhariaKernel

PhariaKernel allows you to execute Cognitive Business Units called Skills. These Skills can be written in any language which compiles to WebAssembly (Wasm).
We provide a SDK and dedicated support for Python.
PhariaKernel handles the interaction between these Skills and drivers for functionality like inference and retrieval via the Cognitive System Interface (CSI).
Writing Skills for PhariaKernel is more constrained then shipping an end to end use case in a custom Docker container.
Yet these constraints allow us to make opinionated decisions for the Skill developer.
We strive to take away only the decisions and responsibilities a Skill developer may find "boring" (such as authentication, parallelization of inference calls).
In more technical terms, we aim to reduce the accidental complexity the Skill developer has to engage with.

## Features

- **WebAssembly Skills**: Hosts AI methodology written in Python and makes it available via HTTP endpoints
- **Flexible Inference Backends**: Works with Aleph Alpha inference or any OpenAI-compatible backend
- **OpenTelemetry Tracing**: Integrated observability with support for OpenTelemetry backends like Langfuse and Langsmith, following GenAI semantic conventions
- **Model Context Protocol (MCP)**: Dynamic tool discovery and invocation through MCP servers with per-namespace configuration

## PhariaAI

The Kernel is part of the [PhariaAI product suite](https://docs.aleph-alpha.com/products/pharia-ai/overview/).
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

Now, you can run the Kernel with

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

### Running inside Container

Build the image with

```sh
podman build . --tag pharia-kernel
```

Then, run the image with

```sh
podman run -v ./config.toml:/app/config.toml -p 8081:8081 --env-file .env pharia-kernel
```

We configure the bind address and port via the environment variable `KERNEL_ADDRESS`.
If not configured it defaults to "0.0.0.0:8081", which is necessary in the container, but locally may cause the firewall to complain.
Note that the Kernel can be configured both from environment variables and from a `config.toml` file.
Mounting the config file is optional.

#### MacOS

Podman on MacOS requires a separate virtual machine run by the user.
To compile PhariaKernel, at least 4 GiB of RAM are needed and 8 GiB are recommended. You set this up with

```sh
podman machine init
podman machine set --memory 8192
podman machine start
```

## Release

Releasing in this repository is automated with [release-plz](https://release-plz.ieni.dev/).

1. For every commit to the `main` branch, release-plz creates a release Pull Request.
2. Review the release Pull Request and add new changes, if necessary, by amending the commit directly.
   - Helm chart version:

     By default, only the patch version is incremented.
   - Helm chart changelog date:

     The date is set when the changelog is generated. Update it to the release date if differs.
3. Merge the release Pull Request and release-plz will release the updated packages.

Every time we update the `wit` worlds, we need to clear the Skill build cache in `./skill_build_cache` which is used by the tests.

## Deploying PhariaKernel on Customer side

**PhariaKernel** is intended to be installed as part of **PhariaAI** on premise by the customer it.
It is deployed, as are all other modules of the **PhariaAI**, to the JFrog Artifactory.

![Block Diagram Pharia OS deploy][deployment]

## Usage Examples

The Kernel organizes Skills in [namespaces](https://docs.aleph-alpha.com/products/pharia-ai/configuration/how-to-enable-custom-skill-development/#for-operators).
Each namespace can be configured to pull Skills from either a remote OCI registry or a local directory. Configuration examples can be found in the `.env.example` file.
The Kernel comes configured with a `test-beta` namespace that hosts some hard-coded skills. You can invoke the `hello` skill with

```sh
curl -v 127.0.0.1:8081/v1/skills/test-beta/hello/message-stream \
-H 'Content-Type: application/json' \
-d '"Homer"'
```
