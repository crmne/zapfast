//! Spoken announcements for screen readers: two hidden live-region nodes
//! take turns, so a repeated message is still a change worth reading.

use std::time::{Duration, Instant};

use egui::accesskit::{Live, Role};

/// How long an announcement waits to be spoken: as long as an info toast
/// stays on screen. eframe draws no frame for a minimized, covered or
/// hidden window, so nothing is spoken then; what is older than this when
/// a frame comes is stale and dropped. An error waits, as its toast stays
/// on screen until it is closed.
const FRESH: Duration = crate::app::INFO_TOAST_LIFETIME;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    /// A new message in the open chat.
    Message,
    /// Someone started typing or recording in the open chat.
    Typing,
    /// One of our messages was sent or could not be.
    Sent,
    /// Connection and history state.
    Status,
    /// A confirmation shown as a toast.
    Toast,
    /// Errors: read before anything else waiting.
    Error,
}

/// Whether a `category` item said at `at` is still worth speaking `now`.
fn fresh(category: Category, at: Instant, now: Instant) -> bool {
    category == Category::Error || now.saturating_duration_since(at) < FRESH
}

impl Category {
    /// Categories hidden while locked: messages and typing reveal who wrote
    /// or what, and toasts, errors included, are not drawn on the lock
    /// screen and can name a contact or an account.
    fn private(self) -> bool {
        matches!(
            self,
            Self::Message | Self::Typing | Self::Toast | Self::Error
        )
    }
}

#[derive(Default)]
pub struct Announcer {
    queue: Vec<(Category, String, Instant)>,
    /// Per slot: the text, whether it is assertive, whether it came from a
    /// private category.
    slots: [(String, bool, bool); 2],
    next: usize,
}

pub fn announcer_id(slot: usize) -> egui::Id {
    egui::Id::new(("announcer", slot))
}

impl Announcer {
    pub fn say(&mut self, category: Category, text: impl Into<String>) {
        let text = text.into();
        if !text.trim().is_empty() {
            let now = Instant::now();
            // While no frame speaks them, stale items must not pile up.
            self.queue
                .retain(|(category, _, at)| fresh(*category, *at, now));
            self.queue.push((category, text, now));
        }
    }

    /// Drops the private text waiting to be spoken and empties the slots
    /// that hold some, so it is no longer readable in the tree: behind the
    /// lock screen, or once another account is on screen.
    pub fn forget_private(&mut self) {
        self.queue.retain(|(category, _, _)| !category.private());
        for slot in &mut self.slots {
            if slot.2 {
                *slot = Default::default();
            }
        }
    }

    /// Emits both announcer nodes; call once per frame.
    pub fn show(&mut self, ctx: &egui::Context, private_allowed: bool) {
        let now = Instant::now();
        let items: Vec<_> = std::mem::take(&mut self.queue)
            .into_iter()
            .filter(|(category, _, at)| {
                fresh(*category, *at, now) && (private_allowed || !category.private())
            })
            .map(|(category, text, _)| (category, text))
            .collect();
        if !private_allowed {
            // Text spoken before the lock must not stay readable in the tree
            // behind the lock screen, nor come back after unlocking.
            self.forget_private();
        }
        if !items.is_empty() {
            // Errors first, each category in the order it was said.
            let mut items = items;
            items.sort_by_key(|(category, _)| *category != Category::Error);
            let private = items.iter().any(|(category, _)| category.private());
            let assertive = items
                .iter()
                .any(|(category, _)| *category == Category::Error);
            let mut text = items
                .into_iter()
                .map(|(_, text)| text)
                .collect::<Vec<_>>()
                .join(". ");
            let slot = self.next;
            self.next = 1 - self.next;
            // A screen reader only speaks a live region whose text changed,
            // so an identical text gets an invisible zero-width space.
            if self.slots[slot].0 == text {
                text.push('\u{200B}');
            }
            self.slots[slot] = (text, assertive, private);
        }
        let ppp = f64::from(ctx.pixels_per_point());
        for (slot, (text, assertive, _)) in self.slots.iter().enumerate() {
            ctx.accesskit_node_builder(announcer_id(slot), |node| {
                node.set_role(Role::Label);
                node.set_value(text.clone());
                node.set_live(if *assertive {
                    Live::Assertive
                } else {
                    Live::Polite
                });
                node.set_bounds(egui::accesskit::Rect {
                    x0: 0.0,
                    y0: 0.0,
                    x1: ppp,
                    y1: ppp,
                });
            });
        }
    }

    #[cfg(test)]
    pub fn spoken(&self) -> [&str; 2] {
        [&self.slots[0].0, &self.slots[1].0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(announcer: &mut Announcer, private: bool) -> egui::FullOutput {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            announcer.show(ui.ctx(), private)
        });
        // Dropping unapplied texture deltas would panic.
        output.textures_delta.clear();
        output
    }

    impl Announcer {
        /// Makes everything waiting look `by` older.
        pub(crate) fn age(&mut self, by: Duration) {
            for (_, _, at) in &mut self.queue {
                *at = at.checked_sub(by).unwrap_or(*at);
            }
        }
    }

    #[test]
    fn stale_announcements_are_dropped_but_errors_wait() {
        let mut announcer = Announcer::default();
        announcer.say(Category::Status, "Connection lost");
        announcer.say(Category::Toast, "Copied");
        announcer.say(Category::Error, "Could not send");
        announcer.age(FRESH + Duration::from_millis(1));
        announcer.say(Category::Status, "Back online");
        frame(&mut announcer, true);
        assert_eq!(announcer.spoken()[0], "Could not send. Back online");
    }

    #[test]
    fn stale_items_do_not_pile_up_while_no_frame_speaks() {
        let mut announcer = Announcer::default();
        for _ in 0..100 {
            announcer.say(Category::Toast, "Copied");
            announcer.age(FRESH);
        }
        announcer.say(Category::Toast, "Copied");
        assert_eq!(announcer.queue.len(), 1);
    }

    #[test]
    fn alternates_slots_and_repeats_identical_text() {
        let mut announcer = Announcer::default();
        announcer.say(Category::Status, "Copied");
        frame(&mut announcer, true);
        announcer.say(Category::Status, "Copied");
        frame(&mut announcer, true);
        announcer.say(Category::Status, "Copied");
        frame(&mut announcer, true);
        let [a, b] = announcer.spoken();
        assert_eq!(b, "Copied");
        assert_ne!(
            a, "Copied",
            "third repeat must differ from slot 0's old text"
        );
        assert_eq!(a.trim_end_matches('\u{200B}'), "Copied");
    }

    #[test]
    fn joins_a_frames_items_and_escalates_errors() {
        let mut announcer = Announcer::default();
        announcer.say(Category::Status, "Copied");
        announcer.say(Category::Error, "Could not send");
        let output = frame(&mut announcer, true);
        let tree = output.platform_output.accesskit_update.unwrap();
        let node = tree
            .nodes
            .iter()
            .find(|(id, _)| *id == announcer_id(0).accesskit_id())
            .map(|(_, node)| node)
            .unwrap();
        assert_eq!(node.value(), Some("Could not send. Copied"));
        assert_eq!(node.live(), Some(egui::accesskit::Live::Assertive));
    }

    #[test]
    fn locked_app_drops_private_announcements() {
        let mut announcer = Announcer::default();
        announcer.say(Category::Message, "Ada: secret");
        announcer.say(Category::Typing, "Ada is typing");
        announcer.say(Category::Toast, "Studio: Contact saved");
        announcer.say(Category::Error, "Could not send to Ada");
        announcer.say(Category::Status, "Connection lost");
        frame(&mut announcer, false);
        assert_eq!(announcer.spoken()[0], "Connection lost");
    }

    #[test]
    fn nothing_new_keeps_both_nodes_every_frame() {
        let mut announcer = Announcer::default();
        let output = frame(&mut announcer, true);
        let tree = output.platform_output.accesskit_update.unwrap();
        for slot in 0..2 {
            assert!(
                tree.nodes
                    .iter()
                    .any(|(id, _)| *id == announcer_id(slot).accesskit_id())
            );
        }
    }

    #[test]
    fn locking_clears_text_spoken_before_it() {
        let mut announcer = Announcer::default();
        announcer.say(Category::Message, "Ada: secret");
        frame(&mut announcer, true);
        assert_eq!(announcer.spoken()[0], "Ada: secret");
        let output = frame(&mut announcer, false);
        let tree = output.platform_output.accesskit_update.unwrap();
        assert!(
            tree.nodes
                .iter()
                .all(|(_, node)| node.value() != Some("Ada: secret"))
        );
        assert!(
            announcer
                .spoken()
                .iter()
                .all(|text| !text.contains("secret"))
        );
        // Unlocking does not bring it back.
        frame(&mut announcer, true);
        assert!(
            announcer
                .spoken()
                .iter()
                .all(|text| !text.contains("secret"))
        );
    }
}
