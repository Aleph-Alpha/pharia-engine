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

A [namespace](skill-deployment.md#configuring-namespace) is also associated with a registry to load the skills from. These skill registries can either be directories in filesystem (mostly used for a development setup) or point to an OCI registry (recommended for production).

### Namespace with Local Config and Local Registry

```toml
# `my-team` is the name of this namespace
[namespaces.my-team]
# Path to the local namespace configuration file
config-url = "file://namespace.toml"

# Path to the local skill directory
path = "skills"
```

With the local configuration above, Pharia Kernel will serve any skill deployed at the `skills` subdirectory of its working directory under the namespace `my-team`. This is mostly intended for local development of skills without a remote instance of Pharia Kernel. To deploy skills in production it is recommended to use a remote namespace.

### Namespace with Remote Config and Remote Registry

```toml
# `my-team` is the name of this namespace
[namespaces.my-team]
# The URL to the configuration listing the skills of this namespace
# Pharia kernel will use the contents of the `NAMESPACES__MY_TEAM__CONFIG_ACCESS_TOKEN` environment variable to access (authorize) the config
config-url = "https://github.com/Aleph-Alpha/my-team/blob/main/config.toml"

# OCI Registry to load skills from
# Pharia kernel will use the contents of the `NAMESPACES__MY_TEAM__REGISTRY_USER` and `NAMESPACES__MY_TEAM__REGISTRY_PASSWORD` environment variables to access (authorize) the registry
registry = "registry.acme.com"

# This is the common prefix added to the skill name when composing the OCI repository.
# e.g. ${base-repository}/${skill_name}
base-repository = "my-org/my-team/skills"
```

With the remote configuration above, Pharia Kernel will serve any skill deployed on the specified OCI registry under the namespace `my-team`.

### Authentication against OCI Registries

You can provide each namespace in Pharia Kernel with credentials to authenticate against the specified OCI registry. Set the environment variables that are expected from the operator config:

```shell
NAMESPACES__MY_TEAM__REGISTRY_USER=Joe.Plumber
NAMESPACES__MY_TEAM__REGISTRY_PASSWORD=****
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

Pharia Kernel can be configured to use an OpenTelemetry Collector endpoint by setting the `OTEL_ENDPOINT` environment variable.

```shell
OTEL_ENDPOINT=http://127.0.0.1:4317
```

For local testing, a supported collector like the Jaeger [All in One](https://www.jaegertracing.io/docs/1.60/getting-started/#all-in-one) executable can be used:

```shell
podman run -d -p4317:4317 -p16686:16686 jaegertracing/all-in-one
```

## Local Pharia Kernel setup

### Get Pharia Kernel image

1. Access JFrog Artifactory via token:

   - Login to [JFrog](https://alephalpha.jfrog.io/ui/login/)
   - Click on 'Edit Profile'
   - Click on 'Generate an Identity Token'

2. Pull Pharia Kernel image

```shell
# login in interactive mode
podman login alephalpha.jfrog.io/pharia-kernel-images
# login in non-interactive mode
podman login alephalpha.jfrog.io/pharia-kernel-images -u $JFROG_USER -p $JFROG_PASSWORD

podman pull alephalpha.jfrog.io/pharia-kernel-images/pharia-kernel:latest
podman tag alephalpha.jfrog.io/pharia-kernel-images/pharia-kernel:latest pharia-kernel
```

### Start Pharia Kernel container

In order to run Pharia Kernel, you need to provide a namespace configuration:

1. Create a `skills` folder

   For local skill development, you need a folder that serves as a skill registry to store all compiled skills.

   ```shell
       # create the local skills folder
       mkdir skills
   ```

   All skills in this folder are exposed in the namespace "dev" with the environment variable `NAMESPACES__DEV__DIRECTORY`.
   Any changes in this folder will be picked up by the Pharia Kernel automatically. The `config.toml` and `namespace.toml` should not be provided.

2. Start the container:

   ```shell
       podman run \
           -v ./skills:/app/skills \
           -e NAMESPACES__DEV__DIRECTORY="skills" \
           -e NAMESPACE_UPDATE_INTERVAL=1s \
           -e LOG_LEVEL="pharia_kernel=debug" \
           -p 8081:8081 \
           pharia-kernel
   ```

   You can view the Pharia-Kernel's API documentation at <http://127.0.0.1:8081/api-docs>

### Monitoring local skill execution

You can monitor your skill by connecting the Pharia Kernel to an OpenTelemetry collector, e.g. Jaeger:

```shell
    podman run -d -p 4317:4317 -p 16686:16686 jaegertracing/all-in-one
```

Specify the collector endpoint via the environment variable `OTEL_ENDPOINT`:

```shell
    podman run \
        -v ./skills:/app/skills \
        -e NAMESPACES__DEV__DIRECTORY="skills" \
        -e NAMESPACE_UPDATE_INTERVAL=1s \
        -e OTEL_ENDPOINT=http://host.containers.internal:4317 \
        -p 8081:8081 \
        pharia-kernel
```

You can view the monitoring via your local Jaeger instance at <http://localhost:16686>.
