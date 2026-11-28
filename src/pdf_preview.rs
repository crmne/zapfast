//! State for the native PDF viewer. Rendering stays on its own worker.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use egui::{Key, Modifiers, TextureHandle};

use crate::model::ChatId;
use crate::pdf::{self, Event, Request, TextGlyph, Worker};

const CACHED_PAGES: usize = 6;
const PAGE_GAP: f32 = 16.0;

/// A glyph in document order. The page comes first, so derived `Ord` is
/// reading order for a document whose glyphs arrived in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextCursor {
    page: usize,
    glyph: usize,
}

/// One contiguous, non-overlapping result of the current query.
#[derive(Clone, Copy, Debug)]
struct SearchMatch {
    page: usize,
    first: usize,
    last: usize,
}

/// Where a page was painted, so a pointer can be mapped back to its document.
#[derive(Clone, Copy, Debug)]
pub struct PageGeometry {
    pub page: usize,
    pub rect: egui::Rect,
    pub size: egui::Vec2,
}

impl PageGeometry {
    pub fn document_point(self, pointer: egui::Pos2) -> Option<egui::Pos2> {
        if !self.rect.contains(pointer) || !self.size.x.is_finite() || !self.size.y.is_finite() {
            return None;
        }
        let scale = self.rect.size() / self.size;
        if scale.x <= 0.0 || scale.y <= 0.0 {
            return None;
        }
        Some(egui::pos2(
            (pointer.x - self.rect.min.x) / scale.x,
            (pointer.y - self.rect.min.y) / scale.y,
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Loading,
    Ready,
    Failed(&'static str),
}

pub struct PreviewState {
    path: PathBuf,
    chat: ChatId,
    worker: Option<Worker>,
    started: bool,
    status: Status,
    pages: Vec<(f32, f32)>,
    page: usize,
    zoom: f32,
    jump: Option<usize>,
    requested: Option<(u64, usize, f32)>,
    rendered: VecDeque<(usize, f32, TextureHandle)>,
    page_errors: HashMap<usize, &'static str>,
    next_request: u64,
    text: Vec<Vec<TextGlyph>>,
    search_open: bool,
    query: String,
    matches: Vec<SearchMatch>,
    active_match: usize,
    selection: Option<(TextCursor, TextCursor)>,
}

impl PreviewState {
    pub fn new(path: PathBuf, chat: ChatId) -> Self {
        Self {
            worker: None,
            started: false,
            path,
            chat,
            status: Status::Loading,
            pages: Vec::new(),
            page: 0,
            zoom: 1.0,
            jump: Some(0),
            requested: None,
            rendered: VecDeque::new(),
            page_errors: HashMap::new(),
            next_request: 0,
            text: Vec::new(),
            search_open: false,
            query: String::new(),
            matches: Vec::new(),
            active_match: 0,
            selection: None,
        }
    }

    pub fn search_id() -> egui::Id {
        egui::Id::new("pdf-search")
    }

    pub fn start(&mut self, ctx: &egui::Context) {
        if self.started {
            return;
        }
        self.started = true;
        match Worker::spawn(&self.path, ctx.clone()) {
            Ok(worker) => self.worker = Some(worker),
            Err(_) => self.status = Status::Failed("The PDF viewer could not be started."),
        }
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        self.start(ctx);
        loop {
            let event = match self.worker.as_ref().map(Worker::next_event) {
                Some(Ok(event)) => event,
                Some(Err(std::sync::mpsc::TryRecvError::Empty)) | None => break,
                Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                    self.worker = None;
                    self.status = Status::Failed("The PDF preview stopped unexpectedly.");
                    break;
                }
            };
            match event {
                Event::Ready(pages) => {
                    self.pages = pages;
                    self.status = Status::Ready;
                }
                Event::Text(text) => self.set_text(text),
                Event::Rendered {
                    request,
                    page,
                    image,
                } => {
                    let current = self
                        .requested
                        .is_some_and(|(id, requested, _)| id == request && requested == page);
                    if current {
                        let scale = self.requested.map_or(1.0, |(_, _, scale)| scale);
                        self.requested = None;
                        self.rendered.retain(|(cached, _, _)| *cached != page);
                        if self.rendered.len() == CACHED_PAGES {
                            self.rendered.pop_front();
                        }
                        self.rendered.push_back((
                            page,
                            scale,
                            ctx.load_texture(
                                format!("pdf-page-{request}"),
                                image,
                                egui::TextureOptions::LINEAR,
                            ),
                        ));
                    }
                }
                Event::Failed { request, message } => {
                    if request.is_none() {
                        self.worker = None;
                        self.status = Status::Failed(message);
                        break;
                    }
                    if let Some((id, page, _)) = self.requested
                        && Some(id) == request
                    {
                        self.requested = None;
                        self.page_errors.insert(page, message);
                    }
                }
            }
        }
    }

    pub fn request_visible(
        &mut self,
        viewport: egui::Vec2,
        pixels_per_point: f32,
        visible: std::ops::Range<usize>,
    ) {
        if self.requested.is_some() {
            return;
        }
        let nearby = [visible.end, visible.start.saturating_sub(1)];
        let candidates: Vec<_> = visible
            .chain(nearby)
            .filter(|&page| page < self.pages.len())
            .take(CACHED_PAGES)
            .collect();
        let Some((page, scale)) = candidates.iter().find_map(|&page| {
            let (width, height) = self.pages[page];
            let scale = pdf::render_scale(
                width,
                height,
                fit_scale((width, height), viewport) * self.zoom * pixels_per_point,
            )?;
            (!self.page_errors.contains_key(&page)
                && !self.rendered.iter().any(|(cached, rendered_scale, _)| {
                    *cached == page && (scale - rendered_scale).abs() < scale * 0.05
                }))
            .then_some((page, scale))
        }) else {
            return;
        };
        // Keep visible pages ahead of old pages in the eviction order.
        self.rendered
            .make_contiguous()
            .sort_by_key(|(page, _, _)| candidates.contains(page));
        self.next_request = self.next_request.wrapping_add(1);
        let request = Request {
            id: self.next_request,
            page,
            scale,
        };
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.request(request))
        {
            self.requested = Some((self.next_request, page, scale));
        }
    }

    pub fn move_page(&mut self, step: i8) {
        let Some(next) = self.page.checked_add_signed(isize::from(step)) else {
            return;
        };
        if next < self.pages.len() {
            self.page = next;
            self.jump = Some(next);
        }
    }

    pub fn zoom_by(&mut self, factor: f32) {
        if factor.is_finite() && factor > 0.0 {
            self.zoom = (self.zoom * factor).clamp(0.25, 4.0);
            self.jump = Some(self.page);
            self.page_errors.clear();
        }
    }

    pub fn fit(&mut self) {
        self.zoom = 1.0;
        self.jump = Some(self.page);
        self.page_errors.clear();
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn chat(&self) -> &str {
        &self.chat
    }

    pub fn page(&self) -> usize {
        self.page
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn page_dimensions(&self, page: usize) -> egui::Vec2 {
        self.pages
            .get(page)
            .map_or(egui::Vec2::ZERO, |&(width, height)| {
                egui::vec2(width, height)
            })
    }

    pub fn layout(&self, viewport: egui::Vec2) -> Vec<egui::Rect> {
        let mut top = 0.0;
        self.pages
            .iter()
            .map(|&(width, height)| {
                let size =
                    egui::vec2(width, height) * (fit_scale((width, height), viewport) * self.zoom);
                let rect = egui::Rect::from_min_size(egui::pos2(0.0, top), size);
                top += size.y + PAGE_GAP;
                rect
            })
            .collect()
    }

    pub fn take_jump(&mut self) -> Option<usize> {
        self.jump.take()
    }

    pub fn scrolled_to(&mut self, page: usize) -> bool {
        let changed = self.page != page;
        self.page = page;
        changed
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    pub fn status(&self) -> Status {
        self.status
    }

    pub fn page_error(&self, page: usize) -> Option<&'static str> {
        self.page_errors.get(&page).copied()
    }

    pub fn texture(&self, page: usize) -> Option<&TextureHandle> {
        self.rendered
            .iter()
            .find(|(cached, _, _)| *cached == page)
            .map(|(_, _, texture)| texture)
    }

    pub fn set_text(&mut self, text: Vec<Vec<TextGlyph>>) {
        if self.text == text {
            return;
        }
        self.text = text;
        self.selection = None;
        self.recompute_matches();
    }

    pub fn search_open(&self) -> bool {
        self.search_open
    }

    pub fn open_search(&mut self) {
        self.search_open = true;
    }

    pub fn close_search(&mut self) {
        self.search_open = false;
        self.query.clear();
        self.recompute_matches();
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn set_query(&mut self, query: String) {
        if !query.is_empty() {
            self.search_open = true;
        }
        self.query = query;
        self.recompute_matches();
        if let Some(first) = self.matches.first() {
            self.jump = Some(first.page);
        }
    }

    pub fn match_count(&self) -> usize {
        self.matches.len()
    }

    /// One-based for display. Zero means no matches.
    pub fn active_match(&self) -> usize {
        if self.matches.is_empty() {
            0
        } else {
            self.active_match + 1
        }
    }

    pub fn step_match(&mut self, step: i8) {
        if self.matches.is_empty() {
            return;
        }
        let count = self.matches.len() as i64;
        let next = (self.active_match as i64 + i64::from(step)).rem_euclid(count);
        self.active_match = next as usize;
        self.jump = Some(self.matches[next as usize].page);
    }

    pub fn has_selection(&self) -> bool {
        self.selection.is_some()
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    pub fn selection_text(&self) -> Option<String> {
        let (start, end) = self.normalized_selection()?;
        let mut text = String::new();
        for page in start.page..=end.page {
            let glyphs = self.text.get(page)?;
            if glyphs.is_empty() {
                continue;
            }
            if page > start.page && !text.is_empty() {
                text.push('\n');
            }
            let first = if page == start.page { start.glyph } else { 0 };
            let last = if page == end.page {
                end.glyph.min(glyphs.len().saturating_sub(1))
            } else {
                glyphs.len().saturating_sub(1)
            };
            if first > last {
                continue;
            }
            let mut prev: Option<&TextGlyph> = None;
            for glyph in glyphs[first..=last].iter() {
                if let Some(previous) = prev {
                    let line_height = previous.rect.height().max(glyph.rect.height());
                    if line_height > 0.0
                        && (glyph.origin.1 - previous.origin.1).abs() > line_height * 0.55
                        && !text.ends_with('\n')
                    {
                        text.push('\n');
                    }
                }
                text.push_str(&glyph.text);
                prev = Some(glyph);
            }
        }
        Some(text)
    }

    pub fn hits_text(&self, pointer: egui::Pos2, pages: &[PageGeometry]) -> bool {
        self.exact_cursor(pointer, pages).is_some()
    }

    fn exact_cursor(&self, pointer: egui::Pos2, pages: &[PageGeometry]) -> Option<TextCursor> {
        pages.iter().find_map(|geometry| {
            let document = geometry.document_point(pointer)?;
            let glyphs = self.text.get(geometry.page)?;
            glyphs
                .iter()
                .enumerate()
                .rev()
                .find(|(_, glyph)| glyph.rect.contains(document))
                .map(|(glyph, _)| TextCursor {
                    page: geometry.page,
                    glyph,
                })
        })
    }

    fn nearest_cursor(&self, pointer: egui::Pos2, pages: &[PageGeometry]) -> Option<TextCursor> {
        pages.iter().find_map(|geometry| {
            let document = geometry.document_point(pointer)?;
            let glyphs = self.text.get(geometry.page)?;
            glyphs
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| {
                    glyph_distance(a.rect, document).total_cmp(&glyph_distance(b.rect, document))
                })
                .map(|(glyph, _)| TextCursor {
                    page: geometry.page,
                    glyph,
                })
        })
    }

    /// Starts or extends a selection from a screen pointer. Returns whether
    /// the pointer landed on extractable text, so the UI can claim the drag.
    pub fn select(&mut self, pointer: egui::Pos2, pages: &[PageGeometry], extend: bool) -> bool {
        let cursor = self.exact_cursor(pointer, pages).or_else(|| {
            (extend && self.selection.is_some())
                .then(|| self.nearest_cursor(pointer, pages))
                .flatten()
        });
        let Some(cursor) = cursor else {
            // A fresh press on empty paper ends the old selection; extending
            // past the last glyph keeps it.
            if !extend {
                self.selection = None;
            }
            return false;
        };
        if extend && let Some((anchor, _)) = self.selection {
            self.selection = Some((anchor, cursor));
        } else {
            self.selection = Some((cursor, cursor));
        }
        true
    }

    pub fn active_match_rect(&self, page: usize) -> Option<egui::Rect> {
        let found = self.matches.get(self.active_match)?;
        if found.page != page {
            return None;
        }
        let glyphs = self.text.get(page)?;
        Some(union_rects(glyphs, found.first, found.last))
    }

    /// Inactive first, active last, so the UI paints the current result over
    /// the others.
    pub fn match_rects(&self, page: usize) -> Vec<(egui::Rect, bool)> {
        let Some(glyphs) = self.text.get(page) else {
            return Vec::new();
        };
        let mut inactive = Vec::new();
        let mut active = None;
        for (index, found) in self.matches.iter().enumerate() {
            if found.page != page {
                continue;
            }
            let rect = union_rects(glyphs, found.first, found.last);
            if index == self.active_match {
                active = Some(rect);
            } else {
                inactive.push((rect, false));
            }
        }
        if let Some(rect) = active {
            inactive.push((rect, true));
        }
        inactive
    }

    pub fn selection_rects(&self, page: usize) -> Vec<egui::Rect> {
        let Some((start, end)) = self.normalized_selection() else {
            return Vec::new();
        };
        if page < start.page || page > end.page {
            return Vec::new();
        }
        let Some(glyphs) = self.text.get(page) else {
            return Vec::new();
        };
        if glyphs.is_empty() {
            return Vec::new();
        }
        let first = if page == start.page {
            start.glyph.min(glyphs.len() - 1)
        } else {
            0
        };
        let last = if page == end.page {
            end.glyph.min(glyphs.len() - 1)
        } else {
            glyphs.len() - 1
        };
        if first > last {
            return Vec::new();
        }
        glyphs[first..=last]
            .iter()
            .map(|glyph| glyph.rect)
            .collect()
    }

    fn normalized_selection(&self) -> Option<(TextCursor, TextCursor)> {
        self.selection.map(|(first, second)| {
            if first <= second {
                (first, second)
            } else {
                (second, first)
            }
        })
    }

    fn recompute_matches(&mut self) {
        self.matches.clear();
        self.active_match = 0;
        let needle: String = self.query.chars().flat_map(char::to_lowercase).collect();
        if needle.is_empty() {
            return;
        }
        for (page, glyphs) in self.text.iter().enumerate() {
            let mut folded = String::new();
            let mut owners = Vec::new();
            for (index, glyph) in glyphs.iter().enumerate() {
                for c in glyph.text.chars().flat_map(char::to_lowercase) {
                    // `String::find` returns a byte offset, so every UTF-8
                    // byte needs an owner, not every char.
                    let mut encoded = [0; 4];
                    for _ in c.encode_utf8(&mut encoded).as_bytes() {
                        owners.push(index);
                    }
                    folded.push(c);
                }
            }
            let mut from = 0;
            while let Some(offset) = folded[from..].find(&needle) {
                let start = from + offset;
                let end = start + needle.len();
                let (Some(&first), Some(&last)) = (owners.get(start), owners.get(end - 1)) else {
                    break;
                };
                self.matches.push(SearchMatch { page, first, last });
                from = end;
            }
        }
    }
}

fn glyph_distance(rect: egui::Rect, point: egui::Pos2) -> f32 {
    let dx = if point.x < rect.min.x {
        rect.min.x - point.x
    } else if point.x > rect.max.x {
        point.x - rect.max.x
    } else {
        0.0
    };
    let dy = if point.y < rect.min.y {
        rect.min.y - point.y
    } else if point.y > rect.max.y {
        point.y - rect.max.y
    } else {
        0.0
    };
    // Prefer glyphs on the same line before jumping lines.
    dy * 4.0 + dx
}

fn union_rects(glyphs: &[TextGlyph], first: usize, last: usize) -> egui::Rect {
    let mut rect = glyphs[first].rect;
    for glyph in glyphs.iter().take(last + 1).skip(first + 1) {
        rect.min.x = rect.min.x.min(glyph.rect.min.x);
        rect.min.y = rect.min.y.min(glyph.rect.min.y);
        rect.max.x = rect.max.x.max(glyph.rect.max.x);
        rect.max.y = rect.max.y.max(glyph.rect.max.y);
    }
    rect
}

fn fit_scale(page: (f32, f32), viewport: egui::Vec2) -> f32 {
    (viewport.x / page.0)
        .min(viewport.y / page.1)
        .clamp(0.0001, 2.0)
}

pub fn key_action(key: Key, modifiers: Modifiers) -> Option<crate::model::Action> {
    let command = modifiers.command || modifiers.ctrl;
    match (command, key) {
        (true, Key::F) if !modifiers.alt && !modifiers.shift => Some(crate::model::Action::PdfFind),
        (true, Key::C) if !modifiers.alt && !modifiers.shift => {
            Some(crate::model::Action::PdfCopySelection)
        }
        (true, Key::Plus) | (true, Key::Equals) => Some(crate::model::Action::ZoomPdfBy(1.25)),
        (true, Key::Minus) => Some(crate::model::Action::ZoomPdfBy(0.8)),
        (false, Key::Enter) if !modifiers.any() => Some(crate::model::Action::PdfMatch(1)),
        (false, Key::Enter) if modifiers.shift && !modifiers.alt && !command => {
            Some(crate::model::Action::PdfMatch(-1))
        }
        (false, Key::F3) if !modifiers.alt && !command && !modifiers.shift => {
            Some(crate::model::Action::PdfMatch(1))
        }
        (false, Key::F3) if modifiers.shift && !modifiers.alt && !command => {
            Some(crate::model::Action::PdfMatch(-1))
        }
        (false, Key::Plus) | (false, Key::Equals) if !modifiers.any() => {
            Some(crate::model::Action::ZoomPdfBy(1.25))
        }
        (false, Key::Minus) if !modifiers.any() => Some(crate::model::Action::ZoomPdfBy(0.8)),
        (false, Key::Num0) if !modifiers.any() => Some(crate::model::Action::FitPdf),
        (false, Key::ArrowLeft | Key::PageUp) if !modifiers.any() => {
            Some(crate::model::Action::PdfPage(-1))
        }
        (false, Key::ArrowRight | Key::PageDown) if !modifiers.any() => {
            Some(crate::model::Action::PdfPage(1))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    #[test]
    fn navigation_and_zoom_stay_in_bounds() {
        let path = PathBuf::from("fixture.pdf");
        let mut state = PreviewState {
            path,
            chat: "chat".into(),
            worker: None,
            started: true,
            status: Status::Ready,
            pages: vec![(200.0, 300.0), (300.0, 200.0)],
            page: 0,
            zoom: 1.0,
            jump: None,
            requested: None,
            rendered: VecDeque::new(),
            page_errors: HashMap::new(),
            next_request: 0,
            text: Vec::new(),
            search_open: false,
            query: String::new(),
            matches: Vec::new(),
            active_match: 0,
            selection: None,
        };
        state.move_page(-1);
        assert_eq!(state.page(), 0);
        state.move_page(1);
        assert_eq!(state.page(), 1);
        assert_eq!(state.take_jump(), Some(1));
        state.move_page(1);
        assert_eq!(state.page(), 1);
        state.zoom_by(2.0);
        assert_eq!(state.zoom(), 2.0);
        state.zoom_by(f32::INFINITY);
        assert_eq!(state.zoom(), 2.0);
        state.fit();
        assert_eq!(state.zoom(), 1.0);
    }

    #[test]
    fn preview_keys_only_map_navigation_and_zoom() {
        assert_eq!(
            key_action(Key::ArrowRight, Modifiers::NONE),
            Some(crate::model::Action::PdfPage(1))
        );
        assert_eq!(
            key_action(Key::PageUp, Modifiers::NONE),
            Some(crate::model::Action::PdfPage(-1))
        );
        assert_eq!(
            key_action(Key::Num0, Modifiers::NONE),
            Some(crate::model::Action::FitPdf)
        );
        assert_eq!(key_action(Key::ArrowRight, Modifiers::SHIFT), None);
    }

    fn test_state() -> PreviewState {
        let mut state = PreviewState::new(PathBuf::from("fixture.pdf"), "chat".into());
        state.started = true;
        state.status = Status::Ready;
        state
    }

    fn glyph(page: usize, order: usize, text: &str, x: f32) -> TextGlyph {
        TextGlyph {
            page,
            order,
            text: text.into(),
            origin: (x, 16.0),
            rect: egui::Rect::from_min_max(egui::pos2(x, 0.0), egui::pos2(x + 10.0, 20.0)),
        }
    }

    #[test]
    fn unicode_search_finds_every_match_in_reading_order() {
        let mut state = test_state();
        state.set_text(vec![
            vec![glyph(0, 0, "Café", 0.0), glyph(0, 1, " notes", 10.0)],
            vec![glyph(1, 0, "CAFÉ", 0.0)],
        ]);
        state.set_query("café".into());
        assert_eq!(state.match_count(), 2);
        assert_eq!(state.active_match(), 1);
        assert_eq!(state.match_rects(0).len(), 1);
        assert_eq!(state.match_rects(1).len(), 1);
        state.step_match(1);
        assert_eq!(state.active_match(), 2);
        assert_eq!(state.take_jump(), Some(1));
        state.step_match(1);
        assert_eq!(state.active_match(), 1);
        assert_eq!(state.take_jump(), Some(0));
        state.step_match(-1);
        assert_eq!(state.active_match(), 2);
        assert_eq!(state.take_jump(), Some(1));
        state.set_query(String::new());
        assert_eq!(state.match_count(), 0);
        assert!(state.match_rects(0).is_empty());
    }

    #[test]
    fn drag_selection_copies_text_and_survives_reverse_drag() {
        let mut state = test_state();
        state.set_text(vec![
            vec![glyph(0, 0, "Find", 0.0), glyph(0, 1, " me", 10.0)],
            vec![glyph(1, 0, " next", 0.0)],
        ]);
        let pages = [PageGeometry {
            page: 0,
            rect: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(200.0, 300.0)),
            size: vec2(200.0, 300.0),
        }];
        assert!(state.select(egui::pos2(5.0, 5.0), &pages, false));
        assert!(state.select(egui::pos2(15.0, 5.0), &pages, true));
        assert_eq!(state.selection_text().as_deref(), Some("Find me"));
        assert_eq!(state.selection_rects(0).len(), 2);
        assert!(state.has_selection());
        assert!(state.select(egui::pos2(15.0, 5.0), &pages, false));
        assert!(state.select(egui::pos2(5.0, 5.0), &pages, true));
        assert_eq!(state.selection_text().as_deref(), Some("Find me"));
        assert_eq!(state.selection_rects(0).len(), 2);
        assert!(!state.select(egui::pos2(150.0, 150.0), &pages, false));
        assert!(!state.has_selection());
        state.clear_selection();
        assert!(!state.has_selection());
    }

    #[test]
    fn selection_across_an_empty_page_skips_it() {
        let mut state = test_state();
        state.set_text(vec![
            vec![glyph(0, 0, "A", 0.0)],
            Vec::new(),
            vec![glyph(2, 0, "B", 0.0)],
        ]);
        let pages = [
            PageGeometry {
                page: 0,
                rect: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(200.0, 300.0)),
                size: vec2(200.0, 300.0),
            },
            PageGeometry {
                page: 2,
                rect: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(200.0, 300.0)),
                size: vec2(200.0, 300.0),
            },
        ];
        assert!(state.select(egui::pos2(5.0, 5.0), &pages, false));
        assert!(state.select(egui::pos2(5.0, 5.0), &pages[1..], true));
        assert_eq!(state.selection_text().as_deref(), Some("A\nB"));
    }

    #[test]
    fn pdf_keys_map_find_navigation_and_copy() {
        let ctrl = Modifiers {
            ctrl: true,
            ..Modifiers::NONE
        };
        let shift = Modifiers {
            shift: true,
            ..Modifiers::NONE
        };
        assert_eq!(
            key_action(Key::F, ctrl),
            Some(crate::model::Action::PdfFind)
        );
        assert_eq!(
            key_action(Key::C, ctrl),
            Some(crate::model::Action::PdfCopySelection)
        );
        assert_eq!(
            key_action(Key::F3, Modifiers::NONE),
            Some(crate::model::Action::PdfMatch(1))
        );
        assert_eq!(
            key_action(Key::F3, shift),
            Some(crate::model::Action::PdfMatch(-1))
        );
        assert_eq!(key_action(Key::F, Modifiers::NONE), None);
    }

    #[test]
    fn scaled_geometry_maps_screen_points_and_multiline_selection() {
        let mut state = test_state();
        state.pages = vec![(200.0, 300.0)];
        let line_two = TextGlyph {
            page: 0,
            order: 1,
            text: "Second".into(),
            origin: (10.0, 46.0),
            rect: egui::Rect::from_min_max(egui::pos2(10.0, 30.0), egui::pos2(50.0, 50.0)),
        };
        state.set_text(vec![vec![glyph(0, 0, "First", 10.0), line_two]]);
        let pages = [PageGeometry {
            page: 0,
            rect: egui::Rect::from_min_size(egui::pos2(50.0, 20.0), vec2(400.0, 600.0)),
            size: state.page_dimensions(0),
        }];
        // Screen point (80, 30) maps to document (15, 5), inside "First".
        assert!(state.hits_text(egui::pos2(80.0, 30.0), &pages));
        assert!(state.select(egui::pos2(80.0, 30.0), &pages, false));
        // Extending into whitespace past the end of line 2 picks "Second".
        assert!(state.select(egui::pos2(300.0, 100.0), &pages, true));
        assert_eq!(state.selection_text().as_deref(), Some("First\nSecond"));
    }
}
