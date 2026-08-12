use std::{fs, path::Path};

use anyhow::{bail, Context, Result};

use crate::{
    fsutil::WorkspaceLock,
    ids::IdSequences,
    model::{Entry, EntryId, Horizon, Kind, Notes, Roadmap},
    repository::{Repository, View},
    storage::{load, save, ACTIVE_TITLE, ARCHIVE_TITLE},
    workspace::Workspace,
};

pub fn text(parts: Vec<String>) -> Result<String> {
    let value = parts.join(" ").trim().to_owned();
    if value.is_empty() {
        bail!("text cannot be empty");
    }
    Ok(value)
}

pub fn capture_text(mut parts: Vec<String>) -> Result<(Kind, Option<Horizon>, String)> {
    let last = parts
        .last()
        .ok_or_else(|| anyhow::anyhow!("capture requires text followed by a code such as -n"))?;
    let (kind, horizon) = parse_capture_code(last).ok_or_else(|| {
        anyhow::anyhow!(
            "capture requires a valid final code; use -si/-mi/-li, -st/-mt/-lt, -n, or -x"
        )
    })?;
    parts.pop();
    Ok((kind, horizon, text(parts)?))
}

pub fn parse_capture_code(code: &str) -> Option<(Kind, Option<Horizon>)> {
    match code {
        "-n" => Some((Kind::Note, None)),
        "-x" => Some((Kind::Item, None)),
        "-si" => Some((Kind::Idea, Some(Horizon::Short))),
        "-mi" => Some((Kind::Idea, Some(Horizon::Medium))),
        "-li" => Some((Kind::Idea, Some(Horizon::Long))),
        "-st" => Some((Kind::Todo, Some(Horizon::Short))),
        "-mt" => Some((Kind::Todo, Some(Horizon::Medium))),
        "-lt" => Some((Kind::Todo, Some(Horizon::Long))),
        _ => None,
    }
}

pub(crate) fn add(
    notes: &mut Notes,
    id: Option<EntryId>,
    kind: Kind,
    horizon: Option<Horizon>,
    text: String,
) {
    notes.entries.push(Entry {
        id,
        kind,
        horizon,
        text,
        body: String::new(),
        roadmap: None,
    });
}

pub fn find_index(notes: &Notes, query: &str) -> Result<usize> {
    if let Some(id) = EntryId::parse(query) {
        return notes
            .entries
            .iter()
            .position(|entry| entry.id == Some(id))
            .ok_or_else(|| anyhow::anyhow!("no entry has ID {id}"));
    }
    let query = query.trim().to_lowercase();
    let exact: Vec<usize> = notes
        .entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            (entry.display_text().to_lowercase() == query).then_some(index)
        })
        .collect();
    let matches = if exact.is_empty() {
        notes
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                entry
                    .display_text()
                    .to_lowercase()
                    .contains(&query)
                    .then_some(index)
            })
            .collect()
    } else {
        exact
    };
    match matches.as_slice() {
        [index] => Ok(*index),
        [] => bail!("no entry matches '{query}'"),
        _ => bail!("'{query}' matches more than one entry; use a more specific phrase"),
    }
}

pub(crate) fn roadmap_target(workspace: &Workspace, id: EntryId) -> Result<Entry> {
    let notes = Repository::open(workspace)?.load(View::Active)?;
    let entry = notes
        .entries
        .into_iter()
        .find(|entry| entry.id == Some(id))
        .ok_or_else(|| anyhow::anyhow!("no active entry has ID {id}"))?;
    if entry.roadmap.is_some() {
        bail!("entry {id} already has a Roadmap; remove it manually before generating another");
    }
    Ok(entry)
}

pub(crate) fn attach_roadmap(
    workspace: &Workspace,
    expected: &Entry,
    roadmap: Roadmap,
) -> Result<()> {
    let id = expected
        .id
        .ok_or_else(|| anyhow::anyhow!("Roadmap targets require an entry ID"))?;
    let repository = Repository::open(workspace)?;
    repository.exclusive(|repository| {
        let mut notes = repository.load_unlocked(View::Active)?;
        let index = notes
            .entries
            .iter()
            .position(|entry| entry.id == Some(id))
            .ok_or_else(|| anyhow::anyhow!("active entry {id} no longer exists"))?;
        if &notes.entries[index] != expected {
            bail!("entry {id} changed while its Roadmap was being generated; nothing was saved");
        }
        notes.entries[index].roadmap = Some(roadmap);
        repository.save_unlocked(View::Active, &notes)
    })
}

pub(crate) fn create(
    workspace: &Workspace,
    kind: Kind,
    horizon: Option<Horizon>,
    value: String,
) -> Result<EntryId> {
    let repository = Repository::open(workspace)?;
    repository.exclusive(|repository| {
        let (mut active, archived) = repository.all_unlocked()?;
        IdSequences::require_all_ids([&active, &archived])?;
        let mut sequences = IdSequences::load(&workspace.next_ids_path())?;
        sequences.reconcile([&active, &archived])?;
        let id = sequences.allocate(horizon, kind)?;
        add(&mut active, Some(id), kind, horizon, value);
        repository.save_unlocked(View::Active, &active)?;
        sequences.save(&workspace.next_ids_path())?;
        Ok(id)
    })
}

pub(crate) fn archive_entry(workspace: &Workspace, query: &str) -> Result<()> {
    let repository = Repository::open(workspace)?;
    repository.exclusive(|repository| {
        let (mut active, mut archived) = repository.all_unlocked()?;
        let mut sequences = IdSequences::load(&workspace.next_ids_path())?;
        sequences.reconcile([&active, &archived])?;
        let index = find_index(&active, query)?;
        let entry = active.entries.remove(index);
        archived.entries.push(entry);
        repository.save_both_unlocked(&active, &archived)
    })?;
    Ok(())
}

pub(crate) fn import_notes(workspace: &Workspace, source: &Path) -> Result<usize> {
    let source_notes = load(source)?;
    if source_notes.entries.is_empty() {
        bail!("no entries found; import files using the NIT headings and '- text' entries");
    }
    let repository = Repository::open(workspace)?;
    let imported_count = source_notes.entries.len();
    repository.exclusive(|repository| {
        let (mut target_notes, archived) = repository.all_unlocked()?;
        IdSequences::require_all_ids([&target_notes, &archived])?;
        let mut sequences = IdSequences::load(&workspace.next_ids_path())?;
        let mut imported = source_notes;
        for entry in &mut imported.entries {
            if entry.id.is_some_and(|id| !id.is_current()) {
                entry.id = None;
            }
        }
        sequences.reconcile([&target_notes, &archived, &imported])?;
        for entry in &mut imported.entries {
            if entry.id.is_none() {
                entry.id = Some(sequences.allocate(entry.horizon, entry.kind)?);
            }
        }
        target_notes.entries.extend(imported.entries);
        sequences.reconcile([&target_notes, &archived])?;
        repository.save_unlocked(View::Active, &target_notes)?;
        sequences.save(&workspace.next_ids_path())
    })?;
    Ok(imported_count)
}

pub(crate) fn migrate_timeless_ids(workspace: &Workspace) -> Result<usize> {
    if !workspace.legacy_notes_path().is_file() {
        Repository::open(workspace)?;
        return Ok(0);
    }
    let _lock = WorkspaceLock::exclusive(&workspace.nit_dir())?;
    let notes_path = workspace.legacy_notes_path();
    let archive_path = workspace.legacy_archive_path();
    let sequence_path = workspace.next_ids_path();
    let mut active = load(&notes_path)?;
    let mut archived = load(&archive_path)?;
    let legacy = active
        .entries
        .iter()
        .chain(&archived.entries)
        .filter(|entry| entry.id.is_some_and(|id| !id.is_current()))
        .count();
    let mut sequences = IdSequences::load(&sequence_path)?;
    sequences.reconcile_for_timeless_migration([&active, &archived])?;
    if legacy == 0 {
        sequences.save(&sequence_path)?;
        return Ok(0);
    }

    let notes_backup = workspace.nit_dir().join("notes.pre-timeless.bak");
    let archive_backup = workspace.nit_dir().join("archive.pre-timeless.bak");
    let sequences_backup = workspace.nit_dir().join("next-ids.pre-timeless.bak");
    if notes_backup.exists() || archive_backup.exists() || sequences_backup.exists() {
        bail!("cannot migrate timeless IDs: a pre-timeless backup already exists");
    }

    for entry in active.entries.iter_mut().chain(&mut archived.entries) {
        if entry.id.is_some_and(|id| !id.is_current()) {
            entry.id = Some(sequences.allocate(None, entry.kind)?);
        }
    }
    sequences.reconcile([&active, &archived])?;

    fs::copy(&notes_path, &notes_backup)
        .with_context(|| format!("could not create {}", notes_backup.display()))?;
    fs::copy(&archive_path, &archive_backup)
        .with_context(|| format!("could not create {}", archive_backup.display()))?;
    fs::copy(&sequence_path, &sequences_backup)
        .with_context(|| format!("could not create {}", sequences_backup.display()))?;
    save(&notes_path, &active, ACTIVE_TITLE)?;
    save(&archive_path, &archived, ARCHIVE_TITLE)?;
    sequences.save(&sequence_path)?;
    if load(&notes_path)? != active || load(&archive_path)? != archived {
        bail!("timeless migration validation failed; backups were preserved");
    }
    Ok(legacy)
}

pub(crate) fn assign_missing_ids(workspace: &Workspace) -> Result<usize> {
    if !workspace.legacy_notes_path().is_file() {
        Repository::open(workspace)?;
        return Ok(0);
    }
    let _lock = WorkspaceLock::exclusive(&workspace.nit_dir())?;
    let notes_path = workspace.legacy_notes_path();
    let archive_path = workspace.legacy_archive_path();
    let mut active = load(&notes_path)?;
    let mut archived = load(&archive_path)?;
    let mut sequences = IdSequences::load(&workspace.next_ids_path())?;
    sequences.reconcile([&active, &archived])?;
    let missing = active
        .entries
        .iter()
        .chain(&archived.entries)
        .filter(|entry| entry.id.is_none())
        .count();
    if missing == 0 {
        sequences.save(&workspace.next_ids_path())?;
        return Ok(0);
    }

    let notes_backup = workspace.nit_dir().join("notes.pre-ids.bak");
    let archive_backup = workspace.nit_dir().join("archive.pre-ids.bak");
    if notes_backup.exists() || archive_backup.exists() {
        bail!("cannot assign IDs: a pre-ID backup already exists");
    }
    fs::copy(&notes_path, &notes_backup)
        .with_context(|| format!("could not create {}", notes_backup.display()))?;
    fs::copy(&archive_path, &archive_backup)
        .with_context(|| format!("could not create {}", archive_backup.display()))?;

    for entry in active.entries.iter_mut().chain(&mut archived.entries) {
        if entry.id.is_none() {
            entry.id = Some(sequences.allocate(entry.horizon, entry.kind)?);
        }
    }
    sequences.reconcile([&active, &archived])?;
    save(&notes_path, &active, ACTIVE_TITLE)?;
    save(&archive_path, &archived, ARCHIVE_TITLE)?;
    sequences.save(&workspace.next_ids_path())?;
    if load(&notes_path)? != active || load(&archive_path)? != archived {
        bail!("ID assignment validation failed; backups were preserved");
    }
    Ok(missing)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn finds_a_unique_text_fragment() {
        let mut notes = Notes::default();
        add(
            &mut notes,
            EntryId::new(None, Kind::Note, 1),
            Kind::Note,
            None,
            "buy coffee".into(),
        );
        assert_eq!(find_index(&notes, "coffee").unwrap(), 0);
        assert_eq!(find_index(&notes, "N-0001").unwrap(), 0);
    }

    #[test]
    fn parses_short_capture_codes() {
        let (kind, horizon, value) =
            capture_text(vec!["build".into(), "RISC-V".into(), "-st".into()]).unwrap();
        assert_eq!(kind, Kind::Todo);
        assert_eq!(horizon, Some(Horizon::Short));
        assert_eq!(value, "build RISC-V");
        assert_eq!(
            parse_capture_code("-li"),
            Some((Kind::Idea, Some(Horizon::Long)))
        );
        assert_eq!(parse_capture_code("-n"), Some((Kind::Note, None)));
        assert_eq!(parse_capture_code("-x"), Some((Kind::Item, None)));
        assert_eq!(parse_capture_code("-sn"), None);
    }

    #[test]
    fn capture_requires_a_combined_code() {
        assert!(capture_text(vec!["buy".into(), "coffee".into()]).is_err());
    }

    #[test]
    fn roadmap_attachment_revalidates_the_entry() {
        let directory =
            std::env::temp_dir().join(format!("nit-roadmap-attachment-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let workspace = Workspace::init(&directory).unwrap().workspace;
        let id = create(&workspace, Kind::Note, None, "Learn".into()).unwrap();
        let target = roadmap_target(&workspace, id).unwrap();
        let roadmap = Roadmap {
            steps: vec![crate::model::RoadmapStep {
                title: "First".into(),
                description: "Do the first step.".into(),
            }],
        };
        attach_roadmap(&workspace, &target, roadmap.clone()).unwrap();
        assert_eq!(
            Repository::open(&workspace)
                .unwrap()
                .load(View::Active)
                .unwrap()
                .entries[0]
                .roadmap,
            Some(roadmap)
        );

        let stale = roadmap_target(&workspace, id).unwrap_err();
        assert!(stale.to_string().contains("already has a Roadmap"));
        fs::remove_dir_all(directory).unwrap();
    }
}
