//! A bounded, account-local queue for attachments the reader selected.

use std::collections::{HashSet, VecDeque};

use crate::model::{ChatId, Content, MediaState, Message};

/// Includes both waiting and running files. There is no persistent backlog.
pub(crate) const LIMIT: usize = 128;
const PARALLEL: usize = 2;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Target {
    pub chat: ChatId,
    pub message: String,
}

#[derive(Default)]
pub(crate) struct State {
    pending: VecDeque<Target>,
    active: HashSet<Target>,
}

impl State {
    /// Reject the whole addition if it does not fit, rather than silently
    /// downloading only part of the selection. Repeated selections share jobs.
    pub fn enqueue(&mut self, targets: Vec<Target>) -> Result<Vec<Target>, ()> {
        let mut seen: HashSet<_> = self.pending.iter().chain(&self.active).cloned().collect();
        let added: Vec<_> = targets
            .into_iter()
            .filter(|target| seen.insert(target.clone()))
            .collect();
        if self.pending.len() + self.active.len() + added.len() > LIMIT {
            return Err(());
        }
        self.pending.extend(added.iter().cloned());
        Ok(added)
    }

    pub fn next(&mut self) -> Option<Target> {
        if self.active.len() >= PARALLEL {
            return None;
        }
        let target = self.pending.pop_front()?;
        self.active.insert(target.clone());
        Some(target)
    }

    pub fn finish(&mut self, chat: &str, message: &str) {
        self.active.remove(&Target {
            chat: chat.into(),
            message: message.into(),
        });
    }

    pub fn cancel_pending(&mut self) -> impl Iterator<Item = Target> + '_ {
        self.pending.drain(..)
    }
}

fn missing(content: &Content) -> bool {
    crate::app::can_select(content)
        && content.media().is_some_and(|media| {
            media.path.is_none() && !matches!(media.state, MediaState::Downloading)
        })
}

/// Iterate in transcript order; never copy message bodies or thumbnails.
pub(crate) fn targets<'a>(
    chat: &'a str,
    messages: &'a [Message],
    selected: &'a [String],
) -> impl Iterator<Item = Target> + 'a {
    let selected: HashSet<_> = selected.iter().map(String::as_str).collect();
    messages
        .iter()
        .filter(move |message| selected.contains(message.id.as_str()) && missing(&message.content))
        .take(LIMIT + 1)
        .map(move |message| Target {
            chat: chat.into(),
            message: message.id.clone(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(id: usize) -> Target {
        Target {
            chat: "synthetic".into(),
            message: id.to_string(),
        }
    }

    #[test]
    fn overlapping_batches_share_jobs_and_capacity_includes_running_files() {
        let mut queue = State::default();
        assert_eq!(queue.enqueue(vec![target(0), target(0)]).unwrap().len(), 1);
        assert_eq!(queue.next(), Some(target(0)));
        assert_eq!(queue.enqueue(vec![target(0), target(1)]).unwrap().len(), 1);
        queue.enqueue((2..LIMIT).map(target).collect()).unwrap();
        assert!(
            queue
                .enqueue(vec![target(LIMIT), target(LIMIT + 1)])
                .is_err()
        );
        assert_eq!(queue.pending.len() + queue.active.len(), LIMIT);
        assert_eq!(queue.next(), Some(target(1)));
        assert_eq!(queue.next(), None);
        queue.finish("synthetic", "0");
        assert_eq!(queue.next(), Some(target(2)));
    }

    #[test]
    fn disconnect_cancels_waiting_work_but_keeps_running_slots_until_completion() {
        let mut queue = State::default();
        queue.enqueue((0..4).map(target).collect()).unwrap();
        queue.next();
        queue.next();
        assert_eq!(queue.cancel_pending().count(), 2);
        queue.enqueue(vec![target(4)]).unwrap();
        assert!(queue.next().is_none());
        queue.finish("synthetic", "0");
        assert_eq!(queue.next(), Some(target(4)));
    }
}
