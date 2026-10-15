//! Phone linking with a QR code or pairing code.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Stroke, Vec2};

use crate::app::App;
use crate::backend::LinkStatus;
use crate::model::{Action, Dialog, PairingHistory};
use crate::qr::Qr;
use crate::theme::{self, Icon};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    card(app, ui, "login", "A native WhatsApp client.", body);
}

/// The centred card of the linking and lock screens: the logo, the name,
/// a line under it, and `body`.
pub(super) fn card(
    app: &mut App,
    ui: &mut egui::Ui,
    salt: &str,
    tagline: &str,
    body: impl FnOnce(&mut App, &mut egui::Ui),
) {
    let palette = app.palette;
    egui::CentralPanel::default()
        .frame(Frame::new().fill(palette.window))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            let top = theme::blend(palette.window, palette.accent, 0.10);
            super::widgets::paint_vertical_gradient(ui, rect, top, palette.window);
            let card_width = (460.0_f32.min(rect.width() - 24.0)).max(0.0);
            // Center the card using its previous height. Its content determines
            // the next frame's height.
            let height_id = ui.id().with(format!("{salt}-card-height"));
            let known_height = ui
                .ctx()
                .data(|data| data.get_temp::<f32>(height_id))
                .unwrap_or(560.0);
            let card_height = (known_height.min(rect.height() - 24.0)).max(0.0);
            let card =
                egui::Rect::from_center_size(rect.center(), Vec2::new(card_width, card_height));
            let mut card_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(card)
                    .layout(Layout::top_down(Align::Center)),
            );
            let frame = Frame::new()
                .fill(palette.panel)
                .stroke(Stroke::new(1.0, palette.outline))
                .corner_radius(CornerRadius::same(theme::RADIUS + 8))
                .inner_margin(Margin::same(32))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 16],
                    blur: 48,
                    spread: 0,
                    color: palette.shadow,
                });
            let chooser_content_height =
                (rect.height() - 24.0 - frame.total_margin().sum().y).max(0.0);
            let shown = frame.show(&mut card_ui, |ui| {
                ui.set_width((card_width - 64.0).max(0.0));
                if salt == "login" && app.link == LinkStatus::ChooseHistory {
                    // Use the actual panel below the title bar, including
                    // the frame's padding and stroke, rather than the card's
                    // cached height. Its content can then grow the card.
                    ui.set_max_height(chooser_content_height);
                }
                ui.spacing_mut().item_spacing.y = 8.0;
                let (logo, _) = ui.allocate_exact_size(Vec2::splat(64.0), egui::Sense::hover());
                theme::mark(ui, logo.center(), 64.0);
                ui.add_space(4.0);
                theme::text(ui, "ZapFast", theme::bold(28.0), palette.text);
                theme::text(ui, tagline, theme::regular(14.5), palette.secondary);
                ui.add_space(16.0);
                body(app, ui);
            });
            let height = shown.response.rect.height();
            if (height - known_height).abs() > 2.0 {
                ui.ctx()
                    .data_mut(|data| data.insert_temp(height_id, height.round()));
                ui.ctx().request_repaint();
            }
        });
}

fn body(app: &mut App, ui: &mut egui::Ui) {
    if app.link == LinkStatus::ChooseHistory {
        // Keep every chooser control reachable at the minimum window height.
        // The normal linking and lock cards retain their existing layout.
        egui::ScrollArea::vertical()
            .id_salt("pairing-history-scroll")
            .max_height(ui.available_height().max(24.0))
            .show(ui, |ui| body_content(app, ui));
    } else {
        body_content(app, ui);
    }
}

fn body_content(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    match app.link.clone() {
        LinkStatus::ChooseHistory => history_choice(app, ui),
        LinkStatus::Starting | LinkStatus::Connecting => {
            busy(ui, palette.accent, "Connecting to WhatsApp…");
        }
        LinkStatus::Connected | LinkStatus::Disconnected { .. } => {
            busy(ui, palette.accent, "Linked. Waiting for your chats…");
        }
        LinkStatus::LoggedOut => {
            theme::icon(ui, Icon::Smartphone, 28.0, palette.warning);
            theme::paragraph(
                ui,
                "This computer was unlinked from your phone. Requesting a new code.",
                theme::regular(14.0),
                palette.text,
            );
            ui.add_space(8.0);
            busy(ui, palette.accent, "Requesting a new code…");
        }
        LinkStatus::Failed(message) => {
            let key_lost = archive_key_lost(&message);
            theme::icon(ui, Icon::CircleAlert, 28.0, palette.danger);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(message)
                        .font(theme::regular(13.5))
                        .color(palette.danger),
                )
                .wrap(),
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if theme::pill_button(
                    ui,
                    &palette,
                    &crate::i18n::gettext(app.locale, "Try again"),
                    true,
                )
                .clicked()
                {
                    app.actions.push(Action::Reconnect);
                }
                if key_lost
                    && theme::pill_button(
                        ui,
                        &palette,
                        &crate::i18n::gettext(app.locale, "Start over…"),
                        false,
                    )
                    .clicked()
                {
                    app.actions
                        .push(Action::ShowDialog(crate::model::Dialog::ConfirmStartOver));
                }
            });
        }
        LinkStatus::Unlinked {
            qr,
            pair_code,
            pairing_phone,
        } => {
            if let Some(code) = pair_code {
                pair_code_view(app, ui, &code, pairing_phone.as_deref());
            } else if let Some(phone) = pairing_phone {
                busy(
                    ui,
                    palette.accent,
                    &format!("Requesting a code for +{phone}…"),
                );
            } else if let Some(qr) = qr {
                qr_view(app, ui, &qr);
            } else {
                busy(ui, palette.accent, "Waiting for a code from WhatsApp…");
            }
        }
    }
    super::accounts::login_choices(app, ui);
    ui.add_space(18.0);
    let disclaimer = theme::paragraph(
        ui,
        "Unofficial client. Using it may be against WhatsApp's terms of service.",
        theme::regular(11.5),
        palette.dim,
    );
    #[cfg(test)]
    ui.ctx().data_mut(|data| {
        data.insert_temp(egui::Id::new("pairing-history-disclaimer"), disclaimer.id);
    });
    #[cfg(not(test))]
    let _ = disclaimer;
}

fn history_choice(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    theme::paragraph(
        ui,
        crate::i18n::gettext(app.locale, "History to request"),
        theme::semibold(18.0),
        palette.text,
    );
    let mut selected = app.pairing_history;
    let mut full = selected.days().is_some();
    ui.horizontal(|ui| {
        ui.radio_value(&mut full, false, crate::i18n::gettext(app.locale, "Recent"));
        ui.radio_value(&mut full, true, crate::i18n::gettext(app.locale, "Full"));
    });
    if full {
        if selected == PairingHistory::Recent {
            selected = PairingHistory::FullOneYear;
        }
        let label = match selected {
            PairingHistory::FullTwoYears => crate::i18n::gettext(app.locale, "2 years"),
            PairingHistory::FullThreeYears => crate::i18n::gettext(app.locale, "3 years"),
            _ => crate::i18n::gettext(app.locale, "1 year"),
        };
        egui::ComboBox::from_id_salt("pairing-history-years")
            .selected_text(label)
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut selected,
                    PairingHistory::FullOneYear,
                    crate::i18n::gettext(app.locale, "1 year"),
                );
                ui.selectable_value(
                    &mut selected,
                    PairingHistory::FullTwoYears,
                    crate::i18n::gettext(app.locale, "2 years"),
                );
                ui.selectable_value(
                    &mut selected,
                    PairingHistory::FullThreeYears,
                    crate::i18n::gettext(app.locale, "3 years"),
                );
            });
        theme::paragraph(
            ui,
            crate::i18n::gettext(
                app.locale,
                "Ask for older history during linking. Your phone may supply less. Longer periods can take more time and disk space.",
            ),
            theme::regular(13.0),
            palette.secondary,
        );
    } else {
        selected = PairingHistory::Recent;
        theme::paragraph(
            ui,
            crate::i18n::gettext(
                app.locale,
                "Start with recent conversations for a quicker setup. Older messages can still be requested when you scroll up.",
            ),
            theme::regular(13.0),
            palette.secondary,
        );
    }
    if selected != app.pairing_history {
        app.actions.push(Action::ChoosePairingHistory {
            account: app.account().id.clone(),
            history: selected,
        });
    }
    ui.add_space(8.0);
    let response = theme::pill_button(
        ui,
        &palette,
        &crate::i18n::gettext(app.locale, "Continue to linking"),
        true,
    );
    #[cfg(test)]
    ui.ctx().data_mut(|data| {
        data.insert_temp(egui::Id::new("pairing-history-continue"), response.id);
    });
    if response.clicked() {
        app.actions
            .push(Action::StartPairing(app.account().id.clone()));
    }
}

pub(super) fn busy(ui: &mut egui::Ui, color: egui::Color32, label: &str) {
    ui.horizontal(|ui| {
        let width = 24.0
            + 8.0
            + ui.painter()
                .layout_no_wrap(label.to_owned(), theme::medium(14.0), color)
                .size()
                .x;
        ui.add_space((ui.available_width() - width).max(0.0) / 2.0);
        theme::spinner(ui, 18.0, color);
        theme::text(ui, label, theme::medium(14.0), ui.visuals().text_color());
    });
}

fn qr_view(app: &mut App, ui: &mut egui::Ui, code: &str) {
    let palette = app.palette;
    theme::text(
        ui,
        "Link this computer",
        theme::semibold(16.0),
        palette.text,
    );
    let side = 260.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(side), egui::Sense::hover());
    match Qr::encode(code) {
        Some(qr) => qr.paint(ui, rect, egui::Color32::BLACK, egui::Color32::WHITE),
        None => {
            ui.painter().rect_filled(rect, 8.0, palette.surface);
            theme::paint_icon(ui, Icon::CircleAlert, rect, 32.0, palette.danger);
        }
    }
    ui.add_space(4.0);
    let steps = [
        "Open WhatsApp on your phone",
        "Tap Menu or Settings, then Linked devices",
        "Tap Link a device and point the phone at this code",
    ];
    for (index, step) in steps.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            theme::text(
                ui,
                format!("{}.", index + 1),
                theme::semibold(13.0),
                palette.accent,
            );
            theme::text(ui, *step, theme::regular(13.0), palette.secondary);
        });
    }
    ui.add_space(10.0);
    if theme::link(
        ui,
        "Link with phone number instead",
        theme::medium(13.0),
        palette.link,
    )
    .clicked()
    {
        app.actions.push(Action::ShowDialog(Dialog::PairWithPhone));
    }
}

fn pair_code_view(app: &mut App, ui: &mut egui::Ui, code: &str, phone: Option<&str>) {
    let palette = app.palette;
    theme::text(
        ui,
        "Enter this code on your phone",
        theme::semibold(16.0),
        palette.text,
    );
    if let Some(phone) = phone {
        theme::text(
            ui,
            format!("for +{phone}"),
            theme::regular(13.0),
            palette.secondary,
        );
    }
    ui.add_space(8.0);
    let shown = if code.len() == 8 && !code.contains('-') {
        format!("{}-{}", &code[..4], &code[4..])
    } else {
        code.to_owned()
    };
    Frame::new()
        .fill(palette.surface)
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::symmetric(22, 12))
        .show(ui, |ui| {
            theme::text(ui, &shown, theme::bold(30.0), palette.text);
        });
    ui.add_space(8.0);
    let steps = [
        "Open WhatsApp on your phone",
        "Tap Menu or Settings, then Linked devices",
        "Tap Link a device, then Link with phone number instead",
    ];
    for (index, step) in steps.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            theme::text(
                ui,
                format!("{}.", index + 1),
                theme::semibold(13.0),
                palette.accent,
            );
            theme::text(ui, *step, theme::regular(13.0), palette.secondary);
        });
    }
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.add_space((ui.available_width() - 200.0).max(0.0) / 2.0);
        if theme::soft_button(ui, &palette, Some(Icon::Copy), "Copy code", false).clicked() {
            app.actions.push(Action::CopyText(code.to_owned()));
        }
    });
}

/// Whether the archive's key is gone for good, rather than the keyring being
/// locked or unavailable, which "Try again" can fix.
fn archive_key_lost(message: &str) -> bool {
    message.contains("OS keyring key is missing")
        || message.contains("archive key in the OS keyring is invalid")
        || message.contains("could not be unlocked with its OS keyring key")
}

#[cfg(test)]
mod start_over_tests {
    fn chooser_frame(
        app: &mut crate::app::App,
        ctx: &egui::Context,
        size: egui::Vec2,
        events: Vec<egui::Event>,
    ) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ui| app.frame_ui(ui),
        );
        output.textures_delta.clear();
    }

    #[test]
    fn pairing_chooser_stays_accessible_with_retained_archive_chats() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, _events) = crate::app::App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        );
        app.chats.push(crate::model::Chat::new(
            "fixture@s.whatsapp.net".into(),
            "Saved chat".into(),
        ));
        app.link = crate::backend::LinkStatus::ChooseHistory;
        let ctx = egui::Context::default();
        app.attach(&ctx);
        for _ in 0..4 {
            chooser_frame(&mut app, &ctx, egui::vec2(1180.0, 780.0), Vec::new());
        }
        assert!(
            ctx.data(|data| data.get_temp::<egui::Id>(egui::Id::new("pairing-history-continue")))
                .is_some(),
            "saved chats must not hide the pairing chooser"
        );
        assert!(!app.is_linked());
        assert!(!app.account().is_linked());
        assert_eq!(app.chats.len(), 1, "the local archive stays available");
    }

    #[test]
    fn history_chooser_keyboard_reveals_and_activates_continue_at_minimum_size() {
        use crate::backend::{Backend, Command, LinkStatus};
        use crate::model::PairingHistory;

        for size in [
            egui::vec2(400.0, 300.0),
            egui::vec2(600.0, 500.0),
            egui::vec2(1180.0, 780.0),
        ] {
            for history in [PairingHistory::Recent, PairingHistory::FullThreeYears] {
                let root = tempfile::tempdir().unwrap();
                let (mut app, _events) = crate::app::App::headless(
                    crate::paths::AppDirs::under(root.path()),
                    crate::settings::Settings::default(),
                );
                let (backend, mut commands) = Backend::recording();
                app.backend = backend;
                app.link = LinkStatus::ChooseHistory;
                app.pairing_history = history;
                let ctx = egui::Context::default();
                app.attach(&ctx);
                for _ in 0..4 {
                    chooser_frame(&mut app, &ctx, size, Vec::new());
                }
                if size.y == 780.0 {
                    let disclaimer = ctx
                        .data(|data| {
                            data.get_temp::<egui::Id>(egui::Id::new("pairing-history-disclaimer"))
                        })
                        .expect("the chooser draws its disclaimer");
                    let response = ctx.read_response(disclaimer).unwrap();
                    assert!(
                        response
                            .interact_rect
                            .contains_rect(response.rect.shrink(0.5)),
                        "the wide chooser shows the complete disclaimer: {history:?}, {response:?}"
                    );
                }
                let button = ctx
                    .data(|data| {
                        data.get_temp::<egui::Id>(egui::Id::new("pairing-history-continue"))
                    })
                    .expect("the chooser draws its Continue button");
                for _ in 0..12 {
                    chooser_frame(
                        &mut app,
                        &ctx,
                        size,
                        vec![egui::Event::Key {
                            key: egui::Key::Tab,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        }],
                    );
                    for _ in 0..3 {
                        chooser_frame(&mut app, &ctx, size, Vec::new());
                    }
                    if ctx.memory(|memory| memory.focused()) == Some(button) {
                        break;
                    }
                }
                assert_eq!(
                    ctx.memory(|memory| memory.focused()),
                    Some(button),
                    "Continue is reachable by Tab at {size:?} for {history:?}"
                );
                let response = ctx.read_response(button).unwrap();
                assert!(
                    response
                        .interact_rect
                        .contains_rect(response.rect.shrink(0.5)),
                    "keyboard focus reveals the whole button: {size:?}, {response:?}"
                );
                assert!(
                    egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(response.rect),
                    "Continue stays inside the window"
                );
                // The full app frame reports window presence before any input.
                // Account for that command without permitting pairing to start
                // merely because the chooser was displayed or focused.
                assert!(matches!(commands.try_recv(), Ok(Command::SetOnline(true))));
                assert!(
                    commands.try_recv().is_err(),
                    "displaying or focusing Continue must not start pairing: {size:?}, {history:?}"
                );
                assert_eq!(app.link, LinkStatus::ChooseHistory);
                chooser_frame(
                    &mut app,
                    &ctx,
                    size,
                    vec![egui::Event::Key {
                        key: egui::Key::Enter,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
                assert!(matches!(
                    commands.try_recv(),
                    Ok(Command::StartPairing(selected)) if selected == history
                ));
                assert_eq!(app.link, LinkStatus::Starting);
                assert!(commands.try_recv().is_err(), "Continue starts pairing once");
            }
        }
    }

    #[test]
    fn only_a_lost_key_offers_to_start_over() {
        for lost in [
            "The archive is encrypted but its OS keyring key is missing. Restore the original keyring; the archive has not been changed",
            "The archive key in the OS keyring is invalid",
            "The archive could not be unlocked with its OS keyring key: file is not a database",
        ] {
            assert!(super::archive_key_lost(lost), "{lost}");
        }
        for recoverable in [
            "Unlock your OS keyring and restart ZapFast",
            "The OS keyring could not open ZapFast's archive key",
        ] {
            assert!(!super::archive_key_lost(recoverable), "{recoverable}");
        }
    }
}
