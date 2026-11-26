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
    let buck = include_bytes!("../../../examples/buck.litzsim");
    std::fs::write(folder.0.join("buck.litzsim"), buck).unwrap();
    let started = folder.run(&["start", "buck.litzsim"], true);
    let run = started["runDirectory"].as_str().unwrap();
    let completed = terminal(&folder, run, "completed");
    assert_eq!(completed["simulatedTime"], completed["stopTime"]);
    let result = PathBuf::from(run).join("result.litzwave");
    let wave = litzsim_core::waves::read(&result).unwrap();
    let rows: Vec<_> = wave.rows().collect();
    let peak = (0..rows.len())
        .max_by(|&a, &b| rows[a][2].total_cmp(&rows[b][2]))
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
        for (channel, column, name, unit) in [(0, 2, "L1_A", "A"), (1, 3, "VC_V", "V")] {
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
            if column == 2 {
                assert!(rows[from..=to].iter().enumerate().all(|(index, row)| {
                    index + from == peak || row[column] < rows[peak][column]
                }));
                assert!((peak - 1..=peak + 1).all(|row| points.contains(&row)));
            }
        }
    }

    let mut broken: Value = serde_json::from_slice(buck).unwrap();
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
    let status = request(json!({ "op": "status", "runId": started["runId"] }));
    assert_eq!(status["stopTime"], 100_000.0);
    assert_eq!(status["revision"], inspection["revision"]);
    drop(input);
    assert!(serve.wait().unwrap().success());
    let run = started["runDirectory"].as_str().unwrap();
    assert_eq!(folder.run(&["status", run], true)["state"], "canceled");
    assert!(!PathBuf::from(run).join("result.pending").exists());
}
