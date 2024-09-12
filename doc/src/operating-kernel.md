# Operating Pharia Kernel

This manual is for Operators running Pharia Kernel for their business or department. In case you are more interested in developing Skills or interfacing with Pharia Kernel in order to invoke deployed skills you should consider the developer or user manual instead.

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

Pharia Kernel wants to enable teams outside of the Pharia Kernel Operators to deploy skills in self service. To achieve this, skills are organized into namespaces. The Pharia Kernel Operators maintain the list of namespaces and associate each one with a configuration file, which in turn is owned by a team.

Each namespace configuration typically would reside in a Git repository owned by the Team which owns the namespace. Changes in this file will be automatically detected by the Kernel.

A namespace is also associated with a registry to load the skills from. These skill registries can either be directories in filesystem (mostly used for a development setup) or point to an OCI registry (recommended for production).

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
# The URL to the configuration listing the skills of this namespace
config_url = "https://gitlab.aleph-alpha.de/api/v4/projects/966/repository/files/config.toml/raw?ref=main"
# Pharia kernel will use the contents of this environment variable to access (authorize) the above URL
config_access_token_env_var = "GITLAB_CONFIG_ACCESS_TOKEN"
# Registry to load skills from
registry = { type = "oci", registry = "registry.gitlab.aleph-alpha.de", repository = "engineering/pharia-skills/skills" }
```

With the remote configuration above, Pharia Kernel will serve any skill deployed on the specified OCI registry under the namespace "my-team".

### Authentication against OCI Registries

Currently Pharia Kernel uses the same credentials to authenticate against all OCI registries. To set these credentials use these environment variables:

```shell
SKILL_REGISTRY_USER=Joe.Plumber
SKILL_REGISTRY_PASSWORD=****
```

## Update interval 

The environment variable `NAMESPACE_UPDATE_INTERVAL` controls how much time the kernel waits between checking for changes in namespace configurations. The default is 10 seconds.

```shell
NAMESPACE_UPDATE_INTERVAL=10s
```

## Logging

By default, only logs of `ERROR` level are output. You can change this by setting the `LOG_LEVEL` environment variable to one of `trace`, `debug`, `info`, `warn`, or `error` (default).

```shell
LOG_LEVEL=info
```


## Observability

Pharia Kernel can be configured to use an OpenTelemetry Collector endpoint by setting the `OPEN_TELEMETRY_ENDPOINT` environment variable.

```shell
OPEN_TELEMETRY_ENDPOINT=http://127.0.0.1:4317
```

For local testing, a supported collector like the Jaeger [All in One](https://www.jaegertracing.io/docs/1.60/getting-started/#all-in-one) executable can be used:

```shell
podman run -d -p4317:4317 -p16686:16686 jaegertracing/all-in-one
```
