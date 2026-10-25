use super::*;

struct Fixture {
    ctx: Context,
    controller: Autoscroll,
    enabled: bool,
    tall: bool,
    second: bool,
    frame_number: u64,
    frame_rate: f64,
    continuous: bool,
    /// Moves the first list's offset after drawing, as the message list does
    /// when rows above the viewport change height.
    shift: f32,
    areas: Vec<(Id, Rect)>,
    texts: Vec<Rect>,
    buttons: Vec<Rect>,
    inputs: Vec<Rect>,
    sliders: Vec<Rect>,
    popup: bool,
    text: String,
    value: f32,
    /// Whether any selectable text was dragged.
    dragged: bool,
    /// Whether the first list's control was clicked.
    clicked: bool,
    /// Whether the views saw an Escape key event in the latest frame.
    escaped: bool,
}

impl Fixture {
    fn new() -> Self {
        Self::at_rate(60.0)
    }

    fn at_rate(frame_rate: f64) -> Self {
        let mut fixture = Self {
            ctx: Context::default(),
            controller: Autoscroll::default(),
            enabled: true,
            tall: true,
            second: true,
            frame_number: 0,
            frame_rate,
            continuous: false,
            shift: 0.0,
            areas: Vec::new(),
            texts: Vec::new(),
            buttons: Vec::new(),
            inputs: Vec::new(),
            sliders: Vec::new(),
            popup: false,
            text: String::new(),
            value: 0.5,
            dragged: false,
            clicked: false,
            escaped: false,
        };
        fixture.frame(vec![], true);
        fixture.frame(vec![], true);
        fixture
    }

    fn frame(&mut self, events: Vec<egui::Event>, focused: bool) {
        let ctx = self.ctx.clone();
        self.frame_number += 1;
        self.areas.clear();
        self.texts.clear();
        self.buttons.clear();
        self.inputs.clear();
        self.sliders.clear();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(560.0, 340.0))),
                time: Some(self.frame_number as f64 / self.frame_rate),
                events,
                focused,
                ..Default::default()
            },
            |ui| {
                if self.continuous {
                    ctx.request_repaint();
                }
                self.controller.begin(&ctx, self.enabled);
                self.escaped = ctx.input(|input| {
                    input.events.iter().any(|event| {
                        matches!(
                            event,
                            egui::Event::Key {
                                key: egui::Key::Escape,
                                ..
                            }
                        )
                    })
                });
                ui.put(
                    Rect::from_min_size(egui::pos2(10.0, 275.0), egui::vec2(180.0, 35.0)),
                    egui::Button::new("Outside control"),
                );
                for index in 0..if self.second { 2 } else { 1 } {
                    let rect = Rect::from_min_size(
                        egui::pos2(10.0 + index as f32 * 280.0, 10.0),
                        egui::vec2(240.0, 220.0),
                    );
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .id_salt(index)
                            .max_rect(rect)
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );
                    let output = show(
                        &mut child,
                        ScrollArea::vertical()
                            .id_salt(index)
                            .max_height(220.0)
                            .animated(false),
                        |ui| {
                            let text = ui.add_sized(
                                [220.0, 36.0],
                                egui::Label::new("Selectable message text")
                                    .sense(egui::Sense::click_and_drag()),
                            );
                            primary_drag(ui, &text);
                            self.dragged |= text.dragged();
                            self.texts.push(text.rect);
                            let button = ui.add_sized(
                                [200.0, 30.0],
                                egui::Button::new("A control inside the list"),
                            );
                            self.clicked |= index == 0 && button.clicked();
                            self.buttons.push(button.rect);
                            let input = ui.add(
                                egui::TextEdit::singleline(&mut self.text).desired_width(200.0),
                            );
                            self.inputs.push(input.rect);
                            let slider = ui.add(egui::Slider::new(&mut self.value, 0.0..=1.0));
                            self.sliders.push(slider.rect);
                            if self.tall {
                                ui.allocate_space(egui::vec2(220.0, 700.0));
                            }
                        },
                    );
                    if index == 0 && self.shift != 0.0 {
                        let mut state = output.state;
                        state.offset.y += self.shift;
                        state.store(&ctx, output.id);
                    }
                    self.areas.push((output.id, output.inner_rect));
                }
                if self.popup {
                    egui::Area::new(Id::new("popup"))
                        .order(egui::Order::Foreground)
                        .fixed_pos(egui::pos2(20.0, 60.0))
                        .show(&ctx, |ui| {
                            ui.allocate_space(egui::vec2(150.0, 120.0));
                        });
                }
                self.controller.finish(&ctx, self.enabled);
            },
        );
        output.textures_delta.clear();
    }

    fn middle(&mut self, position: Pos2) {
        self.frame(
            vec![
                egui::Event::PointerMoved(position),
                button(position, egui::PointerButton::Middle, true),
            ],
            true,
        );
        self.frame(
            vec![button(position, egui::PointerButton::Middle, false)],
            true,
        );
    }

    fn offsets(&self) -> Vec<Vec2> {
        self.areas
            .iter()
            .map(|(id, _)| {
                egui::scroll_area::State::load(&self.ctx, *id)
                    .unwrap()
                    .offset
            })
            .collect()
    }
}

fn button(pos: Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn escape(pressed: bool, repeat: bool) -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed,
        repeat,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn scrolling_stays_with_the_starting_list_when_the_pointer_crosses_another_pane() {
    let mut f = Fixture::new();
    f.middle(f.texts[0].center());
    assert!(f.controller.active(), "the text must arm the gesture");
    assert!(f.controller.scrolls(f.areas[0].0));
    assert!(!f.controller.scrolls(f.areas[1].0));
    let across = egui::pos2(400.0, 200.0);
    for _ in 0..4 {
        f.frame(vec![egui::Event::PointerMoved(across)], true);
    }
    let offsets = f.offsets();
    assert!(offsets[0].y > 0.0);
    assert_eq!(
        offsets[1],
        Vec2::ZERO,
        "the pointer must not transfer scroll ownership"
    );
}

#[test]
fn a_press_anywhere_in_a_list_arms_except_on_text_fields_sliders_and_popups() {
    for target in 0..2 {
        let mut f = Fixture::new();
        let position = match target {
            0 => f.texts[0].center(),
            _ => f.buttons[0].center(),
        };
        f.middle(position);
        assert!(f.controller.active(), "text or button {target} arms");
    }
    for target in 0..4 {
        let mut f = Fixture::new();
        let position = match target {
            0 => f.inputs[0].center(),
            1 => f.sliders[0].center(),
            2 => egui::pos2(100.0, 290.0),
            _ => egui::pos2(500.0, 320.0),
        };
        f.middle(position);
        assert!(
            !f.controller.active(),
            "text field, slider or chrome {target} must not arm"
        );
        assert!(f.offsets().iter().all(|offset| *offset == Vec2::ZERO));
    }
    let mut f = Fixture::new();
    f.middle(egui::pos2(100.0, 180.0));
    assert!(
        f.controller.active(),
        "blank space inside an overflowing list is eligible"
    );
    let mut f = Fixture::new();
    f.popup = true;
    f.frame(vec![], true);
    f.frame(vec![], true);
    f.middle(egui::pos2(60.0, 100.0));
    assert!(
        !f.controller.active(),
        "a popup keeps the list beneath it still"
    );
    let mut f = Fixture::new();
    f.tall = false;
    f.frame(vec![], true);
    f.frame(vec![], true);
    f.middle(f.texts[0].center());
    assert!(
        !f.controller.active(),
        "a list that fits must not claim the gesture"
    );
}

#[test]
fn a_held_middle_button_never_drags_or_selects_the_text_under_it() {
    let mut f = Fixture::new();
    let anchor = f.texts[0].center();
    f.frame(
        vec![
            egui::Event::PointerMoved(anchor),
            button(anchor, egui::PointerButton::Middle, true),
        ],
        true,
    );
    for step in 1..=10 {
        f.frame(
            vec![egui::Event::PointerMoved(
                anchor + egui::vec2(0.0, 6.0 * step as f32),
            )],
            true,
        );
    }
    assert!(f.offsets()[0].y > 0.0, "the list scrolls");
    assert!(!f.dragged, "the text under the press does not drag");
    assert!(
        !f.ctx
            .plugin_opt::<egui::text_selection::LabelSelectionState>()
            .is_some_and(|selection| selection.lock().has_selection()),
        "nothing is selected"
    );
}

#[test]
fn focus_loss_disabling_and_disappearing_owner_cancel_without_more_movement() {
    for reason in 0..3 {
        let mut f = Fixture::new();
        f.middle(f.texts[1].center());
        f.frame(
            vec![egui::Event::PointerMoved(egui::pos2(400.0, 180.0))],
            true,
        );
        assert!(f.controller.active());
        if reason == 1 {
            f.enabled = false;
        }
        if reason == 2 {
            f.second = false;
        }
        f.frame(vec![], reason != 0);
        assert!(!f.controller.active());
        let stopped = f.offsets();
        for _ in 0..3 {
            f.frame(vec![], true);
        }
        assert_eq!(f.offsets(), stopped);
    }
}

#[test]
fn escape_clicks_and_wheel_stop_the_gesture_without_immediately_rearming_it() {
    for reason in 0..4 {
        let mut f = Fixture::new();
        let anchor = f.texts[0].center();
        f.middle(anchor);
        assert!(f.controller.active());
        let event = match reason {
            0 => escape(true, false),
            1 => button(anchor, egui::PointerButton::Middle, true),
            2 => button(anchor, egui::PointerButton::Primary, true),
            _ => egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -20.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            },
        };
        f.frame(vec![event], true);
        assert!(
            !f.controller.active(),
            "cancellation {reason} must not rearm in the same frame"
        );
    }
}

#[test]
fn the_click_that_stops_a_toggled_gesture_does_not_click_what_is_under_it() {
    let mut f = Fixture::new();
    let control = f.buttons[0].center();
    f.middle(f.texts[0].center());
    assert!(f.controller.active());
    f.frame(
        vec![
            egui::Event::PointerMoved(control),
            button(control, egui::PointerButton::Primary, true),
        ],
        true,
    );
    assert!(!f.controller.active(), "the press stops the gesture");
    f.frame(
        vec![button(control, egui::PointerButton::Primary, false)],
        true,
    );
    assert!(!f.clicked, "and does not click the control under it");
    assert_eq!(f.buttons[0].center(), control, "the list stood still");
    f.frame(
        vec![button(control, egui::PointerButton::Primary, true)],
        true,
    );
    f.frame(
        vec![button(control, egui::PointerButton::Primary, false)],
        true,
    );
    assert!(f.clicked, "the next click reaches it");
}

#[test]
fn the_escape_that_stops_the_gesture_reaches_nothing_behind_it() {
    let mut f = Fixture::new();
    f.middle(f.texts[0].center());
    assert!(f.controller.active());
    for (pressed, repeat) in [(true, false), (true, true), (false, false)] {
        f.frame(vec![escape(pressed, repeat)], true);
        assert!(!f.controller.active(), "Escape stops the gesture");
        assert!(!f.escaped, "the views never see that Escape");
    }
    f.frame(vec![escape(true, false)], true);
    assert!(f.escaped, "the next Escape reaches them");
    // A release lost to another window keeps no later Escape away.
    let mut f = Fixture::new();
    f.middle(f.texts[0].center());
    f.frame(vec![escape(true, false)], true);
    assert!(!f.controller.active() && !f.escaped);
    f.frame(vec![escape(true, false)], true);
    assert!(f.escaped, "a fresh Escape reaches the views");
}

#[test]
fn a_middle_click_and_small_pointer_movements_do_not_nudge_the_list() {
    let mut f = Fixture::new();
    let anchor = f.texts[0].center();
    f.middle(anchor);
    for y in [0.0, 5.0, 11.0] {
        f.frame(
            vec![egui::Event::PointerMoved(anchor + egui::vec2(50.0, y))],
            true,
        );
    }
    assert_eq!(f.offsets(), vec![Vec2::ZERO, Vec2::ZERO]);
    f.frame(
        vec![egui::Event::PointerMoved(anchor + egui::vec2(0.0, 40.0))],
        true,
    );
    assert!(f.offsets()[0].y > 0.0);
}

#[test]
fn each_step_starts_from_where_the_list_moved_itself() {
    let mut f = Fixture::new();
    let anchor = f.texts[0].center();
    f.middle(anchor);
    let below = anchor + egui::vec2(0.0, 40.0);
    for _ in 0..5 {
        f.frame(vec![egui::Event::PointerMoved(below)], true);
    }
    let before = f.offsets()[0].y;
    assert!(before > 0.0);
    f.shift = 100.0;
    f.frame(vec![egui::Event::PointerMoved(below)], true);
    f.shift = 0.0;
    let after = f.offsets()[0].y;
    let step = 0.04 * 40.0_f32.powf(2.2) / 60.0;
    assert!(
        (after - (before + 100.0 + step)).abs() < 0.5,
        "the list's own move is kept: {before} + 100 + {step} became {after}"
    );
}

#[test]
fn an_inner_list_owns_the_press_only_while_it_can_scroll() {
    for tall in [true, false] {
        let ctx = Context::default();
        let mut controller = Autoscroll::default();
        let mut frame = 0;
        let mut render = |events| {
            frame += 1;
            let mut ids = [Id::NULL; 2];
            let mut inner_rect = Rect::NOTHING;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(500.0, 340.0))),
                    events,
                    time: Some(frame as f64 / 60.0),
                    ..Default::default()
                },
                |ui| {
                    controller.begin(&ctx, true);
                    let outer = show(
                        ui,
                        ScrollArea::vertical().id_salt("outer").animated(false),
                        |ui| {
                            ui.add_space(20.0);
                            let inner = show(
                                ui,
                                ScrollArea::vertical()
                                    .id_salt("description")
                                    .max_height(120.0)
                                    .animated(false),
                                |ui| {
                                    ui.allocate_space(egui::vec2(
                                        200.0,
                                        if tall { 600.0 } else { 60.0 },
                                    ));
                                },
                            );
                            ids[0] = inner.id;
                            inner_rect = inner.inner_rect;
                            ui.allocate_space(egui::vec2(200.0, 900.0));
                        },
                    );
                    ids[1] = outer.id;
                    controller.finish(&ctx, true);
                },
            );
            output.textures_delta.clear();
            let offsets = ids.map(|id| egui::scroll_area::State::load(&ctx, id).unwrap().offset);
            (inner_rect.center(), controller.active(), offsets)
        };
        render(vec![]);
        let (anchor, _, _) = render(vec![]);
        let (_, active, _) = render(vec![
            egui::Event::PointerMoved(anchor),
            button(anchor, egui::PointerButton::Middle, true),
        ]);
        assert!(active);
        render(vec![button(anchor, egui::PointerButton::Middle, false)]);
        let (_, active, offsets) = render(vec![egui::Event::PointerMoved(
            anchor + egui::vec2(0.0, 60.0),
        )]);
        assert!(active);
        if tall {
            assert!(offsets[0].y > 0.0, "the inner list scrolls");
            assert_eq!(offsets[1], Vec2::ZERO, "the page stays");
        } else {
            assert_eq!(offsets[0], Vec2::ZERO, "the inner list fits");
            assert!(offsets[1].y > 0.0, "the page scrolls instead");
        }
    }
}

#[test]
fn a_list_of_rows_autoscrolls_like_any_other() {
    let ctx = Context::default();
    let mut controller = Autoscroll::default();
    let mut frame = 0;
    let mut render = |events| {
        frame += 1;
        let mut id = Id::NULL;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(300.0, 300.0))),
                events,
                time: Some(frame as f64 / 60.0),
                ..Default::default()
            },
            |ui| {
                controller.begin(&ctx, true);
                let output = show_rows(
                    ui,
                    ScrollArea::vertical().animated(false),
                    30.0,
                    200,
                    |ui, range| {
                        for row in range {
                            ui.add_sized([200.0, 30.0], egui::Button::new(format!("Chat {row}")));
                        }
                    },
                );
                id = output.id;
                controller.finish(&ctx, true);
            },
        );
        output.textures_delta.clear();
        (
            controller.active(),
            egui::scroll_area::State::load(&ctx, id).unwrap().offset.y,
        )
    };
    render(vec![]);
    render(vec![]);
    let anchor = egui::pos2(100.0, 100.0);
    let (active, _) = render(vec![
        egui::Event::PointerMoved(anchor),
        button(anchor, egui::PointerButton::Middle, true),
    ]);
    assert!(active);
    render(vec![button(anchor, egui::PointerButton::Middle, false)]);
    let mut offset = 0.0;
    for _ in 0..10 {
        offset = render(vec![egui::Event::PointerMoved(egui::pos2(100.0, 180.0))]).1;
    }
    assert!(offset > 0.0);
}

#[test]
fn losing_focus_while_drawing_is_suspended_cannot_resume_autoscroll_on_return() {
    let mut f = Fixture::new();
    f.middle(f.texts[0].center());
    f.frame(
        vec![egui::Event::PointerMoved(egui::pos2(100.0, 180.0))],
        true,
    );
    assert!(f.controller.active());
    let stopped = f.offsets();
    f.frame_number += 1;
    let mut output = f.ctx.run_ui(
        egui::RawInput {
            focused: false,
            time: Some(f.frame_number as f64 / 60.0),
            ..Default::default()
        },
        |ui| {
            assert!(f.controller.cancel_if_unfocused(ui.ctx()));
        },
    );
    output.textures_delta.clear();
    f.frame(vec![], true);
    assert!(!f.controller.active());
    assert_eq!(f.offsets(), stopped);
}

#[test]
fn extra_redraws_do_not_make_the_same_one_second_gesture_scroll_further() {
    let distance = |rate: u32| {
        let mut f = Fixture::at_rate(f64::from(rate));
        f.continuous = true;
        f.frame(vec![], true);
        let anchor = f.texts[0].center();
        f.middle(anchor);
        for _ in 0..rate {
            f.frame(
                vec![egui::Event::PointerMoved(anchor + egui::vec2(0.0, 42.0))],
                true,
            );
        }
        f.offsets()[0].y
    };
    let ordinary = distance(60);
    let frequent = distance(2000);
    assert!(ordinary > 0.0);
    assert!(
        (ordinary - frequent).abs() < 1.0,
        "equal time and pointer distance must give equal scrolling: {ordinary} vs {frequent}"
    );
}

#[test]
fn releasing_a_held_middle_button_after_moving_stops_but_a_plain_click_keeps_scrolling() {
    let mut f = Fixture::new();
    let anchor = f.texts[0].center();
    f.frame(
        vec![
            egui::Event::PointerMoved(anchor),
            button(anchor, egui::PointerButton::Middle, true),
        ],
        true,
    );
    assert!(f.controller.active());
    let below = anchor + egui::vec2(0.0, 60.0);
    for _ in 0..10 {
        f.frame(vec![egui::Event::PointerMoved(below)], true);
    }
    let held = f.offsets()[0];
    assert!(held.y > 0.0, "holding the button scrolls");
    f.frame(
        vec![button(below, egui::PointerButton::Middle, false)],
        true,
    );
    assert!(!f.controller.active(), "releasing after moving ends it");
    for _ in 0..5 {
        f.frame(vec![egui::Event::PointerMoved(below)], true);
    }
    assert_eq!(f.offsets()[0], held);

    // Released inside the dead zone, the press was a click: the list keeps
    // following the pointer until the next click.
    let mut f = Fixture::new();
    let anchor = f.texts[0].center();
    f.frame(
        vec![
            egui::Event::PointerMoved(anchor),
            button(anchor, egui::PointerButton::Middle, true),
        ],
        true,
    );
    f.frame(
        vec![
            egui::Event::PointerMoved(anchor + egui::vec2(0.0, 5.0)),
            button(anchor, egui::PointerButton::Middle, false),
        ],
        true,
    );
    assert!(f.controller.active());
    for _ in 0..10 {
        f.frame(
            vec![egui::Event::PointerMoved(anchor + egui::vec2(0.0, 60.0))],
            true,
        );
    }
    assert!(f.controller.active());
    assert!(f.offsets()[0].y > 0.0);
}

#[test]
fn speed_follows_chromium_and_the_time_that_actually_passed() {
    // Without an immediate repaint request egui reports a predicted frame
    // time, not the real one. Thirty frames a second must still scroll as far
    // per second as sixty.
    let distance = |rate: u32, pointer: f32| {
        let mut f = Fixture::at_rate(f64::from(rate));
        let anchor = f.texts[0].center();
        f.middle(anchor);
        let start = f.offsets()[0].y;
        for _ in 0..rate {
            f.frame(
                vec![egui::Event::PointerMoved(anchor + egui::vec2(0.0, pointer))],
                true,
            );
        }
        f.offsets()[0].y - start
    };
    // Chromium scrolls 0.04 * distance ^ 2.2 points a second.
    for pointer in [40.0_f32, 60.0] {
        let expected = 0.04 * pointer.powf(2.2);
        for rate in [30, 60] {
            let scrolled = distance(rate, pointer);
            assert!(
                (scrolled - expected).abs() < expected * 0.05,
                "{pointer} points away at {rate} Hz: {scrolled} instead of {expected}"
            );
        }
    }
    assert_eq!(distance(60, DEAD_ZONE), 0.0);
}

#[test]
fn the_cursor_matches_chromium() {
    use egui::CursorIcon::*;
    let both = egui::vec2(100.0, 100.0);
    let vertical = egui::vec2(0.0, 100.0);
    assert_eq!(cursor(Vec2::ZERO, both), AllScroll);
    assert_eq!(cursor(Vec2::ZERO, vertical), AllScroll);
    for (velocity, icon) in [
        (egui::vec2(0.0, -40.0), ResizeNorth),
        (egui::vec2(0.0, 40.0), ResizeSouth),
        (egui::vec2(40.0, 0.0), ResizeEast),
        (egui::vec2(-40.0, 0.0), ResizeWest),
        (egui::vec2(30.0, -30.0), ResizeNorthEast),
        (egui::vec2(-30.0, 30.0), ResizeSouthWest),
        (egui::vec2(30.0, 30.0), ResizeSouthEast),
        (egui::vec2(-30.0, -30.0), ResizeNorthWest),
    ] {
        assert_eq!(cursor(velocity, both), icon, "{velocity:?}");
    }
    // A list that only scrolls vertically shows only vertical arrows.
    assert_eq!(cursor(egui::vec2(30.0, -30.0), vertical), ResizeNorth);
    assert_eq!(cursor(egui::vec2(30.0, 0.0), vertical), AllScroll);
}
