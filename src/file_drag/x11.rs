//! An XDND source on its own connection, with an explicit pointer/keyboard grab.
use super::Failure;
use anyhow::{Result, ensure};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::{Event, xproto::*};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{CURRENT_TIME, NONE};

struct Atoms {
    aware: Atom,
    proxy: Atom,
    enter: Atom,
    position: Atom,
    status: Atom,
    leave: Atom,
    drop: Atom,
    finished: Atom,
    selection: Atom,
    copy: Atom,
    uri: Atom,
    targets: Atom,
}

impl Atoms {
    fn new(connection: &RustConnection) -> Result<Self> {
        let atom =
            |name: &[u8]| -> Result<_> { Ok(connection.intern_atom(false, name)?.reply()?.atom) };
        Ok(Self {
            aware: atom(b"XdndAware")?,
            proxy: atom(b"XdndProxy")?,
            enter: atom(b"XdndEnter")?,
            position: atom(b"XdndPosition")?,
            status: atom(b"XdndStatus")?,
            leave: atom(b"XdndLeave")?,
            drop: atom(b"XdndDrop")?,
            finished: atom(b"XdndFinished")?,
            selection: atom(b"XdndSelection")?,
            copy: atom(b"XdndActionCopy")?,
            uri: atom(b"text/uri-list")?,
            targets: atom(b"TARGETS")?,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Target {
    window: Window,
    proxy: Window,
    version: u32,
}

struct Session {
    connection: RustConnection,
    source: Window,
    root: Window,
    atoms: Atoms,
    uri: Vec<u8>,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.connection.ungrab_pointer(CURRENT_TIME);
        let _ = self.connection.ungrab_keyboard(CURRENT_TIME);
        let _ = self.connection.destroy_window(self.source);
        let _ = self.connection.flush();
    }
}

impl Session {
    fn open(display: Option<&str>, window: Window, path: &std::path::Path) -> Result<Self> {
        use std::os::unix::ffi::OsStrExt;
        const PATH: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
            .remove(b'/')
            .remove(b'-')
            .remove(b'.')
            .remove(b'_')
            .remove(b'~');
        ensure!(path.is_absolute(), "the dragged file path must be absolute");
        let uri = format!(
            "file://{}\r\n",
            percent_encoding::percent_encode(path.as_os_str().as_bytes(), PATH)
        )
        .into_bytes();
        let (connection, _) = x11rb::connect(display)?;
        let root = connection.get_geometry(window)?.reply()?.root;
        let atoms = Atoms::new(&connection)?;
        let source = connection.generate_id()?;
        connection
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                source,
                root,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
            )?
            .check()?;
        let session = Self {
            connection,
            source,
            root,
            atoms,
            uri,
        };
        session
            .connection
            .change_property32(
                PropMode::REPLACE,
                source,
                session.atoms.aware,
                AtomEnum::ATOM,
                &[5],
            )?
            .check()?;
        session
            .connection
            .set_selection_owner(source, session.atoms.selection, CURRENT_TIME)?
            .check()?;
        ensure!(
            session
                .connection
                .grab_pointer(
                    false,
                    root,
                    EventMask::POINTER_MOTION | EventMask::BUTTON_RELEASE,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                    NONE,
                    NONE,
                    CURRENT_TIME
                )?
                .reply()?
                .status
                == GrabStatus::SUCCESS,
            "the pointer is already grabbed"
        );
        ensure!(
            session
                .connection
                .grab_keyboard(false, root, CURRENT_TIME, GrabMode::ASYNC, GrabMode::ASYNC)?
                .reply()?
                .status
                == GrabStatus::SUCCESS,
            "the keyboard is already grabbed"
        );
        Ok(session)
    }

    fn property(&self, window: Window, property: Atom, type_: Atom) -> Option<u32> {
        self.connection
            .get_property(false, window, property, type_, 0, 1)
            .ok()?
            .reply()
            .ok()?
            .value32()?
            .next()
    }

    fn target(&self) -> Result<Option<(Target, u32)>> {
        let position = self.connection.query_pointer(self.root)?.reply()?;
        let packed = (u32::from(position.root_x as u16) << 16) | u32::from(position.root_y as u16);
        let mut window = position.child;
        if window == NONE {
            return Ok(None);
        }
        // Find the leaf under the pointer, then its nearest XDND-aware ancestor.
        for _ in 0..64 {
            let pointer = self.connection.query_pointer(window)?.reply()?;
            if pointer.child == NONE {
                break;
            }
            window = pointer.child;
        }
        for _ in 0..64 {
            if let Some(version) = self.property(window, self.atoms.aware, AtomEnum::ATOM.into()) {
                let proxy = self
                    .property(window, self.atoms.proxy, AtomEnum::WINDOW.into())
                    .filter(|proxy| {
                        self.property(*proxy, self.atoms.proxy, AtomEnum::WINDOW.into())
                            == Some(*proxy)
                    })
                    .unwrap_or(window);
                if version >= 3 {
                    return Ok(Some((
                        Target {
                            window,
                            proxy,
                            version: version.min(5),
                        },
                        packed,
                    )));
                }
            }
            let parent = self.connection.query_tree(window)?.reply()?.parent;
            if parent == NONE || parent == self.root {
                break;
            }
            window = parent;
        }
        Ok(None)
    }

    fn send(&self, target: Target, type_: Atom, data: [u32; 5]) -> Result<()> {
        let event = ClientMessageEvent::new(32, target.window, type_, data);
        self.connection
            .send_event(false, target.proxy, EventMask::NO_EVENT, event)?
            .check()?;
        self.connection.flush()?;
        Ok(())
    }

    fn selection(&self, request: SelectionRequestEvent) -> Result<()> {
        let property = if request.property == NONE {
            request.target
        } else {
            request.property
        };
        let property = if request.selection == self.atoms.selection
            && request.target == self.atoms.uri
        {
            self.connection
                .change_property8(
                    PropMode::REPLACE,
                    request.requestor,
                    property,
                    self.atoms.uri,
                    &self.uri,
                )?
                .check()?;
            property
        } else if request.selection == self.atoms.selection && request.target == self.atoms.targets
        {
            self.connection
                .change_property32(
                    PropMode::REPLACE,
                    request.requestor,
                    property,
                    AtomEnum::ATOM,
                    &[self.atoms.uri, self.atoms.targets],
                )?
                .check()?;
            property
        } else {
            NONE
        };
        self.connection
            .send_event(
                false,
                request.requestor,
                EventMask::NO_EVENT,
                SelectionNotifyEvent {
                    response_type: SELECTION_NOTIFY_EVENT,
                    sequence: 0,
                    time: request.time,
                    requestor: request.requestor,
                    selection: request.selection,
                    target: request.target,
                    property,
                },
            )?
            .check()?;
        self.connection.flush()?;
        Ok(())
    }

    fn run(&self, cancelled: &AtomicBool) -> Result<()> {
        let keyboard = self
            .connection
            .get_keyboard_mapping(
                self.connection.setup().min_keycode,
                self.connection.setup().max_keycode - self.connection.setup().min_keycode + 1,
            )?
            .reply()?;
        let escape: Vec<u8> = keyboard
            .keysyms
            .chunks(usize::from(keyboard.keysyms_per_keycode))
            .enumerate()
            .filter(|(_, symbols)| symbols.contains(&0xff1b))
            .map(|(index, _)| self.connection.setup().min_keycode + index as u8)
            .collect();
        let initial = self.target()?;
        let mut current = initial.map(|(target, _)| target);
        let mut accepted = false;
        let mut waiting_status = initial.is_some();
        let mut pending_position = None;
        if let Some((target, packed)) = initial {
            self.send(
                target,
                self.atoms.enter,
                [self.source, target.version << 24, self.atoms.uri, 0, 0],
            )?;
            self.send(
                target,
                self.atoms.position,
                [self.source, 0, packed, CURRENT_TIME, self.atoms.copy],
            )?;
        }
        let mut release: Option<(u32, Instant)> = None;
        let mut dropped: Option<Instant> = None;
        loop {
            if cancelled.load(Ordering::Acquire) {
                break;
            }
            if let Some(event) = self.connection.poll_for_event()? {
                match event {
                    Event::MotionNotify(event) if release.is_none() => {
                        let next = self.target()?;
                        let next_target = next.map(|(target, _)| target);
                        if next_target != current {
                            if let Some(old) = current {
                                self.send(old, self.atoms.leave, [self.source, 0, 0, 0, 0])?;
                            }
                            current = next_target;
                            accepted = false;
                            waiting_status = false;
                            pending_position = None;
                            if let Some(target) = current {
                                self.send(
                                    target,
                                    self.atoms.enter,
                                    [self.source, target.version << 24, self.atoms.uri, 0, 0],
                                )?;
                            }
                        }
                        if let Some((target, packed)) = next {
                            if waiting_status {
                                pending_position = Some((packed, event.time));
                            } else {
                                self.send(
                                    target,
                                    self.atoms.position,
                                    [self.source, 0, packed, event.time, self.atoms.copy],
                                )?;
                                waiting_status = true;
                            }
                        }
                    }
                    Event::ButtonRelease(event) if event.detail == 1 => {
                        release = Some((event.time, Instant::now()));
                    }
                    Event::KeyPress(event) if escape.contains(&event.detail) => break,
                    Event::SelectionRequest(event) => self.selection(event)?,
                    Event::SelectionClear(_) => break,
                    Event::ClientMessage(event) => {
                        let data = event.data.as_data32();
                        if current.is_some_and(|target| {
                            data[0] == target.window || data[0] == target.proxy
                        }) {
                            if event.type_ == self.atoms.status {
                                accepted = data[1] & 1 != 0 && data[4] == self.atoms.copy;
                                waiting_status = false;
                                if let Some((packed, time)) = pending_position.take()
                                    && let Some(target) = current
                                {
                                    self.send(
                                        target,
                                        self.atoms.position,
                                        [self.source, 0, packed, time, self.atoms.copy],
                                    )?;
                                    waiting_status = true;
                                }
                            } else if event.type_ == self.atoms.finished {
                                return Ok(());
                            }
                        }
                    }
                    _ => {}
                }
            } else {
                std::thread::sleep(Duration::from_millis(5));
            }
            if let Some((time, at)) = release
                && dropped.is_none()
                && (!waiting_status || at.elapsed() > Duration::from_secs(2))
            {
                if let Some(target) = current.filter(|_| accepted && !waiting_status) {
                    self.send(target, self.atoms.drop, [self.source, 0, time, 0, 0])?;
                    self.connection.ungrab_pointer(CURRENT_TIME)?.check()?;
                    self.connection.ungrab_keyboard(CURRENT_TIME)?.check()?;
                    self.connection.flush()?;
                    dropped = Some(Instant::now());
                } else {
                    break;
                }
            }
            if dropped.is_some_and(|at| at.elapsed() > Duration::from_secs(15)) {
                anyhow::bail!("the drop target did not finish the file drag");
            }
        }
        if dropped.is_none()
            && let Some(target) = current
        {
            self.send(target, self.atoms.leave, [self.source, 0, 0, 0, 0])?;
        }
        Ok(())
    }
}

pub(super) fn start(window: Window, path: PathBuf, ctx: egui::Context) -> Result<(), Failure> {
    let (ready, started) = mpsc::sync_channel(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let abort = cancelled.clone();
    std::thread::Builder::new()
        .name("attachment-drag".into())
        .spawn(move || {
            match Session::open(None, window, &path) {
                Ok(session) => {
                    if ready.send(Ok(())).is_ok()
                        && let Err(error) = session.run(&abort)
                    {
                        super::report_failure(&ctx, error);
                    }
                }
                Err(error) => {
                    let _ = ready.send(Err(Failure::Native(error.to_string())));
                }
            }
            ctx.request_repaint();
        })
        .map_err(|error| Failure::Native(error.to_string()))?;
    match started.recv_timeout(Duration::from_secs(2)) {
        Ok(result) => result,
        Err(error) => {
            cancelled.store(true, Ordering::Release);
            Err(Failure::Native(error.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run explicitly inside an isolated Xvfb server, never on a person's desktop.
    #[test]
    #[ignore = "requires ZAPFAST_XDND_TEST_DISPLAY pointing to an isolated Xvfb server"]
    fn native_copy_handshake_serves_an_encoded_file_uri() {
        let display = std::env::var("ZAPFAST_XDND_TEST_DISPLAY").unwrap();
        let (target, screen) = x11rb::connect(Some(&display)).unwrap();
        let root = target.setup().roots[screen].root;
        let window = target.generate_id().unwrap();
        target
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                window,
                root,
                100,
                100,
                300,
                300,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new(),
            )
            .unwrap()
            .check()
            .unwrap();
        let atoms = Atoms::new(&target).unwrap();
        target
            .change_property32(PropMode::REPLACE, window, atoms.aware, AtomEnum::ATOM, &[5])
            .unwrap()
            .check()
            .unwrap();
        target.map_window(window).unwrap().check().unwrap();
        target
            .warp_pointer(NONE, root, 0, 0, 0, 0, 150, 150)
            .unwrap()
            .check()
            .unwrap();
        let file = tempfile::Builder::new()
            .prefix("synthetic # ")
            .tempfile()
            .unwrap();
        let session = Session::open(Some(&display), root, file.path()).unwrap();
        let source = session.source;
        let uri = session.uri.clone();
        assert!(String::from_utf8_lossy(&uri).contains("%23%20"));
        let cancelled = Arc::new(AtomicBool::new(false));
        struct Cancel(Arc<AtomicBool>);
        impl Drop for Cancel {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let send = |type_, data| {
            target
                .send_event(
                    false,
                    source,
                    EventMask::NO_EVENT,
                    ClientMessageEvent::new(32, source, type_, data),
                )
                .unwrap()
                .check()
                .unwrap();
            target.flush().unwrap();
        };
        let next = || {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(event) = target.poll_for_event().unwrap() {
                    return event;
                }
                assert!(Instant::now() < deadline, "XDND handshake timed out");
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        std::thread::scope(|scope| {
            let abort = cancelled.clone();
            let running = scope.spawn(move || session.run(&abort));
            let _cancel = Cancel(cancelled);
            target
                .send_event(
                    false,
                    source,
                    EventMask::NO_EVENT,
                    MotionNotifyEvent {
                        response_type: MOTION_NOTIFY_EVENT,
                        detail: Motion::NORMAL,
                        sequence: 0,
                        time: CURRENT_TIME,
                        root,
                        event: source,
                        child: NONE,
                        root_x: 150,
                        root_y: 150,
                        event_x: 150,
                        event_y: 150,
                        state: KeyButMask::BUTTON1,
                        same_screen: true,
                    },
                )
                .unwrap()
                .check()
                .unwrap();
            target.flush().unwrap();
            loop {
                if let Event::ClientMessage(event) = next() {
                    if event.type_ == atoms.enter {
                        assert_eq!(event.data.as_data32()[2], atoms.uri);
                    } else if event.type_ == atoms.position {
                        assert_eq!(event.data.as_data32()[4], atoms.copy);
                        break;
                    }
                }
            }
            send(atoms.status, [window, 1, 0, 0, atoms.copy]);
            target
                .send_event(
                    false,
                    source,
                    EventMask::NO_EVENT,
                    ButtonReleaseEvent {
                        response_type: BUTTON_RELEASE_EVENT,
                        detail: 1,
                        sequence: 0,
                        time: CURRENT_TIME,
                        root,
                        event: source,
                        child: NONE,
                        root_x: 150,
                        root_y: 150,
                        event_x: 150,
                        event_y: 150,
                        state: KeyButMask::BUTTON1,
                        same_screen: true,
                    },
                )
                .unwrap()
                .check()
                .unwrap();
            target.flush().unwrap();
            loop {
                if let Event::ClientMessage(event) = next() {
                    if event.type_ == atoms.drop {
                        break;
                    }
                    if event.type_ == atoms.position {
                        assert_eq!(event.data.as_data32()[4], atoms.copy);
                        send(atoms.status, [window, 1, 0, 0, atoms.copy]);
                    }
                }
            }
            let property = target
                .intern_atom(false, b"SYNTHETIC_DROP")
                .unwrap()
                .reply()
                .unwrap()
                .atom;
            target
                .convert_selection(window, atoms.selection, atoms.uri, property, CURRENT_TIME)
                .unwrap()
                .check()
                .unwrap();
            target.flush().unwrap();
            loop {
                if let Event::SelectionNotify(event) = next() {
                    assert_eq!(event.property, property);
                    break;
                }
            }
            let received = target
                .get_property(false, window, property, atoms.uri, 0, 4096)
                .unwrap()
                .reply()
                .unwrap()
                .value;
            assert_eq!(received, uri);
            send(atoms.finished, [window, 1, atoms.copy, 0, 0]);
            running.join().unwrap().unwrap();
        });
        assert!(
            file.path().is_file(),
            "a drag must leave the saved file in place"
        );
        target.destroy_window(window).unwrap().check().unwrap();
    }
}
