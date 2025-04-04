use exports::pharia::skill::message_stream::{Error, Guest};
use pharia::skill::streaming_output::{BeginAttributes, MessageItem, StreamOutput};

wit_bindgen::generate!({ path: "../../wit/skill@0.3", world: "message-stream-skill", features: ["streaming"] });

struct Skill;

impl Guest for Skill {
    fn run(_: Vec<u8>, stream_output: StreamOutput) -> Result<(), Error> {
        stream_output.write(&MessageItem::MessageBegin(BeginAttributes { role: None }));
        for i in 0..u64::MAX {
            stream_output.write(&MessageItem::MessageAppend(i.to_string()));
        }
        Ok(())
    }
}

export!(Skill);
