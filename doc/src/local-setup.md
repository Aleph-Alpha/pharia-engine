
# Local Pharia Kernel setup

## Get Pharia Kernel image

1. Access JFrog Artifactory via token:
    * Login to [JFrog](https://alephalpha.jfrog.io/ui/login/)
    * Click on 'Edit Profile'
    * Click on 'Generate an Identity Token'

2. Pull Pharia Kernel image

```shell
# login in interactive mode
podman login alephalpha.jfrog.io/pharia-kernel-images
# login in non-interactive mode
podman login alephalpha.jfrog.io/pharia-kernel-images -u $JFROG_USER -p $JFROG_PASSWORD

podman pull alephalpha.jfrog.io/pharia-kernel-images/pharia-kernel:latest
podman tag alephalpha.jfrog.io/pharia-kernel-images/pharia-kernel:latest pharia-kernel
```

## Start Pharia Kernel container

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

## Build your skill

1. Set up a virtual Python environment:

    ```shell
        # fetch the WIT world
        curl http://127.0.0.1:8081/skill.wit > skill.wit

        # create python venv
        python -m venv .venv
        source .venv/bin/activate
        pip install componentize-py

        # create the wit bindings
        componentize-py -d skill.wit -w skill bindings .
    ```

2. Write your skill `my_skill.py`:

    a. Simple 'hello world' example:

    ```python
    import skill.exports
    import json

    class SkillHandler(skill.exports.SkillHandler):
        def run(self, input: bytes) -> bytes:
            input = json.loads(input)
            return json.dumps("Hello " + input).encode()
    ```

    b. Example that uses the CSI:

    ```python
        import skill.exports
        from skill.imports import csi
        import json

        class SkillHandler(skill.exports.SkillHandler):
            def run(self, input: bytes) -> bytes:
                input = json.loads(input)
                prompt = f"""<|begin_of_text|><|start_header_id|>system<|end_header_id|>

        Cutting Knowledge Date: December 2023
        Today Date: 23 Jul 2024

        You are a helpful assistant.<|eot_id|><|start_header_id|>user<|end_header_id|>

        Provide a nice greeting for the person named: {input}<|eot_id|><|start_header_id|>assistant<|end_header_id|>"""
                params = csi.CompletionParams(10, None, None, None, [])
                completion = csi.complete( "llama-3.1-8b-instruct", prompt, params)
                return json.dumps(completion.text).encode()
    ```

3. Compile your skill:

    ```shell
        componentize-py -d skill.wit -w skill componentize my_skill -o ./skills/my_skill.wasm
    ```

4. Execute your skill:

    ```shell
        curl -v -X POST 127.0.0.1:8081/execute_skill \
            -H "Authorization: Bearer $PHARIA_AI_TOKEN" \
            -H 'Content-Type: application/json' \
            -d '{"skill":"dev/my_skill", "input":"Homer"}'
    ```

5. Iterate

    Whenever the skill code is changed, you have to compile it again. The new skill version will be picked up by the Pharia Kernel.

    ```shell
        # compile the skill
        componentize-py -d skill.wit -w skill componentize my_skill -o ./skills/my_skill.wasm
    ```

## Monitoring local skill execution

You can monitor your skill by connecting the Pharia Kernel to an OpenTelemetry collector, e.g. Jaeger:

```shell
    podman run -d -p 4317:4317 -p 16686:16686 jaegertracing/all-in-one
```

Specify the collector endpoint via the environment variable `OPEN_TELEMETRY_ENDPOINT`:

```shell
    podman run \
        -v ./skills:/app/skills \
        -e NAMESPACES__DEV__DIRECTORY="skills" \
        -e NAMESPACE_UPDATE_INTERVAL=1s \
        -e OPEN_TELEMETRY_ENDPOINT=http://host.containers.internal:4317 \
        -p 8081:8081 \
        pharia-kernel
```

You can view the monitoring via your local Jaeger instance at <http://localhost:16686>.
