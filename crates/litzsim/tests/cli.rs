use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn serves_json_lines_until_stdin_closes() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_litzsim"))
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let pong = json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") });
    for (request, expected) in [
        (r#"{"op":"ping"}"#, pong.clone()),
        ("{", json!({ "ok": false, "error": "Invalid JSON" })),
        (r#"{"op":"ping"}"#, pong),
    ] {
        writeln!(input, "{request}").unwrap();
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap(), expected);
    }
    drop(input);
    assert!(child.wait().unwrap().success());
}
