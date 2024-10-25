"""
A Pharia Kernel Python skill that will not hit inference, but is only used to evaluate
scalability of Pharia Kernel with regards to memory consumption and latency.
Skill invocation may optionally take wait time and consume memory.
"""

import random
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


def consume_memory(memory: int):
    "consumes small memory chunks if memory is larger than 300 KB, chunk size is between 100 and 300 KB"
    if memory < 300:
        return [bytearray(memory * 1024)]
    chunks = []
    while memory > 300:
        chunk_size = random.randint(100, 300)
        chunk = bytearray(chunk_size * 1024)
        chunk[0] = 0
        chunks.append(chunk)
        memory -= chunk_size
    if memory > 0:
        chunk = bytearray(memory)
        chunk[0] = 0
        chunks.append(chunk)
    return chunks


@skill
def consume(csi: Csi, input: Input) -> str:
    "a python skill without hitting inference, to check resource consumption"
    msg = f"Hello {input.topic}"
    if input.memory:
        buffers = consume_memory(input.memory)
        msg += f"  memory={input.memory:06} KB in {len(buffers)} chunks"
    if input.wait_time:
        time.sleep(input.wait_time / 1000)
        msg += f" time={input.wait_time:06} ms"
    return msg
