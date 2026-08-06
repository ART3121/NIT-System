use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use crate::model::{Kind, Notes};

use super::App;

pub(super) fn draw(frame: &mut ratatui::Frame, notes: &Notes, app: &mut App, indexes: &[usize]) {
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
    frame.render_widget(
        Block::default().style(Style::default().bg(background)),
        frame.area(),
    );
    let areas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(5),
        Constraint::Length(3),
    ])
    .split(frame.area());
    let filters = format!(
        "{} entries  ·  Type: {}  ·  Horizon: {}  ·  View: {}",
        indexes.len(),
        app.kind
            .map(|value| value.to_string())
            .unwrap_or("all".into()),
        app.horizon
            .map(|value| value.to_string())
            .unwrap_or("all".into()),
        if app.archived { "archived" } else { "active" }
    );
    frame.render_widget(
        Paragraph::new(filters)
            .style(Style::default().fg(foreground).bg(panel))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("NIT System")
                    .title_style(Style::default().fg(blue).add_modifier(Modifier::BOLD))
                    .border_style(Style::default().fg(blue))
                    .style(Style::default().bg(panel)),
            ),
        areas[0],
    );
    let entry_areas =
        Layout::horizontal([Constraint::Min(1), Constraint::Length(20)]).split(areas[1]);
    app.list_state
        .select((!indexes.is_empty()).then_some(app.selected));
    let items: Vec<ListItem> = indexes
        .iter()
        .enumerate()
        .map(|(row, index)| {
            let entry = &notes.entries[*index];
            let entry_color = kind_color(entry.kind, blue, cyan, magenta, yellow);
            let prefix = if row == app.selected { "> " } else { "  " };
            ListItem::new(format!("{prefix}{}", entry.text.replace('\n', " ↵ "))).style(
                if row == app.selected {
                    Style::default()
                        .fg(entry_color)
                        .bg(selected_background)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(entry_color).bg(panel)
                },
            )
        })
        .collect();
    frame.render_stateful_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(
                    "Entries · {}/{}",
                    indexes
                        .get(app.selected)
                        .map_or(0, |_| app.selected.saturating_add(1)),
                    indexes.len()
                ))
                .title_style(Style::default().fg(blue).add_modifier(Modifier::BOLD))
                .border_style(Style::default().fg(muted))
                .style(Style::default().bg(panel)),
        ),
        entry_areas[0],
        &mut app.list_state,
    );
    let classifications: Vec<ListItem> = indexes
        .iter()
        .enumerate()
        .map(|(row, index)| {
            let entry = &notes.entries[*index];
            let classification_color = kind_color(entry.kind, blue, cyan, magenta, yellow);
            ListItem::new(format!("{}/{}", entry.horizon, entry.kind)).style(
                if row == app.selected {
                    Style::default()
                        .fg(classification_color)
                        .bg(selected_background)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(classification_color).bg(panel)
                },
            )
        })
        .collect();
    frame.render_stateful_widget(
        List::new(classifications).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Class")
                .title_style(Style::default().fg(magenta).add_modifier(Modifier::BOLD))
                .border_style(Style::default().fg(muted))
                .style(Style::default().bg(panel)),
        ),
        entry_areas[1],
        &mut app.list_state,
    );
    let preview = indexes
        .get(app.selected)
        .map(|index| {
            let entry = &notes.entries[*index];
            format!("{}/{}\n{}", entry.horizon, entry.kind, entry.text)
        })
        .unwrap_or_else(|| "No entries in this view.".into());
    frame.render_widget(
        Paragraph::new(preview)
            .style(Style::default().fg(foreground).bg(panel))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Selected")
                    .title_style(Style::default().fg(cyan).add_modifier(Modifier::BOLD))
                    .border_style(Style::default().fg(muted))
                    .style(Style::default().bg(panel)),
            ),
        areas[2],
    );
    let help = match &app.capture_input {
        Some(input) => format!(":{input}"),
        None => format!(
            ":w text -st | :q quit | ↑↓/jk move | 1-5 type | h/s/m/l horizon | c create | Enter/e edit | a archive | u restore | dd delete | v archived | r reload  {}",
            app.message
        ),
    };
    let capture_mode = app.capture_input.is_some();
    frame.render_widget(
        Paragraph::new(help)
            .style(
                Style::default()
                    .fg(if capture_mode { yellow } else { muted })
                    .bg(panel),
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(if capture_mode {
                        "Capture — :w add / :q quit / Esc cancel"
                    } else {
                        "Help"
                    })
                    .title_style(Style::default().fg(if capture_mode { yellow } else { red }))
                    .border_style(Style::default().fg(if capture_mode { yellow } else { muted }))
                    .style(Style::default().bg(panel)),
            ),
        areas[3],
    );
}

fn kind_color(kind: Kind, idea: Color, note: Color, item: Color, todo: Color) -> Color {
    match kind {
        Kind::Idea => idea,
        Kind::Note => note,
        Kind::Item => item,
        Kind::Todo => todo,
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, Terminal};

    use crate::model::{Entry, Horizon};

    use super::*;

    #[test]
    fn scrolls_to_keep_the_selection_visible() {
        let notes = Notes {
            entries: (0..20)
                .map(|number| Entry {
                    kind: Kind::Note,
                    horizon: Horizon::Short,
                    text: format!("Entry {number}"),
                })
                .collect(),
        };
        let indexes: Vec<usize> = (0..notes.entries.len()).collect();
        let mut app = App {
            selected: 19,
            ..App::default()
        };
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| draw(frame, &notes, &mut app, &indexes))
            .unwrap();

        assert!(app.list_state.offset() > 0);
    }
}
