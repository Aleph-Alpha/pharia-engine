use anyhow::{anyhow, bail, Context as _, Error};
use change_case::snake_case;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
    sync::{LazyLock, OnceLock},
};
use tempfile::{tempdir, TempDir};

const WASI_TARGET: &str = "wasm32-wasip2";

pub struct TestSkill {
    file_name: String,
}

impl TestSkill {
    fn rust_skill(name: &'static str) -> Self {
        Self {
            file_name: snake_case(name),
        }
    }

    fn python_skill(package_name: &str) -> Self {
        Self {
            file_name: package_name.to_owned(),
        }
    }

    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        fs::read(format!("./skill_build_cache/{}.wasm", self.file_name)).unwrap()
    }
}

/// Creates `greet_skill_v0_3.wasm` in `skills` directory, based on `crates/greet-skill-v0_2`
#[must_use]
pub fn given_greet_skill_v0_3() -> TestSkill {
    static WASM_BUILD: LazyLock<()> = LazyLock::new(|| {
        given_rust_skill("greet-skill-v0_3");
    });
    *WASM_BUILD;
    TestSkill::rust_skill("greet-skill-v0_3")
}

/// Creates `greet_skill_v0_2.wasm` in `skills` directory, based on `crates/greet-skill-v0_2`
#[must_use]
pub fn given_greet_skill_v0_2() -> TestSkill {
    static WASM_BUILD: LazyLock<()> = LazyLock::new(|| {
        given_rust_skill("greet-skill-v0_2");
    });
    *WASM_BUILD;
    TestSkill::rust_skill("greet-skill-v0_2")
}

/// Creates `search_skill.wasm` in `skills` directory, based on `crates/search-skill`
#[must_use]
pub fn given_search_skill() -> TestSkill {
    static WASM_BUILD: LazyLock<()> = LazyLock::new(|| {
        given_rust_skill("search-skill");
    });
    *WASM_BUILD;
    TestSkill::rust_skill("search-skill")
}

/// Creates `search_skill.wasm` in `skills` directory, based on `crates/search-skill`
#[must_use]
pub fn given_doc_metadata_skill() -> TestSkill {
    static WASM_BUILD: LazyLock<()> = LazyLock::new(|| {
        given_rust_skill("doc-metadata-skill");
    });
    *WASM_BUILD;
    TestSkill::rust_skill("doc-metadata-skill")
}

/// Creates `chat_skill.wasm` in `skills` directory, based on `crates/chat-skill`
#[must_use]
pub fn given_chat_skill() -> TestSkill {
    static WASM_BUILD: LazyLock<()> = LazyLock::new(|| {
        given_rust_skill("chat-skill");
    });
    *WASM_BUILD;
    TestSkill::rust_skill("chat-skill")
}

#[must_use]
pub fn given_greet_py_v0_2() -> TestSkill {
    static WASM_BUILD: LazyLock<()> = LazyLock::new(|| {
        given_python_skill("greet-py-v0_2", "0.2");
    });
    *WASM_BUILD;
    TestSkill::python_skill("greet-py-v0_2")
}

#[must_use]
pub fn given_greet_py_v0_3() -> TestSkill {
    static WASM_BUILD: LazyLock<()> = LazyLock::new(|| {
        given_python_skill("greet-py-v0_3", "0.3");
    });
    *WASM_BUILD;
    TestSkill::python_skill("greet-py-v0_3")
}

#[must_use]
pub fn given_invalid_output_skill() -> TestSkill {
    static WASM_BUILD: LazyLock<()> = LazyLock::new(|| given_rust_skill("invalid-output-skill"));
    *WASM_BUILD;
    TestSkill::rust_skill("invalid-output-skill")
}

#[must_use]
pub fn given_csi_from_metadata_skill() -> TestSkill {
    static WASM_BUILD: LazyLock<()> = LazyLock::new(|| given_rust_skill("csi-from-metadata"));
    *WASM_BUILD;
    TestSkill::rust_skill("csi-from-metadata")
}

fn given_python_skill(package_name: &str, wit_version: &str) {
    if !Path::new(&format!("./skill_build_cache/{package_name}.wasm")).exists() {
        build_python_skill(package_name, wit_version);
    }
}

fn given_rust_skill(package_name: &str) {
    let snake_case = change_case::snake_case(package_name);
    if !Path::new(&format!("./skill_build_cache/{snake_case}.wasm")).exists() {
        build_rust_skill(package_name);
    }
}

/// In a nutshell this executes the following commands
///
/// ```shell
/// cargo build -p greet-skill-v0_2 --target wasm32-wasip2 --release
/// wasm-tools strip ./skill_build_cache/greet_skill_v0_2.wasm -o ./skill_build_cache/greet_skill_v0_2.wasm
/// ```
fn build_rust_skill(package_name: &str) {
    let snake_case = change_case::snake_case(package_name);

    // Build the release artefact for web assembly target
    //
    // cargo build -p greet-skill-v0_2 --target wasm32-wasip2 --release
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

    std::fs::copy(
        format!("./target/{WASI_TARGET}/release/{snake_case}.wasm"),
        format!("./skill_build_cache/{snake_case}.wasm"),
    )
    .unwrap();
}

fn build_python_skill(package_name: &str, wit_version: &str) {
    let venv = static_venv();

    let target_path = format!("./skill_build_cache/{package_name}.wasm");

    venv.run(&[
        "componentize-py",
        "-d",
        &format!("wit/skill@{wit_version}/skill.wit"),
        "-w",
        "skill",
        "componentize",
        &format!("{package_name}.app"),
        "-o",
        &target_path,
    ])
    .unwrap();

    // Make resulting skill component smaller
    //
    // wasm-tools strip ./skill_build_cache/greet-py.wasm -o ./skill_build_cache/greet-py.wasm
    Command::new("wasm-tools")
        .args(["strip", &target_path, "-o", &target_path])
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
