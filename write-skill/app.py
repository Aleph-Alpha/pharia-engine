import json

import skill.exports
from skill.exports.skill_handler import SkillMetadata
from skill.imports.response import write


class SkillHandler(skill.exports.SkillHandler):
    def run(self, input: bytes) -> bytes:
        write(b'"Hello 1"')
        # non-valid json
        write(b"Hello 2")
        write(b'"Hello 3"')
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
