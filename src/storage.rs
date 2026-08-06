use std::{fs, path::Path, path::PathBuf};

use anyhow::{anyhow, Context, Result};

use crate::model::{Entry, Horizon, Kind, Notes, HORIZONS, KINDS};

const NOTES_FILE: &str = ".notes";
const ARCHIVE_FILE: &str = ".notes.archive";
pub(crate) const ACTIVE_TITLE: &str = "NIT System";
pub(crate) const ARCHIVE_TITLE: &str = "NIT System — Archived";

pub(crate) fn notes_path() -> Result<PathBuf> {
    Ok(std::env::current_dir()?.join(NOTES_FILE))
}

pub(crate) fn archive_path() -> Result<PathBuf> {
    Ok(std::env::current_dir()?.join(ARCHIVE_FILE))
}

pub(crate) fn load(path: &Path) -> Result<Notes> {
    if !path.exists() {
        return Ok(Notes::default());
    }
    let source =
        fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))?;
    if source.trim().is_empty() {
        return Ok(Notes::default());
    }
    parse_notes(&source).with_context(|| format!("could not parse {}", path.display()))
}

fn parse_notes(source: &str) -> Result<Notes> {
    let mut notes = Notes::default();
    let mut horizon = None;
    let mut kind = None;
    let mut current: Option<Entry> = None;
    for line in source.lines() {
        if let Some(value) = heading_horizon(line) {
            push_entry(&mut notes, &mut current);
            horizon = Some(value);
            kind = None;
            continue;
        }
        if let Some(value) = heading_kind(line) {
            push_entry(&mut notes, &mut current);
            kind = Some(value);
            continue;
        }
        if let Some(value) = line.trim_start().strip_prefix("- ") {
            push_entry(&mut notes, &mut current);
            if let (Some(horizon), Some(kind)) = (horizon, kind) {
                current = Some(Entry {
                    kind,
                    horizon,
                    text: value.to_owned(),
                });
            }
            continue;
        }
        if let Some(entry) = current.as_mut() {
            if line.starts_with("  ") && !line.trim().is_empty() {
                entry.text.push('\n');
                entry.text.push_str(line.trim_start());
            }
        }
    }
    push_entry(&mut notes, &mut current);
    Ok(notes)
}

fn push_entry(notes: &mut Notes, current: &mut Option<Entry>) {
    if let Some(entry) = current.take() {
        if !entry.text.trim().is_empty() {
            notes.entries.push(entry);
        }
    }
}

fn heading_horizon(line: &str) -> Option<Horizon> {
    let line = line.to_lowercase();
    if line.contains("curto prazo") || line.contains("short term") {
        Some(Horizon::Short)
    } else if line.contains("médio prazo")
        || line.contains("medio prazo")
        || line.contains("medium term")
    {
        Some(Horizon::Medium)
    } else if line.contains("longo prazo") || line.contains("long term") {
        Some(Horizon::Long)
    } else {
        None
    }
}

fn heading_kind(line: &str) -> Option<Kind> {
    let line = line.to_lowercase();
    if line.contains("ideas") {
        Some(Kind::Idea)
    } else if line.contains("to-dos") || line.contains("to-do") || line.contains("todos") {
        Some(Kind::Todo)
    } else if line.contains("itens") || line.contains("items") {
        Some(Kind::Item)
    } else if line.contains("notes") {
        Some(Kind::Note)
    } else {
        None
    }
}

pub(crate) fn save(path: &Path, notes: &Notes, title: &str) -> Result<()> {
    let body = render_notes(notes, None, None, title);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("notes path has no parent"))?;
    let temporary = parent.join(format!(
        "{}.tmp",
        path.file_name().unwrap().to_string_lossy()
    ));
    fs::write(&temporary, body)
        .with_context(|| format!("could not write {}", temporary.display()))?;
    fs::rename(&temporary, path)
        .with_context(|| format!("could not replace {}", path.display()))?;
    Ok(())
}

pub(crate) fn render_notes(
    notes: &Notes,
    kind_filter: Option<Kind>,
    horizon_filter: Option<Horizon>,
    title: &str,
) -> String {
    let mut output = format!("# {title}\n");
    for horizon in HORIZONS {
        if horizon_filter.is_some_and(|value| value != horizon) {
            continue;
        }
        let entries_for_horizon = notes.entries.iter().any(|entry| {
            entry.horizon == horizon && kind_filter.is_none_or(|value| value == entry.kind)
        });
        if !entries_for_horizon {
            continue;
        }
        output.push_str(&format!("\n## {}\n", horizon.heading()));
        for kind in KINDS {
            if kind_filter.is_some_and(|value| value != kind) {
                continue;
            }
            let entries: Vec<&Entry> = notes
                .entries
                .iter()
                .filter(|entry| entry.horizon == horizon && entry.kind == kind)
                .collect();
            if entries.is_empty() {
                continue;
            }
            output.push_str(&format!("\n### {}\n", kind.heading()));
            for entry in entries {
                let mut lines = entry.text.lines();
                if let Some(first) = lines.next() {
                    output.push_str("- ");
                    output.push_str(first);
                    output.push('\n');
                }
                for line in lines {
                    output.push_str("  ");
                    output.push_str(line);
                    output.push('\n');
                }
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_previous_notes_layout() {
        let notes = parse_notes(
            "# Curto Prazo\n# Ideas\n- first\n# Notes\n- second\n# Médio Prazo\n# To-do\n- third\n",
        )
        .unwrap();
        assert_eq!(notes.entries.len(), 3);
        assert_eq!(notes.entries[2].horizon, Horizon::Medium);
        assert_eq!(notes.entries[2].kind, Kind::Todo);
    }

    #[test]
    fn rendered_notes_round_trip() {
        let notes = Notes {
            entries: vec![
                Entry {
                    kind: Kind::Item,
                    horizon: Horizon::Medium,
                    text: "reference".into(),
                },
                Entry {
                    kind: Kind::Idea,
                    horizon: Horizon::Long,
                    text: "one\ntwo".into(),
                },
            ],
        };
        let rendered = render_notes(&notes, None, None, ACTIVE_TITLE);
        assert!(rendered.contains("## Long Term"));
        assert!(rendered.contains("### Ideas"));
        assert!(rendered.contains("## Medium Term"));
        assert!(rendered.contains("### Items"));
        assert!(!rendered.contains("Longo Prazo"));
        assert!(!rendered.contains("### Itens"));
        let decoded = parse_notes(&rendered).unwrap();
        assert_eq!(decoded.entries, notes.entries);
    }
}
