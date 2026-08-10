use std::{
    io::{self, IsTerminal, Write},
    path::PathBuf,
};

use anyhow::{bail, Result};

use crate::{
    ai::{generate_roadmap, roadmap_text, GenerateOutcome},
    commands::{
        archive_entry, assign_missing_ids, attach_roadmap, capture_text, create, find_index,
        import_notes, migrate_timeless_ids, parse_capture_code, roadmap_target, text,
    },
    editor,
    model::{EntryId, Horizon, Kind},
    storage::{load, render_notes, save, ACTIVE_TITLE, ARCHIVE_TITLE},
    tui,
    workspace::{appears_ignored, ensure_private, migrate, Workspace},
};

#[derive(Debug, PartialEq, Eq)]
enum InitMode {
    Standard,
    Private,
    Tracked,
}

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Tui,
    Capture(Vec<String>),
    Init(InitMode),
    Migrate,
    Root,
    Path,
    Status,
    AssignIds,
    MigrateTimeless,
    AiRoadmap(EntryId),
    List {
        classification: Option<(Kind, Option<Horizon>)>,
        archived: bool,
    },
    Show {
        query: Vec<String>,
        archived: bool,
    },
    Edit {
        query: Vec<String>,
        archived: bool,
    },
    Archive(Vec<String>),
    Import(PathBuf),
    Help,
    Version,
}

pub(crate) fn run() -> Result<()> {
    let arguments = std::env::args().skip(1).collect();
    execute(parse_arguments(arguments)?)
}

fn parse_arguments(arguments: Vec<String>) -> Result<Action> {
    let Some(command) = arguments.first().map(String::as_str) else {
        return Ok(Action::Tui);
    };
    let remaining = &arguments[1..];

    match command {
        "-init" => parse_init(remaining),
        "-migrate" => no_arguments(remaining, Action::Migrate, "nit -migrate"),
        "-root" => no_arguments(remaining, Action::Root, "nit -root"),
        "-path" => no_arguments(remaining, Action::Path, "nit -path"),
        "-status" => no_arguments(remaining, Action::Status, "nit -status"),
        "-assign-ids" => no_arguments(remaining, Action::AssignIds, "nit -assign-ids"),
        "-migrate-timeless" => {
            no_arguments(remaining, Action::MigrateTimeless, "nit -migrate-timeless")
        }
        "-ai-roadmap" => match remaining {
            [id] => EntryId::parse(id)
                .map(Action::AiRoadmap)
                .ok_or_else(|| anyhow::anyhow!("usage: nit -ai-roadmap <ID>")),
            _ => bail!("usage: nit -ai-roadmap <ID>"),
        },
        "-list" => parse_list(remaining),
        "-show" => {
            let (query, archived) = query_options(remaining)?;
            Ok(Action::Show { query, archived })
        }
        "-edit" => {
            let (query, archived) = query_options(remaining)?;
            Ok(Action::Edit { query, archived })
        }
        "-archive" => Ok(Action::Archive(remaining.to_vec())),
        "-import" => match remaining {
            [path] => Ok(Action::Import(PathBuf::from(path))),
            _ => bail!("usage: nit -import <path>"),
        },
        "-tui" => no_arguments(remaining, Action::Tui, "nit -tui"),
        "-help" | "--help" | "-h" => Ok(Action::Help),
        "-version" | "--version" | "-V" => Ok(Action::Version),
        value if value.starts_with('-') => {
            bail!("unknown command '{value}'; run 'nit -help' for usage")
        }
        _ => Ok(Action::Capture(arguments)),
    }
}

fn parse_init(arguments: &[String]) -> Result<Action> {
    match arguments {
        [] => Ok(Action::Init(InitMode::Standard)),
        [option] if option == "--private" => Ok(Action::Init(InitMode::Private)),
        [option] if option == "--tracked" => Ok(Action::Init(InitMode::Tracked)),
        _ => bail!("usage: nit -init [--private|--tracked]"),
    }
}

fn no_arguments(arguments: &[String], action: Action, usage: &str) -> Result<Action> {
    if !arguments.is_empty() {
        bail!("usage: {usage}");
    }
    Ok(action)
}

fn parse_list(arguments: &[String]) -> Result<Action> {
    let mut classification = None;
    let mut archived = false;

    for argument in arguments {
        if argument == "--archived" {
            if archived {
                bail!("--archived was provided more than once");
            }
            archived = true;
        } else if let Some(value) = parse_capture_code(argument) {
            if classification.replace(value).is_some() {
                bail!("provide only one classification code to -list");
            }
        } else {
            bail!("unknown -list argument '{argument}'; use a code such as -n or --archived");
        }
    }

    Ok(Action::List {
        classification,
        archived,
    })
}

fn query_options(arguments: &[String]) -> Result<(Vec<String>, bool)> {
    let mut query = Vec::new();
    let mut archived = false;
    for argument in arguments {
        if argument == "--archived" {
            if archived {
                bail!("--archived was provided more than once");
            }
            archived = true;
        } else {
            query.push(argument.clone());
        }
    }
    Ok((query, archived))
}

fn execute(action: Action) -> Result<()> {
    match action {
        Action::Init(mode) => initialize(mode),
        Action::Migrate => {
            let workspace = migrate(&std::env::current_dir()?)?;
            println!(
                "Migrated legacy NIT workspace to {}.",
                workspace.nit_dir().display()
            );
            Ok(())
        }
        Action::Help => {
            print_help();
            Ok(())
        }
        Action::Version => {
            println!("nit {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        action => {
            let workspace = Workspace::discover()?;
            execute_in_workspace(action, &workspace)
        }
    }
}

fn initialize(mode: InitMode) -> Result<()> {
    let result = Workspace::init(&std::env::current_dir()?)?;
    let tracked_warning = mode == InitMode::Tracked && appears_ignored(result.workspace.root())?;
    match mode {
        InitMode::Private => ensure_private(result.workspace.root())?,
        InitMode::Tracked => {}
        InitMode::Standard => {}
    }
    if result.already_existed {
        println!(
            "NIT workspace already exists at {}",
            result.workspace.nit_dir().display()
        );
    } else {
        println!(
            "Initialized NIT workspace at {}",
            result.workspace.nit_dir().display()
        );
    }
    if tracked_warning {
        println!("Workspace initialized, but `.nit/` appears to be ignored by .gitignore.");
    }
    Ok(())
}

fn execute_in_workspace(action: Action, workspace: &Workspace) -> Result<()> {
    match action {
        Action::Tui => tui::run(workspace)?,
        Action::Capture(message) => {
            let (kind, horizon, value) = capture_text(message)?;
            let id = create(workspace, kind, horizon, value)?;
            println!("Added {id} ({}).", classification_label(kind, horizon));
        }
        Action::Root => println!("{}", workspace.root().display()),
        Action::Path => println!("{}", workspace.nit_dir().display()),
        Action::Status => {
            let active = load(&workspace.notes_path())?;
            let archived = load(&workspace.archive_path())?;
            println!(
                "NIT Workspace\n\nRoot: {}\nStorage: {}\nActive entries: {}\nArchived entries: {}",
                workspace.root().display(),
                workspace.nit_dir().display(),
                active.entries.len(),
                archived.entries.len()
            );
        }
        Action::AssignIds => {
            let assigned = assign_missing_ids(workspace)?;
            if assigned == 0 {
                println!("All entries already have IDs.");
            } else {
                println!("Assigned IDs to {assigned} entries.");
                println!("Backups: notes.pre-ids.bak, archive.pre-ids.bak");
            }
        }
        Action::MigrateTimeless => {
            let migrated = migrate_timeless_ids(workspace)?;
            if migrated == 0 {
                println!("All Note and Item IDs are already timeless.");
            } else {
                println!("Migrated {migrated} Note/Item IDs to timeless IDs.");
                println!(
                    "Backups: notes.pre-timeless.bak, archive.pre-timeless.bak, next-ids.pre-timeless.bak"
                );
            }
        }
        Action::AiRoadmap(id) => execute_ai_roadmap(workspace, id)?,
        Action::List {
            classification,
            archived,
        } => {
            let storage = if archived {
                workspace.archive_path()
            } else {
                workspace.notes_path()
            };
            let notes = load(&storage)?;
            let (kind, horizon) = classification
                .map(|(kind, horizon)| (Some(kind), horizon))
                .unwrap_or((None, None));
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
        Action::Show { query, archived } => {
            let storage = if archived {
                workspace.archive_path()
            } else {
                workspace.notes_path()
            };
            let notes = load(&storage)?;
            let query = text(query)?;
            let entry = &notes.entries[find_index(&notes, &query)?];
            println!(
                "{}\n{}\n\n{}",
                entry
                    .id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "No ID".into()),
                entry.classification(),
                entry.text
            );
            if let Some(roadmap) = &entry.roadmap {
                println!("\nRoadmap\n\n{}", roadmap_text(roadmap));
            }
        }
        Action::Edit { query, archived } => {
            let storage = if archived {
                workspace.archive_path()
            } else {
                workspace.notes_path()
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
        Action::Archive(query) => archive_entry(workspace, &text(query)?)?,
        Action::Import(source) => {
            let source = if source.is_absolute() {
                source
            } else {
                std::env::current_dir()?.join(source)
            };
            let imported = import_notes(workspace, &source)?;
            println!("Imported {imported} entries.");
        }
        Action::Init(_) | Action::Migrate | Action::Help | Action::Version => {
            unreachable!("non-workspace action reached workspace dispatcher")
        }
    }
    Ok(())
}

fn execute_ai_roadmap(workspace: &Workspace, id: EntryId) -> Result<()> {
    let entry = roadmap_target(workspace, id)?;
    println!("Preparing local Ollama and generating a Roadmap for {id}…");
    io::stdout().flush()?;
    let roadmap = match generate_roadmap(&entry, false)? {
        GenerateOutcome::Ready(roadmap) => roadmap,
        GenerateOutcome::NeedsPull(model) => {
            let size = if model == "qwen3:1.7b" {
                " (approximately 1.4 GB)"
            } else {
                ""
            };
            if !confirm(&format!(
                "Model {model} is not installed{size}. Download it?"
            ))? {
                println!("Model was not downloaded. Run `ollama pull {model}` when ready.");
                return Ok(());
            }
            println!("Downloading {model} and generating the Roadmap…");
            io::stdout().flush()?;
            match generate_roadmap(&entry, true)? {
                GenerateOutcome::Ready(roadmap) => roadmap,
                GenerateOutcome::NeedsPull(_) => unreachable!("pull was explicitly allowed"),
            }
        }
    };
    println!("\nRoadmap for {id}\n\n{}\n", roadmap_text(&roadmap));
    if confirm(&format!("Attach this Roadmap to {id}?"))? {
        attach_roadmap(workspace, &entry, roadmap)?;
        println!("Roadmap attached to {id}.");
    } else {
        println!("Roadmap discarded; no files were changed.");
    }
    Ok(())
}

fn confirm(question: &str) -> Result<bool> {
    if !io::stdin().is_terminal() {
        return Ok(false);
    }
    print!("{question} [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(answer.trim().eq_ignore_ascii_case("y"))
}

fn classification_label(kind: Kind, horizon: Option<Horizon>) -> String {
    horizon
        .map(|value| format!("{value}/{kind}"))
        .unwrap_or_else(|| kind.to_string())
}

fn print_help() {
    println!(
        "NIT System terminal notes\n\n\
Usage:\n  nit                                      Open the TUI\n  nit <text> -<code>                       Capture an entry\n  nit -init [--private|--tracked]          Create a workspace\n  nit -migrate                             Migrate legacy files\n  nit -assign-ids                          Assign IDs to existing entries\n  nit -migrate-timeless                    Convert timed Note/Item IDs safely\n  nit -ai-roadmap <ID>                     Generate a local AI Roadmap\n  nit -root                                Print the workspace root\n  nit -path                                Print the .nit directory\n  nit -status                              Show workspace statistics\n  nit -list [code] [--archived]            List entries\n  nit -show <text> [--archived]            Show one entry\n  nit -edit <text> [--archived]            Edit one entry\n  nit -archive <text>                      Archive one entry\n  nit -import <path>                       Import notes\n  nit -tui                                 Open the TUI explicitly\n  nit -help                                Show this help\n  nit -version                             Show the version\n\n\
Capture codes:\n  Ideas:     -si short  -mi medium  -li long\n  To-dos:   -st short  -mt medium  -lt long\n  Timeless: -n note    -x item"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_words_are_always_capture_text() {
        assert_eq!(
            parse_arguments(words(&["list", "of", "project", "ideas", "-n"])).unwrap(),
            Action::Capture(words(&["list", "of", "project", "ideas", "-n"]))
        );
    }

    #[test]
    fn commands_require_the_dash_prefix() {
        assert_eq!(
            parse_arguments(words(&["-show", "project", "ideas"])).unwrap(),
            Action::Show {
                query: words(&["project", "ideas"]),
                archived: false,
            }
        );
        assert_eq!(
            parse_arguments(words(&["-init", "--private"])).unwrap(),
            Action::Init(InitMode::Private)
        );
    }

    #[test]
    fn init_modes_are_mutually_exclusive() {
        assert_eq!(
            parse_arguments(words(&["-init", "--tracked"])).unwrap(),
            Action::Init(InitMode::Tracked)
        );
        assert!(parse_arguments(words(&["-init", "--private", "--tracked"])).is_err());
    }

    #[test]
    fn assign_ids_is_an_explicit_command() {
        assert_eq!(
            parse_arguments(words(&["-assign-ids"])).unwrap(),
            Action::AssignIds
        );
        assert!(parse_arguments(words(&["-assign-ids", "extra"])).is_err());
    }

    #[test]
    fn ai_roadmap_requires_one_entry_id() {
        assert_eq!(
            parse_arguments(words(&["-ai-roadmap", "li-1"])).unwrap(),
            Action::AiRoadmap(EntryId::parse("LI-0001").unwrap())
        );
        assert!(parse_arguments(words(&["-ai-roadmap"])).is_err());
        assert!(parse_arguments(words(&["-ai-roadmap", "not-an-id"])).is_err());
    }

    #[test]
    fn list_filters_use_combined_codes() {
        assert_eq!(
            parse_arguments(words(&["-list", "-n", "--archived"])).unwrap(),
            Action::List {
                classification: Some((Kind::Note, None)),
                archived: true,
            }
        );
        assert!(parse_arguments(words(&["-list", "note"])).is_err());
    }

    #[test]
    fn literal_type_subcommands_no_longer_exist() {
        assert!(matches!(
            parse_arguments(words(&["note", "something", "-n"])).unwrap(),
            Action::Capture(_)
        ));
    }

    fn words(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }
}
