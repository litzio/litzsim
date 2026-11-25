use serde_json::{Value, json};
use std::io::{self, BufRead, Write};
use std::process::ExitCode;

fn serve(input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    for line in input.lines() {
        let response = match serde_json::from_str::<Value>(&line?) {
            Ok(request) if request.get("op").and_then(Value::as_str) == Some("ping") => {
                litzsim_core::ping()
            }
            Ok(_) => json!({ "ok": false, "error": "Unknown operation" }),
            Err(_) => json!({ "ok": false, "error": "Invalid JSON" }),
        };
        writeln!(output, "{response}")?;
        output.flush()?;
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [flag] if flag == "--version" => println!("litzsim {}", litzsim_core::VERSION),
        [mode] if mode == "serve" => {
            if let Err(error) = serve(io::stdin().lock(), io::stdout().lock()) {
                eprintln!("LitzSim I/O error: {error}");
                return ExitCode::FAILURE;
            }
        }
        _ => {
            eprintln!("Usage: litzsim --version | litzsim serve");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
