use litzsim_core::{
    circuit::{self, Document, SaveMode},
    solver, waves,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

type Response = Result<Value, Value>;

const SNAPSHOT: &str = "circuit.litzsim";
const RESULT: &str = "result.litzwave";
const STAGED: &str = "result.pending";
const STATUS: &str = "status.json";
const WORKER_ERROR: &str = "solver-error.json";
const CONTROL: &str = "status.lock";
const POLL: Duration = Duration::from_millis(10);
const GRACE: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
enum State {
    Running,
    Stopping,
    Completed,
    Canceled,
    Failed,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    run_id: String,
    revision: String,
    state: State,
    simulated_time: f64,
    stop_time: f64,
    solver_pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Start<T> {
    directory: PathBuf,
    document: T,
    expected_revision: String,
}

#[derive(Default)]
pub struct Runs {
    directories: HashMap<String, PathBuf>,
    supervisors: Vec<Child>,
    wave: Option<(PathBuf, waves::Waveform)>,
}

impl Runs {
    pub fn start(
        &mut self,
        directory: PathBuf,
        document: Document,
        expected_revision: &str,
    ) -> Response {
        self.supervisors
            .retain_mut(|child| !matches!(child.try_wait(), Ok(Some(_))));
        let request = Start {
            directory,
            document,
            expected_revision: expected_revision.into(),
        };
        let mut child = child_command("_supervise")?
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(io_error)?;
        let response = (|| {
            let mut input = child.stdin.as_mut().expect("supervisor input pipe");
            serde_json::to_writer(&mut input, &request).map_err(io_error)?;
            input.write_all(b"\n").map_err(io_error)?;
            accepted(&mut child)
        })();
        match response {
            Ok(response) => {
                let id = response["runId"].as_str().expect("supervisor run ID");
                let path = response["runDirectory"]
                    .as_str()
                    .expect("supervisor run directory");
                self.directories.insert(id.into(), path.into());
                self.supervisors.push(child);
                Ok(response)
            }
            Err(error) => {
                let _ = child.wait();
                Err(error)
            }
        }
    }

    pub fn directory(&self, id: &str, parent: Option<&Path>) -> Result<PathBuf, Value> {
        if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit() || byte == b'-') {
            return Err(error("invalidRequest", "Invalid run ID"));
        }
        if let Some(parent) = parent {
            return Ok(parent
                .join("runs")
                .join(id)
                .canonicalize()
                .map_err(io_error)?);
        }
        self.directories.get(id).cloned().ok_or_else(|| {
            error(
                "unknownRun",
                "Unknown run ID. Supply the circuit directory to reopen a run.",
            )
        })
    }

    pub fn wave(
        &mut self,
        path: PathBuf,
        channels: &[String],
        from: f64,
        to: f64,
        pixels: u32,
    ) -> Result<waves::View<'_>, Value> {
        if read_status(&path)?.state != State::Completed {
            return Err(error(
                "runNotCompleted",
                "Waveforms are available only for completed runs",
            ));
        }
        if self.wave.as_ref().is_none_or(|cached| cached.0 != path) {
            let waveform = waves::read(&path.join(RESULT)).map_err(io_error)?;
            self.wave = Some((path, waveform));
        }
        let (_, waveform) = self.wave.as_ref().expect("waveform loaded");
        waveform
            .view(channels, from, to, pixels)
            .map_err(|err| error("invalidWaveRequest", err.to_string()))
    }
}

impl Drop for Runs {
    fn drop(&mut self) {
        for child in &mut self.supervisors {
            drop(child.stdin.take());
        }
        for child in &mut self.supervisors {
            let _ = child.wait();
        }
    }
}

pub fn start_file(path: &OsStr) -> Response {
    let mut supervisor = child_command("_supervise")?
        .arg(path)
        .stdout(Stdio::piped())
        .spawn()
        .map_err(io_error)?;
    let response = accepted(&mut supervisor);
    if response.is_err() {
        let _ = supervisor.wait();
    }
    response
}

fn child_command(mode: &str) -> Result<Command, Value> {
    let mut command = Command::new(std::env::current_exe().map_err(io_error)?);
    command
        .arg(mode)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        prevent_pipe_inheritance().map_err(io_error)?;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    Ok(command)
}

#[cfg(windows)]
fn prevent_pipe_inheritance() -> io::Result<()> {
    use std::ffi::c_void;
    type Handle = *mut c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(which: u32) -> Handle;
        fn GetFileType(handle: Handle) -> u32;
        fn SetHandleInformation(handle: Handle, mask: u32, flags: u32) -> i32;
    }
    const PIPE: u32 = 3;
    const INHERIT: u32 = 1;
    // Windows can inherit the launcher pipes even when the child uses
    // redirected stdio. Command still passes its explicitly configured pipes.
    for which in [-10_i32, -11, -12] {
        // These calls only inspect or change this process's standard handles.
        unsafe {
            let handle = GetStdHandle(which as u32);
            if GetFileType(handle) == PIPE && SetHandleInformation(handle, INHERIT, 0) == 0 {
                return Err(io::Error::last_os_error());
            }
        }
    }
    Ok(())
}

fn accepted(child: &mut Child) -> Response {
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("supervisor output pipe"))
        .read_line(&mut line)
        .map_err(io_error)?;
    let response: Value = serde_json::from_str(&line).map_err(|_| {
        error(
            "runStartFailed",
            "Run supervisor exited without acknowledging start",
        )
    })?;
    if response["ok"] == true {
        Ok(response)
    } else {
        Err(response["error"].clone())
    }
}

pub fn supervise(file: Option<&OsStr>) -> Result<(), Value> {
    let mut input = BufReader::new(io::stdin());
    let start = match file {
        Some(file) => {
            let path = Path::new(file).canonicalize().map_err(io_error)?;
            let opened = circuit::open(&path).map_err(|err| json!(err))?;
            Start {
                directory: path.parent().expect("circuit parent").into(),
                document: opened.document,
                expected_revision: opened.revision,
            }
        }
        None => {
            let mut line = String::new();
            input.read_line(&mut line).map_err(io_error)?;
            let start: Start<Box<serde_json::value::RawValue>> = serde_json::from_str(&line)
                .map_err(|err| error("invalidRequest", err.to_string()))?;
            Start {
                directory: start.directory,
                document: Document::from_bytes(start.document.get().as_bytes())
                    .map_err(|err| json!(err))?,
                expected_revision: start.expected_revision,
            }
        }
    };
    let revision = start.document.revision();
    if revision != start.expected_revision {
        return Err(json!({
            "code": "revisionConflict", "message": "Circuit revision conflict",
            "expected": start.expected_revision, "actual": revision,
        }));
    }
    let root = start
        .directory
        .canonicalize()
        .map_err(io_error)?
        .join("runs");
    if root.to_str().is_none() {
        return Err(error(
            "invalidRequest",
            "Run directory must be valid Unicode",
        ));
    }
    fs::create_dir_all(&root).map_err(io_error)?;
    let active = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("active.lock"))
        .map_err(io_error)?;
    active.try_lock().map_err(|err| match err {
        TryLockError::WouldBlock => {
            error("runInProgress", "A run is already active in this directory")
        }
        TryLockError::Error(err) => io_error(err),
    })?;
    let (path, run_id) = create_run(&root).map_err(io_error)?;
    circuit::save(&path.join(SNAPSHOT), &start.document, SaveMode::New)
        .map_err(|err| json!(err))?;
    let control = File::create_new(path.join(CONTROL)).map_err(io_error)?;
    let mut status = Status {
        run_id,
        revision,
        state: State::Running,
        simulated_time: 0.0,
        stop_time: start.document.stop_time_s(),
        solver_pid: None,
        error: None,
    };
    control.lock().map_err(io_error)?;
    let mut worker = match child_command("_solve")?
        .arg(&path)
        .stdin(Stdio::piped())
        .spawn()
    {
        Ok(child) => Worker(child),
        Err(err) => {
            let cause = io_error(err);
            status.state = State::Failed;
            status.error = Some(cause.clone());
            write_status(&path, &status).map_err(io_error)?;
            return Err(cause);
        }
    };
    status.solver_pid = Some(worker.0.id());
    write_status(&path, &status).map_err(io_error)?;
    control.unlock().map_err(io_error)?;

    let response = json!({
        "ok": true, "runId": status.run_id, "revision": status.revision,
        "state": status.state, "runDirectory": path,
    });
    let owner_closed = match file {
        Some(_) => Arc::default(),
        None => watch_eof(input),
    };
    if acknowledge(&response).is_err() {
        owner_closed.store(true, Ordering::Release);
    }
    let exit = monitor(&path, &mut worker, &owner_closed);
    // Even when monitoring fails, reap the solver before changing the final state.
    drop(worker);
    control.lock().map_err(io_error)?;
    let mut status = read_json(&path.join(STATUS)).map_err(io_error)?;
    finish(&path, &mut status, exit);
    let published = write_status(&path, &status).map_err(io_error);
    if published.is_err() && status.state == State::Completed {
        let _ = fs::remove_file(path.join(RESULT));
    }
    drop(active);
    published
}

pub fn acknowledge(response: &Value) -> io::Result<()> {
    let mut output = io::stdout().lock();
    writeln!(output, "{response}")?;
    output.flush()
}

fn create_run(root: &Path) -> io::Result<(PathBuf, String)> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for sequence in 0.. {
        let id = format!("{timestamp}-{}-{sequence}", std::process::id());
        let path = root.join(&id);
        match fs::create_dir(&path) {
            Ok(()) => return Ok((path, id)),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    unreachable!()
}

struct Worker(Child);

impl Drop for Worker {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            drop(self.0.stdin.take());
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn monitor(
    path: &Path,
    worker: &mut Worker,
    owner_closed: &AtomicBool,
) -> Result<ExitStatus, Value> {
    let mut stopping = None;
    loop {
        if stopping.is_none() {
            let state = if owner_closed.load(Ordering::Acquire) {
                stop_request(path)?.state
            } else {
                read_status(path)?.state
            };
            if state == State::Stopping {
                drop(worker.0.stdin.take());
                stopping = Some(Instant::now());
            }
        }
        if let Some(exit) = worker.0.try_wait().map_err(io_error)? {
            return Ok(exit);
        }
        if stopping.is_some_and(|since| since.elapsed() >= GRACE) {
            // The child can exit between try_wait and kill.
            let _ = worker.0.kill();
            return worker.0.wait().map_err(io_error);
        }
        thread::sleep(POLL);
    }
}

fn finish(path: &Path, status: &mut Status, exit: Result<ExitStatus, Value>) {
    let result = match exit {
        Ok(_) if status.state == State::Stopping => {
            status.state = State::Canceled;
            Ok(())
        }
        Ok(exit) if exit.success() => fs::rename(path.join(STAGED), path.join(RESULT))
            .map_err(io_error)
            .map(|()| {
                status.state = State::Completed;
                status.simulated_time = status.stop_time;
            }),
        Ok(exit) => Err(read_json(&path.join(WORKER_ERROR))
            .unwrap_or_else(|_| error("solverExited", format!("Solver exited with {exit}")))),
        Err(error) => Err(error),
    };
    if let Err(error) = result {
        status.state = State::Failed;
        status.error = Some(error);
    }
    for name in [STAGED, "status.json.tmp", WORKER_ERROR] {
        let _ = fs::remove_file(path.join(name));
    }
}

pub fn solve(path: &Path) -> bool {
    let result = solve_staged(path);
    if let Err(error) = result {
        let _ = fs::write(path.join(WORKER_ERROR), error.to_string());
        return false;
    }
    true
}

fn solve_staged(path: &Path) -> Result<(), Value> {
    let owner_closed = watch_eof(io::stdin());
    let opened = circuit::open(&path.join(SNAPSHOT)).map_err(|err| json!(err))?;
    let mut next_poll = Instant::now();
    let mut next_progress = next_poll + Duration::from_millis(100);
    let mut progress_error = None;
    let wave = solver::simulate_observed(&opened.document, |time| {
        if owner_closed.load(Ordering::Acquire) {
            return true;
        }
        let now = Instant::now();
        if now < next_poll {
            return false;
        }
        next_poll = now + POLL;
        let mut observe = || -> Result<bool, Value> {
            let _control = control(path)?;
            let mut status: Status = read_json(&path.join(STATUS)).map_err(io_error)?;
            if status.state == State::Stopping {
                return Ok(true);
            }
            if now >= next_progress {
                status.simulated_time = time;
                write_status(path, &status).map_err(io_error)?;
                next_progress = now + Duration::from_millis(100);
            }
            Ok(false)
        };
        match observe() {
            Ok(stop) => stop,
            Err(err) => {
                progress_error = Some(err);
                true
            }
        }
    });
    if let Some(error) = progress_error {
        return Err(error);
    }
    waves::stage(&path.join(STAGED), &wave.map_err(|err| json!(err))?).map_err(io_error)
}

fn watch_eof(mut input: impl Read + Send + 'static) -> Arc<AtomicBool> {
    let closed = Arc::new(AtomicBool::new(false));
    let notify = closed.clone();
    thread::spawn(move || {
        let _ = io::copy(&mut input, &mut io::sink());
        notify.store(true, Ordering::Release);
    });
    closed
}

pub fn status(path: &Path) -> Response {
    Ok(ok(json!(read_status(path)?)))
}

pub fn stop(path: &Path) -> Response {
    Ok(ok(json!(stop_request(path)?)))
}

fn read_status(path: &Path) -> Result<Status, Value> {
    let _control = control(path)?;
    read_json(&path.join(STATUS)).map_err(io_error)
}

fn stop_request(path: &Path) -> Result<Status, Value> {
    let _control = control(path)?;
    let mut status: Status = read_json(&path.join(STATUS)).map_err(io_error)?;
    if status.state == State::Running {
        status.state = State::Stopping;
        write_status(path, &status).map_err(io_error)?;
    }
    Ok(status)
}

fn control(path: &Path) -> Result<File, Value> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path.join(CONTROL))
        .map_err(io_error)?;
    file.lock().map_err(io_error)?;
    Ok(file)
}

fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<T> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn write_status(path: &Path, value: &Status) -> io::Result<()> {
    let staged = path.join("status.json.tmp");
    let result = fs::write(&staged, serde_json::to_vec(value)?)
        .and_then(|()| fs::rename(&staged, path.join(STATUS)));
    if result.is_err() {
        let _ = fs::remove_file(staged);
    }
    result
}

fn ok(mut result: Value) -> Value {
    result["ok"] = true.into();
    result
}

fn error(code: &str, message: impl Into<String>) -> Value {
    json!({ "code": code, "message": message.into() })
}

fn io_error(err: impl std::fmt::Display) -> Value {
    error("ioError", err.to_string())
}
