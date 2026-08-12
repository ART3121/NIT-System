use nit_core::EntryId;
use nitcat::ViewerState;

use std::ops::{Deref, DerefMut};

use super::BrowserContext;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NoteViewerState {
    pub(super) id: EntryId,
    core: ViewerState,
    pub(super) return_to_navigator: bool,
    pub(super) browser_context: BrowserContext,
}

impl NoteViewerState {
    pub(super) fn from_browser(
        id: EntryId,
        return_to_navigator: bool,
        browser_context: BrowserContext,
    ) -> Self {
        Self::new(id, return_to_navigator, browser_context)
    }

    fn new(id: EntryId, return_to_navigator: bool, browser_context: BrowserContext) -> Self {
        Self {
            id,
            core: ViewerState::new(),
            return_to_navigator,
            browser_context,
        }
    }
}

impl Deref for NoteViewerState {
    type Target = ViewerState;

    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

impl DerefMut for NoteViewerState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.core
    }
}
