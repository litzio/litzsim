use litzsim_core::circuit::{self, Document, Operation, SaveMode};
use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{Value, json};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

pub fn file_command(args: &[OsString]) -> Result<Value, Value> {
    let (path, opened, result) = match args {
        [cmd, file, flag] if cmd == "inspect" && flag == "--json" => {
            return inspect_file(Path::new(file));
        }
        [cmd, file, flag] if cmd == "layout" && flag == "--json" => {
            let opened = circuit::open(Path::new(file)).map_err(core_error)?;
            return Ok(success(litzsim_core::layout::layout(&opened.document)));
        }
        [cmd, file, out] if cmd == "run" => return run(Path::new(file), Path::new(out)),
        [cmd, file, expect, revision, option, input]
            if cmd == "apply" && expect == "--expect" && option == "--ops" =>
        {
            let path = Path::new(file);
            let bytes = read_input(input)?;
            let ops = parse_ops(&bytes)?;
            let opened = circuit::open(path).map_err(core_error)?;
            let result =
                circuit::apply(&opened.document, text_arg(revision)?, &ops).map_err(core_error)?;
            (path, opened, result)
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
            let previous = circuit::open(Path::new(input)).map_err(core_error)?;
            let opened = circuit::open(path).map_err(core_error)?;
            let result = circuit::undo(
                &opened.document,
                text_arg(after)?,
                text_arg(before)?,
                &previous.document,
            )
            .map_err(core_error)?;
            (path, opened, result)
        }
        _ => return Err(usage()),
    };
    let disk_revision = circuit::save(
        path,
        &result.document,
        SaveMode::Replace {
            expected_disk_revision: &opened.disk_revision,
        },
    )
    .map_err(core_error)?;
    Ok(json!({
        "ok": true, "document": result.document, "receipt": result.receipt, "diskRevision": disk_revision
    }))
}

fn run(path: &Path, out: &Path) -> Result<Value, Value> {
    let opened = circuit::open(path).map_err(core_error)?;
    let wave = litzsim_core::solver::simulate(&opened.document, || false).map_err(core_error)?;
    litzsim_core::waves::write(out, &wave)
        .map_err(|error| json!({ "code": "ioError", "message": error.to_string() }))?;
    Ok(json!({
        "ok": true,
        "rows": wave.rows().len(),
        "channels": wave.channels(),
        "stopTime_s": wave.rows().last().expect("initial sample")[0],
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
    file: Option<PathBuf>,
    document: Option<&'a RawValue>,
    expected_revision: Option<&'a str>,
    ops: Option<&'a RawValue>,
    expected_after: Option<&'a str>,
    expected_before: Option<&'a str>,
    previous: Option<&'a RawValue>,
    expected_disk_revision: Option<&'a str>,
}

pub fn serve_request(line: &str) -> Value {
    let response = match serde_json::from_str::<Request<'_>>(line) {
        Ok(request) => handle(request),
        Err(error) if error.is_syntax() || error.is_eof() => {
            Err(json!({ "code": "invalidJson", "message": "Invalid JSON" }))
        }
        Err(error) => Err(request_error(error.to_string())),
    };
    response.unwrap_or_else(|error| json!({ "ok": false, "error": error }))
}

fn handle(request: Request<'_>) -> Result<Value, Value> {
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
            Ok(success(
                circuit::undo(&current, after, before, &previous).map_err(core_error)?,
            ))
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
    request_error(
        "Usage: litzsim --version | serve | inspect FILE --json | layout FILE --json | apply FILE --expect REV --ops OPS.json | undo FILE --expect AFTER --before BEFORE --previous SNAPSHOT | run FILE OUT",
    )
}
