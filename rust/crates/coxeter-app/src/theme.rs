//! Light and dark themes: the custom properties of `assets/app.css`
//! (`:root` and `:root[data-theme="dark"]`) as an egui style.
//!
//! The map's own colours are not here: they come from the stylesheets
//! `coxeter-render` resolves (`assets/map.css`), through the canvas.

use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Visuals};

/// `var(--…)` of `app.css`, for one theme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub dark: bool,
    pub bg: Color32,
    pub panel: Color32,
    pub panel_2: Color32,
    pub ink: Color32,
    pub ink_2: Color32,
    pub ink_3: Color32,
    pub line: Color32,
    pub accent: Color32,
    pub accent_ink: Color32,
    pub danger: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub series_1: Color32,
    pub series_2: Color32,
    pub hover: Color32,
    pub accent_soft: Color32,
    pub accent_line: Color32,
    pub success_bg: Color32,
    pub success_line: Color32,
    pub danger_bg: Color32,
    pub danger_line: Color32,
    pub map_surface: Color32,
    pub map_surround: Color32,
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

impl Palette {
    /// `:root`.
    pub const LIGHT: Palette = Palette {
        dark: false,
        bg: hex(0xf3f6fa),
        panel: hex(0xffffff),
        panel_2: hex(0xf8fafc),
        ink: hex(0x0f172a),
        ink_2: hex(0x475569),
        ink_3: hex(0x5b6b80),
        line: hex(0xdde5ef),
        accent: hex(0x2563eb),
        accent_ink: hex(0xffffff),
        danger: hex(0xbe123c),
        ok: hex(0x15803d),
        warn: hex(0xa85b08),
        series_1: hex(0x2a78d6),
        series_2: hex(0xeb6834),
        hover: hex(0xedf2f8),
        accent_soft: hex(0xeaf2ff),
        accent_line: hex(0xa8c8fa),
        success_bg: hex(0xeffbf3),
        success_line: hex(0xb5e6c6),
        danger_bg: hex(0xfff2f3),
        danger_line: hex(0xf5bec8),
        map_surface: hex(0xffffff),
        map_surround: hex(0xf3f6fa),
    };

    /// `:root[data-theme="dark"]`.
    pub const DARK: Palette = Palette {
        dark: true,
        bg: hex(0x0b1220),
        panel: hex(0x141d2d),
        panel_2: hex(0x192435),
        ink: hex(0xecf1f8),
        ink_2: hex(0xbac7d9),
        ink_3: hex(0x9eafc6),
        line: hex(0x334157),
        accent: hex(0x60a5fa),
        accent_ink: hex(0x0b1220),
        danger: hex(0xfb7185),
        ok: hex(0x4ade80),
        warn: hex(0xfbbf24),
        series_1: hex(0x3987e5),
        series_2: hex(0xd95926),
        hover: hex(0x243349),
        accent_soft: hex(0x1d3454),
        accent_line: hex(0x477cbe),
        success_bg: hex(0x102d25),
        success_line: hex(0x285f46),
        danger_bg: hex(0x351d2b),
        danger_line: hex(0x7d374a),
        map_surface: hex(0x0f172a),
        map_surround: hex(0x0b1220),
    };

    pub fn of(dark: bool) -> Palette {
        if dark { Palette::DARK } else { Palette::LIGHT }
    }

    /// The palette of the theme egui is drawing with.
    pub fn current(ctx: &egui::Context) -> Palette {
        Palette::of(ctx.theme() == egui::Theme::Dark)
    }
}

/// A CSS colour (`#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb(…)`, `rgba(…)`) as egui
/// draws it; `fallback` when it cannot be read.
pub fn css_color(value: &str, fallback: Color32) -> Color32 {
    let v = value.trim();
    if let Some(h) = v.strip_prefix('#') {
        let digit = |c: char| c.to_digit(16).map(|d| d as u8);
        let chars: Vec<char> = h.chars().collect();
        let parse = |a: usize, b: usize| Some(digit(chars[a])? * 16 + digit(chars[b])?);
        return match chars.len() {
            3 => (|| Some(Color32::from_rgb(digit(chars[0])? * 17, digit(chars[1])? * 17, digit(chars[2])? * 17)))(),
            6 => (|| Some(Color32::from_rgb(parse(0, 1)?, parse(2, 3)?, parse(4, 5)?)))(),
            8 => (|| Some(Color32::from_rgba_unmultiplied(parse(0, 1)?, parse(2, 3)?, parse(4, 5)?, parse(6, 7)?)))(),
            _ => None,
        }
        .unwrap_or(fallback);
    }
    let inner = v.strip_prefix("rgba(").or_else(|| v.strip_prefix("rgb(")).and_then(|s| s.strip_suffix(')'));
    if let Some(inner) = inner {
        let parts: Vec<f32> = inner.split([',', ' ', '/']).filter(|p| !p.is_empty()).filter_map(|p| p.trim().parse().ok()).collect();
        if parts.len() >= 3 {
            let a = parts.get(3).map_or(255, |a| if *a <= 1.0 { (a * 255.0).round() as u8 } else { *a as u8 });
            return Color32::from_rgba_unmultiplied(parts[0] as u8, parts[1] as u8, parts[2] as u8, a);
        }
    }
    fallback
}

/// `#rrggbb` of a colour (what `<input type="color">` holds).
pub fn to_hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

/// The font sizes of `app.css`, in points.
pub mod size {
    /// `body { font-size: 14px }`.
    pub const BODY: f32 = 13.5;
    /// `button { font-size: 12.5px }`.
    pub const BUTTON: f32 = 12.5;
    /// `.hint { font-size: 12px }`.
    pub const HINT: f32 = 12.0;
    /// `.card > h2 { font-size: 11px }`.
    pub const CARD_TITLE: f32 = 11.0;
    /// `.section-label { font-size: 10.5px }`.
    pub const LABEL: f32 = 10.5;
    pub const SMALL: f32 = 11.0;
    pub const HEADING: f32 = 15.0;
}

/// The bold face (`font-weight: 700`), a family of its own in egui.
pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

/// Apply a theme to the whole interface.
pub fn apply(ctx: &egui::Context, dark: bool) {
    let p = Palette::of(dark);
    ctx.set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
    ctx.style_mut_of(if dark { egui::Theme::Dark } else { egui::Theme::Light }, |style| {
        let mut v = if dark { Visuals::dark() } else { Visuals::light() };
        v.dark_mode = dark;
        v.override_text_color = Some(p.ink);
        v.panel_fill = p.panel;
        v.window_fill = p.panel;
        v.window_stroke = Stroke::new(1.0, p.line);
        v.window_corner_radius = CornerRadius::same(10);
        v.menu_corner_radius = CornerRadius::same(10);
        v.extreme_bg_color = p.panel;
        v.faint_bg_color = p.panel_2;
        v.code_bg_color = p.panel_2;
        v.hyperlink_color = p.accent;
        v.warn_fg_color = p.warn;
        v.error_fg_color = p.danger;
        v.selection.bg_fill = p.accent.gamma_multiply(0.35);
        v.selection.stroke = Stroke::new(1.0, p.accent);
        let radius = CornerRadius::same(8);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = p.panel;
        w.noninteractive.weak_bg_fill = p.panel;
        w.noninteractive.bg_stroke = Stroke::new(1.0, p.line);
        w.noninteractive.fg_stroke = Stroke::new(1.0, p.ink_2);
        w.noninteractive.corner_radius = radius;
        w.inactive.bg_fill = p.panel;
        w.inactive.weak_bg_fill = p.panel;
        w.inactive.bg_stroke = Stroke::new(1.0, p.line);
        w.inactive.fg_stroke = Stroke::new(1.0, p.ink);
        w.inactive.corner_radius = radius;
        w.hovered.bg_fill = p.hover;
        w.hovered.weak_bg_fill = p.hover;
        w.hovered.bg_stroke = Stroke::new(1.0, p.ink_3);
        w.hovered.fg_stroke = Stroke::new(1.0, p.ink);
        w.hovered.corner_radius = radius;
        w.hovered.expansion = 0.0;
        w.active.bg_fill = p.accent_soft;
        w.active.weak_bg_fill = p.accent_soft;
        w.active.bg_stroke = Stroke::new(1.0, p.accent_line);
        w.active.fg_stroke = Stroke::new(1.0, p.accent);
        w.active.corner_radius = radius;
        w.active.expansion = 0.0;
        w.open.bg_fill = p.panel;
        w.open.weak_bg_fill = p.panel;
        w.open.bg_stroke = Stroke::new(1.0, p.accent_line);
        w.open.fg_stroke = Stroke::new(1.0, p.ink);
        w.open.corner_radius = radius;
        v.slider_trailing_fill = true;
        v.striped = true;
        style.visuals = v;

        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 4.0);
        style.spacing.interact_size = egui::vec2(28.0, 26.0);
        style.spacing.combo_width = 140.0;
        style.spacing.text_edit_width = 120.0;
        style.spacing.slider_width = 120.0;
        style.spacing.menu_margin = egui::Margin::same(6);
        style.text_styles = [
            (TextStyle::Small, FontId::new(size::SMALL, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(size::BODY, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(size::BUTTON, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(size::HEADING, bold())),
            (TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace)),
        ]
        .into();
        style.interaction.tooltip_delay = 0.35;
    });
}
