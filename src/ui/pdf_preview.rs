//! Full-window native PDF reader for downloaded chat documents.

use egui::{Align, Frame, Layout, Margin, Rect, Sense, UiBuilder, vec2};

use crate::app::App;
use crate::model::Action;
use crate::pdf_preview::{PageGeometry, PreviewState, Status};
use crate::theme::{self, Icon};

const HEADER: f32 = 56.0;

pub(crate) fn modal_id() -> egui::Id {
    egui::Id::new("pdf-preview")
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(preview) = app.pdf_preview.as_mut() else {
        return;
    };
    let palette = app.palette;
    let locale = app.locale;
    let screen = ctx.content_rect();
    let header_rect = Rect::from_min_size(screen.min, vec2(screen.width(), HEADER));
    let stage = Rect::from_min_max(egui::pos2(screen.left(), header_rect.bottom()), screen.max);
    preview.poll(ctx);

    let mut actions = Vec::new();
    let id = modal_id();
    let area = egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min);
    let response = egui::Modal::new(id)
        .area(area)
        .frame(Frame::new().fill(palette.overlay))
        .backdrop_color(palette.shadow)
        .show(ctx, |ui| {
            ui.set_min_size(screen.size());
            ui.painter().rect_filled(screen, 0.0, palette.overlay);
            ui.painter().rect_filled(stage, 0.0, palette.surface);

            let mut head = ui.new_child(UiBuilder::new().max_rect(header_rect));
            head.set_clip_rect(header_rect);
            header(&mut head, preview, &palette, locale, &mut actions);

            let mut body = ui.new_child(UiBuilder::new().max_rect(stage));
            body.set_clip_rect(stage);
            page(&mut body, preview, stage, &palette, locale, &mut actions);
        });
    if response.should_close() {
        actions.push(Action::ClosePdfPreview);
    }
    app.actions.extend(actions);
}

fn header(
    ui: &mut egui::Ui,
    preview: &mut PreviewState,
    palette: &theme::Palette,
    locale: crate::i18n::Locale,
    actions: &mut Vec<Action>,
) {
    let rect = ui.max_rect();
    super::caption_drag(ui, rect);
    let room = super::caption_room(ui.ctx(), rect, 0.0);
    let inset = Rect::from_min_max(
        rect.min + vec2(room.left, 0.0),
        rect.max - vec2(room.right, 0.0),
    );
    let mut header = ui.new_child(UiBuilder::new().max_rect(inset));
    header.set_clip_rect(rect);
    Frame::new()
        .fill(palette.panel)
        .inner_margin(Margin::symmetric(12, 6))
        .show(&mut header, |ui| {
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), HEADER - 12.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    if theme::icon_button(
                        ui,
                        Icon::X,
                        18.0,
                        palette.secondary,
                        palette.text,
                        crate::i18n::gettext(locale, "Close PDF preview (Esc)").as_ref(),
                    )
                    .clicked()
                    {
                        actions.push(Action::ClosePdfPreview);
                    }
                    ui.add_space(8.0);
                    let name = preview
                        .path()
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| {
                            crate::i18n::gettext(locale, "PDF document").into_owned()
                        });
                    let search_t = ui.ctx().animate_bool_with_time(
                        egui::Id::new("pdf-search-bar"),
                        preview.search_open(),
                        0.16,
                    );
                    const SEARCH_BAR_WIDTH: f32 = 320.0;
                    let reserved = 290.0
                        + if search_t > 0.0 {
                            (SEARCH_BAR_WIDTH + 8.0) * search_t
                        } else {
                            0.0
                        }
                        + if preview.has_selection() { 68.0 } else { 0.0 };
                    ui.allocate_ui_with_layout(
                        vec2((ui.available_width() - reserved).max(0.0), 36.0),
                        Layout::left_to_right(Align::Center),
                        |ui| theme::text(ui, name, theme::medium(14.0), palette.text),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::icon_button(
                            ui,
                            Icon::ExternalLink,
                            18.0,
                            palette.secondary,
                            palette.text,
                            crate::i18n::gettext(locale, "Open in another app").as_ref(),
                        )
                        .clicked()
                        {
                            actions.push(Action::ClosePdfPreview);
                            actions.push(Action::OpenFile(preview.path().to_owned()));
                        }
                        if theme::icon_button(
                            ui,
                            Icon::Plus,
                            18.0,
                            palette.secondary,
                            palette.text,
                            crate::i18n::gettext(locale, "Zoom in").as_ref(),
                        )
                        .clicked()
                        {
                            actions.push(Action::ZoomPdfBy(1.25));
                        }
                        if theme::soft_button(
                            ui,
                            palette,
                            None,
                            &format!("{:.0}%", preview.zoom() * 100.0),
                            false,
                        )
                        .on_hover_text(crate::i18n::gettext(locale, "Fit page to window (0)"))
                        .clicked()
                        {
                            actions.push(Action::FitPdf);
                        }
                        if theme::icon_button(
                            ui,
                            Icon::Minus,
                            18.0,
                            palette.secondary,
                            palette.text,
                            crate::i18n::gettext(locale, "Zoom out").as_ref(),
                        )
                        .clicked()
                        {
                            actions.push(Action::ZoomPdfBy(0.8));
                        }
                        ui.add_space(8.0);
                        if theme::icon_button(
                            ui,
                            Icon::ChevronRight,
                            18.0,
                            palette.secondary,
                            palette.text,
                            crate::i18n::gettext(locale, "Next page").as_ref(),
                        )
                        .clicked()
                        {
                            actions.push(Action::PdfPage(1));
                        }
                        theme::text(
                            ui,
                            format!("{} / {}", preview.page() + 1, preview.page_count()),
                            theme::regular(13.0),
                            palette.secondary,
                        );
                        if theme::icon_button(
                            ui,
                            Icon::ChevronLeft,
                            18.0,
                            palette.secondary,
                            palette.text,
                            crate::i18n::gettext(locale, "Previous page").as_ref(),
                        )
                        .clicked()
                        {
                            actions.push(Action::PdfPage(-1));
                        }
                        ui.add_space(12.0);
                        let search_tip = format!(
                            "{} ({})",
                            crate::i18n::gettext(locale, "Find in PDF"),
                            super::keys::label("Ctrl+F")
                        );
                        if theme::icon_button(
                            ui,
                            Icon::Search,
                            18.0,
                            if preview.search_open() {
                                palette.accent
                            } else {
                                palette.secondary
                            },
                            palette.text,
                            &search_tip,
                        )
                        .clicked()
                        {
                            actions.push(if preview.search_open() {
                                Action::ClosePdfSearch
                            } else {
                                Action::PdfFind
                            });
                        }
                        if search_t > 0.0 {
                            let animated_width = (SEARCH_BAR_WIDTH * search_t).round();
                            ui.allocate_ui_with_layout(
                                vec2(animated_width, 32.0),
                                Layout::right_to_left(Align::Center),
                                |ui| {
                                    ui.set_clip_rect(ui.clip_rect().intersect(ui.max_rect()));
                                    if theme::icon_button(
                                        ui,
                                        Icon::X,
                                        14.0,
                                        palette.secondary,
                                        palette.text,
                                        crate::i18n::gettext(locale, "Close search").as_ref(),
                                    )
                                    .clicked()
                                    {
                                        actions.push(Action::ClosePdfSearch);
                                    }
                                    if theme::icon_button(
                                        ui,
                                        Icon::ChevronRight,
                                        18.0,
                                        palette.secondary,
                                        palette.text,
                                        crate::i18n::gettext(locale, "Next match").as_ref(),
                                    )
                                    .clicked()
                                    {
                                        actions.push(Action::PdfMatch(1));
                                    }
                                    if theme::icon_button(
                                        ui,
                                        Icon::ChevronLeft,
                                        18.0,
                                        palette.secondary,
                                        palette.text,
                                        crate::i18n::gettext(locale, "Previous match").as_ref(),
                                    )
                                    .clicked()
                                    {
                                        actions.push(Action::PdfMatch(-1));
                                    }
                                    theme::text(
                                        ui,
                                        format!(
                                            "{} / {}",
                                            preview.active_match(),
                                            preview.match_count()
                                        ),
                                        theme::regular(13.0),
                                        palette.secondary,
                                    );
                                    let mut query = preview.query().to_owned();
                                    let search = egui::TextEdit::singleline(&mut query)
                                        .id(PreviewState::search_id())
                                        .hint_text(crate::i18n::gettext(locale, "Find in PDF"))
                                        .desired_width(150.0)
                                        .show(ui)
                                        .response;
                                    if search.changed() {
                                        preview.set_query(query);
                                    }
                                },
                            );
                        }
                        if preview.has_selection()
                            && theme::soft_button(
                                ui,
                                palette,
                                None,
                                crate::i18n::gettext(locale, "Copy").as_ref(),
                                false,
                            )
                            .clicked()
                        {
                            actions.push(Action::PdfCopySelection);
                        }
                    });
                },
            );
        });
}

fn page(
    ui: &mut egui::Ui,
    preview: &mut PreviewState,
    stage: Rect,
    palette: &theme::Palette,
    locale: crate::i18n::Locale,
    actions: &mut Vec<Action>,
) {
    let view = stage.shrink(16.0);
    if !view.is_positive() {
        return;
    }
    let message = match preview.status() {
        Status::Loading => Some(crate::i18n::gettext(locale, "Opening PDF…").into_owned()),
        Status::Failed(message) => Some(crate::i18n::gettext(locale, message).into_owned()),
        Status::Ready => None,
    };
    if let Some(message) = message {
        ui.scope_builder(UiBuilder::new().max_rect(view), |ui| {
            ui.vertical_centered_justified(|ui| {
                ui.add_space((view.height() * 0.35).max(16.0));
                if preview.status() == Status::Loading {
                    ui.spinner();
                }
                theme::text(ui, message, theme::regular(14.0), palette.secondary);
                if matches!(preview.status(), Status::Failed(_))
                    && theme::soft_button(
                        ui,
                        palette,
                        Some(Icon::ExternalLink),
                        crate::i18n::gettext(locale, "Open in another app").as_ref(),
                        false,
                    )
                    .clicked()
                {
                    actions.push(Action::ClosePdfPreview);
                    actions.push(Action::OpenFile(preview.path().to_owned()));
                }
            });
        });
        return;
    }

    let pages = preview.layout(view.size());
    let Some(last) = pages.last() else {
        return;
    };
    let width = pages.iter().map(Rect::width).fold(view.width(), f32::max);
    let height = if pages.len() == 1 {
        last.bottom().max(view.height())
    } else {
        last.bottom().max(last.top() + view.height())
    };
    let mut scroll = egui::ScrollArea::both()
        .id_salt(("pdf-document", preview.path()))
        .auto_shrink(false);
    if let Some(page) = preview.take_jump() {
        let mut y = pages[page].top();
        let doc_size = preview.page_dimensions(page);
        if doc_size.x > 0.0
            && doc_size.y > 0.0
            && let Some(found) = preview.active_match_rect(page)
        {
            let matched = document_rect(found, pages[page], doc_size);
            let max_y = (pages[page].bottom() - view.height()).max(pages[page].top());
            y = (matched.center().y - view.height() * 0.35).clamp(pages[page].top(), max_y);
        }
        scroll = scroll.scroll_offset(vec2((width - view.width()) / 2.0, y));
    }
    ui.scope_builder(UiBuilder::new().max_rect(view), |ui| {
        let output = scroll.show_viewport(ui, |ui, viewport| {
            let (content_rect, content) =
                ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
            let mut geometry = Vec::new();
            for index in visible_pages(&pages, viewport.top(), viewport.height()) {
                let page = pages[index];
                let image = page.translate(
                    content_rect.min.to_vec2() + vec2((width - page.width()) / 2.0, 0.0),
                );
                let doc_size = preview.page_dimensions(index);
                let size = if doc_size.x > 0.0 && doc_size.y > 0.0 {
                    doc_size
                } else {
                    vec2(page.width(), page.height())
                };
                geometry.push(PageGeometry {
                    page: index,
                    rect: image,
                    size,
                });
                let painter = ui.painter().with_clip_rect(image);
                painter.rect_filled(image, 0.0, egui::Color32::WHITE);
                if let Some(texture) = preview.texture(index) {
                    egui::Image::from_texture(texture).paint_at(ui, image);
                } else {
                    let message = preview.page_error(index).unwrap_or("Rendering page…");
                    painter.text(
                        image.center(),
                        egui::Align2::CENTER_CENTER,
                        crate::i18n::gettext(locale, message),
                        theme::regular(14.0),
                        egui::Color32::DARK_GRAY,
                    );
                }
                for (found, active) in preview.match_rects(index) {
                    let screen = document_rect(found, image, size);
                    painter.rect_filled(
                        screen,
                        2.0,
                        if active {
                            egui::Color32::from_rgba_unmultiplied(0xff, 0x98, 0x00, 145)
                        } else {
                            egui::Color32::from_rgba_unmultiplied(0xff, 0xd5, 0x4f, 115)
                        },
                    );
                    if active {
                        painter.rect_stroke(
                            screen,
                            2.0,
                            egui::Stroke::new(1.5, egui::Color32::from_rgb(0xe6, 0x51, 0x00)),
                            egui::StrokeKind::Outside,
                        );
                    }
                }
                let selection_fill = if palette.dark {
                    egui::Color32::from_rgba_unmultiplied(0x3b, 0x82, 0xf6, 86)
                } else {
                    alpha(palette.accent, 86)
                };
                for found in preview.selection_rects(index) {
                    painter.rect_filled(document_rect(found, image, size), 1.0, selection_fill);
                }
                painter.rect_stroke(
                    image,
                    0.0,
                    egui::Stroke::new(1.0, palette.outline),
                    egui::StrokeKind::Inside,
                );
            }
            if content.drag_started() {
                if let Some(origin) = ui.input(|input| input.pointer.press_origin()) {
                    preview.select(origin, &geometry, false);
                    if let Some(pointer) = content.interact_pointer_pos() {
                        preview.select(pointer, &geometry, true);
                    }
                } else if let Some(pointer) = content.interact_pointer_pos() {
                    preview.select(pointer, &geometry, false);
                }
            } else if content.dragged()
                && let Some(pointer) = content.interact_pointer_pos()
            {
                preview.select(pointer, &geometry, true);
            } else if content.clicked() {
                preview.clear_selection();
            }
            if content.hovered()
                && let Some(pointer) = ui.ctx().pointer_latest_pos()
                && preview.hits_text(pointer, &geometry)
            {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
            }
        });
        let top = output.state.offset.y;
        let height = output.inner_rect.height();
        let visible = visible_pages(&pages, top, height);
        // Use the page at the reading edge, so a jump to a short sheet
        // doesn't immediately skip it in favor of a taller sheet below it.
        let current = visible.start.min(pages.len() - 1);
        if preview.scrolled_to(current) {
            ui.ctx().request_repaint();
        }
        preview.request_visible(view.size(), ui.ctx().pixels_per_point(), visible);
    });
}

fn visible_pages(pages: &[Rect], top: f32, height: f32) -> std::ops::Range<usize> {
    pages.partition_point(|page| page.bottom() <= top)
        ..pages.partition_point(|page| page.top() < top + height)
}

fn document_rect(document: Rect, image: Rect, size: egui::Vec2) -> Rect {
    let scale = image.size() / size;
    Rect::from_min_max(
        egui::pos2(
            image.min.x + document.min.x * scale.x,
            image.min.y + document.min.y * scale.y,
        ),
        egui::pos2(
            image.min.x + document.max.x * scale.x,
            image.min.y + document.max.y * scale.y,
        ),
    )
}

fn alpha(color: egui::Color32, opacity: u8) -> egui::Color32 {
    let [red, green, blue, _] = color.to_array();
    egui::Color32::from_rgba_unmultiplied(red, green, blue, opacity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_range_handles_page_edges_and_gaps() {
        let pages = [
            Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(200.0, 300.0)),
            Rect::from_min_size(egui::pos2(0.0, 316.0), vec2(300.0, 200.0)),
            Rect::from_min_size(egui::pos2(0.0, 532.0), vec2(200.0, 300.0)),
        ];
        assert_eq!(visible_pages(&pages, 0.0, 300.0), 0..1);
        assert_eq!(visible_pages(&pages, 290.0, 300.0), 0..3);
        assert_eq!(visible_pages(&pages, 300.0, 16.0), 1..1);
        assert_eq!(visible_pages(&pages, 316.0, 200.0), 1..2);
        assert_eq!(visible_pages(&pages, 532.0, 600.0), 2..3);
    }

    #[test]
    fn document_rect_follows_the_painted_page() {
        let image = Rect::from_min_size(egui::pos2(50.0, 70.0), vec2(400.0, 600.0));
        let document = Rect::from_min_size(egui::pos2(5.0, 10.0), vec2(2.0, 3.0));
        assert_eq!(
            document_rect(document, image, vec2(200.0, 300.0)),
            Rect::from_min_size(egui::pos2(60.0, 90.0), vec2(4.0, 6.0))
        );
    }
}
