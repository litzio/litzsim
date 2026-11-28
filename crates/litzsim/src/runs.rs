use litzsim_core::{
    circuit::{self, Document, SaveMode},
    solver, waves,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Read};
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
const ADMISSION: &str = "admission.lock";
const ACTIVE: &str = "active.lock";
const STARTING: &str = "starting";
const PINNED: &str = "pinned";
const DELETING: &str = ".deleting";
/// Unpinned runs of one circuit file kept when a new run is admitted.
const KEEP: usize = 10;
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
    /// File name of the circuit the run was started from; absent in runs
    /// made before circuits were recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    circuit: Option<String>,
    /// Admission time, Unix ms; absent in runs made before it was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    started_at: Option<f64>,
    revision: String,
    state: State,
    simulated_time: f64,
    stop_time: f64,
    solver_pid: Option<u32>,
    #[serde(default)]
    warnings: Vec<solver::Warning>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

#[derive(Deserialize, Serialize)]
struct Reservation {
    run_id: String,
    supervisor_pid: u32,
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
        file: PathBuf,
        document: Document,
        expected_revision: &str,
    ) -> Response {
        self.supervisors
            .retain_mut(|child| !matches!(child.try_wait(), Ok(Some(_))));
        let (response, child) = admit(&file, &document, expected_revision, true)?;
        self.directories.insert(
            response["runId"].as_str().expect("run ID").into(),
            response["runDirectory"]
                .as_str()
                .expect("run directory")
                .into(),
        );
        self.supervisors.push(child);
        Ok(response)
    }

    pub fn directory(&self, id: &str, parent: Option<&Path>) -> Result<PathBuf, Value> {
        if !valid_run_id(id) {
            return Err(error("invalidRequest", "Invalid run ID"));
        }
        if let Some(parent) = parent {
            return parent.join("runs").join(id).canonicalize().map_err(|err| {
                if err.kind() == io::ErrorKind::NotFound {
                    error("unknownRun", "Unknown run ID in the circuit directory")
                } else {
                    io_error(err)
                }
            });
        }
        self.directories.get(id).cloned().ok_or_else(|| {
            error(
                "unknownRun",
                "Unknown run ID. Supply the circuit directory to reopen a run.",
            )
        })
    }

    /// Deletes a run folder. The cached waveform is held in memory, not as an open
    /// file, so a run shown in this session can be deleted.
    pub fn delete(&mut self, path: PathBuf) -> Response {
        if self.wave.as_ref().is_some_and(|cached| cached.0 == path) {
            self.wave = None;
        }
        let response = delete(&path)?;
        self.directories.retain(|_, directory| *directory != path);
        Ok(response)
    }

    pub fn wave(
        &mut self,
        path: PathBuf,
        channels: &[String],
        from: f64,
        to: f64,
        pixels: u32,
    ) -> Result<waves::View<'_>, Value> {
        self.waveform(path)?
            .view(channels, from, to, pixels)
            .map_err(|err| error("invalidWaveRequest", err.to_string()))
    }

    pub fn measure(
        &mut self,
        path: PathBuf,
        channels: &[String],
        from: f64,
        to: f64,
    ) -> Result<waves::Measurement<'_>, Value> {
        self.waveform(path)?
            .measure(channels, from, to)
            .map_err(|err| error("invalidWaveRequest", err.to_string()))
    }

    fn waveform(&mut self, path: PathBuf) -> Result<&waves::Waveform, Value> {
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
        Ok(&self.wave.as_ref().expect("waveform loaded").1)
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
    let path = Path::new(path).canonicalize().map_err(io_error)?;
    let opened = circuit::open(&path).map_err(|err| json!(err))?;
    let (response, _) = admit(&path, &opened.document, &opened.revision, false)?;
    Ok(response)
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

/// Admits a run of `document`, saved as the circuit `file`, into the `runs`
/// folder next to it, then removes the circuit's oldest unpinned runs.
fn admit(
    file: &Path,
    document: &Document,
    expected_revision: &str,
    owned: bool,
) -> Result<(Value, Child), Value> {
    let revision = document.revision();
    if revision != expected_revision {
        return Err(json!({
            "code": "revisionConflict", "message": "Circuit revision conflict",
            "expected": expected_revision, "actual": revision,
        }));
    }
    solver::validate_probe_reference(document).map_err(|err| json!(err))?;
    let (directory, circuit) = circuit_file(file)?;
    let root = directory.join("runs");
    if root.to_str().is_none() {
        return Err(error(
            "invalidRequest",
            "Run directory must be valid Unicode",
        ));
    }
    let circuit = circuit.as_str();
    fs::create_dir_all(&root).map_err(io_error)?;
    let _admission = admission_lock(&root)?;
    let _active = active_lock(&root)?;
    check_reservation(&root)?;
    let mut command = child_command("_supervise")?;
    let started = SystemTime::now();
    let (path, run_id) = create_run(&root, started).map_err(io_error)?;
    circuit::save(&path.join(SNAPSHOT), document, SaveMode::New).map_err(|err| json!(err))?;
    File::create_new(path.join(CONTROL)).map_err(io_error)?;
    let mut status = Status {
        run_id,
        circuit: Some(circuit.into()),
        started_at: Some(unix_ms(started)),
        revision,
        state: State::Running,
        simulated_time: 0.0,
        stop_time: document.stop_time_s(),
        solver_pid: None,
        warnings: solver::warnings(document),
        error: None,
    };
    write_status(&path, &status).map_err(io_error)?;
    command.arg(&path);
    if owned {
        command.arg("--owned").stdin(Stdio::piped());
    }
    let launch = (|| {
        let child = command.spawn().map_err(io_error)?;
        let reservation = Reservation {
            run_id: status.run_id.clone(),
            supervisor_pid: child.id(),
        };
        if let Err(err) = replace_json(&root, STARTING, &reservation) {
            drop(Worker(child));
            return Err(io_error(err));
        }
        Ok::<_, Value>(child)
    })();
    let child = match launch {
        Ok(child) => child,
        Err(cause) => {
            status.state = State::Failed;
            status.error = Some(cause.clone());
            write_status(&path, &status).map_err(io_error)?;
            return Err(cause);
        }
    };
    let pruned = prune(&root, circuit);
    let response = json!({
        "ok": true, "runId": status.run_id, "revision": status.revision,
        "state": status.state, "runDirectory": path, "pruned": pruned,
    });
    Ok((response, child))
}

/// The folder and file name of a saved circuit file.
fn circuit_file(file: &Path) -> Result<(PathBuf, String), Value> {
    let file = file.canonicalize().map_err(io_error)?;
    if !file.is_file() {
        return Err(error("invalidRequest", "Expected a saved circuit file"));
    }
    let directory = file.parent().expect("file parent").to_path_buf();
    let name = file
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| error("invalidRequest", "Circuit file name must be valid Unicode"))?;
    Ok((directory, name.into()))
}

/// Removes the circuit's unpinned runs beyond the newest [`KEEP`], oldest
/// first, and returns their IDs. Runs in progress count but stay. A run whose
/// folder cannot be removed (a file in it is open) stays until a later
/// admission; earlier failures left as `*.deleting` folders are retried here.
/// Runs without a recorded circuit are never removed here.
fn prune(root: &Path, circuit: &str) -> Vec<String> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut runs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if id.ends_with(DELETING) {
            let _ = fs::remove_dir_all(&path);
            continue;
        }
        if !valid_run_id(&id) || path.join(PINNED).exists() {
            continue;
        }
        let Ok(status) = read_json::<Status>(&path.join(STATUS)) else {
            continue;
        };
        if status.circuit.as_deref() == Some(circuit) {
            runs.push((status.started_at.unwrap_or_default(), id, status.state));
        }
    }
    runs.sort_by(newest_first);
    runs.into_iter()
        .skip(KEEP)
        .rev()
        .filter(|(_, _, state)| !matches!(state, State::Running | State::Stopping))
        .filter_map(|(_, id, _)| remove_run(root, &id).ok().map(|()| id))
        .collect()
}

fn newest_first<T>(a: &(f64, String, T), b: &(f64, String, T)) -> std::cmp::Ordering {
    b.0.total_cmp(&a.0).then_with(|| b.1.cmp(&a.1))
}

/// Moves the run out of the listing, then deletes it. On Windows the rename
/// fails while any file in the folder is open, so nothing is deleted then.
fn remove_run(root: &Path, id: &str) -> io::Result<()> {
    let trash = root.join(format!("{id}{DELETING}"));
    let _ = fs::remove_dir_all(&trash);
    fs::rename(root.join(id), &trash)?;
    let _ = fs::remove_dir_all(trash);
    Ok(())
}

/// The `runs` folder and ID of a run folder; locks it against admission, which
/// prunes runs.
fn run_entry(path: &Path) -> Result<(PathBuf, String, File), Value> {
    let path = path.canonicalize().map_err(io_error)?;
    let id = path
        .file_name()
        .and_then(OsStr::to_str)
        .filter(|id| valid_run_id(id))
        .ok_or_else(|| error("invalidRequest", "Expected a run directory"))?
        .to_owned();
    let root = path.parent().expect("run parent").to_path_buf();
    let admission = admission_lock(&root)?;
    Ok((root, id, admission))
}

/// Pins a run so admission never removes it, or unpins it. The mark is an
/// empty `pinned` file in the run folder.
pub fn pin(path: &Path, pinned: bool) -> Response {
    let status = read_status(path)?;
    let (root, id, _admission) = run_entry(path)?;
    let mark = root.join(&id).join(PINNED);
    let result = if pinned {
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(mark)
            .map(drop)
    } else {
        match fs::remove_file(mark) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    };
    result.map_err(io_error)?;
    Ok(ok(json!({ "runId": status.run_id, "pinned": pinned })))
}

/// Deletes a finished run folder, pinned or not.
pub fn delete(path: &Path) -> Response {
    // Settle a run whose supervisor died before checking its state.
    let status = recover_run(path)?;
    let (root, id, _admission) = run_entry(path)?;
    if matches!(read_status(path)?.state, State::Running | State::Stopping) {
        return Err(error("runInProgress", "Stop the run before deleting it"));
    }
    remove_run(&root, &id).map_err(|err| {
        error(
            "runInUse",
            format!("Run {id} was not deleted. Close programs using its files and retry: {err}"),
        )
    })?;
    Ok(ok(json!({ "runId": status.run_id })))
}

fn check_reservation(root: &Path) -> Result<(), Value> {
    if let Some(reservation) = read_reservation(root)?
        && !clear_dead_reservation(root, reservation)?
    {
        return Err(run_in_progress());
    }
    Ok(())
}

fn read_reservation(root: &Path) -> Result<Option<Reservation>, Value> {
    match read_json(&root.join(STARTING)) {
        Ok(reservation) => Ok(Some(reservation)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(io_error(err)),
    }
}

fn clear_dead_reservation(root: &Path, reservation: Reservation) -> Result<bool, Value> {
    if supervisor_alive(reservation.supervisor_pid).map_err(io_error)? {
        return Ok(false);
    }
    if !valid_run_id(&reservation.run_id) {
        return Err(error("invalidRequest", "Invalid run admission ID"));
    }
    let path = root.join(reservation.run_id);
    let _control = control(&path)?;
    let mut status: Status = read_json(&path.join(STATUS)).map_err(io_error)?;
    status.state = State::Failed;
    status.error = Some(error(
        "supervisorExited",
        "Run supervisor exited during startup",
    ));
    write_status(&path, &status).map_err(io_error)?;
    fs::remove_file(root.join(STARTING)).map_err(io_error)?;
    Ok(true)
}

fn valid_run_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit() || byte == b'-')
}

#[cfg(windows)]
fn supervisor_alive(pid: u32) -> io::Result<bool> {
    use std::ffi::c_void;
    type Handle = *mut c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
        fn CloseHandle(handle: Handle) -> i32;
    }
    // Open a synchronization-only handle and poll the process without waiting.
    unsafe {
        let handle = OpenProcess(0x0010_0000, 0, pid);
        if handle.is_null() {
            return match io::Error::last_os_error() {
                err if err.raw_os_error() == Some(87) => Ok(false),
                err if err.kind() == io::ErrorKind::PermissionDenied => Ok(true),
                err => Err(err),
            };
        }
        let state = WaitForSingleObject(handle, 0);
        let wait_error = io::Error::last_os_error();
        CloseHandle(handle);
        match state {
            0 => Ok(false),
            258 => Ok(true),
            _ => Err(wait_error),
        }
    }
}

#[cfg(unix)]
fn supervisor_alive(pid: u32) -> io::Result<bool> {
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    let pid = i32::try_from(pid)
        .ok()
        .filter(|pid| *pid > 0)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid supervisor PID"))?;
    // Signal zero checks process existence without delivering a signal.
    if unsafe { kill(pid, 0) } == 0 {
        return Ok(true);
    }
    let err = io::Error::last_os_error();
    match err.kind() {
        io::ErrorKind::PermissionDenied => Ok(true),
        io::ErrorKind::NotFound => Ok(false),
        _ if err.raw_os_error() == Some(3) => Ok(false), // ESRCH
        _ => Err(err),
    }
}

fn admission_lock(root: &Path) -> Result<File, Value> {
    let lock = folder_lock(root, ADMISSION)?;
    lock.lock().map_err(io_error)?;
    Ok(lock)
}

fn active_lock(root: &Path) -> Result<File, Value> {
    try_folder_lock(root, ACTIVE)?.ok_or_else(run_in_progress)
}

fn try_folder_lock(root: &Path, name: &str) -> Result<Option<File>, Value> {
    let lock = folder_lock(root, name)?;
    match lock.try_lock() {
        Ok(()) => Ok(Some(lock)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(err)) => Err(io_error(err)),
    }
}

fn folder_lock(root: &Path, name: &str) -> Result<File, Value> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(name))
        .map_err(io_error)
}

fn run_in_progress() -> Value {
    error("runInProgress", "A run is already active in this directory")
}

pub fn supervise(path: &Path, owned: bool) -> Result<(), Value> {
    let root = path
        .parent()
        .ok_or_else(|| error("invalidRequest", "Expected a run directory"))?;
    let admission = admission_lock(root)?;
    let active = active_lock(root)?;
    let reservation = root.join(STARTING);
    let reserved: Reservation = read_json(&reservation).map_err(io_error)?;
    if path.file_name() != Some(OsStr::new(&reserved.run_id))
        || reserved.supervisor_pid != std::process::id()
    {
        return Err(error(
            "invalidRequest",
            "Run has no matching admission reservation",
        ));
    }
    // From here the supervisor holds the OS lease until the terminal status is visible.
    fs::remove_file(reservation).map_err(io_error)?;
    drop(admission);

    let owner_closed = if owned {
        watch_eof(io::stdin())
    } else {
        Arc::default()
    };
    let exit = supervise_solver(path, &owner_closed);
    let _control = control(path)?;
    let mut status = read_json(&path.join(STATUS)).map_err(io_error)?;
    finish(path, &mut status, exit);
    let published = write_status(path, &status).map_err(io_error);
    if published.is_err() && status.state == State::Completed {
        let _ = fs::remove_file(path.join(RESULT));
    }
    drop(active);
    published
}

fn supervise_solver(path: &Path, owner_closed: &AtomicBool) -> Result<Option<ExitStatus>, Value> {
    if owner_closed.load(Ordering::Acquire) {
        stop_request(path)?;
        return Ok(None);
    }
    if read_status(path)?.state == State::Stopping {
        return Ok(None);
    }
    let mut worker = Worker(
        child_command("_solve")?
            .arg(path)
            .stdin(Stdio::piped())
            .spawn()
            .map_err(io_error)?,
    );
    {
        let _control = control(path)?;
        let mut status: Status = read_json(&path.join(STATUS)).map_err(io_error)?;
        status.solver_pid = Some(worker.0.id());
        write_status(path, &status).map_err(io_error)?;
    }
    // Worker drops and is reaped even when updating or monitoring fails.
    monitor(path, &mut worker, owner_closed).map(Some)
}

/// Creates a run folder named by the local start time, `YYYYMMDD-HHMMSS`,
/// with `-1`, `-2`, ... appended when that second is taken.
fn create_run(root: &Path, started: SystemTime) -> io::Result<(PathBuf, String)> {
    let stamp = local_stamp(started)?;
    for sequence in 0_u32.. {
        let id = match sequence {
            0 => stamp.clone(),
            _ => format!("{stamp}-{sequence}"),
        };
        let path = root.join(&id);
        match fs::create_dir(&path) {
            Ok(()) => return Ok((path, id)),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    unreachable!()
}

fn unix_ms(time: SystemTime) -> f64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as f64
}

#[cfg(windows)]
fn local_stamp(time: SystemTime) -> io::Result<String> {
    use std::ffi::c_void;
    #[repr(C)]
    #[derive(Default)]
    struct Calendar {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }
    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn FileTimeToSystemTime(time: *const FileTime, calendar: *mut Calendar) -> i32;
        fn SystemTimeToTzSpecificLocalTime(
            zone: *const c_void,
            universal: *const Calendar,
            local: *mut Calendar,
        ) -> i32;
    }
    // FILETIME counts 100 ns intervals from 1601-01-01 UTC.
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_secs();
    let ticks = (seconds + 11_644_473_600) * 10_000_000;
    let file_time = FileTime {
        low: ticks as u32,
        high: (ticks >> 32) as u32,
    };
    let (mut universal, mut local) = (Calendar::default(), Calendar::default());
    // Both calls only convert the values passed in; a null zone is the current one.
    unsafe {
        if FileTimeToSystemTime(&file_time, &mut universal) == 0
            || SystemTimeToTzSpecificLocalTime(std::ptr::null(), &universal, &mut local) == 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    let Calendar {
        year,
        month,
        day,
        hour,
        minute,
        second,
        ..
    } = local;
    Ok(format!(
        "{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}"
    ))
}

#[cfg(unix)]
fn local_stamp(time: SystemTime) -> io::Result<String> {
    use std::ffi::{c_char, c_int, c_long};
    #[repr(C)]
    struct Tm {
        second: c_int,
        minute: c_int,
        hour: c_int,
        day: c_int,
        month: c_int,
        year: c_int,
        weekday: c_int,
        yearday: c_int,
        dst: c_int,
        offset: c_long,
        zone: *const c_char,
    }
    unsafe extern "C" {
        fn localtime_r(time: *const i64, result: *mut Tm) -> *mut Tm;
    }
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_secs() as i64;
    let mut tm = std::mem::MaybeUninit::<Tm>::zeroed();
    // localtime_r fills only the caller's buffer.
    let tm = unsafe {
        if localtime_r(&seconds, tm.as_mut_ptr()).is_null() {
            return Err(io::Error::last_os_error());
        }
        tm.assume_init()
    };
    Ok(format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        tm.year + 1900,
        tm.month + 1,
        tm.day,
        tm.hour,
        tm.minute,
        tm.second
    ))
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

fn finish(path: &Path, status: &mut Status, exit: Result<Option<ExitStatus>, Value>) {
    let result = match exit {
        Ok(_) if status.state == State::Stopping => {
            status.state = State::Canceled;
            Ok(())
        }
        Ok(Some(exit)) if exit.success() => fs::rename(path.join(STAGED), path.join(RESULT))
            .map_err(io_error)
            .map(|()| {
                status.state = State::Completed;
                status.simulated_time = status.stop_time;
            }),
        Ok(Some(exit)) => Err(read_json(&path.join(WORKER_ERROR))
            .unwrap_or_else(|_| error("solverExited", format!("Solver exited with {exit}")))),
        Ok(None) => unreachable!("solver skipped after a stop request"),
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

/// Runs of a circuit file, and runs without a recorded circuit, newest first.
pub fn list_circuit(file: &Path) -> Response {
    let (directory, circuit) = circuit_file(file)?;
    list(&directory, Some(&circuit))
}

/// Runs of a circuit folder, newest first, or only those of `circuit` and those
/// without a recorded circuit. Completed runs add the rows, time range and
/// signals of their result, read from its header.
pub fn list(directory: &Path, circuit: Option<&str>) -> Response {
    let root = directory.join("runs");
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Ok(ok(json!({ "runs": [] })));
        }
        Err(err) => return Err(io_error(err)),
    };
    let mut runs = Vec::new();
    for entry in entries {
        let entry = entry.map_err(io_error)?;
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        // A run folder is listed once admission has written its status.
        let path = entry.path();
        if !valid_run_id(&id) || !path.join(STATUS).is_file() {
            continue;
        }
        if let Some(run) = list_run(&path, id, circuit)? {
            runs.push(run);
        }
    }
    runs.sort_by(newest_first);
    let runs: Vec<_> = runs.into_iter().map(|(_, _, run)| run).collect();
    Ok(ok(json!({ "runs": runs })))
}

/// A listed run with its start time and ID, or none when it belongs to another
/// circuit or its folder was removed while it was read, as another process's
/// pruning or deletion does. Other errors fail the listing.
fn list_run(
    path: &Path,
    id: String,
    circuit: Option<&str>,
) -> Result<Option<(f64, String, Value)>, Value> {
    match read_listed_run(path, id, circuit) {
        Err(_)
            if fs::symlink_metadata(path)
                .is_err_and(|err| err.kind() == io::ErrorKind::NotFound) =>
        {
            Ok(None)
        }
        result => result,
    }
}

fn read_listed_run(
    path: &Path,
    id: String,
    circuit: Option<&str>,
) -> Result<Option<(f64, String, Value)>, Value> {
    let status = recover_run(path)?;
    if circuit.is_some() && status.circuit.is_some() && status.circuit.as_deref() != circuit {
        return Ok(None);
    }
    // Runs before recorded start times were named by Unix nanoseconds.
    let started = status.started_at.unwrap_or_else(|| {
        id.split('-')
            .next()
            .and_then(|nanos| nanos.parse::<u128>().ok())
            .map_or(0.0, |nanos| (nanos / 1_000_000) as f64)
    });
    let mut run = json!(status);
    run["startedAt"] = json!(started);
    run["circuit"] = json!(status.circuit);
    run["pinned"] = path.join(PINNED).exists().into();
    if status.state == State::Completed {
        let summary = waves::summary(&path.join(RESULT)).map_err(io_error)?;
        run["rows"] = summary.rows.into();
        run["timeRange"] = json!(summary.time_range);
        run["channels"] = json!(summary.channels);
    }
    Ok(Some((started, id, run)))
}

pub fn status(path: &Path) -> Response {
    Ok(ok(json!(recover_run(path)?)))
}

pub fn stop(path: &Path) -> Response {
    let status = recover_run(path)?;
    if status.state == State::Running {
        return Ok(ok(json!(stop_request(path)?)));
    }
    Ok(ok(json!(status)))
}

fn recover_run(path: &Path) -> Result<Status, Value> {
    let status = read_status(path)?;
    if !matches!(status.state, State::Running | State::Stopping) {
        return Ok(status);
    }
    let run_directory = path.canonicalize().map_err(io_error)?;
    let root = run_directory
        .parent()
        .ok_or_else(|| error("invalidRequest", "Expected a run directory"))?;
    let Some(_admission) = try_folder_lock(root, ADMISSION)? else {
        return Ok(status);
    };
    let Some(_active) = try_folder_lock(root, ACTIVE)? else {
        return Ok(status);
    };
    if let Some(reservation) = read_reservation(root)?
        && reservation.run_id == status.run_id
    {
        if clear_dead_reservation(root, reservation)? {
            return read_status(path);
        }
        return Ok(status);
    }

    // A healthy supervisor holds the active lock through terminal publication.
    // Re-read under the status lock in case it just finished and released it.
    let _control = control(path)?;
    let mut status: Status = read_json(&path.join(STATUS)).map_err(io_error)?;
    if matches!(status.state, State::Running | State::Stopping) {
        status.state = State::Failed;
        status.error = Some(error(
            "supervisorExited",
            "Run supervisor exited before completing the run",
        ));
        write_status(path, &status).map_err(io_error)?;
    }
    Ok(status)
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
    replace_json(path, STATUS, value)
}

fn replace_json(path: &Path, name: &str, value: &impl Serialize) -> io::Result<()> {
    let staged = path.join(format!("{name}.tmp"));
    let result = fs::write(&staged, serde_json::to_vec(value)?)
        .and_then(|()| fs::rename(&staged, path.join(name)));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_folders_are_named_by_local_start_time_and_never_reused() {
        let root = std::env::temp_dir().join(format!("litzsim-run-names-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let time = SystemTime::now();
        let stamp = local_stamp(time).unwrap();
        let ids: Vec<_> = (0..3).map(|_| create_run(&root, time).unwrap().1).collect();
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(
            ids,
            [stamp.clone(), format!("{stamp}-1"), format!("{stamp}-2")]
        );
        assert!(valid_run_id(&ids[2]));
        let (date, clock) = stamp.split_once('-').unwrap();
        let field = |text: &str, range: std::ops::Range<usize>| text[range].parse::<u32>().unwrap();
        assert!(date.len() == 8 && clock.len() == 6, "{stamp}");
        assert!(field(date, 0..4) >= 2026);
        assert!((1..=12).contains(&field(date, 4..6)) && (1..=31).contains(&field(date, 6..8)));
        assert!(field(clock, 0..2) < 24 && field(clock, 2..4) < 60 && field(clock, 4..6) < 61);
    }

    #[test]
    fn listing_skips_runs_removed_while_reading_but_reports_other_errors() {
        let directory = std::env::temp_dir().join(format!("litzsim-list-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let root = directory.join("runs");
        let run = |id: &str, status: &[u8]| {
            let path = root.join(id);
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join(STATUS), status).unwrap();
            fs::write(path.join(CONTROL), b"").unwrap();
            path
        };
        let failed = |id: &str| {
            json!({
                "runId": id, "revision": "r", "state": "failed",
                "simulatedTime": 0.0, "stopTime": 1.0, "solverPid": null,
            })
            .to_string()
        };
        run("20261128-100000", failed("20261128-100000").as_bytes());
        let gone = run("20261128-110000", failed("20261128-110000").as_bytes());
        assert_eq!(
            list(&directory, None).unwrap()["runs"]
                .as_array()
                .unwrap()
                .len(),
            2
        );

        // Deletion renames the folder away after the listing found its status.
        fs::remove_dir_all(&gone).unwrap();
        assert!(
            list_run(&gone, "20261128-110000".into(), None)
                .unwrap()
                .is_none()
        );

        // A run that is still there but unreadable fails the listing.
        run("20261128-120000", b"{");
        let err = list(&directory, None).unwrap_err();
        fs::remove_dir_all(&directory).unwrap();
        assert_eq!(err["code"], "ioError", "{err}");
    }

    #[test]
    fn admission_does_not_wait_for_supervisor_handshake() {
        let directory =
            std::env::temp_dir().join(format!("litzsim-admission-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let file = directory.join("rc.litzsim");
        fs::write(&file, include_bytes!("../../../examples/rc.litzsim")).unwrap();
        let document =
            Document::from_bytes(include_bytes!("../../../examples/rc.litzsim")).unwrap();
        let revision = document.revision();

        // In a unit test current_exe is the test runner. The private supervisor
        // arguments make it exit without initializing a solver or replying.
        let (first, mut child) = admit(&file, &document, &revision, false).unwrap();
        let path = Path::new(first["runDirectory"].as_str().unwrap());
        assert_eq!(first["state"], "running");
        assert_eq!(
            circuit::open(&path.join(SNAPSHOT)).unwrap().revision,
            revision
        );
        assert!(status(path).unwrap()["solverPid"].is_null());
        child.wait().unwrap();

        let failed = status(path).unwrap();
        assert_eq!(failed["state"], "failed");
        assert_eq!(failed["error"]["code"], "supervisorExited");
        let (next, mut child) = admit(&file, &document, &revision, false).unwrap();
        assert_ne!(first["runId"], next["runId"]);
        child.wait().unwrap();
        fs::remove_dir_all(directory).unwrap();
    }
}
