use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("litzsim-cli-{name}-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn run(&self, args: &[&str], ok: bool) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_litzsim"))
            .args(args)
            .current_dir(&self.0)
            .output()
            .unwrap();
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(output.status.success(), ok, "{response}");
        assert_eq!(response["ok"], ok);
        response
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn run_publishes_ordered_original_rows_and_rejects_invalid_circuits() {
    use litzsim_core::{circuit::Document, solver, waves};

    let directory = TestDirectory::new("run");
    let rc = include_bytes!("../../../examples/rc.litzsim");
    std::fs::write(directory.0.join("rc.litzsim"), rc).unwrap();
    let summary = directory.run(&["run", "rc.litzsim", "rc.litzwave"], true);
    let out = directory.0.join("rc.litzwave");
    let wave = waves::read(&out).unwrap();
    let expected = solver::simulate(&Document::from_bytes(rc).unwrap(), || false).unwrap();
    assert_eq!(summary["rows"], 501);
    assert_eq!(summary["stopTime_s"], 0.005);
    assert_eq!(
        summary["channels"],
        json!([{"name":"time","unit":"s"},{"name":"VC_V","unit":"V"}])
    );
    assert_eq!(wave.channels(), expected.channels());
    assert!(wave.rows().eq(expected.rows()));

    let bytes = std::fs::read(&out).unwrap();
    directory.run(&["run", "rc.litzsim", "rc.litzwave"], false);
    assert_eq!(std::fs::read(&out).unwrap(), bytes);
    let truncated = directory.0.join("truncated.litzwave");
    std::fs::write(&truncated, &bytes[..bytes.len() - 1]).unwrap();
    assert!(waves::read(&truncated).is_err());

    let mut broken: Value = serde_json::from_slice(rc).unwrap();
    broken["nets"]["output"]
        .as_array_mut()
        .unwrap()
        .retain(|pin| pin != "VC.p");
    std::fs::write(
        directory.0.join("broken.litzsim"),
        serde_json::to_vec(&broken).unwrap(),
    )
    .unwrap();
    let missing = directory.run(&["run", "broken.litzsim", "missing.litzwave"], false);
    assert_eq!(missing["error"]["targetId"], "VC.p");
    assert!(!directory.0.join("missing.litzwave").exists());

    broken["parts"]["R1"]["type"] = json!("sink");
    std::fs::write(
        directory.0.join("broken.litzsim"),
        serde_json::to_vec(&broken).unwrap(),
    )
    .unwrap();
    let unsupported = directory.run(&["run", "broken.litzsim", "unsupported.litzwave"], false);
    assert_eq!(unsupported["error"]["targetId"], "R1");
    assert!(
        unsupported["error"]["message"]
            .as_str()
            .unwrap()
            .contains("sink")
    );
    assert!(!directory.0.join("unsupported.litzwave").exists());

    let pfc = include_bytes!("../../../examples/pfc.litzsim");
    std::fs::write(directory.0.join("pfc.litzsim"), pfc).unwrap();
    let unsupported = directory.run(&["run", "pfc.litzsim", "pfc.litzwave"], false);
    assert_eq!(unsupported["error"]["code"], "invalidCircuit");
    assert!(
        unsupported["error"]["message"]
            .as_str()
            .unwrap()
            .contains("control")
    );
    assert!(!directory.0.join("pfc.litzwave").exists());
}

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
        (
            "{",
            json!({ "ok": false, "error": { "code": "invalidJson", "message": "Invalid JSON" } }),
        ),
        (
            r#"{"op":"unknown"}"#,
            json!({ "ok": false, "error": { "code": "unknownOperation", "message": "Unknown operation" } }),
        ),
        (r#"{"op":"ping"}"#, pong),
    ] {
        writeln!(input, "{request}").unwrap();
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap(), expected);
    }
    let mut request = |request: Value| -> Value {
        writeln!(input, "{request}").unwrap();
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };
    let blank = request(json!({ "op": "new" }));
    let edited = request(json!({
        "op": "apply", "document": blank["document"], "expectedRevision": blank["revision"],
        "ops": [{"op": "add", "type": "ammeter", "grid": [8, 4]}]
    }));
    assert_eq!(edited["receipt"]["created"], json!(["ammeter1"]));
    let inspection = request(json!({ "op": "inspect", "document": edited["document"] }));
    assert_eq!(inspection["pinPositions"]["ammeter1.p"], json!([7, 4]));
    assert_eq!(
        inspection["measurements"]["ammeter1"],
        json!({
            "quantity": "current", "unit": "A", "fromPin": "p", "toPin": "n"
        })
    );
    assert_eq!(
        inspection["missingPins"],
        json!(["ammeter1.n", "ammeter1.p"])
    );
    let mut undo = json!({
        "op": "undo", "document": edited["document"], "expectedAfter": inspection["revision"],
        "expectedBefore": edited["receipt"]["beforeRevision"],
        "previous": blank["document"]
    });
    let undone = request(undo.clone());
    assert_eq!(undone["document"], blank["document"]);
    assert_eq!(undone["receipt"]["afterRevision"], blank["revision"]);
    undo["previous"] = edited["document"].clone();
    let rejected = request(undo);
    assert_eq!(rejected["ok"], false);
    assert_eq!(rejected["error"]["code"], "previousMismatch");
    assert_eq!(rejected["error"]["targetId"], "previous");
    assert_eq!(rejected["error"]["actual"], inspection["revision"]);

    let directory = TestDirectory::new("serve");
    let file = directory.0.join("circuit.litzsim");
    let saved = request(json!({ "op": "save", "file": file, "document": edited["document"] }));
    let opened = request(json!({ "op": "inspect", "file": file }));
    assert_eq!(opened["diskRevision"], saved["diskRevision"]);
    assert_eq!(opened["document"], edited["document"]);
    request(json!({
        "op": "save", "file": file, "document": undone["document"],
        "expectedDiskRevision": opened["diskRevision"]
    }));
    assert_eq!(
        request(json!({ "op": "inspect", "file": file }))["document"],
        blank["document"]
    );
    drop(input);
    assert!(child.wait().unwrap().success());
}

#[test]
fn file_commands_round_trip_and_reject_failed_batches() {
    const BUCK: &[u8] = include_bytes!("../../../examples/buck.litzsim");
    const PFC: &[u8] = include_bytes!("../../../examples/pfc.litzsim");
    let cleanup = TestDirectory::new("file");
    let directory = &cleanup.0;
    std::fs::write(directory.join("buck.litzsim"), BUCK).unwrap();
    std::fs::write(directory.join("snapshot.litzsim"), BUCK).unwrap();
    std::fs::write(directory.join("pfc.litzsim"), PFC).unwrap();
    let run = |args: &[&str], ok| cleanup.run(args, ok);
    let original = run(&["inspect", "buck.litzsim", "--json"], true);
    let before = original["revision"].as_str().unwrap();
    assert_ne!(original["diskRevision"], original["revision"]);
    assert_eq!(original["pinNets"]["Q1.g"], "gate");
    assert_eq!(original["pinPositions"]["Q1.g"], json!([7, 2]));
    assert_eq!(original["measurements"]["VC"]["fromPin"], "p");

    let apply = |ok| {
        run(
            &[
                "apply",
                "buck.litzsim",
                "--expect",
                before,
                "--ops",
                "ops.json",
            ],
            ok,
        )
    };
    let ops = directory.join("ops.json");
    std::fs::write(
        &ops,
        r#"[
        {"op":"mirror","id":"Q1","axis":"horizontal"},
        {"op":"disconnect","pin":"Q1.missing"}
    ]"#,
    )
    .unwrap();
    let failure = apply(false);
    assert_eq!(failure["error"]["operation"], 2);
    assert_eq!(failure["error"]["targetId"], "Q1.missing");
    assert_eq!(std::fs::read(directory.join("buck.litzsim")).unwrap(), BUCK);

    std::fs::write(
        &ops,
        r#"[
        {"op":"mirror","id":"Q1","axis":"horizontal"},
        {"op":"setParameter","id":"R1","key":"resistance_ohm","value":47}
    ]"#,
    )
    .unwrap();
    let edited = apply(true);
    assert_eq!(edited["receipt"]["beforeRevision"], before);
    let after = edited["receipt"]["afterRevision"].as_str().unwrap();
    let stored = run(&["inspect", "buck.litzsim", "--json"], true);
    assert_eq!(stored["revision"], after);
    assert_eq!(stored["diskRevision"], edited["diskRevision"]);
    assert_eq!(stored["document"]["layout"]["Q1"]["mirror"], "horizontal");
    assert_eq!(stored["pinPositions"]["Q1.g"], json!([9, 2]));
    assert_eq!(
        stored["document"]["parts"]["R1"]["parameters"]["resistance_ohm"].as_f64(),
        Some(47.0)
    );

    let restored = run(
        &[
            "undo",
            "buck.litzsim",
            "--expect",
            after,
            "--before",
            before,
            "--previous",
            "snapshot.litzsim",
        ],
        true,
    );
    assert_eq!(restored["receipt"]["afterRevision"], before);
    let final_read = run(&["inspect", "buck.litzsim", "--json"], true);
    assert_eq!(final_read["document"], original["document"]);
    assert_eq!(final_read["pinPositions"], original["pinPositions"]);
    assert_eq!(final_read["diskRevision"], before);

    let rejected = run(
        &[
            "apply",
            "pfc.litzsim",
            "--expect",
            before,
            "--ops",
            "ops.json",
        ],
        false,
    );
    assert!(
        rejected["error"]["message"]
            .as_str()
            .unwrap()
            .contains("control")
    );
    assert_eq!(std::fs::read(directory.join("pfc.litzsim")).unwrap(), PFC);
}

#[test]
fn undo_rejects_a_buck_snapshot_for_edited_rc() {
    const RC: &[u8] = include_bytes!("../../../examples/rc.litzsim");
    const BUCK: &[u8] = include_bytes!("../../../examples/buck.litzsim");
    let directory = TestDirectory::new("previous-mismatch");
    let rc = directory.0.join("u.litzsim");
    std::fs::write(&rc, RC).unwrap();
    std::fs::write(directory.0.join("buck.litzsim"), BUCK).unwrap();
    std::fs::write(
        directory.0.join("sp.json"),
        r#"[{"op":"setParameter","id":"R1","key":"resistance_ohm","value":2200}]"#,
    )
    .unwrap();

    let initial = directory.run(&["inspect", "u.litzsim", "--json"], true);
    let edited = directory.run(
        &[
            "apply",
            "u.litzsim",
            "--expect",
            initial["revision"].as_str().unwrap(),
            "--ops",
            "sp.json",
        ],
        true,
    );
    let current = directory.run(&["inspect", "u.litzsim", "--json"], true);
    let edited_bytes = std::fs::read(&rc).unwrap();
    let rejected = directory.run(
        &[
            "undo",
            "u.litzsim",
            "--expect",
            current["revision"].as_str().unwrap(),
            "--before",
            edited["receipt"]["beforeRevision"].as_str().unwrap(),
            "--previous",
            "buck.litzsim",
        ],
        false,
    );

    let buck = directory.run(&["inspect", "buck.litzsim", "--json"], true);
    assert_eq!(rejected["error"]["code"], "previousMismatch");
    assert_eq!(rejected["error"]["targetId"], "previous");
    assert_eq!(rejected["error"]["expected"], initial["revision"]);
    assert_eq!(rejected["error"]["actual"], buck["revision"]);
    assert_eq!(std::fs::read(&rc).unwrap(), edited_bytes);
}
