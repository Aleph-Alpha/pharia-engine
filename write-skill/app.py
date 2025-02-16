import json
from typing import Generator

import skill.exports
from skill.exports.skill_handler import SkillMetadata
from skill.imports.inference import (
    ChatParams,
    ChatRequest,
    ChatStreamRequest,
    Logprobs_No,
    Message,
    MessageDelta,
)
from skill.imports.response import write


def chat_stream(
    model: str, messages: list[Message], params: ChatParams
) -> Generator[MessageDelta, None, None]:
    """Stream chat responses as a generator function.
    
    A utility function that we could introduce in the SDK."""
    stream = ChatStreamRequest(ChatRequest(model, messages, params))
    while (item := stream.next()) is not None:
        yield item


def stream_skill(func):
    """A decorator that could be exposed similarly in the SDK.

    Allows for skills to yield bytes.
    """

    class SkillHandler(skill.exports.SkillHandler):
        def run(self, input: bytes) -> bytes:
            for delta in func(input):
                write(delta)

            # For now skills still need to return a value.
            # This requirement might go once we introduce a separate
            # stream skill type in the wit world.
            return b'""'

        def metadata(self) -> SkillMetadata:
            return SkillMetadata(
                "A skill that can yield.",
                json.dumps(
                    {"type": "string", "description": "The name of the person to greet"}
                ).encode(),
                json.dumps(
                    {"type": "string", "description": "A friendly greeting message"}
                ).encode(),
            )

    func.__globals__["SkillHandler"] = SkillHandler
    return func


@stream_skill
def my_skill(input: bytes) -> Generator[bytes, None, None]:
    """An example skill that can yield bytes.

    This skill can use Python native yield and does never need to know
    about the `write` function.
    """
    data = json.loads(input)
    content = data["content"]
    role = data["role"]
    model = "pharia-1-llm-7b-control"
    messages = [Message(role=role, content=content)]
    params = ChatParams(
        max_tokens=None,
        temperature=0.0,
        top_p=None,
        frequency_penalty=0.0,
        presence_penalty=0.0,
        logprobs=Logprobs_No(),
    )
    for delta in chat_stream(model, messages, params):
        yield json.dumps({"role": delta.role, "content": delta.content}).encode()
