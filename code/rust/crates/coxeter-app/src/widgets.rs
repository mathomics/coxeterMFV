//! The controls of `assets/app.css`, on egui: cards, hints, chips, the
//! button variants (`.primary`, `.danger`, `[aria-pressed]`), and form fields
//! with the browser's two events, which is what `onSidebarInput` of
//! `src/ui/app.js` switches on: `input` while the user works a control, and
//! `change` once it is committed (Enter, leaving the field, releasing a
//! slider, closing a colour picker).
//!
//! Every control can carry an accessible name (`aria-label`), which is how
//! the interface tests find it.

use egui::{Color32, CornerRadius, Frame, Margin, Response, RichText, Sense, Stroke, Ui, Vec2, WidgetText};

use crate::theme::{Palette, bold, css_color, size, to_hex};

// Accessibility

/// `aria-label`: the accessible name of a control (what tests find it by).
pub fn aria_label(response: &Response, label: &str) {
    let label = label.to_string();
    response.ctx.accesskit_node_builder(response.id, |node| node.set_label(label));
}

/// `aria-pressed`: a toggle button's state, for assistive technology and tests.
pub fn aria_pressed(response: &Response, pressed: bool) {
    response.ctx.accesskit_node_builder(response.id, |node| {
        node.set_toggled(if pressed { egui::accesskit::Toggled::True } else { egui::accesskit::Toggled::False })
    });
}

/// An accessible description (`title`) that tests can read.
pub fn aria_description(response: &Response, text: &str) {
    let text = text.to_string();
    response.ctx.accesskit_node_builder(response.id, |node| node.set_description(text));
}

// Text

/// Text at a size and colour.
pub fn text(s: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(s).size(size).color(color)
}

/// `<strong>`.
pub fn strong(s: impl Into<String>) -> RichText {
    RichText::new(s).family(bold())
}

/// `.section-label`: small capitals above a group.
pub fn section_label(ui: &mut Ui, s: &str) -> Response {
    let p = Palette::current(ui.ctx());
    ui.label(RichText::new(s.to_uppercase()).size(size::LABEL).family(bold()).color(p.ink_3).extra_letter_spacing(0.5))
}

/// `.hint`: a wrapped explanatory paragraph.
pub fn hint(ui: &mut Ui, s: impl Into<String>) -> Response {
    let p = Palette::current(ui.ctx());
    ui.add(egui::Label::new(text(s, size::HINT, p.ink_2)).wrap())
}

/// A `.hint` with `<strong>` runs: `parts` alternate plain and strong text,
/// starting with plain.
pub fn hint_rich(ui: &mut Ui, parts: &[&str]) -> Response {
    let p = Palette::current(ui.ctx());
    let mut job = egui::text::LayoutJob::default();
    for (i, part) in parts.iter().enumerate() {
        let family = if i % 2 == 1 { bold() } else { egui::FontFamily::Proportional };
        let color = if i % 2 == 1 { p.ink } else { p.ink_2 };
        job.append(part, 0.0, egui::TextFormat { font_id: egui::FontId::new(size::HINT, family), color, ..Default::default() });
    }
    job.wrap.max_width = ui.available_width();
    ui.add(egui::Label::new(job).wrap())
}

/// `.empty`.
pub fn empty(ui: &mut Ui, s: impl Into<String>) -> Response {
    let p = Palette::current(ui.ctx());
    ui.add(egui::Label::new(text(s, 12.5, p.ink_2)).wrap())
}

/// `.inspector-note`.
pub fn note(ui: &mut Ui, s: impl Into<String>) -> Response {
    let p = Palette::current(ui.ctx());
    ui.add(egui::Label::new(text(s, 10.5, p.ink_3)).wrap())
}

/// `<h3>`: a heading inside a card.
pub fn subheading(ui: &mut Ui, s: &str) -> Response {
    let p = Palette::current(ui.ctx());
    ui.add_space(4.0);
    ui.label(RichText::new(s).size(12.5).family(bold()).color(p.ink))
}

/// `.inspector-section-title`.
pub fn section_title(ui: &mut Ui, s: &str) -> Response {
    let p = Palette::current(ui.ctx());
    ui.add_space(8.0);
    let r = ui.label(RichText::new(s.to_uppercase()).size(10.0).family(bold()).color(p.ink_3).extra_letter_spacing(0.6));
    ui.add_space(2.0);
    r
}

/// `<code>`.
pub fn code(s: impl Into<String>) -> RichText {
    RichText::new(s).monospace().size(11.5)
}

// Boxes

/// `.card` in a workspace page: a panel with a title (`<h2>`) and an
/// optional count beside it (`<h2>Species <span>3 active</span></h2>`).
pub fn card<R>(ui: &mut Ui, title: &str, badge: Option<&str>, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = Palette::current(ui.ctx());
    Frame::new()
        .fill(p.panel)
        .stroke(Stroke::new(1.0, p.line))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).size(13.0).family(bold()).color(p.ink));
                if let Some(badge) = badge {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(text(badge, 11.0, p.ink_3));
                    });
                }
            });
            ui.add_space(6.0);
            add(ui)
        })
        .inner
}

/// A bordered box on the secondary surface (`.species-item`,
/// `.model-editor-form`, `.pathway-manager-item`…).
pub fn boxed<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = Palette::current(ui.ctx());
    Frame::new()
        .fill(p.panel_2)
        .stroke(Stroke::new(1.0, p.line))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(10, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// A tinted message box (`.analysis-message.is-success`, `.is-error`…).
pub fn tinted<R>(ui: &mut Ui, fill: Color32, line: Color32, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, line))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// `.chip`: a small rounded tag, optionally coloured (`fill`, `text`,
/// `stroke` as CSS colours).
pub fn chip(ui: &mut Ui, label: &str, colors: Option<(&str, &str, &str)>) -> Response {
    let p = Palette::current(ui.ctx());
    let (fill, ink, line) = match colors {
        Some((f, t, s)) => (css_color(f, p.panel_2), css_color(t, p.ink_2), css_color(s, p.line)),
        None => (p.panel_2, p.ink_2, p.line),
    };
    Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, line))
        .corner_radius(CornerRadius::same(99))
        .inner_margin(Margin::symmetric(7, 2))
        .show(ui, |ui| ui.label(RichText::new(label.to_uppercase()).size(10.0).family(bold()).color(ink)))
        .response
}

/// A clickable chip (`<button class="chip">`), e.g. a gene or metabolite link.
pub fn chip_button(ui: &mut Ui, label: &str, active: bool) -> Response {
    let p = Palette::current(ui.ctx());
    let (fill, ink, line) = if active { (p.accent_soft, p.accent, p.accent_line) } else { (p.panel_2, p.ink_2, p.line) };
    ui.add(
        egui::Button::new(RichText::new(label).size(11.0).family(bold()).color(ink))
            .fill(fill)
            .stroke(Stroke::new(1.0, line))
            .corner_radius(CornerRadius::same(99))
            .min_size(Vec2::new(0.0, 20.0)),
    )
}

/// A round colour dot (`.species-swatch`).
pub fn swatch(ui: &mut Ui, color: Color32, diameter: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(diameter), Sense::hover());
    ui.painter().circle_filled(rect.center(), diameter / 2.0, color);
    response
}

/// A square legend swatch with a border (`.legend-swatch`).
pub fn square_swatch(ui: &mut Ui, fill: Color32, stroke: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
    ui.painter().rect(rect, CornerRadius::same(4), fill, Stroke::new(2.0, stroke), egui::StrokeKind::Inside);
    response
}

// Buttons

/// The look of a `<button>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Look {
    #[default]
    Normal,
    /// `.primary`.
    Primary,
    /// `.danger`.
    Danger,
    /// `.danger` on the danger surface (`.inspector-delete`).
    DangerBox,
    /// `.secondary`.
    Secondary,
    /// `[aria-pressed="true"]` / `.is-active` on a toggle.
    Pressed,
    /// A toggle that is off (`aria-pressed="false"`).
    Unpressed,
    /// `.is-active` on a tool or segmented button: accent on the soft surface.
    Active,
    /// No border or fill until hovered (toolbar icons, tabs).
    Flat,
}

/// A `<button>`: label, look, enabled, `title`, full width.
#[derive(Debug, Clone)]
pub struct Btn<'a> {
    pub label: WidgetText,
    pub look: Look,
    pub enabled: bool,
    pub title: Option<&'a str>,
    pub aria: Option<&'a str>,
    pub wide: bool,
    pub min_height: f32,
}

impl<'a> Btn<'a> {
    pub fn new(label: impl Into<WidgetText>) -> Btn<'a> {
        Btn { label: label.into(), look: Look::Normal, enabled: true, title: None, aria: None, wide: false, min_height: 28.0 }
    }
    pub fn look(mut self, look: Look) -> Self {
        self.look = look;
        self
    }
    pub fn primary(self) -> Self {
        self.look(Look::Primary)
    }
    pub fn danger(self) -> Self {
        self.look(Look::Danger)
    }
    /// `aria-pressed`.
    pub fn pressed(self, on: bool) -> Self {
        self.look(if on { Look::Pressed } else { Look::Unpressed })
    }
    /// `.is-active`.
    pub fn active(self, on: bool) -> Self {
        self.look(if on { Look::Active } else { Look::Normal })
    }
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }
    /// `title`: shown on hover (also when disabled, where it says why).
    pub fn title(mut self, title: &'a str) -> Self {
        self.title = (!title.is_empty()).then_some(title);
        self
    }
    /// `aria-label`.
    pub fn aria(mut self, label: &'a str) -> Self {
        self.aria = Some(label);
        self
    }
    /// `.grow` / `.wide`: take the row's width.
    pub fn wide(mut self) -> Self {
        self.wide = true;
        self
    }
    pub fn height(mut self, h: f32) -> Self {
        self.min_height = h;
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let p = Palette::current(ui.ctx());
        let hovered = self.enabled && hovered_next(ui);
        let (fill, ink, line) = match (self.look, hovered) {
            (Look::Normal | Look::Unpressed, false) => (p.panel, p.ink, p.line),
            (Look::Normal | Look::Unpressed, true) => (p.hover, p.ink, p.ink_3),
            (Look::Primary | Look::Pressed, false) => (p.accent, p.accent_ink, Color32::TRANSPARENT),
            (Look::Primary | Look::Pressed, true) => (darken(p.accent, 0.88), p.accent_ink, Color32::TRANSPARENT),
            (Look::Danger, false) => (p.panel, p.danger, p.line),
            (Look::Danger, true) => (p.danger_bg, p.danger, p.danger_line),
            (Look::DangerBox, _) => (p.danger_bg, p.danger, p.danger_line),
            (Look::Secondary, false) => (p.panel_2, p.ink_2, p.line),
            (Look::Secondary, true) => (p.hover, p.ink, p.ink_3),
            (Look::Active, false) => (p.accent_soft, p.accent, p.accent_line),
            (Look::Active, true) => (p.accent_soft, p.accent, p.accent),
            (Look::Flat, false) => (Color32::TRANSPARENT, p.ink_2, Color32::TRANSPARENT),
            (Look::Flat, true) => (p.hover, p.ink, Color32::TRANSPARENT),
        };
        let label = match self.label {
            WidgetText::Text(s) => WidgetText::RichText(RichText::new(s).size(size::BUTTON).color(ink).into()),
            other => other,
        };
        let mut button = egui::Button::new(label)
            .fill(fill)
            .stroke(Stroke::new(1.0, line))
            .corner_radius(CornerRadius::same(8))
            .min_size(Vec2::new(0.0, self.min_height));
        if self.wide {
            button = button.min_size(Vec2::new(ui.available_width(), self.min_height));
        }
        let response = ui.add_enabled(self.enabled, button);
        if let Some(aria) = self.aria {
            aria_label(&response, aria);
        }
        if matches!(self.look, Look::Pressed | Look::Unpressed) {
            aria_pressed(&response, self.look == Look::Pressed);
        }
        match self.title {
            Some(title) if self.enabled => response.on_hover_text(title),
            Some(title) => response.on_disabled_hover_text(title),
            None => response,
        }
    }
}

/// Is the widget `ui` is about to add under the pointer? It asks for last
/// frame's response of the next automatic id, which is the id the next
/// button gets. `Button::fill` overrides egui's own hover look, so every
/// button with its own colours (the CSS `:hover` rules) picks its hover colour
/// from this; egui repaints when the hover changes, so the frame of lag never
/// shows.
pub fn hovered_next(ui: &Ui) -> bool {
    ui.ctx().read_response(ui.next_auto_id()).is_some_and(|r| r.hovered())
}

/// `c` with its colour channels scaled by `f` (alpha kept): a darker shade
/// for a hovered accent button.
pub fn darken(c: Color32, f: f32) -> Color32 {
    let k = |v: u8| (v as f32 * f).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgba_premultiplied(k(c.r()), k(c.g()), k(c.b()), c.a())
}

/// A plain `<button>`.
pub fn button(ui: &mut Ui, label: impl Into<WidgetText>) -> Response {
    Btn::new(label).show(ui)
}

/// `<button class="primary">`.
pub fn primary_button(ui: &mut Ui, label: impl Into<WidgetText>) -> Response {
    Btn::new(label).primary().show(ui)
}

/// `<button class="danger">`.
pub fn danger_button(ui: &mut Ui, label: impl Into<WidgetText>) -> Response {
    Btn::new(label).danger().show(ui)
}

/// A toggle (`<button aria-pressed>`).
pub fn toggle_button(ui: &mut Ui, label: impl Into<WidgetText>, pressed: bool, title: &str) -> Response {
    Btn::new(label).pressed(pressed).title(title).show(ui)
}

/// A link-styled button (`.table-link`).
pub fn link_button(ui: &mut Ui, label: &str) -> Response {
    let p = Palette::current(ui.ctx());
    ui.add(egui::Button::new(RichText::new(label).color(p.accent).size(12.0)).frame(false))
}

/// A row of buttons that share the width equally (`.row` of `.grow` buttons).
pub fn grow_row(ui: &mut Ui, count: usize, mut add: impl FnMut(&mut Ui, usize)) {
    if count == 0 {
        return;
    }
    let gap = ui.spacing().item_spacing.x;
    let width = ((ui.available_width() - gap * (count as f32 - 1.0)) / count as f32).max(10.0);
    ui.horizontal_top(|ui| {
        for i in 0..count {
            ui.allocate_ui_with_layout(Vec2::new(width, 28.0), egui::Layout::top_down_justified(egui::Align::Center), |ui| {
                ui.set_width(width);
                add(ui, i)
            });
        }
    });
}

// Fields

/// What a field did this frame: the browser's `input` and `change` events.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum FieldEvent<T> {
    #[default]
    None,
    /// The value is being worked (typing, dragging).
    Input(T),
    /// The value was committed (Enter, blur, release).
    Change(T),
}

impl<T> FieldEvent<T> {
    pub fn is_none(&self) -> bool {
        matches!(self, FieldEvent::None)
    }
    pub fn is_change(&self) -> bool {
        matches!(self, FieldEvent::Change(_))
    }
    /// The value of either event.
    pub fn value(self) -> Option<T> {
        match self {
            FieldEvent::None => None,
            FieldEvent::Input(v) | FieldEvent::Change(v) => Some(v),
        }
    }
    /// The value of a `change` only.
    pub fn changed(self) -> Option<T> {
        match self {
            FieldEvent::Change(v) => Some(v),
            _ => None,
        }
    }
}

/// Options of a text or number field.
#[derive(Debug, Clone, Default)]
pub struct Field<'a> {
    pub width: Option<f32>,
    pub placeholder: &'a str,
    pub aria: Option<&'a str>,
    pub enabled: bool,
    /// `aria-invalid="true"`: drawn with the danger border.
    pub invalid: bool,
    pub multiline: Option<usize>,
    pub title: Option<&'a str>,
}

impl<'a> Field<'a> {
    pub fn new() -> Field<'a> {
        Field { enabled: true, ..Default::default() }
    }
    pub fn width(mut self, w: f32) -> Self {
        self.width = Some(w);
        self
    }
    pub fn placeholder(mut self, s: &'a str) -> Self {
        self.placeholder = s;
        self
    }
    pub fn aria(mut self, s: &'a str) -> Self {
        self.aria = Some(s);
        self
    }
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }
    pub fn invalid(mut self, on: bool) -> Self {
        self.invalid = on;
        self
    }
    pub fn rows(mut self, rows: usize) -> Self {
        self.multiline = Some(rows);
        self
    }
    pub fn title(mut self, s: &'a str) -> Self {
        self.title = Some(s);
        self
    }
}

#[derive(Clone, Default)]
struct EditBuffer {
    text: String,
    /// The value shown when editing began: a field the store changed under
    /// the user (a slider beside it) takes the new value once left.
    origin: String,
    /// The pass it was last written in: a field that vanished while edited
    /// (its panel was replaced) is not committed when it comes back.
    pass: u64,
}

/// An `<input type="text">` (or `<textarea>`) showing `value`.
///
/// While it has the keyboard it keeps what the user types (`input` on every
/// edit); Enter (not in a textarea) or leaving it commits (`change`). When it
/// does not have the keyboard it shows `value` as the caller has it now, as a
/// re-rendered panel would.
pub fn text_field(ui: &mut Ui, id: egui::Id, value: &str, opts: Field) -> (Response, FieldEvent<String>) {
    let p = Palette::current(ui.ctx());
    let pass = ui.ctx().cumulative_pass_nr();
    let buffer: Option<EditBuffer> = ui.data(|d| d.get_temp::<EditBuffer>(id)).filter(|b| b.pass + 2 >= pass);
    let mut text = buffer.as_ref().map_or_else(|| value.to_string(), |b| b.text.clone());
    let before = text.clone();
    let mut edit = match opts.multiline {
        Some(rows) => egui::TextEdit::multiline(&mut text).desired_rows(rows),
        None => egui::TextEdit::singleline(&mut text),
    }
    .id(id)
    .hint_text(opts.placeholder)
    .margin(Margin::symmetric(8, 5))
    .background_color(p.panel);
    edit = edit.desired_width(opts.width.unwrap_or(f32::INFINITY));
    let frame_color = if opts.invalid { p.danger } else { p.line };
    let response = ui
        .scope(|ui| {
            ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::new(1.0, frame_color);
            ui.visuals_mut().widgets.hovered.bg_stroke = Stroke::new(1.0, if opts.invalid { p.danger } else { p.ink_3 });
            ui.add_enabled(opts.enabled, edit)
        })
        .inner;
    if let Some(aria) = opts.aria {
        aria_label(&response, aria);
    }
    let response = match opts.title {
        Some(t) => response.on_hover_text(t),
        None => response,
    };
    let mut event = FieldEvent::None;
    let origin = buffer.as_ref().map_or_else(|| value.to_string(), |b| b.origin.clone());
    if response.has_focus() {
        if text != before {
            event = FieldEvent::Input(text.clone());
        }
        ui.data_mut(|d| d.insert_temp(id, EditBuffer { text: text.clone(), origin, pass }));
    } else if buffer.is_some() || response.lost_focus() {
        // Blur (or Enter) commits what was typed, if it differs from what
        // was shown (the browser fires no `change` otherwise).
        ui.data_mut(|d| d.remove::<EditBuffer>(id));
        if text != origin {
            event = FieldEvent::Change(text);
        }
    }
    (response, event)
}

/// An `<input type="number">` showing `value` (the text the panel would
/// render). Events carry the text as typed: `""`, `"-"` and `"1e"` are the
/// caller's to judge, as `valueAsNumber` is NaN for them.
pub fn number_field(ui: &mut Ui, id: egui::Id, value: &str, opts: Field) -> (Response, FieldEvent<String>) {
    let width = opts.width.unwrap_or(72.0);
    text_field(ui, id, value, Field { width: Some(width), ..opts })
}

/// `el.valueAsNumber`: the number a number field holds, or NaN.
pub fn value_as_number(text: &str) -> f64 {
    let t = text.trim();
    if t.is_empty() {
        return f64::NAN;
    }
    coxeter_core::js::string_to_number(t)
}

/// An `<input type="range">`. `Input` while dragging (and on every arrow
/// key), `Change` when the drag ends or a key moved it.
pub fn slider(ui: &mut Ui, value: f64, min: f64, max: f64, step: f64, enabled: bool, aria: &str) -> (Response, FieldEvent<f64>) {
    let mut v = value;
    let (lo, hi) = if min <= max { (min, max) } else { (max, min) };
    let mut s = egui::Slider::new(&mut v, lo..=hi).show_value(false);
    if step > 0.0 && step.is_finite() {
        s = s.step_by(step);
    }
    let response = ui.add_enabled(enabled, s);
    aria_label(&response, aria);
    // egui reports a change when it snaps a value to the step as it draws
    // the slider; only a change the user made is an event.
    let id = response.id;
    let by_user = response.dragged()
        || response.drag_stopped()
        || response.has_focus()
        || ui.input(|i| {
            use egui::accesskit::Action;
            [Action::SetValue, Action::Increment, Action::Decrement].into_iter().any(|a| i.has_accesskit_action_request(id, a))
        });
    if !by_user {
        return (response, FieldEvent::None);
    }
    let event = if response.drag_stopped() {
        FieldEvent::Change(v)
    } else if response.changed() {
        if response.dragged() { FieldEvent::Input(v) } else { FieldEvent::Change(v) }
    } else {
        FieldEvent::None
    };
    (response, event)
}

/// A `<select>`: `current` among `options` (`(value, label)`). Returns the
/// value picked (the `change` event).
pub fn select<T: PartialEq + Clone>(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    current: &T,
    options: &[(T, String)],
    width: f32,
    enabled: bool,
    aria: &str,
) -> Option<T> {
    let selected = options.iter().find(|(v, _)| v == current).map(|(_, l)| l.clone()).unwrap_or_default();
    let mut picked = None;
    let response = ui
        .add_enabled_ui(enabled, |ui| {
            egui::ComboBox::from_id_salt(id_salt).selected_text(selected).width(width).truncate().show_ui(ui, |ui| {
                for (value, label) in options {
                    if ui.selectable_label(value == current, label).clicked() && value != current {
                        picked = Some(value.clone());
                    }
                }
            })
        })
        .inner
        .response;
    aria_label(&response, aria);
    picked
}

/// A `<select>` with `<optgroup>`s: `groups` of `(group label, options)`.
pub fn select_grouped(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    current: &str,
    groups: &[(String, Vec<(String, String)>)],
    width: f32,
    aria: &str,
) -> Option<String> {
    let selected = groups.iter().flat_map(|(_, o)| o).find(|(v, _)| v == current).map(|(_, l)| l.clone()).unwrap_or_default();
    let mut picked = None;
    let p = Palette::current(ui.ctx());
    let response = egui::ComboBox::from_id_salt(id_salt).selected_text(selected).width(width).truncate().show_ui(ui, |ui| {
        for (group, options) in groups {
            ui.label(RichText::new(group).size(10.5).family(bold()).color(p.ink_3));
            for (value, label) in options {
                if ui.selectable_label(value == current, label).clicked() && value != current {
                    picked = Some(value.clone());
                }
            }
        }
    });
    aria_label(&response.response, aria);
    picked
}

/// A `<select>` with `<optgroup>`s whose options may be disabled: `groups`
/// of `(group label, options)`, each option `(value, label, enabled)`.
pub fn select_grouped_enabled(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    current: &str,
    groups: &[(String, Vec<(String, String, bool)>)],
    width: f32,
    enabled: bool,
    aria: &str,
) -> Option<String> {
    let selected = groups.iter().flat_map(|(_, o)| o).find(|(v, _, _)| v == current).map(|(_, l, _)| l.clone()).unwrap_or_default();
    let mut picked = None;
    let p = Palette::current(ui.ctx());
    let response = ui
        .add_enabled_ui(enabled, |ui| {
            egui::ComboBox::from_id_salt(id_salt).selected_text(selected).width(width).truncate().show_ui(ui, |ui| {
                for (group, options) in groups {
                    ui.label(RichText::new(group).size(10.5).family(bold()).color(p.ink_3));
                    for (value, label, on) in options {
                        let r = ui.add_enabled(*on, egui::Button::selectable(value == current, label.as_str()));
                        if r.clicked() && value != current {
                            picked = Some(value.clone());
                        }
                    }
                }
            })
        })
        .inner
        .response;
    aria_label(&response, aria);
    picked
}

/// An `<input type="checkbox">` with its label. Returns the new state when clicked.
pub fn checkbox(ui: &mut Ui, checked: bool, label: impl Into<WidgetText>, enabled: bool, aria: Option<&str>) -> (Response, Option<bool>) {
    let mut on = checked;
    let response = ui.add_enabled(enabled, egui::Checkbox::new(&mut on, label));
    if let Some(aria) = aria {
        aria_label(&response, aria);
    }
    let changed = (on != checked).then_some(on);
    (response, changed)
}

#[derive(Clone)]
struct ColorDraft {
    color: Color32,
    origin: Color32,
}

/// An `<input type="color">` with its hex beside it. The swatch opens a
/// picker; the colour is applied when the picker closes (`change`), so one
/// choice is one edit rather than one per pointer move. The hex field
/// applies on Enter or blur.
pub fn color_field(ui: &mut Ui, id: egui::Id, value: &str, aria: &str) -> Option<String> {
    let p = Palette::current(ui.ctx());
    let current = css_color(value, p.ink_3);
    let popup_id = id.with("popup");
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let mut draft: Option<ColorDraft> = ui.data(|d| d.get_temp(id));
    let shown = draft.as_ref().filter(|_| open).map_or(current, |d| d.color);
    let mut committed = None;
    ui.horizontal(|ui| {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(30.0, 22.0), Sense::click());
        ui.painter().rect(rect, CornerRadius::same(5), shown, Stroke::new(1.0, p.line), egui::StrokeKind::Inside);
        aria_label(&response, &format!("{aria} picker"));
        egui::Popup::menu(&response).id(popup_id).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            let d = draft.get_or_insert(ColorDraft { color: current, origin: current });
            let mut c = d.color;
            egui::color_picker::color_picker_color32(ui, &mut c, egui::color_picker::Alpha::Opaque);
            d.color = c;
            ui.data_mut(|m| m.insert_temp(id, d.clone()));
        });
        let now_open = egui::Popup::is_id_open(ui.ctx(), popup_id);
        if open && !now_open {
            if let Some(d) = draft.take() {
                if d.color != d.origin {
                    committed = Some(to_hex(d.color));
                }
            }
            ui.data_mut(|m| m.remove::<ColorDraft>(id));
        }
        let (_, event) = text_field(ui, id.with("hex"), &to_hex(current), Field::new().width(70.0).aria(aria));
        if let FieldEvent::Change(hex) = event {
            let parsed = css_color(&hex, Color32::TRANSPARENT);
            if parsed != Color32::TRANSPARENT && parsed != current {
                committed = Some(to_hex(parsed));
            }
        }
    });
    committed
}

// Read-outs

/// `.property-row`: a label and its value in two columns (inside an
/// `egui::Grid`), or stacked (`is-stacked`).
pub fn property_row(ui: &mut Ui, label: &str, stacked: bool, value: impl FnOnce(&mut Ui)) {
    let p = Palette::current(ui.ctx());
    if stacked {
        ui.vertical(|ui| {
            ui.label(text(label, 11.0, p.ink_3));
            value(ui);
        });
    } else {
        ui.horizontal_top(|ui| {
            let w = 104.0_f32.min(ui.available_width() * 0.45);
            ui.allocate_ui_with_layout(Vec2::new(w, 16.0), egui::Layout::left_to_right(egui::Align::TOP), |ui| {
                ui.set_min_width(w);
                ui.add(egui::Label::new(text(label, 11.0, p.ink_3)).wrap());
            });
            ui.vertical(|ui| value(ui));
        });
    }
    ui.add(egui::Separator::default().spacing(4.0));
}

/// A wrapped value in a property row.
pub fn value_text(ui: &mut Ui, s: impl Into<String>) -> Response {
    ui.add(egui::Label::new(RichText::new(s).size(12.0)).wrap())
}

/// `.readout`: boxed figures side by side (`<dt>` over `<dd>`).
pub fn readout(ui: &mut Ui, items: &[(&str, String)]) {
    let p = Palette::current(ui.ctx());
    grow_row(ui, items.len(), |ui, i| {
        Frame::new()
            .fill(p.panel)
            .stroke(Stroke::new(1.0, p.line))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(9, 7))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(items[i].0.to_uppercase()).size(10.5).color(p.ink_3));
                ui.label(RichText::new(&items[i].1).size(15.0).family(bold()));
            });
    });
}

/// A horizontal bar with segments `(fraction 0..1 left, width)`: `.range-bar`.
pub fn range_bar(ui: &mut Ui, left: f32, width: f32, mark: f32) {
    let p = Palette::current(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 12.0), Sense::hover());
    let bar = egui::Rect::from_min_size(rect.min + Vec2::new(0.0, 3.0), Vec2::new(rect.width(), 6.0));
    ui.painter().rect_filled(bar, CornerRadius::same(3), p.line);
    let x0 = bar.min.x + bar.width() * left.clamp(0.0, 1.0);
    let w = bar.width() * width.clamp(0.0, 1.0);
    ui.painter().rect_filled(egui::Rect::from_min_size(egui::pos2(x0, bar.min.y), Vec2::new(w, 6.0)), CornerRadius::same(3), p.accent.gamma_multiply(0.55));
    let mx = bar.min.x + bar.width() * mark.clamp(0.0, 1.0);
    ui.painter().rect_filled(egui::Rect::from_min_size(egui::pos2(mx - 1.0, rect.min.y), Vec2::new(2.0, 12.0)), CornerRadius::ZERO, p.ink);
}

#[cfg(test)]
mod hover_tests {
    /// `hovered_next` reads the id the next button really gets, enabled or not.
    #[test]
    fn hovered_next_names_the_next_button() {
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            let predicted = ui.next_auto_id();
            let r = super::Btn::new("Map JSON").show(ui);
            assert_eq!(predicted, r.id);
            let predicted = ui.next_auto_id();
            let r = ui.add_enabled(false, egui::Button::new("Disabled"));
            assert_eq!(predicted, r.id);
        });
        harness.run();
    }
}
