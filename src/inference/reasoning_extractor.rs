use crate::inference::ChatEventV2;

/// Extracts the reasoning content from a stream of chat events.
///
/// With the 0.4 WIT world we introduced concept of reasoning to chat responses.
/// However AA inference API has not stabilized this feature yet.
/// To not block the release of the 0.4 WIT world we temporarily implement the missing parsing
/// logic until AA Inference API v2 is publicly available.
pub struct ReasoningExtractor {
    is_thinking: bool,
}

impl ReasoningExtractor {
    const START_TAG: &str = "<think>";
    const END_TAG: &str = "</think>";

    pub fn new() -> Self {
        Self { is_thinking: false }
    }

    pub fn extract(&mut self, event: ChatEventV2) -> Vec<ChatEventV2> {
        match (&event, self.is_thinking) {
            (ChatEventV2::MessageAppend { content, .. }, false) => {
                let contains_reasoning = content.starts_with(Self::START_TAG);
                if contains_reasoning && let Some(reasoning_ends) = content.find(Self::END_TAG) {
                    let reasoning_part = content[Self::START_TAG.len()..reasoning_ends].to_owned();
                    let content_part = content[reasoning_ends + Self::END_TAG.len()..].to_owned();
                    let mut events = vec![];
                    if !reasoning_part.is_empty() {
                        events.push(ChatEventV2::reasoning(reasoning_part));
                    }
                    if !content_part.is_empty() {
                        events.push(ChatEventV2::content(content_part));
                    }
                    events
                } else if contains_reasoning {
                    self.is_thinking = true;
                    let reasoning_part = content[Self::START_TAG.len()..].to_owned();
                    if reasoning_part.is_empty() {
                        vec![]
                    } else {
                        vec![ChatEventV2::reasoning(reasoning_part)]
                    }
                } else {
                    vec![event]
                }
            }
            (ChatEventV2::MessageAppend { content, .. }, true) => {
                if let Some(reasoning_ends) = content.find(Self::END_TAG) {
                    self.is_thinking = false;
                    let reasoning_part = content[..reasoning_ends].to_owned();
                    let content_part = content[reasoning_ends + Self::END_TAG.len()..].to_owned();
                    let mut events = vec![];

                    if !reasoning_part.is_empty() {
                        events.push(ChatEventV2::reasoning(reasoning_part));
                    }
                    if !content_part.is_empty() {
                        events.push(ChatEventV2::content(content_part));
                    }
                    events
                } else {
                    vec![ChatEventV2::reasoning(content.to_owned())]
                }
            }
            _ => vec![event],
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::inference::{ChatEventV2, FinishReason, reasoning_extractor::ReasoningExtractor};

    impl ChatEventV2 {
        fn begin() -> Self {
            Self::MessageBegin {
                role: "assistant".to_owned(),
            }
        }

        fn end() -> Self {
            Self::MessageEnd {
                finish_reason: FinishReason::Stop,
            }
        }
    }

    #[test]
    fn stream_split_on_message_with_chunk_having_reasoning_and_content() {
        // Given a message where no think tag is present in the content
        let stream = vec![
            ChatEventV2::begin(),
            ChatEventV2::content("<think>I am "),
            ChatEventV2::content("thinking...</think>The ans"),
            ChatEventV2::content("wer is 42"),
            ChatEventV2::end(),
        ];
        let mut extractor = ReasoningExtractor::new();

        // When proceessing the message
        let mut stream_transformed: Vec<ChatEventV2> = vec![];
        for event in &stream {
            stream_transformed.extend(extractor.extract(event.clone()));
        }

        // Then the reasoning content is not set
        let stream_expected = vec![
            ChatEventV2::begin(),
            ChatEventV2::reasoning("I am "),
            ChatEventV2::reasoning("thinking..."),
            ChatEventV2::content("The ans"),
            ChatEventV2::content("wer is 42"),
            ChatEventV2::end(),
        ];
        assert_eq!(stream_transformed, stream_expected);
    }

    #[test]
    fn stream_split_content_and_reasoning_content_on_message_with_think_tag_splits() {
        // Given a message where no think tag is present in the content
        let stream = vec![
            ChatEventV2::begin(),
            ChatEventV2::content("<think>I am "),
            ChatEventV2::content("thinking...</think>"),
            ChatEventV2::content("The answer is 42"),
            ChatEventV2::end(),
        ];
        let mut extractor = ReasoningExtractor::new();

        // When proceessing the message
        let mut stream_transformed: Vec<ChatEventV2> = vec![];
        for event in &stream {
            stream_transformed.extend(extractor.extract(event.clone()));
        }

        // Then the reasoning content is not set
        let stream_expected = vec![
            ChatEventV2::begin(),
            ChatEventV2::reasoning("I am "),
            ChatEventV2::reasoning("thinking..."),
            ChatEventV2::content("The answer is 42"),
            ChatEventV2::end(),
        ];
        assert_eq!(stream_transformed, stream_expected);
    }

    #[test]
    fn stream_split_opening_think_tag_in_own_chunk() {
        // Given a message where no think tag is present in the content
        let stream = vec![
            ChatEventV2::begin(),
            ChatEventV2::content("<think>"),
            ChatEventV2::content("I am "),
            ChatEventV2::content("thinking...</think>"),
            ChatEventV2::content("The answer is 42"),
            ChatEventV2::end(),
        ];
        let mut extractor = ReasoningExtractor::new();

        // When proceessing the message
        let mut stream_transformed: Vec<ChatEventV2> = vec![];
        for event in &stream {
            stream_transformed.extend(extractor.extract(event.clone()));
        }

        // Then the reasoning content is not set
        let stream_expected = vec![
            ChatEventV2::begin(),
            ChatEventV2::reasoning("I am "),
            ChatEventV2::reasoning("thinking..."),
            ChatEventV2::content("The answer is 42"),
            ChatEventV2::end(),
        ];
        assert_eq!(stream_transformed, stream_expected);
    }

    #[test]
    fn stream_split_closing_think_tag_in_own_chunk() {
        // Given a message where no think tag is present in the content
        let stream = vec![
            ChatEventV2::begin(),
            ChatEventV2::content("<think>I am "),
            ChatEventV2::content("thinking..."),
            ChatEventV2::content("</think>"),
            ChatEventV2::content("The answer is 42"),
            ChatEventV2::end(),
        ];
        let mut extractor = ReasoningExtractor::new();

        // When proceessing the message
        let mut stream_transformed: Vec<ChatEventV2> = vec![];
        for event in &stream {
            stream_transformed.extend(extractor.extract(event.clone()));
        }

        // Then the reasoning content is not set
        let stream_expected = vec![
            ChatEventV2::begin(),
            ChatEventV2::reasoning("I am "),
            ChatEventV2::reasoning("thinking..."),
            ChatEventV2::content("The answer is 42"),
            ChatEventV2::end(),
        ];
        assert_eq!(stream_transformed, stream_expected);
    }
    #[test]
    fn stream_split_opening_and_closing_think_tag_in_own_chunk() {
        // Given a message where no think tag is present in the content
        let stream = vec![
            ChatEventV2::begin(),
            ChatEventV2::content("<think>I am thinking...</think> The"),
            ChatEventV2::content(" answer is 42"),
            ChatEventV2::end(),
        ];
        let mut extractor = ReasoningExtractor::new();

        // When proceessing the message
        let mut stream_transformed: Vec<ChatEventV2> = vec![];
        for event in &stream {
            stream_transformed.extend(extractor.extract(event.clone()));
        }

        // Then the reasoning content is not set
        let stream_expected = vec![
            ChatEventV2::begin(),
            ChatEventV2::reasoning("I am thinking..."),
            ChatEventV2::content(" The"),
            ChatEventV2::content(" answer is 42"),
            ChatEventV2::end(),
        ];
        assert_eq!(stream_transformed, stream_expected);
    }
    #[test]
    fn stream_split_content_and_reasoning_content_on_message_with_think_tag_inside_splits() {
        // Given a message where no think tag is present in the content
        let stream = vec![
            ChatEventV2::begin(),
            ChatEventV2::content("Hello, <think>I am "),
            ChatEventV2::content("thinking...</think>"),
            ChatEventV2::content("The answer is 42"),
            ChatEventV2::end(),
        ];
        let mut extractor = ReasoningExtractor::new();

        // When proceessing the message
        let mut stream_transformed: Vec<ChatEventV2> = vec![];
        for event in &stream {
            stream_transformed.extend(extractor.extract(event.clone()));
        }

        // Then the reasoning content is not set
        assert_eq!(stream_transformed, stream);
    }

    #[test]
    fn stream_split_content_and_reasoning_content_has_no_effect_without_think_tag() {
        // Given a message where no think tag is present in the content
        let stream = vec![
            ChatEventV2::begin(),
            ChatEventV2::content("The answer is 42"),
            ChatEventV2::end(),
        ];

        // When proceessing the message
        let mut extractor = ReasoningExtractor::new();

        let mut stream_transformed: Vec<ChatEventV2> = vec![];
        for event in &stream {
            stream_transformed.extend(extractor.extract(event.clone()));
        }

        // Then the reasoning content is not set
        assert_eq!(stream_transformed, stream);
    }
}
