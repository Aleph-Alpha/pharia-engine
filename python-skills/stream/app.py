import json
from typing import Generator

import stream_skill.exports
from stream_skill.exports.stream_skill_handler import StreamSkillMetadata
from stream_skill.imports.chat_response import MessageDelta as OutputMessageDelta
from stream_skill.imports.chat_response import write_stream_event
from stream_skill.imports.inference import (
    ChatParams,
    ChatRequest,
    ChatStreamRequest,
    Logprobs_No,
    Message,
    MessageDelta,
)


def csi_chat_stream(
    model: str, messages: list[Message], params: ChatParams
) -> Generator[MessageDelta, None, None]:
    """Stream chat responses as a generator function.

    A utility function that we could introduce in the SDK."""
    stream = ChatStreamRequest(ChatRequest(model, messages, params))
    while (item := stream.next()) is not None:
        yield item


def chat_skill(func):
    """A decorator that could be exposed similarly in the SDK.

    Allows for skills to yield bytes.
    """

    class StreamSkillHandler(stream_skill.exports.StreamSkillHandler):
        def run(self, input: bytes):
            for delta in func(input):
                output_delta = OutputMessageDelta(
                    role=delta.role, content=delta.content
                )
                write_stream_event(output_delta)

        def metadata(self) -> StreamSkillMetadata:
            return StreamSkillMetadata(
                "A skill that can yield.",
                json.dumps(
                    {"type": "string", "description": "The name of the person to greet"}
                ).encode(),
                json.dumps(
                    {"type": "string", "description": "A friendly greeting message"}
                ).encode(),
            )

    func.__globals__["StreamSkillHandler"] = StreamSkillHandler
    return func


@chat_skill
def my_skill(input: bytes) -> Generator[MessageDelta, None, None]:
    """An example skill that can yield bytes.

    This skill can use Python native yield and does never need to know
    about the `write` function.
    """
    data = json.loads(input)
    model = "pharia-1-llm-7b-control"
    messages = [
        Message(role=message["role"], content=message["content"])
        for message in data["messages"]
    ]
    params = ChatParams(
        max_tokens=None,
        temperature=0.0,
        top_p=None,
        frequency_penalty=0.0,
        presence_penalty=0.0,
        logprobs=Logprobs_No(),
    )
    # A user may either return the generator directly, or loop through it and yield individual outputs himself.
    return csi_chat_stream(model, messages, params)
