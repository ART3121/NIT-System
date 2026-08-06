use clap::ValueEnum;

pub(crate) const HORIZONS: [Horizon; 3] = [Horizon::Short, Horizon::Medium, Horizon::Long];
pub(crate) const KINDS: [Kind; 4] = [Kind::Idea, Kind::Note, Kind::Item, Kind::Todo];

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
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

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) kind: Kind,
    pub(crate) horizon: Horizon,
    pub(crate) text: String,
}

#[derive(Default, Debug)]
pub(crate) struct Notes {
    pub(crate) entries: Vec<Entry>,
}
