import skill
from skill.imports import csi


class Skill(skill.Skill):
    def run(self, in_: str) -> str:
        prompt = f"""<|begin_of_text|><|start_header_id|>system<|end_header_id|>

Cutting Knowledge Date: December 2023
Today Date: 23 Jul 2024

You are a helpful assistant.<|eot_id|><|start_header_id|>user<|end_header_id|>

Provide a nice greeting for the person named: {in_}<|eot_id|><|start_header_id|>assistant<|end_header_id|>"""

        return csi.complete_text(prompt, "pharia-1-llm-7b-control")
