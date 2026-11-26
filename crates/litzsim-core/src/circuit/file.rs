use super::{Document, Error, revision};
use serde::Serialize;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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
    let path = save_path(path)?;
    if path.file_name().is_none() {
        return Err(Error::new("ioError", "Expected a circuit file path"));
    }
    if matches!(mode, SaveMode::New) {
        check_destination(&path, &mode)?;
    }
    let lock_directory = std::env::temp_dir().join("litzsim-locks");
    fs::create_dir_all(&lock_directory).map_err(io_error)?;
    let lock_key = revision(path.as_os_str().as_encoded_bytes());
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_directory.join(format!("{lock_key}.lock")))
        .map_err(io_error)?;
    lock.lock().map_err(io_error)?;

    check_destination(&path, &mode)?;
    let bytes = document.to_bytes();
    let staged = StagedFile::write(&path, |file| file.write_all(&bytes)).map_err(io_error)?;
    match mode {
        SaveMode::New => {
            // Publish a complete file atomically, with no overwrite even if a
            // program that doesn't take this lock creates the destination.
            fs::hard_link(&staged.0, &path).map_err(|error| match error.kind() {
                io::ErrorKind::AlreadyExists => {
                    Error::new("fileExists", "Circuit file already exists")
                }
                _ => io_error(error),
            })?;
        }
        SaveMode::Replace { .. } => {
            // Also catch edits by programs that wrote while the temp file synced.
            check_destination(&path, &mode)?;
            fs::rename(&staged.0, &path).map_err(io_error)?;
        }
    }
    Ok(revision(&bytes))
}

fn check_destination(path: &Path, mode: &SaveMode<'_>) -> Result<(), Error> {
    match mode {
        SaveMode::New => match fs::symlink_metadata(path) {
            Ok(_) => Err(Error::new("fileExists", "Circuit file already exists")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error(error)),
        },
        SaveMode::Replace {
            expected_disk_revision,
        } => {
            let actual = match fs::read(path) {
                Ok(bytes) => Some(revision(&bytes)),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(io_error(error)),
            };
            if actual.as_deref() == Some(*expected_disk_revision) {
                Ok(())
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

fn save_path(path: &Path) -> Result<PathBuf, Error> {
    match path.canonicalize() {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let name = path.file_name().ok_or_else(|| io_error(error))?;
            let parent = path
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            Ok(parent.canonicalize().map_err(io_error)?.join(name))
        }
        Err(error) => Err(io_error(error)),
    }
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
