pub(crate) const HORIZONS: [Horizon; 3] = [Horizon::Short, Horizon::Medium, Horizon::Long];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    Idea,
    Note,
    Item,
    Todo,
}

impl Kind {
    pub(crate) fn heading(self) -> &'static str {
        match self {
            Self::Idea => "Ideas",
            Self::Note => "Notes",
            Self::Item => "Items",
            Self::Todo => "To-dos",
        }
    }

    pub(crate) fn id_code(self) -> char {
        match self {
            Self::Idea => 'I',
            Self::Note => 'N',
            Self::Item => 'X',
            Self::Todo => 'T',
        }
    }

    pub(crate) fn uses_horizon(self) -> bool {
        matches!(self, Self::Idea | Self::Todo)
    }
}

impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Idea => "idea",
                Self::Note => "note",
                Self::Item => "item",
                Self::Todo => "todo",
            }
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Horizon {
    Short,
    Medium,
    Long,
}

impl Horizon {
    pub(crate) fn heading(self) -> &'static str {
        match self {
            Self::Short => "Short Term",
            Self::Medium => "Medium Term",
            Self::Long => "Long Term",
        }
    }

    pub(crate) fn id_code(self) -> char {
        match self {
            Self::Short => 'S',
            Self::Medium => 'M',
            Self::Long => 'L',
        }
    }
}

impl std::fmt::Display for Horizon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Short => "short",
                Self::Medium => "medium",
                Self::Long => "long",
            }
        )
    }
}

pub(crate) fn valid_classification(kind: Kind, horizon: Option<Horizon>) -> bool {
    kind.uses_horizon() == horizon.is_some()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct EntryId {
    horizon: Option<Horizon>,
    kind: Kind,
    sequence: u64,
}

impl EntryId {
    pub(crate) fn new(horizon: Option<Horizon>, kind: Kind, sequence: u64) -> Option<Self> {
        (sequence > 0 && valid_classification(kind, horizon)).then_some(Self {
            horizon,
            kind,
            sequence,
        })
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        let value = value.trim().strip_prefix('@').unwrap_or(value.trim());
        let (classification, sequence) = value.split_once('-')?;
        if sequence.is_empty() || !sequence.chars().all(|character| character.is_ascii_digit()) {
            return None;
        }
        let sequence = sequence.parse().ok()?;
        if let Some((horizon, kind)) = classification_from_code(classification) {
            return Self::new(horizon, kind, sequence);
        }
        let (horizon, kind) = legacy_classification_from_code(classification)?;
        (sequence > 0).then_some(Self {
            horizon: Some(horizon),
            kind,
            sequence,
        })
    }

    pub(crate) fn horizon(self) -> Option<Horizon> {
        self.horizon
    }

    pub(crate) fn kind(self) -> Kind {
        self.kind
    }

    pub(crate) fn sequence(self) -> u64 {
        self.sequence
    }

    pub(crate) fn is_current(self) -> bool {
        valid_classification(self.kind, self.horizon)
    }
}

impl std::fmt::Display for EntryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(horizon) = self.horizon {
            write!(
                f,
                "{}{}-{:04}",
                horizon.id_code(),
                self.kind.id_code(),
                self.sequence
            )
        } else {
            write!(f, "{}-{:04}", self.kind.id_code(), self.sequence)
        }
    }
}

pub(crate) fn classification_from_code(value: &str) -> Option<(Option<Horizon>, Kind)> {
    let value = value.to_ascii_uppercase();
    match value.as_str() {
        "N" => Some((None, Kind::Note)),
        "X" => Some((None, Kind::Item)),
        _ => {
            let mut characters = value.chars();
            let horizon = match characters.next()? {
                'S' => Horizon::Short,
                'M' => Horizon::Medium,
                'L' => Horizon::Long,
                _ => return None,
            };
            let kind = match characters.next()? {
                'I' => Kind::Idea,
                'T' => Kind::Todo,
                _ => return None,
            };
            characters.next().is_none().then_some((Some(horizon), kind))
        }
    }
}

pub(crate) fn legacy_classification_from_code(value: &str) -> Option<(Horizon, Kind)> {
    let mut characters = value
        .chars()
        .map(|character| character.to_ascii_uppercase());
    let horizon = match characters.next()? {
        'S' => Horizon::Short,
        'M' => Horizon::Medium,
        'L' => Horizon::Long,
        _ => return None,
    };
    let kind = match characters.next()? {
        'N' => Kind::Note,
        'X' => Kind::Item,
        _ => return None,
    };
    characters.next().is_none().then_some((horizon, kind))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RoadmapStep {
    pub(crate) title: String,
    pub(crate) description: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Roadmap {
    pub(crate) steps: Vec<RoadmapStep>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) id: Option<EntryId>,
    pub(crate) kind: Kind,
    pub(crate) horizon: Option<Horizon>,
    pub(crate) text: String,
    pub(crate) roadmap: Option<Roadmap>,
}

impl Entry {
    pub(crate) fn classification(&self) -> String {
        self.horizon
            .map(|horizon| format!("{horizon}/{}", self.kind))
            .unwrap_or_else(|| self.kind.to_string())
    }
}

#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct Notes {
    pub(crate) entries: Vec<Entry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_ids_are_human_readable_and_case_insensitive() {
        let todo = EntryId::new(Some(Horizon::Short), Kind::Todo, 1).unwrap();
        let note = EntryId::new(None, Kind::Note, 1).unwrap();
        assert_eq!(todo.to_string(), "ST-0001");
        assert_eq!(note.to_string(), "N-0001");
        assert_eq!(EntryId::parse("st-1"), Some(todo));
        assert_eq!(EntryId::parse("@N-0001"), Some(note));
        assert_eq!(EntryId::parse("ST-0000"), None);
        assert_eq!(EntryId::parse("invalid"), None);
    }

    #[test]
    fn only_ideas_and_todos_accept_horizons() {
        assert!(EntryId::new(Some(Horizon::Long), Kind::Idea, 1).is_some());
        assert!(EntryId::new(None, Kind::Note, 1).is_some());
        assert!(EntryId::new(None, Kind::Item, 1).is_some());
        assert!(EntryId::new(Some(Horizon::Short), Kind::Note, 1).is_none());
        assert!(EntryId::new(None, Kind::Todo, 1).is_none());
    }

    #[test]
    fn legacy_timed_note_and_item_ids_remain_readable() {
        let note = EntryId::parse("SN-0003").unwrap();
        assert_eq!(note.kind(), Kind::Note);
        assert_eq!(note.horizon(), Some(Horizon::Short));
        assert!(!note.is_current());
    }

    #[test]
    fn ids_expand_after_four_digits() {
        let id = EntryId::new(Some(Horizon::Long), Kind::Idea, 10_000).unwrap();
        assert_eq!(id.to_string(), "LI-10000");
    }
}
