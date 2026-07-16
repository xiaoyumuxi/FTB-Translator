use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn sibling(path: &Path, kind: &str) -> PathBuf {
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!(
        ".{name}.ftb-translator-{kind}-{}-{sequence}",
        std::process::id()
    ))
}

fn create_temporary(path: &Path) -> io::Result<(PathBuf, fs::File)> {
    for _ in 0..1000 {
        let temporary = sibling(path, "tmp");
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "cannot allocate atomic-write temporary file",
    ))
}

pub fn write(path: &Path, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let (temporary, mut file) = create_temporary(path)?;
    let result = (|| {
        file.write_all(bytes.as_ref())?;
        file.sync_all()?;
        if let Ok(metadata) = fs::metadata(path) {
            fs::set_permissions(&temporary, metadata.permissions())?;
        }
        drop(file);

        #[cfg(not(target_os = "windows"))]
        fs::rename(&temporary, path)?;

        #[cfg(target_os = "windows")]
        {
            let rollback = sibling(path, "rollback");
            let had_original = path.exists();
            if had_original {
                fs::rename(path, &rollback)?;
            }
            if let Err(error) = fs::rename(&temporary, path) {
                if had_original {
                    let _ = fs::rename(&rollback, path);
                }
                return Err(error);
            }
            if had_original {
                let _ = fs::remove_file(rollback);
            }
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_new_files_and_replaces_existing_content() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        write(&path, b"first").unwrap();
        write(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
