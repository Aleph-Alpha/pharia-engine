package main

import (
	"fmt"

	api "gitlab.aleph-alpha.de/engineering/pharia-kernel/greet-go/api"
)

func init() {
	a := SkillImpl{}
	api.SetSkill(a)
}

type SkillImpl struct {
}

func (e SkillImpl) Run(in string) string {
	prompt := fmt.Sprintf(`### Instruction:
Provide a nice greeting for the person utilizing its given name

### Input:
Name: %s

### Response:`, in)
	return api.PhariaSkillCsiCompleteText(prompt, "luminous-nextgen-7b")
}

//go:generate wit-bindgen tiny-go ../wit --out-dir=api
func main() {}
