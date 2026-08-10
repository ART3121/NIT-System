use std::{collections::HashSet, fs, path::Path};

use anyhow::{anyhow, bail, Context, Result};

use crate::model::{Entry, EntryId, Horizon, Kind, Notes, Roadmap, RoadmapStep, HORIZONS};

pub(crate) const ACTIVE_TITLE: &str = "NIT System";
pub(crate) const ARCHIVE_TITLE: &str = "NIT System — Archived";

#[derive(Clone, Copy)]
enum Scope {
    Timeless,
    Horizon(Horizon),
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
    let mut scope = None;
    let mut kind = None;
    let mut current: Option<Entry> = None;
    let mut current_step: Option<RoadmapStep> = None;
    for line in source.lines() {
        if let Some(value) = line.trim_start().strip_prefix("- ") {
            push_entry(&mut notes, &mut current, &mut current_step)?;
            if let (Some(scope), Some(kind)) = (scope, kind) {
                let horizon = section_horizon(scope, kind)?;
                let (id, value) = entry_id_and_text(value);
                if let Some(id) = id {
                    validate_id_for_entry(id, kind, horizon)?;
                }
                current = Some(Entry {
                    id,
                    kind,
                    horizon,
                    text: value.to_owned(),
                    roadmap: None,
                });
            }
            continue;
        }
        if let Some(value) = heading_scope(line) {
            push_entry(&mut notes, &mut current, &mut current_step)?;
            scope = Some(value);
            kind = None;
            continue;
        }
        if let Some(value) = heading_kind(line) {
            push_entry(&mut notes, &mut current, &mut current_step)?;
            kind = Some(value);
            continue;
        }
        if let Some(entry) = current.as_mut() {
            if line == "  **Roadmap**" {
                if entry.roadmap.is_some() {
                    bail!("entry contains more than one Roadmap section");
                }
                entry.roadmap = Some(Roadmap::default());
                continue;
            }
            if entry.roadmap.is_some() {
                let expected = entry.roadmap.as_ref().unwrap().steps.len()
                    + usize::from(current_step.is_some())
                    + 1;
                if let Some(title) = roadmap_step_title(line, expected) {
                    push_roadmap_step(entry, &mut current_step)?;
                    current_step = Some(RoadmapStep {
                        title: title.to_owned(),
                        description: String::new(),
                    });
                    continue;
                }
                if let Some(description) = line.strip_prefix("     ") {
                    let step = current_step
                        .as_mut()
                        .ok_or_else(|| anyhow!("Roadmap description appears before a step"))?;
                    if !step.description.is_empty() {
                        bail!("Roadmap step has more than one description line");
                    }
                    if description.trim().is_empty() {
                        bail!("Roadmap step description cannot be empty");
                    }
                    step.description = description.trim().to_owned();
                    continue;
                }
                bail!("invalid Roadmap line: {}", line.trim());
            }
            if line.starts_with("  ") && !line.trim().is_empty() {
                entry.text.push('\n');
                entry.text.push_str(line.trim_start());
            }
        }
    }
    push_entry(&mut notes, &mut current, &mut current_step)?;
    validate_unique_ids(&notes)?;
    Ok(notes)
}

fn section_horizon(scope: Scope, kind: Kind) -> Result<Option<Horizon>> {
    match (scope, kind.uses_horizon()) {
        (Scope::Horizon(horizon), true) => Ok(Some(horizon)),
        (Scope::Timeless, false) | (Scope::Horizon(_), false) => Ok(None),
        (Scope::Timeless, true) => bail!("{} entries require a time horizon", kind.heading()),
    }
}

fn validate_id_for_entry(id: EntryId, kind: Kind, horizon: Option<Horizon>) -> Result<()> {
    if id.kind() != kind {
        bail!("entry ID {id} does not match its {kind} section");
    }
    if id.is_current() && id.horizon() != horizon {
        bail!("entry ID {id} does not match its section");
    }
    if !id.is_current() && kind.uses_horizon() {
        bail!("legacy entry ID {id} is not valid for {kind}");
    }
    Ok(())
}

fn entry_id_and_text(value: &str) -> (Option<EntryId>, &str) {
    let Some(bracketed) = value.strip_prefix('[') else {
        return (None, value);
    };
    let Some((candidate, text)) = bracketed.split_once("] ") else {
        return (None, value);
    };
    match EntryId::parse(candidate) {
        Some(id) => (Some(id), text),
        None => (None, value),
    }
}

fn validate_unique_ids(notes: &Notes) -> Result<()> {
    let mut ids = HashSet::new();
    for id in notes.entries.iter().filter_map(|entry| entry.id) {
        if !ids.insert(id) {
            bail!("duplicate entry ID: {id}");
        }
    }
    Ok(())
}

fn roadmap_step_title(line: &str, expected: usize) -> Option<&str> {
    let prefix = format!("  {expected}. ");
    let title = line.strip_prefix(&prefix)?.trim();
    (!title.is_empty()).then_some(title)
}

fn push_roadmap_step(entry: &mut Entry, current_step: &mut Option<RoadmapStep>) -> Result<()> {
    if let Some(step) = current_step.take() {
        if step.title.trim().is_empty() || step.description.trim().is_empty() {
            bail!("Roadmap steps require a title and description");
        }
        entry
            .roadmap
            .as_mut()
            .expect("Roadmap exists while parsing a step")
            .steps
            .push(step);
    }
    Ok(())
}

fn push_entry(
    notes: &mut Notes,
    current: &mut Option<Entry>,
    current_step: &mut Option<RoadmapStep>,
) -> Result<()> {
    if let Some(entry) = current.as_mut() {
        push_roadmap_step(entry, current_step)?;
        if entry
            .roadmap
            .as_ref()
            .is_some_and(|roadmap| roadmap.steps.is_empty())
        {
            bail!("Roadmap must contain at least one step");
        }
    }
    if let Some(entry) = current.take() {
        if !entry.text.trim().is_empty() {
            notes.entries.push(entry);
        }
    }
    Ok(())
}

fn heading_scope(line: &str) -> Option<Scope> {
    let line = heading_text(line)?.to_lowercase();
    match line.as_str() {
        "timeless" | "atemporal" => Some(Scope::Timeless),
        "short term" | "curto prazo" => Some(Scope::Horizon(Horizon::Short)),
        "medium term" | "médio prazo" | "medio prazo" => Some(Scope::Horizon(Horizon::Medium)),
        "long term" | "longo prazo" => Some(Scope::Horizon(Horizon::Long)),
        _ => None,
    }
}

fn heading_kind(line: &str) -> Option<Kind> {
    let line = heading_text(line)?.to_lowercase();
    match line.as_str() {
        "ideas" => Some(Kind::Idea),
        "notes" => Some(Kind::Note),
        "items" | "itens" => Some(Kind::Item),
        "to-do" | "to-dos" | "todos" => Some(Kind::Todo),
        _ => None,
    }
}

fn heading_text(line: &str) -> Option<&str> {
    let line = line.trim_start();
    let marker_length = line
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if marker_length == 0 {
        return None;
    }
    let remainder = &line[marker_length..];
    if !remainder.chars().next()?.is_whitespace() {
        return None;
    }
    let heading = remainder.trim();
    (!heading.is_empty()).then_some(heading)
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

    if horizon_filter.is_none() {
        let timeless_kinds = [Kind::Note, Kind::Item];
        let has_timeless = timeless_kinds.iter().any(|kind| {
            kind_filter.is_none_or(|value| value == *kind)
                && notes
                    .entries
                    .iter()
                    .any(|entry| entry.kind == *kind && entry.horizon.is_none())
        });
        if has_timeless {
            output.push_str("\n## Timeless\n");
            for kind in timeless_kinds {
                if kind_filter.is_some_and(|value| value != kind) {
                    continue;
                }
                write_entries(&mut output, notes, kind, None);
            }
        }
    }

    for horizon in HORIZONS {
        if horizon_filter.is_some_and(|value| value != horizon) {
            continue;
        }
        let temporal_kinds = [Kind::Idea, Kind::Todo];
        let has_entries = temporal_kinds.iter().any(|kind| {
            kind_filter.is_none_or(|value| value == *kind)
                && notes
                    .entries
                    .iter()
                    .any(|entry| entry.kind == *kind && entry.horizon == Some(horizon))
        });
        if !has_entries {
            continue;
        }
        output.push_str(&format!("\n## {}\n", horizon.heading()));
        for kind in temporal_kinds {
            if kind_filter.is_some_and(|value| value != kind) {
                continue;
            }
            write_entries(&mut output, notes, kind, Some(horizon));
        }
    }
    output
}

fn write_entries(output: &mut String, notes: &Notes, kind: Kind, horizon: Option<Horizon>) {
    let entries: Vec<&Entry> = notes
        .entries
        .iter()
        .filter(|entry| entry.horizon == horizon && entry.kind == kind)
        .collect();
    if entries.is_empty() {
        return;
    }
    output.push_str(&format!("\n### {}\n", kind.heading()));
    for entry in entries {
        let mut lines = entry.text.lines();
        if let Some(first) = lines.next() {
            output.push_str("- ");
            if let Some(id) = entry.id {
                output.push_str(&format!("[{id}] "));
            }
            output.push_str(first);
            output.push('\n');
        }
        for line in lines {
            output.push_str("  ");
            output.push_str(line);
            output.push('\n');
        }
        if let Some(roadmap) = &entry.roadmap {
            output.push_str("  **Roadmap**\n");
            for (index, step) in roadmap.steps.iter().enumerate() {
                output.push_str(&format!("  {}. {}\n", index + 1, step.title));
                output.push_str("     ");
                output.push_str(&step.description);
                output.push('\n');
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_timed_notes_are_read_as_timeless_without_losing_legacy_ids() {
        let notes =
            parse_notes("# NIT System\n\n## Short Term\n\n### Notes\n- [SN-0001] old note\n")
                .unwrap();
        assert_eq!(notes.entries[0].horizon, None);
        assert_eq!(notes.entries[0].id.unwrap().to_string(), "SN-0001");
        assert!(!notes.entries[0].id.unwrap().is_current());
    }

    #[test]
    fn rendered_notes_round_trip() {
        let notes = Notes {
            entries: vec![
                Entry {
                    id: EntryId::new(None, Kind::Item, 1),
                    kind: Kind::Item,
                    horizon: None,
                    text: "reference".into(),
                    roadmap: None,
                },
                Entry {
                    id: EntryId::new(Some(Horizon::Long), Kind::Idea, 1),
                    kind: Kind::Idea,
                    horizon: Some(Horizon::Long),
                    text: "one\ntwo".into(),
                    roadmap: None,
                },
            ],
        };
        let rendered = render_notes(&notes, None, None, ACTIVE_TITLE);
        assert!(rendered.contains("## Timeless"));
        assert!(rendered.contains("### Items"));
        assert!(rendered.contains("## Long Term"));
        assert!(rendered.contains("### Ideas"));
        assert_eq!(parse_notes(&rendered).unwrap(), notes);
    }

    #[test]
    fn entry_text_is_never_parsed_as_a_heading() {
        let notes = parse_notes(
            "# NIT System\n\n## Timeless\n\n### Notes\n\
             - Review notes from the meeting\n\
             - Study short term memory\n\
             - Organize project ideas\n\
             - Check items from the list\n",
        )
        .unwrap();
        assert_eq!(notes.entries.len(), 4);
        assert!(notes
            .entries
            .iter()
            .all(|entry| entry.kind == Kind::Note && entry.horizon.is_none()));
    }

    #[test]
    fn headings_require_exact_markdown_heading_text() {
        assert!(matches!(
            heading_scope("## Short Term"),
            Some(Scope::Horizon(Horizon::Short))
        ));
        assert_eq!(heading_kind("### Notes"), Some(Kind::Note));
        assert!(heading_scope("- Study short term memory").is_none());
        assert_eq!(heading_kind("- Organize project ideas"), None);
        assert!(heading_scope("## Short Term Planning").is_none());
        assert_eq!(heading_kind("### Notes from the meeting"), None);
        assert!(heading_scope("##Short Term").is_none());
    }

    #[test]
    fn ids_round_trip_without_changing_entry_text() {
        let notes = Notes {
            entries: vec![Entry {
                id: EntryId::new(Some(Horizon::Short), Kind::Todo, 1),
                kind: Kind::Todo,
                horizon: Some(Horizon::Short),
                text: "Fix parser".into(),
                roadmap: None,
            }],
        };
        let rendered = render_notes(&notes, None, None, ACTIVE_TITLE);
        assert!(rendered.contains("- [ST-0001] Fix parser"));
        assert_eq!(parse_notes(&rendered).unwrap(), notes);
    }

    #[test]
    fn roadmaps_round_trip_as_human_readable_markdown() {
        let notes = Notes {
            entries: vec![Entry {
                id: EntryId::new(Some(Horizon::Long), Kind::Idea, 1),
                kind: Kind::Idea,
                horizon: Some(Horizon::Long),
                text: "Learn Kubernetes".into(),
                roadmap: Some(Roadmap {
                    steps: vec![
                        RoadmapStep {
                            title: "Containers".into(),
                            description: "Understand images and runtimes.".into(),
                        },
                        RoadmapStep {
                            title: "Pods".into(),
                            description: "Run a basic workload.".into(),
                        },
                    ],
                }),
            }],
        };
        let rendered = render_notes(&notes, None, None, ACTIVE_TITLE);
        assert!(rendered.contains("  **Roadmap**"));
        assert!(rendered.contains("  1. Containers"));
        assert!(rendered.contains("     Understand images and runtimes."));
        assert_eq!(parse_notes(&rendered).unwrap(), notes);
    }

    #[test]
    fn malformed_roadmaps_are_rejected() {
        let missing_description = "# NIT System\n\n## Long Term\n\n### Ideas\n- [LI-0001] Learn\n  **Roadmap**\n  1. First\n";
        let skipped_number = "# NIT System\n\n## Long Term\n\n### Ideas\n- [LI-0001] Learn\n  **Roadmap**\n  2. Second\n     Description\n";
        assert!(parse_notes(missing_description).is_err());
        assert!(parse_notes(skipped_number).is_err());
    }

    #[test]
    fn id_must_match_its_section_and_be_unique() {
        let wrong_section = "# NIT System\n\n## Timeless\n\n### Notes\n- [X-0001] mismatch\n";
        assert!(parse_notes(wrong_section).is_err());
        let duplicate =
            "# NIT System\n\n## Timeless\n\n### Notes\n- [N-0001] one\n- [N-0001] two\n";
        assert!(parse_notes(duplicate).is_err());
    }
}
