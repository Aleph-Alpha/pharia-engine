# Pharia Kernel

Pharia Kernel allows you to execute Cognitive Business Units called skills. These Skill can be written in a number of languages, including Python. The kernel handles the interaction between these skills and drivers for functionality like inference and retrival via the cognitive system interface. This enables to deploy RAG usecases serverless.

The entire Stack including Kernel, Inference, Document Index, etc is called **Pharia OS**.

The current prototype is deployed at <https://pharia-kernel.aleph-alpha-playground.stackit.rocks/>

The status page for uptime robot is found at: <https://stats.uptimerobot.com/gjXpoIPMnv>

![Block Diagram Pharia OS](./tam/pharia-os-running.drawio.svg)

## Contributing

In this repository we stick to Conventional commits. See: <https://www.conventionalcommits.org/en/v1.0.0/>.


### Local Development setup

There are some prerequisites you need to install once

```shell
# We need the Was target to be able to compile the skills
rustup target add wasm32-wasi
# The wasm-tools are also required for our skill build tooling
cargo install wasm-tools
```

Every time we change the example skill we need to rebuild them.

```shell
./build-skill.sh
```

### Building and running the kernel container using Podman

You can build the image with

```shell
podman build . --tag pharia-kernel
```

On Apple Silicon, you need to specify the target platform

```shell
podman build . --tag pharia-kernel --platform linux/arm64
```

Then, run the image with

```shell
podman run -p 8081:8081 pharia-kernel
```
## pushing skill oci

login to container registry:
```shell
podman login registry.gitlab.aleph-alpha.de
```

push container:
```shell
wasm-to-oci push skills/greet-py.wasm registry.gitlab.aleph-alpha.de/engineering/pharia-kernel/skills/greet-py:v1
```

pull container:
```shell
wasm-to-oci pull registry.gitlab.aleph-alpha.de/engineering/pharia-kernel/skills/greet-py:v1 --out skills/oci.wasm
```

## Design Pharia Kernel

Pharia Kernel is a single process running in a docker container, running actors in a tokio runtime.

![Block Diagram Kernel Overview](./tam/kernel-block.drawio.svg)

* **Shell**: Exposes interface for applications. Handles http requests.
* **Skill Executer**: Invokes skill in green threads. Forwards their input and output to the shell. Exposes the **C**ognitive **System** **I**nterface (CSI) to the skills.
* **Context Message Bus**: Exposes the combined API of all drivers via channel to the **Skill Executer** and handles messaging between drivers.
* **Drivers**: Act as ports for the various external systems.

## Deploying Pharia Kernel on Customer side

**Pharia Kernel** is intended to be installed **on premise** by the customer it. It is deployed, as are all other modules of the **Pharia OS**, to the JFrog Artifactory. Our colleagues at the Pharia OS Team are going to develop tooling for deploying tooling for rolling it out. Until they come up with a name it is here called "Pharia Up".

![Block Diagram Pharia OS deploy](./tam/pharia-os-deployment.drawio.svg)
