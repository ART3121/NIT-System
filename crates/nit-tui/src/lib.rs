mod ui;
mod viewer;

use std::{
    collections::HashSet,
    io::{stdout, IsTerminal},
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
    cursor::Show,
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseButton, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, layout::Rect, widgets::ListState, Terminal};

use nit_ai::{generate_roadmap_cancellable, GenerateOutcome};
use nit_core::{capture_text, Entry, EntryId, Horizon, Kind, NitApi, Notes, Roadmap, View};
use nit_editor as editor;
use viewer::NoteViewerState;

struct TerminalSession {
    active: bool,
}

impl TerminalSession {
    fn enter() -> Result<Self> {
        if !stdout().is_terminal() {
            bail!("NIT TUI requires an interactive terminal; use `nit -help` for CLI commands");
        }
        enable_raw_mode()?;
        let mut out = stdout();
        if let Err(error) = execute!(out, EnterAlternateScreen, EnableMouseCapture) {
            let _ = disable_raw_mode();
            return Err(error.into());
        }
        Ok(Self { active: true })
    }

    fn restore(&mut self) -> Result<()> {
        if !self.active {
            return Ok(());
        }
        self.active = false;
        let raw_result = disable_raw_mode();
        let mut out = stdout();
        let screen_result = execute!(out, DisableMouseCapture, LeaveAlternateScreen, Show);
        raw_result?;
        screen_result?;
        Ok(())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

struct App {
    screen: Screen,
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
    ai_worker: Option<thread::JoinHandle<()>>,
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
    search_input: Option<String>,
    search_query: String,
    navigator_focus: bool,
    tree_visible: bool,
    navigator_selected: usize,
    navigator_notes_expanded: bool,
    navigator_note: Option<EntryId>,
    navigator_area: Option<Rect>,
    viewer_area: Option<Rect>,
    viewer_render_cache: Option<ViewerRenderCache>,
}

struct ViewerRenderCache {
    entry: Entry,
    width: usize,
    query: String,
    document: nitcat::markdown::RenderedDocument,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Screen {
    Browser,
    NoteViewer(NoteViewerState),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct BrowserContext {
    kind: Option<Kind>,
    horizon: Option<Horizon>,
    selected: usize,
    navigator_selected: usize,
    navigator_note: Option<EntryId>,
}

impl BrowserContext {
    fn capture(app: &App) -> Self {
        Self {
            kind: app.kind,
            horizon: app.horizon,
            selected: app.selected,
            navigator_selected: app.navigator_selected,
            navigator_note: app.navigator_note,
        }
    }

    fn restore(self, app: &mut App) {
        app.kind = self.kind;
        app.horizon = self.horizon;
        app.selected = self.selected;
        app.navigator_selected = self.navigator_selected;
        app.navigator_note = self.navigator_note;
        app.list_state.select(Some(self.selected));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum NavigatorNode {
    All,
    Notes,
    Note(EntryId, String),
    Ideas,
    Items,
    Todos,
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
            screen: Screen::Browser,
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
            ai_worker: None,
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
            search_input: None,
            search_query: String::new(),
            navigator_focus: false,
            tree_visible: true,
            navigator_selected: 0,
            navigator_notes_expanded: true,
            navigator_note: None,
            navigator_area: None,
            viewer_area: None,
            viewer_render_cache: None,
        }
    }
}

fn start_ai(app: &mut App, entry: Entry, allow_pull: bool) {
    let (sender, receiver) = mpsc::channel();
    let worker_entry = entry.clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);
    let worker = thread::spawn(move || {
        let _ = sender.send(generate_roadmap_cancellable(
            &worker_entry,
            allow_pull,
            &worker_cancelled,
        ));
    });
    app.ai_target = Some(entry);
    app.ai_receiver = Some(receiver);
    app.ai_worker = Some(worker);
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

fn start_selected_roadmap(app: &mut App, notes: &Notes, indexes: &[usize], nit: &dyn NitApi) {
    if app.archived {
        app.message = "AI Roadmaps can only target active entries.".into();
    } else if app.ai_receiver.is_some() {
        app.message = "An AI operation is already running.".into();
    } else if let Some(index) = indexes.get(app.selected) {
        match notes.entries[*index].id {
            Some(id) => match nit.roadmap_target(id) {
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
    if app
        .ai_worker
        .take()
        .is_some_and(|worker| worker.join().is_err())
    {
        app.message = "AI worker stopped unexpectedly.".into();
    }
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
    let query = app
        .search_input
        .as_deref()
        .unwrap_or(&app.search_query)
        .to_lowercase();
    notes
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            app.kind.is_none_or(|value| value == entry.kind)
                && app.horizon.is_none_or(|value| entry.horizon == Some(value))
                && app.navigator_note.is_none_or(|id| entry.id == Some(id))
                && (query.is_empty() || entry.matches_lowercase_query(&query))
        })
        .map(|(index, _)| index)
        .collect()
}

fn navigator_nodes(notes: &Notes, expanded: bool) -> Vec<NavigatorNode> {
    let mut nodes = vec![NavigatorNode::All, NavigatorNode::Notes];
    if expanded {
        nodes.extend(
            notes
                .entries
                .iter()
                .filter(|entry| entry.kind == Kind::Note)
                .filter_map(|entry| {
                    entry
                        .id
                        .map(|id| NavigatorNode::Note(id, entry.text.clone()))
                }),
        );
    }
    nodes.extend([
        NavigatorNode::Ideas,
        NavigatorNode::Items,
        NavigatorNode::Todos,
    ]);
    nodes
}

fn select_navigator_node(app: &mut App, node: &NavigatorNode) {
    app.navigator_note = None;
    match node {
        NavigatorNode::All => app.kind = None,
        NavigatorNode::Notes => {
            app.kind = Some(Kind::Note);
            app.navigator_notes_expanded = !app.navigator_notes_expanded;
        }
        NavigatorNode::Note(id, _) => {
            app.kind = Some(Kind::Note);
            app.navigator_note = Some(*id);
            app.navigator_focus = false;
        }
        NavigatorNode::Ideas => app.kind = Some(Kind::Idea),
        NavigatorNode::Items => app.kind = Some(Kind::Item),
        NavigatorNode::Todos => app.kind = Some(Kind::Todo),
    }
    app.selected = 0;
}

fn open_note(app: &mut App, notes: &Notes, id: EntryId, return_to_navigator: bool) {
    if notes
        .entries
        .iter()
        .any(|entry| entry.id == Some(id) && entry.kind == Kind::Note)
    {
        app.screen = Screen::NoteViewer(NoteViewerState::from_browser(
            id,
            return_to_navigator,
            BrowserContext::capture(app),
        ));
        app.navigator_note = Some(id);
        app.kind = Some(Kind::Note);
        app.horizon = None;
        app.navigator_focus = false;
        app.message = format!("Opened {id}.");
    } else {
        app.message = format!("Note {id} is not available in this view.");
    }
}

fn close_note_viewer(app: &mut App) {
    let (return_to_navigator, browser_context) = match &app.screen {
        Screen::NoteViewer(viewer) => (viewer.return_to_navigator, viewer.browser_context.clone()),
        Screen::Browser => return,
    };
    app.screen = Screen::Browser;
    browser_context.restore(app);
    app.navigator_focus = return_to_navigator;
    app.viewer_area = None;
    app.message = "Note viewer closed.".into();
}

fn toggle_tree(app: &mut App) {
    app.tree_visible = !app.tree_visible;
    if app.tree_visible {
        app.message = "Navigator tree shown.".into();
    } else {
        app.navigator_focus = false;
        app.navigator_area = None;
        app.message = "Navigator tree hidden. Press t or Tab to show it.".into();
    }
}

fn start_roadmap_for_id(app: &mut App, nit: &dyn NitApi, id: EntryId) {
    if app.archived {
        app.message = "AI Roadmaps can only target active entries.".into();
    } else if app.ai_receiver.is_some() {
        app.message = "An AI operation is already running.".into();
    } else {
        match nit.roadmap_target(id) {
            Ok(target) => start_ai(app, target, false),
            Err(error) => app.message = error.to_string(),
        }
    }
}

fn editable_entry(entry: &Entry) -> String {
    if entry.kind == Kind::Note {
        format!("# {}\n\n{}", entry.text, entry.body)
    } else {
        entry.text.clone()
    }
}

fn apply_edited_entry(entry: &mut Entry, edited: &str) -> Result<()> {
    if entry.kind != Kind::Note {
        let value = edited.trim();
        if value.is_empty() {
            bail!("entry text cannot be empty");
        }
        entry.text = value.to_owned();
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

fn capture(input: &str, notes: &mut Notes, app: &mut App, nit: &dyn NitApi) -> Result<()> {
    let mut parts = input.split_whitespace();
    if parts.next() != Some("w") {
        bail!("Use :w <text> -st to add, or :q to quit.");
    }
    let (kind, horizon, value) = capture_text(parts.map(str::to_owned).collect())?;
    let id = nit.create(kind, horizon, value)?;
    let active = nit.load(View::Active)?;
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

pub fn run(nit: &dyn NitApi) -> Result<()> {
    run_session(nit)
}

fn run_session(nit: &dyn NitApi) -> Result<()> {
    let (mut notes, archived_notes) = nit.all()?;
    let missing_ids = notes
        .entries
        .iter()
        .chain(&archived_notes.entries)
        .filter(|entry| entry.id.is_none())
        .count();
    let mut app = App::default();
    let mut session = TerminalSession::enter()?;
    let out = stdout();
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;
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
                Event::Mouse(mouse) => {
                    if matches!(
                        mouse.kind,
                        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
                    ) && !app.navigator_focus
                        && app
                            .viewer_area
                            .is_some_and(|area| contains(area, mouse.column, mouse.row))
                    {
                        if let Screen::NoteViewer(viewer) = &mut app.screen {
                            viewer.scroll_by(if mouse.kind == MouseEventKind::ScrollDown {
                                3
                            } else {
                                -3
                            });
                        }
                    } else if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                        && app
                            .help_button_area
                            .is_some_and(|area| contains(area, mouse.column, mouse.row))
                    {
                        app.help_open = !app.help_open;
                        app.message = if app.help_open {
                            "Help opened.".into()
                        } else {
                            "Help closed.".into()
                        };
                    } else if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                        if let Some(area) = app
                            .navigator_area
                            .filter(|area| contains(*area, mouse.column, mouse.row))
                        {
                            let row =
                                usize::from(mouse.row.saturating_sub(area.y.saturating_add(1)));
                            let nodes = navigator_nodes(&notes, app.navigator_notes_expanded);
                            if let Some(node) = nodes.get(row).cloned() {
                                app.navigator_selected = row;
                                if let NavigatorNode::Note(id, _) = node {
                                    open_note(&mut app, &notes, id, true);
                                } else {
                                    app.screen = Screen::Browser;
                                    select_navigator_node(&mut app, &node);
                                }
                            }
                        }
                    }
                    continue;
                }
                Event::Resize(_, _) | Event::FocusGained | Event::FocusLost | Event::Paste(_) => {
                    continue
                }
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
                                match nit.attach_roadmap(&target, roadmap.clone()) {
                                    Ok(()) => {
                                        app.ai_target = None;
                                        notes = nit.load(View::Active)?;
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
                            let viewer_id = match &app.screen {
                                Screen::NoteViewer(viewer) => Some(viewer.id),
                                Screen::Browser => None,
                            };
                            if let Some(id) = viewer_id {
                                start_roadmap_for_id(&mut app, nit, id);
                            } else {
                                start_selected_roadmap(&mut app, &notes, &indexes, nit);
                            }
                        } else {
                            app.message =
                                format!("{} is planned but not available yet.", tool.name());
                        }
                    }
                    _ => {}
                }
                continue;
            }
            let viewer_searching = matches!(
                &app.screen,
                Screen::NoteViewer(viewer) if viewer.search_input.is_some()
            );
            if viewer_searching {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    continue;
                }
                match key.code {
                    KeyCode::Esc => {
                        if let Screen::NoteViewer(viewer) = &mut app.screen {
                            viewer.search_input = None;
                        }
                        app.message = "Note search cancelled.".into();
                    }
                    KeyCode::Enter => {
                        if let Screen::NoteViewer(viewer) = &mut app.screen {
                            viewer.search_query = viewer.search_input.take().unwrap_or_default();
                            viewer.selected_match = 0;
                            if let Some(line) = viewer.match_lines.first() {
                                viewer.scroll = (*line).min(viewer.max_scroll());
                            }
                            app.message = if viewer.search_query.is_empty() {
                                "Note search cleared.".into()
                            } else if viewer.match_lines.is_empty() {
                                format!("No matches for '{}'.", viewer.search_query)
                            } else {
                                format!(
                                    "Found {} matching lines for '{}'.",
                                    viewer.match_lines.len(),
                                    viewer.search_query
                                )
                            };
                        }
                    }
                    KeyCode::Backspace => {
                        if let Screen::NoteViewer(viewer) = &mut app.screen {
                            if let Some(input) = viewer.search_input.as_mut() {
                                input.pop();
                            }
                        }
                    }
                    KeyCode::Char(character) => {
                        if let Screen::NoteViewer(viewer) = &mut app.screen {
                            if let Some(input) = viewer.search_input.as_mut() {
                                input.push(character);
                            }
                        }
                    }
                    _ => {}
                }
                continue;
            }
            if app.search_input.is_some() {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    continue;
                }
                match key.code {
                    KeyCode::Esc => {
                        app.search_input = None;
                        app.message = "Search input cancelled.".into();
                    }
                    KeyCode::Enter => {
                        app.search_query = app.search_input.take().unwrap_or_default();
                        app.selected = 0;
                        app.message = if app.search_query.is_empty() {
                            "Search cleared.".into()
                        } else {
                            format!("Searching for '{}'.", app.search_query)
                        };
                    }
                    KeyCode::Backspace => {
                        if let Some(input) = app.search_input.as_mut() {
                            input.pop();
                        }
                    }
                    KeyCode::Char(character) => {
                        if let Some(input) = app.search_input.as_mut() {
                            input.push(character);
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
                            start_selected_roadmap(&mut app, &notes, &indexes, nit);
                        } else if let Err(error) = capture(&input, &mut notes, &mut app, nit) {
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
            if is_unmodified_press(&key) && key.code == KeyCode::Char('t') {
                toggle_tree(&mut app);
                continue;
            }
            if is_key_press(&key)
                && ((key.code == KeyCode::Tab && key.modifiers.is_empty())
                    || (key.code == KeyCode::BackTab
                        && (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)))
            {
                if !app.tree_visible {
                    app.tree_visible = true;
                    app.navigator_focus = true;
                } else {
                    app.navigator_focus = !app.navigator_focus;
                }
                app.message = if app.navigator_focus {
                    "Navigator focused.".into()
                } else if matches!(app.screen, Screen::NoteViewer(_)) {
                    "Note viewer focused.".into()
                } else {
                    "Entries focused.".into()
                };
                continue;
            }
            let shifted_viewer_key = matches!(app.screen, Screen::NoteViewer(_))
                && key.modifiers == KeyModifiers::SHIFT
                && matches!(key.code, KeyCode::Char('G' | 'N'));
            if !is_unmodified_press(&key) && !shifted_viewer_key {
                continue;
            }
            if app.navigator_focus {
                let nodes = navigator_nodes(&notes, app.navigator_notes_expanded);
                match key.code {
                    KeyCode::Char(':') => {
                        app.capture_input = Some(String::new());
                        app.navigator_focus = false;
                        app.message.clear();
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        app.navigator_selected =
                            (app.navigator_selected + 1).min(nodes.len().saturating_sub(1));
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        app.navigator_selected = app.navigator_selected.saturating_sub(1);
                    }
                    KeyCode::Right => app.navigator_notes_expanded = true,
                    KeyCode::Left => {
                        app.navigator_notes_expanded = false;
                        app.navigator_selected = app.navigator_selected.min(1);
                    }
                    KeyCode::Enter => {
                        if let Some(node) = nodes.get(app.navigator_selected).cloned() {
                            if let NavigatorNode::Note(id, _) = node {
                                open_note(&mut app, &notes, id, true);
                            } else {
                                app.screen = Screen::Browser;
                                select_navigator_node(&mut app, &node);
                            }
                        }
                    }
                    KeyCode::Char('v') => {
                        app.screen = Screen::Browser;
                        app.archived = !app.archived;
                        notes = nit.load(if app.archived {
                            View::Archived
                        } else {
                            View::Active
                        })?;
                        app.navigator_selected = 0;
                        app.navigator_note = None;
                        app.selected = 0;
                    }
                    KeyCode::Esc => app.navigator_focus = false,
                    _ => {}
                }
                continue;
            }
            if matches!(app.screen, Screen::NoteViewer(_)) {
                match key.code {
                    KeyCode::Char(':') => {
                        app.capture_input = Some(String::new());
                        app.message.clear();
                    }
                    KeyCode::Char('/') => {
                        if let Screen::NoteViewer(viewer) = &mut app.screen {
                            viewer.search_input = Some(viewer.search_query.clone());
                        }
                        app.message = "Type a query to find within this Note.".into();
                    }
                    KeyCode::Esc => {
                        let has_query = matches!(
                            &app.screen,
                            Screen::NoteViewer(viewer) if !viewer.search_query.is_empty()
                        );
                        if has_query {
                            if let Screen::NoteViewer(viewer) = &mut app.screen {
                                viewer.search_query.clear();
                                viewer.match_lines.clear();
                                viewer.selected_match = 0;
                            }
                            app.message = "Note search cleared.".into();
                        } else {
                            close_note_viewer(&mut app);
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if let Screen::NoteViewer(viewer) = &mut app.screen {
                            viewer.scroll_by(1);
                        }
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if let Screen::NoteViewer(viewer) = &mut app.screen {
                            viewer.scroll_by(-1);
                        }
                    }
                    KeyCode::PageDown => {
                        if let Screen::NoteViewer(viewer) = &mut app.screen {
                            let page = viewer.viewport_height.saturating_sub(1) as isize;
                            viewer.scroll_by(page);
                        }
                    }
                    KeyCode::PageUp => {
                        if let Screen::NoteViewer(viewer) = &mut app.screen {
                            let page = viewer.viewport_height.saturating_sub(1) as isize;
                            viewer.scroll_by(-page);
                        }
                    }
                    KeyCode::Home | KeyCode::Char('g') => {
                        if let Screen::NoteViewer(viewer) = &mut app.screen {
                            viewer.scroll = 0;
                        }
                    }
                    KeyCode::End | KeyCode::Char('G') => {
                        if let Screen::NoteViewer(viewer) = &mut app.screen {
                            viewer.scroll = viewer.max_scroll();
                        }
                    }
                    KeyCode::Char('n') => {
                        if let Screen::NoteViewer(viewer) = &mut app.screen {
                            viewer.jump_to_match(true);
                        }
                    }
                    KeyCode::Char('N') => {
                        if let Screen::NoteViewer(viewer) = &mut app.screen {
                            viewer.jump_to_match(false);
                        }
                    }
                    KeyCode::Char('i') => {
                        if app.ai_receiver.is_some() {
                            app.message = "An AI operation is already running.".into();
                        } else {
                            app.ai_menu_open = true;
                            app.message =
                                "AI mode opened for this Note. Select an action and press Enter."
                                    .into();
                        }
                    }
                    KeyCode::Char('r') => {
                        notes = nit.load(if app.archived {
                            View::Archived
                        } else {
                            View::Active
                        })?;
                        let id = match &app.screen {
                            Screen::NoteViewer(viewer) => viewer.id,
                            Screen::Browser => unreachable!(),
                        };
                        if !notes.entries.iter().any(|entry| entry.id == Some(id)) {
                            close_note_viewer(&mut app);
                            app.message = format!("Note {id} no longer exists in this view.");
                        } else {
                            app.message = "Note reloaded.".into();
                        }
                    }
                    KeyCode::Char('e') => {
                        if !nit.allows_external_editor() {
                            app.message = "External editing is disabled for Vault Storage.".into();
                            continue;
                        }
                        let id = match &app.screen {
                            Screen::NoteViewer(viewer) => viewer.id,
                            Screen::Browser => unreachable!(),
                        };
                        if let Some(index) =
                            notes.entries.iter().position(|entry| entry.id == Some(id))
                        {
                            terminal.clear()?;
                            disable_raw_mode()?;
                            execute!(
                                terminal.backend_mut(),
                                DisableMouseCapture,
                                LeaveAlternateScreen
                            )?;
                            let edited = editor::open(&editable_entry(&notes.entries[index]));
                            execute!(
                                terminal.backend_mut(),
                                EnterAlternateScreen,
                                EnableMouseCapture
                            )?;
                            enable_raw_mode()?;
                            match edited {
                                Ok(value) => {
                                    apply_edited_entry(&mut notes.entries[index], &value)?;
                                    nit.save(
                                        if app.archived {
                                            View::Archived
                                        } else {
                                            View::Active
                                        },
                                        &notes,
                                    )?;
                                    app.message = "Note saved and reloaded.".into();
                                }
                                Err(error) => app.message = error.to_string(),
                            }
                        } else {
                            close_note_viewer(&mut app);
                            app.message = format!("Note {id} no longer exists in this view.");
                        }
                    }
                    _ => {}
                }
                continue;
            }
            if key.code != KeyCode::Char('d') {
                app.delete_armed = false;
            }
            match key.code {
                KeyCode::Char('/') => {
                    app.search_input = Some(app.search_query.clone());
                    app.message = "Type a search query and press Enter.".into();
                }
                KeyCode::Esc if !app.search_query.is_empty() => {
                    app.search_query.clear();
                    app.selected = 0;
                    app.message = "Search cleared.".into();
                }
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
                KeyCode::Char('1') => {
                    app.kind = None;
                    app.navigator_note = None;
                    app.navigator_selected = 0;
                }
                KeyCode::Char('2') => {
                    app.kind = Some(Kind::Idea);
                    app.navigator_note = None;
                }
                KeyCode::Char('3') => {
                    app.kind = Some(Kind::Note);
                    app.navigator_note = None;
                }
                KeyCode::Char('4') => {
                    app.kind = Some(Kind::Item);
                    app.navigator_note = None;
                }
                KeyCode::Char('5') => {
                    app.kind = Some(Kind::Todo);
                    app.navigator_note = None;
                }
                KeyCode::Char('h') => app.horizon = None,
                KeyCode::Char('s') => app.horizon = Some(Horizon::Short),
                KeyCode::Char('m') => app.horizon = Some(Horizon::Medium),
                KeyCode::Char('l') => app.horizon = Some(Horizon::Long),
                KeyCode::Char('v') => {
                    app.archived = !app.archived;
                    notes = nit.load(if app.archived {
                        View::Archived
                    } else {
                        View::Active
                    })?;
                    app.selected = 0;
                    app.navigator_selected = 0;
                    app.navigator_note = None;
                }
                KeyCode::Char('u') if app.archived => {
                    if let Some(index) = indexes.get(app.selected) {
                        let entry = notes.entries.remove(*index);
                        let mut active = nit.load(View::Active)?;
                        active.entries.push(entry);
                        nit.save_all(&active, &notes)?;
                        app.archived = false;
                        app.selected = active.entries.len().saturating_sub(1);
                        notes = active;
                        app.message = "Restored to active entries.".into();
                    }
                }
                KeyCode::Char('r') => {
                    notes = nit.load(if app.archived {
                        View::Archived
                    } else {
                        View::Active
                    })?;
                    app.message = "Reloaded.".into();
                }
                KeyCode::Char('c') if !app.archived => {
                    if !nit.allows_external_editor() {
                        app.message =
                            "External drafting is disabled for Vault Storage; use quick capture."
                                .into();
                        continue;
                    }
                    terminal.clear()?;
                    disable_raw_mode()?;
                    execute!(
                        terminal.backend_mut(),
                        DisableMouseCapture,
                        LeaveAlternateScreen
                    )?;
                    let kind = app.kind.unwrap_or(Kind::Todo);
                    let drafted = editor::open(if kind == Kind::Note { "# " } else { "" });
                    execute!(
                        terminal.backend_mut(),
                        EnterAlternateScreen,
                        EnableMouseCapture
                    )?;
                    enable_raw_mode()?;
                    match drafted {
                        Ok(value) => {
                            let horizon = kind
                                .uses_horizon()
                                .then_some(app.horizon.unwrap_or(Horizon::Short));
                            let mut draft = Entry {
                                id: None,
                                kind,
                                horizon,
                                text: String::new(),
                                body: String::new(),
                                roadmap: None,
                            };
                            let parsed = apply_edited_entry(&mut draft, &value);
                            if let Err(error) = parsed {
                                app.message = error.to_string();
                                continue;
                            }
                            match nit.create(kind, horizon, draft.text.clone()) {
                                Ok(id) => {
                                    notes = nit.load(View::Active)?;
                                    if kind == Kind::Note && !draft.body.is_empty() {
                                        if let Some(entry) = notes
                                            .entries
                                            .iter_mut()
                                            .find(|entry| entry.id == Some(id))
                                        {
                                            entry.body = draft.body;
                                        }
                                        nit.save(View::Active, &notes)?;
                                    }
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
                        let mut archived = nit.load(View::Archived)?;
                        archived.entries.push(entry);
                        nit.save_all(&notes, &archived)?;
                        app.message = "Archived.".into();
                    }
                }
                KeyCode::Char('d') => {
                    if !app.delete_armed {
                        app.delete_armed = true;
                        app.message = "Press d again to delete permanently.".into();
                    } else if let Some(index) = indexes.get(app.selected) {
                        notes.entries.remove(*index);
                        nit.save(
                            if app.archived {
                                View::Archived
                            } else {
                                View::Active
                            },
                            &notes,
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
                        if entry.kind == Kind::Note {
                            if let Some(id) = entry.id {
                                open_note(&mut app, &notes, id, false);
                            } else {
                                app.message =
                                    "This Note needs an ID before it can be opened.".into();
                            }
                        } else if let (Some(id), Some(roadmap)) = (entry.id, entry.roadmap.as_ref())
                        {
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
                    if !nit.allows_external_editor() {
                        app.message = "External editing is disabled for Vault Storage.".into();
                        continue;
                    }
                    if let Some(index) = indexes.get(app.selected) {
                        terminal.clear()?;
                        disable_raw_mode()?;
                        execute!(
                            terminal.backend_mut(),
                            DisableMouseCapture,
                            LeaveAlternateScreen
                        )?;
                        let edited = editor::open(&editable_entry(&notes.entries[*index]));
                        execute!(
                            terminal.backend_mut(),
                            EnterAlternateScreen,
                            EnableMouseCapture
                        )?;
                        enable_raw_mode()?;
                        match edited {
                            Ok(value) => {
                                apply_edited_entry(&mut notes.entries[*index], &value)?;
                                nit.save(
                                    if app.archived {
                                        View::Archived
                                    } else {
                                        View::Active
                                    },
                                    &notes,
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
    if app
        .ai_worker
        .take()
        .is_some_and(|worker| worker.join().is_err())
    {
        app.message = "AI worker stopped unexpectedly.".into();
    }
    session.restore()?;
    result
}

fn classification_label(kind: Kind, horizon: Option<Horizon>) -> String {
    horizon
        .map(|value| format!("{value}/{kind}"))
        .unwrap_or_else(|| kind.to_string())
}

#[cfg(test)]
mod tests {
    use nit_core::Nit;

    use std::fs;

    use crossterm::event::{KeyEvent, KeyEventKind, KeyModifiers};

    use super::*;
    use nit_core::Workspace;

    #[test]
    fn capture_adds_an_active_entry() {
        let directory = std::env::temp_dir().join(format!("nit-tui-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let workspace = Workspace::init(&directory).unwrap().workspace;
        let nit = Nit::open(&workspace).unwrap();
        let mut notes = Notes::default();
        let mut app = App {
            archived: true,
            ..App::default()
        };

        capture("w Nova Nota -n", &mut notes, &mut app, &nit).unwrap();
        assert_eq!(notes.entries.len(), 1);
        assert_eq!(notes.entries[0].kind, Kind::Note);
        assert_eq!(notes.entries[0].horizon, None);
        assert_eq!(notes.entries[0].id.unwrap().to_string(), "N-0001");
        assert!(!app.archived);
        assert!(capture("Nova Nota -n", &mut notes, &mut app, &nit).is_err());
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

    #[test]
    fn search_is_incremental_and_includes_note_bodies() {
        let notes = Notes {
            entries: vec![Entry {
                id: EntryId::new(None, Kind::Note, 1),
                kind: Kind::Note,
                horizon: None,
                text: "Architecture".into(),
                body: "Retry policy belongs to the worker.".into(),
                roadmap: None,
            }],
        };
        let app = App {
            search_input: Some("retry policy".into()),
            ..App::default()
        };
        assert_eq!(filtered_indexes(&notes, &app), [0]);
    }

    #[test]
    fn note_viewer_opens_and_returns_to_its_previous_focus() {
        let id = EntryId::new(None, Kind::Note, 1).unwrap();
        let notes = Notes {
            entries: vec![Entry {
                id: Some(id),
                kind: Kind::Note,
                horizon: None,
                text: "Architecture".into(),
                body: "Details".into(),
                roadmap: None,
            }],
        };
        let mut app = App {
            kind: None,
            horizon: Some(Horizon::Long),
            selected: 4,
            navigator_selected: 2,
            navigator_note: None,
            ..App::default()
        };
        open_note(&mut app, &notes, id, true);
        assert!(matches!(app.screen, Screen::NoteViewer(ref viewer) if viewer.id == id));
        assert!(!app.navigator_focus);
        assert_eq!(app.navigator_note, Some(id));

        close_note_viewer(&mut app);
        assert_eq!(app.screen, Screen::Browser);
        assert!(app.navigator_focus);
        assert_eq!(app.kind, None);
        assert_eq!(app.horizon, Some(Horizon::Long));
        assert_eq!(app.selected, 4);
        assert_eq!(app.navigator_selected, 2);
        assert_eq!(app.navigator_note, None);
    }

    #[test]
    fn tree_toggle_hides_it_and_releases_navigator_focus() {
        let mut app = App {
            navigator_focus: true,
            navigator_area: Some(Rect::new(0, 0, 28, 20)),
            ..App::default()
        };

        toggle_tree(&mut app);
        assert!(!app.tree_visible);
        assert!(!app.navigator_focus);
        assert_eq!(app.navigator_area, None);

        toggle_tree(&mut app);
        assert!(app.tree_visible);
    }
}
