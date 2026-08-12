use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthChar;

#[derive(Clone, Copy)]
pub struct MarkdownPalette {
    pub foreground: Color,
    pub muted: Color,
    pub blue: Color,
    pub cyan: Color,
    pub magenta: Color,
    pub yellow: Color,
    pub code_background: Color,
    pub match_background: Color,
}

#[derive(Clone)]
pub struct RenderedDocument {
    pub lines: Vec<Line<'static>>,
    pub match_lines: Vec<usize>,
}

pub fn render(
    source: &str,
    width: usize,
    query: &str,
    palette: MarkdownPalette,
) -> RenderedDocument {
    let logical = MarkdownBuilder::new(palette).parse(source);
    let mut lines = wrap_lines(logical, width.max(1));
    let query = query.trim().to_lowercase();
    let mut match_lines = Vec::new();
    if !query.is_empty() {
        for (index, line) in lines.iter_mut().enumerate() {
            let text = line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
                .to_lowercase();
            if text.contains(&query) {
                match_lines.push(index);
                for span in &mut line.spans {
                    span.style = span.style.bg(palette.match_background);
                }
            }
        }
    }
    RenderedDocument { lines, match_lines }
}

struct MarkdownBuilder {
    palette: MarkdownPalette,
    lines: Vec<Line<'static>>,
    spans: Vec<Span<'static>>,
    style: Style,
    styles: Vec<Style>,
    lists: Vec<Option<u64>>,
    quote_depth: usize,
    line_started: bool,
    link_destinations: Vec<String>,
}

impl MarkdownBuilder {
    fn new(palette: MarkdownPalette) -> Self {
        Self {
            palette,
            lines: Vec::new(),
            spans: Vec::new(),
            style: Style::default().fg(palette.foreground),
            styles: Vec::new(),
            lists: Vec::new(),
            quote_depth: 0,
            line_started: false,
            link_destinations: Vec::new(),
        }
    }

    fn parse(mut self, source: &str) -> Vec<Line<'static>> {
        let options = Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_TABLES
            | Options::ENABLE_FOOTNOTES;
        for event in Parser::new_ext(source, options) {
            self.event(event);
        }
        self.finish_line(false);
        while self.lines.last().is_some_and(|line| line.spans.is_empty()) {
            self.lines.pop();
        }
        self.lines
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text)
            | Event::Html(text)
            | Event::InlineHtml(text)
            | Event::InlineMath(text)
            | Event::DisplayMath(text) => self.push_text(&text),
            Event::Code(code) => self.push_styled(
                code.into_string(),
                self.style
                    .fg(self.palette.yellow)
                    .bg(self.palette.code_background),
            ),
            Event::SoftBreak | Event::HardBreak => self.finish_line(true),
            Event::Rule => {
                self.finish_line(false);
                self.push_styled("─".repeat(24), Style::default().fg(self.palette.muted));
                self.finish_line(true);
            }
            Event::TaskListMarker(done) => self.push_styled(
                if done { "[x] " } else { "[ ] " },
                Style::default().fg(self.palette.yellow),
            ),
            Event::FootnoteReference(reference) => self.push_styled(
                format!("[^{reference}]"),
                Style::default().fg(self.palette.blue),
            ),
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { level, .. } => {
                self.finish_line(false);
                self.push_style(heading_style(level, self.palette));
            }
            Tag::BlockQuote(_) => {
                self.quote_depth += 1;
                self.push_style(self.style.fg(self.palette.muted));
            }
            Tag::CodeBlock(_) => {
                self.finish_line(false);
                self.push_style(
                    self.style
                        .fg(self.palette.yellow)
                        .bg(self.palette.code_background),
                );
            }
            Tag::List(start) => self.lists.push(start),
            Tag::Item => {
                self.finish_line(false);
                let marker = match self.lists.last_mut() {
                    Some(Some(number)) => {
                        let marker = format!("{number}. ");
                        *number += 1;
                        marker
                    }
                    _ => "• ".to_owned(),
                };
                self.push_styled(marker, Style::default().fg(self.palette.yellow));
            }
            Tag::Emphasis => self.push_style(self.style.add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.push_style(self.style.add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self.push_style(self.style.add_modifier(Modifier::CROSSED_OUT)),
            Tag::Superscript | Tag::Subscript => {
                self.push_style(self.style.add_modifier(Modifier::ITALIC));
            }
            Tag::Link { dest_url, .. } => {
                self.link_destinations.push(dest_url.into_string());
                self.push_style(
                    self.style
                        .fg(self.palette.blue)
                        .add_modifier(Modifier::UNDERLINED),
                );
            }
            Tag::Image { dest_url, .. } => {
                self.link_destinations.push(dest_url.into_string());
                self.push_styled("[image: ", Style::default().fg(self.palette.magenta));
            }
            Tag::TableCell => {
                if self.line_started {
                    self.push_styled(" │ ", Style::default().fg(self.palette.muted));
                }
            }
            Tag::TableRow | Tag::TableHead => self.finish_line(false),
            Tag::FootnoteDefinition(label) => {
                self.finish_line(false);
                self.push_styled(
                    format!("[^{label}] "),
                    Style::default().fg(self.palette.blue),
                );
            }
            Tag::DefinitionListTitle => self.push_style(self.style.add_modifier(Modifier::BOLD)),
            Tag::DefinitionListDefinition => {
                self.finish_line(false);
                self.push_styled("  ", self.style);
            }
            Tag::Paragraph
            | Tag::HtmlBlock
            | Tag::DefinitionList
            | Tag::Table(_)
            | Tag::MetadataBlock(_) => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.finish_line(true);
                self.blank_line();
            }
            TagEnd::Heading(_) => {
                self.finish_line(true);
                self.pop_style();
                self.blank_line();
            }
            TagEnd::BlockQuote(_) => {
                self.finish_line(false);
                self.quote_depth = self.quote_depth.saturating_sub(1);
                self.pop_style();
                self.blank_line();
            }
            TagEnd::CodeBlock => {
                self.finish_line(false);
                self.pop_style();
                self.blank_line();
            }
            TagEnd::List(_) => {
                self.finish_line(false);
                self.lists.pop();
                self.blank_line();
            }
            TagEnd::Item => self.finish_line(false),
            TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::DefinitionListTitle => self.pop_style(),
            TagEnd::Link => {
                self.pop_style();
                if let Some(destination) = self.link_destinations.pop() {
                    self.push_styled(
                        format!(" ({destination})"),
                        Style::default().fg(self.palette.muted),
                    );
                }
            }
            TagEnd::Image => {
                if let Some(destination) = self.link_destinations.pop() {
                    self.push_styled(
                        format!("] ({destination})"),
                        Style::default().fg(self.palette.muted),
                    );
                }
            }
            TagEnd::TableRow | TagEnd::TableHead => self.finish_line(false),
            TagEnd::FootnoteDefinition | TagEnd::DefinitionListDefinition => self.finish_line(true),
            TagEnd::Table
            | TagEnd::TableCell
            | TagEnd::HtmlBlock
            | TagEnd::DefinitionList
            | TagEnd::MetadataBlock(_) => {}
        }
    }

    fn push_style(&mut self, style: Style) {
        self.styles.push(self.style);
        self.style = style;
    }

    fn pop_style(&mut self) {
        if let Some(style) = self.styles.pop() {
            self.style = style;
        }
    }

    fn push_text(&mut self, text: &str) {
        for (index, part) in text.split('\n').enumerate() {
            if index > 0 {
                self.finish_line(true);
            }
            if !part.is_empty() {
                self.push_styled(part.to_owned(), self.style);
            }
        }
    }

    fn push_styled(&mut self, value: impl Into<String>, style: Style) {
        self.ensure_prefix();
        self.spans.push(Span::styled(value.into(), style));
        self.line_started = true;
    }

    fn ensure_prefix(&mut self) {
        if self.line_started || self.quote_depth == 0 {
            return;
        }
        self.spans.push(Span::styled(
            "│ ".repeat(self.quote_depth),
            Style::default().fg(self.palette.magenta),
        ));
        self.line_started = true;
    }

    fn finish_line(&mut self, allow_empty: bool) {
        if self.line_started || allow_empty {
            self.lines.push(Line::from(std::mem::take(&mut self.spans)));
        }
        self.line_started = false;
    }

    fn blank_line(&mut self) {
        if self.lines.last().is_some_and(|line| !line.spans.is_empty()) {
            self.lines.push(Line::default());
        }
    }
}

fn heading_style(level: HeadingLevel, palette: MarkdownPalette) -> Style {
    let color = match level {
        HeadingLevel::H1 => palette.cyan,
        HeadingLevel::H2 => palette.blue,
        HeadingLevel::H3 => palette.magenta,
        HeadingLevel::H4 | HeadingLevel::H5 | HeadingLevel::H6 => palette.yellow,
    };
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

fn wrap_lines(lines: Vec<Line<'static>>, width: usize) -> Vec<Line<'static>> {
    let mut wrapped = Vec::new();
    for line in lines {
        if line.spans.is_empty() {
            wrapped.push(Line::default());
            continue;
        }
        let mut spans = Vec::new();
        let mut current_width = 0;
        for span in line.spans {
            for character in span.content.chars() {
                let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
                if current_width > 0 && current_width + character_width > width {
                    wrapped.push(Line::from(std::mem::take(&mut spans)));
                    current_width = 0;
                }
                push_character(&mut spans, character, span.style);
                current_width += character_width;
            }
        }
        wrapped.push(Line::from(spans));
    }
    wrapped
}

fn push_character(spans: &mut Vec<Span<'static>>, character: char, style: Style) {
    if let Some(last) = spans.last_mut().filter(|span| span.style == style) {
        last.content.to_mut().push(character);
    } else {
        spans.push(Span::styled(character.to_string(), style));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> MarkdownPalette {
        MarkdownPalette {
            foreground: Color::White,
            muted: Color::DarkGray,
            blue: Color::Blue,
            cyan: Color::Cyan,
            magenta: Color::Magenta,
            yellow: Color::Yellow,
            code_background: Color::Black,
            match_background: Color::Red,
        }
    }

    #[test]
    fn renders_and_wraps_common_markdown() {
        let rendered = render(
            "## Design\n\n- **Parse** input\n- Read `Markdown`",
            12,
            "",
            palette(),
        );
        let text = rendered
            .lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert!(text.iter().any(|line| line.contains("Design")));
        assert!(text.iter().any(|line| line.contains("Parse")));
        assert!(text.len() > 4);
        assert!(rendered
            .lines
            .iter()
            .flat_map(|line| &line.spans)
            .any(|span| { span.style.add_modifier.contains(Modifier::BOLD) }));
    }

    #[test]
    fn records_matching_visual_lines() {
        let rendered = render("alpha\nbeta alpha", 40, "ALPHA", palette());
        assert_eq!(rendered.match_lines.len(), 2);
    }
}
