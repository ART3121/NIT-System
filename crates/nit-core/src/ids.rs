use std::{collections::HashSet, path::Path};

use anyhow::{bail, Context, Result};

use crate::fsutil::{atomic_write, read_text_limited, MAX_STORAGE_BYTES};
use crate::model::{
    classification_from_code, legacy_classification_from_code, EntryId, Horizon, Kind, Notes,
};

const CLASSIFICATION_COUNT: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IdSequences {
    next: [u64; CLASSIFICATION_COUNT],
}

impl Default for IdSequences {
    fn default() -> Self {
        Self {
            next: [1; CLASSIFICATION_COUNT],
        }
    }
}

impl IdSequences {
    pub(crate) fn from_values(next: [u64; CLASSIFICATION_COUNT]) -> Result<Self> {
        if next.contains(&0) {
            bail!("Vault ID sequences must be greater than zero");
        }
        Ok(Self { next })
    }

    pub(crate) fn values(&self) -> [u64; CLASSIFICATION_COUNT] {
        self.next
    }

    pub(crate) fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let source = read_text_limited(path, MAX_STORAGE_BYTES)
            .with_context(|| format!("could not read ID sequences from {}", path.display()))?;
        Self::parse(&source).with_context(|| format!("could not parse {}", path.display()))
    }

    pub(crate) fn save(&self, path: &Path) -> Result<()> {
        atomic_write(path, self.render())
    }

    pub(crate) fn reconcile<'a>(
        &mut self,
        collections: impl IntoIterator<Item = &'a Notes>,
    ) -> Result<()> {
        self.reconcile_impl(collections, false)
    }

    pub(crate) fn reconcile_for_timeless_migration<'a>(
        &mut self,
        collections: impl IntoIterator<Item = &'a Notes>,
    ) -> Result<()> {
        self.reconcile_impl(collections, true)
    }

    fn reconcile_impl<'a>(
        &mut self,
        collections: impl IntoIterator<Item = &'a Notes>,
        allow_legacy: bool,
    ) -> Result<()> {
        let mut ids = HashSet::new();
        for notes in collections {
            for entry in &notes.entries {
                if let Some(id) = entry.id {
                    if !ids.insert(id) {
                        bail!("duplicate entry ID: {id}");
                    }
                    if !id.is_current() {
                        if allow_legacy {
                            continue;
                        }
                        bail!(
                            "workspace contains timed Note/Item IDs; run `nit -migrate-timeless`"
                        );
                    }
                    if id.horizon() != entry.horizon || id.kind() != entry.kind {
                        bail!(
                            "entry ID {id} does not match its {} classification",
                            entry.classification()
                        );
                    }
                    let index = sequence_index(entry.horizon, entry.kind)?;
                    self.next[index] = self.next[index].max(id.sequence().saturating_add(1));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn require_all_ids<'a>(
        collections: impl IntoIterator<Item = &'a Notes>,
    ) -> Result<()> {
        let missing = collections
            .into_iter()
            .flat_map(|notes| &notes.entries)
            .filter(|entry| entry.id.is_none())
            .count();
        if missing > 0 {
            bail!("workspace contains {missing} entries without IDs; run `nit -assign-ids` first");
        }
        Ok(())
    }

    pub(crate) fn allocate(&mut self, horizon: Option<Horizon>, kind: Kind) -> Result<EntryId> {
        let index = sequence_index(horizon, kind)?;
        let sequence = self.next[index];
        self.next[index] = sequence
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("ID sequence exhausted for {kind}"))?;
        EntryId::new(horizon, kind, sequence)
            .ok_or_else(|| anyhow::anyhow!("invalid horizon for {kind}"))
    }

    fn parse(source: &str) -> Result<Self> {
        let mut sequences = Self::default();
        let mut seen_current = HashSet::new();
        for line in source.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split_whitespace();
            let code = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("missing classification code"))?;
            let value = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("missing sequence for {code}"))?;
            if parts.next().is_some() {
                bail!("unexpected content in ID sequence line: {line}");
            }
            let value: u64 = value
                .parse()
                .with_context(|| format!("invalid next sequence for {code}: {value}"))?;
            if value == 0 {
                bail!("next sequence for {code} must be greater than zero");
            }

            if let Some((horizon, kind)) = classification_from_code(code) {
                let index = sequence_index(horizon, kind)?;
                if !seen_current.insert(index) {
                    bail!("duplicate ID classification: {code}");
                }
                sequences.next[index] = sequences.next[index].max(value);
                continue;
            }
            if let Some((_legacy_horizon, kind)) = legacy_classification_from_code(code) {
                let index = sequence_index(None, kind)?;
                sequences.next[index] = sequences.next[index].max(value);
                continue;
            }
            bail!("unknown ID classification: {code}");
        }
        Ok(sequences)
    }

    pub(crate) fn render(&self) -> String {
        let mut output = String::from("# Next NIT entry IDs\n");
        for (horizon, kind) in classifications() {
            let index = sequence_index(horizon, kind).expect("valid built-in classification");
            let code = horizon
                .map(|value| format!("{}{}", value.id_code(), kind.id_code()))
                .unwrap_or_else(|| kind.id_code().to_string());
            output.push_str(&format!("{code} {}\n", self.next[index]));
        }
        output
    }
}

fn classifications() -> [(Option<Horizon>, Kind); CLASSIFICATION_COUNT] {
    [
        (Some(Horizon::Short), Kind::Idea),
        (Some(Horizon::Medium), Kind::Idea),
        (Some(Horizon::Long), Kind::Idea),
        (None, Kind::Note),
        (None, Kind::Item),
        (Some(Horizon::Short), Kind::Todo),
        (Some(Horizon::Medium), Kind::Todo),
        (Some(Horizon::Long), Kind::Todo),
    ]
}

fn sequence_index(horizon: Option<Horizon>, kind: Kind) -> Result<usize> {
    classifications()
        .iter()
        .position(|classification| *classification == (horizon, kind))
        .ok_or_else(|| anyhow::anyhow!("invalid horizon for {kind}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Entry;

    #[test]
    fn classifications_have_independent_sequences() {
        let mut sequences = IdSequences::default();
        assert_eq!(
            sequences
                .allocate(Some(Horizon::Short), Kind::Todo)
                .unwrap()
                .to_string(),
            "ST-0001"
        );
        assert_eq!(
            sequences
                .allocate(Some(Horizon::Short), Kind::Todo)
                .unwrap()
                .to_string(),
            "ST-0002"
        );
        assert_eq!(
            sequences.allocate(None, Kind::Note).unwrap().to_string(),
            "N-0001"
        );
        assert_eq!(
            sequences
                .allocate(Some(Horizon::Long), Kind::Idea)
                .unwrap()
                .to_string(),
            "LI-0001"
        );
    }

    #[test]
    fn reconciliation_prevents_reusing_existing_ids() {
        let notes = Notes {
            entries: vec![Entry {
                id: EntryId::new(Some(Horizon::Long), Kind::Idea, 3),
                horizon: Some(Horizon::Long),
                kind: Kind::Idea,
                text: "existing".into(),
                body: String::new(),
                roadmap: None,
            }],
        };
        let mut sequences = IdSequences::default();
        sequences.reconcile([&notes]).unwrap();
        assert_eq!(
            sequences
                .allocate(Some(Horizon::Long), Kind::Idea)
                .unwrap()
                .to_string(),
            "LI-0004"
        );
    }

    #[test]
    fn legacy_sequence_files_fold_timed_notes_and_items() {
        let source = "SN 2\nMN 7\nLN 4\nSX 3\nMX 2\nLX 9\nST 5\n";
        let sequences = IdSequences::parse(source).unwrap();
        let rendered = sequences.render();
        assert!(rendered.contains("N 7"));
        assert!(rendered.contains("X 9"));
        assert!(rendered.contains("ST 5"));
    }

    #[test]
    fn legacy_ids_require_explicit_migration() {
        let entry = Entry {
            id: EntryId::parse("SN-0001"),
            horizon: None,
            kind: Kind::Note,
            text: "first".into(),
            body: String::new(),
            roadmap: None,
        };
        assert!(IdSequences::default()
            .reconcile([&Notes {
                entries: vec![entry.clone()]
            }])
            .is_err());
        assert!(IdSequences::default()
            .reconcile_for_timeless_migration([&Notes {
                entries: vec![entry]
            }])
            .is_ok());
    }

    #[test]
    fn sequence_file_is_human_readable() {
        let mut sequences = IdSequences::default();
        sequences
            .allocate(Some(Horizon::Short), Kind::Todo)
            .unwrap();
        sequences.allocate(None, Kind::Note).unwrap();
        let rendered = sequences.render();
        assert!(rendered.contains("ST 2"));
        assert!(rendered.contains("N 2"));
        assert_eq!(IdSequences::parse(&rendered).unwrap(), sequences);
    }
}
