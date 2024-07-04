# Developing and deploying Skills

## Developing Skills

A Skill is a WebAssembly Component compiled against our WIT (**W**ebAssembly **I**nterface **T**ypes) we call the **C**ognitive **S**ystem **I**nterface (CSI). The good thing is that you can compile almost any language into WebAssembly. Here we explain only how it works for Python.

## Developing Skills in Python

This is a step-by-step instruction of how to write Skills. There is also an example repository at <https://gitlab.aleph-alpha.de/engineering/haiku-skill-python> for you to explore.

First you need to create the Python bindings for the Cognitive System Interface. In order to do this, copy this file to your directory. We call it `skill.wit`:

```wit
package pharia:skill;

world skill {
    import csi;
    export run: func(in: string) -> string;
}

interface csi {
    complete-text: func(prompt: string, model: string) -> string;
}
```

In order to create the bindings you need to install `componentize-py`. Choose the virtualization of your choice (conda, virtual env, ...) and pip install it.

```shell
pip install componentize-py
```

With `componentize-py` installed, we can now generate the bindings:

```shell
componentize-py -d skill.wit -w skill bindings .
```

This will create a `skill` module containing bindings to the Cognitive System Interface in the current directory. You can now import this module in your Python code. Here is an example of a Skill creating a haiku using the `skill` module.

```Python
import skill
from skill.imports import csi


class Skill(skill.Skill):
    def run(self, in_: str) -> str:
        prompt = f"""Write a haiku about {in_}

### Response:"""

        return csi.complete_text(prompt, "luminous-extended-control")
```

Now that we have written our Skill, we need to compile it into a component.

```shell
componentize-py -d skill.wit -w skill componentize haiku -o ./haiku.wasm
```

This will create a file called `haiku.wasm`, which can now be deployed into Pharia Kernel.

## Deploying Skills to the Kernel

Skills are not containers. Yet, we still publish them into container repositories. Ask your administrator which ones are linked to your instance of Pharia Kernel. At Aleph Alpha we are currently using the registry `registry.gitlab.aleph-alpha.de` and the repository `engineering/pharia-kernel/skills`. It is recommended to publish the Skill using the `pharia-skill` command line tool. `pharia-skill` is deployed as a container image to our Artifactory. You can acquire it with Podman like this.

```shell
podman login alephalpha.jfrog.io/pharia-kernel-images -u $JFROG_USER -p $JFROG_PASSWORD
podman pull alephalpha.jfrog.io/pharia-kernel-images/pharia-skill:latest
podman tag alephalpha.jfrog.io/pharia-kernel-images/pharia-skill:latest pharia-skill
```

Feel free to use `docker` instead, if you are more familiar with that tooling.

With the tooling available we can now upload the Skill.

```shell
podman run -v ./haiku.wasm:/haiku.wasm pharia-skill publish -R registry.gitlab.aleph-alpha.de -r engineering/pharia-kernel/skills -u DUMMY_USER_NAME -p $GITLAB_TOKEN ./haiku.wasm
```

With our Gitlab registry, any user name will work, as long as you use a token. You can generate a token on your profile page. It is important to give write privilege.

Congratulations! Your Skill is now deployed. You can now use the HTTP API to call it.
