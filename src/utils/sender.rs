use std::path::PathBuf;

use crate::app::App;
use crate::model::{Action, ChatId, PollDraft, ReplyTarget};

/// Fluent sender utility for dispatching messages, media, reactions, and polls.
pub struct Sender<'a> {
    app: &'a mut App,
    target_chat: Option<ChatId>,
    reply_target: Option<ReplyTarget>,
}

impl<'a> Sender<'a> {
    pub fn new(app: &'a mut App) -> Self {
        let default_chat = app.account().open_chat.clone();
        Self {
            app,
            target_chat: default_chat,
            reply_target: None,
        }
    }

    pub fn to(mut self, chat: impl Into<ChatId>) -> Self {
        self.target_chat = Some(chat.into());
        self
    }

    pub fn reply_to(mut self, target: ReplyTarget) -> Self {
        self.reply_target = Some(target);
        self
    }

    fn resolve_chat(&self) -> Option<ChatId> {
        self.target_chat.clone().or_else(|| self.app.account().open_chat.clone())
    }

    pub fn text(&mut self, text: impl Into<String>) -> bool {
        let Some(chat) = self.resolve_chat() else {
            return false;
        };
        self.app.actions.push(Action::SendText {
            chat,
            text: text.into(),
            quoting: self.reply_target.take(),
        });
        true
    }

    pub fn files(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        let file_list: Vec<PathBuf> = paths.into_iter().collect();
        if file_list.is_empty() {
            return;
        }
        self.app.actions.push(Action::SendFiles(file_list));
    }

    pub fn file(&mut self, path: impl Into<PathBuf>) {
        self.files([path.into()]);
    }

    pub fn sticker(&mut self, path: impl Into<PathBuf>) {
        self.app.actions.push(Action::SendSticker(path.into()));
    }

    pub fn reaction(&mut self, message_id: impl Into<String>, emoji: impl Into<String>) -> bool {
        let Some(chat) = self.resolve_chat() else {
            return false;
        };
        self.app.actions.push(Action::React {
            chat,
            message: message_id.into(),
            emoji: emoji.into(),
        });
        true
    }

    pub fn location(&mut self, latitude: f64, longitude: f64) -> bool {
        let Some(chat) = self.resolve_chat() else {
            return false;
        };
        self.app.actions.push(Action::SendLocation {
            chat,
            latitude,
            longitude,
            quoting: self.reply_target.take(),
        });
        true
    }

    pub fn poll(
        &mut self,
        question: impl Into<String>,
        options: impl IntoIterator<Item = impl Into<String>>,
        multiple: bool,
    ) -> bool {
        let Some(chat) = self.resolve_chat() else {
            return false;
        };
        let poll_draft = PollDraft {
            question: question.into(),
            options: options.into_iter().map(Into::into).collect(),
            multiple,
        };
        self.app.actions.push(Action::CreatePoll {
            chat,
            draft: poll_draft,
        });
        true
    }

    pub fn delete_for_everyone(&mut self, message_id: impl Into<String>) -> bool {
        let Some(chat) = self.resolve_chat() else {
            return false;
        };
        self.app.actions.push(Action::DeleteForEveryone {
            chat,
            id: message_id.into(),
        });
        true
    }

    pub fn delete_for_me(&mut self, message_id: impl Into<String>) -> bool {
        let Some(chat) = self.resolve_chat() else {
            return false;
        };
        self.app.actions.push(Action::DeleteForMe {
            chat,
            id: message_id.into(),
        });
        true
    }
}
