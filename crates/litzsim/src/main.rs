mod commands;
mod runs;

use serde_json::json;
use std::io::{self, BufRead, Write};
use std::process::ExitCode;

fn serve(input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut runs = runs::Runs::default();
    for line in input.lines() {
        let response = commands::serve_request(&mut runs, &line?);
        writeln!(output, "{}", serde_json::to_string(&response)?)?;
        output.flush()?;
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [flag] if flag == "--version" => println!("litzsim {}", litzsim_core::VERSION),
        [mode, rest @ ..] if mode == "_supervise" && rest.len() <= 1 => {
            if let Err(error) = runs::supervise(rest.first().map(|file| file.as_os_str())) {
                let _ = runs::acknowledge(&json!({ "ok": false, "error": error }));
                return ExitCode::FAILURE;
            }
        }
        [mode, path] if mode == "_solve" => {
            if !runs::solve(path.as_ref()) {
                return ExitCode::FAILURE;
            }
        }
        [mode] if mode == "serve" => {
            if let Err(error) = serve(io::stdin().lock(), io::stdout().lock()) {
                eprintln!("LitzSim I/O error: {error}");
                return ExitCode::FAILURE;
            }
        }
        _ => {
            let response = commands::file_command(&args)
                .unwrap_or_else(|error| json!({ "ok": false, "error": error }));
            println!("{response}");
            if response["ok"] != true {
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}
