//! Export confirmation and account-local progress, using the normal dialog controls.
use crate::app::App;
use crate::i18n::gettext;
use crate::model::{Action, Dialog};
use crate::theme;

/// Draws the resolved chat title and local-only export choices, or the current job's progress.
pub(super) fn show(app: &mut App, ui: &mut egui::Ui, chat: &str, mut include_media: bool) {
    let locale = app.locale;
    let palette = app.palette;
    theme::text(
        ui,
        gettext(locale, "Export chat").as_ref(),
        theme::medium(20.0),
        palette.text,
    );
    if let Some(chat) = app.chat(chat) {
        ui.label(app.chat_title(chat));
    }
    ui.label(gettext(locale, "Save all messages stored on this computer. Missing history and attachments will not be downloaded."));
    ui.label(gettext(
        locale,
        "Exported files are not encrypted. Locked chats cannot be exported.",
    ));
    ui.add_space(8.0);
    let busy = app.chat_export.is_some();
    if ui
        .add_enabled(
            !busy,
            egui::Checkbox::new(
                &mut include_media,
                gettext(locale, "Include downloaded attachments (ZIP)"),
            ),
        )
        .changed()
    {
        app.actions.push(Action::ShowDialog(Dialog::ExportChat {
            chat: chat.into(),
            include_media,
        }));
    }
    if let Some((_, counts)) = app.chat_export {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(gettext(locale, "Exporting…"));
        });
        ui.label(
            gettext(
                locale,
                "{messages} messages, {files} attachments; {missing} unavailable",
            )
            .replace("{messages}", &counts.messages.to_string())
            .replace("{files}", &counts.files.to_string())
            .replace("{missing}", &counts.missing.to_string()),
        );
    }
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if theme::pill_button(ui, &palette, &gettext(locale, "Cancel"), false).clicked() {
            app.actions.push(Action::CloseDialog);
        }
        if ui
            .add_enabled_ui(!busy, |ui| {
                theme::pill_button(ui, &palette, &gettext(locale, "Export…"), true)
            })
            .inner
            .clicked()
        {
            app.actions.push(Action::ExportChat {
                chat: chat.into(),
                include_media,
            });
        }
    });
}
