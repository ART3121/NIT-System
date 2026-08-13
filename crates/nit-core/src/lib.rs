mod commands;
mod fsutil;
mod ids;
mod model;
mod repository;
mod storage;
pub mod vault;
mod vault_repository;
mod workspace;

pub use commands::{capture_text, find_index, parse_capture_code, text};
pub use model::*;
pub use repository::View;
pub use storage::{render_notes, ACTIVE_TITLE, ARCHIVE_TITLE};
pub use vault_repository::{VaultWorkspaceId, VaultWorkspaceInfo};
pub use workspace::{appears_ignored, ensure_private, migrate, InitResult, Workspace};

use anyhow::Result;
use repository::Repository;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct Nit {
    repository: Repository,
    snapshots: Arc<Mutex<[Option<Notes>; 2]>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocatedEntry {
    pub view: View,
    pub entry: Entry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub active_entries: usize,
    pub archived_entries: usize,
}

/// Shared domain API implemented by an in-process `Nit` and by the desktop
/// Session client. Frontends depend on this surface instead of storage details.
pub trait NitApi {
    fn allows_external_editor(&self) -> bool;
    fn load(&self, view: View) -> Result<Notes>;
    fn save(&self, view: View, notes: &Notes) -> Result<()>;
    fn all(&self) -> Result<(Notes, Notes)>;
    fn save_all(&self, active: &Notes, archived: &Notes) -> Result<()>;
    fn status(&self) -> Result<Status>;
    fn find_by_id(&self, id: EntryId) -> Result<LocatedEntry>;
    fn search(
        &self,
        query: &str,
        views: &[View],
        classification: Option<(Kind, Option<Horizon>)>,
    ) -> Result<Vec<(View, Entry)>>;
    fn create(&self, kind: Kind, horizon: Option<Horizon>, text: String) -> Result<EntryId>;
    fn archive(&self, query: &str) -> Result<()>;
    fn import(&self, source: &std::path::Path) -> Result<usize>;
    fn roadmap_target(&self, id: EntryId) -> Result<Entry>;
    fn attach_roadmap(&self, entry: &Entry, roadmap: Roadmap) -> Result<()>;
}

impl Nit {
    pub fn discover() -> Result<Self> {
        let workspace = Workspace::discover()?;
        Self::open(&workspace)
    }

    pub fn open(workspace: &Workspace) -> Result<Self> {
        Ok(Self {
            repository: Repository::open(workspace)?,
            snapshots: Arc::new(Mutex::new([None, None])),
        })
    }

    pub fn open_vault(vault: Arc<vault::Vault>, workspace_id: VaultWorkspaceId) -> Result<Self> {
        Ok(Self {
            repository: Repository::open_vault(vault, workspace_id)?,
            snapshots: Arc::new(Mutex::new([None, None])),
        })
    }

    pub fn create_vault_workspace(
        vault: &Arc<vault::Vault>,
        name: impl Into<String>,
    ) -> Result<VaultWorkspaceInfo> {
        Repository::create_vault_workspace(vault, name)
    }

    pub fn vault_workspaces(vault: &vault::Vault) -> Result<Vec<VaultWorkspaceInfo>> {
        Repository::vault_workspaces(vault)
    }

    #[doc(hidden)]
    pub fn bind_vault(vault: &vault::Vault, binding: &str) -> Result<()> {
        Repository::bind_vault(vault, binding)
    }

    #[doc(hidden)]
    pub fn vault_binding(vault: &vault::Vault) -> Result<Option<String>> {
        Repository::vault_binding(vault)
    }

    pub fn workspace(&self) -> Option<&Workspace> {
        self.repository.workspace()
    }

    pub fn vault_workspace(&self) -> Result<Option<VaultWorkspaceInfo>> {
        self.repository.vault_workspace()
    }

    pub fn load(&self, view: View) -> Result<Notes> {
        let notes = self.repository.load(view)?;
        self.remember(view, &notes)?;
        Ok(notes)
    }

    pub fn save(&self, view: View, notes: &Notes) -> Result<()> {
        let expected = self.expected(view)?;
        self.save_if_unchanged(view, &expected, notes)?;
        self.remember(view, notes)
    }

    #[doc(hidden)]
    pub fn save_if_unchanged(&self, view: View, expected: &Notes, notes: &Notes) -> Result<()> {
        self.repository.save_if_unchanged(view, expected, notes)
    }

    pub fn all(&self) -> Result<(Notes, Notes)> {
        let (active, archived) = self.repository.all()?;
        let mut snapshots = self
            .snapshots
            .lock()
            .map_err(|_| anyhow::anyhow!("workspace snapshot state is unavailable; restart NIT"))?;
        snapshots[0] = Some(active.clone());
        snapshots[1] = Some(archived.clone());
        Ok((active, archived))
    }

    pub fn save_all(&self, active: &Notes, archived: &Notes) -> Result<()> {
        let expected_active = self.expected(View::Active)?;
        let expected_archived = self.expected(View::Archived)?;
        self.save_all_if_unchanged(&expected_active, &expected_archived, active, archived)?;
        let mut snapshots = self
            .snapshots
            .lock()
            .map_err(|_| anyhow::anyhow!("workspace snapshot state is unavailable; restart NIT"))?;
        snapshots[0] = Some(active.clone());
        snapshots[1] = Some(archived.clone());
        Ok(())
    }

    #[doc(hidden)]
    pub fn save_all_if_unchanged(
        &self,
        expected_active: &Notes,
        expected_archived: &Notes,
        active: &Notes,
        archived: &Notes,
    ) -> Result<()> {
        self.repository
            .save_all_if_unchanged(expected_active, expected_archived, active, archived)
    }

    pub fn status(&self) -> Result<Status> {
        let (active, archived) = self.all()?;
        Ok(Status {
            active_entries: active.entries.len(),
            archived_entries: archived.entries.len(),
        })
    }

    pub fn find_by_id(&self, id: EntryId) -> Result<LocatedEntry> {
        for view in [View::Active, View::Archived] {
            if let Some(entry) = self
                .load(view)?
                .entries
                .into_iter()
                .find(|entry| entry.id == Some(id))
            {
                return Ok(LocatedEntry { view, entry });
            }
        }
        anyhow::bail!("Entry {id} was not found in the active or archived collection")
    }

    pub fn search(
        &self,
        query: &str,
        views: &[View],
        classification: Option<(Kind, Option<Horizon>)>,
    ) -> Result<Vec<(View, Entry)>> {
        self.repository.search(query, views, classification)
    }

    pub fn create(&self, kind: Kind, horizon: Option<Horizon>, text: String) -> Result<EntryId> {
        let id = self.repository.create_entry(kind, horizon, text)?;
        self.all()?;
        Ok(id)
    }

    pub fn archive(&self, query: &str) -> Result<()> {
        self.repository.archive_entry(query)?;
        self.all()?;
        Ok(())
    }

    pub fn import(&self, source: &std::path::Path) -> Result<usize> {
        let count = self.repository.import_entries(storage::load(source)?)?;
        self.all()?;
        Ok(count)
    }

    pub fn assign_missing_ids(&self) -> Result<usize> {
        self.workspace()
            .map(commands::assign_missing_ids)
            .unwrap_or(Ok(0))
    }

    pub fn assign_missing_ids_in(workspace: &Workspace) -> Result<usize> {
        commands::assign_missing_ids(workspace)
    }

    pub fn migrate_timeless_ids(&self) -> Result<usize> {
        self.workspace()
            .map(commands::migrate_timeless_ids)
            .unwrap_or(Ok(0))
    }

    pub fn migrate_timeless_ids_in(workspace: &Workspace) -> Result<usize> {
        commands::migrate_timeless_ids(workspace)
    }

    pub fn roadmap_target(&self, id: EntryId) -> Result<Entry> {
        self.repository.roadmap_target(id)
    }

    pub fn attach_roadmap(&self, entry: &Entry, roadmap: Roadmap) -> Result<()> {
        self.repository.attach_roadmap(entry, roadmap)?;
        self.all()?;
        Ok(())
    }

    fn remember(&self, view: View, notes: &Notes) -> Result<()> {
        let mut snapshots = self
            .snapshots
            .lock()
            .map_err(|_| anyhow::anyhow!("workspace snapshot state is unavailable; restart NIT"))?;
        snapshots[view_index(view)] = Some(notes.clone());
        Ok(())
    }

    fn expected(&self, view: View) -> Result<Notes> {
        self.snapshots
            .lock()
            .map_err(|_| anyhow::anyhow!("workspace snapshot state is unavailable; restart NIT"))?
            [view_index(view)]
        .clone()
        .ok_or_else(|| anyhow::anyhow!("load this workspace view before saving it"))
    }
}

impl NitApi for Nit {
    fn allows_external_editor(&self) -> bool {
        true
    }

    fn load(&self, view: View) -> Result<Notes> {
        Self::load(self, view)
    }

    fn save(&self, view: View, notes: &Notes) -> Result<()> {
        Self::save(self, view, notes)
    }

    fn all(&self) -> Result<(Notes, Notes)> {
        Self::all(self)
    }

    fn save_all(&self, active: &Notes, archived: &Notes) -> Result<()> {
        Self::save_all(self, active, archived)
    }

    fn status(&self) -> Result<Status> {
        Self::status(self)
    }

    fn find_by_id(&self, id: EntryId) -> Result<LocatedEntry> {
        Self::find_by_id(self, id)
    }

    fn search(
        &self,
        query: &str,
        views: &[View],
        classification: Option<(Kind, Option<Horizon>)>,
    ) -> Result<Vec<(View, Entry)>> {
        Self::search(self, query, views, classification)
    }

    fn create(&self, kind: Kind, horizon: Option<Horizon>, text: String) -> Result<EntryId> {
        Self::create(self, kind, horizon, text)
    }

    fn archive(&self, query: &str) -> Result<()> {
        Self::archive(self, query)
    }

    fn import(&self, source: &std::path::Path) -> Result<usize> {
        Self::import(self, source)
    }

    fn roadmap_target(&self, id: EntryId) -> Result<Entry> {
        Self::roadmap_target(self, id)
    }

    fn attach_roadmap(&self, entry: &Entry, roadmap: Roadmap) -> Result<()> {
        Self::attach_roadmap(self, entry, roadmap)
    }
}

fn view_index(view: View) -> usize {
    match view {
        View::Active => 0,
        View::Archived => 1,
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, sync::Arc};

    use secrecy::SecretString;

    use super::*;

    #[test]
    fn stale_frontend_snapshot_cannot_overwrite_newer_entries() {
        let root = std::env::temp_dir().join(format!(
            "nit-stale-snapshot-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let workspace = Workspace::init(&root).unwrap().workspace;
        let first = Nit::open(&workspace).unwrap();
        let second = Nit::open(&workspace).unwrap();
        let (stale_active, _) = second.all().unwrap();

        first
            .create(Kind::Note, None, "newer entry".into())
            .unwrap();
        let error = second.save(View::Active, &stale_active).unwrap_err();
        assert!(format!("{error:#}").contains("workspace changed"));
        assert_eq!(first.load(View::Active).unwrap().entries.len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn vault_storage_matches_core_domain_behavior_without_plaintext_files() {
        let temp = tempfile::tempdir().unwrap();
        let password = SecretString::from("vault password".to_owned());
        let vault = Arc::new(vault::Vault::create(temp.path().join("vault"), &password).unwrap());
        let first = Nit::create_vault_workspace(&vault, "Portable project").unwrap();
        let second = Nit::create_vault_workspace(&vault, "Independent project").unwrap();
        let nit = Nit::open_vault(vault.clone(), first.id).unwrap();
        let stale = Nit::open_vault(vault.clone(), first.id).unwrap();
        let (stale_active, _) = stale.all().unwrap();

        let note = nit
            .create(Kind::Note, None, "Encrypted architecture".into())
            .unwrap();
        let idea = nit
            .create(Kind::Idea, Some(Horizon::Long), "Portable mode".into())
            .unwrap();
        let todo = nit
            .create(Kind::Todo, Some(Horizon::Short), "Test removal".into())
            .unwrap();
        assert_eq!(note.to_string(), "N-0001");
        assert_eq!(idea.to_string(), "LI-0001");
        assert_eq!(todo.to_string(), "ST-0001");
        assert!(nit
            .create(Kind::Note, Some(Horizon::Short), "invalid".into())
            .is_err());

        let import_path = temp.path().join("import.md");
        let import_source = Notes {
            entries: vec![Entry {
                id: None,
                kind: Kind::Item,
                horizon: None,
                text: "Imported reference".into(),
                body: String::new(),
                roadmap: None,
            }],
        };
        fs::write(
            &import_path,
            render_notes(&import_source, None, None, ACTIVE_TITLE),
        )
        .unwrap();
        assert_eq!(nit.import(&import_path).unwrap(), 1);

        let mut active = nit.load(View::Active).unwrap();
        let note_entry = active
            .entries
            .iter_mut()
            .find(|entry| entry.id == Some(note))
            .unwrap();
        note_entry.body = "Plaintext must remain only in memory.".into();
        nit.save(View::Active, &active).unwrap();

        let target = nit.roadmap_target(todo).unwrap();
        nit.attach_roadmap(
            &target,
            Roadmap {
                steps: vec![RoadmapStep {
                    title: "Disconnect".into(),
                    description: "Verify the session is invalidated.".into(),
                }],
            },
        )
        .unwrap();
        assert_eq!(
            nit.search("disconnect", &[View::Active], None)
                .unwrap()
                .len(),
            1
        );

        nit.archive("Encrypted architecture").unwrap();
        let status = nit.status().unwrap();
        assert_eq!(status.active_entries, 3);
        assert_eq!(status.archived_entries, 1);
        assert!(stale.save(View::Active, &stale_active).is_err());

        let other = Nit::open_vault(vault.clone(), second.id).unwrap();
        assert_eq!(other.status().unwrap().active_entries, 0);
        assert!(nit.workspace().is_none());
        assert_eq!(nit.vault_workspace().unwrap().unwrap(), first);

        drop(other);
        drop(stale);
        drop(nit);
        drop(vault);
        let reopened = Arc::new(vault::Vault::open(temp.path().join("vault"), &password).unwrap());
        let reopened_nit = Nit::open_vault(reopened, first.id).unwrap();
        assert_eq!(reopened_nit.status().unwrap(), status);

        let needle = b"Plaintext must remain only in memory.";
        for item in fs::read_dir(temp.path().join("vault/objects")).unwrap() {
            let bytes = fs::read(item.unwrap().path()).unwrap();
            assert!(!bytes.windows(needle.len()).any(|window| window == needle));
        }
    }
}
