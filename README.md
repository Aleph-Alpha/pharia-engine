# PhariaKernel

PhariaKernel allows you to execute Cognitive Business Units called skills. These Skill can be written in any language which compiles to web assembly. Yet we provide a SDK and dedicated support for Python. The kernel handles the interaction between these skills and drivers for functionality like inference and retrieval via the cognitive system interface. This enables to deploy RAG usecases serverless. Writing skills for the kernel is more constrained then shipping an end to end use case in a custom docker container. Yet these constrains allow us to make opinionated decisions for the skill developer. We strive to take away only the decisions and responsibilities a skill developer may find "boring" (such as authentication, parallelization of inference calls). In more technical terms we aim to reduce the accidental complexity the skill developer has to engage with.

## Contributing

In this repository we stick to Conventional commits. See: <https://www.conventionalcommits.org/en/v1.0.0/>.

### Release

Releasing in this repository is automated with [release-plz](https://release-plz.ieni.dev/).

1. For every commit to the `main` branch, release-plz creates a release Pull Request.
2. Review the release Pull Request and add new changes, if necessary, by amending the commit directly.
   - Helm chart version:

     By default, only the patch version is incremented.
   - Helm chart changelog date:

     The date is set when the changelog is generated. Update it to the release date if differs.
3. Merge the release Pull Request and release-plz will release the updated packages.

### Local Development setup

There are some prerequisites you need to install once

```shell
# We need the Wasm target to be able to compile the skills
rustup target add wasm32-wasip2
# We need wasm-tools to strip skill binaries
cargo install wasm-tools
```

Every time we update the `wit` worlds, we we need to clear the Skill build cache in `./skill_build_cache` which is used by the tests.

### Building and running the kernel container using Podman

Now, you can build the image with

```shell
podman build . --tag pharia-kernel
```

Then, run the image with

```shell
podman run -v ./config.toml:/app/config.toml -p 8081:8081 --env-file .env pharia-kernel
```

We configure the bind address and port via the environment variable `KERNEL_ADDRESS`.
If not configured it defaults to "0.0.0.0:8081", which is necessary in the container, but locally may cause the firewall to complain.

#### MacOS

Podman on MacOS requires a separate virtual machine run by the user. To compile PhariaKernel, at least 4 GiB of RAM are needed and 8 GiB are recommended. You set this up with

```shell
podman machine init
podman machine set --memory 8192
podman machine start
```

When building the image, you need to specify the target platform.

```shell
podman build . --tag pharia-kernel --platform linux/arm64
```

### curl commands

health check:

```shell
curl -v GET 127.0.0.1:8081/health
```

list skills:

```shell
curl -v GET 127.0.0.1:8081/skills
curl -v GET 127.0.0.1:8081/cached_skills
j
```

run a skill:

```shell
set -a; source .env
curl -v -X POST 127.0.0.1:8081/v1/skills/pharia-kernel-team/greet_skill/run \
-H "Authorization: Bearer $PHARIA_AI_TOKEN" \
-H 'Content-Type: application/json' \
-d '"Homer"'
```

## Deploying PhariaKernel on Customer side

**PhariaKernel** is intended to be installed **on premise** by the customer it. It is deployed, as are all other modules of the **PhariaOS**, to the JFrog Artifactory. Our colleagues at the PhariaOS Team are going to develop tooling for deploying tooling for rolling it out. Until they come up with a name it is here called "Pharia Up".

![Block Diagram Pharia OS deploy][deployment]

## Helpful links for internal deployment

The current prototype is deployed at <https://pharia-kernel.aleph-alpha.stackit.run>

The status page for uptime robot is found at: <https://stats.uptimerobot.com/gjXpoIPMnv>

The secrets for the deployment can be added to the [Vault](https://vault.management-prod01.stackit.run/ui/vault/secrets/c-aa01/list/projects/pharia-kernel/)

[deployment]: ./tam/deployment.drawio.svg

## Adding Python Bindings After Updating the WIT World

For testing, we have multiple Python skills with bindings to the WIT world.
If you have updated the WIT world and want to create new bindings run the following command (assuming you are in the folder of the skill):

```sh
componentize-py --all-features -d ../../wit/skill@0.3/skill.wit -w skill bindings --world-module skill .
```