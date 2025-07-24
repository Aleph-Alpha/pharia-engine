use anyhow::{Context as _, Error, anyhow, bail};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{LazyLock, OnceLock},
};
use tempfile::{TempDir, tempdir};

use crate::assert_uv_installed;

const WASI_TARGET: &str = "wasm32-wasip2";
static REPO_DIR: LazyLock<PathBuf> = LazyLock::new(|| {
    let output = std::process::Command::new(env!("CARGO"))
        .arg("locate-project")
        .arg("--workspace")
        .arg("--message-format=plain")
        .output()
        .unwrap()
        .stdout;
    let cargo_path = Path::new(std::str::from_utf8(&output).unwrap().trim());
    cargo_path.parent().unwrap().to_path_buf()
});
static SKILL_BUILD_CACHE_DIR: LazyLock<PathBuf> =
    LazyLock::new(|| REPO_DIR.join("skill_build_cache"));

pub struct TestSkill {
    path: PathBuf,
}

impl TestSkill {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        fs::read(&self.path).unwrap()
    }
}

#[must_use]
pub fn given_skill_tool_invocation() -> TestSkill {
    static WASM_BUILD: LazyLock<PathBuf> = LazyLock::new(|| given_rust_skill("tool-invocation"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

#[must_use]
pub fn given_skill_infinite_streaming() -> TestSkill {
    static WASM_BUILD: LazyLock<PathBuf> = LazyLock::new(|| given_rust_skill("infinite-streaming"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

#[must_use]
pub fn given_rust_skill_chat_v0_4() -> TestSkill {
    static WASM_BUILD: LazyLock<PathBuf> = LazyLock::new(|| given_rust_skill("chat-v0_4"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

#[must_use]
pub fn given_rust_skill_greet_v0_3() -> TestSkill {
    static WASM_BUILD: LazyLock<PathBuf> = LazyLock::new(|| given_rust_skill("greet-v0_3"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

#[must_use]
pub fn given_rust_skill_complete_with_echo() -> TestSkill {
    static WASM_BUILD: LazyLock<PathBuf> = LazyLock::new(|| given_rust_skill("complete-with-echo"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

#[must_use]
pub fn given_rust_skill_greet_v0_2() -> TestSkill {
    static WASM_BUILD: LazyLock<PathBuf> = LazyLock::new(|| given_rust_skill("greet-v0_2"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

#[must_use]
pub fn given_rust_skill_explain() -> TestSkill {
    static WASM_BUILD: LazyLock<PathBuf> = LazyLock::new(|| given_rust_skill("explain"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

#[must_use]
pub fn given_rust_skill_search() -> TestSkill {
    static WASM_BUILD: LazyLock<PathBuf> = LazyLock::new(|| given_rust_skill("search"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

#[must_use]
pub fn given_rust_skill_doc_metadata() -> TestSkill {
    static WASM_BUILD: LazyLock<PathBuf> = LazyLock::new(|| given_rust_skill("doc-metadata"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

#[must_use]
pub fn given_rust_skill_chat() -> TestSkill {
    static WASM_BUILD: LazyLock<PathBuf> = LazyLock::new(|| given_rust_skill("chat"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

#[must_use]
pub fn given_python_skill_greet_v0_2() -> TestSkill {
    static WASM_BUILD: LazyLock<PathBuf> =
        LazyLock::new(|| given_python_skill("greet-v0_2", WitVersion::V0_2, "skill"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

#[must_use]
pub fn given_python_skill_greet_v0_3() -> TestSkill {
    static WASM_BUILD: LazyLock<PathBuf> =
        LazyLock::new(|| given_python_skill("greet-v0_3", WitVersion::V0_3, "skill"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

#[must_use]
pub fn given_invalid_output_skill() -> TestSkill {
    static WASM_BUILD: LazyLock<PathBuf> = LazyLock::new(|| given_rust_skill("invalid-output"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

#[must_use]
pub fn given_complete_stream_skill() -> TestSkill {
    static WASM_BUILD: LazyLock<PathBuf> = LazyLock::new(|| given_rust_skill("complete-stream"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

#[must_use]
pub fn given_chat_stream_skill() -> TestSkill {
    static WASM_BUILD: LazyLock<PathBuf> = LazyLock::new(|| given_rust_skill("chat-stream"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

#[must_use]
pub fn given_streaming_output_skill() -> TestSkill {
    static WASM_BUILD: LazyLock<PathBuf> = LazyLock::new(|| given_rust_skill("streaming-output"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

enum WitVersion {
    V0_2,
    V0_3,
}

impl WitVersion {
    fn path(&self) -> &str {
        match self {
            WitVersion::V0_2 => "wit/skill@0.2",
            WitVersion::V0_3 => "wit/skill@0.3",
        }
    }
}

/// Creates `{package-name}-py.wasm` in `SKILL_BUILD_CACHE_DIR` directory, based on `python-skills/{package-name}`
fn given_python_skill(package_name: &str, wit_version: WitVersion, world: &str) -> PathBuf {
    let target_path = SKILL_BUILD_CACHE_DIR.join(format!("{package_name}-py.wasm"));
    if !target_path.exists() {
        build_python_skill(package_name, &target_path, wit_version, world);
    }
    target_path
}

/// Creates `{package-name}-rs.wasm` in `SKILL_BUILD_CACHE_DIR` directory, based on `crates/{package-name}`
fn given_rust_skill(package_name: &str) -> PathBuf {
    let target_path = SKILL_BUILD_CACHE_DIR.join(format!("{package_name}-rs.wasm"));
    if !target_path.exists() {
        build_rust_skill(package_name);
    }
    target_path
}

/// In a nutshell this executes the following commands
///
/// ```shell
/// cargo build -p greet-v0_2 --target wasm32-wasip2 --release
/// wasm-tools strip ./skill_build_cache/greet-v0_2-rs.wasm -o ./skill_build_cache/greet-v0_2-rs.wasm
/// ```
fn build_rust_skill(package_name: &str) {
    // Build the release artefact for web assembly target
    //
    // cargo build -p greet-v0_2 --target wasm32-wasip2 --release
    let output = Command::new("cargo")
        .args([
            "build",
            "-p",
            package_name,
            "--target",
            WASI_TARGET,
            "--release",
        ])
        .output()
        .unwrap();
    error_on_status("Building web assembly failed.", output).unwrap();

    fs::create_dir_all(SKILL_BUILD_CACHE_DIR.as_path()).unwrap();

    let snake_case = change_case::snake_case(package_name);
    std::fs::copy(
        REPO_DIR
            .join("target")
            .join(WASI_TARGET)
            .join("release")
            .join(format!("{snake_case}.wasm")),
        SKILL_BUILD_CACHE_DIR.join(format!("{package_name}-rs.wasm")),
    )
    .unwrap();
}

fn build_python_skill(
    package_name: &str,
    target_path: &Path,
    wit_version: WitVersion,
    world: &str,
) {
    let venv = static_venv();

    fs::create_dir_all(SKILL_BUILD_CACHE_DIR.as_path()).unwrap();

    venv.run(&[
        "componentize-py",
        "-d",
        REPO_DIR.join(wit_version.path()).to_str().unwrap(),
        "-w",
        world,
        "componentize",
        &format!("python-skills.{package_name}.app"),
        "-o",
        target_path.to_str().unwrap(),
    ])
    .unwrap();

    // Make resulting skill component smaller
    //
    // wasm-tools strip ./skill_build_cache/greet-py.wasm -o ./skill_build_cache/greet-py.wasm
    Command::new("wasm-tools")
        .args([
            "strip",
            target_path.to_str().unwrap(),
            "-o",
            target_path.to_str().unwrap(),
        ])
        .status()
        .unwrap();
}

fn static_venv() -> &'static Venv {
    static VENV: OnceLock<Venv> = OnceLock::new();
    VENV.get_or_init(|| Venv::new().unwrap())
}

// A venv with componentize-py in a temporary directory
struct Venv {
    directory: TempDir,
}

impl Venv {
    pub fn new() -> Result<Self, Error> {
        assert_uv_installed();

        let directory = tempdir().expect("Must be able to create temporary directory");
        let venv_path = directory.path().join("venv");
        create_virtual_environment(&venv_path)?;
        install_componentize_py(&venv_path)?;

        Ok(Venv { directory })
    }

    pub fn run(&self, args: &[&str]) -> Result<(), Error> {
        let venv_path = self.directory.path().join("venv");
        let venv_path = venv_path
            .to_str()
            .context("Path to virtual environment must be representable in UTF-8.")?;
        // Run the Python interpreter in the virtual environment. Use cmd on windows or python
        // interpreter directly on other platforms

        let output = Command::new("uv")
            .args(["run", "--python", venv_path])
            .args(args)
            .current_dir(REPO_DIR.as_path())
            .output()?;

        if !output.status.success() {
            let standard_error = String::from_utf8_lossy(&output.stderr);
            bail!(
                "Failed to run command in virtual environment. Args:\n\
            {args:?}\n\
            Standard Error:\n\
            {standard_error}",
            )
        }
        Ok(())
    }
}

fn create_virtual_environment(venv_path: &Path) -> Result<(), Error> {
    let venv_path = venv_path
        .to_str()
        .expect("Temporary path must be representable in UTF-8");
    let output = Command::new("uv")
        .args(["venv", venv_path])
        .output()
        .context("Failed to execute uv command.")?;
    error_on_status(
        "Failed to create virtual environment for Python Skill building.",
        output,
    )?;
    Ok(())
}

fn install_componentize_py(venv_path: &Path) -> Result<(), Error> {
    let output = Command::new("uv")
        .args([
            "pip",
            "install",
            "--python",
            venv_path.to_str().unwrap(),
            "componentize-py==0.16.0",
        ])
        .output()?;
    error_on_status(
        "Failed to install componentize-py in virtual environment.",
        output,
    )?;
    Ok(())
}

fn error_on_status(context: &str, output: Output) -> anyhow::Result<()> {
    if output.status.success() {
        Ok(())
    } else {
        let standard_error = String::from_utf8_lossy(&output.stderr);
        Err(anyhow!("{context}\nStandard error:\n{standard_error}"))
    }
}
