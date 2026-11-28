use crate::runs::{self, Runs};
use litzsim_core::circuit::{self, Document, Operation, SaveMode};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::{Value, json};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

pub fn file_command(args: &[OsString]) -> Result<Value, Value> {
    let (result, disk_revision) = match args {
        [cmd, file, flag] if cmd == "inspect" && flag == "--json" => {
            return inspect_file(Path::new(file));
        }
        [cmd, file, flag] if cmd == "layout" && flag == "--json" => {
            let opened = circuit::open(Path::new(file)).map_err(core_error)?;
            return Ok(success(litzsim_core::layout::layout(&opened.document)));
        }
        [cmd, file, out] if cmd == "run" => return run(Path::new(file), Path::new(out)),
        [cmd, file] if cmd == "start" => return runs::start_file(file),
        [cmd, path] if cmd == "status" => return runs::status(Path::new(path)),
        [cmd, path] if cmd == "stop" => return runs::stop(Path::new(path)),
        [cmd, path] if cmd == "pin" => return runs::pin(Path::new(path), true),
        [cmd, path] if cmd == "unpin" => return runs::pin(Path::new(path), false),
        [cmd, path] if cmd == "delete" => return runs::delete(Path::new(path)),
        [cmd, path, options @ ..] if cmd == "wave" => return wave_file(path.into(), options),
        [cmd, path, options @ ..] if cmd == "measure" => {
            return measure_file(path.into(), options);
        }
        [cmd, path] if cmd == "runs" => {
            let path = Path::new(path);
            return if path.is_file() {
                runs::list_circuit(path)
            } else {
                runs::list(path, None)
            };
        }
        [cmd, file, expect, revision, option, input]
            if cmd == "apply" && expect == "--expect" && option == "--ops" =>
        {
            let path = Path::new(file);
            let bytes = read_input(input)?;
            let ops = parse_ops(&bytes)?;
            let opened = circuit::open(path).map_err(core_error)?;
            let result =
                circuit::apply(&opened.document, text_arg(revision)?, &ops).map_err(core_error)?;
            let disk_revision = circuit::save(
                path,
                &result.document,
                SaveMode::Replace {
                    expected_disk_revision: &opened.disk_revision,
                },
            )
            .map_err(core_error)?;
            (result, disk_revision)
        }
        [
            cmd,
            file,
            expect,
            after,
            before_flag,
            before,
            previous_flag,
            input,
        ]
        | [
            cmd,
            file,
            expect,
            after,
            previous_flag,
            input,
            before_flag,
            before,
        ] if cmd == "undo"
            && expect == "--expect"
            && before_flag == "--before"
            && previous_flag == "--previous" =>
        {
            let path = Path::new(file);
            let bytes = read_input(input)?;
            let previous = Document::from_bytes(&bytes).map_err(core_error)?;
            let opened = circuit::open(path).map_err(core_error)?;
            let result = circuit::undo(
                &opened.document,
                text_arg(after)?,
                text_arg(before)?,
                &previous,
            )
            .map_err(core_error)?;
            let disk_revision =
                circuit::save_snapshot(path, &bytes, &opened.disk_revision).map_err(core_error)?;
            (result, disk_revision)
        }
        _ => return Err(usage()),
    };
    Ok(json!({
        "ok": true, "document": result.document, "receipt": result.receipt, "diskRevision": disk_revision
    }))
}

fn measure_file(path: PathBuf, options: &[OsString]) -> Result<Value, Value> {
    let (channels, from, to, pixels) = wave_options(options)?;
    if pixels.is_some() {
        return Err(usage());
    }
    let mut runs = Runs::default();
    let measured = runs.measure(
        path,
        &channels,
        required(from, "--from")?,
        required(to, "--to")?,
    )?;
    Ok(success(measured))
}

type WaveOptions = (Vec<String>, Option<f64>, Option<f64>, Option<u32>);

fn wave_options(options: &[OsString]) -> Result<WaveOptions, Value> {
    let (mut channels, mut from, mut to, mut pixels) = (Vec::new(), None, None, None);
    for option in options.chunks(2) {
        let [flag, value] = option else {
            return Err(usage());
        };
        let value = value.to_str().ok_or_else(usage)?;
        match flag.to_str() {
            Some("--channel") => channels.push(value.into()),
            Some("--from") if from.is_none() => from = Some(value.parse().map_err(|_| usage())?),
            Some("--to") if to.is_none() => to = Some(value.parse().map_err(|_| usage())?),
            Some("--pixels") if pixels.is_none() => {
                pixels = Some(value.parse().map_err(|_| usage())?)
            }
            _ => return Err(usage()),
        }
    }
    Ok((channels, from, to, pixels))
}

fn wave_file(path: PathBuf, options: &[OsString]) -> Result<Value, Value> {
    let (channels, from, to, pixels) = wave_options(options)?;
    let mut runs = Runs::default();
    let view = runs.wave(
        path,
        &channels,
        required(from, "--from")?,
        required(to, "--to")?,
        required(pixels, "--pixels")?,
    )?;
    Ok(success(view))
}

fn run(path: &Path, out: &Path) -> Result<Value, Value> {
    let opened = circuit::open(path).map_err(core_error)?;
    let (wave, warnings) =
        litzsim_core::solver::simulate_with_warnings(&opened.document, |_| false)
            .map_err(core_error)?;
    litzsim_core::waves::write(out, &wave)
        .map_err(|error| json!({ "code": "ioError", "message": error.to_string() }))?;
    Ok(json!({
        "ok": true,
        "rows": wave.rows().len(),
        "channels": wave.channels(),
        "stopTime_s": wave.rows().last().expect("initial sample")[0],
        "warnings": warnings,
    }))
}

fn inspect_file(path: &Path) -> Result<Value, Value> {
    let opened = circuit::open(path).map_err(core_error)?;
    let inspection = opened.document.inspect();
    let mut response = success(inspection);
    response["diskRevision"] = opened.disk_revision.into();
    Ok(response)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request<'a> {
    op: &'a str,
    part_id: Option<&'a str>,
    file: Option<PathBuf>,
    document: Option<&'a RawValue>,
    expected_revision: Option<&'a str>,
    ops: Option<&'a RawValue>,
    expected_after: Option<&'a str>,
    expected_before: Option<&'a str>,
    previous: Option<&'a RawValue>,
    expected_disk_revision: Option<&'a str>,
    directory: Option<PathBuf>,
    run_id: Option<&'a str>,
    channels: Option<Vec<String>>,
    #[serde(rename = "from_s")]
    from_s: Option<f64>,
    #[serde(rename = "to_s")]
    to_s: Option<f64>,
    pixels: Option<u32>,
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum Reply<'a> {
    Json(Value),
    Wave {
        ok: bool,
        #[serde(flatten)]
        view: litzsim_core::waves::View<'a>,
    },
    Measure {
        ok: bool,
        #[serde(flatten)]
        measured: litzsim_core::waves::Measurement<'a>,
    },
}

pub fn serve_request<'a>(runs: &'a mut Runs, line: &str) -> Reply<'a> {
    let response = match serde_json::from_str::<Request<'_>>(line) {
        Ok(request) if request.op == "wave" => {
            return match wave_view(runs, request) {
                Ok(view) => Reply::Wave { ok: true, view },
                Err(error) => Reply::Json(json!({ "ok": false, "error": error })),
            };
        }
        Ok(request) if request.op == "measure" => {
            return match measure_view(runs, request) {
                Ok(measured) => Reply::Measure { ok: true, measured },
                Err(error) => Reply::Json(json!({ "ok": false, "error": error })),
            };
        }
        Ok(request) => handle(runs, request),
        Err(error) if error.is_syntax() || error.is_eof() => {
            Err(json!({ "code": "invalidJson", "message": "Invalid JSON" }))
        }
        Err(error) => Err(request_error(error.to_string())),
    };
    Reply::Json(response.unwrap_or_else(|error| json!({ "ok": false, "error": error })))
}

fn wave_view<'a>(
    runs: &'a mut Runs,
    request: Request<'_>,
) -> Result<litzsim_core::waves::View<'a>, Value> {
    let id = required(request.run_id, "runId")?;
    let path = runs.directory(id, request.directory.as_deref())?;
    runs.wave(
        path,
        &required(request.channels, "channels")?,
        required(request.from_s, "from_s")?,
        required(request.to_s, "to_s")?,
        required(request.pixels, "pixels")?,
    )
}

fn measure_view<'a>(
    runs: &'a mut Runs,
    request: Request<'_>,
) -> Result<litzsim_core::waves::Measurement<'a>, Value> {
    let id = required(request.run_id, "runId")?;
    let path = runs.directory(id, request.directory.as_deref())?;
    runs.measure(
        path,
        &required(request.channels, "channels")?,
        required(request.from_s, "from_s")?,
        required(request.to_s, "to_s")?,
    )
}

fn handle(runs: &mut Runs, request: Request<'_>) -> Result<Value, Value> {
    match request.op {
        "ping" => Ok(litzsim_core::ping()),
        "new" => Ok(success(Document::new().inspect())),
        "inspect" => match (request.file, request.document) {
            (Some(file), None) => inspect_file(&file),
            (None, Some(raw)) => Ok(success(document(raw)?.inspect())),
            _ => Err(request_error("inspect requires either file or document")),
        },
        "apply" => {
            let document = document(required(request.document, "document")?)?;
            let expected = required(request.expected_revision, "expectedRevision")?;
            let ops = parse_ops(required(request.ops, "ops")?.get().as_bytes())?;
            let edited = circuit::apply(&document, expected, &ops).map_err(core_error)?;
            let layout = litzsim_core::layout::layout(&edited.document);
            Ok(
                json!({ "ok": true, "document": edited.document, "receipt": edited.receipt, "layout": layout }),
            )
        }
        "undo" => {
            let current = document(required(request.document, "document")?)?;
            let after = required(request.expected_after, "expectedAfter")?;
            let before = required(request.expected_before, "expectedBefore")?;
            let previous = document(required(request.previous, "previous")?)?;
            let undone = circuit::undo(&current, after, before, &previous).map_err(core_error)?;
            let layout = litzsim_core::layout::layout(&undone.document);
            Ok(
                json!({ "ok": true, "document": undone.document, "receipt": undone.receipt, "layout": layout }),
            )
        }
        "layout" => match (request.file, request.document) {
            (Some(file), None) => {
                let opened = circuit::open(&file).map_err(core_error)?;
                Ok(success(litzsim_core::layout::layout(&opened.document)))
            }
            (None, Some(raw)) => Ok(success(litzsim_core::layout::layout(&document(raw)?))),
            _ => Err(request_error("layout requires either file or document")),
        },
        "sourcePreview" => {
            let document = match (request.file, request.document) {
                (Some(file), None) => circuit::open(&file).map_err(core_error)?.document,
                (None, Some(raw)) => document(raw)?,
                _ => {
                    return Err(request_error(
                        "sourcePreview requires either file or document",
                    ));
                }
            };
            let preview = litzsim_core::solver::source_preview(
                &document,
                required(request.part_id, "partId")?,
            )
            .map_err(core_error)?;
            Ok(success(preview))
        }
        "save" => {
            let file = required(request.file, "file")?;
            let document = document(required(request.document, "document")?)?;
            let mode = match request.expected_disk_revision {
                Some(expected_disk_revision) => SaveMode::Replace {
                    expected_disk_revision,
                },
                None => SaveMode::New,
            };
            let disk_revision = circuit::save(&file, &document, mode).map_err(core_error)?;
            Ok(json!({ "ok": true, "diskRevision": disk_revision }))
        }
        "start" => runs.start(
            required(request.file, "file")?,
            document(required(request.document, "document")?)?,
            required(request.expected_revision, "expectedRevision")?,
        ),
        "runs" => match (request.file, request.directory) {
            (Some(file), None) => runs::list_circuit(&file),
            (None, Some(directory)) => runs::list(&directory, None),
            _ => Err(request_error("runs requires either file or directory")),
        },
        "status" | "stop" | "pin" | "unpin" | "delete" => {
            let id = required(request.run_id, "runId")?;
            let path = runs.directory(id, request.directory.as_deref())?;
            match request.op {
                "status" => runs::status(&path),
                "stop" => runs::stop(&path),
                "pin" | "unpin" => runs::pin(&path, request.op == "pin"),
                _ => runs.delete(path),
            }
        }
        _ => Err(json!({ "code": "unknownOperation", "message": "Unknown operation" })),
    }
}

fn document(raw: &RawValue) -> Result<Document, Value> {
    Document::from_bytes(raw.get().as_bytes()).map_err(core_error)
}

fn parse_ops(bytes: &[u8]) -> Result<Vec<Operation>, Value> {
    let ops: Vec<&RawValue> =
        serde_json::from_slice(bytes).map_err(|error| request_error(error.to_string()))?;
    ops.iter()
        .enumerate()
        .map(|(index, raw)| {
            serde_json::from_str(raw.get()).map_err(|error| {
                let mut response = json!({
                    "code": "invalidOperation", "operation": index + 1, "message": error.to_string()
                });
                let value: Value = serde_json::from_str(raw.get()).unwrap_or(Value::Null);
                if let Some(id) = ["id", "pin", "a", "type"]
                    .iter()
                    .find_map(|key| value[*key].as_str())
                {
                    response["targetId"] = id.into();
                }
                response
            })
        })
        .collect()
}

fn text_arg(arg: &OsStr) -> Result<&str, Value> {
    arg.to_str()
        .ok_or_else(|| request_error("expected UTF-8 revision"))
}

fn read_input(file: &OsStr) -> Result<Vec<u8>, Value> {
    std::fs::read(file).map_err(|error| json!({ "code": "ioError", "message": error.to_string() }))
}

fn required<T>(value: Option<T>, field: &str) -> Result<T, Value> {
    value.ok_or_else(|| request_error(format!("missing field: {field}")))
}

fn success(value: impl serde::Serialize) -> Value {
    let mut response = json!(value);
    response["ok"] = true.into();
    response
}

fn core_error(error: circuit::Error) -> Value {
    json!(error)
}

fn request_error(message: impl Into<String>) -> Value {
    json!({ "code": "invalidRequest", "message": message.into() })
}

fn usage() -> Value {
    request_error(concat!(
        "Usage: litzsim --version | serve | inspect FILE --json | layout FILE --json",
        " | apply FILE --expect REV --ops OPS.json",
        " | undo FILE --expect AFTER --before BEFORE --previous SNAPSHOT",
        " | run FILE OUT | start FILE | status RUN_DIR | stop RUN_DIR",
        " | pin RUN_DIR | unpin RUN_DIR | delete RUN_DIR",
        " | wave RUN_DIR --channel NAME [--channel NAME...] --from TIME --to TIME --pixels WIDTH",
        " | measure RUN_DIR --channel NAME [--channel NAME...] --from TIME --to TIME | runs DIR|FILE"
    ))
}
