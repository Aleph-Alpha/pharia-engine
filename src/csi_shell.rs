#[cfg(test)]
mod tests {
    use serde_json::json;

    /// This structs allows us to represent versioned interactions with the CSI.
    /// The members of this enum provide the glue code to translate between a function
    /// defined in a versioned wit world and the `CsiForSkills` trait.
    /// By introducing this abstraction, we can expose a versioned interface of the CSI over http.
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "snake_case", tag = "version")]
    pub enum VersionedCsiRequest {
        V0_2(V0_2CsiRequest),
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "snake_case", tag = "function")]
    pub enum V0_2CsiRequest {
        Complete(CompletionRequest),
    }

    #[derive(serde::Deserialize)]
    pub struct CompletionRequest {
        pub model: String,
        pub prompt: String,
        pub params: CompletionParams,
    }

    #[derive(serde::Deserialize)]
    pub struct CompletionParams {
        pub max_tokens: u32,
        pub temperature: Option<f32>,
        pub top_k: Option<u32>,
        pub top_p: Option<f32>,
        pub stop: Vec<String>,
    }

    #[test]
    fn csi_v_2_request_is_deserialized() {
        // Given a request in JSON format
        let request = json!({
            "version": "v0_2",
            "function": "complete",
            "prompt": "Hello",
            "model": "llama-3.1-8b-instruct",
            "params": {
                "max_tokens": 128,
                "temperature": null,
                "top_k": null,
                "top_p": null,
                "stop": []
            }
        });

        // When it is deserialized into a `VersionedCsiRequest`
        let result: Result<VersionedCsiRequest, serde_json::Error> =
            serde_json::from_value(request);

        // Then it should be deserialized successfully
        assert!(result.is_ok());
    }
}