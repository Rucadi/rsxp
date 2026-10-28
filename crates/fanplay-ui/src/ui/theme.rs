// SPDX-License-Identifier: GPL-2.0-or-later
//! Adventure Journal — the shared visual language for every FANPLAY frontend.
//!
//! [`apply`] installs the selected theme and shared typography. Adventure Journal
//! is the default; Arcade Neon, Moonlit Grove and Field Guide are built in.
//! The module also owns reusable workspace widgets and custom theme loading.
//!
//! `apply` is idempotent per [`egui::Context`]. Fonts are installed once (they
//! are relatively expensive), while the style and visuals are refreshed each
//! frame so a host that swaps themes cannot leave the workspace light-on-light.
use egui::{
    Align, Color32, CornerRadius, FontFamily, FontId, Frame, Layout, Response, RichText, Sense,
    Stroke, StrokeKind, Ui, Vec2,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
};
static ACTIVE_THEME: OnceLock<Mutex<ThemeDefinition>> = OnceLock::new();
static THEME_REVISION: AtomicU64 = AtomicU64::new(1);

// ---------------------------------------------------------------------------
// Palette
// ---------------------------------------------------------------------------

pub const PINK: Color32 = Color32::from_rgb(0x28, 0x65, 0x4D); // primary forest
pub const CYAN: Color32 = Color32::from_rgb(0xB5, 0x6A, 0x3A); // warm terracotta accent
pub const LIME: Color32 = Color32::from_rgb(0x8B, 0xCD, 0xAB); // success
pub const WARNING: Color32 = Color32::from_rgb(0x81, 0x56, 0x00);
pub const ERROR: Color32 = Color32::from_rgb(0xA9, 0x36, 0x3E);
pub const INFO: Color32 = CYAN;

pub const BG: Color32 = Color32::from_rgb(0xEA, 0xE6, 0xD8);
pub const SURFACE: Color32 = Color32::from_rgb(0xFF, 0xF9, 0xEC);
pub const SURFACE_RAISED: Color32 = Color32::from_rgb(0xF3, 0xEF, 0xE2);
pub const BORDER: Color32 = Color32::from_rgb(0xA6, 0xB5, 0xA3);
pub const TEXT: Color32 = Color32::from_rgb(0x26, 0x3C, 0x36);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x52, 0x64, 0x5C);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(0x7A, 0x88, 0x7D);

/// User-editable theme metadata. Save these definitions in
/// `<FANPLAY_HOME>/config/themes.json`; unknown themes fall back safely.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeDefinition {
    pub name: String,
    pub background: String,
    pub surface: String,
    pub surface_inset: String,
    pub text: String,
    pub text_muted: String,
    pub primary: String,
    pub secondary: String,
    pub accent: String,
    pub error: String,
}

impl Default for ThemeDefinition {
    fn default() -> Self {
        Self {
            name: "Adventure Journal".into(),
            background: "#EAE6D8".into(),
            surface: "#FFF9EC".into(),
            surface_inset: "#F3EFE2".into(),
            text: "#263C36".into(),
            text_muted: "#52645C".into(),
            primary: "#28654D".into(),
            secondary: "#B56A3A".into(),
            accent: "#D4A43B".into(),
            error: "#A9363E".into(),
        }
    }
}

pub fn builtin_themes() -> Vec<ThemeDefinition> {
    vec![
        ThemeDefinition::default(),
        ThemeDefinition {
            name: "Arcade Neon".into(),
            background: "#0A0A14".into(),
            surface: "#12121F".into(),
            surface_inset: "#1A1A2E".into(),
            text: "#EEEEF0".into(),
            text_muted: "#9999AA".into(),
            primary: "#FF006E".into(),
            secondary: "#00F0FF".into(),
            accent: "#39FF14".into(),
            error: "#FF006E".into(),
        },
        ThemeDefinition {
            name: "Moonlit Grove".into(),
            background: "#172722".into(),
            surface: "#22382F".into(),
            surface_inset: "#1C3028".into(),
            text: "#F4F0E3".into(),
            text_muted: "#B5C5B7".into(),
            primary: "#8BCDAB".into(),
            secondary: "#95C9ED".into(),
            accent: "#E7BE65".into(),
            error: "#F39A9A".into(),
        },
        ThemeDefinition {
            name: "Field Guide".into(),
            background: "#F2EBDD".into(),
            surface: "#FFFDF5".into(),
            surface_inset: "#E5EEF3".into(),
            text: "#263746".into(),
            text_muted: "#60717D".into(),
            primary: "#D83A3A".into(),
            secondary: "#2D6FA3".into(),
            accent: "#D9A62E".into(),
            error: "#B32635".into(),
        },
    ]
}

pub fn load_themes(path: &Path) -> Vec<ThemeDefinition> {
    let mut themes = builtin_themes();
    if let Ok(bytes) = fs::read(path)
        && let Ok(custom) = serde_json::from_slice::<Vec<ThemeDefinition>>(&bytes)
    {
        for theme in custom {
            if !theme.name.trim().is_empty() {
                themes.retain(|existing| existing.name != theme.name);
                themes.push(theme);
            }
        }
    }
    themes
}

pub fn save_themes(path: &Path, themes: &[ThemeDefinition]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        path,
        serde_json::to_vec_pretty(themes).expect("theme serialization"),
    )
}

pub fn set_active_theme(theme: ThemeDefinition) {
    let slot = ACTIVE_THEME.get_or_init(|| Mutex::new(ThemeDefinition::default()));
    *slot.lock().expect("theme lock") = theme;
    THEME_REVISION.fetch_add(1, Ordering::Relaxed);
}

fn active_theme() -> ThemeDefinition {
    ACTIVE_THEME
        .get_or_init(|| Mutex::new(ThemeDefinition::default()))
        .lock()
        .expect("theme lock")
        .clone()
}

pub fn background() -> Color32 {
    hex(&active_theme().background, BG)
}
pub fn surface() -> Color32 {
    hex(&active_theme().surface, SURFACE)
}
pub fn surface_inset() -> Color32 {
    hex(&active_theme().surface_inset, SURFACE_RAISED)
}
pub fn text() -> Color32 {
    hex(&active_theme().text, TEXT)
}
pub fn text_muted() -> Color32 {
    hex(&active_theme().text_muted, TEXT_MUTED)
}
pub fn primary() -> Color32 {
    hex(&active_theme().primary, PINK)
}
pub fn secondary() -> Color32 {
    hex(&active_theme().secondary, CYAN)
}
pub fn accent() -> Color32 {
    hex(&active_theme().accent, LIME)
}
pub fn error() -> Color32 {
    hex(&active_theme().error, ERROR)
}

fn hex(value: &str, fallback: Color32) -> Color32 {
    let value = value.trim_start_matches('#');
    if value.len() == 6
        && let Ok(rgb) = u32::from_str_radix(value, 16)
    {
        return Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
    }
    fallback
}

/// Standard 8px spacing scale.
pub const XS: f32 = 4.0;
pub const SM: f32 = 8.0;
pub const MD: f32 = 16.0;
pub const LG: f32 = 24.0;
pub const XL: f32 = 32.0;
pub const XXL: f32 = 48.0;

const BUNGEE_FAMILY: &str = "heading";

// Embedding the OFL typefaces keeps every frontend (native, web, Android)
// identical without system font dependencies.
const BUNGEE: &[u8] = include_bytes!("../../assets/fonts/WorkSans.ttf");
const WORK_SANS: &[u8] = include_bytes!("../../assets/fonts/WorkSans.ttf");
const SPACE_MONO: &[u8] = include_bytes!("../../assets/fonts/SpaceMono-Regular.ttf");

// ---------------------------------------------------------------------------
// Typography
// ---------------------------------------------------------------------------

/// Friendly bold heading face; kept as a named family for existing callers.
pub fn display_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BUNGEE_FAMILY.into()))
}

/// Work Sans — body copy, labels, and button text.
pub fn body_font(size: f32) -> FontId {
    FontId::proportional(size)
}

/// Space Mono — scores, counters, paths, and any numerical display.
pub fn mono_font(size: f32) -> FontId {
    FontId::monospace(size)
}

/// A Bungee headline in the primary text colour.
pub fn display(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).font(display_font(size)).color(TEXT)
}

/// A Bungee headline with a neon accent.
pub fn display_color(text: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(text).font(display_font(size)).color(color)
}

/// Uppercase Space Mono label used above sections and metrics.
pub fn eyebrow(text: impl Into<String>) -> RichText {
    RichText::new(text.into().to_uppercase())
        .font(mono_font(11.0))
        .color(TEXT_MUTED)
}

fn install_fonts(context: &egui::Context) {
    let id = egui::Id::unique("fanplay-fonts-v3");
    if context.data_mut(|d| d.get_temp::<bool>(id).unwrap_or(false)) {
        return;
    }
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "work-sans".into(),
        egui::FontData::from_static(WORK_SANS).into(),
    );
    fonts.font_data.insert(
        "space-mono".into(),
        egui::FontData::from_static(SPACE_MONO).into(),
    );
    fonts
        .font_data
        .insert("bungee".into(), egui::FontData::from_static(BUNGEE).into());

    let proportional = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let monospace = fonts
        .families
        .get(&FontFamily::Monospace)
        .cloned()
        .unwrap_or_default();

    let mut proportional_with_body = vec!["work-sans".to_owned()];
    proportional_with_body.extend(proportional.iter().cloned());
    fonts
        .families
        .insert(FontFamily::Proportional, proportional_with_body);

    let mut mono_with_body = vec!["space-mono".to_owned()];
    mono_with_body.extend(monospace.iter().cloned());
    fonts.families.insert(FontFamily::Monospace, mono_with_body);

    // Bungee is display-only. Keep the proportional stack as a fallback so
    // emoji and unusual accents still render inside headings.
    let mut bungee = vec!["bungee".to_owned()];
    bungee.extend(proportional);
    fonts
        .families
        .insert(FontFamily::Name(BUNGEE_FAMILY.into()), bungee);

    context.set_fonts(fonts);
    context.data_mut(|d| d.insert_temp(id, true));
}

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Installs the Adventure Journal style. Safe to call every frame.
pub fn apply(context: &egui::Context) {
    let revision = THEME_REVISION.load(Ordering::Relaxed);
    let revision_id = egui::Id::unique("fanplay-theme-revision");
    if context.data(|data| data.get_temp::<u64>(revision_id)) == Some(revision) {
        return;
    }
    let theme = active_theme();
    let dark = theme.background.starts_with("#0A") || theme.name == "Arcade Neon";
    context.set_theme(if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });
    install_fonts(context);

    let bg = hex(&theme.background, BG);
    let surface = hex(&theme.surface, SURFACE);
    let inset = hex(&theme.surface_inset, SURFACE_RAISED);
    let text = hex(&theme.text, TEXT);
    let muted = hex(&theme.text_muted, TEXT_MUTED);
    let primary = hex(&theme.primary, PINK);
    let secondary = hex(&theme.secondary, CYAN);
    let error = hex(&theme.error, ERROR);
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.panel_fill = bg;
    visuals.window_fill = inset;
    visuals.extreme_bg_color = bg;
    visuals.text_edit_bg_color = Some(surface);
    visuals.faint_bg_color = inset;
    visuals.code_bg_color = bg;
    visuals.hyperlink_color = secondary;
    visuals.warn_fg_color = WARNING;
    visuals.error_fg_color = error;
    visuals.weak_text_color = Some(muted);
    visuals.selection.bg_fill = primary.gamma_multiply(0.22);
    visuals.selection.stroke = Stroke::new(1.0, primary);
    visuals.override_text_color = Some(text);
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.menu_corner_radius = CornerRadius::same(12);
    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 2],
        blur: 8,
        spread: 0,
        color: Color32::BLACK.gamma_multiply(0.12),
    };
    visuals.popup_shadow = egui::epaint::Shadow {
        offset: [0, 2],
        blur: 6,
        spread: 0,
        color: Color32::BLACK.gamma_multiply(0.10),
    };
    visuals.text_cursor.stroke = Stroke::new(2.0, CYAN);
    visuals.button_frame = true;
    visuals.striped = true;
    visuals.indent_has_left_vline = false;
    visuals.slider_trailing_fill = true;

    let radius = CornerRadius::same(8);
    visuals.widgets.noninteractive = egui::style::WidgetVisuals {
        bg_fill: SURFACE,
        weak_bg_fill: SURFACE,
        bg_stroke: Stroke::new(1.0, BORDER),
        corner_radius: radius,
        fg_stroke: Stroke::new(1.0, TEXT),
        expansion: 0.0,
    };
    visuals.widgets.inactive = egui::style::WidgetVisuals {
        bg_fill: SURFACE,
        weak_bg_fill: SURFACE,
        bg_stroke: Stroke::new(1.0, BORDER),
        corner_radius: radius,
        fg_stroke: Stroke::new(1.0, TEXT),
        expansion: 0.0,
    };
    visuals.widgets.hovered = egui::style::WidgetVisuals {
        bg_fill: SURFACE_RAISED,
        weak_bg_fill: SURFACE_RAISED,
        bg_stroke: Stroke::new(1.0, PINK),
        corner_radius: radius,
        fg_stroke: Stroke::new(1.0, Color32::WHITE),
        expansion: 1.0,
    };
    visuals.widgets.active = egui::style::WidgetVisuals {
        bg_fill: PINK,
        weak_bg_fill: PINK,
        bg_stroke: Stroke::new(1.0, PINK),
        corner_radius: radius,
        fg_stroke: Stroke::new(1.0, Color32::WHITE),
        expansion: 0.0,
    };
    visuals.widgets.open = visuals.widgets.hovered;
    context.set_visuals(visuals);

    context.all_styles_mut(|style| {
        style.interaction.selectable_labels = true;
        style.animation_time = 0.14;
        style.spacing.item_spacing = Vec2::new(SM, SM);
        style.spacing.button_padding = Vec2::new(14.0, 8.0);
        style.spacing.window_margin = egui::Margin::same(16);
        style.spacing.menu_margin = egui::Margin::same(8);
        style.spacing.indent = 18.0;
        style.spacing.interact_size = Vec2::new(44.0, 40.0);
        style.spacing.slider_width = 180.0;
        style.spacing.slider_rail_height = 6.0;
        style.spacing.combo_height = 280.0;
        style.spacing.scroll.bar_width = 8.0;
        style.spacing.scroll.handle_min_length = 24.0;
        style.spacing.scroll.bar_inner_margin = 4.0;
        style.spacing.scroll.floating = true;

        style
            .text_styles
            .insert(egui::TextStyle::Heading, display_font(28.0));
        style
            .text_styles
            .insert(egui::TextStyle::Body, body_font(16.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, body_font(16.0));
        style
            .text_styles
            .insert(egui::TextStyle::Small, body_font(12.0));
        style
            .text_styles
            .insert(egui::TextStyle::Monospace, mono_font(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Name("display".into()), display_font(48.0));
        style
            .text_styles
            .insert(egui::TextStyle::Name("title".into()), display_font(22.0));
        style
            .text_styles
            .insert(egui::TextStyle::Name("label".into()), mono_font(11.0));
        style
            .text_styles
            .insert(egui::TextStyle::Name("metric".into()), mono_font(26.0));
    });
    context.data_mut(|data| data.insert_temp(revision_id, revision));
}

// ---------------------------------------------------------------------------
// Glow
// ---------------------------------------------------------------------------

/// Fractional pulse in `0..=1`, driven by egui's frame clock. Use it for
/// featured glows and live status dots, not for body text.
pub fn pulse(context: &egui::Context) -> f32 {
    let time = context.input(|input| input.time) as f32;
    (time * 2.2).sin() * 0.5 + 0.5
}

/// Paints a soft neon halo behind `rect`. Call this before painting the
/// element itself so the glow reads as light rather than an outline.
pub fn glow(painter: &egui::Painter, rect: egui::Rect, radius: u8, color: Color32, strength: f32) {
    let _ = strength;
    painter.rect_stroke(
        rect,
        CornerRadius::same(radius),
        Stroke::new(1.0, color.gamma_multiply(0.35)),
        StrokeKind::Inside,
    );
}

/// A thin neon rule with a soft fade at both ends.
pub fn divider(ui: &mut Ui) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 1.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::ZERO, BORDER);
    ui.add_space(SM);
}

/// Uppercase section label with a short neon tick.
pub fn section_label(ui: &mut Ui, text: &str, color: Color32) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(3.0, 12.0), Sense::hover());
        glow(ui.painter(), rect, 1, color, 0.8);
        ui.painter().rect_filled(rect, CornerRadius::same(2), color);
        ui.add_space(XS);
        ui.label(
            RichText::new(text.to_uppercase())
                .font(mono_font(11.0))
                .color(color),
        );
    });
}

/// A small keyboard cap, e.g. `F1` or `Enter`.
pub fn keycap(ui: &mut Ui, label: &str) {
    Frame::new()
        .fill(surface_inset())
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(label).font(mono_font(11.0)).color(TEXT));
        });
}

// ---------------------------------------------------------------------------
// Buttons
// ---------------------------------------------------------------------------

/// Button emphasis, mapped directly to the design system's variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Secondary,
    Ghost,
    Destructive,
    Success,
}

impl ButtonKind {
    fn colors(self) -> (Color32, Color32, Color32) {
        // (fill, text, glow)
        match self {
            Self::Primary => (primary(), Color32::WHITE, primary()),
            Self::Secondary => (Color32::TRANSPARENT, secondary(), secondary()),
            Self::Ghost => (Color32::TRANSPARENT, text_muted(), Color32::TRANSPARENT),
            Self::Destructive => (error(), Color32::from_rgb(0xFF, 0xF9, 0xEC), error()),
            Self::Success => (accent(), text(), accent()),
        }
    }
}

/// Button size from the design tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonSize {
    Sm,
    Md,
    Lg,
}

impl ButtonSize {
    fn metrics(self) -> (f32, f32, f32) {
        // (horizontal padding, font size, height)
        match self {
            Self::Sm => (14.0, 14.0, 32.0),
            Self::Md => (22.0, 16.0, 40.0),
            Self::Lg => (30.0, 18.0, 48.0),
        }
    }
}

/// Draws a neon button. The response supports click, hover, keyboard focus,
/// and disabled state exactly like [`egui::Button`], while adding the design
/// system's glow and palette.
pub fn button(ui: &mut Ui, label: impl Into<String>, kind: ButtonKind) -> Response {
    button_sized(ui, label, kind, ButtonSize::Md)
}

/// [`button`] with an explicit size.
pub fn button_sized(
    ui: &mut Ui,
    label: impl Into<String>,
    kind: ButtonKind,
    size: ButtonSize,
) -> Response {
    button_impl(ui, label.into(), kind, size, None)
}

/// [`button`] with a fixed width, for card actions and toolbars that need a
/// predictable grid.
pub fn button_width(
    ui: &mut Ui,
    label: impl Into<String>,
    kind: ButtonKind,
    size: ButtonSize,
    width: f32,
) -> Response {
    button_impl(ui, label.into(), kind, size, Some(width))
}

fn button_impl(
    ui: &mut Ui,
    label: String,
    kind: ButtonKind,
    size: ButtonSize,
    width: Option<f32>,
) -> Response {
    let (padding, font_size, height) = size.metrics();
    let font = body_font(font_size);
    let galley = ui.painter().layout_no_wrap(label.clone(), font, TEXT);
    let desired = Vec2::new(
        width.unwrap_or((galley.size().x + padding * 2.0).max(height)),
        height,
    );
    let enabled = ui.is_enabled();
    let (rect, response) = ui.allocate_exact_size(
        desired,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let (fill, text_color, glow_color) = kind.colors();
    let hovered = enabled && response.hovered();

    let fill = if !enabled {
        fill.gamma_multiply(0.3)
    } else if hovered {
        match kind {
            ButtonKind::Ghost => surface_inset(),
            ButtonKind::Secondary => secondary().gamma_multiply(0.10),
            _ => brighten(fill, 0.16),
        }
    } else {
        fill
    };
    let border = match kind {
        ButtonKind::Secondary => Stroke::new(1.0, secondary()),
        ButtonKind::Ghost => Stroke::NONE,
        _ => Stroke::NONE,
    };
    let text_color = if enabled { text_color } else { TEXT_FAINT };

    if enabled && glow_color != Color32::TRANSPARENT {
        let strength = if hovered { 1.4 } else { 0.55 };
        glow(ui.painter(), rect, 8, glow_color, strength);
    }
    ui.painter().rect(
        rect,
        CornerRadius::same(8),
        fill,
        border,
        StrokeKind::Inside,
    );
    ui.painter()
        .galley(rect.center() - galley.size() * 0.5, galley, text_color);

    // The allocated response carries the interaction; keeping it as the same
    // widget preserves pointer, keyboard focus, and accessibility semantics.
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label.clone()));
    if enabled && response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(2.0),
            CornerRadius::same(6),
            Stroke::new(2.0, PINK),
            StrokeKind::Inside,
        );
    }
    response
}

fn brighten(color: Color32, amount: f32) -> Color32 {
    let [r, g, b, _] = color.to_array();
    let lift = |channel: u8| (channel as f32 + (255.0 - channel as f32) * amount) as u8;
    Color32::from_rgb(lift(r), lift(g), lift(b))
}

// ---------------------------------------------------------------------------
// Chips and tags
// ---------------------------------------------------------------------------

/// Semantic chip/tag tone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Pink,
    Cyan,
    Lime,
    Warning,
    Error,
    Neutral,
}

impl Tone {
    pub fn color(self) -> Color32 {
        match self {
            Self::Pink => primary(),
            Self::Cyan => secondary(),
            Self::Lime => accent(),
            Self::Warning => WARNING,
            Self::Error => ERROR,
            Self::Neutral => TEXT_MUTED,
        }
    }
}

/// An interactive filter chip. Selected chips are a solid neon fill; resting
/// chips are a quiet surface with a hairline border.
pub fn chip(ui: &mut Ui, label: &str, tone: Tone, selected: bool) -> Response {
    let font = mono_font(11.0);
    let text = label.to_uppercase();
    let galley = ui.painter().layout_no_wrap(text.clone(), font, TEXT);
    let desired = Vec2::new(galley.size().x + 24.0, 22.0);
    let enabled = ui.is_enabled();
    let (rect, response) = ui.allocate_exact_size(
        desired,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let hovered = enabled && response.hovered();
    let color = tone.color();
    let (fill, text_color, border) = if selected {
        (
            color,
            if matches!(tone, Tone::Lime | Tone::Warning) {
                Color32::from_rgb(0x0A, 0x0A, 0x14)
            } else {
                Color32::WHITE
            },
            Stroke::NONE,
        )
    } else if hovered {
        (
            surface_inset(),
            color,
            Stroke::new(1.0, color.gamma_multiply(0.65)),
        )
    } else {
        (surface(), text_muted(), Stroke::new(1.0, BORDER))
    };
    let (fill, text_color) = if enabled {
        (fill, text_color)
    } else {
        (fill.gamma_multiply(0.4), TEXT_FAINT)
    };
    if selected && enabled {
        glow(ui.painter(), rect, 4, color, 0.35);
    }
    ui.painter().rect(
        rect,
        CornerRadius::same(4),
        fill,
        border,
        StrokeKind::Inside,
    );
    ui.painter()
        .galley(rect.center() - galley.size() * 0.5, galley, text_color);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, enabled, selected, text.clone())
    });
    response
}

/// A non-interactive status tag with a tinted fill.
pub fn tag(ui: &mut Ui, label: &str, tone: Tone) {
    let color = tone.color();
    let label_text = label.to_uppercase();
    Frame::new()
        .fill(color.gamma_multiply(0.22))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.38)))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(
                RichText::new(label_text)
                    .font(mono_font(10.0))
                    .color(text()),
            );
        });
}

/// A live status dot. Pulse it with [`pulse`] for active sessions.
pub fn status_dot(ui: &mut Ui, color: Color32, activity: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    let center = rect.center();
    ui.painter()
        .circle_filled(center, 3.0, color.gamma_multiply(0.35 + activity * 0.45));
    ui.painter().circle_filled(center, 1.6, color);
}

// ---------------------------------------------------------------------------
// Surfaces and progress
// ---------------------------------------------------------------------------

/// The standard card frame. `elevated` adds the raised fill, accent border,
/// and coloured glow reserved for featured or interactive content.
pub fn card(elevated: bool, accent: Option<Color32>) -> Frame {
    let accent = accent.unwrap_or(primary());
    let mut frame = Frame::new()
        .fill(if elevated { surface_inset() } else { surface() })
        .stroke(Stroke::new(
            1.0,
            if elevated {
                accent.gamma_multiply(0.55)
            } else {
                BORDER
            },
        ))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::same(20));
    if elevated {
        frame = frame.shadow(egui::epaint::Shadow {
            offset: [0, 0],
            blur: 18,
            spread: 0,
            color: accent.gamma_multiply(0.20),
        });
    }
    frame
}

/// Animated neon progress bar used by the download/loading screen.
pub fn neon_progress(ui: &mut Ui, fraction: f32, width: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 12.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(6), SURFACE_RAISED);
    painter.rect_stroke(
        rect,
        CornerRadius::same(6),
        Stroke::new(1.0, BORDER),
        StrokeKind::Inside,
    );
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction > 0.0 {
        let fill = egui::Rect::from_min_size(
            rect.min + Vec2::splat(2.0),
            Vec2::new((rect.width() - 4.0) * fraction, rect.height() - 4.0),
        );
        glow(painter, fill, 5, PINK, 0.8);
        painter.rect_filled(fill, CornerRadius::same(5), PINK);
        // A cyan leading edge reads like a CRT scan head.
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(fill.right() - 3.0, fill.top()),
                Vec2::new(3.0, fill.height()),
            ),
            CornerRadius::same(2),
            CYAN,
        );
    }
}

/// Lay a row of key hints: `F1 Controls · F2 Tools`.
pub fn shortcut_hint(ui: &mut Ui, hints: &[(&str, &str)]) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for (index, (key, action)) in hints.iter().enumerate() {
            if index > 0 {
                ui.label(RichText::new("·").color(TEXT_FAINT));
            }
            keycap(ui, key);
            ui.label(
                RichText::new(*action)
                    .font(body_font(12.0))
                    .color(TEXT_MUTED),
            );
        }
    });
}

/// Centered, full-width empty-state block.
pub fn empty_state(ui: &mut Ui, glyph: &str, title: &str, body: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(XXL);
        ui.label(
            RichText::new(glyph)
                .font(display_font(40.0))
                .color(PINK.gamma_multiply(0.55)),
        );
        ui.add_space(SM);
        ui.label(display(title, 24.0));
        ui.add_space(XS);
        ui.label(RichText::new(body).font(body_font(14.0)).color(TEXT_MUTED));
        ui.add_space(XXL);
    });
}

/// A responsive grid of equal-width cells. Returns the cell size so callers
/// can draw covers before laying out the rest of the card.
pub fn grid_cell(ui: &Ui, minimum: f32, maximum: f32, gap: f32) -> (usize, f32) {
    let width = ui.available_width();
    let columns = (((width + gap) / (minimum + gap)).floor() as usize).max(1);
    let cell = ((width - gap * (columns.saturating_sub(1)) as f32) / columns as f32)
        .clamp(minimum.min(maximum), maximum);
    (columns, cell)
}

/// Right-aligns content in a horizontal row.
pub fn right_aligned<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.with_layout(Layout::right_to_left(Align::Center), add)
        .inner
}

/// A small labelled value used by the performance and session panels.
pub fn stat(ui: &mut Ui, label: &str, value: &str, color: Color32) {
    Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(eyebrow(label));
            ui.add_space(XS);
            ui.label(RichText::new(value).font(mono_font(26.0)).color(color));
        });
}

/// Convenience for centring a line of text inside a known rect.
pub fn centered_text(ui: &Ui, rect: egui::Rect, text: &str, font: FontId, color: Color32) {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color);
    ui.painter()
        .galley(rect.center() - galley.size() * 0.5, galley, color);
}

#[cfg(test)]
mod tests {
    #[test]
    fn workspace_fonts_cover_labels_and_accents() {
        let context = egui::Context::default();
        context
            .run_ui(egui::RawInput::default(), |ui| {
                super::apply(ui.ctx());
                ui.fonts_mut(|fonts| {
                    for font in [
                        egui::FontId::proportional(14.0),
                        egui::FontId::monospace(14.0),
                        super::display_font(28.0),
                    ] {
                        assert!(fonts.has_glyphs(&font, "fanplay Pokédex Poción Conexión × · −"));
                    }
                });
            })
            .drop_without_applying_deltas();
    }

    #[test]
    fn theme_is_idempotent_and_keeps_the_palette() {
        let context = egui::Context::default();
        for _ in 0..3 {
            context
                .run_ui(egui::RawInput::default(), |ui| super::apply(ui.ctx()))
                .drop_without_applying_deltas();
        }
        context
            .run_ui(egui::RawInput::default(), |ui| {
                assert_eq!(ui.visuals().panel_fill, super::BG);
                assert_eq!(ui.visuals().hyperlink_color, super::CYAN);
                assert_eq!(ui.visuals().error_fg_color, super::ERROR);
            })
            .drop_without_applying_deltas();
    }
}
