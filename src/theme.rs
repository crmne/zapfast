//! Colors, typography, icons, and base widgets.
//!
//! The UI uses Inter's font weights and Lucide icons. [`Palette`] holds all
//! light and dark theme colors.

use egui::{Color32, CornerRadius, Response, Sense, Stroke, Vec2};

/// A local JSON palette, known by its filename in the themes directory.
pub type CustomTheme = fastframe_theme::CustomTheme<Palette>;

/// The palettes Settings offers: local files, the shared presets, and on
/// Linux the live Omarchy palette.
pub type Catalog = fastframe_theme::Catalog<Palette>;

/// What a normal launch adds to the catalogue. Demos and tests leave it out
/// and stay isolated from the desktop and its files.
pub const DESKTOP_THEMES: fastframe_theme::DesktopThemes = fastframe_theme::DesktopThemes {
    slug: "zapfast",
    omarchy_template: include_str!("../contrib/omarchy/zapfast.json.tpl"),
    presets: true,
};

/// The shared palettes, as ZapFast reads them.
pub fn presets() -> impl Iterator<Item = CustomTheme> {
    fastframe_theme::presets::themes::<Palette>()
}

/// What the theme setting says under it while the catalogue loads or when
/// something went wrong.
pub fn theme_status(status: fastframe_theme::Status) -> &'static str {
    use fastframe_theme::{Problem, Status};
    match status {
        Status::Loading => "Loading local themes…",
        Status::SelectedUnavailable => {
            "The selected theme is unavailable. Keeping the last usable appearance. See the log for details."
        }
        Status::Problem(Problem::Unreadable) => {
            "The themes folder could not be read. See the log for details."
        }
        Status::Problem(Problem::TooManyEntries) => {
            "The themes folder has more than 512 entries. Keep fewer files there to list the custom palettes."
        }
        Status::Problem(Problem::TooManyThemes) => {
            "Only 128 custom palettes can be listed. Keep fewer JSON files in the themes folder to see the rest."
        }
        Status::Problem(Problem::OmarchyUnreadable) => {
            "The Omarchy palette could not be loaded. Keeping the last usable appearance. See the log for details."
        }
        Status::Problem(Problem::LoaderFailed | _) => {
            "Custom themes could not be loaded. Run zapfast reload-themes to try again."
        }
    }
}

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
            // Secondary and dim text reach WCAG AA (4.5:1) on the window,
            // panel, surface, and menu colours.
            secondary: Color32::from_rgb(0x97, 0xa5, 0xae),
            dim: Color32::from_rgb(0x8a, 0x98, 0x9f),
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
            // Secondary and dim text reach WCAG AA (4.5:1) on the window,
            // panel, surface, chat, and menu colours. The accent is the green
            // WhatsApp uses in its light theme, readable as text on white and
            // under white button labels.
            secondary: Color32::from_rgb(0x51, 0x5f, 0x67),
            dim: Color32::from_rgb(0x63, 0x6b, 0x72),
            accent: Color32::from_rgb(0x00, 0x80, 0x69),
            accent_hover: Color32::from_rgb(0x00, 0x6e, 0x5a),
            on_accent: Color32::WHITE,
            danger: Color32::from_rgb(0xea, 0x00, 0x38),
            warning: Color32::from_rgb(0xa0, 0x6b, 0x00),
            overlay: Color32::from_rgb(0xff, 0xff, 0xff),
            shadow: Color32::from_black_alpha(50),
            chat: Color32::from_rgb(0xef, 0xea, 0xe2),
            bubble_in: Color32::from_rgb(0xff, 0xff, 0xff),
            bubble_out: Color32::from_rgb(0xd9, 0xfd, 0xd3),
            link: Color32::from_rgb(0x02, 0x7e, 0xb5),
            // The lighter blue vanished on the green outgoing bubble.
            read: Color32::from_rgb(0x02, 0x7e, 0xb5),
        }
    }

    /// The palette for a message bubble's contents. Secondary and dim text,
    /// and the read ticks, move toward the text colour just far enough to
    /// stay readable on the bubble: a grey that reads on the panel can vanish
    /// on the outgoing bubble. Works for custom themes as well.
    pub fn on_bubble(&self, own: bool) -> Self {
        let fill = if own { self.bubble_out } else { self.bubble_in };
        Self {
            secondary: readable_on(fill, self.secondary, self.text, 4.5),
            dim: readable_on(fill, self.dim, self.text, 4.5),
            // Icons need 3:1 (WCAG 1.4.11).
            read: readable_on(fill, self.read, self.text, 3.0),
            ..*self
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

impl fastframe_theme::Palette for Palette {
    fn base(base: fastframe_theme::Base) -> Self {
        match base {
            fastframe_theme::Base::Dark => Self::dark(),
            fastframe_theme::Base::Light => Self::light(),
        }
    }

    fn set(&mut self, name: &str, color: Color32) -> bool {
        match name {
            "window" => self.window = color,
            "panel" => self.panel = color,
            "surface" => self.surface = color,
            "surface_hover" => self.surface_hover = color,
            "surface_active" => self.surface_active = color,
            "outline" => self.outline = color,
            "text" => self.text = color,
            "secondary" => self.secondary = color,
            "dim" => self.dim = color,
            "accent" => self.accent = color,
            "accent_hover" => self.accent_hover = color,
            "on_accent" => self.on_accent = color,
            "danger" => self.danger = color,
            "warning" => self.warning = color,
            "overlay" => self.overlay = color,
            "shadow" => self.shadow = color,
            "chat" => self.chat = color,
            "bubble_in" => self.bubble_in = color,
            "bubble_out" => self.bubble_out = color,
            "link" => self.link = color,
            "read" => self.read = color,
            _ => return false,
        }
        true
    }

    /// Spotifast palettes share the sixteen interface colours. Derive the
    /// chat-only colours when importing one, while keeping explicit ZapFast
    /// overrides.
    fn derive(&mut self, given: &std::collections::BTreeSet<&str>) {
        if given.contains("window") && !given.contains("chat") {
            self.chat = self.window;
        }
        if given.contains("surface") && !given.contains("bubble_in") {
            self.bubble_in = self.surface;
        }
        if given.contains("accent") {
            if !given.contains("bubble_out") {
                self.bubble_out = self.surface.lerp_to_gamma(self.accent, 0.18);
            }
            if !given.contains("link") {
                self.link = self.accent;
            }
            if !given.contains("read") {
                self.read = self.accent;
            }
        }
        if given.contains("panel") && !given.contains("overlay") {
            self.overlay = self.panel;
        }
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
pub const FOCUS_STROKE_WIDTH: f32 = 1.0;
pub const ROW_HEIGHT: f32 = 68.0;
pub const TOP_BAR_HEIGHT: f32 = 60.0;

pub fn regular(size: f32) -> egui::FontId {
    fastframe_fonts::Weight::Regular.font_id(size)
}

pub fn medium(size: f32) -> egui::FontId {
    fastframe_fonts::Weight::Medium.font_id(size)
}

pub fn semibold(size: f32) -> egui::FontId {
    fastframe_fonts::Weight::SemiBold.font_id(size)
}

pub fn bold(size: f32) -> egui::FontId {
    fastframe_fonts::Weight::Bold.font_id(size)
}

/// Installs fonts, icons, and base style.
pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    egui_extras::install_image_loaders(ctx);
    // Served by a loader that never forgets them, so `reduce_texture_memory`
    // below cannot leave an icon drawn at two sizes without bytes.
    fastframe_icons::install::<Icon>(ctx);
    // Drop the raw bytes and the decoded pixels once a texture is on the GPU.
    // egui keeps all three copies of every image otherwise, and only ever
    // evicts the textures of SVGs.
    ctx.options_mut(|options| options.reduce_texture_memory = true);
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
    // Hinting, sub-pixel positions and glyph coverage as the desktop draws
    // them. On Linux coverage stays linear in both themes, as FreeType and
    // cairo draw it; egui's dark curve (2c - c²) made text heavier than GTK's.
    text_rendering().apply_to_visuals(visuals);
    visuals.panel_fill = palette.panel;
    visuals.window_fill = palette.overlay;
    visuals.extreme_bg_color = palette.surface;
    visuals.faint_bg_color = palette.surface;
    visuals.code_bg_color = palette.surface;
    visuals.override_text_color = Some(palette.text);
    visuals.weak_text_color = Some(palette.secondary);
    visuals.hyperlink_color = palette.link;
    visuals.selection.bg_fill = palette.accent.gamma_multiply(0.35);
    visuals.selection.stroke = Stroke::new(FOCUS_STROKE_WIDTH, palette.accent);
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

/// Inter at four weights, egui's own fonts behind it, and installed fonts
/// for the scripts Inter lacks, hinted as the desktop asks.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = fastframe_fonts::FontSetup::default().definitions();
    text_rendering().apply_to(&mut fonts);
    ctx.set_fonts(fonts);
}

/// The desktop's text rendering: read once, on the first window, and kept
/// current by [`follow_text_rendering`].
static TEXT_RENDERING: std::sync::Mutex<Option<fastframe_text::TextRendering>> =
    std::sync::Mutex::new(None);

/// Set when the desktop's text rendering changed and no window has applied
/// it yet.
static TEXT_RENDERING_CHANGED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// How the desktop draws text. The first call reads its settings, which can
/// block for about a second on Linux when the desktop portal does not answer;
/// tests use the platform's defaults, so the machine does not decide them.
pub fn text_rendering() -> fastframe_text::TextRendering {
    *TEXT_RENDERING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_or_insert_with(|| {
            if cfg!(test) {
                fastframe_text::TextRendering::platform_default()
            } else {
                fastframe_text::detect()
            }
        })
}

/// Follows changes to the desktop's font settings (the desktop portal on
/// Linux) and calls `wake` so a window picks them up through
/// [`apply_text_rendering_change`]. Elsewhere, and without a portal, the
/// settings read at start stay.
pub fn follow_text_rendering(wake: impl Fn() + Send + 'static) {
    let current = text_rendering();
    let watched = fastframe_text::watch::watch(current, move |rendering| {
        *TEXT_RENDERING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(rendering);
        TEXT_RENDERING_CHANGED.store(true, std::sync::atomic::Ordering::Release);
        wake();
    });
    if let Err(error) = watched {
        log::debug!("not following the desktop's font settings: {error}");
    }
}

/// Reinstalls the fonts when the desktop's text rendering changed since the
/// last call. Returns whether it did, so the caller reapplies its visuals.
pub fn apply_text_rendering_change(ctx: &egui::Context) -> bool {
    let changed = TEXT_RENDERING_CHANGED.swap(false, std::sync::atomic::Ordering::AcqRel);
    if changed {
        install_fonts(ctx);
    }
    changed
}

fastframe_icons::icons! {
    /// Every icon the interface draws. Icons Spotifast ships too come from
    /// fastframe-icons (`lucide`); the rest are ZapFast's own files.
    pub enum Icon {
        prefix: "zapfast-icon-",
        directory: "../assets/icons/",
        Archive => "archive",
        ArrowDown => "arrow-down",
        ArrowLeft => lucide "arrow-left",
        Ban => "ban",
        Bell => "bell",
        BellOff => "bell-off",
        Calendar => "calendar",
        Check => lucide "check",
        CheckCheck => "check-check",
        ChevronDown => lucide "chevron-down",
        ChevronLeft => lucide "chevron-left",
        ChevronRight => lucide "chevron-right",
        ChevronUp => lucide "chevron-up",
        ListChecks => "list-checks",
        CircleAlert => lucide "circle-alert",
        CircleCheck => lucide "circle-check",
        CircleX => lucide "circle-x",
        Clock => lucide "clock",
        Contact => "contact",
        Copy => lucide "copy",
        Download => "download",
        Timer => "timer",
        Ellipsis => lucide "ellipsis",
        ExternalLink => lucide "external-link",
        Eye => lucide "eye",
        EyeOff => lucide "eye-off",
        FileText => "file-text",
        Forward => "forward",
        Gif => "gif",
        Heart => "heart",
        Image => "image",
        Info => lucide "info",
        Keyboard => "keyboard",
        Lock => lucide "lock",
        LockOpen => "lock-open",
        LogOut => lucide "log-out",
        MapPin => "map-pin",
        Maximize => lucide "maximize-2",
        MessageCircle => "message-circle",
        Mic => lucide "mic",
        Minimize => lucide "minimize-2",
        Minus => lucide "minus",
        Monitor => lucide "monitor",
        Moon => lucide "moon",
        PanelLeft => lucide "panel-left",
        Paperclip => "paperclip",
        Pause => lucide "pause",
        Pencil => lucide "pencil",
        Phone => "phone",
        Pin => lucide "pin",
        PinOff => lucide "pin-off",
        Play => lucide "play",
        Plus => lucide "plus",
        QrCode => "qr-code",
        Refresh => lucide "refresh-cw",
        Reply => "reply",
        Search => lucide "search",
        Send => "send",
        Settings => lucide "settings",
        Smartphone => lucide "smartphone",
        Smile => "smile",
        SquarePen => lucide "square-pen",
        Star => "star",
        StarFill => "star-fill",
        StarOff => "star-off",
        Sticker => "sticker",
        Sun => lucide "sun",
        Tag => "tag",
        Trash => lucide "trash-2",
        User => lucide "user",
        Users => lucide "users",
        Video => "video",
        Volume2 => lucide "volume-2",
        VolumeX => lucide "volume-x",
        WifiOff => "wifi-off",
        X => lucide "x",
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
    reveal_focus(&response);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tooltip)
    });
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
    reveal_focus(&response);
    focus_outline_on_fill(ui, response.id, rect, diameter / 2.0, fill);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tooltip)
    });
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
    reveal_focus(&response);
    focus_outline_on_fill(
        ui,
        response.id,
        rect,
        rect.height() / 2.0,
        if primary {
            palette.accent
        } else {
            Color32::TRANSPARENT
        },
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    // A disabled Ui already fades the painter; it must not react to hover.
    let enabled = ui.is_enabled();
    if ui.is_rect_visible(rect) {
        let hovered = enabled && response.hovered();
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
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
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
    reveal_focus(&response);
    focus_outline(ui, response.id, rect, rect.height() / 2.0);
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

/// Whether focus last moved by keyboard, like the web's `:focus-visible`.
pub fn keyboard_focus_id() -> egui::Id {
    egui::Id::new("keyboard-focus")
}

/// The visible control may be larger than its text editor, or circular rather
/// than rectangular. Keep its outline geometry with the current frame only.
#[derive(Clone, Copy, Debug)]
pub struct FocusOutline {
    pub rect: egui::Rect,
    pub radius: f32,
    pub clip: egui::Rect,
    pub frame: u64,
    pub fill: Color32,
}

pub fn focus_outline(ui: &egui::Ui, id: egui::Id, rect: egui::Rect, radius: f32) {
    focus_outline_on_fill(ui, id, rect, radius, Color32::TRANSPARENT);
}

fn focus_outline_on_fill(
    ui: &egui::Ui,
    id: egui::Id,
    rect: egui::Rect,
    radius: f32,
    fill: Color32,
) {
    let outline = FocusOutline {
        rect,
        radius,
        clip: ui.clip_rect(),
        frame: ui.ctx().cumulative_frame_nr(),
        fill,
    };
    ui.ctx()
        .data_mut(|data| data.insert_temp(id.with("focus-outline"), outline));
}

/// Scrolls a widget that keyboard focus reached into view. egui does not do
/// this itself, and a scroll target only counts when set while the widget's
/// scroll area is being laid out, so each focusable widget calls this.
pub fn reveal_focus(response: &Response) {
    let keyboard = response
        .ctx
        .data(|data| data.get_temp::<bool>(keyboard_focus_id()).unwrap_or(false));
    // Once per focus change: scrolling again before the first scroll lands
    // would overshoot.
    if keyboard && response.gained_focus() && response.interact_rect != response.rect {
        // Jump rather than glide: the focus should be visible at once.
        // A small margin avoids subpixel clipping when scroll offsets round to
        // device pixels, and leaves room for the focus stroke.
        let mut target = response.clone();
        target.rect = target.rect.expand(4.0);
        target.scroll_to_me_animation(None, egui::style::ScrollAnimation::none());
    }
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

/// WCAG contrast ratio between two opaque colours, from 1 to 21.
pub fn contrast(a: Color32, b: Color32) -> f32 {
    fn luminance(color: Color32) -> f32 {
        let channel = |value: u8| {
            let value = f32::from(value) / 255.0;
            if value <= 0.040_45 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
    }
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// `color`, blended toward `toward` only as far as needed to reach `target`
/// contrast on `background`.
pub fn readable_on(background: Color32, color: Color32, toward: Color32, target: f32) -> Color32 {
    let mut step = 0.0;
    loop {
        let candidate = blend(color, toward, step);
        if step >= 1.0 || contrast(candidate, background) >= target {
            return candidate;
        }
        step = (step + 0.05_f32).min(1.0);
    }
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
    if cfg!(target_os = "macos") {
        fastframe_macos::traffic_light_inset(ctx)
    } else if macos_chrome(ctx) && !ctx.input(|input| input.viewport().fullscreen.unwrap_or(false))
    {
        // The demo's preview of the macOS layout on other platforms.
        fastframe_macos::TRAFFIC_LIGHTS_WIDTH / ctx.zoom_factor()
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

    /// The palette decides the theme, and the desktop's rendering its text
    /// options: linear coverage in both themes on Linux, as GTK draws it.
    #[test]
    fn text_follows_the_desktop_rendering_in_both_themes() {
        let rendering = text_rendering();
        for palette in [Palette::dark(), Palette::light()] {
            let ctx = egui::Context::default();
            apply(&ctx, &palette);
            let options = ctx.global_style().visuals.text_options;
            assert_eq!(
                options.color_transfer_function,
                rendering.color_transfer_function(palette.dark),
                "dark: {}",
                palette.dark
            );
            assert_eq!(options.subpixel_binning, rendering.subpixel_positioning);
            #[cfg(target_os = "linux")]
            assert_eq!(
                options.color_transfer_function,
                egui::epaint::FontColorTransferFunction::Off
            );
        }
    }

    fn assert_readable(name: &str, pairs: &[(&str, Color32, Color32)], target: f32) {
        for (what, color, background) in pairs {
            let ratio = contrast(*color, *background);
            assert!(
                ratio >= target,
                "{name}: {what} is {ratio:.2}:1, needs {target}:1"
            );
        }
    }

    /// Secondary and dim carry real content: previews, times, numbers,
    /// hints. They must reach WCAG AA wherever the built-in themes put them.
    #[test]
    fn built_in_text_colours_reach_aa() {
        for (name, p) in [("dark", Palette::dark()), ("light", Palette::light())] {
            let mut pairs = Vec::new();
            for (surface, background) in [
                ("window", p.window),
                ("panel", p.panel),
                ("surface", p.surface),
                ("menu", p.overlay),
            ] {
                pairs.push((
                    "secondary on ".to_owned() + surface,
                    p.secondary,
                    background,
                ));
                pairs.push(("dim on ".to_owned() + surface, p.dim, background));
            }
            pairs.push(("accent text on panel".into(), p.accent, p.panel));
            pairs.push(("button label on accent".into(), p.on_accent, p.accent));
            let pairs: Vec<_> = pairs
                .iter()
                .map(|(what, color, background)| (what.as_str(), *color, *background))
                .collect();
            assert_readable(name, &pairs, 4.5);
        }
        let light = Palette::light();
        assert_readable(
            "light",
            &[
                ("secondary on chat", light.secondary, light.chat),
                ("dim on chat", light.dim, light.chat),
            ],
            4.5,
        );
    }

    /// Bubble contents stay readable for every palette, custom ones included.
    #[test]
    fn bubble_text_is_readable_in_every_palette() {
        let palettes = [("dark", Palette::dark()), ("light", Palette::light())]
            .into_iter()
            .map(|(name, palette)| (name.to_owned(), palette))
            .chain(presets().map(|theme| (theme.filename.clone(), theme.palette)));
        for (name, palette) in palettes {
            for own in [false, true] {
                let fill = if own {
                    palette.bubble_out
                } else {
                    palette.bubble_in
                };
                let bubble = palette.on_bubble(own);
                let name = format!("{name}, {} bubble", if own { "own" } else { "their" });
                assert_readable(
                    &name,
                    &[
                        ("secondary", bubble.secondary, fill),
                        ("dim", bubble.dim, fill),
                    ],
                    4.5,
                );
                assert_readable(&name, &[("read ticks", bubble.read, fill)], 3.0);
            }
        }
    }

    #[test]
    fn spotifast_palettes_also_colour_the_conversation() {
        let themes: Vec<_> = presets().collect();
        assert_eq!(themes.len(), 8);
        for theme in themes {
            let palette = theme.palette;
            assert_eq!(palette.chat, palette.window);
            assert_eq!(palette.bubble_in, palette.surface);
            assert_ne!(palette.bubble_out, palette.bubble_in);
            assert_eq!(palette.link, palette.accent);
            assert_eq!(
                palette.dark,
                !matches!(
                    theme.filename.as_str(),
                    "Catppuccin Latte.json" | "Rose Pine Dawn.json"
                )
            );
        }
    }

    #[test]
    fn rose_pine_dawn_hovered_primary_buttons_keep_readable_content() {
        let palette = presets()
            .find(|theme| theme.filename == "Rose Pine Dawn.json")
            .unwrap()
            .palette;
        let ratio = contrast(palette.on_accent, palette.accent_hover);
        assert!(ratio >= 4.5, "hover contrast is only {ratio:.2}:1");
    }

    /// Every colour ZapFast has can be set by name, including the chat
    /// colours Spotifast's palettes lack; explicit ones win over derived ones.
    #[test]
    fn palette_files_set_every_colour_and_keep_explicit_chat_colours() {
        let palette: Palette = fastframe_theme::parse_palette(
            r##"{"base":"light","colors":{"window":"#101010","accent":"#203040","chat":"#010203","bubble_out":"#040506","shadow":"#00000080"}}"##,
        )
        .unwrap();
        assert!(!palette.dark);
        assert_eq!(palette.chat, Color32::from_rgb(1, 2, 3));
        assert_eq!(palette.bubble_out, Color32::from_rgb(4, 5, 6));
        assert_eq!(palette.link, Color32::from_rgb(0x20, 0x30, 0x40));
        assert_eq!(palette.shadow, Color32::from_black_alpha(128));
        assert_eq!(palette.panel, Palette::light().panel);
        for name in [
            "window",
            "panel",
            "surface",
            "surface_hover",
            "surface_active",
            "outline",
            "text",
            "secondary",
            "dim",
            "accent",
            "accent_hover",
            "on_accent",
            "danger",
            "warning",
            "overlay",
            "shadow",
            "chat",
            "bubble_in",
            "bubble_out",
            "link",
            "read",
        ] {
            let mut palette = Palette::dark();
            assert!(
                fastframe_theme::Palette::set(&mut palette, name, Color32::RED),
                "{name}"
            );
        }
        assert!(
            fastframe_theme::parse_palette::<Palette>(r##"{"colors":{"typo":"#ffffff"}}"##)
                .is_err()
        );
    }

    /// ZapFast's Omarchy template adds the chat colours to the base ones and
    /// renders as Omarchy's own renderer does.
    #[test]
    fn the_omarchy_template_renders_like_omarchy_in_light_and_dark_themes() {
        const TEMPLATE: &str = include_str!("../contrib/omarchy/zapfast.json.tpl");
        for (colors, expected) in [
            (
                include_str!("../tests/fixtures/omarchy/catppuccin.tsv"),
                include_str!("../tests/fixtures/omarchy/catppuccin.json"),
            ),
            (
                include_str!("../tests/fixtures/omarchy/catppuccin-latte.tsv"),
                include_str!("../tests/fixtures/omarchy/catppuccin-latte.json"),
            ),
        ] {
            let actual =
                fastframe_theme::omarchy::render_seed::<Palette>(TEMPLATE, colors).unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&actual).unwrap(),
                serde_json::from_str::<serde_json::Value>(expected).unwrap()
            );
        }
        assert_eq!(DESKTOP_THEMES.omarchy_template, TEMPLATE);
    }

    /// The hook packages install must be the one fastframe-theme describes.
    /// A Windows checkout may turn its line endings into CRLF.
    #[test]
    fn the_shipped_omarchy_hook_has_not_drifted() {
        assert_eq!(
            include_str!("../contrib/omarchy/zapfast-theme").replace("\r\n", "\n"),
            fastframe_theme::omarchy::hook_script("zapfast")
        );
    }

    #[test]
    fn readable_on_leaves_a_readable_colour_alone() {
        let light = Palette::light();
        assert_eq!(
            readable_on(light.bubble_in, light.secondary, light.text, 4.5),
            light.secondary
        );
    }

    #[test]
    fn hsl_hits_the_primaries() {
        assert_eq!(hsl(0.0, 1.0, 0.5), Color32::from_rgb(255, 0, 0));
        assert_eq!(hsl(120.0, 1.0, 0.5), Color32::from_rgb(0, 255, 0));
        assert_eq!(hsl(240.0, 1.0, 0.5), Color32::from_rgb(0, 0, 255));
    }
}
