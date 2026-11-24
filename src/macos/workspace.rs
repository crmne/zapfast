//! NSWorkspace sleep, display, and session events while the shell has no window.

use std::sync::Mutex;

use objc2::rc::Retained;
use objc2::{AnyThread, DefinedClass, MainThreadMarker, define_class, msg_send, sel};
use objc2_app_kit::{
    NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceScreensDidSleepNotification,
    NSWorkspaceScreensDidWakeNotification, NSWorkspaceSessionDidBecomeActiveNotification,
    NSWorkspaceSessionDidResignActiveNotification, NSWorkspaceWillSleepNotification,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObject};

pub(crate) enum Event {
    Sleep,
    Wake,
    ScreenUnavailable(bool),
}

struct ObserverState {
    pending: Mutex<Pending>,
    waker: fastframe_shell::Waker,
}

#[derive(Default)]
struct Pending {
    events: Vec<Event>,
    screen_asleep: bool,
    sleeping: bool,
    inactive: bool,
}

define_class!(
    // SAFETY: the observer has no superclass requirements beyond NSObject.
    #[unsafe(super(NSObject))]
    #[name = "ZapFastWorkspaceObserver"]
    #[ivars = ObserverState]
    struct Observer;

    impl Observer {
        #[unsafe(method(workspaceChanged:))]
        fn workspace_changed(&self, notice: &NSNotification) {
            let state = self.ivars();
            let name = notice.name();
            let mut pending = state.pending.lock().unwrap_or_else(|p| p.into_inner());
            // SAFETY: AppKit's exported notification names are immutable.
            unsafe {
                if *name == *NSWorkspaceWillSleepNotification {
                    pending.sleeping = true;
                    pending.events.push(Event::Sleep);
                } else if *name == *NSWorkspaceDidWakeNotification {
                    pending.sleeping = false;
                    pending.events.push(Event::Wake);
                } else if *name == *NSWorkspaceScreensDidSleepNotification {
                    pending.screen_asleep = true;
                } else if *name == *NSWorkspaceScreensDidWakeNotification {
                    pending.screen_asleep = false;
                } else {
                    pending.inactive = *name == *NSWorkspaceSessionDidResignActiveNotification;
                }
            }
            let unavailable = pending.inactive || pending.screen_asleep || pending.sleeping;
            pending.events.push(Event::ScreenUnavailable(unavailable));
            drop(pending);
            state.waker.wake();
        }
    }
);

pub(crate) struct Workspace {
    center: Retained<NSNotificationCenter>,
    observer: Retained<Observer>,
}

impl Workspace {
    pub(crate) fn watch(waker: &fastframe_shell::Waker) -> Option<Self> {
        MainThreadMarker::new()?;
        let workspace = NSWorkspace::sharedWorkspace();
        let center = workspace.notificationCenter();
        let observer = Observer::alloc().set_ivars(ObserverState {
            pending: Mutex::default(),
            waker: waker.clone(),
        });
        // SAFETY: initialize the NSObject superclass and register only the
        // selector above, whose argument is an NSNotification.
        let observer: Retained<Observer> = unsafe { msg_send![super(observer), init] };
        unsafe {
            for name in [
                NSWorkspaceWillSleepNotification,
                NSWorkspaceDidWakeNotification,
                NSWorkspaceScreensDidSleepNotification,
                NSWorkspaceScreensDidWakeNotification,
                NSWorkspaceSessionDidBecomeActiveNotification,
                NSWorkspaceSessionDidResignActiveNotification,
            ] {
                center.addObserver_selector_name_object(
                    observer.as_ref(),
                    sel!(workspaceChanged:),
                    Some(name),
                    None,
                );
            }
        }
        Some(Self { center, observer })
    }

    pub(crate) fn events(&self) -> Vec<Event> {
        std::mem::take(
            &mut self
                .observer
                .ivars()
                .pending
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .events,
        )
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        // SAFETY: this observer was registered above and is still retained.
        unsafe { self.center.removeObserver(self.observer.as_ref()) };
    }
}
