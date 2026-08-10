mod ui;

use std::{
    collections::HashSet,
    io::stdout,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{bail, Result};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseButton, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, layout::Rect, widgets::ListState, Terminal};

use crate::{
    ai::{generate_roadmap_cancellable, GenerateOutcome},
    commands::{attach_roadmap, capture_text, create, roadmap_target},
    editor,
    ids::IdSequences,
    model::{Entry, EntryId, Horizon, Kind, Notes, Roadmap},
    storage::{load, save, ACTIVE_TITLE, ARCHIVE_TITLE},
    workspace::Workspace,
};

struct App {
    kind: Option<Kind>,
    horizon: Option<Horizon>,
    archived: bool,
    selected: usize,
    list_state: ListState,
    delete_armed: bool,
    capture_input: Option<String>,
    message: String,
    expanded: HashSet<EntryId>,
    ai_receiver: Option<Receiver<Result<GenerateOutcome>>>,
    ai_target: Option<Entry>,
    ai_dialog: Option<AiDialog>,
    ai_review_scroll: u16,
    ai_cancel: Option<Arc<AtomicBool>>,
    ai_started: Option<Instant>,
    ai_stage: Option<&'static str>,
    ai_menu_open: bool,
    ai_menu_selected: usize,
    help_open: bool,
    help_button_area: Option<Rect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AiTool {
    Roadmap,
    Summarize,
    Rewrite,
    ExtractTasks,
    Organize,
}

const AI_TOOLS: [AiTool; 5] = [
    AiTool::Roadmap,
    AiTool::Summarize,
    AiTool::Rewrite,
    AiTool::ExtractTasks,
    AiTool::Organize,
];

impl AiTool {
    fn name(self) -> &'static str {
        match self {
            Self::Roadmap => "Generate Roadmap",
            Self::Summarize => "Summarize",
            Self::Rewrite => "Rewrite note",
            Self::ExtractTasks => "Extract tasks",
            Self::Organize => "Organize entry",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Roadmap => "Create an actionable Roadmap for the selected entry.",
            Self::Summarize => "Create a concise summary of the selected entry.",
            Self::Rewrite => "Rewrite the selected entry with clearer language.",
            Self::ExtractTasks => "Turn actionable parts of the entry into tasks.",
            Self::Organize => "Restructure unorganized text into useful sections.",
        }
    }

    fn enabled(self) -> bool {
        matches!(self, Self::Roadmap)
    }
}

enum AiDialog {
    Pull { model: String },
    Review { roadmap: Roadmap },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AiDialogAction {
    Accept,
    Reject,
    ScrollDown,
    ScrollUp,
}

impl Default for App {
    fn default() -> Self {
        Self {
            kind: None,
            horizon: None,
            archived: false,
            selected: 0,
            list_state: ListState::default().with_selected(Some(0)),
            delete_armed: false,
            capture_input: None,
            message: String::new(),
            expanded: HashSet::new(),
            ai_receiver: None,
            ai_target: None,
            ai_dialog: None,
            ai_review_scroll: 0,
            ai_cancel: None,
            ai_started: None,
            ai_stage: None,
            ai_menu_open: false,
            ai_menu_selected: 0,
            help_open: false,
            help_button_area: None,
        }
    }
}

fn start_ai(app: &mut App, entry: Entry, allow_pull: bool) {
    let (sender, receiver) = mpsc::channel();
    let worker_entry = entry.clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);
    thread::spawn(move || {
        let _ = sender.send(generate_roadmap_cancellable(
            &worker_entry,
            allow_pull,
            &worker_cancelled,
        ));
    });
    app.ai_target = Some(entry);
    app.ai_receiver = Some(receiver);
    app.ai_dialog = None;
    app.ai_review_scroll = 0;
    app.ai_cancel = Some(cancelled);
    app.ai_started = Some(Instant::now());
    app.ai_stage = Some(if allow_pull {
        "Preparing the model and generating the Roadmap"
    } else {
        "Generating the Roadmap with local Ollama"
    });
    app.ai_menu_open = false;
    app.message = if allow_pull {
        "Downloading model and generating Roadmap…".into()
    } else {
        "Generating Roadmap with local Ollama…".into()
    };
}

fn start_selected_roadmap(app: &mut App, notes: &Notes, indexes: &[usize], workspace: &Workspace) {
    if app.archived {
        app.message = "AI Roadmaps can only target active entries.".into();
    } else if app.ai_receiver.is_some() {
        app.message = "An AI operation is already running.".into();
    } else if let Some(index) = indexes.get(app.selected) {
        match notes.entries[*index].id {
            Some(id) => match roadmap_target(workspace, id) {
                Ok(target) => start_ai(app, target, false),
                Err(error) => app.message = error.to_string(),
            },
            None => app.message = "The selected entry needs an ID.".into(),
        }
    } else {
        app.message = "No entry selected.".into();
    }
}

fn poll_ai(app: &mut App) {
    let result = match app.ai_receiver.as_ref().map(Receiver::try_recv) {
        Some(Ok(result)) => Some(result),
        Some(Err(TryRecvError::Disconnected)) => Some(Err(anyhow::anyhow!(
            "AI worker stopped without returning a result"
        ))),
        Some(Err(TryRecvError::Empty)) | None => None,
    };
    let Some(result) = result else {
        return;
    };
    app.ai_receiver = None;
    app.ai_cancel = None;
    app.ai_started = None;
    app.ai_stage = None;
    match result {
        Ok(GenerateOutcome::NeedsPull(model)) => {
            app.message = format!("Model {model} is not installed.");
            app.ai_dialog = Some(AiDialog::Pull { model });
        }
        Ok(GenerateOutcome::Ready(roadmap)) => {
            app.message = "Review the generated Roadmap before attaching it.".into();
            app.ai_dialog = Some(AiDialog::Review { roadmap });
        }
        Err(error) => {
            app.ai_target = None;
            app.ai_dialog = None;
            app.message = error.to_string();
        }
    }
}

fn filtered_indexes(notes: &Notes, app: &App) -> Vec<usize> {
    notes
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            app.kind.is_none_or(|value| value == entry.kind)
                && app.horizon.is_none_or(|value| entry.horizon == Some(value))
        })
        .map(|(index, _)| index)
        .collect()
}

fn capture(input: &str, notes: &mut Notes, app: &mut App, workspace: &Workspace) -> Result<()> {
    let mut parts = input.split_whitespace();
    if parts.next() != Some("w") {
        bail!("Use :w <text> -st to add, or :q to quit.");
    }
    let (kind, horizon, value) = capture_text(parts.map(str::to_owned).collect())?;
    let id = create(workspace, kind, horizon, value)?;
    let active = load(&workspace.notes_path())?;
    app.archived = false;
    app.selected = active.entries.len().saturating_sub(1);
    *notes = active;
    app.message = format!("Added {id} ({}).", classification_label(kind, horizon));
    Ok(())
}

fn is_key_press(key: &KeyEvent) -> bool {
    key.kind == KeyEventKind::Press
}

fn is_ctrl_c(key: &KeyEvent) -> bool {
    is_key_press(key)
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c' | 'C'))
}

fn is_unmodified_press(key: &KeyEvent) -> bool {
    is_key_press(key) && key.modifiers.is_empty()
}

fn is_help_key(key: &KeyEvent) -> bool {
    is_key_press(key)
        && match key.code {
            KeyCode::Char('H') => key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT,
            KeyCode::Char('h') => key.modifiers == KeyModifiers::SHIFT,
            _ => false,
        }
}

fn contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && column < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}

fn ai_dialog_action(key: &KeyEvent) -> Option<AiDialogAction> {
    if !is_key_press(key) || !(key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT) {
        return None;
    }
    match key.code {
        KeyCode::Char('y' | 'Y') => Some(AiDialogAction::Accept),
        KeyCode::Char('n' | 'N') | KeyCode::Esc => Some(AiDialogAction::Reject),
        KeyCode::Down | KeyCode::Char('j') => Some(AiDialogAction::ScrollDown),
        KeyCode::Up | KeyCode::Char('k') => Some(AiDialogAction::ScrollUp),
        _ => None,
    }
}

pub(crate) fn run(workspace: &Workspace) -> Result<()> {
    let active_path = workspace.notes_path();
    let archived_path = workspace.archive_path();
    let mut notes = load(&active_path)?;
    let archived_notes = load(&archived_path)?;
    let mut sequences = IdSequences::load(&workspace.next_ids_path())?;
    sequences.reconcile([&notes, &archived_notes])?;
    let missing_ids = notes
        .entries
        .iter()
        .chain(&archived_notes.entries)
        .filter(|entry| entry.id.is_none())
        .count();
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;
    let mut app = App::default();
    if missing_ids > 0 {
        app.message = format!("{missing_ids} entries need IDs; run nit -assign-ids.");
    }
    let result = (|| -> Result<()> {
        loop {
            poll_ai(&mut app);
            let indexes = filtered_indexes(&notes, &app);
            if app.selected >= indexes.len() {
                app.selected = indexes.len().saturating_sub(1);
            }
            terminal.draw(|frame| ui::draw(frame, &notes, &mut app, &indexes))?;
            if !event::poll(Duration::from_millis(250))? {
                continue;
            }
            let key = match event::read()? {
                Event::Key(key) => key,
                Event::Mouse(mouse)
                    if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                        && app
                            .help_button_area
                            .is_some_and(|area| contains(area, mouse.column, mouse.row)) =>
                {
                    app.help_open = !app.help_open;
                    app.message = if app.help_open {
                        "Help opened.".into()
                    } else {
                        "Help closed.".into()
                    };
                    continue;
                }
                Event::Mouse(_)
                | Event::Resize(_, _)
                | Event::FocusGained
                | Event::FocusLost
                | Event::Paste(_) => continue,
            };
            if is_ctrl_c(&key) {
                break;
            }
            if !is_key_press(&key) {
                continue;
            }
            if app.ai_dialog.is_some() {
                match ai_dialog_action(&key) {
                    Some(AiDialogAction::Accept) => match app.ai_dialog.take() {
                        Some(AiDialog::Pull { .. }) => {
                            if let Some(target) = app.ai_target.clone() {
                                start_ai(&mut app, target, true);
                            }
                        }
                        Some(AiDialog::Review { roadmap }) => {
                            if let Some(target) = app.ai_target.clone() {
                                match attach_roadmap(workspace, &target, roadmap.clone()) {
                                    Ok(()) => {
                                        app.ai_target = None;
                                        notes = load(&active_path)?;
                                        if let Some(id) = target.id {
                                            app.expanded.insert(id);
                                        }
                                        app.message =
                                            "Roadmap accepted and attached to the entry.".into();
                                    }
                                    Err(error) => {
                                        app.ai_dialog = Some(AiDialog::Review { roadmap });
                                        app.message = format!(
                                            "Could not attach the Roadmap: {error}. Press N to reject."
                                        );
                                    }
                                }
                            }
                        }
                        None => {}
                    },
                    Some(AiDialogAction::Reject) => {
                        app.ai_dialog = None;
                        app.ai_target = None;
                        app.message = "Roadmap rejected; no files were changed.".into();
                    }
                    Some(AiDialogAction::ScrollDown) => {
                        app.ai_review_scroll = app.ai_review_scroll.saturating_add(1)
                    }
                    Some(AiDialogAction::ScrollUp) => {
                        app.ai_review_scroll = app.ai_review_scroll.saturating_sub(1)
                    }
                    None => {}
                }
                continue;
            }
            if app.help_open {
                if is_help_key(&key) || (is_unmodified_press(&key) && key.code == KeyCode::Esc) {
                    app.help_open = false;
                    app.message = "Help closed.".into();
                }
                continue;
            }
            if app.ai_menu_open {
                if !is_unmodified_press(&key) {
                    continue;
                }
                match key.code {
                    KeyCode::Char('i') | KeyCode::Esc => {
                        app.ai_menu_open = false;
                        app.message = "AI mode closed.".into();
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        app.ai_menu_selected = (app.ai_menu_selected + 1).min(AI_TOOLS.len() - 1);
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        app.ai_menu_selected = app.ai_menu_selected.saturating_sub(1);
                    }
                    KeyCode::Enter => {
                        let tool = AI_TOOLS[app.ai_menu_selected];
                        if tool.enabled() {
                            start_selected_roadmap(&mut app, &notes, &indexes, workspace);
                        } else {
                            app.message =
                                format!("{} is planned but not available yet.", tool.name());
                        }
                    }
                    _ => {}
                }
                continue;
            }
            if app.capture_input.is_some() {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    continue;
                }
                match key.code {
                    KeyCode::Esc => {
                        app.capture_input = None;
                        app.message = "Capture cancelled.".into();
                    }
                    KeyCode::Enter => {
                        let input = app.capture_input.take().unwrap_or_default();
                        if input.trim() == "q" {
                            break;
                        }
                        if input.trim() == "ai-roadmap" {
                            start_selected_roadmap(&mut app, &notes, &indexes, workspace);
                        } else if let Err(error) = capture(&input, &mut notes, &mut app, workspace)
                        {
                            app.message = error.to_string();
                        }
                    }
                    KeyCode::Backspace => {
                        if let Some(input) = app.capture_input.as_mut() {
                            input.pop();
                        }
                    }
                    KeyCode::Char(character) => {
                        if let Some(input) = app.capture_input.as_mut() {
                            input.push(character);
                        }
                    }
                    _ => {}
                }
                continue;
            }
            if is_help_key(&key) {
                app.help_open = true;
                app.message = "Help opened.".into();
                continue;
            }
            if !is_unmodified_press(&key) {
                continue;
            }
            if key.code != KeyCode::Char('d') {
                app.delete_armed = false;
            }
            match key.code {
                KeyCode::Char('i') => {
                    if app.ai_receiver.is_some() {
                        app.message = "An AI operation is already running.".into();
                    } else {
                        app.ai_menu_open = true;
                        app.message = "AI mode opened. Select an action and press Enter.".into();
                    }
                }
                KeyCode::Char(':') => {
                    app.capture_input = Some(String::new());
                    app.message.clear();
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if app.selected + 1 < indexes.len() {
                        app.selected += 1;
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => app.selected = app.selected.saturating_sub(1),
                KeyCode::Char('1') => app.kind = None,
                KeyCode::Char('2') => app.kind = Some(Kind::Idea),
                KeyCode::Char('3') => app.kind = Some(Kind::Note),
                KeyCode::Char('4') => app.kind = Some(Kind::Item),
                KeyCode::Char('5') => app.kind = Some(Kind::Todo),
                KeyCode::Char('h') => app.horizon = None,
                KeyCode::Char('s') => app.horizon = Some(Horizon::Short),
                KeyCode::Char('m') => app.horizon = Some(Horizon::Medium),
                KeyCode::Char('l') => app.horizon = Some(Horizon::Long),
                KeyCode::Char('v') => {
                    app.archived = !app.archived;
                    notes = load(if app.archived {
                        &archived_path
                    } else {
                        &active_path
                    })?;
                    app.selected = 0;
                }
                KeyCode::Char('u') if app.archived => {
                    if let Some(index) = indexes.get(app.selected) {
                        let entry = notes.entries.remove(*index);
                        let mut active = load(&active_path)?;
                        active.entries.push(entry);
                        save(&active_path, &active, ACTIVE_TITLE)?;
                        save(&archived_path, &notes, ARCHIVE_TITLE)?;
                        app.archived = false;
                        app.selected = active.entries.len().saturating_sub(1);
                        notes = active;
                        app.message = "Restored to active entries.".into();
                    }
                }
                KeyCode::Char('r') => {
                    notes = load(if app.archived {
                        &archived_path
                    } else {
                        &active_path
                    })?;
                    app.message = "Reloaded.".into();
                }
                KeyCode::Char('c') if !app.archived => {
                    terminal.clear()?;
                    disable_raw_mode()?;
                    execute!(
                        terminal.backend_mut(),
                        DisableMouseCapture,
                        LeaveAlternateScreen
                    )?;
                    let drafted = editor::open("");
                    execute!(
                        terminal.backend_mut(),
                        EnterAlternateScreen,
                        EnableMouseCapture
                    )?;
                    enable_raw_mode()?;
                    match drafted {
                        Ok(value) => {
                            let kind = app.kind.unwrap_or(Kind::Todo);
                            let horizon = kind
                                .uses_horizon()
                                .then_some(app.horizon.unwrap_or(Horizon::Short));
                            match create(workspace, kind, horizon, value) {
                                Ok(id) => {
                                    notes = load(&active_path)?;
                                    app.selected = notes.entries.len().saturating_sub(1);
                                    app.message = format!(
                                        "Added {id} ({}).",
                                        classification_label(kind, horizon)
                                    );
                                }
                                Err(error) => app.message = error.to_string(),
                            }
                        }
                        Err(error) => app.message = error.to_string(),
                    }
                }
                KeyCode::Char('a') if !app.archived => {
                    if let Some(index) = indexes.get(app.selected) {
                        let entry = notes.entries.remove(*index);
                        let mut archived = load(&archived_path)?;
                        archived.entries.push(entry);
                        save(&archived_path, &archived, ARCHIVE_TITLE)?;
                        save(&active_path, &notes, ACTIVE_TITLE)?;
                        app.message = "Archived.".into();
                    }
                }
                KeyCode::Char('d') => {
                    if !app.delete_armed {
                        app.delete_armed = true;
                        app.message = "Press d again to delete permanently.".into();
                    } else if let Some(index) = indexes.get(app.selected) {
                        notes.entries.remove(*index);
                        save(
                            if app.archived {
                                &archived_path
                            } else {
                                &active_path
                            },
                            &notes,
                            if app.archived {
                                ARCHIVE_TITLE
                            } else {
                                ACTIVE_TITLE
                            },
                        )?;
                        app.delete_armed = false;
                        app.message = "Deleted permanently.".into();
                    } else {
                        app.delete_armed = false;
                        app.message = "No entry selected.".into();
                    }
                }
                KeyCode::Enter => {
                    if let Some(index) = indexes.get(app.selected) {
                        let entry = &notes.entries[*index];
                        if let (Some(id), Some(roadmap)) = (entry.id, entry.roadmap.as_ref()) {
                            if app.expanded.remove(&id) {
                                app.message = "Roadmap collapsed.".into();
                            } else {
                                app.expanded.insert(id);
                                app.message =
                                    format!("Roadmap expanded ({} steps).", roadmap.steps.len());
                            }
                        } else {
                            app.message = "The selected entry has no Roadmap.".into();
                        }
                    }
                }
                KeyCode::Char('e') => {
                    if let Some(index) = indexes.get(app.selected) {
                        terminal.clear()?;
                        disable_raw_mode()?;
                        execute!(
                            terminal.backend_mut(),
                            DisableMouseCapture,
                            LeaveAlternateScreen
                        )?;
                        let edited = editor::open(&notes.entries[*index].text);
                        execute!(
                            terminal.backend_mut(),
                            EnterAlternateScreen,
                            EnableMouseCapture
                        )?;
                        enable_raw_mode()?;
                        match edited {
                            Ok(value) => {
                                notes.entries[*index].text = value;
                                save(
                                    if app.archived {
                                        &archived_path
                                    } else {
                                        &active_path
                                    },
                                    &notes,
                                    if app.archived {
                                        ARCHIVE_TITLE
                                    } else {
                                        ACTIVE_TITLE
                                    },
                                )?;
                                app.message = "Saved.".into();
                            }
                            Err(error) => app.message = error.to_string(),
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    })();
    if let Some(cancel) = &app.ai_cancel {
        cancel.store(true, Ordering::Relaxed);
    }
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    result
}

fn classification_label(kind: Kind, horizon: Option<Horizon>) -> String {
    horizon
        .map(|value| format!("{value}/{kind}"))
        .unwrap_or_else(|| kind.to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crossterm::event::{KeyEvent, KeyEventKind, KeyModifiers};

    use super::*;

    #[test]
    fn capture_adds_an_active_entry() {
        let directory = std::env::temp_dir().join(format!("nit-tui-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let workspace = Workspace::init(&directory).unwrap().workspace;
        let mut notes = Notes::default();
        let mut app = App {
            archived: true,
            ..App::default()
        };

        capture("w Nova Nota -n", &mut notes, &mut app, &workspace).unwrap();
        assert_eq!(notes.entries.len(), 1);
        assert_eq!(notes.entries[0].kind, Kind::Note);
        assert_eq!(notes.entries[0].horizon, None);
        assert_eq!(notes.entries[0].id.unwrap().to_string(), "N-0001");
        assert!(!app.archived);
        assert!(capture("Nova Nota -n", &mut notes, &mut app, &workspace).is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn modified_keys_do_not_trigger_plain_shortcuts() {
        for character in ['c', 'd', 'v', 's'] {
            let key = KeyEvent::new(KeyCode::Char(character), KeyModifiers::CONTROL);
            assert!(!is_unmodified_press(&key));
        }
    }

    #[test]
    fn ctrl_c_is_an_explicit_exit_key() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let plain_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE);
        assert!(is_ctrl_c(&ctrl_c));
        assert!(!is_ctrl_c(&plain_c));
    }

    #[test]
    fn release_and_repeat_events_are_ignored() {
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let key = KeyEvent::new_with_kind(KeyCode::Char('d'), KeyModifiers::NONE, kind);
            assert!(!is_key_press(&key));
            assert!(!is_unmodified_press(&key));
        }
    }

    #[test]
    fn roadmap_dialog_accepts_y_and_rejects_n() {
        for key in [
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::SHIFT),
        ] {
            assert_eq!(ai_dialog_action(&key), Some(AiDialogAction::Accept));
        }
        for key in [
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT),
        ] {
            assert_eq!(ai_dialog_action(&key), Some(AiDialogAction::Reject));
        }
        assert_eq!(
            ai_dialog_action(&KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL)),
            None
        );
    }

    #[test]
    fn ai_menu_exposes_only_roadmap_as_enabled() {
        assert!(AI_TOOLS[0].enabled());
        assert_eq!(AI_TOOLS[0], AiTool::Roadmap);
        assert!(AI_TOOLS[1..].iter().all(|tool| !tool.enabled()));
    }

    #[test]
    fn uppercase_h_opens_help_without_conflicting_with_horizon_filter() {
        let uppercase = KeyEvent::new(KeyCode::Char('H'), KeyModifiers::NONE);
        let shifted_uppercase = KeyEvent::new(KeyCode::Char('H'), KeyModifiers::SHIFT);
        let shifted_lowercase = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::SHIFT);
        let horizon = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE);
        assert!(is_help_key(&uppercase));
        assert!(is_help_key(&shifted_uppercase));
        assert!(is_help_key(&shifted_lowercase));
        assert!(!is_help_key(&horizon));
    }

    #[test]
    fn help_button_hit_testing_uses_its_rendered_rectangle() {
        let area = Rect::new(2, 10, 7, 1);
        assert!(contains(area, 2, 10));
        assert!(contains(area, 8, 10));
        assert!(!contains(area, 9, 10));
        assert!(!contains(area, 2, 11));
    }
}
