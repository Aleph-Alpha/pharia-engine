use anyhow::{anyhow, bail, Context as _, Error};
use std::{
    path::Path,
    process::{Command, Output},
    sync::{LazyLock, OnceLock},
};
use tempfile::{tempdir, TempDir};
use wasi_preview1_component_adapter_provider::WASI_SNAPSHOT_PREVIEW1_REACTOR_ADAPTER;
use wit_component::ComponentEncoder;

const WASI_TARGET: &str = "wasm32-wasip1";

/// Creates `greet_skill.wasm` in `skills` directory, based on `crates/greet-skill`
pub fn given_greet_skill() {
    static WASM_BUILD: LazyLock<()> = LazyLock::new(|| {
        given_rust_skill("greet-skill");
    });
    *WASM_BUILD;
}

/// Creates `greet_skill_v0_1.wasm` in `skills` directory, based on `crates/greet-skill-v0_1`
pub fn given_greet_skill_v0_1() {
    static WASM_BUILD: LazyLock<()> = LazyLock::new(|| {
        given_rust_skill("greet-skill-v0_1");
    });
    *WASM_BUILD;
}

/// Creates `greet_skill_v0_2.wasm` in `skills` directory, based on `crates/greet-skill-v0_2`
pub fn given_greet_skill_v0_2() {
    static WASM_BUILD: LazyLock<()> = LazyLock::new(|| {
        given_rust_skill("greet-skill-v0_2");
    });
    *WASM_BUILD;
}

/// Creates `greet-py.wasm` in `skills` directory, based on `greet-py`
pub fn given_greet_py() {
    static WASM_BUILD: LazyLock<()> = LazyLock::new(|| {
        given_python_skill("greet-py", "unversioned");
    });
    *WASM_BUILD;
}

pub fn given_greet_py_v0_2() {
    static WASM_BUILD: LazyLock<()> = LazyLock::new(|| {
        given_python_skill("greet-py-v0_2", "0.2");
    });
    *WASM_BUILD;
}

fn given_python_skill(package_name: &str, wit_version: &str) {
    if !Path::new(&format!("./skills/{package_name}.wasm")).exists() {
        build_python_skill(package_name, wit_version);
    }
}

fn given_rust_skill(package_name: &str) {
    let snake_case = change_case::snake_case(package_name);
    if !Path::new(&format!("./skills/{snake_case}.wasm")).exists() {
        build_rust_skill(package_name);
    }
}

/// In a nutshell this executes the following commands
///
/// ```shell
/// cargo build -p greet-skill-v0_2 --target wasm32-wasip1 --release
/// wasm-tools component new \
///     ./target/wasm32-wasip1/release/greet_skill_v0_2.wasm \
///     -o ./skills/greet_skill_v0_2.wasm \
///     --adapt ./wasi_snapshot_preview1.reactor-25.0.0.wasm
/// wasm-tools strip ./skills/greet_skill_v0_2.wasm -o ./skills/greet_skill_v0_2.wasm
/// ```
fn build_rust_skill(package_name: &str) {
    let snake_case = change_case::snake_case(package_name);

    // Build the release artefact for web assembly target
    //
    // cargo build -p greet-skill-v0_2 --target wasm32-wasip1 --release
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

    // Until Rust supports wasi 0.2 natively, we have to adapt wasi 0.1 calls to 0.2
    let wasm_p1_bytes =
        std::fs::read(format!("./target/{WASI_TARGET}/release/{snake_case}.wasm")).unwrap();

    let wasm_p2_bytes = ComponentEncoder::default()
        .module(&wasm_p1_bytes)
        .unwrap()
        .adapter(
            "wasi_snapshot_preview1",
            WASI_SNAPSHOT_PREVIEW1_REACTOR_ADAPTER,
        )
        .unwrap()
        .validate(true)
        .encode()
        .unwrap();

    std::fs::write(format!("./skills/{snake_case}.wasm"), wasm_p2_bytes).unwrap();

    // wasm-tools strip ./skills/greet_skill_v0_2.wasm -o ./skills/greet_skill_v0_2.wasm
    let output = Command::new("wasm-tools")
        .args([
            "strip",
            &format!("./skills/{snake_case}.wasm"),
            "-o",
            &format!("./skills/{snake_case}.wasm"),
        ])
        .output()
        .unwrap();
    error_on_status("Stripping web assembly component", output).unwrap();
}

fn build_python_skill(package_name: &str, wit_version: &str) {
    let venv = static_venv();

    venv.run(&[
        "componentize-py",
        "-d",
        &format!("wit/skill@{wit_version}/skill.wit"),
        "-w",
        "skill",
        "componentize",
        &format!("{package_name}.app"),
        "-o",
        &format!("./skills/{package_name}.wasm"),
    ])
    .unwrap();

    // Make resulting skill component smaller
    //
    // wasm-tools strip ./skills/greet-py.wasm -o ./skills/greet-py.wasm
    Command::new("wasm-tools")
        .args([
            "strip",
            &format!("./skills/{package_name}.wasm"),
            "-o",
            &format!("./skills/{package_name}.wasm"),
        ])
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
            "Failed to run commond in virtual environment. Args:\n\
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
        create_virtual_enviroment(&venv_path)?;
        run_in_venv(
            &venv_path,
            &["python3", "-m", "pip", "install", "componentize-py"],
        )?;

        Ok(Venv { directory })
    }

    pub fn run(&self, args: &[&str]) -> Result<(), Error> {
        run_in_venv(&self.directory.path().join("venv"), args)?;
        Ok(())
    }
}

fn create_virtual_enviroment(venv_path: &Path) -> Result<(), Error> {
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

fn error_on_status(context: &str, output: Output) -> Result<(), anyhow::Error> {
    if output.status.success() {
        Ok(())
    } else {
        let standard_error = String::from_utf8_lossy(&output.stderr);
        Err(anyhow!("{context}\nStandard error:\n{standard_error}"))
    }
}
