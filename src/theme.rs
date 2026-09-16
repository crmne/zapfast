//! Colors, typography, icons, and base widgets.
//!
//! The UI uses Inter's font weights and Lucide icons. [`Palette`] holds all
//! light and dark theme colors.

use egui::{Color32, CornerRadius, Response, Sense, Stroke, Vec2};

pub mod custom;
#[cfg(target_os = "linux")]
mod omarchy;
pub(crate) mod presets;
#[cfg(target_os = "linux")]
mod watch;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Palette {
    pub dark: bool,
    pub window: Color32,
    pub panel: Color32,
    pub surface: Color32,
    pub surface_hover: Color32,
    pub surface_active: Color32,
    pub outline: Color32,
    pub text: Color32,
    pub secondary: Color32,
    pub dim: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub on_accent: Color32,
    pub danger: Color32,
    pub warning: Color32,
    pub overlay: Color32,
    pub shadow: Color32,
    /// Conversation background behind message bubbles.
    pub chat: Color32,
    /// Incoming message bubble.
    pub bubble_in: Color32,
    /// Outgoing message bubble.
    pub bubble_out: Color32,
    pub link: Color32,
    /// Read-receipt blue.
    pub read: Color32,
}

impl Palette {
    pub fn dark() -> Self {
        Self {
            dark: true,
            window: Color32::from_rgb(0x0b, 0x14, 0x1a),
            panel: Color32::from_rgb(0x11, 0x1b, 0x21),
            surface: Color32::from_rgb(0x20, 0x2c, 0x33),
            surface_hover: Color32::from_rgb(0x2a, 0x39, 0x42),
            surface_active: Color32::from_rgb(0x35, 0x44, 0x4d),
            outline: Color32::from_rgb(0x22, 0x2d, 0x34),
            text: Color32::from_rgb(0xe9, 0xed, 0xef),
            secondary: Color32::from_rgb(0x86, 0x96, 0xa0),
            dim: Color32::from_rgb(0x66, 0x77, 0x81),
            accent: Color32::from_rgb(0x00, 0xa8, 0x84),
            accent_hover: Color32::from_rgb(0x06, 0xcf, 0x9c),
            on_accent: Color32::from_rgb(0x0b, 0x14, 0x1a),
            danger: Color32::from_rgb(0xf1, 0x5c, 0x6d),
            warning: Color32::from_rgb(0xff, 0xd2, 0x79),
            overlay: Color32::from_rgb(0x23, 0x31, 0x38),
            shadow: Color32::from_black_alpha(140),
            chat: Color32::from_rgb(0x0b, 0x14, 0x1a),
            bubble_in: Color32::from_rgb(0x20, 0x2c, 0x33),
            bubble_out: Color32::from_rgb(0x00, 0x5c, 0x4b),
            link: Color32::from_rgb(0x53, 0xbd, 0xeb),
            read: Color32::from_rgb(0x53, 0xbd, 0xeb),
        }
    }

    pub fn light() -> Self {
        Self {
            dark: false,
            window: Color32::from_rgb(0xf0, 0xf2, 0xf5),
            panel: Color32::from_rgb(0xff, 0xff, 0xff),
            surface: Color32::from_rgb(0xf0, 0xf2, 0xf5),
            surface_hover: Color32::from_rgb(0xe6, 0xe9, 0xec),
            surface_active: Color32::from_rgb(0xd9, 0xdd, 0xe1),
            outline: Color32::from_rgb(0xe9, 0xed, 0xef),
            text: Color32::from_rgb(0x11, 0x1b, 0x21),
            secondary: Color32::from_rgb(0x66, 0x77, 0x81),
            dim: Color32::from_rgb(0x8f, 0x9c, 0xa5),
            accent: Color32::from_rgb(0x00, 0xa8, 0x84),
            accent_hover: Color32::from_rgb(0x00, 0x8f, 0x6f),
            on_accent: Color32::WHITE,
            danger: Color32::from_rgb(0xea, 0x00, 0x38),
            warning: Color32::from_rgb(0xa0, 0x6b, 0x00),
            overlay: Color32::from_rgb(0xff, 0xff, 0xff),
            shadow: Color32::from_black_alpha(50),
            chat: Color32::from_rgb(0xef, 0xea, 0xe2),
            bubble_in: Color32::from_rgb(0xff, 0xff, 0xff),
            bubble_out: Color32::from_rgb(0xd9, 0xfd, 0xd3),
            link: Color32::from_rgb(0x02, 0x7e, 0xb5),
            read: Color32::from_rgb(0x53, 0xbd, 0xeb),
        }
    }

    /// Group-sender color derived from the avatar hue.
    pub fn sender(&self, hue: f32) -> Color32 {
        if self.dark {
            hsl(hue, 0.6, 0.68)
        } else {
            hsl(hue, 0.65, 0.38)
        }
    }

    /// Avatar background derived from the chat hue.
    pub fn avatar(&self, hue: f32) -> Color32 {
        let (saturation, lightness) = if self.dark {
            (0.38, 0.42)
        } else {
            (0.45, 0.62)
        };
        hsl(hue, saturation, lightness)
    }
}

/// Converts HSL to color bytes for non-egui drawing.
pub fn hsl_rgb(hue: f32, saturation: f32, lightness: f32) -> [u8; 3] {
    let color = hsl(hue, saturation, lightness);
    [color.r(), color.g(), color.b()]
}

fn hsl(hue: f32, saturation: f32, lightness: f32) -> Color32 {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let sector = hue / 60.0;
    let x = chroma * (1.0 - (sector % 2.0 - 1.0).abs());
    let (r, g, b) = match sector as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = lightness - chroma / 2.0;
    let channel = |value: f32| ((value + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(channel(r), channel(g), channel(b))
}

pub const RADIUS: u8 = 8;
pub const RADIUS_SMALL: u8 = 4;
pub const ROW_HEIGHT: f32 = 68.0;
pub const TOP_BAR_HEIGHT: f32 = 60.0;

const INTER_MEDIUM: &str = "inter-medium";
const INTER_SEMIBOLD: &str = "inter-semibold";
const INTER_BOLD: &str = "inter-bold";

pub fn regular(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Proportional)
}

pub fn medium(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(INTER_MEDIUM.into()))
}

pub fn semibold(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(INTER_SEMIBOLD.into()))
}

pub fn bold(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(INTER_BOLD.into()))
}

/// Installs fonts, icons, and base style.
pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    register_icons(ctx);
    egui_extras::install_image_loaders(ctx);
}

/// Applies the palette to egui widgets.
pub fn apply(ctx: &egui::Context, palette: &Palette) {
    let mut style = (*ctx.global_style()).clone();
    let visuals = &mut style.visuals;
    *visuals = if palette.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.dark_mode = palette.dark;
    visuals.panel_fill = palette.panel;
    visuals.window_fill = palette.overlay;
    visuals.extreme_bg_color = palette.surface;
    visuals.faint_bg_color = palette.surface;
    visuals.code_bg_color = palette.surface;
    visuals.override_text_color = Some(palette.text);
    visuals.weak_text_color = Some(palette.secondary);
    visuals.hyperlink_color = palette.link;
    visuals.selection.bg_fill = palette.accent.gamma_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, palette.accent);
    visuals.window_stroke = Stroke::new(1.0, palette.outline);
    visuals.window_corner_radius = CornerRadius::same(RADIUS + 2);
    visuals.menu_corner_radius = CornerRadius::same(RADIUS);
    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 6],
        blur: 24,
        spread: 0,
        color: palette.shadow,
    };
    visuals.popup_shadow = egui::epaint::Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: palette.shadow,
    };
    let corner = CornerRadius::same(RADIUS_SMALL + 2);
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = corner;
        widget.bg_stroke = Stroke::NONE;
        widget.fg_stroke = Stroke::new(1.0, palette.text);
        widget.expansion = 0.0;
    }
    visuals.widgets.noninteractive.corner_radius = corner;
    visuals.widgets.noninteractive.bg_fill = palette.panel;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.outline);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette.text);
    visuals.widgets.inactive.bg_fill = palette.surface;
    visuals.widgets.inactive.weak_bg_fill = palette.surface;
    visuals.widgets.hovered.bg_fill = palette.surface_hover;
    visuals.widgets.hovered.weak_bg_fill = palette.surface_hover;
    visuals.widgets.active.bg_fill = palette.surface_active;
    visuals.widgets.active.weak_bg_fill = palette.surface_active;
    visuals.widgets.open.bg_fill = palette.surface_hover;
    visuals.widgets.open.weak_bg_fill = palette.surface_hover;
    visuals.text_cursor.stroke = Stroke::new(2.0, palette.accent);
    visuals.striped = false;
    visuals.slider_trailing_fill = true;
    visuals.handle_shape = egui::style::HandleShape::Circle;

    use egui::FontFamily::{Monospace, Proportional};
    use egui::{FontId, TextStyle};
    style.text_styles = [
        (TextStyle::Small, FontId::new(11.5, Proportional)),
        (TextStyle::Body, FontId::new(14.0, Proportional)),
        (TextStyle::Button, FontId::new(14.0, Proportional)),
        (TextStyle::Heading, FontId::new(22.0, Proportional)),
        (TextStyle::Monospace, FontId::new(13.0, Monospace)),
    ]
    .into();
    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(12.0, 6.0);
    style.spacing.interact_size = Vec2::new(40.0, 28.0);
    style.spacing.menu_margin = egui::Margin::same(6);
    style.spacing.window_margin = egui::Margin::same(16);
    style.spacing.scroll = egui::style::ScrollStyle {
        bar_width: 8.0,
        floating_width: 6.0,
        floating_allocated_width: 0.0,
        handle_min_length: 28.0,
        bar_inner_margin: 3.0,
        bar_outer_margin: 2.0,
        dormant_background_opacity: 0.0,
        dormant_handle_opacity: 0.0,
        active_background_opacity: 0.0,
        active_handle_opacity: 0.55,
        interact_handle_opacity: 0.85,
        foreground_color: true,
        ..egui::style::ScrollStyle::floating()
    };
    style.interaction.selectable_labels = false;
    style.interaction.tooltip_delay = 0.4;
    style.animation_time = 0.12;
    style.url_in_tooltip = false;
    ctx.set_global_style(style);
}

fn install_fonts(ctx: &egui::Context) {
    use egui::epaint::text::VariationCoords;
    use egui::{FontData, FontDefinitions, FontFamily};
    use std::sync::Arc;

    let mut fonts = FontDefinitions::default();
    let inter = include_bytes!("../assets/fonts/InterVariable.ttf");
    let weighted = |weight: f32| {
        let mut data = FontData::from_static(inter);
        data.tweak.coords = VariationCoords::new([(b"wght", weight)]);
        Arc::new(data)
    };
    fonts.font_data.insert("inter".to_owned(), weighted(400.0));
    fonts
        .font_data
        .insert(INTER_MEDIUM.to_owned(), weighted(500.0));
    fonts
        .font_data
        .insert(INTER_SEMIBOLD.to_owned(), weighted(600.0));
    fonts
        .font_data
        .insert(INTER_BOLD.to_owned(), weighted(700.0));

    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "inter".to_owned());
    let fallbacks: Vec<String> = fonts.families[&FontFamily::Proportional]
        .iter()
        .skip(1)
        .cloned()
        .collect();
    for name in [INTER_MEDIUM, INTER_SEMIBOLD, INTER_BOLD] {
        let mut family = vec![name.to_owned()];
        family.extend(fallbacks.iter().cloned());
        fonts.families.insert(FontFamily::Name(name.into()), family);
    }

    // Append system fallbacks after Inter and emoji fonts.
    for font in crate::system_fonts::fallbacks() {
        let mut data = FontData::from_static(&font.bytes);
        data.index = font.index;
        fonts.font_data.insert(font.name.clone(), Arc::new(data));
        for family in fonts.families.values_mut() {
            family.push(font.name.clone());
        }
    }

    ctx.set_fonts(fonts);
}

macro_rules! icons {
    ($($variant:ident => $file:literal),* $(,)?) => {
        &[$((
            Icon::$variant,
            concat!("bytes://zapfast-icon-", $file, ".svg"),
            include_bytes!(concat!("../assets/icons/", $file, ".svg")).as_slice(),
        )),*]
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Icon {
    Archive,
    ArrowDown,
    ArrowLeft,
    Ban,
    Bell,
    BellOff,
    Check,
    CheckCheck,
    ChevronDown,
    ChevronLeft,
    ChevronRight,
    ChevronUp,
    ChartBar,
    CircleAlert,
    CircleCheck,
    CircleX,
    Clock,
    Contact,
    Copy,
    Download,
    DisappearingMessages,
    Ellipsis,
    ExternalLink,
    Eye,
    EyeOff,
    FileText,
    Forward,
    Gif,
    Image,
    Info,
    Keyboard,
    Lock,
    LogOut,
    MapPin,
    Maximize,
    MessageCircle,
    Mic,
    Minimize,
    Minus,
    Monitor,
    Moon,
    PanelLeft,
    Paperclip,
    Pause,
    Pencil,
    Phone,
    Pin,
    PinOff,
    Play,
    Plus,
    QrCode,
    Refresh,
    Reply,
    Search,
    Send,
    Settings,
    Smartphone,
    Smile,
    SquarePen,
    Sticker,
    Sun,
    Trash,
    User,
    Users,
    Video,
    VolumeX,
    WifiOff,
    X,
}

const ICONS: &[(Icon, &str, &[u8])] = icons! {
    Archive => "archive",
    ArrowDown => "arrow-down",
    ArrowLeft => "arrow-left",
    Ban => "ban",
    Bell => "bell",
    BellOff => "bell-off",
    Check => "check",
    CheckCheck => "check-check",
    ChevronDown => "chevron-down",
    ChevronLeft => "chevron-left",
    ChevronRight => "chevron-right",
    ChevronUp => "chevron-up",
    ChartBar => "chart-bar",
    CircleAlert => "circle-alert",
    CircleCheck => "circle-check",
    CircleX => "circle-x",
    Clock => "clock",
    Contact => "contact",
    Copy => "copy",
    Download => "download",
    DisappearingMessages => "disappearing-messages",
    Ellipsis => "ellipsis",
    ExternalLink => "external-link",
    Eye => "eye",
    EyeOff => "eye-off",
    FileText => "file-text",
    Forward => "forward",
    Gif => "gif",
    Image => "image",
    Info => "info",
    Keyboard => "keyboard",
    Lock => "lock",
    LogOut => "log-out",
    MapPin => "map-pin",
    Maximize => "maximize-2",
    MessageCircle => "message-circle",
    Mic => "mic",
    Minimize => "minimize-2",
    Minus => "minus",
    Monitor => "monitor",
    Moon => "moon",
    PanelLeft => "panel-left",
    Paperclip => "paperclip",
    Pause => "pause",
    Pencil => "pencil",
    Phone => "phone",
    Pin => "pin",
    PinOff => "pin-off",
    Play => "play",
    Plus => "plus",
    QrCode => "qr-code",
    Refresh => "refresh-cw",
    Reply => "reply",
    Search => "search",
    Send => "send",
    Settings => "settings",
    Smartphone => "smartphone",
    Smile => "smile",
    SquarePen => "square-pen",
    Sticker => "sticker",
    Sun => "sun",
    Trash => "trash-2",
    User => "user",
    Users => "users",
    Video => "video",
    VolumeX => "volume-x",
    WifiOff => "wifi-off",
    X => "x",
};

impl Icon {
    pub fn uri(self) -> &'static str {
        ICONS
            .iter()
            .find(|(icon, _, _)| *icon == self)
            .map_or("", |(_, uri, _)| *uri)
    }

    pub fn image(self, color: Color32, size: f32) -> egui::Image<'static> {
        egui::Image::new(self.uri())
            .tint(color)
            .fit_to_exact_size(Vec2::splat(size))
    }
}

fn register_icons(ctx: &egui::Context) {
    for (_, uri, bytes) in ICONS {
        ctx.include_bytes(*uri, *bytes);
    }
}

/// A static icon.
pub fn icon(ui: &mut egui::Ui, icon: Icon, size: f32, color: Color32) -> Response {
    ui.add(icon.image(color, size))
}

/// Paints a centered icon in `rect` without allocating space.
pub fn paint_icon(ui: &egui::Ui, icon: Icon, rect: egui::Rect, size: f32, color: Color32) {
    let icon_rect = egui::Rect::from_center_size(rect.center(), Vec2::splat(size));
    icon.image(color, size).paint_at(ui, icon_rect);
}

/// Frameless icon button with hover color.
pub fn icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    size: f32,
    color: Color32,
    hover: Color32,
    tooltip: &str,
) -> Response {
    let edge = size + 12.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(edge), Sense::click());
    if ui.is_rect_visible(rect) {
        let tint = if response.hovered() || response.has_focus() {
            hover
        } else {
            color
        };
        let scale = if response.is_pointer_button_down_on() {
            0.92
        } else {
            1.0
        };
        paint_icon(ui, icon, rect, size * scale, tint);
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if tooltip.is_empty() {
        response
    } else {
        response.on_hover_text(tooltip)
    }
}

/// Round filled icon button.
pub fn circle_button(
    ui: &mut egui::Ui,
    icon: Icon,
    diameter: f32,
    fill: Color32,
    fill_hover: Color32,
    icon_color: Color32,
    tooltip: &str,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(diameter), Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let grow = if hovered { 1.05 } else { 1.0 };
        let radius = diameter / 2.0 * grow;
        let fill = if hovered { fill_hover } else { fill };
        ui.painter().circle_filled(rect.center(), radius, fill);
        let icon_size = diameter * 0.46;
        let icon_rect = egui::Rect::from_center_size(rect.center(), Vec2::splat(icon_size));
        icon.image(icon_color, icon_size).paint_at(ui, icon_rect);
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if tooltip.is_empty() {
        response
    } else {
        response.on_hover_text(tooltip)
    }
}

/// Draws the app logo.
pub fn logo(ui: &egui::Ui, center: egui::Pos2, diameter: f32, disc: Color32, glyph: Color32) {
    ui.painter().circle_filled(center, diameter / 2.0, disc);
    // Match `packaging/icons/zapfast.svg`.
    let icon_size = diameter * 0.56;
    let icon_rect = egui::Rect::from_center_size(
        center - Vec2::new(0.0, diameter * 0.02),
        Vec2::splat(icon_size),
    );
    Icon::MessageCircle
        .image(glyph, icon_size)
        .paint_at(ui, icon_rect);
}

/// A pill-shaped text button: filled for the primary action, outlined otherwise.
pub fn pill_button(ui: &mut egui::Ui, palette: &Palette, label: &str, primary: bool) -> Response {
    let font = semibold(13.0);
    let color = if primary {
        palette.on_accent
    } else {
        palette.text
    };
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, color);
    let padding = Vec2::new(18.0, 8.0);
    let size = galley.size() + padding * 2.0;
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let radius = rect.height() / 2.0;
        if primary {
            let fill = if hovered {
                palette.accent_hover
            } else {
                palette.accent
            };
            ui.painter().rect_filled(rect, radius, fill);
        } else {
            let stroke_color = if hovered { palette.text } else { palette.dim };
            ui.painter().rect_stroke(
                rect,
                radius,
                Stroke::new(1.0, stroke_color),
                egui::StrokeKind::Inside,
            );
        }
        let pos = rect.center() - galley.size() / 2.0;
        ui.painter().galley(pos, galley, color);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Subtle button with an optional icon and label.
pub fn soft_button(
    ui: &mut egui::Ui,
    palette: &Palette,
    icon: Option<Icon>,
    label: &str,
    active: bool,
) -> Response {
    let font = medium(13.0);
    let color = if active { palette.window } else { palette.text };
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, color);
    let icon_size = 15.0;
    let icon_width = if icon.is_some() { icon_size + 6.0 } else { 0.0 };
    let padding = Vec2::new(12.0, 7.0);
    let size = Vec2::new(galley.size().x + icon_width, galley.size().y) + padding * 2.0;
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let fill = if active {
            palette.text
        } else if hovered {
            palette.surface_hover
        } else {
            palette.surface
        };
        ui.painter().rect_filled(rect, rect.height() / 2.0, fill);
        let mut x = rect.left() + padding.x;
        if let Some(icon) = icon {
            let icon_rect = egui::Rect::from_center_size(
                egui::pos2(x + icon_size / 2.0, rect.center().y),
                Vec2::splat(icon_size),
            );
            icon.image(color, icon_size).paint_at(ui, icon_rect);
            x += icon_width;
        }
        let pos = egui::pos2(x, rect.center().y - galley.size().y / 2.0);
        ui.painter().galley(pos, galley, color);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Calculates [`soft_button`] width before layout.
pub fn soft_button_width(ui: &egui::Ui, label: &str, icon: bool) -> f32 {
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), medium(13.0), Color32::WHITE);
    let icon_width = if icon { 15.0 + 6.0 } else { 0.0 };
    galley.size().x + icon_width + 24.0
}

/// Animated busy indicator with timer-based repainting.
pub fn spinner(ui: &mut egui::Ui, size: f32, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    if ui.is_rect_visible(rect) {
        paint_spinner(ui, rect, size, color);
    }
    response
}

/// Paints a centered spinner without allocating space.
pub fn paint_spinner(ui: &egui::Ui, rect: egui::Rect, size: f32, color: Color32) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(33));
    let radius = size / 2.0 - 2.0;
    let start = ui.input(|input| input.time) * std::f64::consts::TAU * 1.2;
    let sweep = 250_f64.to_radians();
    let points = (0..20)
        .map(|index| {
            let angle = start + sweep * f64::from(index) / 19.0;
            let (sin, cos) = angle.sin_cos();
            rect.center() + radius * egui::vec2(cos as f32, sin as f32)
        })
        .collect();
    ui.painter()
        .add(egui::Shape::line(points, Stroke::new(2.0, color)));
}

/// Truncated single-line text.
pub fn text(
    ui: &mut egui::Ui,
    text: impl Into<String>,
    font: egui::FontId,
    color: Color32,
) -> Response {
    ui.add(
        egui::Label::new(egui::RichText::new(text).font(font).color(color))
            .truncate()
            .selectable(false),
    )
}

/// Selectable text label.
pub fn selectable_text(
    ui: &mut egui::Ui,
    text: impl Into<String>,
    font: egui::FontId,
    color: Color32,
) -> Response {
    ui.add(egui::Label::new(egui::RichText::new(text).font(font).color(color)).selectable(true))
}

/// Wrapping text label.
pub fn paragraph(
    ui: &mut egui::Ui,
    text: impl Into<String>,
    font: egui::FontId,
    color: Color32,
) -> Response {
    ui.add(
        egui::Label::new(egui::RichText::new(text).font(font).color(color))
            .wrap()
            .selectable(false),
    )
}

/// Clickable single-line text with a hover underline.
pub fn link(
    ui: &mut egui::Ui,
    text: impl Into<String>,
    font: egui::FontId,
    color: Color32,
) -> Response {
    let response = ui.add(
        egui::Label::new(egui::RichText::new(text).font(font).color(color))
            .truncate()
            .selectable(false)
            .sense(Sense::click()),
    );
    if response.hovered() {
        let rect = response.rect;
        ui.painter()
            .hline(rect.x_range(), rect.bottom() - 1.0, Stroke::new(1.0, color));
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn section_title(ui: &mut egui::Ui, palette: &Palette, label: &str) -> Response {
    text(ui, label, bold(17.0), palette.text)
}

pub fn subtle(ui: &mut egui::Ui, palette: &Palette, label: &str) -> Response {
    text(ui, label, regular(13.0), palette.secondary)
}

/// Mixes two colors; `t = 1` returns `b`.
pub fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgba_unmultiplied(
        mix(a.r(), b.r()),
        mix(a.g(), b.g()),
        mix(a.b(), b.b()),
        mix(a.a(), b.a()),
    )
}

/// Native macOS layout, also selectable in offline layout previews.
pub fn macos_chrome(ctx: &egui::Context) -> bool {
    #[cfg(any(test, feature = "demo"))]
    if ctx.data(|data| {
        data.get_temp::<bool>(egui::Id::new("macos-preview"))
            .unwrap_or(false)
    }) {
        return true;
    }
    let _ = ctx;
    cfg!(target_os = "macos")
}

#[cfg(any(test, feature = "demo"))]
pub fn preview_macos(ctx: &egui::Context) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("macos-preview"), true));
}

/// Horizontal clearance for native buttons; they do not scale with UI zoom.
pub fn traffic_light_inset(ctx: &egui::Context) -> f32 {
    if macos_chrome(ctx) && !ctx.input(|input| input.viewport().fullscreen.unwrap_or(false)) {
        84.0 / ctx.zoom_factor()
    } else {
        0.0
    }
}

/// Traffic-light strip used only while linking, before the chat header exists.
pub fn titlebar_inset(ctx: &egui::Context) -> f32 {
    if macos_chrome(ctx) && !ctx.input(|input| input.viewport().fullscreen.unwrap_or(false)) {
        28.0 / ctx.zoom_factor()
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_has_a_file() {
        for (icon, uri, bytes) in ICONS {
            assert!(!bytes.is_empty(), "{icon:?} is empty");
            assert!(uri.ends_with(".svg"));
            assert_eq!(icon.uri(), *uri);
        }
    }

    #[test]
    fn hsl_hits_the_primaries() {
        assert_eq!(hsl(0.0, 1.0, 0.5), Color32::from_rgb(255, 0, 0));
        assert_eq!(hsl(120.0, 1.0, 0.5), Color32::from_rgb(0, 255, 0));
        assert_eq!(hsl(240.0, 1.0, 0.5), Color32::from_rgb(0, 0, 255));
    }
}
