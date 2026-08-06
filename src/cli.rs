use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::{
    commands::{archive_entry, capture_text, create, find_index, import_notes, text},
    editor,
    model::{Horizon, Kind},
    storage::{archive_path, load, notes_path, render_notes, save, ACTIVE_TITLE, ARCHIVE_TITLE},
    tui,
};

#[derive(Parser)]
#[command(name = "nit", version, about = "NIT System terminal notes")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Fast capture. End the text with -st, -li, -ln, etc. With no message, opens the TUI.
    #[arg(trailing_var_arg = true)]
    message: Vec<String>,
}

#[derive(Subcommand)]
enum Command {
    Idea {
        #[arg(value_enum)]
        horizon: Option<Horizon>,
        text: Vec<String>,
    },
    Note {
        #[arg(value_enum)]
        horizon: Option<Horizon>,
        text: Vec<String>,
    },
    Item {
        #[arg(value_enum)]
        horizon: Option<Horizon>,
        text: Vec<String>,
    },
    Todo {
        #[arg(value_enum)]
        horizon: Option<Horizon>,
        text: Vec<String>,
    },
    List {
        #[arg(value_enum)]
        kind: Option<Kind>,
        #[arg(value_enum)]
        horizon: Option<Horizon>,
        #[arg(long)]
        archived: bool,
    },
    Show {
        text: Vec<String>,
        #[arg(long)]
        archived: bool,
    },
    Edit {
        text: Vec<String>,
        #[arg(long)]
        archived: bool,
    },
    Archive {
        text: Vec<String>,
    },
    Tui,
    Import {
        path: PathBuf,
    },
}

pub(crate) fn run() -> Result<()> {
    let cli = Cli::parse();
    let path = notes_path()?;
    match cli.command {
        None => {
            if cli.message.is_empty() {
                tui::run(&path)?;
            } else {
                let (kind, horizon, value) = capture_text(cli.message)?;
                create(&path, kind, horizon, value)?;
            }
        }
        Some(Command::Idea {
            text: value,
            horizon,
        }) => create(
            &path,
            Kind::Idea,
            horizon.unwrap_or(Horizon::Short),
            text(value)?,
        )?,
        Some(Command::Note {
            text: value,
            horizon,
        }) => create(
            &path,
            Kind::Note,
            horizon.unwrap_or(Horizon::Short),
            text(value)?,
        )?,
        Some(Command::Item {
            text: value,
            horizon,
        }) => create(
            &path,
            Kind::Item,
            horizon.unwrap_or(Horizon::Short),
            text(value)?,
        )?,
        Some(Command::Todo {
            text: value,
            horizon,
        }) => create(
            &path,
            Kind::Todo,
            horizon.unwrap_or(Horizon::Short),
            text(value)?,
        )?,
        Some(Command::List {
            kind,
            horizon,
            archived,
        }) => {
            let storage = if archived {
                archive_path()?
            } else {
                path.clone()
            };
            let notes = load(&storage)?;
            print!(
                "{}",
                render_notes(
                    &notes,
                    kind,
                    horizon,
                    if archived {
                        ARCHIVE_TITLE
                    } else {
                        ACTIVE_TITLE
                    }
                )
            );
        }
        Some(Command::Show {
            text: query,
            archived,
        }) => {
            let storage = if archived {
                archive_path()?
            } else {
                path.clone()
            };
            let notes = load(&storage)?;
            let query = text(query)?;
            let entry = &notes.entries[find_index(&notes, &query)?];
            println!("{}/{}\n\n{}", entry.horizon, entry.kind, entry.text);
        }
        Some(Command::Edit {
            text: query,
            archived,
        }) => {
            let storage = if archived {
                archive_path()?
            } else {
                path.clone()
            };
            let mut notes = load(&storage)?;
            let query = text(query)?;
            let index = find_index(&notes, &query)?;
            notes.entries[index].text = editor::open(&notes.entries[index].text)?;
            save(
                &storage,
                &notes,
                if archived {
                    ARCHIVE_TITLE
                } else {
                    ACTIVE_TITLE
                },
            )?;
            println!("Updated.");
        }
        Some(Command::Archive { text: query }) => archive_entry(&text(query)?)?,
        Some(Command::Tui) => tui::run(&path)?,
        Some(Command::Import { path: source }) => {
            let source = if source.is_absolute() {
                source
            } else {
                std::env::current_dir()?.join(source)
            };
            import_notes(&path, &source)?;
        }
    }
    Ok(())
}
