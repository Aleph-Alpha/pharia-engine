# Operating Pharia Kernel

Pharia Kernel allows you to execute Cognitive Business Units called skills. These Skill can be written in a number of languages, including Python. The kernel handles the interaction between these skills and drivers for functionality like inference and retrival via the cognitive system interface. This enables to deploy RAG usecases serverless.

## How to get it

We deploy Pharia Kernel as a Container image to JFrog. You can fetch them like this:

```shell
podman login alephalpha.jfrog.io/pharia-kernel-images
podman pull alephalpha.jfrog.io/pharia-kernel-images/pharia-kernel:latest
podman tag alephalpha.jfrog.io/pharia-kernel-images/pharia-kernel:latest pharia-kernel
```

## Starting

You can start the container and expose its shell at 8081 like this

```shell
podman run -p 8081:8081 pharia-kernel
```

## Deploying Skills

Pharia Kernel will automatically serve any skill deployed at the `skills` subdirectory of its working directory. However, this is mostly intended for local development of Skills without a remote instance of Pharia Kernel. To deploy skills in production it is recommened to use a container registry. At least if you want to deploy them dynamically. Pharia Kernel will look at the following Enviroment variables (here with example):

```shell
SKILL_REGISTRY_USER=Joe.Plumber
SKILL_REGISTRY_PASSWORD=****
SKILL_REGISTRY=registry.gitlab.aleph-alpha.de
SKILL_REPOSITORY=engineering/pharia-kernel/skills
```

Skills we be loaded lazily if invoked.
