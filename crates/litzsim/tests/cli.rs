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
fn probe_requires_ground_before_run_admission_and_reports_an_unconnected_pin() {
    let directory = TestDirectory::new("probe-diagnostics");
    let file = directory.0.join("probe.litzsim");
    let mut circuit: Value =
        serde_json::from_str(include_str!("../../../examples/buck-probe.litzsim")).unwrap();
    circuit["nets"]["sw"]
        .as_array_mut()
        .unwrap()
        .retain(|pin| pin != "VP1.p");
    std::fs::write(&file, serde_json::to_vec(&circuit).unwrap()).unwrap();
    let detached = directory.run(&["run", "probe.litzsim", "probe.litzwave"], false);
    assert_eq!(detached["error"]["code"], "invalidCircuit");
    assert_eq!(detached["error"]["targetId"], "VP1.p");
    assert_eq!(detached["error"]["message"], "Unconnected pins: VP1.p");

    circuit["nets"]["sw"]
        .as_array_mut()
        .unwrap()
        .push(json!("VP1.p"));
    circuit["parts"].as_object_mut().unwrap().remove("GND");
    circuit["layout"].as_object_mut().unwrap().remove("GND");
    circuit["nets"]["ground"]
        .as_array_mut()
        .unwrap()
        .retain(|pin| pin != "GND.ref");
    std::fs::write(&file, serde_json::to_vec(&circuit).unwrap()).unwrap();
    let foreground = directory.run(&["run", "probe.litzsim", "probe.litzwave"], false);
    let background = directory.run(&["start", "probe.litzsim"], false);
    assert_eq!(foreground["error"], background["error"]);
    assert_eq!(background["error"]["code"], "invalidCircuit");
    assert_eq!(background["error"]["targetId"], "VP1");
    assert!(
        background["error"]["message"]
            .as_str()
            .unwrap()
            .contains("VP1")
    );
    assert!(!directory.0.join("probe.litzwave").exists());
    assert!(!directory.0.join("runs").exists());
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
    assert_eq!(summary["warnings"], json!([]));
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
fn inrush_warning_is_shared_by_run_and_status() {
    let folder = TestDirectory::new("inrush");
    let mut rc: Value =
        serde_json::from_slice(include_bytes!("../../../examples/rc.litzsim")).unwrap();
    rc["parts"].as_object_mut().unwrap().remove("R1");
    rc["layout"].as_object_mut().unwrap().remove("R1");
    rc["nets"].as_object_mut().unwrap().remove("input");
    rc["nets"]["output"] = json!(["V1.p", "C1.p", "VC.p"]);
    rc["simulation"] = json!({"timeStep_s": 1e-6, "stopTime_s": 5e-6});
    std::fs::write(folder.0.join("inrush.litzsim"), rc.to_string()).unwrap();

    let summary = folder.run(&["run", "inrush.litzsim", "inrush.litzwave"], true);
    let warning = &summary["warnings"][0];
    assert_eq!(summary["warnings"].as_array().unwrap().len(), 1);
    assert_eq!(warning["code"], "inrushTimeStep");
    assert_eq!(warning["partIds"], json!(["C1", "V1"]));
    assert_eq!(warning["timeConstant_s"], 1e-12);
    assert_eq!(warning["timeStep_s"], 1e-6);
    let started = folder.run(&["start", "inrush.litzsim"], true);
    let run = started["runDirectory"].as_str().unwrap();
    let status = terminal(&folder, run, "completed");
    assert_eq!(status["warnings"], summary["warnings"]);

    rc["parts"]["R1"] = json!({"type": "resistor", "parameters": {"resistance_ohm": 0.02}});
    rc["parts"]["C1"]["parameters"]["esr_ohm"] = json!(0.01);
    rc["parts"]["V1"]["parameters"]["internalResistance_ohm"] = json!(0.03);
    rc["nets"]["input"] = json!(["V1.p", "R1.p"]);
    rc["nets"]["output"] = json!(["R1.n", "C1.p", "VC.p"]);
    std::fs::write(folder.0.join("inrush.litzsim"), rc.to_string()).unwrap();
    let series = folder.run(&["run", "inrush.litzsim", "series.litzwave"], true);
    let warning = &series["warnings"][0];
    assert_eq!(series["warnings"].as_array().unwrap().len(), 1);
    assert_eq!(warning["partIds"], json!(["C1", "R1", "V1"]));
    assert!((warning["timeConstant_s"].as_f64().unwrap() - 6e-8).abs() < 1e-21);
}

#[test]
fn replacing_unedited_examples_preserves_raw_bytes() {
    use litzsim_core::circuit::{SaveMode, open, save};
    use std::fs;

    let folder = TestDirectory::new("unchanged-save");
    let path = folder.0.join("circuit.litzsim");
    for bytes in [
        include_bytes!("../../../examples/rc.litzsim").as_slice(),
        include_bytes!("../../../examples/rlc.litzsim").as_slice(),
        include_bytes!("../../../examples/buck.litzsim").as_slice(),
    ] {
        fs::write(&path, bytes).unwrap();
        let opened = open(&path).unwrap();
        let revision = save(
            &path,
            &opened.document,
            SaveMode::Replace {
                expected_disk_revision: &opened.disk_revision,
            },
        )
        .unwrap();
        assert_eq!(revision, opened.disk_revision);
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }

    let mut explicit: Value =
        serde_json::from_slice(include_bytes!("../../../examples/rc.litzsim")).unwrap();
    explicit["parts"]["C1"]["parameters"]["esr_ohm"] = json!(1e-6);
    explicit["parts"]["V1"]["parameters"]["internalResistance_ohm"] = json!(0);
    fs::write(&path, explicit.to_string()).unwrap();
    let opened = open(&path).unwrap();
    let revision = save(
        &path,
        &opened.document,
        SaveMode::Replace {
            expected_disk_revision: &opened.disk_revision,
        },
    )
    .unwrap();
    assert_ne!(revision, opened.disk_revision);
    assert_eq!(fs::read(&path).unwrap(), opened.document.to_bytes());
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
    let mut request = |request: Value, expected_ok: bool| -> Value {
        writeln!(input, "{request}").unwrap();
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["ok"], expected_ok, "{}: {response}", request["op"]);
        response
    };
    let blank = request(json!({ "op": "new" }), true);
    let edited = request(
        json!({
            "op": "apply", "document": blank["document"], "expectedRevision": blank["revision"],
            "ops": [{"op": "add", "type": "ammeter", "grid": [8, 4]}]
        }),
        true,
    );
    assert_eq!(edited["receipt"]["created"], json!(["AM1"]));
    assert_eq!(
        edited["layout"]["revision"],
        edited["receipt"]["afterRevision"]
    );
    assert_eq!(edited["layout"]["pins"]["AM1.p"]["at"], json!([224, 128]));
    assert_eq!(
        edited["layout"]["symbols"][0]["currentArrow"]["to"],
        json!([266, 137])
    );
    let inspection = request(
        json!({ "op": "inspect", "document": edited["document"] }),
        true,
    );
    assert_eq!(inspection["pinPositions"]["AM1.p"], json!([7, 4]));
    assert_eq!(
        inspection["measurements"]["AM1"],
        json!({
            "quantity": "current", "unit": "A", "fromPin": "p", "toPin": "n"
        })
    );
    assert_eq!(inspection["missingPins"], json!(["AM1.n", "AM1.p"]));
    let mut undo = json!({
        "op": "undo", "document": edited["document"], "expectedAfter": inspection["revision"],
        "expectedBefore": edited["receipt"]["beforeRevision"],
        "previous": blank["document"]
    });
    let undone = request(undo.clone(), true);
    assert_eq!(undone["document"], blank["document"]);
    assert_eq!(undone["receipt"]["afterRevision"], blank["revision"]);
    assert_eq!(undone["layout"]["revision"], blank["revision"]);
    assert_eq!(undone["layout"]["symbols"], json!([]));
    undo["previous"] = edited["document"].clone();
    let rejected = request(undo, false);
    assert_eq!(rejected["error"]["code"], "previousMismatch");
    assert_eq!(rejected["error"]["targetId"], "previous");
    assert_eq!(rejected["error"]["actual"], inspection["revision"]);

    let directory = TestDirectory::new("serve");
    let file = directory.0.join("circuit.litzsim");
    let saved = request(
        json!({ "op": "save", "file": file, "document": edited["document"] }),
        true,
    );
    let opened = request(json!({ "op": "inspect", "file": file }), true);
    assert_eq!(opened["diskRevision"], saved["diskRevision"]);
    assert_eq!(opened["document"], edited["document"]);
    let mut from_file = directory.run(&["layout", "circuit.litzsim", "--json"], true);
    from_file.as_object_mut().unwrap().remove("ok");
    assert_eq!(from_file, edited["layout"]);
    for (key, source) in [
        ("file", json!(file)),
        ("document", edited["document"].clone()),
    ] {
        let mut drawn = request(json!({ "op": "layout", key: source }), true);
        drawn.as_object_mut().unwrap().remove("ok");
        assert_eq!(drawn, edited["layout"]);
    }
    let replaced = request(
        json!({
            "op": "save", "file": file, "document": undone["document"],
            "expectedDiskRevision": opened["diskRevision"]
        }),
        true,
    );
    let reopened = request(json!({ "op": "inspect", "file": file }), true);
    assert_eq!(reopened["diskRevision"], replaced["diskRevision"]);
    assert_eq!(reopened["document"], blank["document"]);

    #[cfg(windows)]
    {
        use litzsim_core::circuit::REPLACE_RETRY_DELAYS;
        use std::fs::{self, OpenOptions};
        use std::os::windows::fs::OpenOptionsExt;
        use std::time::{Duration, Instant};

        const FILE_SHARE_READ: u32 = 0x1;
        const FILE_SHARE_WRITE: u32 = 0x2;
        let original = fs::read(&file).unwrap();
        // This test process holds the file without delete sharing while the CLI saves.
        let _reader = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&file)
            .unwrap();
        let retry_budget: Duration = REPLACE_RETRY_DELAYS.into_iter().sum();
        let started = Instant::now();
        let rejected = request(
            json!({
                "op": "save", "file": file, "document": edited["document"],
                "expectedDiskRevision": reopened["diskRevision"]
            }),
            false,
        );
        let elapsed = started.elapsed();
        // Include CLI round-trip, disk I/O and Windows scheduler overhead.
        assert!(
            elapsed >= retry_budget,
            "replacement took {elapsed:?} with a {retry_budget:?} sleep budget"
        );
        assert_eq!(rejected["error"]["code"], "ioError");
        let path = file.canonicalize().unwrap();
        let message = rejected["error"]["message"].as_str().unwrap();
        assert!(
            message.starts_with(&format!("Could not replace {}: ", path.display())),
            "{message}"
        );
        assert_eq!(fs::read(&file).unwrap(), original);
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
    }
    drop(input);
    assert!(child.wait().unwrap().success());
}

#[cfg(windows)]
#[test]
fn replacing_a_readonly_circuit_reports_the_destination() {
    use litzsim_core::circuit::{Document, SaveMode, save};
    use std::fs;

    let directory = TestDirectory::new("readonly-save");
    let path = directory.0.join("circuit.litzsim");
    let original = include_bytes!("../../../examples/rc.litzsim");
    let disk_revision = save(
        &path,
        &Document::from_bytes(original).unwrap(),
        SaveMode::New,
    )
    .unwrap();
    let path = path.canonicalize().unwrap();
    let permissions = fs::metadata(&path).unwrap().permissions();
    let mut readonly = permissions.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&path, readonly).unwrap();

    let result = save(
        &path,
        &Document::new(),
        SaveMode::Replace {
            expected_disk_revision: &disk_revision,
        },
    );
    fs::set_permissions(&path, permissions).unwrap();

    let error = result.unwrap_err();
    assert_eq!(error.code, "ioError");
    assert!(
        error
            .message
            .starts_with(&format!("Could not replace {}: ", path.display())),
        "{}",
        error.message
    );
    assert_eq!(
        litzsim_core::circuit::open(&path).unwrap().disk_revision,
        disk_revision
    );
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
    let original_layout = run(&["layout", "buck.litzsim", "--json"], true);
    let before = original["revision"].as_str().unwrap();
    assert_ne!(original["diskRevision"], original["revision"]);
    assert_eq!(original["pinNets"]["Q1.g"], "gate");
    assert_eq!(original["pinPositions"]["Q1.g"], json!([7, 2]));
    assert_eq!(original["measurements"]["VC"]["fromPin"], "p");
    let original_document = litzsim_core::circuit::open(&directory.join("buck.litzsim"))
        .unwrap()
        .document;
    let canonical_before = original_document.to_bytes();

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
        r#"[{"op":"setRecording","id":"PWM","quantity":"current","enabled":true}]"#,
    )
    .unwrap();
    let unsupported = apply(false);
    assert_eq!(unsupported["error"]["operation"], 1);
    assert_eq!(unsupported["error"]["targetId"], "PWM");
    assert_eq!(std::fs::read(directory.join("buck.litzsim")).unwrap(), BUCK);

    std::fs::write(
        &ops,
        r#"[
        {"op":"mirror","id":"Q1","axis":"horizontal"},
        {"op":"setParameter","id":"R1","key":"resistance_ohm","value":47},
        {"op":"rename","id":"VC","newId":"Vout"},
        {"op":"setRecording","id":"L1","quantity":"voltage","enabled":true},
        {"op":"setRecording","id":"L1","quantity":"current","enabled":true}
    ]"#,
    )
    .unwrap();
    let edited = apply(true);
    assert_eq!(edited["receipt"]["beforeRevision"], before);
    assert_eq!(edited["receipt"]["renamed"], json!({"VC": "Vout"}));
    let after = edited["receipt"]["afterRevision"].as_str().unwrap();
    let stored = run(&["inspect", "buck.litzsim", "--json"], true);
    assert_eq!(stored["revision"], after);
    assert_eq!(stored["diskRevision"], edited["diskRevision"]);
    assert_eq!(stored["document"]["layout"]["Q1"]["mirror"], "horizontal");
    assert_eq!(stored["document"]["version"], 1);
    assert_eq!(
        stored["document"]["parts"]["L1"]["record"],
        json!({"voltage": true, "current": true})
    );
    assert_eq!(stored["pinPositions"]["Q1.g"], json!([9, 2]));
    assert_eq!(stored["pinNets"]["Vout.p"], "output");
    assert_eq!(stored["measurements"]["Vout"]["fromPin"], "p");
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
    assert_eq!(
        std::fs::read(directory.join("buck.litzsim")).unwrap(),
        canonical_before
    );
    assert_eq!(
        run(&["layout", "buck.litzsim", "--json"], true),
        original_layout
    );

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
fn renaming_a_recorded_inductor_preserves_samples_and_undo_restores_file() {
    use litzsim_core::waves;

    let folder = TestDirectory::new("rename-recording");
    let file = folder.0.join("buck.litzsim");
    let ops = folder.0.join("ops.json");
    std::fs::write(&file, include_bytes!("../../../examples/buck.litzsim")).unwrap();
    let inspect = folder.run(&["inspect", "buck.litzsim", "--json"], true);
    let apply = |revision, operation: &str| {
        std::fs::write(&ops, operation).unwrap();
        folder.run(
            &[
                "apply",
                "buck.litzsim",
                "--expect",
                revision,
                "--ops",
                "ops.json",
            ],
            true,
        )
    };
    let recorded = apply(
        inspect["revision"].as_str().unwrap(),
        r#"[{"op":"setRecording","id":"L1","quantity":"current","enabled":true}]"#,
    );
    let before = recorded["receipt"]["afterRevision"].as_str().unwrap();
    let original = std::fs::read(&file).unwrap();
    std::fs::write(folder.0.join("snapshot.litzsim"), &original).unwrap();
    folder.run(&["run", "buck.litzsim", "before.litzwave"], true);

    let renamed = apply(before, r#"[{"op":"rename","id":"L1","newId":"Lmain"}]"#);
    assert_eq!(renamed["receipt"]["renamed"], json!({"L1": "Lmain"}));
    folder.run(&["run", "buck.litzsim", "after.litzwave"], true);
    let previous = waves::read(&folder.0.join("before.litzwave")).unwrap();
    let current = waves::read(&folder.0.join("after.litzwave")).unwrap();
    let previous_column = previous
        .channels()
        .iter()
        .position(|channel| channel.name == "L1_A")
        .unwrap();
    let current_column = current
        .channels()
        .iter()
        .position(|channel| channel.name == "Lmain_A")
        .unwrap();
    assert!(
        current
            .channels()
            .iter()
            .all(|channel| channel.name != "L1_A")
    );
    assert_eq!(current.channels()[current_column].unit, "A");
    assert!(previous.rows().len() > 0);
    assert!(
        previous
            .rows()
            .map(|row| (row[0].to_bits(), row[previous_column].to_bits()))
            .eq(current
                .rows()
                .map(|row| (row[0].to_bits(), row[current_column].to_bits())))
    );

    folder.run(
        &[
            "undo",
            "buck.litzsim",
            "--expect",
            renamed["receipt"]["afterRevision"].as_str().unwrap(),
            "--before",
            before,
            "--previous",
            "snapshot.litzsim",
        ],
        true,
    );
    assert_eq!(std::fs::read(&file).unwrap(), original);
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

fn terminal(directory: &TestDirectory, run: &str, state: &str) -> Value {
    let start = std::time::Instant::now();
    loop {
        let status = directory.run(&["status", run], true);
        if status["state"] == state {
            return status;
        }
        assert!(
            status["state"] == "running" || status["state"] == "stopping",
            "unexpected terminal status: {status}"
        );
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "{status}"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn write_long_rc(folder: &TestDirectory) {
    let mut document: Value =
        serde_json::from_slice(include_bytes!("../../../examples/rc.litzsim")).unwrap();
    document["simulation"]["stopTime_s"] = json!(100_000.0);
    document["simulation"]["outputStep_s"] = json!(1.0);
    std::fs::write(folder.0.join("long.litzsim"), document.to_string()).unwrap();
}

#[test]
fn simultaneous_starts_admit_only_one_in_the_same_directory() {
    let folder = TestDirectory::new("simultaneous-starts");
    write_long_rc(&folder);
    let children: Vec<_> = (0..4)
        .map(|_| {
            Command::new(env!("CARGO_BIN_EXE_litzsim"))
                .args(["start", "long.litzsim"])
                .current_dir(&folder.0)
                .stdout(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    let mut admitted = Vec::new();
    let mut rejected = Vec::new();
    for child in children {
        let output = child.wait_with_output().unwrap();
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        if output.status.success() {
            admitted.push(response);
        } else {
            rejected.push(response);
        }
    }
    for response in &admitted {
        let path = response["runDirectory"].as_str().unwrap();
        folder.run(&["stop", path], true);
        terminal(&folder, path, "canceled");
    }
    assert_eq!(admitted.len(), 1, "{admitted:?}");
    assert_eq!(rejected.len(), 3, "{rejected:?}");
    assert!(
        rejected
            .iter()
            .all(|r| r["error"]["code"] == "runInProgress")
    );
}

#[test]
fn stop_before_supervisor_claims_reservation_cancels_without_spawning_solver() {
    use std::fs::{self, File, OpenOptions};

    let folder = TestDirectory::new("early-stop");
    let root = folder.0.join("runs");
    let run = root.join("1-2-0");
    fs::create_dir_all(&run).unwrap();
    fs::write(
        run.join("circuit.litzsim"),
        include_bytes!("../../../examples/rc.litzsim"),
    )
    .unwrap();
    File::create_new(run.join("status.lock")).unwrap();
    fs::write(
        run.join("status.json"),
        json!({
            "runId": "1-2-0", "revision": "fixture", "state": "running",
            "simulatedTime": 0, "stopTime": 0.01, "solverPid": null,
        })
        .to_string(),
    )
    .unwrap();

    // Hold the admission gate so the actual supervisor cannot start the solver
    // before the public stop command reaches the prepared run.
    let gate = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(root.join("admission.lock"))
        .unwrap();
    gate.lock().unwrap();
    let mut supervisor = Command::new(env!("CARGO_BIN_EXE_litzsim"))
        .arg("_supervise")
        .arg(&run)
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    fs::write(
        root.join("starting"),
        json!({
            "run_id": "1-2-0", "supervisor_pid": supervisor.id(),
        })
        .to_string(),
    )
    .unwrap();
    let path = run.to_str().unwrap();
    let stopped = folder.run(&["stop", path], true);
    drop(gate);
    assert!(supervisor.wait().unwrap().success());
    assert_eq!(stopped["state"], "stopping");
    let canceled = folder.run(&["status", path], true);
    assert_eq!(canceled["state"], "canceled");
    assert!(canceled["solverPid"].is_null());
    assert!(!run.join("result.pending").exists());
    #[cfg(windows)]
    {
        let escaped_path = path.replace('\'', "''");
        let script = format!(
            r#"$ErrorActionPreference='Stop'
            $live = Get-CimInstance Win32_Process -Filter "Name='litzsim.exe'" |
                Where-Object {{ $_.CommandLine -and $_.CommandLine.IndexOf('{escaped_path}', [StringComparison]::OrdinalIgnoreCase) -ge 0 }}
            if ($live) {{ throw 'A process for the canceled run is still alive' }}"#
        );
        let result = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[cfg(windows)]
#[test]
fn status_and_stop_report_failure_after_the_supervisor_is_killed() {
    use std::time::{Duration, Instant};

    let folder = TestDirectory::new("supervisor-killed");
    write_long_rc(&folder);
    let mut recovered = Vec::new();
    for operation in ["status", "stop"] {
        let started = folder.run(&["start", "long.litzsim"], true);
        let path = started["runDirectory"].as_str().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let solver = loop {
            let status = folder.run(&["status", path], true);
            assert_eq!(status["state"], "running");
            if let Some(pid) = status["solverPid"].as_u64() {
                break pid;
            }
            assert!(Instant::now() < deadline, "{status}");
            std::thread::sleep(Duration::from_millis(20));
        };
        let powershell = format!(
            r"$ErrorActionPreference='Stop'
            $worker = Get-Process -Id {solver}
            $null = $worker.Handle
            $entry = Get-CimInstance Win32_Process -Filter 'ProcessId={solver}'
            $owner = Get-CimInstance Win32_Process -Filter ('ProcessId=' + $entry.ParentProcessId)
            if ($owner.CommandLine -notmatch '\s_supervise\s') {{ throw 'Unexpected solver parent' }}
            Stop-Process -Id $entry.ParentProcessId -Force
            if (!$worker.WaitForExit(10000)) {{ throw 'Solver survived supervisor exit' }}"
        );
        let result = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &powershell])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        recovered.push(folder.run(&[operation, path], true));
    }
    for status in recovered {
        assert_eq!(status["state"], "failed", "{status}");
        assert_eq!(status["error"]["code"], "supervisorExited", "{status}");
    }
}

fn wave_request(
    directory: &TestDirectory,
    run: &str,
    ok: bool,
    from: &str,
    to: &str,
    pixels: &str,
) -> Value {
    directory.run(
        &[
            "wave",
            run,
            "--channel",
            "L1_A",
            "--channel",
            "VC_V",
            "--from",
            from,
            "--to",
            to,
            "--pixels",
            pixels,
        ],
        ok,
    )
}

#[test]
fn long_run_snapshots_and_releases_the_directory_after_stop() {
    use std::time::{Duration, Instant};
    let folder = TestDirectory::new("long-run");
    let circuit = folder.0.join("source.litzsim");
    let rc = include_bytes!("../../../examples/rc.litzsim");
    let mut long: Value = serde_json::from_slice(rc).unwrap();
    long["simulation"]["stopTime_s"] = json!(100_000.0);
    long["simulation"]["outputStep_s"] = json!(1.0);
    std::fs::write(&circuit, long.to_string()).unwrap();
    let limit = Duration::from_secs(10);

    let t = Instant::now();
    let started = folder.run(&["start", "source.litzsim"], true);
    let start_time = t.elapsed();
    assert!(start_time < limit);
    let run = started["runDirectory"].as_str().unwrap();
    assert_eq!(started["state"], "running");
    let snapshot = PathBuf::from(run).join("circuit.litzsim");
    let old = litzsim_core::circuit::open(&snapshot).unwrap();
    assert_eq!(started["revision"], old.revision);

    std::fs::write(&circuit, rc).unwrap();
    let rejected = folder.run(&["start", "source.litzsim"], false);
    assert_eq!(rejected["error"]["code"], "runInProgress");
    std::thread::sleep(Duration::from_millis(10_050));
    let t = Instant::now();
    let running = folder.run(&["status", run], true);
    let status_time = t.elapsed();
    assert!(status_time < limit);
    assert_eq!(running["state"], "running");
    assert_eq!(running["stopTime"], 100_000.0);
    assert!(running["simulatedTime"].as_f64().unwrap() > 0.0);
    assert_eq!(
        litzsim_core::circuit::open(&snapshot).unwrap().revision,
        old.revision
    );

    let t = Instant::now();
    let stopping = folder.run(&["stop", run], true);
    let stop_time = t.elapsed();
    assert!(stop_time < limit);
    assert_eq!(stopping["state"], "stopping");
    let canceled = terminal(&folder, run, "canceled");
    assert_eq!(canceled["solverPid"], running["solverPid"]);
    assert!(
        canceled["simulatedTime"].as_f64().unwrap() >= running["simulatedTime"].as_f64().unwrap()
    );
    assert_eq!(
        wave_request(&folder, run, false, "0", "1", "10")["error"]["code"],
        "runNotCompleted"
    );
    assert!(!PathBuf::from(run).join("result.litzwave").exists());
    assert!(!PathBuf::from(run).join("result.pending").exists());

    let next = folder.run(&["start", "source.litzsim"], true);
    assert_ne!(next["runId"], started["runId"]);
    let next_dir = next["runDirectory"].as_str().unwrap();
    terminal(&folder, next_dir, "completed");
    let after_completion = folder.run(&["start", "source.litzsim"], true);
    terminal(
        &folder,
        after_completion["runDirectory"].as_str().unwrap(),
        "completed",
    );
    eprintln!("long-run CLI start {start_time:?}, status {status_time:?}, stop {stop_time:?}");
}

#[test]
fn managed_buck_wave_matches_the_original_rows_and_failed_runs_have_no_wave() {
    let folder = TestDirectory::new("managed-buck");
    let mut buck: Value =
        serde_json::from_str(include_str!("../../../examples/buck.litzsim")).unwrap();
    buck["parts"]["L1"]["record"] = json!({"current": true});
    std::fs::write(folder.0.join("buck.litzsim"), buck.to_string()).unwrap();
    let started = folder.run(&["start", "buck.litzsim"], true);
    let run = started["runDirectory"].as_str().unwrap();
    let completed = terminal(&folder, run, "completed");
    assert_eq!(completed["simulatedTime"], completed["stopTime"]);
    let result = PathBuf::from(run).join("result.litzwave");
    let wave = litzsim_core::waves::read(&result).unwrap();
    let rows: Vec<_> = wave.rows().collect();
    let peak = (0..rows.len())
        .max_by(|&a, &b| rows[a][1].total_cmp(&rows[b][1]))
        .unwrap();
    for (from, to, pixels) in [(0, rows.len() - 1, "80"), (peak - 20, peak + 20, "5")] {
        let (from_time, to_time) = (rows[from][0], rows[to][0]);
        let envelope = wave_request(
            &folder,
            run,
            true,
            &from_time.to_string(),
            &to_time.to_string(),
            pixels,
        );
        assert_eq!(envelope["timeRange"], json!([from_time, to_time]));
        for (channel, column, name, unit) in [(0, 1, "L1_A", "A"), (1, 2, "VC_V", "V")] {
            let series = &envelope["channels"][channel];
            assert_eq!(series["name"], name);
            assert_eq!(series["unit"], unit);
            let mut points = Vec::new();
            for point in series["points"].as_array().unwrap() {
                let row = point["row"].as_u64().unwrap() as usize;
                assert!((from..=to).contains(&row));
                assert_eq!(point["t_s"], rows[row][0]);
                assert_eq!(point["value"], rows[row][column]);
                points.push(row);
            }
            assert!(points.windows(2).all(|pair| pair[0] < pair[1]));
            let bucket_width = (to_time - from_time) / pixels.parse::<f64>().unwrap();
            let extrema = |low: f64, high: f64, last_bucket: bool| {
                let indices = (from..=to).filter(|&row| {
                    rows[row][0] >= low
                        && (rows[row][0] < high || last_bucket && rows[row][0] == high)
                });
                let min = indices
                    .clone()
                    .min_by(|&a, &b| rows[a][column].total_cmp(&rows[b][column]));
                let max = indices.max_by(|&a, &b| rows[a][column].total_cmp(&rows[b][column]));
                (min, max)
            };
            let (min, max) = extrema(from_time, to_time, true);
            assert_eq!(series["minimum"], rows[min.unwrap()][column]);
            assert_eq!(series["maximum"], rows[max.unwrap()][column]);
            let bins = pixels.parse::<u32>().unwrap();
            for bucket in 0..bins {
                let low = from_time + f64::from(bucket) * bucket_width;
                let high = from_time + f64::from(bucket + 1) * bucket_width;
                let (min, max) = extrema(low, high, bucket + 1 == bins);
                for index in [min, max].into_iter().flatten() {
                    assert!(
                        points
                            .iter()
                            .any(|&point| rows[point][column] == rows[index][column]),
                        "missing extremum for {name} in bucket {bucket}: row {index}"
                    );
                }
            }
            if column == 1 {
                // The current peak sits on a switching edge, recorded as left and
                // right limit rows at one time with the same (continuous) value.
                let tops: Vec<_> = (from..=to)
                    .filter(|&row| rows[row][column] == rows[peak][column])
                    .collect();
                assert!(tops.iter().all(|&row| rows[row][0] == rows[peak][0]));
                assert!(tops.len() <= 2);
                let (first, last) = (tops[0], *tops.last().unwrap());
                assert!((first - 1..=last + 1).all(|row| points.contains(&row)));
            }
        }
    }

    // Cursor rows and interval statistics, against the rows joined by lines.
    let cursors = [rows[peak][0] - 3.3e-7, rows[rows.len() / 2][0] + 1.0e-8];
    let (a, b) = (cursors[0].min(cursors[1]), cursors[0].max(cursors[1]));
    let measured = folder.run(
        &[
            "measure",
            run,
            "--channel",
            "L1_A",
            "--channel",
            "VC_V",
            "--from",
            &a.to_string(),
            "--to",
            &b.to_string(),
        ],
        true,
    );
    assert_eq!(measured["timeRange"], json!([a, b]));
    assert_eq!(measured["deltaTime_s"], b - a);
    assert_eq!(measured["frequency_Hz"], 1.0 / (b - a));
    // Nearest row; of two rows at one (edge) time, the one on the cursor's side,
    // and the left limit when the cursor is exactly on the edge.
    let nearest = |t: f64| {
        (0..rows.len())
            .min_by(|&x, &y| {
                (rows[x][0] - t)
                    .abs()
                    .total_cmp(&(rows[y][0] - t).abs())
                    .then(if rows[x][0] < t { y.cmp(&x) } else { x.cmp(&y) })
            })
            .unwrap()
    };
    for (channel, column) in [(0, 1), (1, 2)] {
        let stats = &measured["channels"][channel];
        assert_eq!(
            stats["delta"].as_f64().unwrap(),
            stats["to"]["value"].as_f64().unwrap() - stats["from"]["value"].as_f64().unwrap()
        );
        for (cursor, time) in [("from", a), ("to", b)] {
            let row = nearest(time);
            assert_eq!(stats[cursor]["row"], row);
            assert_eq!(stats[cursor]["t_s"], rows[row][0]);
            assert_eq!(stats[cursor]["value"], rows[row][column]);
        }
        let lerp = |t: f64| {
            let i = rows.iter().position(|row| row[0] > t).unwrap();
            let (p, q) = (rows[i - 1], rows[i]);
            p[column] + (q[column] - p[column]) * (t - p[0]) / (q[0] - p[0])
        };
        // Fine trapezoids of the joined lines converge on the exact integrals.
        let steps = 400_000;
        let (mut area, mut square) = (0.0, 0.0);
        for k in 0..steps {
            let (t0, t1) = (
                a + (b - a) * k as f64 / steps as f64,
                a + (b - a) * (k + 1) as f64 / steps as f64,
            );
            let (v0, v1) = (lerp(t0), lerp(t1));
            area += (t1 - t0) * (v0 + v1) / 2.0;
            square += (t1 - t0) * (v0 * v0 + v1 * v1) / 2.0;
        }
        let average = area / (b - a);
        let rms = (square / (b - a)).sqrt();
        let scale = rows.iter().map(|row| row[column].abs()).fold(0.0, f64::max);
        assert!((stats["average"].as_f64().unwrap() - average).abs() < 1e-6 * scale);
        assert!((stats["rms"].as_f64().unwrap() - rms).abs() < 1e-4 * scale);
        let inside = rows.iter().filter(|row| row[0] > a && row[0] < b);
        let maximum = inside
            .clone()
            .map(|row| row[column])
            .fold(lerp(a).max(lerp(b)), f64::max);
        let minimum = inside
            .map(|row| row[column])
            .fold(lerp(a).min(lerp(b)), f64::min);
        assert_eq!(stats["maximum"], maximum);
        assert_eq!(stats["minimum"], minimum);
    }
    assert_eq!(measured["channels"][0]["maximum"], rows[peak][1]);

    let mut broken = buck;
    broken["nets"]["output"]
        .as_array_mut()
        .unwrap()
        .retain(|pin| pin != "VC.p");
    std::fs::write(folder.0.join("broken.litzsim"), broken.to_string()).unwrap();
    let started = folder.run(&["start", "broken.litzsim"], true);
    let failed_dir = started["runDirectory"].as_str().unwrap();
    let failed = terminal(&folder, failed_dir, "failed");
    assert_eq!(failed["error"]["targetId"], "VC.p");
    assert_eq!(
        wave_request(&folder, failed_dir, false, "0", "1", "10")["error"]["code"],
        "runNotCompleted"
    );
    // The folder's runs, newest first; only the completed one describes a result.
    let listed = folder.run(&["runs", "."], true);
    let listed = listed["runs"].as_array().unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0]["runId"], started["runId"]);
    assert_eq!(listed[0]["state"], "failed");
    assert!(listed[0].get("channels").is_none());
    assert_eq!(listed[1]["state"], "completed");
    assert_eq!(listed[1]["revision"], completed["revision"]);
    assert_eq!(listed[1]["rows"], rows.len());
    assert_eq!(
        listed[1]["timeRange"],
        json!([rows[0][0], rows[rows.len() - 1][0]])
    );
    assert_eq!(listed[1]["channels"], json!(wave.channels()[1..]));
    assert!(listed[0]["startedAt"].as_f64() >= listed[1]["startedAt"].as_f64());
    let empty = TestDirectory::new("no-runs");
    assert_eq!(empty.run(&["runs", "."], true)["runs"], json!([]));

    let files = std::fs::read_dir(failed_dir).unwrap();
    assert!(files.map(|entry| entry.unwrap().file_name()).all(|name| {
        name == "status.json" || name == "status.lock" || name == "circuit.litzsim"
    }));
}

#[test]
fn serve_owns_run_lifetime_and_checks_the_working_revision() {
    let folder = TestDirectory::new("serve-runs");
    let mut serve = Command::new(env!("CARGO_BIN_EXE_litzsim"))
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = serve.stdin.take().unwrap();
    let mut output = BufReader::new(serve.stdout.take().unwrap());
    let mut request = |json: Value| -> Value {
        writeln!(input, "{json}").unwrap();
        let mut response = String::new();
        output.read_line(&mut response).unwrap();
        serde_json::from_str(&response).unwrap()
    };
    let mut working: Value =
        serde_json::from_slice(include_bytes!("../../../examples/rc.litzsim")).unwrap();
    working["simulation"]["stopTime_s"] = json!(100_000.0);
    working["simulation"]["outputStep_s"] = json!(1.0);
    let inspection = request(json!({ "op": "inspect", "document": working }));
    let mut start = json!({
        "op": "start", "directory": folder.0, "document": working, "expectedRevision": "stale",
    });
    assert_eq!(request(start.clone())["error"]["code"], "revisionConflict");
    start["expectedRevision"] = inspection["revision"].clone();
    let started = request(start);
    assert_eq!(started["state"], "running", "{started}");
    let missing = request(json!({
        "op": "status", "runId": "1-2-3", "directory": folder.0,
    }));
    assert_eq!(missing["error"]["code"], "unknownRun");
    let status = request(json!({ "op": "status", "runId": started["runId"] }));
    assert_eq!(status["stopTime"], 100_000.0);
    assert_eq!(status["revision"], inspection["revision"]);
    drop(input);
    assert!(serve.wait().unwrap().success());
    let run = started["runDirectory"].as_str().unwrap();
    assert_eq!(folder.run(&["status", run], true)["state"], "canceled");
    assert!(!PathBuf::from(run).join("result.pending").exists());
}
