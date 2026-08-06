mod ui;

use std::{io::stdout, path::Path, time::Duration};

use anyhow::{bail, Result};
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, widgets::ListState, Terminal};

use crate::{
    commands::{add, capture_text},
    editor,
    model::{Horizon, Kind, Notes},
    storage::{archive_path, load, save, ACTIVE_TITLE, ARCHIVE_TITLE},
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
                && app.horizon.is_none_or(|value| value == entry.horizon)
        })
        .map(|(index, _)| index)
        .collect()
}

fn capture(input: &str, notes: &mut Notes, app: &mut App, active_path: &Path) -> Result<()> {
    let mut parts = input.split_whitespace();
    if parts.next() != Some("w") {
        bail!("Use :w <text> -st to add, or :q to quit.");
    }
    let (kind, horizon, value) = capture_text(parts.map(str::to_owned).collect())?;
    let mut active = load(active_path)?;
    add(&mut active, kind, horizon, value);
    save(active_path, &active, ACTIVE_TITLE)?;
    app.archived = false;
    app.selected = active.entries.len().saturating_sub(1);
    *notes = active;
    app.message = format!("Added {kind}/{horizon}.");
    Ok(())
}

pub(crate) fn run(active_path: &Path) -> Result<()> {
    let archived_path = archive_path()?;
    let mut notes = load(active_path)?;
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;
    let mut app = App::default();
    let result = (|| -> Result<()> {
        loop {
            let indexes = filtered_indexes(&notes, &app);
            if app.selected >= indexes.len() {
                app.selected = indexes.len().saturating_sub(1);
            }
            terminal.draw(|frame| ui::draw(frame, &notes, &mut app, &indexes))?;
            if !event::poll(Duration::from_millis(250))? {
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if app.capture_input.is_some() {
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
                        if let Err(error) = capture(&input, &mut notes, &mut app, active_path) {
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
            if key.code != KeyCode::Char('d') {
                app.delete_armed = false;
            }
            match key.code {
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
                        active_path
                    })?;
                    app.selected = 0;
                }
                KeyCode::Char('u') if app.archived => {
                    if let Some(index) = indexes.get(app.selected) {
                        let entry = notes.entries.remove(*index);
                        let mut active = load(active_path)?;
                        active.entries.push(entry);
                        save(&archived_path, &notes, ARCHIVE_TITLE)?;
                        save(active_path, &active, ACTIVE_TITLE)?;
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
                        active_path
                    })?;
                    app.message = "Reloaded.".into();
                }
                KeyCode::Char('c') if !app.archived => {
                    terminal.clear()?;
                    disable_raw_mode()?;
                    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
                    let drafted = editor::open("");
                    execute!(terminal.backend_mut(), EnterAlternateScreen)?;
                    enable_raw_mode()?;
                    match drafted {
                        Ok(value) => {
                            let kind = app.kind.unwrap_or(Kind::Todo);
                            let horizon = app.horizon.unwrap_or(Horizon::Short);
                            add(&mut notes, kind, horizon, value);
                            save(active_path, &notes, ACTIVE_TITLE)?;
                            app.message = format!("Added {kind}/{horizon}.");
                        }
                        Err(error) => app.message = error.to_string(),
                    }
                }
                KeyCode::Char('a') if !app.archived => {
                    if let Some(index) = indexes.get(app.selected) {
                        let entry = notes.entries.remove(*index);
                        let mut archived = load(&archived_path)?;
                        archived.entries.push(entry);
                        save(active_path, &notes, ACTIVE_TITLE)?;
                        save(&archived_path, &archived, ARCHIVE_TITLE)?;
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
                                active_path
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
                KeyCode::Char('e') | KeyCode::Enter => {
                    if let Some(index) = indexes.get(app.selected) {
                        terminal.clear()?;
                        disable_raw_mode()?;
                        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
                        let edited = editor::open(&notes.entries[*index].text);
                        execute!(terminal.backend_mut(), EnterAlternateScreen)?;
                        enable_raw_mode()?;
                        match edited {
                            Ok(value) => {
                                notes.entries[*index].text = value;
                                save(
                                    if app.archived {
                                        &archived_path
                                    } else {
                                        active_path
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
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn capture_adds_an_active_entry() {
        let directory = std::env::temp_dir().join(format!("nit-tui-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let active_path = directory.join(".notes");
        let mut notes = Notes::default();
        let mut app = App {
            archived: true,
            ..App::default()
        };

        capture("w Nova Nota -sn", &mut notes, &mut app, &active_path).unwrap();
        assert_eq!(notes.entries.len(), 1);
        assert_eq!(notes.entries[0].kind, Kind::Note);
        assert_eq!(notes.entries[0].horizon, Horizon::Short);
        assert!(!app.archived);
        assert!(capture("Nova Nota -sn", &mut notes, &mut app, &active_path).is_err());
        fs::remove_dir_all(directory).unwrap();
    }
}
