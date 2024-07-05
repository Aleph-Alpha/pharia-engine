# Operating Pharia Kernel

This manual is for Operators running Pharia Kernel for their buisness or department. In case you are more interessted in developing Skills or interfacing with Pharia Kernel in order to invoke deployed skills you should consider the developer or user manual instead.

## How to get it

We deploy Pharia Kernel as a container image to JFrog. You can fetch them like this:

```shell
podman login alephalpha.jfrog.io/pharia-kernel-images
podman pull alephalpha.jfrog.io/pharia-kernel-images/pharia-kernel:latest
podman tag alephalpha.jfrog.io/pharia-kernel-images/pharia-kernel:latest pharia-kernel
```

## Starting

You can start the container and expose its shell at port 8081 like this

```shell
podman run -p 8081:8081 pharia-kernel
```

## Deploying Skills

Pharia Kernel will automatically serve any Skill deployed at the `skills` subdirectory of its working directory. However, this is mostly intended for local development of Skills without a remote instance of Pharia Kernel. To deploy skills in production it is recommended to use a container registry. At least if you want to deploy them dynamically. Pharia Kernel will look at the following Enviroment variables (here with example):

```shell
SKILL_REGISTRY_USER=Joe.Plumber
SKILL_REGISTRY_PASSWORD=****
SKILL_REGISTRY=registry.gitlab.aleph-alpha.de
SKILL_REPOSITORY=engineering/pharia-skills/skills
```

Skills we be loaded lazily if invoked.


## Logging

By default, only logs of `ERROR` level are output. You can change this by setting the `PHARIA_KERNEL_LOG` environment variable to one of `debug`, `info`, `warn`, or `error` (default).

```shell
PHARIA_KERNEL_LOG=info
```

## Using on-premise inference

By default the kernel uses the Aleph Alpha SAAS [inference API](https://api.aleph-alpha.com). You can change this by setting the `INFERENCE_ADDRESS` environment variable.

```shell
INFERENCE_ADDRESS=https://inference.acme.com
```
