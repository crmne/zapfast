use std::path::Path;

use crate::model::{Chat, ChatKind, Content, Message};
use crate::utils::phone;

/// Inspection extensions for incoming and existing WhatsApp messages.
pub trait MessageExt {
    fn plain_text(&self) -> Option<&str>;
    fn media_path(&self) -> Option<&Path>;
    fn is_from_me(&self) -> bool;
    fn is_media(&self) -> bool;
    fn is_voice(&self) -> bool;
    fn is_sticker(&self) -> bool;
    fn sender_phone(&self) -> Option<String>;
    fn has_reactions(&self) -> bool;
}

impl MessageExt for Message {
    fn plain_text(&self) -> Option<&str> {
        match &self.content {
            Content::Text { text, .. } | Content::Interactive { text, .. } => Some(text.as_str()),
            Content::Image { caption, .. }
            | Content::Video { caption, .. }
            | Content::Document { caption, .. } => caption.as_deref(),
            _ => None,
        }
    }

    fn media_path(&self) -> Option<&Path> {
        self.content.media().and_then(|media| media.path.as_deref())
    }

    #[inline]
    fn is_from_me(&self) -> bool {
        self.from_me
    }

    #[inline]
    fn is_media(&self) -> bool {
        self.content.media().is_some()
    }

    #[inline]
    fn is_voice(&self) -> bool {
        matches!(&self.content, Content::Audio { voice_note: true, .. })
    }

    #[inline]
    fn is_sticker(&self) -> bool {
        matches!(&self.content, Content::Sticker { .. })
    }

    fn sender_phone(&self) -> Option<String> {
        let (user, server) = self.sender.split_once('@')?;
        if server != "s.whatsapp.net" {
            return None;
        }
        Some(phone(user))
    }

    #[inline]
    fn has_reactions(&self) -> bool {
        !self.reactions.is_empty()
    }
}

/// Inspection extensions for conversations.
pub trait ChatExt {
    fn phone_number(&self) -> Option<String>;
    fn is_direct(&self) -> bool;
    fn is_group_chat(&self) -> bool;
    fn is_channel_chat(&self) -> bool;
}

impl ChatExt for Chat {
    fn phone_number(&self) -> Option<String> {
        if self.kind != ChatKind::Direct {
            return None;
        }
        let user = self.id.split('@').next()?;
        Some(phone(user))
    }

    #[inline]
    fn is_direct(&self) -> bool {
        self.kind == ChatKind::Direct
    }

    #[inline]
    fn is_group_chat(&self) -> bool {
        self.is_group()
    }

    #[inline]
    fn is_channel_chat(&self) -> bool {
        self.is_channel()
    }
}
