//! A manual date-scoped request pass for the active account.

use crate::{app::App, i18n::gettext, model::Action, theme};

/// Draws date entry, account-local progress and the actions valid for that progress.
pub(super) fn show(ui: &mut egui::Ui, app: &mut App) {
    let locale = app.locale;
    let palette = app.palette;
    theme::text(
        ui,
        gettext(locale, "Import history since a date"),
        theme::medium(14.0),
        palette.text,
    );
    ui.label(gettext(locale, "Ask your phone for messages from a UTC date, up to three years ago. WhatsApp may return less. Files are not downloaded by this import."));
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        let label = ui.label(gettext(locale, "Start date (YYYY-MM-DD)"));
        let id = app.id.clone();
        let response = ui.add(
            egui::TextEdit::singleline(&mut app.history_import_date)
                .id_salt(("history-import-date", id))
                .desired_width(112.0)
                .hint_text("YYYY-MM-DD"),
        );
        response.labelled_by(label.id);
        let since =
            crate::model::parse_history_import_date(&app.history_import_date, crate::util::now());
        ui.add_enabled_ui(since.is_some(), |ui| {
            if theme::soft_button(ui, &palette, None, &gettext(locale, "Start import"), false)
                .clicked()
                && let Some(since) = since
            {
                app.actions.push(Action::StartHistoryImport(since));
            }
        });
    });
    if !app.history_import_date.is_empty()
        && crate::model::parse_history_import_date(&app.history_import_date, crate::util::now())
            .is_none()
    {
        ui.label(gettext(
            locale,
            "Use YYYY-MM-DD within the last 1,095 days (UTC).",
        ));
    }
    let progress = app.history_import.clone();
    if progress.failed {
        ui.label(if progress.since.is_some() {
            gettext(locale, "Import stopped. Progress could not be read or saved. Check disk space, then retry or start a new pass.")
        } else {
            gettext(locale, "Import stopped. Progress could not be read or saved. Check disk space, then start a new pass.")
        });
    }
    let Some(since) = progress.since else {
        return;
    };
    if let Ok(date) = jiff::Timestamp::from_second(since) {
        ui.label(gettext(locale, "Requested since {date} (UTC)").replace(
            "{date}",
            &date.to_zoned(jiff::tz::TimeZone::UTC).date().to_string(),
        ));
    }
    ui.label(
        gettext(
            locale,
            "Chats: {done} finished, {waiting} need retry, {excluded} excluded, {total} known.",
        )
        .replace("{done}", &progress.completed.to_string())
        .replace("{waiting}", &progress.stopped.to_string())
        .replace("{excluded}", &progress.excluded.to_string())
        .replace("{total}", &progress.total.to_string()),
    );
    ui.label(if progress.failed {
        gettext(locale, "Saved messages stay in your archive.")
    } else if progress.finished {
        gettext(
            locale,
            "Request pass finished. This does not guarantee complete history.",
        )
    } else if progress.paused {
        gettext(
            locale,
            "Import paused. Saved messages stay in your archive.",
        )
    } else if progress.total == 0 {
        gettext(locale, "Waiting for chats and a connection to your phone.")
    } else if progress.stopped > 0
        && progress.completed + progress.excluded + progress.stopped == progress.total
    {
        gettext(
            locale,
            "Some chats did not make progress. Retry unfinished chats when your phone is online.",
        )
    } else {
        gettext(
            locale,
            "Requests are paced. Keep your phone online; progress resumes after restarting.",
        )
    });
    ui.horizontal_wrapped(|ui| {
        if !progress.finished && !progress.failed {
            let text = if progress.paused {
                gettext(locale, "Resume import")
            } else {
                gettext(locale, "Pause import")
            };
            if theme::soft_button(ui, &palette, None, &text, false).clicked() {
                app.actions
                    .push(Action::PauseHistoryImport(!progress.paused));
            }
        }
        if (progress.stopped > 0 || progress.failed)
            && theme::soft_button(
                ui,
                &palette,
                None,
                &gettext(locale, "Retry unfinished chats"),
                false,
            )
            .clicked()
        {
            app.actions.push(Action::RetryHistoryImport);
        }
    });
}
