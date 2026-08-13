use std::{
    io::{self, IsTerminal, Write},
    path::PathBuf,
};

use anyhow::{bail, Result};

use nit_ai::{generate_roadmap, roadmap_text, GenerateOutcome};
use nit_core::{
    appears_ignored, capture_text, ensure_private, find_index, migrate, parse_capture_code,
    render_notes, text, EntryId, Horizon, Kind, Nit, View, Workspace, ACTIVE_TITLE, ARCHIVE_TITLE,
};
use nit_editor as editor;
use nit_tui as tui;

fn write_stdout(arguments: std::fmt::Arguments<'_>) -> Result<()> {
    match io::stdout().lock().write_fmt(arguments) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(error.into()),
    }
}

macro_rules! print {
    ($($argument:tt)*) => {{
        write_stdout(format_args!($($argument)*))?;
    }};
}

macro_rules! println {
    () => {{
        write_stdout(format_args!("\n"))?;
    }};
    ($($argument:tt)*) => {{
        write_stdout(format_args!("{}\n", format_args!($($argument)*)))?;
    }};
}

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
    Search {
        query: Vec<String>,
        classification: Option<(Kind, Option<Horizon>)>,
        archived: bool,
        all: bool,
    },
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
    Completions(CompletionShell),
    CompletionIds,
    Help,
    Version,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CompletionShell {
    Bash,
    Zsh,
    Fish,
}

impl CompletionShell {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "bash" => Some(Self::Bash),
            "zsh" => Some(Self::Zsh),
            "fish" => Some(Self::Fish),
            _ => None,
        }
    }
}

pub fn run() -> Result<()> {
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
        "-v" => bail!("`nit -v` was removed in 0.4.0; use `nitcat <NOTE-ID>`"),
        "-ai-roadmap" => match remaining {
            [id] => EntryId::parse(id)
                .map(Action::AiRoadmap)
                .ok_or_else(|| anyhow::anyhow!("usage: nit -ai-roadmap <ID>")),
            _ => bail!("usage: nit -ai-roadmap <ID>"),
        },
        "-search" => parse_search(remaining),
        "-ls" => parse_list(remaining),
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
        "-completions" => match remaining {
            [shell] => CompletionShell::parse(shell)
                .map(Action::Completions)
                .ok_or_else(|| anyhow::anyhow!("usage: nit -completions <bash|zsh|fish>")),
            _ => bail!("usage: nit -completions <bash|zsh|fish>"),
        },
        "-completion-ids" => no_arguments(remaining, Action::CompletionIds, "nit -completion-ids"),
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
                bail!("provide only one classification code to -ls");
            }
        } else {
            bail!("unknown -ls argument '{argument}'; use a code such as -n or --archived");
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

fn parse_search(arguments: &[String]) -> Result<Action> {
    let mut query = Vec::new();
    let mut classification = None;
    let mut archived = false;
    let mut all = false;
    for argument in arguments {
        if argument == "--archived" {
            archived = true;
        } else if argument == "--all" {
            all = true;
        } else if let Some(value) = parse_capture_code(argument) {
            if classification.replace(value).is_some() {
                bail!("provide only one classification code to -search");
            }
        } else {
            query.push(argument.clone());
        }
    }
    if archived && all {
        bail!("--archived and --all cannot be combined");
    }
    if query.is_empty() {
        bail!("usage: nit -search <query> [code] [--archived|--all]");
    }
    Ok(Action::Search {
        query,
        classification,
        archived,
        all,
    })
}

fn apply_edited_text(entry: &mut nit_core::Entry, edited: &str) -> Result<()> {
    if entry.kind != Kind::Note {
        let value = edited.trim().to_owned();
        if value.is_empty() {
            bail!("entry text cannot be empty");
        }
        entry.text = value;
        return Ok(());
    }
    let normalized = edited.replace("\r\n", "\n");
    let mut lines = normalized.lines();
    let title = lines
        .next()
        .and_then(|line| line.strip_prefix("# "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Note must start with '# <title>'"))?;
    entry.text = title.to_owned();
    entry.body = lines
        .collect::<Vec<_>>()
        .join("\n")
        .trim_matches('\n')
        .to_owned();
    Ok(())
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
            print_help()?;
            Ok(())
        }
        Action::Version => {
            println!("nit {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Action::Completions(shell) => {
            print!("{}", completion_script(shell));
            Ok(())
        }
        Action::AssignIds => {
            let workspace = Workspace::discover()?;
            let assigned = Nit::assign_missing_ids_in(&workspace)?;
            if assigned == 0 {
                println!("All entries already have IDs.");
            } else {
                println!("Assigned IDs to {assigned} entries.");
                println!("Backups: notes.pre-ids.bak, archive.pre-ids.bak");
            }
            Ok(())
        }
        Action::MigrateTimeless => {
            let workspace = Workspace::discover()?;
            let migrated = Nit::migrate_timeless_ids_in(&workspace)?;
            if migrated == 0 {
                println!("All Note and Item IDs are already timeless.");
            } else {
                println!("Migrated {migrated} Note/Item IDs to timeless IDs.");
                println!(
                    "Backups: notes.pre-timeless.bak, archive.pre-timeless.bak, next-ids.pre-timeless.bak"
                );
            }
            Ok(())
        }
        action => {
            let nit = Nit::discover()?;
            execute_in_workspace(action, &nit)
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

fn execute_in_workspace(action: Action, nit: &Nit) -> Result<()> {
    let workspace = nit
        .workspace()
        .ok_or_else(|| anyhow::anyhow!("this command requires a Plain Storage workspace path"))?;
    match action {
        Action::Tui => tui::run(nit)?,
        Action::Capture(message) => {
            let (kind, horizon, value) = capture_text(message)?;
            let id = nit.create(kind, horizon, value)?;
            println!("Added {id} ({}).", classification_label(kind, horizon));
        }
        Action::Root => println!("{}", workspace.root().display()),
        Action::Path => println!("{}", workspace.nit_dir().display()),
        Action::Status => {
            let status = nit.status()?;
            println!(
                "NIT Workspace\n\nRoot: {}\nStorage: {}\nActive entries: {}\nArchived entries: {}",
                workspace.root().display(),
                workspace.nit_dir().display(),
                status.active_entries,
                status.archived_entries
            );
        }
        Action::AiRoadmap(id) => execute_ai_roadmap(nit, id)?,
        Action::Search {
            query,
            classification,
            archived,
            all,
        } => {
            let query = text(query)?;
            let views = if all {
                vec![View::Active, View::Archived]
            } else if archived {
                vec![View::Archived]
            } else {
                vec![View::Active]
            };
            for (view, entry) in nit.search(&query, &views, classification)? {
                println!(
                    "{} · {} · {}{}",
                    entry
                        .id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "—".into()),
                    entry.classification(),
                    entry.text,
                    if view == View::Archived {
                        " · archived"
                    } else {
                        ""
                    }
                );
            }
        }
        Action::List {
            classification,
            archived,
        } => {
            let mut notes = nit.load(if archived {
                View::Archived
            } else {
                View::Active
            })?;
            for entry in &mut notes.entries {
                if entry.kind == Kind::Note {
                    entry.body.clear();
                    entry.roadmap = None;
                }
            }
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
            let notes = nit.load(if archived {
                View::Archived
            } else {
                View::Active
            })?;
            let query = text(query)?;
            let entry = &notes.entries[find_index(&notes, &query)?];
            println!(
                "{}\n{}\n\n{}",
                entry
                    .id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "No ID".into()),
                entry.classification(),
                entry.display_text()
            );
            if let Some(roadmap) = &entry.roadmap {
                println!("\nRoadmap\n\n{}", roadmap_text(roadmap));
            }
        }
        Action::Edit { query, archived } => {
            let view = if archived {
                View::Archived
            } else {
                View::Active
            };
            let mut notes = nit.load(view)?;
            let query = text(query)?;
            let index = find_index(&notes, &query)?;
            let edited = if notes.entries[index].kind == Kind::Note {
                editor::open(&format!(
                    "# {}\n\n{}",
                    notes.entries[index].text, notes.entries[index].body
                ))?
            } else {
                editor::open(&notes.entries[index].text)?
            };
            apply_edited_text(&mut notes.entries[index], &edited)?;
            nit.save(view, &notes)?;
            println!("Updated.");
        }
        Action::Archive(query) => {
            nit.archive(&text(query)?)?;
            println!("Archived.");
        }
        Action::Import(source) => {
            let source = if source.is_absolute() {
                source
            } else {
                std::env::current_dir()?.join(source)
            };
            let imported = nit.import(&source)?;
            println!("Imported {imported} entries.");
        }
        Action::CompletionIds => {
            let (active, archived) = nit.all()?;
            for id in active
                .entries
                .iter()
                .chain(&archived.entries)
                .filter_map(|entry| entry.id)
            {
                println!("{id}");
            }
        }
        Action::Init(_)
        | Action::Migrate
        | Action::AssignIds
        | Action::MigrateTimeless
        | Action::Completions(_)
        | Action::Help
        | Action::Version => {
            unreachable!("non-workspace action reached workspace dispatcher")
        }
    }
    Ok(())
}

fn execute_ai_roadmap(nit: &Nit, id: EntryId) -> Result<()> {
    let entry = nit.roadmap_target(id)?;
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
        nit.attach_roadmap(&entry, roadmap)?;
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

fn print_help() -> Result<()> {
    println!(
        r#"NIT System terminal notes

Usage:
  nit                                      Open the TUI
  nit <text> -<code>                       Capture an entry
  nit -init [--private|--tracked]          Create a workspace
  nit -migrate                             Migrate legacy files
  nit -assign-ids                          Assign IDs to existing entries
  nit -migrate-timeless                    Convert timed Note/Item IDs safely
  nit -ai-roadmap <ID>                     Generate a local AI Roadmap
  nit -root                                Print the workspace root
  nit -path                                Print the .nit directory
  nit -status                              Show workspace statistics
  nit -search <text> [code] [--archived|--all]
                                           Search titles, bodies, IDs, and Roadmaps
  nit -ls [code] [--archived]              List entries (Notes: ID and title only)
  nit -show <text> [--archived]            Show one entry
  nit -edit <text> [--archived]            Edit one entry
  nit -archive <text>                      Archive one entry
  nit -import <path>                       Import entries
  nit -completions <bash|zsh|fish>         Generate shell completions
  nit -tui                                 Open the TUI explicitly
  nit -help                                Show this help
  nit -version                             Show the version

Capture codes:
  Ideas:     -si short  -mi medium  -li long
  To-dos:   -st short  -mt medium  -lt long
  Timeless: -n note    -x item"#
    );
    Ok(())
}

fn completion_script(shell: CompletionShell) -> &'static str {
    match shell {
        CompletionShell::Bash => include_str!("../../../completions/bash/nit"),
        CompletionShell::Zsh => include_str!("../../../completions/zsh/_nit"),
        CompletionShell::Fish => include_str!("../../../completions/fish/nit.fish"),
    }
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
    fn removed_viewer_command_points_to_nitcat() {
        let error = parse_arguments(words(&["-v", "N-0001"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("nitcat"));
    }

    #[test]
    fn list_filters_use_combined_codes() {
        assert_eq!(
            parse_arguments(words(&["-ls", "-n", "--archived"])).unwrap(),
            Action::List {
                classification: Some((Kind::Note, None)),
                archived: true,
            }
        );
        assert!(parse_arguments(words(&["-ls", "note"])).is_err());
        assert!(parse_arguments(words(&["-list"])).is_err());
    }

    #[test]
    fn completion_generation_does_not_require_a_workspace() {
        assert_eq!(
            parse_arguments(words(&["-completions", "fish"])).unwrap(),
            Action::Completions(CompletionShell::Fish)
        );
        assert!(completion_script(CompletionShell::Bash).contains("complete -F _nit nit"));
        assert!(parse_arguments(words(&["-completions", "powershell"])).is_err());
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
