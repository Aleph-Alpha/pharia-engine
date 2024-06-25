import skill
from skill.imports import csi


class Skill(skill.Skill):
    def run(self, in_: str) -> str:
        prompt = f"""### Instruction:
Provide a nice greeting for the person utilizing its given name

### Input:
Name: {in_}

### Response:"""

        return csi.complete_text(prompt, "luminous-nextgen-7b")
