use anyhow::{Context as _, Error, anyhow, bail};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
    sync::{LazyLock, OnceLock},
};
use tempfile::{TempDir, tempdir};

const WASI_TARGET: &str = "wasm32-wasip2";
const SKILL_BUILD_CACHE_DIR: &str = "./skill_build_cache";

pub struct TestSkill {
    path: String,
}

impl TestSkill {
    fn new(path: String) -> Self {
        Self { path }
    }

    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        fs::read(&self.path).unwrap()
    }
}

#[must_use]
pub fn given_rust_skill_greet_v0_3() -> TestSkill {
    static WASM_BUILD: LazyLock<String> = LazyLock::new(|| given_rust_skill("greet-v0_3"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

#[must_use]
pub fn given_rust_skill_greet_v0_2() -> TestSkill {
    static WASM_BUILD: LazyLock<String> = LazyLock::new(|| given_rust_skill("greet-v0_2"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

#[must_use]
pub fn given_rust_skill_explain() -> TestSkill {
    static WASM_BUILD: LazyLock<String> = LazyLock::new(|| given_rust_skill("explain"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

#[must_use]
pub fn given_rust_skill_search() -> TestSkill {
    static WASM_BUILD: LazyLock<String> = LazyLock::new(|| given_rust_skill("search"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

#[must_use]
pub fn given_rust_skill_doc_metadata() -> TestSkill {
    static WASM_BUILD: LazyLock<String> = LazyLock::new(|| given_rust_skill("doc-metadata"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

#[must_use]
pub fn given_rust_skill_chat() -> TestSkill {
    static WASM_BUILD: LazyLock<String> = LazyLock::new(|| given_rust_skill("chat"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

#[must_use]
pub fn given_python_skill_greet_v0_2() -> TestSkill {
    static WASM_BUILD: LazyLock<String> =
        LazyLock::new(|| given_python_skill("greet-v0_2", "0.2", "skill"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

#[must_use]
pub fn given_python_skill_greet_v0_3() -> TestSkill {
    static WASM_BUILD: LazyLock<String> =
        LazyLock::new(|| given_python_skill("greet-v0_3", "0.3", "skill"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

#[must_use]
pub fn given_invalid_output_skill() -> TestSkill {
    static WASM_BUILD: LazyLock<String> = LazyLock::new(|| given_rust_skill("invalid-output"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

#[must_use]
pub fn given_csi_from_metadata_skill() -> TestSkill {
    static WASM_BUILD: LazyLock<String> = LazyLock::new(|| given_rust_skill("csi-from-metadata"));
    let target_path = WASM_BUILD.clone();
    TestSkill::new(target_path)
}

/// Creates `{package-name}-py.wasm` in `SKILL_BUILD_CACHE_DIR` directory, based on `python-skills/{package-name}`
fn given_python_skill(package_name: &str, wit_version: &str, world: &str) -> String {
    let target_path = format!("{SKILL_BUILD_CACHE_DIR}/{package_name}-py.wasm");
    if !Path::new(&target_path).exists() {
        build_python_skill(package_name, &target_path, wit_version, world);
    }
    target_path
}

/// Creates `{package-name}-rs.wasm` in `SKILL_BUILD_CACHE_DIR` directory, based on `crates/{package-name}`
fn given_rust_skill(package_name: &str) -> String {
    let target_path = format!("{SKILL_BUILD_CACHE_DIR}/{package_name}-rs.wasm");
    if !Path::new(&target_path).exists() {
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

    fs::create_dir_all(SKILL_BUILD_CACHE_DIR).unwrap();

    let snake_case = change_case::snake_case(package_name);
    std::fs::copy(
        format!("./target/{WASI_TARGET}/release/{snake_case}.wasm"),
        format!("{SKILL_BUILD_CACHE_DIR}/{package_name}-rs.wasm"),
    )
    .unwrap();
}

fn build_python_skill(package_name: &str, target_path: &str, wit_version: &str, world: &str) {
    let venv = static_venv();

    fs::create_dir_all(SKILL_BUILD_CACHE_DIR).unwrap();

    venv.run(&[
        "componentize-py",
        "-d",
        &format!("wit/skill@{wit_version}/skill.wit"),
        "-w",
        world,
        "componentize",
        &format!("python-skills.{package_name}.app"),
        "-o",
        target_path,
    ])
    .unwrap();

    // Make resulting skill component smaller
    //
    // wasm-tools strip ./skill_build_cache/greet-py.wasm -o ./skill_build_cache/greet-py.wasm
    Command::new("wasm-tools")
        .args(["strip", target_path, "-o", target_path])
        .status()
        .unwrap();
}

/// Run a command in the given python virtual environment
fn run_in_venv(venv_path: &Path, args: &[&str]) -> Result<Vec<u8>, Error> {
    let venv_path = venv_path
        .to_str()
        .context("Path to virtual environment must be representable in UTF-8.")?;
    // Run the Python interpreter in the virtual environment. Use cmd on windows or python
    // interpreter directly on other platforms

    let mut cmd = if cfg!(target_os = "windows") {
        let activate_path = format!("{venv_path}\\Scripts\\activate.bat");
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", &activate_path, "&&"]).args(args);
        cmd
    } else {
        let mut cmd = Command::new(format!("{venv_path}/bin/{}", args[0]));
        cmd.args(&args[1..]);
        cmd
    };

    let output = cmd.output()?;
    if !output.status.success() {
        let standard_error = String::from_utf8_lossy(&output.stderr);
        bail!(
            "Failed to run command in virtual environment. Args:\n\
            {args:?}\n\
            Standard Error:\n\
            {standard_error}",
        )
    }

    let standard_out = output.stdout;

    Ok(standard_out)
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
        let directory = tempdir().expect("Must be able to create temporary directory");
        let venv_path = directory.path().join("venv");
        create_virtual_environment(&venv_path)?;
        run_in_venv(
            &venv_path,
            &["python", "-m", "pip", "install", "componentize-py"],
        )?;

        Ok(Venv { directory })
    }

    pub fn run(&self, args: &[&str]) -> Result<(), Error> {
        run_in_venv(&self.directory.path().join("venv"), args)?;
        Ok(())
    }
}

fn create_virtual_environment(venv_path: &Path) -> Result<(), Error> {
    let venv_path = venv_path
        .to_str()
        .expect("Temporary path must be representable in UTF-8");
    let output = Command::new("python3")
        .args(["-m", "venv", venv_path])
        .output()
        .context("Failed to start 'python' command.")?;
    error_on_status(
        "Failed to create virtual environment for Python Skill building.",
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
