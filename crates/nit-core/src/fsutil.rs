use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, Context, Result};
use fs2::FileExt;
use tempfile::NamedTempFile;

pub(crate) const MAX_STORAGE_BYTES: u64 = 16 * 1024 * 1024;

pub(crate) struct WorkspaceLock {
    file: File,
}

pub(crate) struct WorkspaceTransaction {
    nit_dir: PathBuf,
    active: bool,
}

impl WorkspaceTransaction {
    pub(crate) fn begin(nit_dir: &Path) -> Result<Self> {
        recover_transaction(nit_dir)?;
        let destination = nit_dir.join(".transaction");
        let staging = temporary_directory(nit_dir, ".transaction.prepare.")?;
        let snapshot = staging.path().join("snapshot");
        fs::create_dir(&snapshot)?;
        for relative in STORAGE_PATHS {
            let source = nit_dir.join(relative);
            if source.exists() {
                copy_path(&source, &snapshot.join(relative))?;
            }
        }
        sync_tree(&snapshot)?;
        let staging = staging.keep();
        fs::rename(&staging, &destination)
            .with_context(|| format!("could not prepare transaction in {}", nit_dir.display()))?;
        sync_directory(nit_dir)?;
        Ok(Self {
            nit_dir: nit_dir.to_path_buf(),
            active: true,
        })
    }

    pub(crate) fn commit(mut self) -> Result<()> {
        let journal = self.nit_dir.join(".transaction");
        fs::remove_dir_all(&journal).with_context(|| {
            format!("could not finish transaction in {}", self.nit_dir.display())
        })?;
        sync_directory(&self.nit_dir)?;
        self.active = false;
        Ok(())
    }

    pub(crate) fn rollback(&mut self) -> Result<()> {
        if self.active {
            restore_transaction(&self.nit_dir)?;
            self.active = false;
        }
        Ok(())
    }
}

impl Drop for WorkspaceTransaction {
    fn drop(&mut self) {
        let _ = self.rollback();
    }
}

const STORAGE_PATHS: [&str; 9] = [
    "ideas",
    "items",
    "todos",
    "notes",
    "archive/ideas",
    "archive/items",
    "archive/todos",
    "archive/notes",
    "next-ids",
];

impl WorkspaceLock {
    pub(crate) fn exclusive(nit_dir: &Path) -> Result<Self> {
        fs::create_dir_all(nit_dir)?;
        let path = nit_dir.join(".lock");
        reject_symlink(&path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("could not open workspace lock {}", path.display()))?;
        FileExt::lock_exclusive(&file)
            .with_context(|| format!("could not lock workspace {}", nit_dir.display()))?;
        Ok(Self { file })
    }

    pub(crate) fn shared(nit_dir: &Path) -> Result<Self> {
        fs::create_dir_all(nit_dir)?;
        let path = nit_dir.join(".lock");
        reject_symlink(&path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("could not open workspace lock {}", path.display()))?;
        FileExt::lock_shared(&file)
            .with_context(|| format!("could not lock workspace {}", nit_dir.display()))?;
        Ok(Self { file })
    }
}

impl Drop for WorkspaceLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

pub(crate) fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("refusing to use symbolic link: {}", path.display())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("could not inspect {}", path.display())),
    }
}

pub(crate) fn recover_transaction(nit_dir: &Path) -> Result<()> {
    let journal = nit_dir.join(".transaction");
    match fs::symlink_metadata(&journal) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            bail!(
                "invalid workspace transaction journal: {}",
                journal.display()
            )
        }
        Ok(_) => {}
    }
    restore_transaction(nit_dir)
}

fn restore_transaction(nit_dir: &Path) -> Result<()> {
    let journal = nit_dir.join(".transaction");
    let snapshot = journal.join("snapshot");
    if !snapshot.is_dir() {
        bail!(
            "workspace transaction snapshot is missing: {}",
            snapshot.display()
        );
    }
    for relative in STORAGE_PATHS {
        let destination = nit_dir.join(relative);
        match fs::symlink_metadata(&destination) {
            Ok(metadata) => {
                if metadata.is_dir() && !metadata.file_type().is_symlink() {
                    fs::remove_dir_all(&destination)?;
                } else {
                    fs::remove_file(&destination)?;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let source = snapshot.join(relative);
        if source.exists() {
            copy_path(&source, &destination)?;
        }
    }
    sync_tree(nit_dir)?;
    fs::remove_dir_all(&journal)?;
    sync_directory(nit_dir)
}

fn copy_path(source: &Path, destination: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        bail!(
            "refusing symbolic link in workspace storage: {}",
            source.display()
        );
    }
    if metadata.is_dir() {
        fs::create_dir_all(destination)?;
        for item in fs::read_dir(source)? {
            let item = item?;
            copy_path(&item.path(), &destination.join(item.file_name()))?;
        }
    } else if metadata.is_file() {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        snapshot_file(source, destination)?;
    } else {
        bail!("unsupported workspace file type: {}", source.display());
    }
    Ok(())
}

#[cfg(unix)]
fn snapshot_file(source: &Path, destination: &Path) -> Result<()> {
    if fs::hard_link(source, destination).is_err() {
        fs::copy(source, destination)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn snapshot_file(source: &Path, destination: &Path) -> Result<()> {
    // A real copy preserves rollback semantics without retaining the original
    // file identity, which would block replacement on Windows.
    fs::copy(source, destination)?;
    Ok(())
}

fn sync_tree(path: &Path) -> Result<()> {
    if path.is_file() {
        return File::open(path)?.sync_all().map_err(Into::into);
    }
    for item in fs::read_dir(path)? {
        sync_tree(&item?.path())?;
    }
    sync_directory(path)
}

pub(crate) fn read_text_limited(path: &Path, limit: u64) -> Result<String> {
    reject_symlink(path)?;
    let metadata =
        fs::metadata(path).with_context(|| format!("could not inspect {}", path.display()))?;
    if metadata.len() > limit {
        bail!(
            "{} is too large ({} bytes; limit is {} bytes)",
            path.display(),
            metadata.len(),
            limit
        );
    }
    let file = File::open(path).with_context(|| format!("could not read {}", path.display()))?;
    let capacity = usize::try_from(metadata.len())
        .unwrap_or(0)
        .min(limit as usize);
    let mut bytes = Vec::with_capacity(capacity);
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("could not read {}", path.display()))?;
    if bytes.len() as u64 > limit {
        bail!("{} exceeds the {} byte limit", path.display(), limit);
    }
    String::from_utf8(bytes).with_context(|| format!("{} is not valid UTF-8", path.display()))
}

pub(crate) fn atomic_write(path: &Path, contents: impl AsRef<[u8]>) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("storage path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)?;
    reject_symlink(path)?;

    let mut temporary = NamedTempFile::new_in(parent)
        .with_context(|| format!("could not create temporary file in {}", parent.display()))?;
    temporary
        .write_all(contents.as_ref())
        .with_context(|| format!("could not stage {}", path.display()))?;
    temporary
        .as_file()
        .sync_all()
        .with_context(|| format!("could not sync staged file for {}", path.display()))?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("could not replace {}", path.display()))?;
    sync_directory(parent)?;
    Ok(())
}

#[cfg(unix)]
pub(crate) fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .with_context(|| format!("could not sync directory {}", path.display()))
}

#[cfg(not(unix))]
pub(crate) fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}

pub(crate) fn temporary_directory(parent: &Path, prefix: &str) -> Result<tempfile::TempDir> {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(parent)
        .with_context(|| {
            format!(
                "could not create temporary directory in {}",
                parent.display()
            )
        })
}

pub(crate) fn ensure_regular_or_missing(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("refusing to use symbolic link: {}", path.display())
        }
        Ok(metadata) if !metadata.is_file() => bail!("path is not a file: {}", path.display()),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("could not inspect {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_directory(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "nit-fsutil-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn unfinished_transaction_is_recovered_from_its_snapshot() {
        let nit_dir = test_directory("recovery");
        fs::write(nit_dir.join("ideas"), "before").unwrap();
        let transaction = WorkspaceTransaction::begin(&nit_dir).unwrap();
        atomic_write(&nit_dir.join("ideas"), "after").unwrap();
        std::mem::forget(transaction);

        recover_transaction(&nit_dir).unwrap();
        assert_eq!(fs::read_to_string(nit_dir.join("ideas")).unwrap(), "before");
        assert!(!nit_dir.join(".transaction").exists());
        fs::remove_dir_all(nit_dir).unwrap();
    }

    #[test]
    fn committed_transaction_keeps_new_data() {
        let nit_dir = test_directory("commit");
        fs::write(nit_dir.join("ideas"), "before").unwrap();
        let transaction = WorkspaceTransaction::begin(&nit_dir).unwrap();
        atomic_write(&nit_dir.join("ideas"), "after").unwrap();
        transaction.commit().unwrap();

        recover_transaction(&nit_dir).unwrap();
        assert_eq!(fs::read_to_string(nit_dir.join("ideas")).unwrap(), "after");
        fs::remove_dir_all(nit_dir).unwrap();
    }
}
