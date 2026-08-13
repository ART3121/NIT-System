use std::{collections::HashSet, fs, path::Path, sync::Arc};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::{
    fsutil::{
        atomic_write, read_text_limited, recover_transaction, reject_symlink, temporary_directory,
        WorkspaceLock, WorkspaceTransaction, MAX_STORAGE_BYTES,
    },
    ids::IdSequences,
    model::{Entry, EntryId, Horizon, Kind, Notes, Roadmap, RoadmapStep},
    storage::{load, render_notes, ACTIVE_TITLE, ARCHIVE_TITLE},
    vault::Vault,
    vault_repository::{VaultRepository, VaultWorkspaceId, VaultWorkspaceInfo},
    workspace::Workspace,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum View {
    Active,
    Archived,
}

impl View {
    pub fn archived(self) -> bool {
        self == Self::Archived
    }
}

#[derive(Clone)]
pub(crate) struct Repository {
    backend: RepositoryBackend,
}

#[derive(Clone)]
enum RepositoryBackend {
    Plain(Workspace),
    Vault(VaultRepository),
}

#[derive(Clone, Default)]
pub(crate) struct RepositoryState {
    pub(crate) active: Notes,
    pub(crate) archived: Notes,
    pub(crate) sequences: IdSequences,
}

impl Repository {
    pub(crate) fn open(workspace: &Workspace) -> Result<Self> {
        let repository = Self {
            backend: RepositoryBackend::Plain(workspace.clone()),
        };
        {
            let _lock = WorkspaceLock::exclusive(&workspace.nit_dir())?;
            recover_transaction(&workspace.nit_dir())?;
            migrate_layout(workspace)?;
        }
        repository.validate_layout()?;
        Ok(repository)
    }

    pub(crate) fn open_vault(vault: Arc<Vault>, workspace_id: VaultWorkspaceId) -> Result<Self> {
        let repository = Self {
            backend: RepositoryBackend::Vault(VaultRepository::open(vault, workspace_id)?),
        };
        let state = repository.state()?;
        validate_state(&state)?;
        Ok(repository)
    }

    pub(crate) fn create_vault_workspace(
        vault: &Arc<Vault>,
        name: impl Into<String>,
    ) -> Result<VaultWorkspaceInfo> {
        VaultRepository::create_workspace(vault, name)
    }

    pub(crate) fn vault_workspaces(vault: &Vault) -> Result<Vec<VaultWorkspaceInfo>> {
        VaultRepository::list_workspaces(vault)
    }

    pub(crate) fn bind_vault(vault: &Vault, binding: &str) -> Result<()> {
        VaultRepository::bind(vault, binding)
    }

    pub(crate) fn vault_binding(vault: &Vault) -> Result<Option<String>> {
        VaultRepository::binding(vault)
    }

    pub(crate) fn load(&self, view: View) -> Result<Notes> {
        if let RepositoryBackend::Vault(repository) = &self.backend {
            let state = repository.read()?;
            validate_state(&state)?;
            return Ok(match view {
                View::Active => state.active,
                View::Archived => state.archived,
            });
        }
        let workspace = self.plain_workspace()?;
        let nit_dir = workspace.nit_dir();
        let lock = WorkspaceLock::shared(&nit_dir)?;
        if nit_dir.join(".transaction").exists() {
            drop(lock);
            let _lock = WorkspaceLock::exclusive(&nit_dir)?;
            recover_transaction(&nit_dir)?;
            return load_from(workspace, view);
        }
        load_from(workspace, view)
    }

    pub(crate) fn save_if_unchanged(
        &self,
        view: View,
        expected: &Notes,
        notes: &Notes,
    ) -> Result<()> {
        if matches!(self.backend, RepositoryBackend::Vault(_)) {
            return self.update_state(|state| {
                let current = match view {
                    View::Active => &mut state.active,
                    View::Archived => &mut state.archived,
                };
                if *current != *expected {
                    bail!("workspace changed since it was loaded; reload before saving to avoid losing data");
                }
                *current = notes.clone();
                state
                    .sequences
                    .reconcile([&state.active, &state.archived])?;
                Ok(())
            });
        }
        self.exclusive(|repository| {
            let current = repository.load_unlocked(view)?;
            if current != *expected {
                bail!(
                    "workspace changed since it was loaded; reload before saving to avoid losing data"
                );
            }
            repository.save_unlocked(view, notes)
        })
    }

    pub(crate) fn save_all_if_unchanged(
        &self,
        expected_active: &Notes,
        expected_archived: &Notes,
        active: &Notes,
        archived: &Notes,
    ) -> Result<()> {
        if matches!(self.backend, RepositoryBackend::Vault(_)) {
            return self.update_state(|state| {
                if state.active != *expected_active || state.archived != *expected_archived {
                    bail!("workspace changed since it was loaded; reload before saving to avoid losing data");
                }
                state.active = active.clone();
                state.archived = archived.clone();
                state
                    .sequences
                    .reconcile([&state.active, &state.archived])?;
                Ok(())
            });
        }
        self.exclusive(|repository| {
            let (current_active, current_archived) = repository.all_unlocked()?;
            if current_active != *expected_active || current_archived != *expected_archived {
                bail!(
                    "workspace changed since it was loaded; reload before saving to avoid losing data"
                );
            }
            repository.save_both_unlocked(active, archived)
        })
    }

    pub(crate) fn save_unlocked(&self, view: View, notes: &Notes) -> Result<()> {
        let workspace = self.plain_workspace()?;
        validate_collection(notes)?;
        let other = self.load_unlocked(match view {
            View::Active => View::Archived,
            View::Archived => View::Active,
        })?;
        validate_global_ids([notes, &other])?;
        save_to(workspace, view, notes)?;
        if !semantically_equal(&self.load_unlocked(view)?, notes) {
            bail!("workspace validation failed after saving");
        }
        Ok(())
    }

    pub(crate) fn all(&self) -> Result<(Notes, Notes)> {
        if let RepositoryBackend::Vault(repository) = &self.backend {
            let state = repository.read()?;
            validate_state(&state)?;
            return Ok((state.active, state.archived));
        }
        let nit_dir = self.plain_workspace()?.nit_dir();
        let lock = WorkspaceLock::shared(&nit_dir)?;
        if nit_dir.join(".transaction").exists() {
            drop(lock);
            let _lock = WorkspaceLock::exclusive(&nit_dir)?;
            recover_transaction(&nit_dir)?;
            return self.all_unlocked();
        }
        self.all_unlocked()
    }

    pub(crate) fn all_unlocked(&self) -> Result<(Notes, Notes)> {
        Ok((
            self.load_unlocked(View::Active)?,
            self.load_unlocked(View::Archived)?,
        ))
    }

    pub(crate) fn load_unlocked(&self, view: View) -> Result<Notes> {
        load_from(self.plain_workspace()?, view)
    }

    pub(crate) fn save_both_unlocked(&self, active: &Notes, archived: &Notes) -> Result<()> {
        validate_collection(active)?;
        validate_collection(archived)?;
        validate_global_ids([active, archived])?;
        let workspace = self.plain_workspace()?;
        save_to(workspace, View::Active, active)?;
        save_to(workspace, View::Archived, archived)?;
        let (saved_active, saved_archived) = self.all_unlocked()?;
        if !semantically_equal(&saved_active, active)
            || !semantically_equal(&saved_archived, archived)
        {
            bail!("workspace validation failed after saving");
        }
        Ok(())
    }

    pub(crate) fn exclusive<T>(&self, operation: impl FnOnce(&Self) -> Result<T>) -> Result<T> {
        let nit_dir = self.plain_workspace()?.nit_dir();
        let _lock = WorkspaceLock::exclusive(&nit_dir)?;
        recover_transaction(&nit_dir)?;
        let mut transaction = WorkspaceTransaction::begin(&nit_dir)?;
        match operation(self) {
            Ok(value) => {
                transaction.commit()?;
                Ok(value)
            }
            Err(error) => {
                if let Err(rollback_error) = transaction.rollback() {
                    return Err(error).context(format!(
                        "transaction rollback also failed: {rollback_error}"
                    ));
                }
                Err(error)
            }
        }
    }

    pub(crate) fn search(
        &self,
        query: &str,
        views: &[View],
        classification: Option<(Kind, Option<crate::model::Horizon>)>,
    ) -> Result<Vec<(View, Entry)>> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            bail!("search query cannot be empty");
        }
        let mut matches = Vec::new();
        for view in views {
            for entry in self.load(*view)?.entries {
                if classification
                    .is_some_and(|(kind, horizon)| entry.kind != kind || entry.horizon != horizon)
                {
                    continue;
                }
                if entry.matches_lowercase_query(&query) {
                    matches.push((*view, entry));
                }
            }
        }
        Ok(matches)
    }

    pub(crate) fn workspace(&self) -> Option<&Workspace> {
        match &self.backend {
            RepositoryBackend::Plain(workspace) => Some(workspace),
            RepositoryBackend::Vault(_) => None,
        }
    }

    pub(crate) fn vault_workspace(&self) -> Result<Option<VaultWorkspaceInfo>> {
        match &self.backend {
            RepositoryBackend::Plain(_) => Ok(None),
            RepositoryBackend::Vault(repository) => repository.info().map(Some),
        }
    }

    pub(crate) fn create_entry(
        &self,
        kind: Kind,
        horizon: Option<Horizon>,
        value: String,
    ) -> Result<EntryId> {
        self.update_state(|state| {
            IdSequences::require_all_ids([&state.active, &state.archived])?;
            state
                .sequences
                .reconcile([&state.active, &state.archived])?;
            let id = state.sequences.allocate(horizon, kind)?;
            crate::commands::add(&mut state.active, Some(id), kind, horizon, value);
            Ok(id)
        })
    }

    pub(crate) fn archive_entry(&self, query: &str) -> Result<()> {
        self.update_state(|state| {
            state
                .sequences
                .reconcile([&state.active, &state.archived])?;
            let index = crate::commands::find_index(&state.active, query)?;
            let entry = state.active.entries.remove(index);
            state.archived.entries.push(entry);
            Ok(())
        })
    }

    pub(crate) fn import_entries(&self, mut imported: Notes) -> Result<usize> {
        if imported.entries.is_empty() {
            bail!("no entries found; import files using the NIT headings and '- text' entries");
        }
        let imported_count = imported.entries.len();
        self.update_state(|state| {
            IdSequences::require_all_ids([&state.active, &state.archived])?;
            for entry in &mut imported.entries {
                if entry.id.is_some_and(|id| !id.is_current()) {
                    entry.id = None;
                }
            }
            state
                .sequences
                .reconcile([&state.active, &state.archived, &imported])?;
            for entry in &mut imported.entries {
                if entry.id.is_none() {
                    entry.id = Some(state.sequences.allocate(entry.horizon, entry.kind)?);
                }
            }
            state.active.entries.extend(imported.entries);
            state
                .sequences
                .reconcile([&state.active, &state.archived])?;
            Ok(())
        })?;
        Ok(imported_count)
    }

    pub(crate) fn roadmap_target(&self, id: EntryId) -> Result<Entry> {
        let entry = self
            .load(View::Active)?
            .entries
            .into_iter()
            .find(|entry| entry.id == Some(id))
            .ok_or_else(|| anyhow!("no active entry has ID {id}"))?;
        if entry.roadmap.is_some() {
            bail!("entry {id} already has a Roadmap; remove it manually before generating another");
        }
        Ok(entry)
    }

    pub(crate) fn attach_roadmap(&self, expected: &Entry, roadmap: Roadmap) -> Result<()> {
        let id = expected
            .id
            .ok_or_else(|| anyhow!("Roadmap targets require an entry ID"))?;
        self.update_state(|state| {
            let index = state
                .active
                .entries
                .iter()
                .position(|entry| entry.id == Some(id))
                .ok_or_else(|| anyhow!("active entry {id} no longer exists"))?;
            if &state.active.entries[index] != expected {
                bail!(
                    "entry {id} changed while its Roadmap was being generated; nothing was saved"
                );
            }
            state.active.entries[index].roadmap = Some(roadmap);
            Ok(())
        })
    }

    fn state(&self) -> Result<RepositoryState> {
        match &self.backend {
            RepositoryBackend::Plain(workspace) => {
                let (active, archived) = self.all()?;
                let mut sequences = IdSequences::load(&workspace.next_ids_path())?;
                sequences.reconcile([&active, &archived])?;
                let state = RepositoryState {
                    active,
                    archived,
                    sequences,
                };
                validate_state(&state)?;
                Ok(state)
            }
            RepositoryBackend::Vault(repository) => {
                let state = repository.read()?;
                validate_state(&state)?;
                Ok(state)
            }
        }
    }

    fn update_state<T>(
        &self,
        operation: impl FnOnce(&mut RepositoryState) -> Result<T>,
    ) -> Result<T> {
        match &self.backend {
            RepositoryBackend::Plain(workspace) => self.exclusive(|repository| {
                let (active, archived) = repository.all_unlocked()?;
                let mut state = RepositoryState {
                    sequences: IdSequences::load(&workspace.next_ids_path())?,
                    active,
                    archived,
                };
                state
                    .sequences
                    .reconcile([&state.active, &state.archived])?;
                let value = operation(&mut state)?;
                validate_state(&state)?;
                repository.save_both_unlocked(&state.active, &state.archived)?;
                state.sequences.save(&workspace.next_ids_path())?;
                Ok(value)
            }),
            RepositoryBackend::Vault(repository) => repository.update(|state| {
                let value = operation(state)?;
                validate_state(state)?;
                Ok(value)
            }),
        }
    }

    fn plain_workspace(&self) -> Result<&Workspace> {
        self.workspace()
            .ok_or_else(|| anyhow!("operation requires Plain Storage"))
    }

    fn validate_layout(&self) -> Result<()> {
        let workspace = self.plain_workspace()?;
        let _lock = WorkspaceLock::exclusive(&workspace.nit_dir())?;
        recover_transaction(&workspace.nit_dir())?;
        let nit = workspace.nit_dir();
        for directory in [
            nit.as_path(),
            workspace.notes_dir(false).as_path(),
            workspace.archive_path().as_path(),
            workspace.notes_dir(true).as_path(),
        ] {
            reject_symlink(directory)?;
        }
        if !workspace.notes_dir(false).is_dir()
            || !workspace.archive_path().is_dir()
            || !workspace.notes_dir(true).is_dir()
        {
            bail!("invalid NIT 0.3 workspace layout at {}", nit.display());
        }
        for path in [
            workspace.ideas_path(false),
            workspace.items_path(false),
            workspace.todos_path(false),
            workspace.ideas_path(true),
            workspace.items_path(true),
            workspace.todos_path(true),
        ] {
            reject_symlink(&path)?;
            if !path.is_file() {
                bail!("missing NIT storage file: {}", path.display());
            }
        }
        let (active, archived) = self.all_unlocked()?;
        validate_global_ids([&active, &archived])
    }
}

fn load_from(workspace: &Workspace, view: View) -> Result<Notes> {
    let archived = view.archived();
    let mut entries = load_note_files(&workspace.notes_dir(archived))?;
    for (path, expected_kind) in [
        (workspace.ideas_path(archived), Kind::Idea),
        (workspace.items_path(archived), Kind::Item),
        (workspace.todos_path(archived), Kind::Todo),
    ] {
        for entry in load(&path)?.entries {
            if entry.kind != expected_kind {
                bail!(
                    "{} contains a {} entry; expected only {} entries",
                    path.display(),
                    entry.kind,
                    expected_kind
                );
            }
            entries.push(entry);
        }
    }
    let notes = Notes { entries };
    validate_collection(&notes)?;
    Ok(notes)
}

fn load_note_files(directory: &Path) -> Result<Vec<Entry>> {
    reject_symlink(directory)?;
    let paths = fs::read_dir(directory)
        .with_context(|| format!("could not read {}", directory.display()))?
        .map(|entry| entry.map(|value| value.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    let mut entries = Vec::new();
    for path in paths {
        if path.extension().and_then(|value| value.to_str()) != Some("md") {
            continue;
        }
        reject_symlink(&path)?;
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| anyhow!("invalid Note filename: {}", path.display()))?;
        let id = EntryId::parse(stem)
            .filter(|value| value.kind() == Kind::Note && value.is_current())
            .ok_or_else(|| anyhow!("invalid Note ID filename: {}", path.display()))?;
        let source = read_text_limited(&path, MAX_STORAGE_BYTES)
            .with_context(|| format!("could not read {}", path.display()))?;
        entries.push(
            parse_note_file(id, &source)
                .with_context(|| format!("could not parse {}", path.display()))?,
        );
    }
    entries.sort_by_key(|entry| entry.id.map(EntryId::sequence).unwrap_or_default());
    Ok(entries)
}

fn parse_note_file(id: EntryId, source: &str) -> Result<Entry> {
    let source = source.replace("\r\n", "\n");
    let mut lines = source.lines();
    let title = lines
        .next()
        .and_then(|line| line.strip_prefix("# "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("Note must start with '# <title>'"))?
        .to_owned();
    let remainder = lines.collect::<Vec<_>>().join("\n");
    let remainder = remainder.trim_matches('\n');
    let (body, roadmap) = if let Some(index) = remainder.rfind("## Roadmap\n") {
        let candidate = &remainder[index + "## Roadmap\n".len()..];
        match parse_note_roadmap(candidate.trim_matches('\n')) {
            Ok(roadmap) => (
                remainder[..index].trim_matches('\n').to_owned(),
                Some(roadmap),
            ),
            Err(_) => (remainder.to_owned(), None),
        }
    } else {
        (remainder.to_owned(), None)
    };
    Ok(Entry {
        id: Some(id),
        kind: Kind::Note,
        horizon: None,
        text: title,
        body,
        roadmap,
    })
}

fn parse_note_roadmap(source: &str) -> Result<Roadmap> {
    if source.is_empty() {
        bail!("Roadmap is empty");
    }
    let lines = source.lines().collect::<Vec<_>>();
    let mut steps = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let prefix = format!("{}. ", steps.len() + 1);
        let title = lines[index]
            .strip_prefix(&prefix)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("invalid Roadmap step"))?;
        index += 1;
        let description = lines
            .get(index)
            .and_then(|line| line.strip_prefix("   "))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("Roadmap step requires a description"))?;
        steps.push(RoadmapStep {
            title: title.to_owned(),
            description: description.to_owned(),
        });
        index += 1;
    }
    Ok(Roadmap { steps })
}

fn render_note_file(entry: &Entry) -> Result<String> {
    if entry.kind != Kind::Note || entry.horizon.is_some() {
        bail!("only timeless Notes can use individual files");
    }
    let title = entry.text.trim();
    if title.is_empty() || title.contains('\n') {
        bail!("Note title must be one non-empty line");
    }
    let mut output = format!("# {title}\n");
    if !entry.body.trim().is_empty() {
        output.push('\n');
        output.push_str(entry.body.trim_matches('\n'));
        output.push('\n');
    }
    if let Some(roadmap) = &entry.roadmap {
        output.push_str("\n## Roadmap\n\n");
        for (index, step) in roadmap.steps.iter().enumerate() {
            output.push_str(&format!(
                "{}. {}\n   {}\n",
                index + 1,
                step.title,
                step.description
            ));
        }
    }
    Ok(output)
}

fn save_to(workspace: &Workspace, view: View, notes: &Notes) -> Result<()> {
    let archived = view.archived();
    for (path, kind) in [
        (workspace.ideas_path(archived), Kind::Idea),
        (workspace.items_path(archived), Kind::Item),
        (workspace.todos_path(archived), Kind::Todo),
    ] {
        let title = if archived {
            ARCHIVE_TITLE
        } else {
            ACTIVE_TITLE
        };
        atomic_write(&path, render_notes(notes, Some(kind), None, title))?;
    }
    sync_note_files(&workspace.notes_dir(archived), notes)
}

fn sync_note_files(directory: &Path, notes: &Notes) -> Result<()> {
    fs::create_dir_all(directory)?;
    let mut desired = HashSet::new();
    for entry in notes
        .entries
        .iter()
        .filter(|entry| entry.kind == Kind::Note)
    {
        let id = entry.id.ok_or_else(|| anyhow!("Note requires an ID"))?;
        let filename = format!("{id}.md");
        desired.insert(filename.clone());
        atomic_write(&directory.join(filename), &render_note_file(entry)?)?;
    }
    for item in fs::read_dir(directory)? {
        let path = item?.path();
        let filename = path.file_name().and_then(|value| value.to_str());
        if path.is_file()
            && path.extension().and_then(|value| value.to_str()) == Some("md")
            && filename.is_some_and(|value| !desired.contains(value))
        {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}

fn validate_collection(notes: &Notes) -> Result<()> {
    let mut ids = HashSet::new();
    for entry in &notes.entries {
        let id = entry
            .id
            .ok_or_else(|| anyhow!("every entry requires an ID"))?;
        if !ids.insert(id) {
            bail!("duplicate entry ID: {id}");
        }
        if id.kind() != entry.kind || id.horizon() != entry.horizon || !id.is_current() {
            bail!("entry ID {id} does not match its classification");
        }
        if entry.kind != Kind::Note && !entry.body.is_empty() {
            bail!("only Notes can contain a body");
        }
    }
    Ok(())
}

fn validate_global_ids<'a>(collections: impl IntoIterator<Item = &'a Notes>) -> Result<()> {
    let mut ids = HashSet::new();
    for notes in collections {
        for id in notes.entries.iter().filter_map(|entry| entry.id) {
            if !ids.insert(id) {
                bail!("duplicate entry ID across active and archive: {id}");
            }
        }
    }
    Ok(())
}

fn validate_state(state: &RepositoryState) -> Result<()> {
    validate_collection(&state.active)?;
    validate_collection(&state.archived)?;
    validate_global_ids([&state.active, &state.archived])?;
    let mut reconciled = state.sequences.clone();
    reconciled.reconcile([&state.active, &state.archived])?;
    if reconciled != state.sequences {
        bail!("ID sequences are behind the entries stored in this workspace");
    }
    Ok(())
}

fn migrate_layout(workspace: &Workspace) -> Result<()> {
    let legacy_notes = workspace.legacy_notes_path();
    let legacy_archive = workspace.legacy_archive_path();
    if workspace.notes_dir(false).is_dir() && workspace.archive_path().is_dir() {
        return Ok(());
    }
    if !legacy_notes.is_file() || !legacy_archive.is_file() {
        bail!("invalid or mixed NIT workspace layout; no files were changed");
    }

    let active = load(&legacy_notes)?;
    let archived = load(&legacy_archive)?;
    IdSequences::require_all_ids([&active, &archived]).map_err(|_| {
        anyhow!("layout migration requires IDs; run `nit -assign-ids` and try again")
    })?;
    if active
        .entries
        .iter()
        .chain(&archived.entries)
        .any(|entry| entry.id.is_some_and(|id| !id.is_current()))
    {
        bail!("layout migration requires current Note/Item IDs; run `nit -migrate-timeless`");
    }

    let root = workspace.root();
    let staging_guard = temporary_directory(root, ".nit.layout-migrate.")?;
    let staging = staging_guard.path().to_path_buf();
    initialize_layout(&staging)?;
    let backups = staging.join("backups/layout-v0.2");
    fs::create_dir_all(&backups)?;
    fs::copy(&legacy_notes, backups.join("notes"))?;
    fs::copy(&legacy_archive, backups.join("archive"))?;
    fs::copy(workspace.next_ids_path(), backups.join("next-ids"))?;
    for item in fs::read_dir(workspace.nit_dir())? {
        let path = item?.path();
        let Some(name) = path.file_name() else {
            continue;
        };
        if path.is_file()
            && name != std::ffi::OsStr::new("notes")
            && name != std::ffi::OsStr::new("archive")
            && name != std::ffi::OsStr::new("next-ids")
        {
            fs::copy(&path, backups.join(name))?;
        }
    }
    fs::copy(workspace.next_ids_path(), staging.join("next-ids"))?;

    let staged_workspace = Workspace::from_root_for_layout(root, staging.clone());
    save_to(&staged_workspace, View::Active, &active)?;
    save_to(&staged_workspace, View::Archived, &archived)?;
    if !semantically_equal(&load_from(&staged_workspace, View::Active)?, &active)
        || !semantically_equal(&load_from(&staged_workspace, View::Archived)?, &archived)
    {
        bail!("layout migration validation failed; the original workspace was preserved");
    }

    let nit = workspace.nit_dir();
    let old_guard = temporary_directory(root, ".nit.layout-old.")?;
    let old = old_guard.path().join("workspace");
    let staging = staging_guard.keep();
    fs::rename(&nit, &old)?;
    if let Err(error) = fs::rename(&staging, &nit) {
        fs::rename(&old, &nit).ok();
        return Err(error).context("could not install migrated NIT workspace");
    }
    fs::remove_dir_all(old)?;
    Ok(())
}

fn semantically_equal(left: &Notes, right: &Notes) -> bool {
    if left.entries.len() != right.entries.len() {
        return false;
    }
    let right_by_id = right
        .entries
        .iter()
        .filter_map(|entry| entry.id.map(|id| (id, entry)))
        .collect::<std::collections::HashMap<_, _>>();
    right_by_id.len() == right.entries.len()
        && left.entries.iter().all(|entry| {
            entry
                .id
                .and_then(|id| right_by_id.get(&id))
                .is_some_and(|candidate| *candidate == entry)
        })
}

pub(crate) fn initialize_layout(path: &Path) -> Result<()> {
    fs::create_dir_all(path.join("notes"))?;
    fs::create_dir_all(path.join("archive/notes"))?;
    for relative in [
        "ideas",
        "items",
        "todos",
        "archive/ideas",
        "archive/items",
        "archive/todos",
    ] {
        let kind = if relative.ends_with("ideas") {
            Kind::Idea
        } else if relative.ends_with("items") {
            Kind::Item
        } else {
            Kind::Todo
        };
        let title = if relative.starts_with("archive/") {
            ARCHIVE_TITLE
        } else {
            ACTIVE_TITLE
        };
        atomic_write(
            &path.join(relative),
            render_notes(&Notes::default(), Some(kind), None, title),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    fn legacy_workspace(name: &str, active: &Notes, archived: &Notes) -> (PathBuf, Workspace) {
        let number = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "nit-repository-{name}-{}-{number}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".nit")).unwrap();
        fs::write(
            root.join(".nit/notes"),
            render_notes(active, None, None, ACTIVE_TITLE),
        )
        .unwrap();
        fs::write(
            root.join(".nit/archive"),
            render_notes(archived, None, None, ARCHIVE_TITLE),
        )
        .unwrap();
        fs::write(root.join(".nit/next-ids"), IdSequences::default().render()).unwrap();
        let workspace = Workspace::discover_from(&root).unwrap();
        (root, workspace)
    }

    #[test]
    fn note_files_round_trip_title_body_and_roadmap() {
        let id = EntryId::new(None, Kind::Note, 1).unwrap();
        let entry = Entry {
            id: Some(id),
            kind: Kind::Note,
            horizon: None,
            text: "Scheduler".into(),
            body: "Study CFS.\n\nKeep examples.".into(),
            roadmap: Some(Roadmap {
                steps: vec![RoadmapStep {
                    title: "Inspect".into(),
                    description: "Read the relevant kernel paths.".into(),
                }],
            }),
        };
        assert_eq!(
            parse_note_file(id, &render_note_file(&entry).unwrap()).unwrap(),
            entry
        );
    }

    #[test]
    fn migrates_version_02_layout_without_semantic_changes() {
        let active = Notes {
            entries: vec![
                Entry {
                    id: EntryId::new(None, Kind::Note, 1),
                    kind: Kind::Note,
                    horizon: None,
                    text: "Parser design".into(),
                    body: "Exact headings remain structural.".into(),
                    roadmap: None,
                },
                Entry {
                    id: EntryId::new(None, Kind::Item, 1),
                    kind: Kind::Item,
                    horizon: None,
                    text: "Parser reference".into(),
                    body: String::new(),
                    roadmap: None,
                },
                Entry {
                    id: EntryId::new(Some(crate::model::Horizon::Long), Kind::Idea, 1),
                    kind: Kind::Idea,
                    horizon: Some(crate::model::Horizon::Long),
                    text: "Plugin API".into(),
                    body: String::new(),
                    roadmap: None,
                },
            ],
        };
        let archived = Notes {
            entries: vec![Entry {
                id: EntryId::new(Some(crate::model::Horizon::Short), Kind::Todo, 1),
                kind: Kind::Todo,
                horizon: Some(crate::model::Horizon::Short),
                text: "Old task".into(),
                body: String::new(),
                roadmap: None,
            }],
        };
        let (root, workspace) = legacy_workspace("migration", &active, &archived);

        let repository = Repository::open(&workspace).unwrap();
        assert!(semantically_equal(
            &repository.load(View::Active).unwrap(),
            &active
        ));
        assert!(semantically_equal(
            &repository.load(View::Archived).unwrap(),
            &archived
        ));
        assert!(root.join(".nit/notes/N-0001.md").is_file());
        assert!(root.join(".nit/backups/layout-v0.2/notes").is_file());
        assert!(root.join(".nit/backups/layout-v0.2/archive").is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mixed_layout_is_rejected_without_replacing_the_legacy_file() {
        let (root, workspace) = legacy_workspace("mixed", &Notes::default(), &Notes::default());
        fs::remove_file(root.join(".nit/archive")).unwrap();
        fs::create_dir_all(root.join(".nit/archive/notes")).unwrap();

        assert!(Repository::open(&workspace).is_err());
        assert!(root.join(".nit/notes").is_file());
        assert!(root.join(".nit/archive").is_dir());
        fs::remove_dir_all(root).unwrap();
    }
}
