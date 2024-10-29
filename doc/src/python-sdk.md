# Developing Skills in Python

A Python SDK is provided to further ease your skill development.

## Installing SDK

The SDK is distributed via the Aleph Alpha Artifactory PyPI. You may need to request a JFrog account, or extend the permission of your JFrog account via the [Product Service Desk](https://aleph-alpha.atlassian.net/servicedesk/customer/portals).

```shell
python -m venv .venv
source .venv/bin/activate
pip install --extra-index-url https://alephalpha.jfrog.io/artifactory/api/pypi/python/simple pharia-kernel-sdk-py
```

## Developing Skills

The `pharia_skill` package provides a decorator to support skill development.
The decorator inserts the Cognitive System Interface (CSI), which always need to be specified as the first argument.
As an example, we write a new skill in `haiku.py`.

```python
# haiku.py
from pharia_skill import CompletionParams, Csi, skill
from pydantic import BaseModel


class Input(BaseModel):
    topic: str


@skill
def haiku(csi: Csi, input: Input) -> str:
    prompt = f"""<|begin_of_text|><|start_header_id|>system<|end_header_id|>

    You are a poet who strictly speaks in haikus.<|eot_id|><|start_header_id|>user<|end_header_id|>

    {input.topic}<|eot_id|><|start_header_id|>assistant<|end_header_id|>"""
    params = CompletionParams(max_tokens=64)
    completion = csi.complete("llama-3.1-8b-instruct", prompt, params)
    return completion.text.strip()
```

### Testing

The `@skill` annotation does not modify the annotated function, which allows the test code to inject different variants of CSI.
The `testing` module provides two implementations of CSI for testing:

- The `DevCsi` can be used for testing Skill code locally against a running Pharia Kernel. See the docstring for how to set it up.
- The `StubCsi` can be used as a base class for mock implementation.

## Building Skills

When building the skills, the wheels that include native dependencies need to be provided additionally for Wasm. For example, the skill SDK has a dependency on `pydantic` v2.5.2.

### Download Pydantic WASI wheels

Supported Pydantic versions:

```toml
pydantic-core = "2.14.5"
pydantic = "2.5.2"
```

Download and unpack WASI wheels (without installation):

```shell
mkdir wasi_deps
cd wasi_deps
curl -OL https://github.com/dicej/wasi-wheels/releases/download/latest/pydantic_core-wasi.tar.gz
tar xf pydantic_core-wasi.tar.gz
cd ..
```

### Compiling Skill to Wasm

You now create the file `haiku.wasm`, ready to be uploaded.

```shell
pip install componentize-py
componentize-py -w skill componentize haiku -o ./haiku.wasm -p . -p wasi_deps
```
