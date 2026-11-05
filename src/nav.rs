//! Browser-like history for the mouse's back and forward buttons.

use crate::model::{ChatId, Page};

/// A place in the interface the back and forward buttons return to: the page
/// and the chat open on it. Overlays are not places; Escape closes those, and
/// the buttons stay inert while one is open. Chats belong to the account that
/// owns them, so the history is cleared when the window switches accounts.
#[derive(Clone, Debug, PartialEq)]
pub struct Location {
    /// The page the window shows.
    pub page: Page,
    /// The chat open on that page, if any.
    pub chat: Option<ChatId>,
}

/// The places visited in this window and where the back and forward buttons
/// stand among them.
#[derive(Default)]
pub struct History {
    /// Places before the current one, oldest first.
    past: Vec<Location>,
    /// Places after the current one, the most recently left first.
    future: Vec<Location>,
    /// The place now showing, if one has been recorded.
    current: Option<Location>,
    /// The two most recently opened chats, oldest first. Unlike the browser
    /// stacks, visiting a page without a chat does not change these.
    recent_chats: Vec<ChatId>,
}

impl History {
    /// The most places kept; older ones fall off the back.
    const LIMIT: usize = 64;

    /// Whether the page and chat now showing are already the recorded place.
    pub fn is_current(&self, page: &Page, chat: Option<&str>) -> bool {
        self.current
            .as_ref()
            .is_some_and(|current| current.page == *page && current.chat.as_deref() == chat)
    }

    /// Records `location` as the place now showing. Visiting a new place drops
    /// the forward history, as a browser does.
    pub fn visit(&mut self, location: Location) {
        self.remember_chat(location.chat.as_deref());
        if self.current.as_ref() == Some(&location) {
            return;
        }
        let Some(previous) = self.current.replace(location) else {
            return;
        };
        self.past.push(previous);
        if self.past.len() > Self::LIMIT {
            self.past.remove(0);
        }
        self.future.clear();
    }

    /// Forgets every place that had `chat` open, for a chat that is gone.
    pub fn forget(&mut self, chat: &str) {
        self.recent_chats.retain(|id| id != chat);
        self.past
            .retain(|location| location.chat.as_deref() != Some(chat));
        self.future
            .retain(|location| location.chat.as_deref() != Some(chat));
        if self
            .current
            .as_ref()
            .is_some_and(|location| location.chat.as_deref() == Some(chat))
        {
            // The next recorded place takes its place without pushing.
            self.current = None;
        }
    }

    /// Steps back, returning the place to show, or `None` at the oldest.
    pub fn back(&mut self) -> Option<Location> {
        let target = self.past.pop()?;
        self.remember_chat(target.chat.as_deref());
        if let Some(current) = self.current.replace(target.clone()) {
            self.future.push(current);
        }
        Some(target)
    }

    /// Steps forward again, returning the place to show, or `None` at the
    /// newest.
    pub fn forward(&mut self) -> Option<Location> {
        let target = self.future.pop()?;
        self.remember_chat(target.chat.as_deref());
        if let Some(current) = self.current.replace(target.clone()) {
            self.past.push(current);
        }
        Some(target)
    }

    /// The most recently opened chat other than the one on screen.
    pub fn recent_chat(&self, open: Option<&str>) -> Option<&str> {
        self.recent_chats
            .iter()
            .rev()
            .find(|chat| Some(chat.as_str()) != open)
            .map(String::as_str)
    }

    fn remember_chat(&mut self, chat: Option<&str>) {
        let Some(chat) = chat else {
            return;
        };
        if self.recent_chats.last().is_some_and(|id| id == chat) {
            return;
        }
        self.recent_chats.retain(|id| id != chat);
        self.recent_chats.push(chat.to_owned());
        if self.recent_chats.len() > 2 {
            self.recent_chats.remove(0);
        }
    }

    /// Forgets every place, for another session or account.
    pub fn clear(&mut self) {
        self.past.clear();
        self.future.clear();
        self.current = None;
        self.recent_chats.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(id: &str) -> Location {
        Location {
            page: Page::Chats,
            chat: Some(id.into()),
        }
    }

    #[test]
    fn recent_chats_toggle_without_retracing_pages_or_older_chats() {
        let mut history = History::default();
        assert_eq!(history.recent_chat(None), None);
        history.visit(chat("a"));
        assert_eq!(history.recent_chat(Some("a")), None);
        history.visit(chat("b"));
        history.visit(chat("c"));
        history.visit(Location {
            page: Page::Settings,
            chat: Some("c".into()),
        });
        history.visit(Location {
            page: Page::Chats,
            chat: None,
        });
        assert_eq!(history.recent_chat(None), Some("c"));
        for (open, previous) in [("c", "b"), ("b", "c"), ("c", "b")] {
            history.visit(chat(open));
            assert_eq!(history.recent_chat(Some(open)), Some(previous));
        }
        assert_eq!(history.recent_chats.len(), 2);
    }

    #[test]
    fn recent_chats_follow_mouse_history_and_forget_removed_chats_and_accounts() {
        let mut history = History::default();
        for id in ["a", "b", "c"] {
            history.visit(chat(id));
        }
        assert_eq!(history.back(), Some(chat("b")));
        assert_eq!(history.recent_chat(Some("b")), Some("c"));
        assert_eq!(history.forward(), Some(chat("c")));
        assert_eq!(history.recent_chat(Some("c")), Some("b"));
        history.forget("b");
        assert_eq!(history.recent_chat(Some("c")), None);
        history.clear();
        assert_eq!(history.recent_chat(None), None);
    }
}
