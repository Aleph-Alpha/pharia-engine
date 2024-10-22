"""
A Pharia Kernel Python skill that will not hit inference, but is only used to evaluate
scalability of Pharia Kernel with regards to memory consumption and latency.
Skill invocation may optionally take wait time and consume memory.
"""

import time

from pharia_skill import Csi, skill
from pydantic import BaseModel


class Input(BaseModel):
    # a message to show, prepended by Hello and appended by resource information
    topic: str
    # in KBytes
    memory: int | None = None
    # in milliseconds, just io - cpu won't be a problem as inference runs elsewhere
    wait_time: int | None = None


@skill
def consume(csi: Csi, input: Input) -> str:
    "a python skill without hitting inference, to check resource consumption"
    msg = f"Hello {input.topic}"
    if input.memory:
        buffer = bytearray(input.memory * 1024)
        buffer[0] = 0
        msg += f"  memory={input.memory:06} KB"
    if input.wait_time:
        time.sleep(input.wait_time / 1000)
        msg += f" time={input.wait_time:06} ms"
    return msg
