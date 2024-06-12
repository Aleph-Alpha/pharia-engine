# Pharia Kernel

Pharia Kernel allows you to execute Cognitive Business Units written in Python and handles their interaction with other modules of Pharia like inference and search.

Pharia Kernel acts as an "opinionated message bus" between business applications, inference and retrieval. This enables for Retrieval Augmented Generation (RAG) among other things. In addition Kernel can be seen as a serverless lambda engine for cognitive business units called "skills".

The entire Stack including Kernel, Inference, Document Index, etc is called **Pharia OS**.

![Block Diagram Pharia OS](./tam/pharia-os-running.drawio.svg)

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

## Getting Started

You can build the image with

```bash
podman build . --tag pharia-kernel
```

On Apple Silicon, you need to specify the target platform

```bash
podman build . --tag pharia-kernel --platform linux/arm64
```

Then, run the image with

```bash
podman run -p 8081:8081 pharia-kernel
```

## Contributing

In this repository we stick to Conventional commits. See: <https://www.conventionalcommits.org/en/v1.0.0/>. 