use serde_json::{Value, json};
use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

fn package_plugin() -> Result<(), Box<dyn Error>> {
    if !cfg!(windows) {
        return Err("The Litzio plugin package currently targets Windows.".into());
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask is inside the workspace");
    let target = root.join("target");
    let build = Command::new(env!("CARGO"))
        .args([
            "build",
            "--release",
            "--locked",
            "--package",
            "litzsim",
            "--target",
            "host-tuple",
            "--message-format=json-render-diagnostics",
        ])
        .arg("--target-dir")
        .arg(&target)
        .current_dir(root)
        .stderr(Stdio::inherit())
        .output()?;
    if !build.status.success() {
        return Err(format!("LitzSim release build failed ({})", build.status).into());
    }
    let executable = String::from_utf8(build.stdout)?
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|message| {
            message["reason"] == "compiler-artifact"
                && message["target"]["name"] == "litzsim"
                && message["executable"].is_string()
        })
        .and_then(|mut message| message["executable"].take().as_str().map(str::to_owned))
        .ok_or("Cargo did not report a LitzSim executable.")?;

    let source = root.join("plugin");
    let output = target.join("plugin/litzsim");
    let bin = output.join("bin");
    fs::create_dir_all(&bin)?;
    for file in ["host.mjs", "screen.jsx"] {
        fs::copy(source.join(file), output.join(file))?;
    }
    let manifest = "litzio-plugin.json";
    let mut config: Value = serde_json::from_slice(&fs::read(source.join(manifest))?)?;
    config["version"] = json!(env!("CARGO_PKG_VERSION"));
    fs::copy(executable, bin.join("litzsim.exe"))?;
    fs::write(output.join(manifest), serde_json::to_vec_pretty(&config)?)?;
    println!("{}", output.display());
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let result = match args.as_slice() {
        [command] if command == "plugin" => package_plugin(),
        _ => Err("Usage: cargo xtask plugin".into()),
    };
    if let Err(error) = result {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
