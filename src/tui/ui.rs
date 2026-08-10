use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthChar;

use crate::{
    ai::roadmap_text,
    model::{Entry, Kind, Notes},
};

use super::{AiDialog, App, AI_TOOLS};

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
    let content_area = if app.ai_menu_open {
        let menu_width = if frame.area().width >= 80 { 30 } else { 24 };
        let columns = Layout::horizontal([Constraint::Min(20), Constraint::Length(menu_width)])
            .split(frame.area());
        draw_ai_menu(
            frame,
            columns[1],
            app,
            panel,
            foreground,
            muted,
            cyan,
            selected_background,
        );
        columns[0]
    } else {
        frame.area()
    };

    let preview = indexes
        .get(app.selected)
        .map(|index| {
            let entry = &notes.entries[*index];
            let roadmap_hint = entry.roadmap.as_ref().map_or(String::new(), |roadmap| {
                format!(
                    "\n\nRoadmap: {} steps · Enter to expand/collapse",
                    roadmap.steps.len()
                )
            });
            format!(
                "{}\n{}{}",
                entry
                    .id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "No ID".into()),
                entry.text,
                roadmap_hint
            )
        })
        .unwrap_or_else(|| "No entries in this view.".into());
    let capture_mode = app.capture_input.is_some();
    let full_inner_width = usize::from(content_area.width.saturating_sub(2)).max(1);
    let help = app
        .capture_input
        .as_ref()
        .map_or_else(String::new, |input| format!(":{input}"));
    let preview_lines = wrap_text(&preview, full_inner_width);
    let help_lines = wrap_text(&help, full_inner_width);
    let (selected_height, measured_command_height) =
        panel_heights(content_area.height, preview_lines.len(), help_lines.len());
    let command_height = if capture_mode {
        measured_command_height
    } else {
        1
    };
    let areas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(selected_height),
        Constraint::Length(command_height),
    ])
    .split(content_area);

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
        Layout::horizontal([Constraint::Min(1), Constraint::Length(11)]).split(areas[1]);
    app.list_state
        .select((!indexes.is_empty()).then_some(app.selected));
    let entry_text_width = usize::from(entry_areas[0].width.saturating_sub(4)).max(1);
    let wrapped_entries: Vec<Vec<String>> = indexes
        .iter()
        .map(|index| {
            let entry = &notes.entries[*index];
            entry_display_lines(
                entry,
                entry_text_width,
                entry.id.is_some_and(|id| app.expanded.contains(&id)),
            )
        })
        .collect();
    let items: Vec<ListItem> = indexes
        .iter()
        .zip(&wrapped_entries)
        .enumerate()
        .map(|(row, (index, wrapped))| {
            let entry = &notes.entries[*index];
            let entry_color = kind_color(entry.kind, blue, cyan, magenta, yellow);
            let lines: Vec<Line> = wrapped
                .iter()
                .enumerate()
                .map(|(line_index, text)| {
                    let prefix = if line_index == 0 && row == app.selected {
                        "> "
                    } else {
                        "  "
                    };
                    Line::from(format!("{prefix}{text}"))
                })
                .collect();
            ListItem::new(lines).style(if row == app.selected {
                Style::default()
                    .fg(entry_color)
                    .bg(selected_background)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(entry_color).bg(panel)
            })
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

    frame.render_stateful_widget(
        List::new(
            indexes
                .iter()
                .zip(&wrapped_entries)
                .enumerate()
                .map(|(row, (index, wrapped))| {
                    let entry = &notes.entries[*index];
                    let color = kind_color(entry.kind, blue, cyan, magenta, yellow);
                    let mut lines = vec![Line::from(
                        entry
                            .id
                            .map(|id| id.to_string())
                            .unwrap_or_else(|| "—".into()),
                    )];
                    lines.resize_with(wrapped.len(), || Line::from(""));
                    ListItem::new(lines).style(if row == app.selected {
                        Style::default()
                            .fg(color)
                            .bg(selected_background)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(color).bg(panel)
                    })
                })
                .collect::<Vec<_>>(),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("ID")
                .title_style(Style::default().fg(yellow).add_modifier(Modifier::BOLD))
                .border_style(Style::default().fg(muted))
                .style(Style::default().bg(panel)),
        ),
        entry_areas[1],
        &mut app.list_state,
    );

    frame.render_widget(
        Paragraph::new(to_lines(&preview_lines))
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

    if capture_mode {
        let visible_command_lines = usize::from(areas[3].height.saturating_sub(2)).max(1);
        let command_scroll = help_lines.len().saturating_sub(visible_command_lines) as u16;
        frame.render_widget(
            Paragraph::new(to_lines(&help_lines))
                .scroll((command_scroll, 0))
                .style(Style::default().fg(yellow).bg(panel))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Capture — :w add / :q or Ctrl+C quit / Esc cancel")
                        .title_style(Style::default().fg(yellow))
                        .border_style(Style::default().fg(yellow))
                        .style(Style::default().bg(panel)),
                ),
            areas[3],
        );
    } else {
        frame.render_widget(
            Block::default()
                .borders(Borders::BOTTOM)
                .title_bottom(
                    Line::from("[H]Help")
                        .right_aligned()
                        .style(Style::default().fg(red).add_modifier(Modifier::BOLD)),
                )
                .border_style(Style::default().fg(muted))
                .style(Style::default().bg(background)),
            areas[3],
        );
    }
    app.help_button_area = (!capture_mode).then(|| help_button_rect(areas[3]));

    if app.ai_receiver.is_some() {
        let popup = centered(frame.area(), 68, 34);
        let elapsed = app
            .ai_started
            .map_or(std::time::Duration::ZERO, |started| started.elapsed());
        let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let spinner = frames[((elapsed.as_millis() / 120) as usize) % frames.len()];
        let elapsed_seconds = elapsed.as_secs();
        let stage = app.ai_stage.unwrap_or("Processing with local Ollama");
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(format!(
                "\n  {spinner} {stage}…\n\n  Processing in the background\n  Elapsed: {elapsed_seconds}s\n\n  The TUI remains responsive. Use :q or Ctrl+C to cancel and exit."
            ))
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(foreground).bg(panel))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" AI processing ")
                    .title_style(Style::default().fg(cyan).add_modifier(Modifier::BOLD))
                    .border_style(Style::default().fg(cyan))
                    .style(Style::default().bg(panel)),
            ),
            popup,
        );
    } else if let Some(dialog) = &app.ai_dialog {
        let popup = centered(frame.area(), 84, 76);
        frame.render_widget(Clear, popup);
        let (title, content) = match dialog {
            AiDialog::Pull { model } => (
                "Ollama model required",
                format!(
                    "Model {model} is not installed.\n\nDownload it now?{}\n\n[y] download and generate   [n/Esc] cancel",
                    if model == "qwen3:1.7b" {
                        " (approximately 1.4 GB)"
                    } else {
                        ""
                    }
                ),
            ),
            AiDialog::Review { roadmap } => (
                "Review AI Roadmap",
                format!(
                    "{}\n\n[Y] accept and attach   [N] reject   [Esc] reject   [↑/↓] scroll",
                    roadmap_text(roadmap)
                ),
            ),
        };
        frame.render_widget(
            Paragraph::new(content)
                .wrap(Wrap { trim: false })
                .scroll((app.ai_review_scroll, 0))
                .style(Style::default().fg(foreground).bg(panel))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(title)
                        .title_style(Style::default().fg(yellow).add_modifier(Modifier::BOLD))
                        .border_style(Style::default().fg(yellow))
                        .style(Style::default().bg(panel)),
                ),
            popup,
        );
    } else if app.help_open {
        let popup = centered(frame.area(), 86, 72);
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(format!(
                "{}\n\n[H] or Esc close",
                grouped_help(&app.message)
            ))
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(foreground).bg(panel))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Help ")
                    .title_style(Style::default().fg(yellow).add_modifier(Modifier::BOLD))
                    .border_style(Style::default().fg(yellow))
                    .style(Style::default().bg(panel)),
            ),
            popup,
        );
    }
}

fn grouped_help(message: &str) -> String {
    let status = if message.is_empty() { "Ready" } else { message };
    format!(
        "NAVIGATION  ↑/k previous · ↓/j next · Enter expand Roadmap\n\
ENTRY       c create · e edit · a archive · u restore · dd delete\n\
VIEW        v active/archived · r reload\n\
TYPE        1 all · 2 ideas · 3 notes · 4 items · 5 to-dos\n\
HORIZON     h all · s short · m medium · l long\n\
AI/CMD      i AI mode · : command · :w add\n\
EXIT        :q quit · Ctrl+C safe exit\n\
STATUS      {status}"
    )
}

fn help_button_rect(area: Rect) -> Rect {
    const LABEL_WIDTH: u16 = 7;
    let width = LABEL_WIDTH.min(area.width);
    Rect::new(area.right().saturating_sub(width), area.y, width, 1)
}

#[allow(clippy::too_many_arguments)]
fn draw_ai_menu(
    frame: &mut ratatui::Frame,
    area: Rect,
    app: &App,
    panel: Color,
    foreground: Color,
    muted: Color,
    accent: Color,
    selected_background: Color,
) {
    let areas = Layout::vertical([Constraint::Min(8), Constraint::Length(8)]).split(area);
    let items = AI_TOOLS
        .iter()
        .enumerate()
        .map(|(index, tool)| {
            let selected = index == app.ai_menu_selected;
            let status = if tool.enabled() {
                "available"
            } else {
                "coming soon"
            };
            let marker = if selected { ">" } else { " " };
            ListItem::new(vec![
                Line::from(format!("{marker} {}", tool.name())),
                Line::from(format!("  {status}")),
            ])
            .style(if selected {
                Style::default()
                    .fg(if tool.enabled() { accent } else { muted })
                    .bg(selected_background)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(if tool.enabled() { foreground } else { muted })
                    .bg(panel)
            })
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" AI Mode [i] ")
                .title_style(Style::default().fg(accent).add_modifier(Modifier::BOLD))
                .border_style(Style::default().fg(accent))
                .style(Style::default().bg(panel)),
        ),
        areas[0],
    );

    let tool = AI_TOOLS[app.ai_menu_selected];
    frame.render_widget(
        Paragraph::new(format!(
            "{}\n\n↑/↓ select\nEnter run\ni/Esc close",
            tool.description()
        ))
        .wrap(Wrap { trim: false })
        .style(
            Style::default()
                .fg(if tool.enabled() { foreground } else { muted })
                .bg(panel),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(if tool.enabled() {
                    " Selected "
                } else {
                    " Disabled "
                })
                .border_style(Style::default().fg(muted))
                .style(Style::default().bg(panel)),
        ),
        areas[1],
    );
}

fn entry_display_lines(entry: &Entry, width: usize, expanded: bool) -> Vec<String> {
    let mut lines = wrap_text(&entry.text, width);
    if !expanded {
        return lines;
    }
    let Some(roadmap) = &entry.roadmap else {
        return lines;
    };
    lines.push("  Roadmap".into());
    for (index, step) in roadmap.steps.iter().enumerate() {
        lines.extend(wrap_indented(
            &step.title,
            width,
            &format!("  {}. ", index + 1),
            "     ",
        ));
        lines.extend(wrap_indented(&step.description, width, "     ", "     "));
    }
    lines
}

fn wrap_indented(text: &str, width: usize, first: &str, continuation: &str) -> Vec<String> {
    let available = width.saturating_sub(first.len()).max(1);
    wrap_text(text, available)
        .into_iter()
        .enumerate()
        .map(|(index, line)| format!("{}{}", if index == 0 { first } else { continuation }, line))
        .collect()
}

fn centered(area: Rect, width_percent: u16, height_percent: u16) -> Rect {
    let width = area.width.saturating_mul(width_percent).saturating_div(100);
    let height = area
        .height
        .saturating_mul(height_percent)
        .saturating_div(100);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width.max(3),
        height.max(3),
    )
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    text.split('\n')
        .flat_map(|line| wrap_line(line, width))
        .collect()
}

fn wrap_line(line: &str, width: usize) -> Vec<String> {
    if line.is_empty() {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;
    for character in line.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if !current.is_empty() && current_width + character_width > width {
            lines.push(current);
            current = String::new();
            current_width = 0;
        }
        current.push(character);
        current_width += character_width;
    }
    lines.push(current);
    lines
}

fn to_lines(lines: &[String]) -> Vec<Line<'_>> {
    lines.iter().map(|line| Line::from(line.as_str())).collect()
}

fn panel_heights(total_height: u16, selected_lines: usize, command_lines: usize) -> (u16, u16) {
    const BASE_LAYOUT_HEIGHT: u16 = 12;
    let available_extra = total_height.saturating_sub(BASE_LAYOUT_HEIGHT);
    let selected_demand = u16::try_from(selected_lines.saturating_sub(1)).unwrap_or(u16::MAX);
    let command_demand = u16::try_from(command_lines.saturating_sub(1)).unwrap_or(u16::MAX);

    let mut selected_extra = selected_demand.min(available_extra.div_ceil(2));
    let mut command_extra = command_demand.min(available_extra.saturating_sub(selected_extra));
    let mut remaining = available_extra.saturating_sub(selected_extra + command_extra);
    let selected_remaining = selected_demand.saturating_sub(selected_extra);
    let addition = selected_remaining.min(remaining);
    selected_extra += addition;
    remaining -= addition;
    command_extra += command_demand.saturating_sub(command_extra).min(remaining);

    (3 + selected_extra, 3 + command_extra)
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
    use ratatui::{backend::TestBackend, buffer::Buffer, Terminal};

    use crate::model::{Entry, Roadmap, RoadmapStep};

    use super::*;

    #[test]
    fn wraps_text_at_the_available_display_width() {
        assert_eq!(wrap_text("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap_text("áéíóú", 3), ["áéí", "óú"]);
        assert_eq!(wrap_text("first\nsecond", 20), ["first", "second"]);
    }

    #[test]
    fn long_content_expands_selected_and_command_panels() {
        let (selected, command) = panel_heights(30, 5, 4);
        assert!(selected > 3);
        assert!(command > 3);
        assert!(selected + command + 6 <= 30);
    }

    #[test]
    fn help_shortcuts_are_grouped_in_separate_rows() {
        let help = grouped_help("Saved.");
        let lines = help.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 8);
        assert!(lines[0].starts_with("NAVIGATION"));
        assert!(lines[1].starts_with("ENTRY"));
        assert!(lines[5].starts_with("AI/CMD"));
        assert_eq!(lines[7], "STATUS      Saved.");
    }

    #[test]
    fn help_stays_hidden_until_the_button_is_activated() {
        let notes = Notes::default();
        let indexes = Vec::new();
        let mut app = App::default();
        let backend = TestBackend::new(90, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| draw(frame, &notes, &mut app, &indexes))
            .unwrap();
        let closed = buffer_text(terminal.backend().buffer());
        assert!(closed.contains("[H]Help"));
        assert!(!closed.contains("Controls"));
        assert!(!closed.contains("AI: [i]"));
        assert!(!closed.contains("NAVIGATION"));

        app.help_open = true;
        terminal
            .draw(|frame| draw(frame, &notes, &mut app, &indexes))
            .unwrap();
        let opened = buffer_text(terminal.backend().buffer());
        assert!(opened.contains("NAVIGATION"));
        assert!(opened.contains("[H] or Esc close"));
    }

    #[test]
    fn help_button_is_placed_at_the_bottom_right() {
        assert_eq!(
            help_button_rect(Rect::new(2, 10, 21, 1)),
            Rect::new(16, 10, 7, 1)
        );
        assert_eq!(
            help_button_rect(Rect::new(2, 10, 5, 1)),
            Rect::new(2, 10, 5, 1)
        );
    }

    #[test]
    fn expanded_entries_show_indented_roadmap_steps() {
        let entry = Entry {
            id: crate::model::EntryId::new(None, Kind::Note, 1),
            kind: Kind::Note,
            horizon: None,
            text: "Learn Kubernetes".into(),
            roadmap: Some(Roadmap {
                steps: vec![RoadmapStep {
                    title: "Containers".into(),
                    description: "Understand container images.".into(),
                }],
            }),
        };
        assert!(!entry_display_lines(&entry, 40, false)
            .iter()
            .any(|line| line.contains("Containers")));
        let expanded = entry_display_lines(&entry, 40, true);
        assert!(expanded.iter().any(|line| line == "  1. Containers"));
        assert!(expanded
            .iter()
            .any(|line| line == "     Understand container images."));
    }

    #[test]
    fn active_ai_job_renders_a_visible_processing_dialog() {
        let notes = Notes::default();
        let indexes = Vec::new();
        let (_sender, receiver) = std::sync::mpsc::channel();
        let mut app = App {
            ai_receiver: Some(receiver),
            ai_started: Some(std::time::Instant::now()),
            ai_stage: Some("Generating test Roadmap"),
            ..App::default()
        };
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| draw(frame, &notes, &mut app, &indexes))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("AI processing"));
        assert!(text.contains("Processing in the background"));
        assert!(text.contains("Generating test Roadmap"));
    }

    #[test]
    fn ai_mode_renders_a_right_action_panel_with_disabled_suggestions() {
        let notes = Notes::default();
        let indexes = Vec::new();
        let mut app = App {
            ai_menu_open: true,
            ..App::default()
        };
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| draw(frame, &notes, &mut app, &indexes))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("AI Mode [i]"));
        assert!(text.contains("Generate Roadmap"));
        assert!(text.contains("Summarize"));
        assert!(text.contains("coming soon"));
        assert!(text.contains("NIT System"));
    }

    #[test]
    fn long_text_wraps_in_entries_selected_and_command_line() {
        let long_text = "X".repeat(120);
        let notes = Notes {
            entries: vec![Entry {
                id: crate::model::EntryId::new(None, Kind::Note, 1),
                kind: Kind::Note,
                horizon: None,
                text: long_text.clone(),
                roadmap: None,
            }],
        };
        let indexes = vec![0];
        let mut app = App {
            capture_input: Some(format!("w {long_text} -n")),
            ..App::default()
        };
        let backend = TestBackend::new(50, 30);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| draw(frame, &notes, &mut app, &indexes))
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert!(buffer_text(buffer).contains("N-0001"));
        let preview = format!("N-0001\n{long_text}");
        let command = format!(":w {long_text} -n");
        let preview_line_count = wrap_text(&preview, 48).len();
        let command_line_count = wrap_text(&command, 48).len();
        let (selected_height, command_height) =
            panel_heights(30, preview_line_count, command_line_count);
        let entry_end = 30 - selected_height - command_height;
        let selected_end = 30 - command_height;
        let entry_lines = rows_containing(buffer, 3, entry_end, 'X');
        let selected_lines = rows_containing(buffer, entry_end, selected_end, 'X');
        let command_lines = rows_containing(buffer, selected_end, 30, 'X');
        assert!(entry_lines >= 2, "entry did not wrap");
        assert!(selected_lines >= 2, "selected preview did not wrap");
        assert!(command_lines >= 2, "command input did not wrap");
        assert!(!buffer_text(buffer).contains("Class"));
    }

    #[test]
    fn scrolls_to_keep_the_selection_visible() {
        let notes = Notes {
            entries: (0..20)
                .map(|number| Entry {
                    id: crate::model::EntryId::new(None, Kind::Note, number + 1),
                    kind: Kind::Note,
                    horizon: None,
                    text: format!("Entry {number}"),
                    roadmap: None,
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

    fn rows_containing(buffer: &Buffer, start: u16, end: u16, character: char) -> usize {
        (start..end.min(buffer.area.height))
            .filter(|row| {
                (0..buffer.area.width)
                    .any(|column| buffer[(column, *row)].symbol().contains(character))
            })
            .count()
    }

    fn buffer_text(buffer: &Buffer) -> String {
        (0..buffer.area.height)
            .map(|row| {
                (0..buffer.area.width)
                    .map(|column| buffer[(column, row)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
