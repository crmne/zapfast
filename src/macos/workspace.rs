//! NSWorkspace sleep, display, and session events while the shell has no window.

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, define_class, msg_send, sel};
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
    events: RefCell<Vec<Event>>,
    waker: fastframe_shell::Waker,
    asleep: Cell<bool>,
    inactive: Cell<bool>,
}

define_class!(
    // SAFETY: the observer has no superclass requirements beyond NSObject.
    #[unsafe(super(NSObject))]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = ObserverState]
    struct Observer;

    impl Observer {
        #[unsafe(method(workspaceChanged:))]
        fn workspace_changed(&self, notice: &NSNotification) {
            let name = notice.name();
            // SAFETY: AppKit's exported notification names are immutable.
            unsafe {
                let event = if *name == *NSWorkspaceWillSleepNotification {
                    Event::Sleep
                } else if *name == *NSWorkspaceDidWakeNotification {
                    Event::Wake
                } else {
                    if *name == *NSWorkspaceScreensDidSleepNotification {
                        self.ivars().asleep.set(true);
                    } else if *name == *NSWorkspaceScreensDidWakeNotification {
                        self.ivars().asleep.set(false);
                    } else {
                        self.ivars().inactive.set(*name == *NSWorkspaceSessionDidResignActiveNotification);
                    }
                    Event::ScreenUnavailable(self.ivars().inactive.get() || self.ivars().asleep.get())
                };
                self.ivars().events.borrow_mut().push(event);
                self.ivars().waker.wake();
            }
        }
    }
);

pub(crate) struct Workspace {
    center: Retained<NSNotificationCenter>,
    observer: Retained<Observer>,
}

impl Workspace {
    pub(crate) fn watch(waker: &fastframe_shell::Waker) -> Option<Self> {
        let main = MainThreadMarker::new()?;
        let workspace = NSWorkspace::sharedWorkspace();
        let center = workspace.notificationCenter();
        let observer = main.alloc::<Observer>().set_ivars(ObserverState {
            events: Default::default(),
            waker: waker.clone(),
            asleep: Cell::new(false),
            inactive: Cell::new(false),
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
        std::mem::take(&mut *self.observer.ivars().events.borrow_mut())
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        // SAFETY: this observer was registered above and is still retained.
        unsafe { self.center.removeObserver(self.observer.as_ref()) };
    }
}
