#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewerState {
    pub scroll: usize,
    pub search_query: String,
    pub search_input: Option<String>,
    pub match_lines: Vec<usize>,
    pub selected_match: usize,
    pub viewport_height: usize,
    pub total_lines: usize,
}

impl ViewerState {
    pub fn new() -> Self {
        Self {
            viewport_height: 1,
            ..Self::default()
        }
    }

    pub fn max_scroll(&self) -> usize {
        self.total_lines.saturating_sub(self.viewport_height)
    }

    pub fn scroll_by(&mut self, amount: isize) {
        self.scroll = if amount.is_negative() {
            self.scroll.saturating_sub(amount.unsigned_abs())
        } else {
            self.scroll
                .saturating_add(amount as usize)
                .min(self.max_scroll())
        };
    }

    pub fn jump_to_match(&mut self, forward: bool) {
        if self.match_lines.is_empty() {
            return;
        }
        self.selected_match = if forward {
            (self.selected_match + 1) % self.match_lines.len()
        } else if self.selected_match == 0 {
            self.match_lines.len() - 1
        } else {
            self.selected_match - 1
        };
        self.scroll = self.match_lines[self.selected_match].min(self.max_scroll());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrolling_and_match_navigation_are_bounded() {
        let mut viewer = ViewerState::new();
        viewer.total_lines = 20;
        viewer.viewport_height = 5;
        viewer.scroll_by(100);
        assert_eq!(viewer.scroll, 15);
        viewer.scroll_by(-100);
        assert_eq!(viewer.scroll, 0);
        viewer.match_lines = vec![3, 12];
        viewer.jump_to_match(true);
        assert_eq!((viewer.selected_match, viewer.scroll), (1, 12));
        viewer.jump_to_match(true);
        assert_eq!((viewer.selected_match, viewer.scroll), (0, 3));
        viewer.jump_to_match(false);
        assert_eq!((viewer.selected_match, viewer.scroll), (1, 12));
    }
}
