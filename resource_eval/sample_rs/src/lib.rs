// Allow because it is part of the bindgen generated code
#![expect(unsafe_op_in_unsafe_fn, unsafe_attr_outside_unsafe)]

use anyhow::anyhow;
use exports::pharia::skill::skill_handler::{Error, Guest};
use serde::Deserialize;
use serde_json::json;
use std::{alloc::Layout, thread::sleep, time::Duration};

#[derive(Debug, Deserialize)]
struct SampleRequest {
    topic: String,
    // memory usage in KBytes
    memory: u32,
    // in milliseconds
    wait_time: u32,
}

// that is the actual skill, but cannot use directly as cannot use client in sub folder :-(
fn consume(input: SampleRequest) -> String {
    let mut mem_size: usize = input.memory as usize;
    if mem_size == 0 {
        mem_size = 1;
    }
    let memory = Layout::array::<u8>(mem_size);
    if input.wait_time > 0 {
        sleep(Duration::from_millis(input.wait_time as u64));
    }
    drop(memory);
    format!("Hello {}", input.topic)
}

// We cannot use the SDK in Pharia Kernel repo, there is no way to exclude subfolders from Cargo magic?
wit_bindgen::generate!({ path: "../../wit/skill@0.2", world: "skill" });

struct Skill;

impl Guest for Skill {
    fn run(input: Vec<u8>) -> Result<Vec<u8>, Error> {
        let sample_request = serde_json::from_slice::<SampleRequest>(&input)
            .map_err(|e| Error::InvalidInput(anyhow!(e).to_string()))?;
        //let sinput: String = String::from_utf8(input).unwrap();
        //let request = SampleRequest {
        //    topic: sinput,
        //    memory: 0,
        //    wait_time: 0,
        //};
        let result = consume(sample_request);
        let output = serde_json::to_vec(&json!(result))
            .map_err(|e| Error::Internal(anyhow!(e).to_string()))?;
        Ok(output)
    }
}

export!(Skill);
