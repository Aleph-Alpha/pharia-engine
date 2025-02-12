import json

import skill.exports
from skill.exports.skill_handler import SkillMetadata
from skill.imports.inference import CompletionParams, CompletionRequest, Logprobs_No, complete


class SkillHandler(skill.exports.SkillHandler):
    def run(self, input: bytes) -> bytes:
        input = json.loads(input)
        prompt = f"""<|begin_of_text|><|start_header_id|>system<|end_header_id|>

Cutting Knowledge Date: December 2023
Today Date: 23 Jul 2024

You are a helpful assistant.<|eot_id|><|start_header_id|>user<|end_header_id|>

Provide a nice greeting for the person named: {input}<|eot_id|><|start_header_id|>assistant<|end_header_id|>"""
        params = CompletionParams(10, None, None, None, [], False, None, None, Logprobs_No())
        completion = complete([CompletionRequest("pharia-1-llm-7b-control", prompt, params)])
        return json.dumps(completion[0].text).encode()

    def metadata(self) -> SkillMetadata:
        return SkillMetadata(
            "Greet skill",
            json.dumps({ "type": "string", "description": "The name of the person to greet" }).encode(),
            json.dumps({ "type": "string", "description": "A friendly greeting message" }).encode()
        )
