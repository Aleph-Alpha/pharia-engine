use crate::inference::{CompleteTextParameters, InferenceApi};

pub trait Runtime {
    fn new() -> Self;
    async fn run_greet(
        &self,
        name: String,
        api_token: String,
        inference_api: &mut InferenceApi,
    ) -> String;
}

pub struct RustRuntime {}

impl Runtime for RustRuntime {
    fn new() -> Self {
        Self {}
    }
    async fn run_greet(
        &self,
        name: String,
        api_token: String,
        inference_api: &mut InferenceApi,
    ) -> String {
        let prompt = format!(
            "### Instruction:
                Provide a nice greeting for the person utilizing its given name

                ### Input:
                Name: {name}

                ### Response:"
        );
        let params = CompleteTextParameters {
            prompt,
            model: "luminous-nextgen-7b".to_owned(),
            max_tokens: 10,
        };
        inference_api.complete_text(params, api_token).await
    }
}
