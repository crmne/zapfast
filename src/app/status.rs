//! Story navigation and playback, without reading the author's conversation.

use super::*;

impl App {
    pub(super) fn status_photo_tool(&mut self, tool: crate::photo::Tool) {
        if self.status_picture_busy() {
            return;
        }
        let Some(draft) = self.status_draft.as_ref().filter(|draft| {
            !draft.busy
                && draft
                    .media
                    .as_ref()
                    .is_some_and(|path| Pending::is_picture_file(path))
        }) else {
            return;
        };
        if draft.editor.is_none() {
            let target = self.next_attachment_id();
            self.status_draft
                .as_mut()
                .expect("a photo draft exists")
                .editor = Some(Box::new(crate::photo::Editor::new(target, tool)));
        } else if let Some(editor) = self
            .status_draft
            .as_mut()
            .and_then(|draft| draft.editor.as_mut())
        {
            editor.tool = tool;
        }
    }

    pub(super) fn preview_status_photo(&mut self) {
        let Some(draft) = self.status_draft.as_mut().filter(|draft| !draft.busy) else {
            return;
        };
        let Some(path) = draft.media.clone() else {
            return;
        };
        let Some(editor) = draft.editor.as_mut().filter(|editor| {
            !editor.applying
                && editor.requested.is_none()
                && editor.preview_revision != Some(editor.revision)
        }) else {
            return;
        };
        editor.requested = Some(editor.revision);
        let command = Command::RenderStatusPhoto {
            target: editor.target,
            revision: editor.revision,
            path,
            scene: editor.scene.clone(),
            apply: false,
        };
        self.backend.send(command);
    }

    pub(super) fn apply_status_photo(&mut self, crop: bool) {
        let Some(draft) = self.status_draft.as_mut().filter(|draft| !draft.busy) else {
            return;
        };
        let Some(path) = draft.media.clone() else {
            return;
        };
        let Some(editor) = draft
            .editor
            .as_mut()
            .filter(|editor| !editor.applying && editor.gesture.is_empty())
        else {
            return;
        };
        if editor.scene == crate::photo::Scene::default() {
            draft.editor = None;
            if crop {
                self.edit_status_picture();
            }
            return;
        }
        editor.applying = true;
        editor.crop_after = crop;
        draft.edit_target = Some(editor.target);
        let command = Command::RenderStatusPhoto {
            target: editor.target,
            revision: editor.revision,
            path,
            scene: editor.scene.clone(),
            apply: true,
        };
        self.backend.send(command);
    }

    /// Whether the shared picture editor is still preparing this Status photo.
    pub fn status_picture_busy(&self) -> bool {
        if self
            .status_draft
            .as_ref()
            .and_then(|draft| draft.editor.as_ref())
            .is_some_and(|editor| editor.applying)
        {
            return true;
        }
        self.status_draft
            .as_ref()
            .and_then(|draft| draft.edit_target)
            .is_some_and(|target| {
                self.picture_inspecting == Some(target)
                    || self.picture_applying.contains_key(&target)
            })
    }

    pub(super) fn edit_status_picture(&mut self) {
        if self.status_picture_busy() {
            return;
        }
        let Some(draft) = self.status_draft.as_ref().filter(|draft| !draft.busy) else {
            return;
        };
        let Some(path) = draft
            .media
            .clone()
            .filter(|path| Pending::is_picture_file(path))
        else {
            return;
        };
        let origin = draft.picture_origin.clone();
        let target = self.next_attachment_id();
        self.status_draft
            .as_mut()
            .expect("a photo draft exists")
            .edit_target = Some(target);
        if let Some(origin) = origin {
            self.picture_edit = Some(crate::model::PictureEdit {
                target,
                source: origin.source,
                width: origin.width,
                height: origin.height,
                crop: origin.crop,
                turns: origin.turns,
            });
        } else {
            self.picture_inspecting = Some(target);
            self.backend.send(Command::InspectPicture { target, path });
        }
    }

    pub(super) fn status_picture_history(&mut self, redo: bool) {
        if self.status_picture_busy() {
            return;
        }
        let Some(draft) = self.status_draft.as_mut().filter(|draft| !draft.busy) else {
            return;
        };
        let previous = if redo {
            draft.redo.pop()
        } else {
            draft.undo.pop()
        };
        if let Some(path) = previous {
            if let Some(current) = draft.media.replace(path) {
                if redo {
                    draft.undo.push(current);
                } else {
                    draft.redo.push(current);
                }
            }
            draft.picture_origin = None;
            draft.edit_target = None;
        }
    }

    pub(super) fn publish_status(&mut self) {
        if self.status_picture_busy() {
            return;
        }
        let Some(audience) = self.status_audience.clone() else {
            self.status_privacy_open = true;
            return;
        };
        let Some(draft) = self.status_draft.as_ref().filter(|draft| !draft.busy) else {
            return;
        };
        if draft.editor.is_some() {
            return;
        }
        if draft.text.trim().is_empty() && draft.media.is_none() {
            return;
        }
        let outgoing = draft.clone();
        if let Some(draft) = self.status_draft.as_mut() {
            draft.busy = true;
            draft.error = None;
        }
        self.backend.send(Command::PublishStatus {
            draft: outgoing,
            audience,
        });
    }

    /// Whether Status replies may be sent from this account. Like WhatsApp,
    /// replies and view receipts follow the account's read receipts: with
    /// them off, the reply bar goes and nothing is sent.
    pub(crate) fn status_replies_allowed(&self) -> bool {
        !self.account_receipts_off
    }

    pub(super) fn reply_status(&mut self, reaction: Option<String>) {
        if self.status_reply_busy || !self.status_replies_allowed() {
            return;
        }
        let Some((sender, id)) = self.status_selected.clone() else {
            return;
        };
        if !self
            .statuses
            .iter()
            .any(|item| item.sender == sender && item.id == id && self.can_view_status(item))
        {
            return;
        }
        let is_reaction = reaction.is_some();
        let text = reaction.unwrap_or_else(|| self.status_reply.trim().to_owned());
        if text.is_empty() {
            return;
        }
        self.status_reply_busy = true;
        self.backend.send(Command::ReplyStatus {
            sender,
            id,
            text,
            reaction: is_reaction,
        });
    }

    pub(super) fn reply_status_sticker(&mut self, path: PathBuf) {
        if self.status_reply_busy || !self.status_replies_allowed() {
            return;
        }
        let Some((sender, id)) = self.status_selected.clone() else {
            return;
        };
        if !self
            .statuses
            .iter()
            .any(|item| item.sender == sender && item.id == id && self.can_view_status(item))
        {
            return;
        }
        self.status_reply_busy = true;
        self.backend
            .send(Command::ReplyStatusSticker { sender, id, path });
    }

    pub(super) fn select_status(&mut self, sender: String, id: String) {
        self.status_step = 0;
        self.status_reply.clear();
        self.status_picker = None;
        self.status_viewers_open = false;
        self.status_delete_confirm = None;
        self.video.stop();
        self.player.stop();
        self.voice_chat = None;
        self.status_selected = self
            .statuses
            .iter()
            .any(|item| item.sender == sender && item.id == id && self.can_view_status(item))
            .then_some((sender.clone(), id.clone()));
        self.status_playback = self.status_selected.as_ref().map(|_| Default::default());
        if self.status_selected.is_some() {
            self.backend.send(Command::DownloadStatus { sender, id });
        }
    }

    pub(super) fn close_status(&mut self) {
        self.status_picker = None;
        self.status_viewers_open = false;
        self.status_delete_confirm = None;
        self.status_selected = None;
        self.status_playback = None;
        self.video.stop();
        self.player.stop();
    }

    pub(super) fn step_status(&mut self, direction: i8) {
        let next = self.status_selected.as_ref().and_then(|selected| {
            crate::status::step(
                self.statuses
                    .iter()
                    .filter(|item| self.can_view_status(item)),
                selected,
                direction,
            )
        });
        if let Some((sender, id)) = next {
            self.select_status(sender, id);
            self.status_step = direction.signum();
        } else if direction > 0 {
            self.close_status();
        }
    }

    pub(super) fn tick_status(&mut self, ctx: &egui::Context) {
        if self.app_lock.is_locked() {
            return;
        }
        // Pages opened without `Action::Open` (a chat, the list, the locked
        // folder, a logout) leave the Status page without closing it: a
        // voice Status must not keep playing behind them.
        if (self.page != Page::Status || self.status_selected.is_none())
            && self
                .player
                .current()
                .is_some_and(|key| key.starts_with("status:"))
        {
            self.player.stop();
        }
        if self.page != Page::Status {
            return;
        }
        let item = self
            .status_selected
            .as_ref()
            .and_then(|(sender, id)| {
                self.statuses.iter().find(|item| {
                    item.sender == *sender && item.id == *id && self.can_view_status(item)
                })
            })
            .cloned();
        let Some(item) = item else {
            if self.status_selected.is_some() {
                self.close_status();
            }
            return;
        };
        if self.status_playback.is_none() {
            self.status_playback = Some(Default::default());
        }
        let key = item.playback_id();
        let path = item.content.media().and_then(|media| media.path.clone());
        let ready = item.content.media().is_none() || path.is_some();
        let wants_play = self
            .status_playback
            .as_ref()
            .is_some_and(|clock| !clock.paused)
            && !self.window_hidden
            && self.window_focused
            && self.status_picker.is_none()
            && !self.status_viewers_open
            && self.status_delete_confirm.is_none()
            && self.status_delete_pending.is_none()
            && !ctx.text_edit_focused()
            && !egui::Popup::is_any_open(ctx)
            && !ctx.input(|input| input.pointer.primary_down());
        let started = self
            .status_playback
            .as_ref()
            .is_some_and(|clock| clock.started);
        if let Some(path) = &path {
            if matches!(item.content, Content::Video { .. }) {
                let current = self.video.status(&key);
                let state = current.as_ref().map(|status| status.state);
                let at_end = current.as_ref().is_some_and(|status| {
                    !status.total.is_zero() && status.position >= status.total
                });
                let toggle = !started && wants_play
                    || wants_play && !at_end && state == Some(crate::video::State::Paused)
                    || !wants_play && state == Some(crate::video::State::Playing);
                if toggle {
                    self.video.set_expanded(true);
                    self.video.toggle(&key, path);
                    self.video_chat = None;
                }
            } else if matches!(item.content, Content::Audio { .. }) {
                let state = self.player.status(&key).state;
                let toggle = !started && wants_play
                    || wants_play && state == crate::audio::State::Paused
                    || !wants_play && state == crate::audio::State::Playing;
                if toggle && let Err(error) = self.player.toggle(&key, path) {
                    self.close_status();
                    self.toast_error(error);
                    return;
                }
            }
        }
        let video = self.video.status(&key);
        let audio = matches!(item.content, Content::Audio { .. }).then(|| self.player.status(&key));
        if ready
            && !self.window_hidden
            && self.window_focused
            && self
                .account_mut()
                .status_seen
                .insert((item.sender.clone(), item.id.clone()))
        {
            self.backend.send(Command::MarkStatusSeen {
                sender: item.sender.clone(),
                id: item.id.clone(),
            });
        }
        let clock = self
            .status_playback
            .as_mut()
            .expect("a selected Status has a playback clock");
        if path.is_some() && wants_play {
            clock.started = true;
        }
        let mut running = ready && wants_play;
        if let Some(video) = video {
            if !video.total.is_zero() {
                clock.duration = video.total;
                clock.elapsed = video.position;
            }
            running &= video.state != crate::video::State::Loading;
        } else if started && matches!(item.content, Content::Video { .. }) {
            // The session that played it has ended without a last frame at
            // its length; the story is over all the same.
            clock.elapsed = clock.duration;
        }
        if let Some(audio) = audio {
            follow_audio(clock, &audio);
            running &= audio.state != crate::audio::State::Loading;
        }
        let timed =
            running && !matches!(item.content, Content::Video { .. } | Content::Audio { .. });
        let ended = clock.advance(Instant::now(), timed);
        if ended {
            self.actions.push(Action::StepStatus {
                account: self.account().id.clone(),
                direction: 1,
            });
        } else if running {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
    }
}

/// Moves a voice Status's clock with its clip. A finished clip reads as idle
/// at position zero, which ends the story rather than restarting it.
fn follow_audio(clock: &mut crate::status::Playback, audio: &crate::audio::Status) {
    if audio.state == crate::audio::State::Idle && clock.started && !clock.elapsed.is_zero() {
        clock.elapsed = clock.duration;
    } else if !audio.total.is_zero() {
        clock.duration = audio.total;
        clock.elapsed = audio.position;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finished_voice_status_ends_instead_of_rewinding() {
        let mut clock = crate::status::Playback::default();
        clock.started = true;
        let total = Duration::from_secs(4);
        follow_audio(
            &mut clock,
            &crate::audio::Status {
                state: crate::audio::State::Playing,
                position: Duration::from_secs(3),
                total,
            },
        );
        assert_eq!(clock.duration, total);
        assert_eq!(clock.elapsed, Duration::from_secs(3));
        // A clip that played to its end is idle at zero, with its length.
        follow_audio(
            &mut clock,
            &crate::audio::Status {
                state: crate::audio::State::Idle,
                position: Duration::ZERO,
                total,
            },
        );
        assert_eq!(clock.elapsed, total);
        assert!(clock.advance(Instant::now(), false), "the story ends");
    }
}
