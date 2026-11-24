//! Non-activating, accessible AppKit content for a floating notification.

use std::cell::Cell;
use std::sync::Arc;
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAnimatablePropertyContainer, NSAnimationContext, NSBackingStoreType, NSButton, NSColor,
    NSControl, NSEvent, NSFloatingWindowLevel, NSFont, NSFontWeightMedium, NSFontWeightSemibold,
    NSImage, NSImageScaling, NSImageView, NSLineBreakMode, NSPanel, NSTextField, NSTrackingArea,
    NSTrackingAreaOptions, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
    NSVisualEffectState, NSVisualEffectView, NSWindow, NSWindowCollectionBehavior,
    NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{NSData, NSPoint, NSRect, NSSize, NSString};

pub(super) const WIDTH: f64 = 352.0;
pub(super) const HEIGHT: f64 = 90.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Event {
    Open,
    Dismiss,
}

#[derive(Clone, Copy)]
struct Hover {
    inside: bool,
    elapsed: Duration,
    updated: Instant,
}

impl Hover {
    fn advance(&mut self, now: Instant) {
        if !self.inside {
            self.elapsed += now.saturating_duration_since(self.updated);
        }
        self.updated = now;
    }
}

pub(super) struct PanelState {
    wake: Arc<dyn Fn() + Send + Sync>,
    event: Cell<Option<Event>>,
    hover: Cell<Hover>,
    frame: Cell<NSRect>,
}

define_class!(
    // SAFETY: this NSPanel subclass can be used only from the main thread.
    #[unsafe(super(NSPanel, NSWindow, objc2_app_kit::NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = objc2::MainThreadOnly]
    #[name = "ZapFastFloatingNotificationPanel"]
    #[ivars = PanelState]
    pub(super) struct Panel;

    impl Panel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key(&self) -> bool {
            false
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main(&self) -> bool {
            false
        }

        #[unsafe(method(openAnnouncement:))]
        fn open_announcement(&self, _sender: &AnyObject) {
            self.ivars().event.set(Some(Event::Open));
            (self.ivars().wake)();
        }

        #[unsafe(method(dismissAnnouncement:))]
        fn dismiss_announcement(&self, _sender: &AnyObject) {
            self.ivars().event.set(Some(Event::Dismiss));
            (self.ivars().wake)();
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            self.hover_changed(true);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            self.hover_changed(false);
        }
    }
);

define_class!(
    // SAFETY: this NSButton subclass only opts into the first mouse click.
    #[unsafe(super(NSButton, NSControl, NSView, objc2_app_kit::NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = objc2::MainThreadOnly]
    #[name = "ZapFastFloatingNotificationButton"]
    struct BannerButton;

    impl BannerButton {
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: &NSEvent) -> bool {
            true
        }
    }
);

impl Panel {
    pub(super) fn new(
        main: MainThreadMarker,
        frame: NSRect,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Retained<Self> {
        let panel = main.alloc::<Self>().set_ivars(PanelState {
            wake,
            event: Cell::new(None),
            hover: Cell::new(Hover {
                inside: false,
                elapsed: Duration::ZERO,
                updated: Instant::now(),
            }),
            frame: Cell::new(frame),
        });
        let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
        // SAFETY: initialize NSPanel with its documented content-rect initializer.
        let panel: Retained<Self> = unsafe {
            msg_send![
                super(panel),
                initWithContentRect: frame,
                styleMask: style,
                backing: NSBackingStoreType::Buffered,
                defer: false,
            ]
        };
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(true);
        panel.setLevel(NSFloatingWindowLevel);
        panel.setHidesOnDeactivate(false);
        panel.setBecomesKeyOnlyIfNeeded(true);
        panel.setExcludedFromWindowsMenu(true);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::MoveToActiveSpace
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        panel
    }

    #[expect(clippy::too_many_arguments)]
    pub(super) fn populate(
        &self,
        main: MainThreadMarker,
        title: &str,
        body: &str,
        group: Option<&str>,
        avatar: Option<&[u8]>,
        can_open: bool,
        dismiss: &str,
    ) {
        let view = NSVisualEffectView::initWithFrame(main.alloc(), rect(0.0, 0.0, WIDTH, HEIGHT));
        view.setMaterial(NSVisualEffectMaterial::Popover);
        view.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        view.setState(NSVisualEffectState::Active);
        round(&view, 17.0);

        let image = avatar
            .and_then(|bytes| NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(bytes)));
        let icon = image.or_else(|| {
            let icon = include_bytes!("../../../packaging/macos/icon-1024.png");
            NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(icon))
        });
        let avatar = NSImageView::initWithFrame(main.alloc(), rect(14.0, 24.0, 42.0, 42.0));
        avatar.setImage(icon.as_deref());
        avatar.setImageScaling(NSImageScaling::ScaleAxesIndependently);
        round(&avatar, 21.0);
        view.addSubview(&avatar);

        let title_label = label(
            main,
            title,
            rect(
                66.0,
                57.0,
                if group.is_some() { 161.0 } else { 248.0 },
                20.0,
            ),
        );
        // SAFETY: these exported font weights are stable AppKit constants.
        title_label.setFont(Some(&NSFont::systemFontOfSize_weight(14.0, unsafe {
            NSFontWeightSemibold
        })));
        view.addSubview(&title_label);

        let body_label =
            NSTextField::wrappingLabelWithString(&NSString::from_str(&single_line(body)), main);
        body_label.setFrame(rect(66.0, 15.0, 246.0, 36.0));
        body_label.setFont(Some(&NSFont::systemFontOfSize(13.0)));
        body_label.setTextColor(Some(&NSColor::secondaryLabelColor()));
        body_label.setMaximumNumberOfLines(2);
        body_label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        view.addSubview(&body_label);

        if let Some(group) = group {
            let count = label(main, group, rect(233.0, 60.0, 79.0, 15.0));
            count.setFont(Some(&NSFont::systemFontOfSize_weight(10.0, unsafe {
                NSFontWeightMedium
            })));
            count.setTextColor(Some(&NSColor::secondaryLabelColor()));
            view.addSubview(&count);
        }

        if can_open {
            let description = format!("{title}. {}", single_line(body));
            let open = self.button(
                main,
                rect(0.0, 0.0, WIDTH, HEIGHT),
                &description,
                sel!(openAnnouncement:),
            );
            open.setTransparent(true);
            view.addSubview(&open);
        }

        let dismiss_text = NSString::from_str(dismiss);
        let close = self.button(
            main,
            rect(315.0, 53.0, 28.0, 28.0),
            "×",
            sel!(dismissAnnouncement:),
        );
        close.setBordered(false);
        close.setFont(Some(&NSFont::systemFontOfSize(19.0)));
        close.setToolTip(Some(&dismiss_text));
        // SAFETY: the standard accessibility selector retains or copies the label.
        let _: () = unsafe { msg_send![&*close, setAccessibilityLabel: &*dismiss_text] };
        view.addSubview(&close);

        // SAFETY: the panel is the tracking owner and retains the content
        // view. Mouse updates also arrive while a different app is active.
        let tracking = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                rect(0.0, 0.0, WIDTH, HEIGHT),
                NSTrackingAreaOptions::MouseEnteredAndExited | NSTrackingAreaOptions::ActiveAlways,
                Some(self.as_ref()),
                None,
            )
        };
        view.addTrackingArea(&tracking);
        self.setContentView(Some(&view));
    }

    fn button(
        &self,
        main: MainThreadMarker,
        frame: NSRect,
        title: &str,
        action: Sel,
    ) -> Retained<BannerButton> {
        let allocated = main.alloc::<BannerButton>().set_ivars(());
        // SAFETY: this initializes the NSButton superclass. This panel
        // implements both selectors handed to the button and owns its view.
        unsafe {
            let button: Retained<BannerButton> = msg_send![super(allocated), initWithFrame: frame];
            button.setTitle(&NSString::from_str(title));
            button.setTarget(Some(self.as_ref()));
            button.setAction(Some(action));
            button
        }
    }

    pub(super) fn move_to(&self, frame: NSRect) {
        if self.ivars().frame.replace(frame) != frame {
            self.setFrame_display(frame, true);
        }
    }

    pub(super) fn appear(&self, left: bool) {
        if NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion() {
            self.orderFrontRegardless();
            return;
        }
        let end = self.ivars().frame.get();
        let mut start = end;
        start.origin.x += if left { -10.0 } else { 10.0 };
        self.setAlphaValue(0.0);
        self.setFrame_display(start, false);
        self.orderFrontRegardless();
        let panel = self.retain();
        NSAnimationContext::runAnimationGroup(&RcBlock::new(
            move |context: std::ptr::NonNull<NSAnimationContext>| {
                // SAFETY: AppKit calls the block with a live animation context.
                unsafe { context.as_ref() }.setDuration(0.19);
                let animator = panel.animator();
                animator.setAlphaValue(1.0);
                animator.setFrame_display(end, true);
            },
        ));
    }

    pub(super) fn dismiss(&self) {
        self.setIgnoresMouseEvents(true);
        if NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion() {
            self.orderOut(None);
            return;
        }
        let panel = self.retain();
        let dismiss = self.retain();
        NSAnimationContext::runAnimationGroup_completionHandler(
            &RcBlock::new(move |context: std::ptr::NonNull<NSAnimationContext>| {
                // SAFETY: AppKit calls the block with a live animation context.
                unsafe { context.as_ref() }.setDuration(0.14);
                panel.animator().setAlphaValue(0.0);
            }),
            Some(&RcBlock::new(move || dismiss.orderOut(None))),
        );
    }

    pub(super) fn take_event(&self) -> Option<Event> {
        self.ivars().event.take()
    }

    fn hover_changed(&self, inside: bool) {
        let mut hover = self.ivars().hover.get();
        hover.advance(Instant::now());
        hover.inside = inside;
        self.ivars().hover.set(hover);
        (self.ivars().wake)();
    }

    pub(super) fn tick_hover(&self, now: Instant) -> (Duration, bool) {
        let mut hover = self.ivars().hover.get();
        hover.advance(now);
        let elapsed = std::mem::take(&mut hover.elapsed);
        self.ivars().hover.set(hover);
        (elapsed, hover.inside)
    }
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}

fn label(main: MainThreadMarker, text: &str, frame: NSRect) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(&single_line(text)), main);
    label.setFrame(frame);
    label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    label
}

fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn round(view: &NSView, radius: f64) {
    view.setWantsLayer(true);
    if let Some(layer) = view.layer() {
        layer.setCornerRadius(radius);
        layer.setMasksToBounds(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_counts_only_the_time_spent_outside_the_banner() {
        let start = Instant::now();
        let mut hover = Hover {
            inside: false,
            elapsed: Duration::ZERO,
            updated: start,
        };
        hover.advance(start + Duration::from_secs(2));
        hover.inside = true;
        hover.advance(start + Duration::from_secs(12));
        hover.inside = false;
        hover.advance(start + Duration::from_secs(15));
        assert_eq!(hover.elapsed, Duration::from_secs(5));
    }
}
