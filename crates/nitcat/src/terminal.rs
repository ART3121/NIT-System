use std::{
    fs::{self, File},
    io::{self, stdout, IsTerminal, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use crossterm::{
    cursor::Show,
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use nit_core::{Entry, EntryId, Kind, Nit, Roadmap, View};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Terminal,
};

use super::{markdown, ViewerState};

const MAX_MARKDOWN_BYTES: u64 = 32 * 1024 * 1024;

struct TerminalSession {
    active: bool,
}

impl TerminalSession {
    fn enter() -> Result<Self> {
        if !stdout().is_terminal() {
            bail!("NIT Cat requires an interactive terminal");
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

fn write_stdout(arguments: std::fmt::Arguments<'_>) -> Result<()> {
    match stdout().lock().write_fmt(arguments) {
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
    ($($argument:tt)*) => {{
        write_stdout(format_args!("{}\n", format_args!($($argument)*)))?;
    }};
}

struct Document {
    title: String,
    source: String,
    origin: Origin,
    revision: u64,
}

struct RenderCache {
    revision: u64,
    width: usize,
    query: String,
    document: markdown::RenderedDocument,
}

enum Origin {
    File(PathBuf),
    Note { nit: Nit, id: EntryId, view: View },
}

impl Document {
    fn from_file(path: &Path) -> Result<Self> {
        let title = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("Markdown")
            .to_owned();
        Ok(Self {
            title,
            source: read_source(path)?,
            origin: Origin::File(path.to_path_buf()),
            revision: 0,
        })
    }

    fn from_note(nit: Nit, id: EntryId) -> Result<Self> {
        let (entry, view) = find_note(&nit, id)?;
        Ok(Self {
            title: format!("{id} · {}", entry.text),
            source: note_markdown(&entry),
            origin: Origin::Note { nit, id, view },
            revision: 0,
        })
    }

    fn reload(&mut self) -> Result<()> {
        match &self.origin {
            Origin::File(path) => self.source = read_source(path)?,
            Origin::Note { nit, id, .. } => {
                let (entry, view) = find_note(nit, *id)?;
                self.title = format!("{id} · {}", entry.text);
                self.source = note_markdown(&entry);
                self.origin = Origin::Note {
                    nit: nit.clone(),
                    id: *id,
                    view,
                };
            }
        }
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }

    fn editable(&self) -> bool {
        matches!(self.origin, Origin::Note { .. })
    }
}

#[derive(Default)]
struct StandaloneApp {
    viewer: ViewerState,
    help_open: bool,
    command_input: Option<String>,
    message: String,
    render_cache: Option<RenderCache>,
}

pub fn run_cli() -> Result<()> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    match arguments.as_slice() {
        [option] if option == "-help" || option == "--help" || option == "-h" => print_help(),
        [option] if option == "-version" || option == "--version" || option == "-V" => {
            println!("nitcat {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        [command, shell] if command == "-completions" => print_completion(shell),
        [command] if command == "-completion-ids" => print_note_ids(),
        [source] => match EntryId::parse(source) {
            Some(id) => run_note(id),
            None => run_file(Path::new(source)),
        },
        _ => bail!("usage: nitcat <file.md|NOTE-ID>"),
    }
}

fn print_completion(shell: &str) -> Result<()> {
    let script = match shell {
        "bash" => include_str!("../../../completions/bash/nitcat"),
        "zsh" => include_str!("../../../completions/zsh/_nitcat"),
        "fish" => include_str!("../../../completions/fish/nitcat.fish"),
        _ => bail!("usage: nitcat -completions <bash|zsh|fish>"),
    };
    print!("{script}");
    Ok(())
}

fn print_note_ids() -> Result<()> {
    let nit = Nit::discover()?;
    let (active, archived) = nit.all()?;
    for id in active
        .entries
        .iter()
        .chain(&archived.entries)
        .filter(|entry| entry.kind == Kind::Note)
        .filter_map(|entry| entry.id)
    {
        println!("{id}");
    }
    Ok(())
}

pub(crate) fn run_file(path: &Path) -> Result<()> {
    run_document(Document::from_file(path)?)
}

fn run_note(id: EntryId) -> Result<()> {
    if id.kind() != Kind::Note {
        bail!("{id} is not a Note ID; use an ID such as N-0001");
    }
    run_document(Document::from_note(Nit::discover()?, id)?)
}

fn run_document(mut document: Document) -> Result<()> {
    let mut app = StandaloneApp {
        viewer: ViewerState::new(),
        message: format!("Opened {}.", document.title),
        ..StandaloneApp::default()
    };

    let mut session = TerminalSession::enter()?;
    let out = stdout();
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;
    let result = run_loop(&mut terminal, &mut document, &mut app);
    session.restore()?;
    result
}

fn read_source(path: &Path) -> Result<String> {
    if !path.is_file() {
        bail!("Markdown file not found: {}", path.display());
    }
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_MARKDOWN_BYTES {
        bail!(
            "Markdown file is too large ({} bytes; limit is {} bytes)",
            metadata.len(),
            MAX_MARKDOWN_BYTES
        );
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    File::open(path)?
        .take(MAX_MARKDOWN_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_MARKDOWN_BYTES {
        bail!("Markdown file exceeds the size limit");
    }
    String::from_utf8(bytes).with_context(|| format!("{} is not valid UTF-8", path.display()))
}

fn find_note(nit: &Nit, id: EntryId) -> Result<(Entry, View)> {
    let located = nit.find_by_id(id).map_err(|_| {
        anyhow::anyhow!("Note {id} was not found in the active or archived collection")
    })?;
    if located.entry.kind != Kind::Note {
        bail!("{id} does not identify a Note");
    }
    Ok((located.entry, located.view))
}

fn note_markdown(entry: &Entry) -> String {
    let mut source = entry.body.trim_matches('\n').to_owned();
    if let Some(roadmap) = &entry.roadmap {
        if !source.is_empty() {
            source.push_str("\n\n");
        }
        source.push_str(&roadmap_markdown(roadmap));
    }
    if source.is_empty() {
        source.push_str("_This Note has no body yet. Press e to edit it._");
    }
    source
}

fn roadmap_markdown(roadmap: &Roadmap) -> String {
    let mut source = String::from("## Roadmap\n\n");
    for (index, step) in roadmap.steps.iter().enumerate() {
        source.push_str(&format!(
            "{}. **{}**\n   {}\n",
            index + 1,
            step.title,
            step.description
        ));
    }
    source
}

fn edit_note(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    document: &mut Document,
) -> Result<bool> {
    let Origin::Note { nit, id, view } = &document.origin else {
        return Ok(false);
    };
    let nit = nit.clone();
    let id = *id;
    let view = *view;
    let mut notes = nit.load(view)?;
    let index = notes
        .entries
        .iter()
        .position(|entry| entry.id == Some(id) && entry.kind == Kind::Note)
        .ok_or_else(|| anyhow::anyhow!("Note {id} no longer exists"))?;
    let initial = format!(
        "# {}\n\n{}",
        notes.entries[index].text, notes.entries[index].body
    );

    terminal.clear()?;
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    let edited = nit_editor::open(&initial);
    execute!(
        terminal.backend_mut(),
        EnterAlternateScreen,
        EnableMouseCapture
    )?;
    enable_raw_mode()?;

    let edited = edited?;
    apply_edited_note(&mut notes.entries[index], &edited)?;
    nit.save(view, &notes)?;
    document.reload()?;
    Ok(true)
}

fn apply_edited_note(entry: &mut Entry, edited: &str) -> Result<()> {
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

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    document: &mut Document,
    app: &mut StandaloneApp,
) -> Result<()> {
    loop {
        terminal.draw(|frame| draw(frame, document, app))?;
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        let event = event::read()?;
        if let Event::Mouse(mouse) = event {
            match mouse.kind {
                MouseEventKind::ScrollDown => app.viewer.scroll_by(3),
                MouseEventKind::ScrollUp => app.viewer.scroll_by(-3),
                _ => {}
            }
            continue;
        }
        let Event::Key(key) = event else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('c') {
            break;
        }
        if let Some(input) = app.viewer.search_input.as_mut() {
            match key.code {
                KeyCode::Esc => {
                    app.viewer.search_input = None;
                    app.message = "Search cancelled.".into();
                }
                KeyCode::Enter => {
                    app.viewer.search_query = app.viewer.search_input.take().unwrap_or_default();
                    app.viewer.selected_match = 0;
                    jump_to_first_match(&mut app.viewer);
                    app.message = search_status(&app.viewer);
                }
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char(character) if key.modifiers.is_empty() => input.push(character),
                _ => {}
            }
            continue;
        }
        if let Some(input) = app.command_input.as_mut() {
            match key.code {
                KeyCode::Esc => app.command_input = None,
                KeyCode::Enter => {
                    let command = app.command_input.take().unwrap_or_default();
                    if command.trim() == "q" {
                        break;
                    }
                    app.message = format!("Unknown command: :{}", command.trim());
                }
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char(character) if key.modifiers.is_empty() => input.push(character),
                _ => {}
            }
            continue;
        }
        if !key.modifiers.is_empty() && key.modifiers != KeyModifiers::SHIFT {
            continue;
        }
        if app.help_open {
            if key.code == KeyCode::Esc || matches!(key.code, KeyCode::Char('H')) {
                app.help_open = false;
            }
            continue;
        }
        match key.code {
            KeyCode::Esc if !app.viewer.search_query.is_empty() => {
                app.viewer.search_query.clear();
                app.viewer.match_lines.clear();
                app.message = "Search cleared.".into();
            }
            KeyCode::Esc | KeyCode::Char('q') => break,
            KeyCode::Char(':') => app.command_input = Some(String::new()),
            KeyCode::Char('/') => app.viewer.search_input = Some(app.viewer.search_query.clone()),
            KeyCode::Down | KeyCode::Char('j') => app.viewer.scroll_by(1),
            KeyCode::Up | KeyCode::Char('k') => app.viewer.scroll_by(-1),
            KeyCode::PageDown => {
                let page = app.viewer.viewport_height.saturating_sub(1) as isize;
                app.viewer.scroll_by(page);
            }
            KeyCode::PageUp => {
                let page = app.viewer.viewport_height.saturating_sub(1) as isize;
                app.viewer.scroll_by(-page);
            }
            KeyCode::Home | KeyCode::Char('g') => app.viewer.scroll = 0,
            KeyCode::End | KeyCode::Char('G') => app.viewer.scroll = app.viewer.max_scroll(),
            KeyCode::Char('n') => app.viewer.jump_to_match(true),
            KeyCode::Char('N') => app.viewer.jump_to_match(false),
            KeyCode::Char('r') => {
                document.reload()?;
                app.viewer.scroll = app.viewer.scroll.min(app.viewer.max_scroll());
                app.message = "Source reloaded.".into();
            }
            KeyCode::Char('e') => {
                if edit_note(terminal, document)? {
                    app.message = "Note saved and reloaded.".into();
                } else {
                    app.message = "Markdown files are read-only in NIT Cat.".into();
                }
            }
            KeyCode::Char('H') => app.help_open = true,
            _ => {}
        }
    }
    Ok(())
}

fn draw(frame: &mut ratatui::Frame, document: &Document, app: &mut StandaloneApp) {
    let background = Color::Rgb(30, 30, 30);
    let panel = Color::Rgb(37, 37, 38);
    let foreground = Color::Rgb(212, 212, 212);
    let muted = Color::Rgb(106, 106, 106);
    let blue = Color::Rgb(86, 156, 214);
    let cyan = Color::Rgb(78, 201, 176);
    let magenta = Color::Rgb(197, 134, 192);
    let yellow = Color::Rgb(220, 220, 170);
    let red = Color::Rgb(244, 71, 71);
    let selected_background = Color::Rgb(38, 79, 120);
    let input_height = if app.viewer.search_input.is_some() || app.command_input.is_some() {
        3
    } else {
        1
    };
    let areas = Layout::vertical([Constraint::Min(3), Constraint::Length(input_height)])
        .split(frame.area());
    let viewport_height = usize::from(areas[0].height.saturating_sub(2)).max(1);
    let width = usize::from(areas[0].width.saturating_sub(2)).max(1);
    let query = app
        .viewer
        .search_input
        .as_deref()
        .unwrap_or(&app.viewer.search_query);
    let cache_matches = app.render_cache.as_ref().is_some_and(|cache| {
        cache.revision == document.revision && cache.width == width && cache.query == query
    });
    if !cache_matches {
        app.render_cache = Some(RenderCache {
            revision: document.revision,
            width,
            query: query.to_owned(),
            document: markdown::render(
                &document.source,
                width,
                query,
                markdown::MarkdownPalette {
                    foreground,
                    muted,
                    blue,
                    cyan,
                    magenta,
                    yellow,
                    code_background: background,
                    match_background: selected_background,
                },
            ),
        });
    }
    let rendered = &app
        .render_cache
        .as_ref()
        .expect("render cache exists")
        .document;
    update_render_state(
        &mut app.viewer,
        &rendered.match_lines,
        rendered.lines.len(),
        viewport_height,
    );
    let selected_line = app
        .viewer
        .match_lines
        .get(app.viewer.selected_match)
        .copied();
    let start = app.viewer.scroll.min(rendered.lines.len());
    let end = start
        .saturating_add(viewport_height)
        .min(rendered.lines.len());
    let mut visible_lines = rendered.lines[start..end].to_vec();
    if let Some(line) = selected_line
        .filter(|index| (start..end).contains(index))
        .and_then(|index| visible_lines.get_mut(index - start))
    {
        for span in &mut line.spans {
            span.style = span
                .style
                .fg(yellow)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
        }
    }
    let match_label = if app.viewer.match_lines.is_empty() {
        String::new()
    } else {
        format!(
            " · match {}/{}",
            app.viewer.selected_match + 1,
            app.viewer.match_lines.len()
        )
    };
    frame.render_widget(
        Paragraph::new(visible_lines)
            .style(Style::default().fg(foreground).bg(panel))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {}{match_label} ", document.title))
                    .title_style(Style::default().fg(cyan).add_modifier(Modifier::BOLD))
                    .border_style(Style::default().fg(blue)),
            ),
        areas[0],
    );
    if let Some(input) = &app.viewer.search_input {
        frame.render_widget(
            Paragraph::new(format!("/{input}"))
                .style(Style::default().fg(yellow).bg(panel))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Find · Enter apply · Esc cancel"),
                ),
            areas[1],
        );
    } else if let Some(input) = &app.command_input {
        frame.render_widget(
            Paragraph::new(format!(":{input}"))
                .style(Style::default().fg(yellow).bg(panel))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Command · :q quit · Esc cancel"),
                ),
            areas[1],
        );
    } else {
        frame.render_widget(
            Block::default()
                .borders(Borders::BOTTOM)
                .title_bottom(
                    Line::from(if document.editable() {
                        "j/k scroll · / find · e edit · r reload · q/Esc quit"
                    } else {
                        "j/k scroll · / find · r reload · q/Esc quit"
                    })
                    .style(Style::default().fg(muted)),
                )
                .title_bottom(
                    Line::from("[H]Help")
                        .right_aligned()
                        .style(Style::default().fg(red).add_modifier(Modifier::BOLD)),
                )
                .border_style(Style::default().fg(muted)),
            areas[1],
        );
    }
    if app.help_open {
        let popup = centered(frame.area(), 72, 62);
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(format!(
                "READ     ↑/k up · ↓/j down · PageUp/PageDown page\n\
JUMP     g/Home beginning · G/End end\n\
SEARCH   / find · n next · N previous · Esc clear\n\
SOURCE   r reload{}\n\
EXIT     q · Esc · :q · Ctrl+C\n\
STATUS   {}\n\n[H] or Esc close",
                if document.editable() {
                    " · e edit Note"
                } else {
                    " · Markdown file is read-only"
                },
                app.message
            ))
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(foreground).bg(panel))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" NIT Cat Help ")
                    .border_style(Style::default().fg(yellow)),
            ),
            popup,
        );
    }
}

fn update_render_state(
    state: &mut ViewerState,
    match_lines: &[usize],
    total_lines: usize,
    viewport_height: usize,
) {
    state.viewport_height = viewport_height;
    state.total_lines = total_lines;
    state.match_lines = match_lines.to_vec();
    state.selected_match = state
        .selected_match
        .min(state.match_lines.len().saturating_sub(1));
    state.scroll = state.scroll.min(state.max_scroll());
}

fn jump_to_first_match(state: &mut ViewerState) {
    if let Some(line) = state.match_lines.first() {
        state.scroll = (*line).min(state.max_scroll());
    }
}

fn search_status(state: &ViewerState) -> String {
    if state.search_query.is_empty() {
        "Search cleared.".into()
    } else if state.match_lines.is_empty() {
        format!("No matches for '{}'.", state.search_query)
    } else {
        format!(
            "Found {} matching lines for '{}'.",
            state.match_lines.len(),
            state.search_query
        )
    }
}

fn centered(area: Rect, width_percent: u16, height_percent: u16) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - height_percent) / 2),
        Constraint::Percentage(height_percent),
        Constraint::Percentage((100 - height_percent) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - width_percent) / 2),
        Constraint::Percentage(width_percent),
        Constraint::Percentage((100 - width_percent) / 2),
    ])
    .split(vertical[1])[1]
}

fn print_help() -> Result<()> {
    println!(
        "NIT Cat — Markdown and NIT Note reader for terminals\n\n\
Usage:\n  nitcat <file.md>\n  nitcat <NOTE-ID>\n  nitcat -completions <bash|zsh|fish>\n  nitcat -help\n  nitcat -version\n\n\
Markdown files are read-only. Notes opened by ID can be edited with e.\n\
If a filename looks like an ID, prefix its path with ./"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn render_state_is_clamped_after_a_file_shrinks() {
        let mut state = ViewerState::new();
        state.scroll = 20;
        state.selected_match = 4;
        update_render_state(&mut state, &[2], 4, 3);
        assert_eq!(state.scroll, 1);
        assert_eq!(state.selected_match, 0);
    }

    #[test]
    fn note_sources_are_found_in_active_and_archived_collections() {
        let directory =
            std::env::temp_dir().join(format!("nitcat-note-source-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let workspace = nit_core::Workspace::init(&directory).unwrap().workspace;
        let nit = Nit::open(&workspace).unwrap();
        let id = nit
            .create(Kind::Note, None, "Reader architecture".into())
            .unwrap();

        assert_eq!(find_note(&nit, id).unwrap().1, View::Active);
        nit.archive(&id.to_string()).unwrap();
        assert_eq!(find_note(&nit, id).unwrap().1, View::Archived);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn note_editing_requires_a_markdown_title() {
        let mut entry = Entry {
            id: EntryId::new(None, Kind::Note, 1),
            kind: Kind::Note,
            horizon: None,
            text: "Old".into(),
            body: String::new(),
            roadmap: None,
        };
        apply_edited_note(&mut entry, "# New title\n\nNew body").unwrap();
        assert_eq!(
            (entry.text.as_str(), entry.body.as_str()),
            ("New title", "New body")
        );
        assert!(apply_edited_note(&mut entry, "Missing heading").is_err());
    }
}
