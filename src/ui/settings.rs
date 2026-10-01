//! The settings page.

use std::borrow::Cow;

use egui::{Align, CornerRadius, Frame, Layout, Margin, Rect, Stroke, Vec2, pos2, vec2};

use crate::app::App;
use crate::i18n::Locale;
use crate::model::{Action, Dialog, Page};
use crate::privacy::{PrivacyChoice, PrivacyKind};
use crate::settings::{Settings, ThemeChoice, WallpaperColor};
use crate::theme::{self, Icon, Palette};
use crate::wallpaper;

use super::widgets;

/// The settings search field, which Ctrl+F focuses on this page.
pub const SEARCH_ID: &str = "settings-search";

/// A settings string with its English source, so the search finds a
/// translated setting by either wording.
#[derive(Clone, Debug, Default)]
struct Text {
    shown: Cow<'static, str>,
    source: Cow<'static, str>,
}

impl From<&'static str> for Text {
    fn from(text: &'static str) -> Self {
        Self {
            shown: text.into(),
            source: text.into(),
        }
    }
}

impl From<String> for Text {
    fn from(text: String) -> Self {
        Self {
            source: text.clone().into(),
            shown: text.into(),
        }
    }
}

/// Translates a settings string and keeps its English source for the search.
fn translated(locale: Locale, source: &'static str) -> Text {
    Text {
        shown: crate::i18n::gettext(locale, source),
        source: source.into(),
    }
}

/// The words a settings row is found by: its title and description, and for
/// rows that draw themselves, the labels of what they contain.
#[derive(Debug, Default)]
struct Row {
    title: Text,
    description: Text,
    keywords: Vec<Text>,
}

/// The settings search, folded like chat search so case and accents do not
/// matter.
struct Filter {
    needle: String,
}

impl Filter {
    fn new(query: &str) -> Self {
        Self {
            needle: crate::util::search_key(query.trim()),
        }
    }

    fn is_empty(&self) -> bool {
        self.needle.is_empty()
    }

    fn finds(&self, text: &Text) -> bool {
        crate::util::search_key(&text.shown).contains(&self.needle)
            || crate::util::search_key(&text.source).contains(&self.needle)
    }

    /// Whether a row shows. A match on the section title keeps the whole
    /// section, as a match on the row keeps the row.
    fn shows(&self, section: &Text, row: &Row) -> bool {
        self.is_empty()
            || self.finds(section)
            || self.finds(&row.title)
            || self.finds(&row.description)
            || row.keywords.iter().any(|keyword| self.finds(keyword))
    }
}

type Draw = Box<dyn FnOnce(&mut egui::Ui, &mut App)>;

enum Control {
    /// The control beside a titled row.
    Row(Draw),
    /// A switch bound to a setting.
    Toggle(fn(&mut Settings) -> &mut bool),
    /// Something that lays itself out, like the account card.
    Block(Draw),
}

/// A titled card of settings, drawn only when the search leaves a row in it.
struct Section {
    title: Text,
    entries: Vec<(Row, Control)>,
}

impl Section {
    fn new(title: Text) -> Self {
        Self {
            title,
            entries: Vec::new(),
        }
    }

    fn row(
        &mut self,
        title: impl Into<Text>,
        description: impl Into<Text>,
        control: impl FnOnce(&mut egui::Ui, &mut App) + 'static,
    ) {
        let row = Row {
            title: title.into(),
            description: description.into(),
            keywords: Vec::new(),
        };
        self.entries.push((row, Control::Row(Box::new(control))));
    }

    fn toggle(
        &mut self,
        title: impl Into<Text>,
        description: impl Into<Text>,
        field: fn(&mut Settings) -> &mut bool,
    ) {
        let row = Row {
            title: title.into(),
            description: description.into(),
            keywords: Vec::new(),
        };
        self.entries.push((row, Control::Toggle(field)));
    }

    fn block(&mut self, keywords: Vec<Text>, draw: impl FnOnce(&mut egui::Ui, &mut App) + 'static) {
        let row = Row {
            keywords,
            ..Row::default()
        };
        self.entries.push((row, Control::Block(Box::new(draw))));
    }

    /// The rows the search leaves, in order.
    fn visible(self, filter: &Filter) -> (Text, Vec<(Row, Control)>) {
        let entries = self
            .entries
            .into_iter()
            .filter(|(row, _)| filter.shows(&self.title, row))
            .collect();
        (self.title, entries)
    }

    /// Draws the section, returning whether anything in it matched.
    fn show(self, ui: &mut egui::Ui, app: &mut App, filter: &Filter) -> bool {
        let (title, entries) = self.visible(filter);
        if entries.is_empty() {
            return false;
        }
        let palette = app.palette;
        section(ui, &palette, &title.shown, |ui| {
            for (row, control) in entries {
                match control {
                    Control::Row(control) => widgets::setting_row(
                        ui,
                        &palette,
                        &row.title.shown,
                        &row.description.shown,
                        |ui| control(ui, app),
                    ),
                    Control::Toggle(field) => {
                        toggle(ui, app, &row.title.shown, &row.description.shown, field);
                    }
                    Control::Block(draw) => draw(ui, app),
                }
            }
        });
        true
    }
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    if theme::macos_chrome(ui.ctx()) {
        super::banner(app, ui);
    }
    let palette = app.palette;
    let locale = app.locale;
    egui::ScrollArea::vertical()
        .id_salt("settings")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // The column keeps a readable width and sits in the middle of a
            // wide window instead of leaving the space on its right empty.
            let full = ui.available_width();
            let width = (full - 2.0 * SIDE_MARGIN).clamp(0.0, COLUMN_WIDTH);
            let column = Rect::from_min_size(
                ui.cursor().min + vec2((full - width) / 2.0, TOP_MARGIN),
                vec2(width, ui.available_height()),
            );
            ui.ctx()
                .data_mut(|data| data.insert_temp(column_id(), column.x_range()));
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(column)
                    .layout(Layout::top_down(Align::Min)),
                |ui| {
                    ui.set_width(width);
                    ui.horizontal(|ui| {
                        if theme::icon_button(
                            ui,
                            Icon::ArrowLeft,
                            20.0,
                            palette.secondary,
                            palette.text,
                            "Back (Esc)",
                        )
                        .clicked()
                        {
                            app.actions.push(Action::Open(Page::Chats));
                        }
                        theme::text(
                            ui,
                            crate::i18n::gettext(locale, "Settings"),
                            theme::bold(24.0),
                            palette.text,
                        );
                    });
                    ui.add_space(14.0);
                    search(app, ui);
                    ui.add_space(6.0);
                    let filter = Filter::new(&app.settings_search);
                    let mut found = false;
                    for section in sections(app) {
                        found |= section.show(ui, app, &filter);
                    }
                    if !found {
                        ui.add_space(24.0);
                        theme::paragraph(
                            ui,
                            crate::i18n::gettext(locale, "No settings match “{query}”.")
                                .replace("{query}", app.settings_search.trim()),
                            theme::regular(14.0),
                            palette.secondary,
                        );
                    }
                },
            );
            ui.add_space(TOP_MARGIN);
        });
}

/// Widest the settings column grows.
const COLUMN_WIDTH: f32 = 640.0;
/// Least space beside the settings column.
const SIDE_MARGIN: f32 = 32.0;
/// Space above and below the settings column.
const TOP_MARGIN: f32 = 24.0;

/// Where the settings column was laid out, for layout tests.
pub fn column_id() -> egui::Id {
    egui::Id::new("settings-column")
}

/// The search field above the settings. Ctrl+F focuses it.
fn search(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let mut text = app.settings_search.clone();
    let response = widgets::search_field(
        ui,
        &palette,
        egui::Id::new(SEARCH_ID),
        &mut text,
        &crate::i18n::gettext(app.locale, "Search settings"),
        ui.available_width(),
    );
    if text != app.settings_search {
        app.actions.push(Action::SearchSettings(text));
    }
    if app.focus_settings_search {
        app.focus_settings_search = false;
        response.request_focus();
        response.scroll_to_me(Some(Align::Min));
    }
}

/// Every settings section, in page order, with the rows this setup offers.
fn sections(app: &App) -> Vec<Section> {
    let locale = app.locale;
    let palette = app.palette;

    let mut appearance = Section::new(translated(locale, "Appearance"));
    let status = app
        .custom_themes
        .status(app.settings.custom_theme.as_deref());
    let detail = if let Some(status) = status {
        theme::theme_status(status).to_owned().into()
    } else if app.custom_themes.follows_omarchy() {
        translated(locale, "Follow system uses your Omarchy colours.")
    } else {
        Text::default()
    };
    appearance.row(translated(locale, "Theme"), detail, theme_picker);
    appearance.row(
        translated(locale, "Font"),
        translated(
            locale,
            "System is your desktop's interface font. Inter looks the same on every computer.",
        ),
        font_picker,
    );
    appearance.row(
        translated(locale, "Wallpaper"),
        Text::default(),
        move |ui, app| {
            let label = if app.settings.wallpaper_image.is_some() {
                crate::i18n::gettext(app.locale, "Image").into_owned()
            } else {
                wallpaper_label(app.locale, app.settings.wallpaper_color_for(palette.dark))
            };
            if theme::soft_button(ui, &palette, Some(Icon::ChevronRight), &label, false).clicked() {
                app.actions.push(Action::Open(Page::Wallpaper));
            }
        },
    );
    appearance.row(
        translated(locale, "Zoom"),
        keyed(translated(
            locale,
            "Ctrl+Plus and Ctrl+Minus work anywhere, and Ctrl+0 resets it.",
        )),
        move |ui, app| {
            if theme::icon_button(
                ui,
                Icon::Plus,
                16.0,
                palette.secondary,
                palette.text,
                &crate::i18n::gettext(app.locale, "Larger"),
            )
            .clicked()
            {
                app.actions.push(Action::ZoomBy(0.1));
            }
            theme::text(
                ui,
                format!("{:.0}%", app.settings.zoom * 100.0),
                theme::medium(13.5),
                palette.text,
            );
            if theme::icon_button(
                ui,
                Icon::Minus,
                16.0,
                palette.secondary,
                palette.text,
                &crate::i18n::gettext(app.locale, "Smaller"),
            )
            .clicked()
            {
                app.actions.push(Action::ZoomBy(-0.1));
            }
        },
    );
    appearance.row(translated(locale, "Language"), "", language_picker);

    let mut chats = Section::new(translated(locale, "Chats"));
    chats.toggle(
        translated(locale, "Enter sends"),
        keyed(translated(locale, "When off, Ctrl+Enter sends.")),
        |settings| &mut settings.enter_sends,
    );
    chats.toggle(
        translated(locale, "Download files automatically"),
        translated(
            locale,
            "Files up to 64 MiB download as they come into view.",
        ),
        |settings| &mut settings.auto_download,
    );
    // macOS has no public API to pause other apps' media.
    if crate::media_pause::SUPPORTED {
        chats.toggle(
            translated(locale, "Pause other media while recording or playing"),
            translated(locale, "Music and videos in other apps resume afterwards."),
            |settings| &mut settings.pause_other_media,
        );
    }
    chats.row(
        translated(locale, "Locked chats code"),
        translated(
            locale,
            "Opens the Locked tab on this computer. It hides chats, it does not encrypt them.",
        ),
        move |ui, app| {
            // The buffer lives in egui memory: the hash is the only
            // stored form, so there is nothing to read it back from.
            let code_id = egui::Id::new("settings-chat-lock-code");
            let mut code: String = ui.data_mut(|data| data.get_temp(code_id).unwrap_or_default());
            let response = ui.add(
                egui::TextEdit::singleline(&mut code)
                    .font(theme::regular(13.0))
                    .text_color(palette.text)
                    .desired_width(220.0)
                    .hint_text(crate::i18n::gettext(app.locale, "Secret code"))
                    .password(true),
            );
            if response.changed() {
                let trimmed = code.trim().to_owned();
                app.actions.push(Action::SetChatLockCode(Some(trimmed)));
            }
            if app.settings.chat_lock_code_hash.is_some()
                && ui
                    .small_button(crate::i18n::gettext(app.locale, "Clear"))
                    .clicked()
            {
                code.clear();
                app.actions.push(Action::SetChatLockCode(None));
            }
            ui.data_mut(|data| data.insert_temp(code_id, code));
        },
    );

    let mut notifications = Section::new(translated(locale, "Notifications"));
    notifications.toggle(
        translated(locale, "Desktop notifications"),
        translated(
            locale,
            "For chats you are not looking at. Muted chats stay quiet.",
        ),
        |settings| &mut settings.notifications,
    );
    if app.settings.notifications {
        let (title, description) = sound_text(locale, false);
        notifications.row(title, description, |ui, app| sound_control(ui, app, false));
        notifications.toggle(
            translated(locale, "Play sounds for group messages"),
            translated(
                locale,
                "When off, only mentions and replies to you make a sound.",
            ),
            |settings| &mut settings.group_sounds,
        );
        let (title, description) = sound_text(locale, true);
        notifications.row(title, description, |ui, app| sound_control(ui, app, true));
    }

    let mut privacy = Section::new(translated(locale, "Privacy"));
    let receipts_note = if app.account_receipts_off {
        translated(locale, "Off for your account, so only groups get them.")
    } else {
        translated(locale, "Let people see when you read their messages.")
    };
    privacy.toggle(
        translated(locale, "Send read receipts"),
        receipts_note,
        |settings| &mut settings.send_read_receipts,
    );
    privacy.toggle(
        translated(locale, "Show when you are typing"),
        "",
        |settings| &mut settings.send_typing,
    );
    app_lock_rows(app, &mut privacy);
    // The account values live on the phone: they are shown once fetched and
    // edited only while connected with a fresh snapshot.
    let editable = app.is_connected() && app.account_privacy.editable();
    let note = if app.account_privacy.fetch_failed {
        Some(crate::i18n::gettext(
            locale,
            "Could not load your account privacy. Trying again when ZapFast reconnects.",
        ))
    } else if !app.is_connected() {
        Some(crate::i18n::gettext(
            locale,
            "Connect to WhatsApp to change your account privacy.",
        ))
    } else if !app.account_privacy.loaded {
        Some(crate::i18n::gettext(
            locale,
            "Loading your account privacy…",
        ))
    } else {
        None
    };
    if let Some(note) = note {
        privacy.block(Vec::new(), move |ui, _app| {
            widgets::rich_text(ui, &note, theme::regular(12.5), palette.secondary);
            ui.add_space(10.0);
        });
    }
    for kind in PrivacyKind::ALL {
        let Some(current) = app.account_privacy.get(kind) else {
            continue;
        };
        let title = Text {
            shown: kind.label(locale),
            source: kind.label(Locale::English),
        };
        let mut description = Text {
            shown: kind.hint(locale),
            source: kind.hint(Locale::English),
        };
        // The people an Except list leaves out are chosen on the phone.
        if current == PrivacyChoice::Except {
            let note = translated(locale, "Change who is excluded on your phone.");
            let join = |hint: &str, note: &str| {
                if hint.is_empty() {
                    note.to_owned()
                } else {
                    format!("{hint} {note}")
                }
            };
            description = Text {
                shown: join(&description.shown, &note.shown).into(),
                source: join(&description.source, &note.source).into(),
            };
        }
        privacy.row(title, description, move |ui, app| {
            ui.add_enabled_ui(editable, |ui| privacy_control(ui, app, kind));
        });
    }

    let mut system = Section::new(translated(locale, "System"));
    system.toggle(
        translated(locale, "Keep running when the window closes"),
        keyed(translated(locale, "Quit from the tray or with Ctrl+Q.")),
        |settings| &mut settings.keep_running_in_background,
    );
    if app.start_with_system.is_some() {
        system.row(
            translated(locale, "Start at login"),
            translated(locale, "Starts in the tray, without a window."),
            move |ui, app| {
                let Some(mut enabled) = app.start_with_system else {
                    return;
                };
                let response = widgets::switch(ui, &palette, &mut enabled);
                theme::reveal_focus(&response);
                let label = crate::i18n::gettext(app.locale, "Start at login");
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::Checkbox,
                        ui.is_enabled(),
                        enabled,
                        label.as_ref(),
                    )
                });
                if response.changed() {
                    app.actions.push(Action::SetStartWithSystem(enabled));
                }
            },
        );
    }
    system.toggle(
        translated(locale, "Check for updates"),
        translated(
            locale,
            "Asks GitHub once a day, sending only the ZapFast version.",
        ),
        |settings| &mut settings.check_for_updates,
    );
    system.toggle(
        translated(locale, "Download updates automatically"),
        translated(
            locale,
            "You still choose when to restart. Package managers and Flatpak update ZapFast themselves.",
        ),
        |settings| &mut settings.download_updates_automatically,
    );
    let environment = app
        .settings
        .proxy
        .is_empty()
        .then(crate::proxy::for_whatsapp)
        .flatten();
    let description = match environment {
        Some(proxy) => {
            let redacted = proxy.redacted();
            let text = translated(locale, "Using {proxy} from the environment.");
            Text {
                shown: text.shown.replace("{proxy}", &redacted).into(),
                source: text.source.replace("{proxy}", &redacted).into(),
            }
        }
        None => translated(
            locale,
            "For WhatsApp, media, and updates. Empty uses ALL_PROXY or HTTPS_PROXY.",
        ),
    };
    system.row(translated(locale, "Proxy"), description, move |ui, app| {
        let id = egui::Id::new("settings-proxy-draft");
        let mut draft = ui
            .data(|data| data.get_temp::<String>(id))
            .unwrap_or_else(|| app.settings.proxy.clone());
        let response = ui.add(
            egui::TextEdit::singleline(&mut draft)
                .hint_text("socks5h://127.0.0.1:9050")
                .font(theme::regular(13.0))
                .text_color(palette.text)
                .desired_width(220.0),
        );
        if response.lost_focus() {
            app.actions.push(Action::SetProxy(draft.clone()));
            ui.data_mut(|data| data.remove::<String>(id));
        } else if response.has_focus() {
            ui.data_mut(|data| data.insert_temp(id, draft));
        }
    });
    system.row(
        translated(locale, "GIPHY API key"),
        if crate::settings::BUILT_IN_GIPHY_KEY.is_some() {
            translated(
                locale,
                "For GIF search. Replaces the key this build includes.",
            )
        } else {
            translated(
                locale,
                "For GIF search. Get a free key at developers.giphy.com.",
            )
        },
        move |ui, app| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut app.settings.giphy_key)
                    .font(theme::regular(13.0))
                    .text_color(palette.text)
                    .desired_width(220.0),
            );
            if response.changed() {
                app.actions.push(Action::SettingsChanged);
            }
        },
    );

    let mut account_section = Section::new(translated(locale, "Account"));
    account_section.block(
        vec![
            translated(locale, "Your name"),
            translated(locale, "About"),
            translated(locale, "Change profile picture"),
            translated(locale, "Unlink this computer"),
        ],
        |ui, app| account(app, ui),
    );

    let mut files = Section::new(translated(locale, "Files"));
    let state = app.dirs.state.clone();
    let open_folder = crate::i18n::gettext(locale, "Open folder");
    files.row(
        translated(locale, "Message archive"),
        app.dirs.archive_db().display().to_string(),
        {
            let open_folder = open_folder.clone();
            move |ui, app| {
                if theme::soft_button(ui, &palette, Some(Icon::ExternalLink), &open_folder, false)
                    .clicked()
                {
                    app.actions.push(Action::OpenFolder(state));
                }
            }
        },
    );
    let custom = app.settings.download_folder.clone();
    let media = custom.clone().unwrap_or_else(|| app.dirs.media_cache_dir());
    files.row(
        translated(locale, "Downloads"),
        media.display().to_string(),
        move |ui, app| {
            if theme::soft_button(ui, &palette, Some(Icon::ExternalLink), &open_folder, false)
                .clicked()
            {
                let _ = std::fs::create_dir_all(&media);
                app.actions.push(Action::OpenFolder(media.clone()));
            }
            if theme::soft_button(
                ui,
                &palette,
                None,
                &crate::i18n::gettext(app.locale, "Change…"),
                false,
            )
            .clicked()
            {
                app.actions.push(Action::PickDownloadFolder);
            }
            if custom.is_some()
                && theme::soft_button(
                    ui,
                    &palette,
                    None,
                    &crate::i18n::gettext(app.locale, "Use default"),
                    false,
                )
                .clicked()
            {
                app.actions.push(Action::SetDownloadFolder(None));
            }
        },
    );
    let log = app.dirs.log_file();
    files.row(
        translated(locale, "Log"),
        log.display().to_string(),
        move |ui, app| {
            if theme::soft_button(
                ui,
                &palette,
                Some(Icon::FileText),
                &crate::i18n::gettext(app.locale, "Open"),
                false,
            )
            .clicked()
            {
                app.actions.push(Action::OpenLog(log));
            }
        },
    );

    let mut about_section = Section::new(translated(locale, "About"));
    about_section.block(
        vec![
            "ZapFast".into(),
            translated(locale, "Keyboard shortcuts"),
            translated(locale, "Source code"),
        ],
        |ui, app| about(app, ui),
    );

    vec![
        appearance,
        chats,
        notifications,
        privacy,
        system,
        account_section,
        files,
        about_section,
    ]
}

/// The app lock: a password, how long ZapFast may go unused, and the form
/// that sets, changes, or removes the password.
fn app_lock_rows(app: &App, privacy: &mut Section) {
    use crate::app_lock::FormMode;
    let locale = app.locale;
    let palette = app.palette;
    let enabled = app.settings.app_lock_hash.is_some();
    privacy.row(
        translated(locale, "App lock"),
        translated(
            locale,
            "Asks for a password at start and after a while unused. It keeps people using this computer out of your chats and encrypts nothing more. A forgotten password means unlinking.",
        ),
        move |ui, app| {
            use crate::i18n::gettext;
            // The row lays its controls out from the right.
            let modes = if enabled {
                vec![
                    (FormMode::TurnOff, gettext(app.locale, "Turn off…")),
                    (FormMode::Change, gettext(app.locale, "Change password…")),
                ]
            } else {
                vec![(FormMode::Set, gettext(app.locale, "Set password…"))]
            };
            for (mode, label) in modes {
                if theme::soft_button(ui, &palette, None, &label, false).clicked() {
                    app.actions.push(Action::AppLockForm(Some(mode)));
                }
            }
        },
    );
    if app.app_lock.form.is_some() {
        // Found by the same words as its row, so a search keeps them together.
        privacy.block(
            vec![
                translated(locale, "App lock"),
                translated(locale, "Lock after"),
            ],
            app_lock_form,
        );
    }
    if enabled {
        privacy.row(
            translated(locale, "Lock after"),
            keyed(translated(
                locale,
                "Time without using ZapFast, also counted while it is in the tray. Ctrl+Shift+L locks it at once.",
            )),
            move |ui, app| {
                let selected = app.settings.app_lock_after;
                let response = egui::ComboBox::from_id_salt("app_lock_after")
                    .selected_text(selected.label(app.locale))
                    .width(200.0_f32.min(ui.available_width()))
                    .show_ui(ui, |ui| {
                        for after in crate::settings::AutoLock::ALL {
                            if theme_option(
                                ui,
                                &palette,
                                after.label(app.locale).as_ref(),
                                after == selected,
                            ) {
                                app.actions.push(Action::SetAutoLock(after));
                            }
                        }
                    });
                theme::reveal_focus(&response.response);
            },
        );
    }
}

/// The password form under the app lock row. Its fields belong to the view;
/// the passwords leave them only for the checking thread.
fn app_lock_form(ui: &mut egui::Ui, app: &mut App) {
    use crate::app_lock::{FormError, FormMode, MIN_PASSWORD_CHARS};
    use crate::i18n::gettext;
    let palette = app.palette;
    let locale = app.locale;
    let Some(form) = app.app_lock.form.as_mut() else {
        return;
    };
    let mode = form.mode;
    let busy = form.busy;
    let mut submit = false;
    ui.add_space(4.0);
    theme::text(
        ui,
        match mode {
            FormMode::Set => gettext(locale, "Set an app lock password"),
            FormMode::Change => gettext(locale, "Change the app lock password"),
            FormMode::TurnOff => gettext(locale, "Turn off the app lock"),
        },
        theme::semibold(14.0),
        palette.text,
    );
    let count = MIN_PASSWORD_CHARS.to_string();
    let mut fields: Vec<(&mut String, String, &'static str)> = Vec::new();
    if mode != FormMode::Set {
        fields.push((
            &mut form.current,
            gettext(locale, "Current password").into_owned(),
            "app-lock-current",
        ));
    }
    if mode != FormMode::TurnOff {
        fields.push((
            &mut form.new,
            gettext(locale, "New password, at least {count} characters").replace("{count}", &count),
            "app-lock-new",
        ));
        fields.push((
            &mut form.confirm,
            gettext(locale, "Type the new password again").into_owned(),
            "app-lock-confirm",
        ));
    }
    let focus_first = ui.memory(|memory| memory.focused().is_none());
    for (index, (text, hint, id)) in fields.into_iter().enumerate() {
        let id = egui::Id::new(id);
        // TextEdit surrenders focus on Enter; take the key before drawing it.
        submit |= ui.memory(|memory| memory.has_focus(id))
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        let response = ui.add_enabled(
            !busy,
            egui::TextEdit::singleline(text)
                .id(id)
                .password(true)
                .hint_text(hint)
                .font(theme::regular(13.0))
                .desired_width(320.0_f32.min(ui.available_width())),
        );
        if index == 0 && focus_first && !busy {
            response.request_focus();
        }
    }
    let error = form.error.map(|error| match error {
        FormError::TooShort => gettext(locale, "The password needs at least {count} characters.")
            .replace("{count}", &count),
        FormError::Mismatch => gettext(locale, "The two new passwords are different.").into_owned(),
        FormError::WrongCurrent => gettext(locale, "Wrong password. Try again.").into_owned(),
    });
    if let Some(error) = error {
        widgets::rich_text(ui, &error, theme::regular(12.5), palette.danger);
    }
    ui.horizontal(|ui| {
        let confirm = match mode {
            FormMode::Set => gettext(locale, "Turn on"),
            FormMode::Change => gettext(locale, "Change password"),
            FormMode::TurnOff => gettext(locale, "Turn off"),
        };
        submit |= ui
            .add_enabled_ui(!busy, |ui| theme::pill_button(ui, &palette, &confirm, true))
            .inner
            .clicked();
        if busy {
            theme::spinner(ui, 16.0, palette.accent);
        } else if theme::pill_button(ui, &palette, &gettext(locale, "Cancel"), false).clicked() {
            app.actions.push(Action::AppLockForm(None));
        }
    });
    ui.add_space(10.0);
    if submit && !busy {
        app.actions.push(Action::SubmitAppLockForm);
    }
}

/// A translated description that names keys, with Cmd and Option on macOS.
/// The search still finds it by its English source.
fn keyed(text: Text) -> Text {
    Text {
        shown: super::keys::label(&text.shown).into(),
        source: text.source,
    }
}

/// The theme menu and the button that opens the themes folder.
fn theme_picker(ui: &mut egui::Ui, app: &mut App) {
    let palette = app.palette;
    ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
        let selected = app
            .settings
            .custom_theme
            .as_deref()
            .map(fastframe_theme::display_name)
            .unwrap_or_else(|| app.settings.theme.label());
        let response = egui::ComboBox::from_id_salt("appearance_theme")
            .selected_text(" ")
            .width(200.0_f32.min(ui.available_width()))
            .height(320.0)
            .show_ui(ui, |ui| {
                for &choice in ThemeChoice::ALL {
                    if theme_option(
                        ui,
                        &palette,
                        choice.label(),
                        app.settings.custom_theme.is_none() && app.settings.theme == choice,
                    ) {
                        app.actions.push(Action::SetTheme(choice));
                    }
                }
                if app.custom_themes.picker_themes().next().is_some() {
                    ui.separator();
                }
                for custom in app.custom_themes.picker_themes() {
                    if theme_option(
                        ui,
                        &palette,
                        fastframe_theme::display_name(&custom.filename),
                        app.settings.custom_theme.as_deref() == Some(custom.filename.as_str()),
                    ) {
                        app.actions
                            .push(Action::SetCustomTheme(custom.filename.clone()));
                    }
                }
            });
        theme::reveal_focus(&response.response);
        let rect = response.response.rect;
        let text = widgets::line(
            ui,
            selected,
            theme::regular(14.0),
            palette.text,
            rect.width() - 36.0,
            1,
        );
        text.paint(
            ui,
            egui::pos2(rect.left() + 8.0, rect.center().y - text.size().y / 2.0),
            palette.text,
        );
        response.response.widget_info(|| {
            let mut info =
                egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, ui.is_enabled(), "Theme");
            info.current_text_value = Some(selected.to_owned());
            info
        });
        if theme::soft_button(
            ui,
            &palette,
            Some(Icon::ExternalLink),
            "Open themes folder",
            false,
        )
        .clicked()
        {
            app.actions.push(Action::OpenThemesFolder);
        }
        if theme::soft_button(
            ui,
            &palette,
            Some(Icon::ExternalLink),
            &crate::i18n::gettext(app.locale, "How to make a theme"),
            false,
        )
        .clicked()
        {
            app.actions.push(Action::OpenUrl(THEMES_GUIDE.to_owned()));
        }
    });
}

/// The website's page on writing a theme.
const THEMES_GUIDE: &str = "https://zapfast.rocks/themes/";

/// The interface language menu.
fn font_picker(ui: &mut egui::Ui, app: &mut App) {
    use crate::settings::FontChoice;
    let palette = app.palette;
    let selected = app.settings.font;
    // "Inter" is a name; "System" is a word.
    let label = |choice: FontChoice| match choice {
        FontChoice::System => crate::i18n::gettext(app.locale, choice.label()).into_owned(),
        FontChoice::Inter => choice.label().to_owned(),
    };
    let response = egui::ComboBox::from_id_salt("interface_font")
        .selected_text(label(selected))
        .width(200.0_f32.min(ui.available_width()))
        .show_ui(ui, |ui| {
            for choice in FontChoice::ALL {
                if theme_option(ui, &palette, &label(choice), selected == choice) {
                    app.actions.push(Action::SetFont(choice));
                }
            }
        });
    theme::reveal_focus(&response.response);
}

fn language_picker(ui: &mut egui::Ui, app: &mut App) {
    let palette = app.palette;
    let selected = app.settings.interface_language;
    let label = match selected {
        Some(locale) => locale.label().to_owned(),
        None => crate::i18n::gettext(app.locale, "Auto").into_owned(),
    };
    let response = egui::ComboBox::from_id_salt("interface_language")
        .selected_text(label)
        .width(200.0_f32.min(ui.available_width()))
        .show_ui(ui, |ui| {
            if theme_option(
                ui,
                &palette,
                crate::i18n::gettext(app.locale, "Auto").as_ref(),
                selected.is_none(),
            ) {
                app.actions.push(Action::SetInterfaceLanguage(None));
            }
            for locale in Locale::ALL {
                if theme_option(ui, &palette, locale.label(), selected == Some(locale)) {
                    app.actions.push(Action::SetInterfaceLanguage(Some(locale)));
                }
            }
        });
    theme::reveal_focus(&response.response);
}

/// Wallpaper colour picker and live preview.
pub fn wallpaper_show(app: &mut App, ui: &mut egui::Ui) {
    if theme::macos_chrome(ui.ctx()) {
        super::banner(app, ui);
    }
    let palette = app.palette;
    let body_height = ui.available_height().max(0.0);
    ui.with_layout(
        Layout::left_to_right(Align::Min).with_main_align(Align::Min),
        |ui| {
            const HEADER_HEIGHT: f32 = 52.0;
            const MIN_PREVIEW_WIDTH: f32 = 180.0;
            let total_width = ui.available_width();
            let left_width = (total_width * 0.42)
                .clamp(220.0, 520.0)
                .min((total_width - MIN_PREVIEW_WIDTH).max(0.0));
            let palette_width = (left_width - 40.0).max(0.0);
            let preview_width = (total_width - left_width).max(0.0);
            ui.allocate_ui_with_layout(
                vec2(left_width, body_height),
                Layout::top_down(Align::Min).with_main_align(Align::Min),
                |ui| {
                    let section = ui.max_rect();
                    let header = Rect::from_min_size(
                        section.left_top(),
                        vec2(left_width, HEADER_HEIGHT),
                    );
                    ui.painter().rect_filled(header, 0.0, palette.panel);
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(header)
                            .layout(
                                Layout::left_to_right(Align::Center)
                                    .with_main_align(Align::Min),
                            ),
                        |ui| {
                            ui.add_space(24.0);
                            if theme::icon_button(
                                ui,
                                Icon::ArrowLeft,
                                20.0,
                                palette.secondary,
                                palette.text,
                                "Back to settings",
                            )
                            .clicked()
                            {
                                app.actions.push(Action::Open(Page::Settings));
                            }
                            theme::text(ui, "Set chat wallpaper", theme::bold(18.0), palette.text);
                        },
                    );

                    let palette_rect = Rect::from_min_size(
                        pos2(section.left() + 20.0, header.bottom() + 20.0),
                        vec2(palette_width, (body_height - HEADER_HEIGHT - 20.0).max(0.0)),
                    );
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(palette_rect)
                            .layout(Layout::top_down(Align::Min).with_main_align(Align::Min)),
                        |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("wallpaper-palette")
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.allocate_ui_with_layout(
                                        vec2(palette_width, 28.0),
                                        Layout::top_down(Align::Center),
                                        |ui| {
                                            let mut doodles = app.settings.show_wallpaper;
                                            let checkbox = ui.checkbox(
                                                &mut doodles,
                                                crate::i18n::gettext(app.locale, "Add doodles"),
                                            );
                                            checkbox.on_hover_text(crate::i18n::gettext(
                                                app.locale,
                                                "Show the default doodles over the selected colour.",
                                            ));
                                            if doodles != app.settings.show_wallpaper {
                                                app.actions.push(Action::SetWallpaperDoodles(doodles));
                                            }
                                        },
                                    );
                                    ui.add_space(12.0);
                                    image_buttons(app, ui, palette_width);
                                    ui.add_space(18.0);
                                    let button_width = 80.0;
                                    let item_spacing = ui.spacing().item_spacing.x;
                                    let columns = ((palette_width + item_spacing)
                                        / (button_width + item_spacing))
                                        .floor()
                                        .max(1.0)
                                        as usize;
                                    let grid_width = button_width * columns as f32
                                        + item_spacing * columns.saturating_sub(1) as f32;
                                    ui.horizontal(|ui| {
                                        ui.add_space((palette_width - grid_width).max(0.0) / 2.0);
                                        ui.horizontal_wrapped(|ui| {
                                            let selected =
                                                app.settings.wallpaper_color_for(palette.dark);
                                            for color in WallpaperColor::choices(palette.dark) {
                                                if wallpaper_color_button(
                                                    ui,
                                                    &palette,
                                                    app.locale,
                                                    *color,
                                                    selected,
                                                ) {
                                                    app.actions.push(Action::SetWallpaperColor(*color));
                                                }
                                            }
                                        });
                                    });
                                });
                        },
                    );
                },
            );
            let divider_x = ui.cursor().left();
            let divider_top = ui.cursor().top();
            ui.allocate_ui_with_layout(
                vec2(preview_width, body_height),
                Layout::top_down(Align::Min).with_main_align(Align::Min),
                |ui| {
                    let section = ui.max_rect();
                    let header = Rect::from_min_size(
                        section.left_top(),
                        vec2(preview_width, HEADER_HEIGHT),
                    );
                    ui.painter().rect_filled(header, 0.0, palette.panel);
                    ui.painter().text(
                        header.center(),
                        egui::Align2::CENTER_CENTER,
                        "Wallpaper preview",
                        theme::bold(18.0),
                        palette.text,
                    );
                    let preview = Rect::from_min_size(
                        header.left_bottom(),
                        vec2(preview_width, (body_height - HEADER_HEIGHT).max(0.0)),
                    );
                    // The same look the chat draws, image and theme colour included.
                    wallpaper::paint_rect(ui, preview, &app.wallpaper());
                },
            );
            ui.painter().line_segment(
                [
                    pos2(divider_x, divider_top),
                    pos2(divider_x, divider_top + body_height),
                ],
                Stroke::new(1.0, palette.outline),
            );
        },
    );
}

/// "Choose image…", and "Remove image" while one is set, centred over the
/// colours. An image replaces the colour and doodles in the chat.
fn image_buttons(app: &mut App, ui: &mut egui::Ui, width: f32) {
    let palette = app.palette;
    let choose = crate::i18n::gettext(app.locale, "Choose image…");
    let remove = crate::i18n::gettext(app.locale, "Remove image");
    let has_image = app.settings.wallpaper_image.is_some();
    let spacing = ui.spacing().item_spacing.x;
    let mut row_width = theme::soft_button_width(ui, &choose, true);
    if has_image {
        row_width += spacing + theme::soft_button_width(ui, &remove, true);
    }
    ui.allocate_ui_with_layout(
        vec2(width, 32.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.add_space(((width - row_width) / 2.0).max(0.0));
            if theme::soft_button(ui, &palette, Some(Icon::Image), &choose, false).clicked() {
                app.actions.push(Action::PickWallpaperImage);
            }
            if has_image
                && theme::soft_button(ui, &palette, Some(Icon::Trash), &remove, false).clicked()
            {
                app.actions.push(Action::RemoveWallpaperImage);
            }
        },
    );
}

/// A wallpaper colour's name in the interface language.
fn wallpaper_label(locale: Locale, color: WallpaperColor) -> String {
    match color {
        WallpaperColor::Theme => {
            crate::i18n::pgettext(locale, "wallpaper colour", "Theme").into_owned()
        }
        color => color.label().to_owned(),
    }
}

fn wallpaper_color_button(
    ui: &mut egui::Ui,
    palette: &Palette,
    locale: Locale,
    color: WallpaperColor,
    selected: WallpaperColor,
) -> bool {
    let fill = color.color32(palette);
    let label = wallpaper_label(locale, color);
    // Theme follows the palette, so its swatch names itself.
    let text = if color == WallpaperColor::Theme {
        egui::RichText::new(label.as_str())
            .font(theme::medium(13.0))
            .color(palette.text)
    } else {
        egui::RichText::new(" ")
    };
    let button = egui::Button::new(text)
        .min_size(Vec2::splat(80.0))
        .fill(fill)
        .stroke(if color == selected {
            Stroke::new(4.0, fill.gamma_multiply(0.5))
        } else if color == WallpaperColor::Theme {
            // Otherwise the swatch vanishes into a panel of the same colour.
            Stroke::new(1.0, palette.outline)
        } else {
            Stroke::NONE
        })
        .corner_radius(CornerRadius::ZERO);
    let response = ui.add(button).on_hover_text(label.as_str());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            ui.is_enabled(),
            color == selected,
            label.as_str(),
        )
    });
    response.clicked()
}

/// A titled group of settings on a rounded card.
fn section(
    ui: &mut egui::Ui,
    palette: &Palette,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    ui.add_space(12.0);
    theme::text(ui, title, theme::bold(18.0), palette.text);
    ui.add_space(8.0);
    Frame::new()
        .fill(theme::blend(palette.panel, palette.surface, 0.5))
        .stroke(Stroke::new(1.0, palette.outline))
        .corner_radius(CornerRadius::same(theme::RADIUS + 2))
        // Every row ends with its own gap, so the bottom margin is smaller.
        .inner_margin(Margin {
            left: 20,
            right: 20,
            top: 16,
            bottom: 6,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add_contents(ui);
        });
    ui.add_space(8.0);
}

/// Our WhatsApp profile, which can be edited in place, and unlinking.
fn account(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let name = app.me_name.clone().unwrap_or_default();
    let me = app.me.clone().unwrap_or_default();
    let phone = crate::model::phone_of(&me)
        .map(crate::util::phone)
        .unwrap_or_else(|| me.clone());
    // The name and About being edited; `None` while not editing.
    let draft_id = ui.id().with("profile_draft");
    let mut draft: Option<(String, String)> = ui.data_mut(|data| data.get_temp(draft_id));
    let mut submitted = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let picture = app.avatar_full(&me).or_else(|| app.avatar(&me));
        let change_picture = crate::i18n::gettext(app.locale, "Change profile picture");
        if widgets::clickable_avatar(
            ui,
            &palette,
            &name,
            &me,
            56.0,
            picture.as_deref(),
            &change_picture,
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(change_picture.as_ref())
        .clicked()
        {
            app.actions.push(Action::PickProfilePicture);
        }
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - 230.0).max(160.0));
            if let Some((draft_name, draft_about)) = &mut draft {
                submitted |= profile_field(
                    ui,
                    &palette,
                    draft_name,
                    &crate::i18n::gettext(app.locale, "Your name"),
                    25,
                );
                ui.add_space(6.0);
                submitted |= profile_field(
                    ui,
                    &palette,
                    draft_about,
                    &crate::i18n::gettext(app.locale, "About"),
                    139,
                );
                return;
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let shown = if name.is_empty() {
                    crate::i18n::gettext(app.locale, "Linked device")
                } else {
                    name.clone().into()
                };
                theme::text(ui, shown, theme::semibold(16.0), palette.text);
                if theme::icon_button(
                    ui,
                    Icon::Pencil,
                    15.0,
                    palette.secondary,
                    palette.text,
                    &crate::i18n::gettext(app.locale, "Edit your name and About"),
                )
                .clicked()
                {
                    draft = Some((name.clone(), app.me_about.clone().unwrap_or_default()));
                }
            });
            theme::text(ui, &phone, theme::regular(13.0), palette.secondary);
            if let Some(about) = &app.me_about {
                theme::paragraph(ui, about, theme::regular(13.0), palette.secondary);
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if theme::soft_button(
                ui,
                &palette,
                Some(Icon::LogOut),
                &crate::i18n::gettext(app.locale, "Unlink this computer"),
                false,
            )
            .clicked()
            {
                app.actions.push(Action::ShowDialog(Dialog::ConfirmUnlink));
            }
        });
    });
    if let Some((draft_name, draft_about)) = &draft {
        ui.add_space(10.0);
        let mut done = false;
        ui.horizontal(|ui| {
            ui.add_space(56.0 + 14.0);
            let name = draft_name.trim();
            let about = draft_about.trim();
            let changed_name =
                (!name.is_empty() && app.me_name.as_deref() != Some(name)).then(|| name.to_owned());
            let changed_about =
                (app.me_about.as_deref().unwrap_or_default() != about).then(|| about.to_owned());
            let save = crate::i18n::gettext(app.locale, "Save");
            if theme::pill_button(ui, &palette, &save, true).clicked() || submitted {
                if changed_name.is_some() || changed_about.is_some() {
                    app.actions.push(Action::SetProfile {
                        name: changed_name,
                        about: changed_about,
                    });
                }
                done = true;
            }
            let cancel = crate::i18n::gettext(app.locale, "Cancel");
            if theme::pill_button(ui, &palette, &cancel, false).clicked() {
                done = true;
            }
        });
        if done {
            draft = None;
        }
    }
    ui.add_space(10.0);
    ui.data_mut(|data| {
        if let Some(draft) = draft {
            data.insert_temp(draft_id, draft);
        } else {
            data.remove::<(String, String)>(draft_id);
        }
    });
}

/// A single-line profile text field with its caption above it. Returns
/// whether Enter submitted it.
fn profile_field(
    ui: &mut egui::Ui,
    palette: &Palette,
    value: &mut String,
    label: &str,
    limit: usize,
) -> bool {
    theme::text(ui, label, theme::medium(12.5), palette.secondary);
    let response = ui.add(
        egui::TextEdit::singleline(value)
            .font(theme::regular(14.0))
            .text_color(palette.text)
            .char_limit(limit)
            .desired_width(ui.available_width()),
    );
    response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))
}

/// The version, links to more about ZapFast, and who made it.
fn about(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let (logo, _) = ui.allocate_exact_size(Vec2::splat(44.0), egui::Sense::hover());
        theme::mark(ui, logo.center(), 44.0);
        ui.vertical(|ui| {
            theme::text(
                ui,
                format!("ZapFast {}", env!("CARGO_PKG_VERSION")),
                theme::semibold(16.0),
                palette.text,
            );
            theme::paragraph(
                ui,
                crate::i18n::gettext(
                    app.locale,
                    "A native WhatsApp client built with Rust, egui, and whatsapp-rust.",
                ),
                theme::regular(13.0),
                palette.secondary,
            );
        });
    });
    ui.add_space(12.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        if let Some(update) = &app.update {
            let label = crate::i18n::gettext(app.locale, "Update to {version}")
                .replace("{version}", &update.version);
            if theme::soft_button(ui, &palette, Some(Icon::Download), &label, false).clicked() {
                app.actions.push(Action::ShowUpdate);
            }
        }
        if theme::soft_button(
            ui,
            &palette,
            Some(Icon::Keyboard),
            &crate::i18n::gettext(app.locale, "Keyboard shortcuts"),
            false,
        )
        .clicked()
        {
            app.actions.push(Action::ShowDialog(Dialog::Shortcuts));
        }
        if theme::soft_button(
            ui,
            &palette,
            Some(Icon::Info),
            &crate::i18n::gettext(app.locale, "About"),
            false,
        )
        .clicked()
        {
            app.actions.push(Action::ShowDialog(Dialog::About));
        }
        if theme::soft_button(
            ui,
            &palette,
            Some(Icon::ExternalLink),
            &crate::i18n::gettext(app.locale, "Source code"),
            false,
        )
        .clicked()
        {
            app.actions
                .push(Action::OpenUrl(env!("CARGO_PKG_REPOSITORY").to_owned()));
        }
    });
    ui.add_space(14.0);
    if widgets::credit(ui, &palette, app.locale) {
        app.actions
            .push(Action::OpenUrl(widgets::AUTHOR_URL.to_owned()));
    }
}

fn toggle(
    ui: &mut egui::Ui,
    app: &mut App,
    label: &str,
    description: &str,
    field: impl Fn(&mut crate::settings::Settings) -> &mut bool,
) {
    let palette = app.palette;
    let mut value = *field(&mut app.settings);
    let mut changed = false;
    widgets::setting_row(ui, &palette, label, description, |ui| {
        let response = widgets::switch(ui, &palette, &mut value);
        theme::reveal_focus(&response);
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), value, label)
        });
        changed = response.changed();
    });
    if changed {
        *field(&mut app.settings) = value;
        app.actions.push(Action::SettingsChanged);
    }
}

/// Title and description of the sound for new messages or for mentions and
/// replies to us, as Pidgin plays one sound for messages and alerts when a
/// chat says your name.
fn sound_text(locale: Locale, mention: bool) -> (Text, Text) {
    if mention {
        (
            translated(locale, "Mention sound"),
            translated(locale, "When a group mentions you or replies to you."),
        )
    } else {
        (translated(locale, "Message sound"), Text::default())
    }
}

/// The sound menu, with a button that plays the current choice.
fn sound_control(ui: &mut egui::Ui, app: &mut App, mention: bool) {
    use crate::i18n::{gettext, pgettext};
    use crate::settings::NotificationSound;
    let palette = app.palette;
    let locale = app.locale;
    let current = if mention {
        app.settings.mention_sound.clone()
    } else {
        app.settings.message_sound.clone()
    };
    let choices = [
        (NotificationSound::Receive, "Pidgin".into()),
        (NotificationSound::Alert, gettext(locale, "Pidgin alert")),
        (NotificationSound::System, gettext(locale, "System default")),
        // Translators: no notification sound.
        (NotificationSound::None, pgettext(locale, "sound", "None")),
    ];
    let selected = match &current {
        NotificationSound::Custom(path) => path.file_name().map_or_else(
            || "Custom".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        ),
        sound => choices
            .iter()
            .find(|(choice, _)| choice == sound)
            .map(|(_, name)| name.to_string())
            .unwrap_or_default(),
    };
    if !matches!(current, NotificationSound::System | NotificationSound::None)
        && theme::icon_button(
            ui,
            Icon::Play,
            16.0,
            palette.secondary,
            palette.text,
            &gettext(locale, "Play"),
        )
        .clicked()
    {
        app.actions.push(Action::PreviewSound(current.clone()));
    }
    let response = egui::ComboBox::from_id_salt(("notification-sound", mention))
        .selected_text(selected)
        .width(170.0_f32.min(ui.available_width()))
        .show_ui(ui, |ui| {
            for (sound, name) in choices {
                if ui.selectable_label(current == sound, name).clicked() {
                    app.actions
                        .push(Action::SetNotificationSound { mention, sound });
                }
            }
            if ui
                .selectable_label(false, gettext(locale, "Choose a file…"))
                .clicked()
            {
                app.actions.push(Action::PickNotificationSound { mention });
            }
        });
    theme::reveal_focus(&response.response);
}

/// One account privacy category's picker: what the phone holds, and the
/// values it takes. While a write is in flight it is disabled, so a second
/// pick cannot race the first.
fn privacy_control(ui: &mut egui::Ui, app: &mut App, kind: PrivacyKind) {
    let palette = app.palette;
    let current = app.account_privacy.get(kind);
    let locale = app.locale;
    let selected = current.map_or(Cow::Borrowed("\u{2014}"), |choice| choice.label(locale));
    let pending = app.account_privacy.pending(kind);
    ui.add_enabled_ui(!pending, |ui| {
        ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
            let salt = format!("privacy_{}", kind.wire_name());
            let response = egui::ComboBox::from_id_salt(salt)
                .selected_text(" ")
                .width(220.0_f32.min(ui.available_width()))
                .show_ui(ui, |ui| {
                    for choice in kind.choices() {
                        if theme_option(
                            ui,
                            &palette,
                            &choice.label(locale),
                            current == Some(*choice),
                        ) {
                            app.actions.push(Action::SetAccountPrivacy {
                                kind,
                                choice: *choice,
                            });
                        }
                    }
                });
            theme::reveal_focus(&response.response);
            let rect = response.response.rect;
            let text = widgets::line(
                ui,
                &selected,
                theme::regular(14.0),
                palette.text,
                rect.width() - 36.0,
                1,
            );
            text.paint(
                ui,
                egui::pos2(rect.left() + 8.0, rect.center().y - text.size().y / 2.0),
                palette.text,
            );
            response.response.widget_info(|| {
                let mut info = egui::WidgetInfo::labeled(
                    egui::WidgetType::ComboBox,
                    ui.is_enabled(),
                    kind.label(locale),
                );
                info.current_text_value = Some(selected.to_string());
                info
            });
        });
    });
}

/// Theme filenames can contain emoji, so paint them through the shared line renderer.
fn theme_option(ui: &mut egui::Ui, palette: &theme::Palette, text: &str, selected: bool) -> bool {
    let response = ui.add(
        egui::Button::selectable(selected, " ").min_size(egui::vec2(ui.available_width(), 28.0)),
    );
    let rect = response.rect;
    let line = widgets::line(
        ui,
        text,
        theme::regular(14.0),
        palette.text,
        rect.width() - 16.0,
        1,
    );
    if ui.is_rect_visible(rect) {
        line.paint(
            ui,
            egui::pos2(rect.left() + 8.0, rect.center().y - line.size().y / 2.0),
            palette.text,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            text,
        )
    });
    response.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(section: Section, filter: &Filter) -> Vec<String> {
        let (_, rows) = section.visible(filter);
        rows.into_iter()
            .map(|(row, _)| row.title.shown.into_owned())
            .collect()
    }

    fn window(locale: Locale) -> Section {
        let mut window = Section::new(translated(locale, "Notifications"));
        window.toggle("Check for updates", "Ask GitHub once a day.", |settings| {
            &mut settings.check_for_updates
        });
        let (title, description) = sound_text(locale, false);
        window.row(title, description, |_, _| {});
        window.toggle(
            translated(locale, "Play sounds for group messages"),
            "",
            |settings| &mut settings.group_sounds,
        );
        window
    }

    #[test]
    fn an_empty_search_shows_every_row() {
        let filter = Filter::new("  ");
        assert!(filter.is_empty());
        assert_eq!(window(Locale::English).visible(&filter).1.len(), 3);
    }

    #[test]
    fn the_search_matches_titles_and_descriptions_ignoring_case_and_accents() {
        let rows = |query| titles(window(Locale::English), &Filter::new(query));
        assert_eq!(
            rows("SOUND"),
            ["Message sound", "Play sounds for group messages"]
        );
        assert_eq!(rows("github"), ["Check for updates"]);
        assert!(rows("proxy").is_empty(), "an empty section is left out");
        // German folds its umlauts, so "tone" finds "Töne".
        let german = |query| titles(window(Locale::German), &Filter::new(query));
        assert_eq!(german("tone"), ["Töne für Gruppennachrichten abspielen"]);
    }

    #[test]
    fn a_translated_setting_is_found_in_english_too() {
        let german = |query| titles(window(Locale::German), &Filter::new(query));
        assert_eq!(german("Nachrichtenton"), ["Nachrichtenton"]);
        assert_eq!(german("message sound"), ["Nachrichtenton"]);
    }

    #[test]
    fn a_matching_section_title_keeps_all_its_rows() {
        let rows = titles(window(Locale::German), &Filter::new("notifications"));
        assert_eq!(rows.len(), 3);
        let rows = titles(window(Locale::German), &Filter::new("benachrichtigungen"));
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn blocks_are_found_by_their_keywords() {
        let mut account = Section::new(translated(Locale::English, "Account"));
        account.block(
            vec![translated(Locale::English, "Unlink this computer")],
            |_, _| {},
        );
        assert_eq!(account.visible(&Filter::new("unlink")).1.len(), 1);
        let mut account = Section::new(translated(Locale::English, "Account"));
        account.block(Vec::new(), |_, _| {});
        assert!(account.visible(&Filter::new("unlink")).1.is_empty());
    }
}
