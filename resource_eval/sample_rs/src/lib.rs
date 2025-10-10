use anyhow::anyhow;
use exports::pharia::skill::skill_handler::{Error, Guest};
use serde::Deserialize;
use serde_json::json;
use std::{
    alloc::{Layout, LayoutError},
    thread::sleep,
    time::Duration,
};

#[derive(Debug, Deserialize)]
struct SampleRequest {
    topic: String,
    // memory usage in KBytes
    memory: u32,
    // in milliseconds
    wait_time: u32,
}

// consumes small memory chunks if memory is larger than 300 KB, chunk size is between 100 and 300
// KB
fn consume_memory(mut mem_size: usize) -> Result<Vec<Layout>, LayoutError> {
    let mut chunks = vec![];
    if mem_size < 300 {
        let memory = Layout::array::<u8>(mem_size)?;
        chunks.push(memory);
        return Ok(chunks);
    }
    while mem_size > 300 {
        let chunk_size = (rand::random::<u64>() as usize % 201) + 100;
        let chunk = Layout::array::<u8>(chunk_size)?;
        chunks.push(chunk);
        mem_size -= chunk_size;
    }
    if mem_size > 0 {
        let chunk = Layout::array::<u8>(mem_size)?;
        chunks.push(chunk);
    }
    Ok(chunks)
}

// that is the actual skill, but cannot use directly as cannot use client in sub folder :-(
fn consume(input: SampleRequest) -> String {
    let mut mem_size: usize = input.memory as usize;
    if mem_size == 0 {
        mem_size = 1;
    }
    let memory = consume_memory(mem_size);
    let memory = match memory {
        Ok(val) => val,
        Err(err) => return format!("execution failed while allocating memory with {err}"),
    };
    let mut wait_msg = "".to_owned();
    if input.wait_time > 0 {
        sleep(Duration::from_millis(input.wait_time as u64));
        wait_msg = format!(" time={:06} ms", input.wait_time);
    }
    let mem_msg = format!("  memory={:06} KB in {} chunks", mem_size, memory.len());
    for chunk in memory.into_iter() {
        // well, i want to explicitly drop it
        #[allow(dropping_copy_types)]
        drop(chunk)
    }
    format!("Hello {}  {} {}", input.topic, mem_msg, wait_msg)
}

// We cannot use the SDK in Pharia Kernel repo, there is no way to exclude subfolders from Cargo
// magic?
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
