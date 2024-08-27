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

Pharia Kernel organizes skills in namespaces. A namespace is a unique identifier which has an associated configuration file specifying deployed skills and an associated registry. Skills will be loaded lazily if invoked. The configuration file and the registry can be local or remote in any combination:

### Namespace with Local Config and Local Registry

```toml
[namespaces.local]
config_url = "file://namespace.toml"
registry = { type = "file", path = "skills" }
```

With the local configuration above, Pharia Kernel will serve any skill deployed at the `skills` subdirectory of its working directory under the namespace "local". This is mostly intended for local development of skills without a remote instance of Pharia Kernel. To deploy skills in production it is recommended to use a remote namespace. 

### Namespace with Remote Config and Remote Registry

```toml
[namespaces.my-team]
config_url = "https://gitlab.aleph-alpha.de/api/v4/projects/966/repository/files/config.toml/raw?ref=main"
registry = { type = "oci", registry = "registry.gitlab.aleph-alpha.de", repository = "engineering/pharia-skills/skills" }
```

With the remote configuration above, Pharia Kernel will serve any skill deployed on the specified OCI registry under the namespace "my-team". If authentication is required, then a universal token can be specified with the following environment variables:

```shell
SKILL_REGISTRY_USER=Joe.Plumber
SKILL_REGISTRY_PASSWORD=****
```

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
