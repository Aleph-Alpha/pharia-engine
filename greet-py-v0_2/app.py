import json

import skill.exports
from skill.imports import csi


class SkillHandler(skill.exports.SkillHandler):
    def run(self, input: bytes) -> bytes:
        input = json.loads(input)
        prompt = f"""<|begin_of_text|><|start_header_id|>system<|end_header_id|>

Cutting Knowledge Date: December 2023
Today Date: 23 Jul 2024

You are a helpful assistant.<|eot_id|><|start_header_id|>user<|end_header_id|>

Provide a nice greeting for the person named: {input}<|eot_id|><|start_header_id|>assistant<|end_header_id|>"""
        params = csi.CompletionParams(10, None, None, None, [])
        completion = csi.complete("llama-3.1-8b-instruct", prompt, params)
        return json.dumps(completion.text).encode()
