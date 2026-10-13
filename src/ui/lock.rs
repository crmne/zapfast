//! The lock screen: everything the app lock hides is left undrawn, and the
//! window shows only this card.

use egui::{Align, Key, Layout, Modifiers};

use crate::app::App;
use crate::app_lock::Forgetting;
use crate::i18n::gettext;
use crate::model::Action;
use crate::theme;

/// The password field, which takes the keyboard as the screen appears.
pub const PASSWORD_ID: &str = "app-lock-password";

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    keys(app, ui.ctx());
    let tagline = gettext(app.locale, "ZapFast is locked");
    super::login::card(app, ui, "lock", &tagline, |app, ui| {
        match app.app_lock.forgetting {
            Forgetting::No => password(app, ui),
            Forgetting::Confirming => confirm_unlink(app, ui),
            Forgetting::Unlinking => {
                let label = gettext(app.locale, "Unlinking this computer…");
                super::login::busy(ui, app.palette.accent, &label);
            }
        }
    });
}

/// Only quitting and closing the window work while locked; every other
/// shortcut would reach what the lock hides.
fn keys(app: &mut App, ctx: &egui::Context) {
    let confirming = app.app_lock.forgetting == Forgetting::Confirming;
    ctx.input_mut(|input| {
        if input.consume_key(Modifiers::COMMAND, Key::Q) {
            app.actions.push(Action::Quit);
        }
        if input.consume_key(Modifiers::COMMAND, Key::W) {
            app.actions.push(Action::CloseWindow);
        }
        if confirming && input.consume_key(Modifiers::NONE, Key::Escape) {
            app.actions.push(Action::ForgotAppPassword(false));
        }
    });
}

fn password(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let locale = app.locale;
    theme::paragraph(
        ui,
        gettext(locale, "Enter your password to unlock."),
        theme::regular(13.5),
        palette.secondary,
    );
    let id = egui::Id::new(PASSWORD_ID);
    // TextEdit surrenders focus on Enter; take the key before drawing it.
    let mut submit = ui.memory(|memory| memory.has_focus(id))
        && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
    let checking = app.app_lock.checking();
    let field = ui.add_enabled(
        !checking,
        egui::TextEdit::singleline(&mut app.app_lock.entry)
            .id(id)
            .password(true)
            .hint_text(gettext(locale, "Password"))
            .desired_width(f32::INFINITY),
    );
    if ui.memory(|memory| memory.focused().is_none()) && !checking {
        field.request_focus();
    }
    if let Some(wait) = app.app_lock.wait_left() {
        let seconds = wait.as_secs_f32().ceil().max(1.0) as u64;
        let text = gettext(
            locale,
            "Too many wrong passwords. Try again in {seconds} s.",
        )
        .replace("{seconds}", &seconds.to_string());
        theme::paragraph(ui, text, theme::regular(13.0), palette.danger);
        ui.ctx()
            .request_repaint_after(wait.min(std::time::Duration::from_millis(250)));
    } else if app.app_lock.wrong {
        theme::paragraph(
            ui,
            gettext(locale, "Wrong password. Try again."),
            theme::regular(13.0),
            palette.danger,
        );
    }
    ui.add_space(4.0);
    let ready = app.app_lock.can_try();
    if checking {
        theme::spinner(ui, 20.0, palette.accent);
    } else {
        submit |= ui
            .add_enabled_ui(ready, |ui| {
                theme::pill_button(ui, &palette, &gettext(locale, "Unlock"), true)
            })
            .inner
            .clicked();
    }
    if submit && ready {
        app.actions.push(Action::UnlockApp);
    }
    ui.add_space(10.0);
    if theme::link(
        ui,
        gettext(locale, "Forgot password? Unlink this computer"),
        theme::regular(13.0),
        palette.accent,
    )
    .clicked()
    {
        app.actions.push(Action::ForgotAppPassword(true));
    }
}

fn confirm_unlink(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let locale = app.locale;
    theme::text(
        ui,
        gettext(locale, "Unlink this computer?"),
        theme::semibold(16.0),
        palette.text,
    );
    theme::paragraph(
        ui,
        gettext(
            locale,
            "Without the password, the only way back in is to unlink. This removes the device from WhatsApp, deletes the chats stored here, and turns the app lock off. You can link again with a new code.",
        ),
        theme::regular(13.5),
        palette.text,
    );
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if super::dialogs::danger_button(ui, app, &gettext(locale, "Unlink")).clicked() {
                app.actions.push(Action::UnlinkLockedApp);
            }
            if theme::pill_button(ui, &palette, &gettext(locale, "Cancel"), false).clicked() {
                app.actions.push(Action::ForgotAppPassword(false));
            }
        });
    });
}
