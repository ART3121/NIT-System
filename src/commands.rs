use std::{fs, path::Path};

use anyhow::{bail, Context, Result};

use crate::{
    model::{Entry, Horizon, Kind, Notes},
    storage::{archive_path, load, notes_path, save, ACTIVE_TITLE, ARCHIVE_TITLE},
};

pub(crate) fn text(parts: Vec<String>) -> Result<String> {
    let value = parts.join(" ").trim().to_owned();
    if value.is_empty() {
        bail!("text cannot be empty");
    }
    Ok(value)
}

pub(crate) fn capture_text(mut parts: Vec<String>) -> Result<(Kind, Horizon, String)> {
    let mut kind = Kind::Todo;
    let mut horizon = Horizon::Short;
    if let Some(last) = parts.last() {
        if let Some((parsed_kind, parsed_horizon)) = parse_capture_code(last) {
            parts.pop();
            kind = parsed_kind;
            horizon = parsed_horizon;
        } else if last.starts_with('-') && last.len() == 3 {
            bail!("unknown capture code '{last}'; use -si/-sn/-sx/-st, -mi/-mn/-mx/-mt, or -li/-ln/-lx/-lt");
        }
    }
    Ok((kind, horizon, text(parts)?))
}

fn parse_capture_code(code: &str) -> Option<(Kind, Horizon)> {
    let mut characters = code.chars();
    if characters.next()? != '-' {
        return None;
    }
    let horizon = match characters.next()? {
        's' => Horizon::Short,
        'm' => Horizon::Medium,
        'l' => Horizon::Long,
        _ => return None,
    };
    let kind = match characters.next()? {
        'i' => Kind::Idea,
        'n' => Kind::Note,
        'x' => Kind::Item,
        't' => Kind::Todo,
        _ => return None,
    };
    characters.next().is_none().then_some((kind, horizon))
}

pub(crate) fn add(notes: &mut Notes, kind: Kind, horizon: Horizon, text: String) {
    notes.entries.push(Entry {
        kind,
        horizon,
        text,
    });
}

pub(crate) fn find_index(notes: &Notes, query: &str) -> Result<usize> {
    let query = query.trim().to_lowercase();
    let exact: Vec<usize> = notes
        .entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| (entry.text.to_lowercase() == query).then_some(index))
        .collect();
    let matches = if exact.is_empty() {
        notes
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                entry.text.to_lowercase().contains(&query).then_some(index)
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

pub(crate) fn create(path: &Path, kind: Kind, horizon: Horizon, value: String) -> Result<()> {
    let mut notes = load(path)?;
    add(&mut notes, kind, horizon, value);
    save(path, &notes, ACTIVE_TITLE)?;
    println!("Added {kind}/{horizon}.");
    Ok(())
}

pub(crate) fn archive_entry(query: &str) -> Result<()> {
    let active_path = notes_path()?;
    let archived_path = archive_path()?;
    let mut active = load(&active_path)?;
    let index = find_index(&active, query)?;
    let entry = active.entries.remove(index);
    let mut archived = load(&archived_path)?;
    archived.entries.push(entry);
    save(&active_path, &active, ACTIVE_TITLE)?;
    save(&archived_path, &archived, ARCHIVE_TITLE)?;
    println!("Archived.");
    Ok(())
}

pub(crate) fn import_notes(target: &Path, source: &Path) -> Result<()> {
    let source_notes = load(source)?;
    if source_notes.entries.is_empty() {
        bail!("no entries found; import files using the INIT headings and '- text' entries");
    }
    let same_file = source == target;
    let mut target_notes = if same_file {
        Notes::default()
    } else {
        load(target)?
    };
    target_notes.entries.extend(source_notes.entries);
    if same_file {
        let backup = target.with_file_name(format!(".notes.legacy.{}.bak", std::process::id()));
        fs::copy(source, &backup)
            .with_context(|| format!("could not create backup {}", backup.display()))?;
        println!("Created backup: {}", backup.display());
    }
    save(target, &target_notes, ACTIVE_TITLE)?;
    println!("Imported entries.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_unique_text_fragment() {
        let mut notes = Notes::default();
        add(&mut notes, Kind::Note, Horizon::Short, "buy coffee".into());
        assert_eq!(find_index(&notes, "coffee").unwrap(), 0);
    }

    #[test]
    fn parses_short_capture_codes() {
        let (kind, horizon, value) =
            capture_text(vec!["build".into(), "RISC-V".into(), "-st".into()]).unwrap();
        assert_eq!(kind, Kind::Todo);
        assert_eq!(horizon, Horizon::Short);
        assert_eq!(value, "build RISC-V");
        assert_eq!(parse_capture_code("-li"), Some((Kind::Idea, Horizon::Long)));
        assert_eq!(parse_capture_code("-lx"), Some((Kind::Item, Horizon::Long)));
    }
}
