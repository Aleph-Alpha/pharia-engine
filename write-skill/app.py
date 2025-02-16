import json

import skill.exports
from skill.exports.skill_handler import SkillMetadata
from skill.imports.inference import (
    ChatParams,
    ChatRequest,
    ChatStreamRequest,
    Logprobs_No,
    Message,
)
from skill.imports.response import write


class SkillHandler(skill.exports.SkillHandler):
    def run(self, input: bytes) -> bytes:
        request = ChatRequest(
            model="llama-3.1-8b-instruct",
            messages=[Message(role="user", content="Hello, how are you?")],
            params=ChatParams(
                max_tokens=None,
                temperature=0.0,
                top_p=None,
                frequency_penalty=0.0,
                presence_penalty=0.0,
                logprobs=Logprobs_No(),
            ),
        )
        with ChatStreamRequest(request) as stream:
            while (delta := stream.next()) is not None:
                write(
                    json.dumps({"role": delta.role, "content": delta.content}).encode()
                )
        return json.dumps("Hello final").encode()

    def metadata(self) -> SkillMetadata:
        return SkillMetadata(
            "Greet skill",
            json.dumps(
                {"type": "string", "description": "The name of the person to greet"}
            ).encode(),
            json.dumps(
                {"type": "string", "description": "A friendly greeting message"}
            ).encode(),
        )
