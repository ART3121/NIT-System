use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use crate::{
    fsutil::{
        atomic_write, ensure_regular_or_missing, read_text_limited, reject_symlink,
        temporary_directory,
    },
    ids::IdSequences,
    model::Notes,
    storage::{load, render_notes, ACTIVE_TITLE, ARCHIVE_TITLE},
};

const NIT_DIRECTORY: &str = ".nit";
const NOTES_FILE: &str = "notes";
const ARCHIVE_FILE: &str = "archive";
const IDEAS_FILE: &str = "ideas";
const ITEMS_FILE: &str = "items";
const TODOS_FILE: &str = "todos";
const NEXT_IDS_FILE: &str = "next-ids";
const LEGACY_NOTES_FILE: &str = ".notes";
const LEGACY_ARCHIVE_FILE: &str = ".notes.archive";
const LEGACY_NOTES_BACKUP: &str = ".notes.legacy.bak";
const LEGACY_ARCHIVE_BACKUP: &str = ".notes.archive.legacy.bak";
const MAX_GITIGNORE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    root: PathBuf,
    nit_dir_override: Option<PathBuf>,
}

pub struct InitResult {
    pub workspace: Workspace,
    pub already_existed: bool,
}

impl Workspace {
    pub fn discover() -> Result<Self> {
        Self::discover_from(&std::env::current_dir()?)
    }

    pub fn discover_from(path: &Path) -> Result<Self> {
        let original = path
            .canonicalize()
            .with_context(|| format!("could not resolve {}", path.display()))?;
        let start = if original.is_dir() {
            original.as_path()
        } else {
            original
                .parent()
                .ok_or_else(|| anyhow::anyhow!("path has no parent: {}", original.display()))?
        };

        for candidate in start.ancestors() {
            let nit_dir = candidate.join(NIT_DIRECTORY);
            reject_symlink(&nit_dir)?;
            if nit_dir.is_dir() {
                return Ok(Self {
                    root: candidate.to_path_buf(),
                    nit_dir_override: None,
                });
            }
        }

        if has_legacy_files(start) {
            bail!(legacy_message(start));
        }
        bail!("No NIT workspace found.\nRun `nit -init` to create one.")
    }

    pub fn init(path: &Path) -> Result<InitResult> {
        let root = path
            .canonicalize()
            .with_context(|| format!("could not resolve {}", path.display()))?;
        if !root.is_dir() {
            bail!("workspace root is not a directory: {}", root.display());
        }

        let workspace = Self {
            root,
            nit_dir_override: None,
        };
        let nit_dir = workspace.nit_dir();
        reject_symlink(&nit_dir)?;
        let already_existed = nit_dir.is_dir();
        if nit_dir.exists() && !already_existed {
            bail!(
                "cannot initialize: {} is not a directory",
                nit_dir.display()
            );
        }
        if !already_existed {
            fs::create_dir(&nit_dir)
                .with_context(|| format!("could not create {}", nit_dir.display()))?;
        }

        if workspace.legacy_notes_path().is_file() || workspace.legacy_archive_path().is_file() {
            return Ok(InitResult {
                workspace,
                already_existed: true,
            });
        }
        create_layout(&workspace.nit_dir())?;
        create_storage_file(&workspace.next_ids_path(), &IdSequences::default().render())?;

        Ok(InitResult {
            workspace,
            already_existed,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn nit_dir(&self) -> PathBuf {
        self.nit_dir_override
            .clone()
            .unwrap_or_else(|| self.root.join(NIT_DIRECTORY))
    }

    pub(crate) fn from_root_for_layout(root: &Path, nit_dir: PathBuf) -> Self {
        Self {
            root: root.to_path_buf(),
            nit_dir_override: Some(nit_dir),
        }
    }

    pub fn archive_path(&self) -> PathBuf {
        self.nit_dir().join(ARCHIVE_FILE)
    }

    pub fn legacy_notes_path(&self) -> PathBuf {
        self.nit_dir().join(NOTES_FILE)
    }

    pub fn legacy_archive_path(&self) -> PathBuf {
        self.nit_dir().join(ARCHIVE_FILE)
    }

    pub fn ideas_path(&self, archived: bool) -> PathBuf {
        self.collection_root(archived).join(IDEAS_FILE)
    }

    pub fn items_path(&self, archived: bool) -> PathBuf {
        self.collection_root(archived).join(ITEMS_FILE)
    }

    pub fn todos_path(&self, archived: bool) -> PathBuf {
        self.collection_root(archived).join(TODOS_FILE)
    }

    pub fn notes_dir(&self, archived: bool) -> PathBuf {
        self.collection_root(archived).join(NOTES_FILE)
    }

    fn collection_root(&self, archived: bool) -> PathBuf {
        if archived {
            self.archive_path()
        } else {
            self.nit_dir()
        }
    }

    pub fn next_ids_path(&self) -> PathBuf {
        self.nit_dir().join(NEXT_IDS_FILE)
    }
}

fn create_layout(nit_dir: &Path) -> Result<()> {
    let archive = nit_dir.join(ARCHIVE_FILE);
    fs::create_dir_all(nit_dir.join(NOTES_FILE))?;
    fs::create_dir_all(archive.join(NOTES_FILE))?;
    for (path, kind, title) in [
        (
            nit_dir.join(IDEAS_FILE),
            crate::model::Kind::Idea,
            ACTIVE_TITLE,
        ),
        (
            nit_dir.join(ITEMS_FILE),
            crate::model::Kind::Item,
            ACTIVE_TITLE,
        ),
        (
            nit_dir.join(TODOS_FILE),
            crate::model::Kind::Todo,
            ACTIVE_TITLE,
        ),
        (
            archive.join(IDEAS_FILE),
            crate::model::Kind::Idea,
            ARCHIVE_TITLE,
        ),
        (
            archive.join(ITEMS_FILE),
            crate::model::Kind::Item,
            ARCHIVE_TITLE,
        ),
        (
            archive.join(TODOS_FILE),
            crate::model::Kind::Todo,
            ARCHIVE_TITLE,
        ),
    ] {
        create_storage_file(
            &path,
            &render_notes(&Notes::default(), Some(kind), None, title),
        )?;
    }
    Ok(())
}

pub fn ensure_private(root: &Path) -> Result<()> {
    let gitignore = root.join(".gitignore");
    ensure_regular_or_missing(&gitignore)?;
    let existing = if gitignore.exists() {
        read_text_limited(&gitignore, MAX_GITIGNORE_BYTES)?
    } else {
        String::new()
    };
    if explicitly_ignores_nit(&existing) {
        return Ok(());
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&gitignore)
        .with_context(|| format!("could not update {}", gitignore.display()))?;
    if !existing.is_empty() && !existing.ends_with('\n') {
        file.write_all(b"\n")?;
    }
    file.write_all(b".nit/\n")?;
    Ok(())
}

pub fn appears_ignored(root: &Path) -> Result<bool> {
    let path = root.join(".gitignore");
    ensure_regular_or_missing(&path)?;
    if path.exists() {
        Ok(explicitly_ignores_nit(&read_text_limited(
            &path,
            MAX_GITIGNORE_BYTES,
        )?))
    } else {
        Ok(false)
    }
}

pub fn migrate(path: &Path) -> Result<Workspace> {
    let root = path
        .canonicalize()
        .with_context(|| format!("could not resolve {}", path.display()))?;
    if !root.is_dir() {
        bail!("migration root is not a directory: {}", root.display());
    }

    let legacy_notes = root.join(LEGACY_NOTES_FILE);
    let legacy_archive = root.join(LEGACY_ARCHIVE_FILE);
    let has_notes = legacy_notes.is_file();
    let has_archive = legacy_archive.is_file();
    if !has_notes && !has_archive {
        bail!("No legacy NIT workspace found in {}.", root.display());
    }

    let nit_dir = root.join(NIT_DIRECTORY);
    if nit_dir.exists() {
        bail!(
            "cannot migrate: {} already exists; no files were changed",
            nit_dir.display()
        );
    }

    let notes_backup = root.join(LEGACY_NOTES_BACKUP);
    let archive_backup = root.join(LEGACY_ARCHIVE_BACKUP);
    if (has_notes && notes_backup.exists()) || (has_archive && archive_backup.exists()) {
        bail!("cannot migrate: a legacy backup already exists; no files were changed");
    }

    let expected_notes = if has_notes {
        load(&legacy_notes)?
    } else {
        Notes::default()
    };
    let expected_archive = if has_archive {
        load(&legacy_archive)?
    } else {
        Notes::default()
    };

    let staging_guard = temporary_directory(&root, ".nit.migrate.")?;
    let staging = staging_guard.path().to_path_buf();

    let staged_notes = staging.join(NOTES_FILE);
    let staged_archive = staging.join(ARCHIVE_FILE);
    let staged_result = (|| -> Result<()> {
        copy_or_create(
            has_notes.then_some(legacy_notes.as_path()),
            &staged_notes,
            ACTIVE_TITLE,
        )?;
        copy_or_create(
            has_archive.then_some(legacy_archive.as_path()),
            &staged_archive,
            ARCHIVE_TITLE,
        )?;
        if load(&staged_notes)? != expected_notes || load(&staged_archive)? != expected_archive {
            bail!("migration validation failed; legacy files were preserved");
        }
        let mut sequences = IdSequences::default();
        sequences.reconcile_for_timeless_migration([&expected_notes, &expected_archive])?;
        atomic_write(&staging.join(NEXT_IDS_FILE), sequences.render())
            .with_context(|| "could not create migrated ID sequences")?;
        Ok(())
    })();
    staged_result?;

    let staging = staging_guard.keep();
    fs::rename(&staging, &nit_dir)
        .with_context(|| format!("could not install workspace at {}", nit_dir.display()))?;
    if has_notes {
        fs::rename(&legacy_notes, &notes_backup).with_context(|| {
            format!(
                "workspace created, but could not back up {}",
                legacy_notes.display()
            )
        })?;
    }
    if has_archive {
        fs::rename(&legacy_archive, &archive_backup).with_context(|| {
            format!(
                "workspace created, but could not back up {}",
                legacy_archive.display()
            )
        })?;
    }

    Ok(Workspace {
        root,
        nit_dir_override: None,
    })
}

fn create_storage_file(path: &Path, contents: &str) -> Result<()> {
    ensure_regular_or_missing(path)?;
    if path.exists() {
        if path.is_file() {
            return Ok(());
        }
        bail!("storage path is not a file: {}", path.display());
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(path)
        .with_context(|| format!("could not create {}", path.display()))?;
    file.write_all(contents.as_bytes())
        .with_context(|| format!("could not initialize {}", path.display()))
}

fn copy_or_create(source: Option<&Path>, destination: &Path, title: &str) -> Result<()> {
    if let Some(source) = source {
        fs::copy(source, destination).with_context(|| {
            format!(
                "could not copy {} to {}",
                source.display(),
                destination.display()
            )
        })?;
    } else {
        atomic_write(
            destination,
            render_notes(&Notes::default(), None, None, title),
        )
        .with_context(|| format!("could not create {}", destination.display()))?;
    }
    Ok(())
}

fn has_legacy_files(path: &Path) -> bool {
    path.join(LEGACY_NOTES_FILE).is_file() || path.join(LEGACY_ARCHIVE_FILE).is_file()
}

fn legacy_message(path: &Path) -> String {
    let mut files = String::new();
    if path.join(LEGACY_NOTES_FILE).is_file() {
        files.push_str("  .notes\n");
    }
    if path.join(LEGACY_ARCHIVE_FILE).is_file() {
        files.push_str("  .notes.archive\n");
    }
    format!(
        "Legacy NIT workspace detected.\n\nCurrent files:\n{files}\nRun `nit -migrate` to migrate this workspace to `.nit/`."
    )
}

fn explicitly_ignores_nit(contents: &str) -> bool {
    contents
        .lines()
        .any(|line| matches!(line.trim(), ".nit/" | ".nit" | "/.nit/" | "/.nit"))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    fn temporary_directory(name: &str) -> PathBuf {
        let number = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "nit-workspace-{name}-{}-{number}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn discovers_root_and_nested_directories() {
        let root = temporary_directory("discovery");
        Workspace::init(&root).unwrap();
        let nested = root.join("one/two/three");
        fs::create_dir_all(&nested).unwrap();
        assert_eq!(Workspace::discover_from(&root).unwrap().root(), root);
        assert_eq!(Workspace::discover_from(&nested).unwrap().root(), root);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn nearest_workspace_wins() {
        let outer = temporary_directory("nearest");
        Workspace::init(&outer).unwrap();
        let inner = outer.join("inner");
        fs::create_dir(&inner).unwrap();
        Workspace::init(&inner).unwrap();
        let nested = inner.join("src");
        fs::create_dir(&nested).unwrap();
        assert_eq!(Workspace::discover_from(&nested).unwrap().root(), inner);
        fs::remove_dir_all(outer).unwrap();
    }

    #[test]
    fn rejects_missing_workspace_and_nit_file() {
        let root = temporary_directory("missing");
        assert!(Workspace::discover_from(&root).is_err());
        fs::write(root.join(".nit"), "not a directory").unwrap();
        assert!(Workspace::discover_from(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovery_terminates_at_filesystem_root() {
        if !Path::new("/.nit").is_dir() {
            assert!(Workspace::discover_from(Path::new("/")).is_err());
        }
    }

    #[test]
    fn paths_are_inside_nit_directory() {
        let root = temporary_directory("paths");
        let workspace = Workspace::init(&root).unwrap().workspace;
        assert_eq!(workspace.nit_dir(), root.join(".nit"));
        assert_eq!(workspace.notes_dir(false), root.join(".nit/notes"));
        assert_eq!(workspace.archive_path(), root.join(".nit/archive"));
        assert_eq!(workspace.next_ids_path(), root.join(".nit/next-ids"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn init_creates_files_without_overwriting_them() {
        let root = temporary_directory("init");
        let first = Workspace::init(&root).unwrap();
        assert!(!first.already_existed);
        fs::write(first.workspace.ideas_path(false), "custom").unwrap();
        let second = Workspace::init(&root).unwrap();
        assert!(second.already_existed);
        assert_eq!(
            fs::read_to_string(second.workspace.ideas_path(false)).unwrap(),
            "custom"
        );
        assert!(second.workspace.notes_dir(false).is_dir());
        assert!(second.workspace.archive_path().is_dir());
        assert!(second.workspace.next_ids_path().is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn private_gitignore_is_appended_once() {
        let root = temporary_directory("private");
        fs::write(root.join(".gitignore"), "target/\ncustom").unwrap();
        ensure_private(&root).unwrap();
        ensure_private(&root).unwrap();
        assert_eq!(
            fs::read_to_string(root.join(".gitignore")).unwrap(),
            "target/\ncustom\n.nit/\n"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn private_creates_gitignore_and_tracked_detection_is_read_only() {
        let root = temporary_directory("tracked");
        assert!(!appears_ignored(&root).unwrap());
        ensure_private(&root).unwrap();
        assert!(appears_ignored(&root).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn migration_preserves_complete_workspace_and_creates_backups() {
        let root = temporary_directory("migration-complete");
        let active = "# NIT System\n\n## Short Term\n\n### Notes\n- active\n";
        let archived = "# NIT System — Archived\n\n## Long Term\n\n### Ideas\n- archived\n";
        fs::write(root.join(".notes"), active).unwrap();
        fs::write(root.join(".notes.archive"), archived).unwrap();
        let workspace = migrate(&root).unwrap();
        assert_eq!(
            fs::read_to_string(workspace.legacy_notes_path()).unwrap(),
            active
        );
        assert_eq!(
            fs::read_to_string(workspace.archive_path()).unwrap(),
            archived
        );
        assert!(workspace.next_ids_path().is_file());
        assert_eq!(
            fs::read_to_string(root.join(".notes.legacy.bak")).unwrap(),
            active
        );
        assert_eq!(
            fs::read_to_string(root.join(".notes.archive.legacy.bak")).unwrap(),
            archived
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn migration_supports_either_legacy_file() {
        for legacy_name in [LEGACY_NOTES_FILE, LEGACY_ARCHIVE_FILE] {
            let root = temporary_directory(&format!("migration-{}", legacy_name.len()));
            fs::write(
                root.join(legacy_name),
                "# NIT System\n\n## Short Term\n\n### Notes\n- value\n",
            )
            .unwrap();
            let workspace = migrate(&root).unwrap();
            assert!(workspace.legacy_notes_path().is_file());
            assert!(workspace.archive_path().is_file());
            let migrated_path = if legacy_name == LEGACY_NOTES_FILE {
                workspace.legacy_notes_path()
            } else {
                workspace.archive_path()
            };
            assert_eq!(load(&migrated_path).unwrap().entries.len(), 1);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn migration_never_overwrites_workspace_or_destination() {
        let root = temporary_directory("migration-conflict");
        fs::write(root.join(".notes"), "# NIT System\n").unwrap();
        fs::create_dir(root.join(".nit")).unwrap();
        fs::write(root.join(".nit/notes"), "keep").unwrap();
        assert!(migrate(&root).is_err());
        assert_eq!(fs::read_to_string(root.join(".nit/notes")).unwrap(), "keep");
        assert!(root.join(".notes").is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_validation_preserves_legacy_files() {
        let root = temporary_directory("migration-invalid");
        fs::write(root.join(".notes"), [0xff, 0xfe]).unwrap();
        assert!(migrate(&root).is_err());
        assert!(root.join(".notes").is_file());
        assert!(!root.join(".nit").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
