use super::*;
use serde_json::json;
use std::fs;
use std::sync::Barrier;

const RC: &[u8] = include_bytes!("../../../../examples/rc.litzsim");
const RLC: &[u8] = include_bytes!("../../../../examples/rlc.litzsim");
const BUCK: &[u8] = include_bytes!("../../../../examples/buck.litzsim");
const PFC: &[u8] = include_bytes!("../../../../examples/pfc.litzsim");

#[test]
fn archived_examples_keep_v1_data_and_have_canonical_bytes() {
    // SHA-256 of the canonical v1 output captured before adding mirror support.
    for (bytes, before_mirror) in [
        (
            RC,
            "beef73be34c48f491566516f2299bccc649ebff4d200850f6e5dd8f138e6ef79",
        ),
        (
            RLC,
            "8dd84ee9b327a2183e8f9329e20426a3f2c9cd47fe261bbb980f6cf13a883cba",
        ),
        (
            BUCK,
            "16799acdc0f0bd193fa5f3f61b46b01e5f125ddd1ae7fbefe119a263b9ee8f5e",
        ),
    ] {
        let original = Document::from_bytes(bytes).unwrap();
        let saved = original.to_bytes();
        let reopened = Document::from_bytes(&saved).unwrap();
        assert_eq!(original, reopened);
        assert_eq!(saved, reopened.to_bytes());
        assert_eq!(revision(&saved), before_mirror);
        let mut source: Value = serde_json::from_slice(bytes).unwrap();
        let mut canonical: Value = serde_json::from_slice(&saved).unwrap();
        normalize_numbers(&mut source);
        normalize_numbers(&mut canonical);
        assert_eq!(source, canonical);
        assert!(reopened.inspect().missing_pins.is_empty());
        let inspection = json!(reopened.inspect());
        assert_eq!(
            inspection["measurements"]["VC"],
            json!({
                "quantity": "voltage", "unit": "V", "fromPin": "p", "toPin": "n"
            })
        );
    }

    let alternative_spelling = std::str::from_utf8(RC)
        .unwrap()
        .replace("0.00001", "1e-5")
        .replace(r#""voltage_V": 5"#, r#""voltage_V": 5e0"#);
    assert_eq!(
        Document::from_bytes(RC).unwrap().revision(),
        Document::from_bytes(alternative_spelling.as_bytes())
            .unwrap()
            .revision()
    );

    let mut settings: Value = serde_json::from_slice(BUCK).unwrap();
    settings["simulation"]["outputStep_s"] = json!(4e-7);
    let with_stride = Document::from_bytes(&serde_json::to_vec(&settings).unwrap()).unwrap();
    assert_eq!(
        json!(with_stride)["simulation"]["outputStep_s"],
        json!(4e-7)
    );
}

#[test]
fn resistance_defaults_stay_out_of_files_and_remain_editable() {
    let original = Document::from_bytes(RC).unwrap();
    let inspection = json!(original.inspect());
    assert_eq!(
        inspection["document"]["parts"]["C1"]["parameters"]["esr_ohm"],
        json!(1e-6)
    );
    assert_eq!(
        inspection["document"]["parts"]["V1"]["parameters"]["internalResistance_ohm"],
        json!(0.0)
    );
    let mut explicit: Value = serde_json::from_slice(RC).unwrap();
    explicit["parts"]["C1"]["parameters"]["esr_ohm"] = json!(1e-6);
    explicit["parts"]["V1"]["parameters"]["internalResistance_ohm"] = json!(0);
    let explicit = Document::from_bytes(&serde_json::to_vec(&explicit).unwrap()).unwrap();
    assert_eq!(explicit.to_bytes(), original.to_bytes());

    let set_esr = |document: &Document, value| {
        apply(
            document,
            &document.revision(),
            &[Operation::SetParameter {
                id: "C1".into(),
                key: "esr_ohm".into(),
                value,
            }],
        )
        .unwrap()
        .document
    };
    let ideal = set_esr(&original, 0.0);
    let saved: Value = serde_json::from_slice(&ideal.to_bytes()).unwrap();
    assert_eq!(saved["parts"]["C1"]["parameters"]["esr_ohm"], json!(0.0));
    let reopened = Document::from_bytes(&ideal.to_bytes()).unwrap();
    assert_eq!(set_esr(&reopened, 1e-6).to_bytes(), original.to_bytes());
}

#[test]
fn mirrors_across_sheet_axes_at_every_rotation_and_restores_on_undo() {
    let original = Document::from_bytes(BUCK).unwrap();
    let mut rotated = original.clone();
    let id = "Q1".to_owned();
    let rotate = || Operation::Rotate { id: id.clone() };
    let mirror = |axis| Operation::Mirror {
        id: id.clone(),
        axis,
    };
    // Q1 has its center at [8, 2]. D, G and S have asymmetric offsets.
    for expected in [
        [[8, 1], [7, 2], [8, 3]],
        [[9, 2], [8, 1], [7, 2]],
        [[8, 3], [9, 2], [8, 1]],
        [[7, 2], [8, 3], [9, 2]],
    ] {
        let pins = |document: &Document| {
            let position = document.inspect().pin_positions;
            [position["Q1.d"], position["Q1.g"], position["Q1.s"]]
        };
        assert_eq!(pins(&rotated), expected);
        let rev = rotated.revision();
        let next_rotation = apply(&rotated, &rev, &[rotate()]).unwrap().document;
        for (axis, field) in [
            (MirrorAxis::Horizontal, "horizontal"),
            (MirrorAxis::Vertical, "vertical"),
        ] {
            let result = apply(&rotated, &rev, &[mirror(axis)]).unwrap();
            assert_eq!(result.receipt.modified, ["Q1"]);
            assert!(result.receipt.nets.is_empty());
            assert_eq!(json!(result.document)["layout"]["Q1"]["mirror"], field);
            let expected_flip = expected.map(|[x, y]| match axis {
                MirrorAxis::Horizontal => [16 - x, y],
                MirrorAxis::Vertical => [x, 4 - y],
            });
            let reopened = Document::from_bytes(&result.document.to_bytes()).unwrap();
            assert_eq!(reopened, result.document);
            assert_eq!(pins(&reopened), expected_flip);
            let turned = apply(&reopened, &reopened.revision(), &[rotate()])
                .unwrap()
                .document;
            assert_eq!(pins(&turned), expected_flip.map(|[x, y]| [10 - y, x - 6]));
            let undone = undo(&reopened, &result.receipt.after_revision, &rev, &rotated).unwrap();
            assert_eq!(undone.document.to_bytes(), rotated.to_bytes());
            let toggled = apply(&reopened, &reopened.revision(), &[mirror(axis)]).unwrap();
            assert_eq!(toggled.document.to_bytes(), rotated.to_bytes());
        }
        let combined = apply(
            &rotated,
            &rev,
            &[mirror(MirrorAxis::Horizontal), mirror(MirrorAxis::Vertical)],
        )
        .unwrap();
        assert!(
            json!(combined.document)["layout"]["Q1"]
                .get("mirror")
                .is_none()
        );
        assert_eq!(
            pins(&combined.document),
            expected.map(|[x, y]| [16 - x, 4 - y])
        );
        rotated = next_rotation;
    }
}

#[test]
fn canonical_orientation_elides_zero_and_folds_double_reflection() {
    let original = Document::from_bytes(BUCK).unwrap();
    let revision = original.revision();
    let rotate = || Operation::Rotate { id: "Q1".into() };
    let ops = [rotate(), rotate(), rotate(), rotate()];
    let cycled = apply(&original, &revision, &ops).unwrap();
    assert_eq!(cycled.document.to_bytes(), original.to_bytes());
    assert_eq!(cycled.receipt.after_revision, revision);
    assert!(cycled.receipt.modified.is_empty());

    let mut input = json!(original);
    input["layout"]["Q1"]["rotation"] = json!(0);
    let explicit_zero = Document::from_bytes(&serde_json::to_vec(&input).unwrap()).unwrap();
    assert_eq!(explicit_zero.to_bytes(), original.to_bytes());

    for turns in 0..4 {
        input["layout"]["Q1"]["rotation"] = json!(turns * 90);
        input["layout"]["Q1"]["mirror"] = json!("both");
        let mirrored = Document::from_bytes(&serde_json::to_vec(&input).unwrap()).unwrap();
        let equivalent = apply(&original, &revision, &ops[..(turns + 2) % 4]).unwrap();
        assert_eq!(mirrored.revision(), equivalent.receipt.after_revision);
        assert_eq!(mirrored.to_bytes(), equivalent.document.to_bytes());
    }
}

fn normalize_numbers(value: &mut Value) {
    match value {
        Value::Object(fields) => fields.values_mut().for_each(normalize_numbers),
        Value::Array(items) => items.iter_mut().for_each(normalize_numbers),
        Value::Number(number) => *value = number.as_f64().unwrap().into(),
        _ => {}
    }
}

#[test]
fn rejects_pfc_and_structural_errors_without_silent_data_loss() {
    let error = Document::from_bytes(PFC).unwrap_err();
    assert_eq!(error.code, "invalidCircuit");
    assert!(error.message.contains("unknown field") && error.message.contains("control"));

    let rc = std::str::from_utf8(RC).unwrap();
    for (old, new, id, reason) in [
        (r#""dc""#, r#""sink""#, Some("V1"), "unsupported part type"),
        (
            r#""voltage_V""#,
            r#""voltage""#,
            Some("V1"),
            "unknown parameter",
        ),
        (
            r#""R1.p""#,
            r#""R1.bad""#,
            Some("R1.bad"),
            "unknown terminal",
        ),
        (
            r#""R1.p""#,
            r#""V1.p""#,
            Some("V1.p"),
            "multiply connected terminal",
        ),
        (
            r#""voltage_V": 5"#,
            r#""voltage_V": 4, "voltage_V": 5"#,
            None,
            "duplicate key",
        ),
    ] {
        let bytes = rc.replacen(old, new, 1);
        let error = Document::from_bytes(bytes.as_bytes()).unwrap_err();
        assert_eq!(error.target_id.as_deref(), id);
        assert!(error.message.contains(reason), "{error}");
    }
}

#[test]
fn new_parts_take_short_designators_and_skip_used_numbers() {
    let rc = Document::from_bytes(RC).unwrap();
    let add = |kind: &str| Operation::Add {
        kind: kind.into(),
        grid: None,
    };
    let kinds = [
        "resistor",
        "dc",
        "sine",
        "inductor",
        "capacitor",
        "diode",
        "nmos",
        "pwm",
        "voltmeter",
        "ammeter",
        "ground",
    ];
    let result = apply(&rc, &rc.revision(), &kinds.map(add)).unwrap();
    // RC already has V1, R1, C1, GND and VC; those IDs stay and numbers are skipped.
    assert_eq!(
        result.receipt.created,
        [
            "AM1", "C2", "D1", "GND1", "L1", "PWM1", "Q1", "R2", "V2", "V3", "VM1"
        ]
    );
    for id in ["V1", "R1", "C1", "GND", "VC"] {
        assert_eq!(result.document.0.parts[id], rc.0.parts[id]);
    }
    let inserted = apply(
        &rc,
        &rc.revision(),
        &[Operation::InsertInBranch {
            kind: "ammeter".into(),
            terminals: ["R1.n".into(), "C1.p".into()],
            from: [160, 32],
            to: [256, 32],
        }],
    )
    .unwrap();
    assert_eq!(inserted.receipt.created, ["AM1"]);
}

#[test]
fn builds_archived_buck_topology_using_only_apply() {
    let buck = Document::from_bytes(BUCK).unwrap();
    let mut names = BTreeMap::new();
    let mut ops = Vec::new();
    for (old_id, part) in &buck.0.parts {
        let pos = buck.0.layout[old_id];
        let new_id = format!("{}1", part_definition(&part.kind).unwrap().id_prefix);
        names.insert(old_id.as_str(), new_id.clone());
        ops.push(Operation::Add {
            kind: part.kind.clone(),
            grid: Some([pos.x, pos.y]),
        });
        for _ in 0..pos.rotation.unwrap_or_default() / 90 {
            ops.push(Operation::Rotate { id: new_id.clone() });
        }
        for (key, value) in &part.parameters {
            ops.push(Operation::SetParameter {
                id: new_id.clone(),
                key: key.clone(),
                value: value.as_f64().unwrap(),
            });
        }
    }
    let mut expected_nets = Vec::new();
    for pins in buck.0.nets.values() {
        let mut mapped: Vec<_> = pins
            .iter()
            .map(|pin| {
                let (id, name) = pin.split_once('.').unwrap();
                format!("{}.{name}", names[id])
            })
            .collect();
        for pin in &mapped[1..] {
            ops.push(Operation::Connect {
                a: mapped[0].clone(),
                b: pin.clone(),
            });
        }
        mapped.sort();
        expected_nets.push(mapped);
    }
    expected_nets.sort();
    let empty = Document::new();
    let result = apply(&empty, &empty.revision(), &ops).unwrap();
    let mut actual: Vec<_> = result.document.0.nets.values().cloned().collect();
    actual.iter_mut().for_each(|pins| pins.sort());
    actual.sort();
    assert_eq!(actual, expected_nets);
    assert_eq!(result.receipt.created.len(), buck.0.parts.len());
    assert!(result.document.inspect().missing_pins.is_empty());
    for (old_id, new_id) in names {
        assert_eq!(result.document.0.parts[&new_id], buck.0.parts[old_id]);
        assert_eq!(result.document.0.layout[&new_id], buck.0.layout[old_id]);
    }
}

#[test]
fn batch_is_atomic_and_receipt_tracks_net_merges_deletes_and_undo() {
    let document = Document::from_bytes(BUCK).unwrap();
    let revision = document.revision();
    let mut ops = vec![
        Operation::Connect {
            a: "V1.p".into(),
            b: "Q1.s".into(),
        },
        Operation::Disconnect { pin: "Q1.s".into() },
        Operation::Move {
            id: "PWM".into(),
            grid: [6, 4],
        },
        Operation::Delete { id: "Q1".into() },
        Operation::Connect {
            a: "L1.p".into(),
            b: "Q1.bad".into(),
        },
    ];
    let error = apply(&document, &revision, &ops).unwrap_err();
    assert_eq!(error.code, "operationFailed");
    assert_eq!(error.operation, Some(5));
    assert_eq!(error.target_id.as_deref(), Some("Q1.bad"));
    assert_eq!(document, Document::from_bytes(BUCK).unwrap());
    assert_eq!(
        apply(&document, "old", &[]).unwrap_err().code,
        "revisionConflict"
    );

    ops.pop();
    let edited = apply(&document, &revision, &ops).unwrap();
    assert_eq!(edited.receipt.before_revision, revision);
    assert_eq!(edited.receipt.modified, ["PWM"]);
    assert_eq!(edited.receipt.deleted, ["Q1"]);
    assert_eq!(
        json!(edited.receipt.nets["input"]),
        json!({
            "before": ["V1.p", "Q1.d"], "after": ["V1.p", "PWM.ref", "D1.k", "L1.p"]
        })
    );
    assert_eq!(
        json!(edited.receipt.nets["sw"]),
        json!({
            "before": ["Q1.s", "PWM.ref", "D1.k", "L1.p"], "after": null
        })
    );
    assert_eq!(
        json!(edited.receipt.nets["gate"]),
        json!({
            "before": ["PWM.out", "Q1.g"], "after": ["PWM.out"]
        })
    );
    assert!(!edited.document.0.layout.contains_key("Q1"));
    assert!(edited.document.0.bends.as_ref().unwrap().is_empty());

    let disconnected = apply(
        &edited.document,
        &edited.receipt.after_revision,
        &[Operation::Disconnect {
            pin: "PWM.out".into(),
        }],
    )
    .unwrap();
    assert_eq!(disconnected.document.inspect().missing_pins, ["PWM.out"]);
    assert!(Document::from_bytes(&disconnected.document.to_bytes()).is_ok());
    assert_eq!(
        json!(disconnected.receipt.nets["gate"]),
        json!({
            "before": ["PWM.out"], "after": null
        })
    );

    assert_eq!(
        undo(&edited.document, &revision, &revision, &document)
            .unwrap_err()
            .code,
        "revisionConflict"
    );
    let undone = undo(
        &edited.document,
        &edited.receipt.after_revision,
        &revision,
        &document,
    )
    .unwrap();
    assert_eq!(undone.document.to_bytes(), document.to_bytes());
    assert_eq!(undone.receipt.created, ["Q1"]);
    assert_eq!(undone.receipt.after_revision, revision);
    assert_eq!(
        undone.receipt.nets["input"].after,
        edited.receipt.nets["input"].before
    );

    let setting = |key: &str, value| Operation::SetSimulation {
        key: key.into(),
        value,
    };
    let timed = apply(
        &document,
        &revision,
        &[setting("stopTime_s", 10.0), setting("outputStep_s", 1e-3)],
    )
    .unwrap();
    let saved = Document::from_bytes(&timed.document.to_bytes()).unwrap();
    assert_eq!(
        json!(saved)["simulation"],
        json!({ "timeStep_s": 1e-7, "stopTime_s": 10.0, "outputStep_s": 1e-3 })
    );
    for (ops, target) in [
        (vec![setting("timeStep_s", 0.0)], "timeStep_s"),
        (vec![setting("outputStep_s", 1.5e-7)], "outputStep_s"),
        (vec![setting("model", 1.0)], "simulation"),
    ] {
        let error = apply(&document, &revision, &ops).unwrap_err();
        assert_eq!(error.code, "operationFailed");
        assert_eq!(error.target_id.as_deref(), Some(target));
    }
}

struct SaveTestDirectory(std::path::PathBuf);

impl SaveTestDirectory {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("litzsim-core-{name}-{}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for SaveTestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn save_guards_raw_bytes_and_serializes_competing_writers() {
    let folder = SaveTestDirectory::new("save");
    let directory = &folder.0;
    let path = directory.join("test.litzsim");
    let document = Document::from_bytes(RC).unwrap();
    let initial = save(&path, &document, SaveMode::New).unwrap();
    let only_circuit = || {
        let names: Vec<_> = fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["test.litzsim"]);
    };
    only_circuit();
    assert_eq!(initial, open(&path).unwrap().disk_revision);
    assert_eq!(
        save(&path, &Document::new(), SaveMode::New)
            .unwrap_err()
            .code,
        "fileExists"
    );
    only_circuit();
    assert_eq!(fs::read(&path).unwrap(), document.to_bytes());

    // Similar canonical content, different raw bytes.
    fs::write(&path, RC).unwrap();
    let current = open(&path).unwrap();
    assert_eq!(initial, current.revision);
    assert_ne!(initial, current.disk_revision);
    let error = save(
        &path,
        &document,
        SaveMode::Replace {
            expected_disk_revision: &initial,
        },
    )
    .unwrap_err();
    assert_eq!(error.code, "diskRevisionConflict");
    assert_eq!(error.expected, Some(json!(initial)));
    assert_eq!(error.actual, Some(json!(current.disk_revision)));
    assert_eq!(fs::read(&path).unwrap(), RC);

    let barrier = Barrier::new(2);
    let results = std::thread::scope(|scope| {
        let write = |grid| {
            let result = apply(
                &document,
                &initial,
                &[Operation::Move {
                    id: "V1".into(),
                    grid,
                }],
            )
            .unwrap();
            barrier.wait();
            save(
                &path,
                &result.document,
                SaveMode::Replace {
                    expected_disk_revision: &current.disk_revision,
                },
            )
        };
        let one = scope.spawn(move || write([1, 1]));
        let two = scope.spawn(move || write([2, 2]));
        [one.join().unwrap(), two.join().unwrap()]
    });
    let actual = open(&path).unwrap().disk_revision;
    assert_eq!(
        results
            .iter()
            .filter_map(|result| result.as_ref().ok())
            .collect::<Vec<_>>(),
        [&actual],
        "{results:?}"
    );
    let error = results
        .iter()
        .find_map(|result| result.as_ref().err())
        .unwrap();
    assert_eq!(error.code, "diskRevisionConflict", "{results:?}");
    assert_eq!(error.actual, Some(json!(actual)), "{results:?}");
    only_circuit();

    let missing = save(
        &directory.join("missing.litzsim"),
        &document,
        SaveMode::Replace {
            expected_disk_revision: &actual,
        },
    )
    .unwrap_err();
    assert_eq!(missing.code, "diskRevisionConflict");
    assert_eq!(missing.actual, Some(serde_json::Value::Null));
}

#[cfg(windows)]
#[test]
fn replacement_succeeds_after_a_reader_releases_its_handle() {
    use super::file::replace_file;
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_SHARE_READ: u32 = 0x1;
    const FILE_SHARE_WRITE: u32 = 0x2;
    let folder = SaveTestDirectory::new("released-reader");
    let path = folder.0.join("circuit.litzsim");
    fs::write(&path, RC).unwrap();
    let current = open(&path).unwrap();
    let mut reader = Some(
        fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&path)
            .unwrap(),
    );
    let replacement = Document::new().to_bytes();
    let staged =
        StagedFile::write(&path, |file| std::io::Write::write_all(file, &replacement)).unwrap();
    let started = std::time::Instant::now();
    replace_file(
        &staged.0,
        &path,
        &SaveMode::Replace {
            expected_disk_revision: &current.disk_revision,
        },
        |from, to| {
            let result = fs::rename(from, to);
            if let Some(held) = reader.take() {
                let error = result.as_ref().unwrap_err();
                assert!(matches!(error.raw_os_error(), Some(5 | 32)), "{error}");
                drop(held);
            }
            result
        },
    )
    .unwrap();
    assert!(reader.is_none());
    assert!(started.elapsed() >= REPLACE_RETRY_DELAYS[0]);
    assert_eq!(fs::read(&path).unwrap(), replacement);
    assert!(!staged.0.exists());
}

#[cfg(windows)]
#[test]
fn windows_replacement_checks_for_changes_before_retrying() {
    use super::file::replace_file;
    use std::io;

    let folder = SaveTestDirectory::new("changed-during-retry");
    let path = folder.0.join("circuit.litzsim");
    fs::write(&path, RC).unwrap();
    let current = open(&path).unwrap();
    let mode = SaveMode::Replace {
        expected_disk_revision: &current.disk_revision,
    };
    let staged = StagedFile::write(&path, |_| Ok(())).unwrap();
    let mut attempts = 0;
    let unsupported = replace_file(&staged.0, &path, &mode, |_, _| {
        attempts += 1;
        Err(io::Error::from_raw_os_error(2))
    })
    .unwrap_err();
    assert_eq!(unsupported.code, "ioError");
    assert_eq!(attempts, 1);

    let conflict = replace_file(&staged.0, &path, &mode, |_, _| {
        attempts += 1;
        fs::write(&path, RLC).unwrap();
        Err(io::Error::from_raw_os_error(32))
    })
    .unwrap_err();
    assert_eq!(conflict.code, "diskRevisionConflict");
    assert_eq!(conflict.actual, Some(json!(revision(RLC))));
    assert_eq!(attempts, 2);
    assert_eq!(fs::read(&path).unwrap(), RLC);
}

#[cfg(windows)]
#[test]
fn windows_save_discards_the_ntfs_deleted_path_of_a_replaced_file() {
    use super::file::save_path;
    use std::path::{Path, PathBuf};

    let requested = Path::new(r"C:\circuits\example.litzsim");
    let mut lookups = [
        PathBuf::from(r"\\?\C:\$Extend\$Deleted\001A-Replaced"),
        PathBuf::from(r"\\?\C:\circuits\example.litzsim"),
    ]
    .into_iter();
    let destination = save_path(requested, |path| {
        assert_eq!(path, requested);
        Ok(lookups.next().expect("expected two lookups"))
    })
    .unwrap();
    assert_eq!(destination, Path::new(r"\\?\C:\circuits\example.litzsim"));
    assert!(lookups.next().is_none());
}

#[test]
fn receipt_reports_voltage_sources_newly_shorted_by_the_edit() {
    let rc = Document::from_bytes(RC).unwrap();
    let connect = |a: &str, b: &str| Operation::Connect {
        a: a.into(),
        b: b.into(),
    };
    // Review repro: R1.p (input) to V1.n (ground) merges both V1 terminals.
    let shorted = apply(&rc, &rc.revision(), &[connect("R1.p", "V1.n")]).unwrap();
    assert_eq!(shorted.receipt.shorted_sources, ["V1"]);
    assert_eq!(json!(shorted.receipt)["shortedSources"], json!(["V1"]));
    // Merging input with output leaves V1 across input and ground.
    let merged = apply(&rc, &rc.revision(), &[connect("R1.p", "C1.p")]).unwrap();
    assert!(merged.receipt.shorted_sources.is_empty());
    // An existing short is not reported again by an unrelated edit, nor on undo's way back.
    let later = apply(
        &shorted.document,
        &shorted.receipt.after_revision,
        &[Operation::Rotate { id: "R1".into() }],
    )
    .unwrap();
    assert!(later.receipt.shorted_sources.is_empty());
    let undone = undo(
        &shorted.document,
        &shorted.receipt.after_revision,
        &rc.revision(),
        &rc,
    )
    .unwrap();
    assert!(undone.receipt.shorted_sources.is_empty());
    // PWM forces its output too; a new source wired to itself is a short.
    let pwm = apply(
        &rc,
        &rc.revision(),
        &[
            Operation::Add {
                kind: "pwm".into(),
                grid: None,
            },
            connect("PWM1.out", "PWM1.ref"),
        ],
    )
    .unwrap();
    assert_eq!(pwm.receipt.shorted_sources, ["PWM1"]);
}
