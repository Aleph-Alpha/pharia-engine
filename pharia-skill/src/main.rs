use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
#[clap(version)]
struct Cli {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Parser)]
enum Command {
    /// Publish a skill to OCI registry
    Publish {
        /// Path to skill wasm file
        skill: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();

    println!("Hello, world!");
}

