use super::{Document, Error, revision};
use serde::Serialize;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(windows)]
use std::time::Duration;

/// Delays between Windows replacement attempts after a sharing error.
#[cfg(windows)]
pub const REPLACE_RETRY_DELAYS: [Duration; 4] = [
    Duration::from_millis(25),
    Duration::from_millis(50),
    Duration::from_millis(75),
    Duration::from_millis(100),
];

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Opened {
    pub document: Document,
    pub revision: String,
    pub disk_revision: String,
}

pub enum SaveMode<'a> {
    New,
    Replace { expected_disk_revision: &'a str },
}

pub fn open(path: &Path) -> Result<Opened, Error> {
    let bytes = fs::read(path).map_err(io_error)?;
    let document = Document::from_bytes(&bytes)?;
    Ok(Opened {
        revision: document.revision(),
        disk_revision: revision(&bytes),
        document,
    })
}

/// Serialize cooperating saves across processes. Release the OS lock even
/// when a process exits without finishing. Keep the lock file in the system
/// temporary directory so waiting processes keep the same file identity.
pub fn save(path: &Path, document: &Document, mode: SaveMode<'_>) -> Result<String, Error> {
    let path = save_path(path, |path| path.canonicalize())
        .map_err(|error| save_io_error("resolve the save path for", path, error))?;
    if path.file_name().is_none() {
        return Err(Error::new("ioError", "Expected a circuit file path"));
    }
    if matches!(mode, SaveMode::New) {
        check_destination(&path, &mode)?;
    }
    let lock_directory = std::env::temp_dir().join("litzsim-locks");
    fs::create_dir_all(&lock_directory).map_err(|error| save_io_error("lock", &path, error))?;
    let lock_key = revision(path.as_os_str().as_encoded_bytes());
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_directory.join(format!("{lock_key}.lock")))
        .map_err(|error| save_io_error("lock", &path, error))?;
    lock.lock()
        .map_err(|error| save_io_error("lock", &path, error))?;

    let bytes = document.to_bytes();
    let existing = check_destination(&path, &mode)?;
    if let Some(existing) = existing {
        if existing == bytes
            || Document::read(&existing).is_ok_and(|(current, has_explicit_defaults)| {
                !has_explicit_defaults && current.to_bytes() == bytes
            })
        {
            return Ok(revision(&existing));
        }
    }
    let staged = StagedFile::write(&path, |file| file.write_all(&bytes))
        .map_err(|error| save_io_error("stage", &path, error))?;
    match mode {
        SaveMode::New => {
            // Publish a complete file atomically, with no overwrite even if a
            // program that doesn't take this lock creates the destination.
            fs::hard_link(&staged.0, &path).map_err(|error| match error.kind() {
                io::ErrorKind::AlreadyExists => {
                    Error::new("fileExists", "Circuit file already exists")
                }
                _ => save_io_error("publish", &path, error),
            })?;
        }
        SaveMode::Replace { .. } => {
            replace_file(&staged.0, &path, &mode, |from, to| fs::rename(from, to))?;
        }
    }
    Ok(revision(&bytes))
}

pub(super) fn replace_file(
    staged: &Path,
    path: &Path,
    mode: &SaveMode<'_>,
    mut rename: impl FnMut(&Path, &Path) -> io::Result<()>,
) -> Result<(), Error> {
    #[cfg(windows)]
    let mut delays = REPLACE_RETRY_DELAYS.into_iter();
    loop {
        // Another program can edit the destination while the staged file syncs
        // or while the replacement waits for a sharing conflict to clear.
        check_destination(path, mode)?;
        let Err(error) = rename(staged, path) else {
            return Ok(());
        };
        #[cfg(windows)]
        if matches!(error.raw_os_error(), Some(5 | 32))
            && let Some(delay) = delays.next()
        {
            std::thread::sleep(delay);
            continue;
        }
        return Err(save_io_error("replace", path, error));
    }
}

fn check_destination(path: &Path, mode: &SaveMode<'_>) -> Result<Option<Vec<u8>>, Error> {
    match mode {
        SaveMode::New => match fs::symlink_metadata(path) {
            Ok(_) => Err(Error::new("fileExists", "Circuit file already exists")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(save_io_error("check the destination for", path, error)),
        },
        SaveMode::Replace {
            expected_disk_revision,
        } => {
            let existing = match fs::read(path) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(save_io_error("read the disk revision for", path, error)),
            };
            let actual = existing.as_deref().map(revision);
            if actual.as_deref() == Some(*expected_disk_revision) {
                Ok(existing)
            } else {
                Err(Error::conflict(
                    "diskRevisionConflict",
                    Some(expected_disk_revision),
                    actual.as_deref(),
                ))
            }
        }
    }
}

pub(super) fn save_path(
    path: &Path,
    mut canonicalize: impl FnMut(&Path) -> io::Result<PathBuf>,
) -> io::Result<PathBuf> {
    let resolved = canonicalize(path);
    #[cfg(windows)]
    let resolved = if resolved
        .as_ref()
        .is_ok_and(|path| is_ntfs_deleted_path(path))
    {
        // Canonicalization can open the old file just before another save replaces
        // it, then ask Windows for its name after NTFS has moved it out of the way.
        // Resolve the requested name again so both saves use the destination's lock.
        let current = canonicalize(path);
        if current
            .as_ref()
            .is_ok_and(|path| is_ntfs_deleted_path(path))
        {
            return Err(io::Error::other("Circuit changed while resolving its path"));
        }
        current
    } else {
        resolved
    };
    match resolved {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let name = path.file_name().ok_or(error)?;
            let parent = path
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            Ok(canonicalize(parent)?.join(name))
        }
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
fn is_ntfs_deleted_path(path: &Path) -> bool {
    use std::path::Component;

    let mut components = path.components();
    matches!(
        (components.next(), components.next(), components.next(), components.next()),
        (Some(Component::Prefix(_)), Some(Component::RootDir), Some(Component::Normal(extend)), Some(Component::Normal(deleted)))
            if extend.eq_ignore_ascii_case("$Extend") && deleted.eq_ignore_ascii_case("$Deleted")
    )
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(path.file_name().expect("file path"));
    name.push(suffix);
    path.with_file_name(name)
}

pub(crate) struct StagedFile(pub(crate) PathBuf);

impl StagedFile {
    pub(crate) fn write(
        path: &Path,
        write: impl FnOnce(&mut File) -> io::Result<()>,
    ) -> io::Result<Self> {
        if path.file_name().is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Expected a file path",
            ));
        }
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let (staged, mut file): (Self, File) = loop {
            let count = NEXT.fetch_add(1, Ordering::Relaxed);
            let temporary = sibling(path, &format!(".{}.{count}.tmp", std::process::id()));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
            {
                Ok(file) => break (Self(temporary), file),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        };
        let result = write(&mut file).and_then(|()| file.sync_all());
        drop(file);
        result?;
        Ok(staged)
    }
}

impl Drop for StagedFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn io_error(error: io::Error) -> Error {
    Error::new("ioError", error.to_string())
}

fn save_io_error(stage: &str, path: &Path, error: io::Error) -> Error {
    Error::new(
        "ioError",
        format!("Could not {stage} {}: {error}", path.display()),
    )
}
