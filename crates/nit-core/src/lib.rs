mod commands;
mod fsutil;
mod ids;
mod model;
mod repository;
mod storage;
mod workspace;

pub use commands::{capture_text, find_index, parse_capture_code, text};
pub use model::*;
pub use repository::View;
pub use storage::{render_notes, ACTIVE_TITLE, ARCHIVE_TITLE};
pub use workspace::{appears_ignored, ensure_private, migrate, InitResult, Workspace};

use anyhow::Result;
use repository::Repository;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct Nit {
    repository: Repository,
    snapshots: Arc<Mutex<[Option<Notes>; 2]>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocatedEntry {
    pub view: View,
    pub entry: Entry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub active_entries: usize,
    pub archived_entries: usize,
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

    pub fn workspace(&self) -> &Workspace {
        self.repository.workspace()
    }

    pub fn load(&self, view: View) -> Result<Notes> {
        let notes = self.repository.load(view)?;
        self.remember(view, &notes)?;
        Ok(notes)
    }

    pub fn save(&self, view: View, notes: &Notes) -> Result<()> {
        let expected = self.expected(view)?;
        self.repository.save_if_unchanged(view, &expected, notes)?;
        self.remember(view, notes)
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
        self.repository.save_all_if_unchanged(
            &expected_active,
            &expected_archived,
            active,
            archived,
        )?;
        let mut snapshots = self
            .snapshots
            .lock()
            .map_err(|_| anyhow::anyhow!("workspace snapshot state is unavailable; restart NIT"))?;
        snapshots[0] = Some(active.clone());
        snapshots[1] = Some(archived.clone());
        Ok(())
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
        let id = commands::create(self.workspace(), kind, horizon, text)?;
        self.all()?;
        Ok(id)
    }

    pub fn archive(&self, query: &str) -> Result<()> {
        commands::archive_entry(self.workspace(), query)?;
        self.all()?;
        Ok(())
    }

    pub fn import(&self, source: &std::path::Path) -> Result<usize> {
        let count = commands::import_notes(self.workspace(), source)?;
        self.all()?;
        Ok(count)
    }

    pub fn assign_missing_ids(&self) -> Result<usize> {
        commands::assign_missing_ids(self.workspace())
    }

    pub fn assign_missing_ids_in(workspace: &Workspace) -> Result<usize> {
        commands::assign_missing_ids(workspace)
    }

    pub fn migrate_timeless_ids(&self) -> Result<usize> {
        commands::migrate_timeless_ids(self.workspace())
    }

    pub fn migrate_timeless_ids_in(workspace: &Workspace) -> Result<usize> {
        commands::migrate_timeless_ids(workspace)
    }

    pub fn roadmap_target(&self, id: EntryId) -> Result<Entry> {
        commands::roadmap_target(self.workspace(), id)
    }

    pub fn attach_roadmap(&self, entry: &Entry, roadmap: Roadmap) -> Result<()> {
        commands::attach_roadmap(self.workspace(), entry, roadmap)?;
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

fn view_index(view: View) -> usize {
    match view {
        View::Active => 0,
        View::Archived => 1,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

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
}
