//! Small custom widgets built on the design tokens.

use egui::{Align, Align2, Color32, CornerRadius, Layout, Rect, Response, Sense, Stroke, vec2};

use crate::theme::{self, Tokens};
use crate::{PdfCraftApp, icons, rtl};

mod bidi_edit;
mod form;
pub use bidi_edit::BidiEdit;
pub use form::{FormRows, form_grid, start_block};

/// Horizontal and vertical inset of a single-line field.
const FIELD_MARGIN: egui::Margin = egui::Margin::symmetric(10, 6);
/// Outer height of a single-line field, including its inset.
const FIELD_MIN_HEIGHT: f32 = 30.0;
/// Padding inside every modal.
const DIALOG_PAD_X: i8 = 20;
const DIALOG_PAD_Y: i8 = 16;

fn dialog_frame(ctx: &egui::Context) -> egui::Frame {
    let t = Tokens::get(ctx);
    egui::Frame::new()
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(DIALOG_PAD_X, DIALOG_PAD_Y))
        .shadow(ctx.global_style().visuals.window_shadow)
}

/// A modal with the card fill, a 10-point corner and room around its contents.
pub fn modal(ctx: &egui::Context, id: impl std::hash::Hash + std::fmt::Debug) -> egui::Modal {
    egui::Modal::new(egui::Id::new(id)).frame(dialog_frame(ctx))
}

/// Show [`modal`], outline its controls, and remember the card rect for layout checks.
///
/// A dialog's body keeps the top-down left layout in every language: its width is only known
/// once the dialog has set it, and an `egui::Grid` laid out against a right edge measures
/// infinite. Its rows and footer are mirrored by [`rtl::horizontal`] and [`rtl::end`], and a
/// form of captions and fields by [`form_grid`], which is a grid in English and hand-laid rows
/// when mirrored.
pub fn show_modal<R>(ctx: &egui::Context, id: &str, content: impl FnOnce(&mut egui::Ui) -> R) -> egui::ModalResponse<R> {
    let stored = egui::Id::new(id);
    modal(ctx, id).show(ctx, |ui| {
        begin_dialog(ui);
        let inner = content(ui);
        // The frame's stroke sits outside this content rect; the stored card is the fill.
        let card = ui.min_rect().expand2(egui::vec2(f32::from(DIALOG_PAD_X), f32::from(DIALOG_PAD_Y)));
        ui.ctx().data_mut(|d| d.insert_temp(stored.with("card"), card));
        inner
    })
}

/// Outlined controls at a shared 30-point height, with a focus stroke and 4-point corners.
///
/// Dialog fills match the theme's field colour, so radio buttons, check boxes, combo boxes
/// and number fields need the border or they disappear into the card.
pub fn begin_dialog(ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let w = &mut ui.visuals_mut().widgets;
    w.inactive.bg_stroke = Stroke::new(1.0, t.border);
    w.inactive.weak_bg_fill = t.field;
    // Slider rails and check-box interiors use the plain fill.
    w.inactive.bg_fill = t.hover;
    w.hovered.bg_stroke = Stroke::new(1.0, t.text_muted);
    for state in [&mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        state.corner_radius = CornerRadius::same(4);
    }
    ui.visuals_mut().selection.stroke = Stroke::new(1.5, t.accent);
    ui.spacing_mut().interact_size.y = FIELD_MIN_HEIGHT;
}

/// The horizontal alignment of a text field holding `text`: against the right edge when the text
/// begins in a right-to-left script, against the left edge when it begins in another; with no
/// strong character yet (an empty field) at the start edge of the interface language. The field
/// follows what is typed, as `dir="auto"` does.
pub(crate) fn field_align(text: &str) -> egui::Align {
    match crate::bidi::first_strong_rtl(text) {
        Some(true) => egui::Align::Max,
        Some(false) => egui::Align::Min,
        None => rtl::text_align(),
    }
}

/// A single-line field tall enough to read and to click. Right-to-left script in it is shown and
/// edited in display order ([`BidiEdit`]); text without any, and a [`BidiEdit::password`], make it
/// a plain `egui::TextEdit`.
pub fn line(text: &mut String) -> BidiEdit<'_> {
    let align = field_align(text.as_str());
    BidiEdit::singleline(text).margin(FIELD_MARGIN).min_size(vec2(0.0, FIELD_MIN_HEIGHT)).horizontal_align(align)
}

/// A multiline field with the same horizontal inset as [`line`]. Right-to-left script in it is
/// wrapped in reading order and edited in display order ([`BidiEdit::multiline`]), each paragraph
/// against the edge it starts from; text without any, and a [`BidiEdit::password`], make it a plain
/// `egui::TextEdit`.
pub fn block(text: &mut String) -> BidiEdit<'_> {
    let align = field_align(text.as_str());
    BidiEdit::multiline(text).margin(egui::Margin::symmetric(10, 6)).horizontal_align(align)
}

/// [`line`] as a plain `egui::TextEdit` on any buffer, for what is typed as it is stored: passwords.
pub fn plain_line(text: &mut dyn egui::TextBuffer) -> egui::TextEdit<'_> {
    let align = field_align(text.as_str());
    egui::TextEdit::singleline(text).margin(FIELD_MARGIN).min_size(vec2(0.0, FIELD_MIN_HEIGHT)).horizontal_align(align)
}

/// [`block`] as a plain `egui::TextEdit`, for code (`code_editor()`): a script is typed and shown
/// in the order it is written.
pub fn plain_block(text: &mut dyn egui::TextBuffer) -> egui::TextEdit<'_> {
    let align = field_align(text.as_str());
    egui::TextEdit::multiline(text).margin(egui::Margin::symmetric(10, 6)).horizontal_align(align)
}

/// One line that stays within `max_width`, cut on a character boundary with an ellipsis.
/// Only the first 160 characters are measured, so a huge name cannot lay out thousands of candidates.
pub(crate) fn fit_line(text: &str, max_width: f32, mut width_of: impl FnMut(&str) -> f32) -> String {
    if !max_width.is_finite() || max_width <= 0.0 {
        return String::new();
    }
    let mut s: String = text.chars().take(160).collect();
    if s.len() == text.len() && width_of(text) <= max_width {
        return text.to_owned();
    }
    loop {
        s.pop();
        if s.is_empty() {
            return if width_of("\u{2026}") <= max_width { "\u{2026}".into() } else { String::new() };
        }
        let candidate = format!("{s}\u{2026}");
        if width_of(&candidate) <= max_width {
            return candidate;
        }
    }
}

pub(crate) fn text_width(ui: &egui::Ui, text: &str, font: &egui::FontId) -> f32 {
    ui.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE).size().x)
}

/// Draw a line of the interface at the start edge, never past `max_width`: `pos` is the middle of
/// the text's start edge, its left end in English and its right end when mirrored (the line runs
/// leftwards, clipped to `[pos.x - max_width, pos.x]`). A line that does not fit is cut in logical
/// order, so the "…" lands where the reader stops: at the right of English text and at the left of
/// Arabic. True when it was cut.
pub(crate) fn paint_start(
    ui: &egui::Ui,
    pos: egui::Pos2,
    text: &crate::i18n::Logical<'_>,
    font: egui::FontId,
    color: Color32,
    max_width: f32,
) -> bool {
    fn interface(s: &str) -> std::borrow::Cow<'_, str> {
        if rtl::mirrored() { std::borrow::Cow::Owned(crate::bidi::display_rtl(s)) } else { std::borrow::Cow::Borrowed(s) }
    }
    paint_fitted(ui, pos, text.as_str(), font, color, max_width, true, interface)
}

/// Draw `text` from a document (a layer name) at the start edge: the fitted line is put in
/// display order by its own direction when it is painted. `pos` is the middle of the text's
/// start edge: its left end in English, its right end when mirrored, where the line runs
/// leftwards and is clipped to `[pos.x - max_width, pos.x]`.
pub(crate) fn paint_start_doc(ui: &egui::Ui, pos: egui::Pos2, text: &str, font: egui::FontId, color: Color32, max_width: f32) {
    paint_fitted(ui, pos, text, font, color, max_width, true, crate::bidi::visual_auto);
}

/// [`paint_start_doc`] for a sentence that may hold document text (an Undo label): reordered only
/// where [`needs_bidi`], since the Arabic interface's own strings are in display order already.
pub(crate) fn paint_start_sentence(ui: &egui::Ui, pos: egui::Pos2, text: &str, font: egui::FontId, color: Color32, max_width: f32) {
    paint_fitted(ui, pos, text, font, color, max_width, true, visual_sentence);
}

/// The clip rectangle of a line `max_width` wide whose start edge is at `pos`.
fn fitted_clip(pos: egui::Pos2, max_width: f32, start: bool) -> Rect {
    let width = max_width.max(0.0);
    let left = if start { rtl::block_left(pos.x, width) } else { pos.x };
    Rect::from_min_size(egui::pos2(left, pos.y - 14.0), vec2(width, 28.0))
}

/// `text` for a painter call when it is a sentence that may hold document text: in display order
/// where [`needs_bidi`], otherwise as it is.
pub fn visual_sentence(text: &str) -> std::borrow::Cow<'_, str> {
    if needs_bidi(text) { crate::bidi::visual_auto(text) } else { std::borrow::Cow::Borrowed(text) }
}

/// Fit `text` to `max_width` in logical order, then paint what `reorder` makes of the fitted line.
/// `start` anchors the line at the start edge of the interface language (see [`paint_start_doc`]);
/// otherwise it is anchored at its left end.
#[allow(clippy::too_many_arguments)]
fn paint_fitted(
    ui: &egui::Ui,
    pos: egui::Pos2,
    text: &str,
    font: egui::FontId,
    color: Color32,
    max_width: f32,
    start: bool,
    reorder: impl for<'a> Fn(&'a str) -> std::borrow::Cow<'a, str>,
) -> bool {
    // Measured as it is painted, in display order.
    let fitted = fit_line(text, max_width, |s| text_width(ui, &reorder(s), &font));
    let cut = fitted != text;
    let shown = reorder(&fitted).into_owned();
    let anchor = if start { rtl::flip(Align2::LEFT_CENTER) } else { Align2::LEFT_CENTER };
    ui.painter().with_clip_rect(fitted_clip(pos, max_width, start)).text(pos, anchor, shown, font, color);
    cut
}

/// A mode-bar tab: text with an underline when active.
pub fn mode_tab(ui: &mut egui::Ui, label: &str, active: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let font = if active { theme::semibold(13.5) } else { theme::medium(13.5) };
    let w = ui.fonts_mut(|f| f.layout_no_wrap(label.to_owned(), font.clone(), t.text).size().x);
    let (rect, resp) = ui.allocate_exact_size(vec2(w + 22.0, 48.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), active, label));
    let press = theme::Press::track(ui, &resp);
    let well = rect.shrink2(vec2(2.0, 9.0));
    if active {
        if press.down > 0.01 {
            ui.painter().rect_filled(well, CornerRadius::same(6), t.pressed.gamma_multiply(press.down));
        }
    } else {
        press.wash(ui, well, 6, false);
    }
    let body = rect.translate(press.offset());
    ui.painter().text(body.center(), Align2::CENTER_CENTER, label, font, if active { t.text } else { t.text_muted });
    if active {
        let r = Rect::from_min_max(rect.left_bottom() + vec2(10.0, -2.0), rect.right_bottom() - vec2(10.0, 0.0));
        ui.painter().rect_filled(r, CornerRadius { nw: 1, ne: 1, sw: 0, se: 0 }, t.accent);
    }
    theme::hand(resp)
}

/// Rounded pill button; `primary` fills with the accent.
pub fn pill_button(ui: &mut egui::Ui, label: &str, primary: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let font = theme::medium(12.5);
    let w = ui.fonts_mut(|f| f.layout_no_wrap(label.to_owned(), font.clone(), t.text).size().x);
    let (rect, resp) = ui.allocate_exact_size(vec2(w + 26.0, 28.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label));
    let press = theme::Press::track(ui, &resp);
    let (fill, stroke, text) = if primary {
        (press.fill(t.accent, t.accent_text, t.accent.lerp_to_gamma(Color32::BLACK, 0.22)), Stroke::NONE, Color32::WHITE)
    } else {
        (press.fill(t.card, t.hover, t.pressed), Stroke::new(1.0, t.border), t.text)
    };
    ui.painter().rect(rect, CornerRadius::same(14), fill, stroke, egui::StrokeKind::Inside);
    ui.painter().text(rect.translate(press.offset()).center(), Align2::CENTER_CENTER, label, font, text);
    theme::hand(resp)
}

/// Icon + label, transparent until hovered.
pub fn ghost_button(ui: &mut egui::Ui, icon: &str, label: &str) -> Response {
    ghost_button_sized(ui, icon, label, 30.0)
}

/// [`ghost_button`] at a given height (the macOS title bar is shorter than the tab strip elsewhere).
pub fn ghost_button_sized(ui: &mut egui::Ui, icon: &str, label: &str, height: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    let font = theme::medium(13.0);
    let w = ui.fonts_mut(|f| f.layout_no_wrap(label.to_owned(), font.clone(), t.text).size().x);
    let (rect, resp) = ui.allocate_exact_size(vec2(w + 38.0, height), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label));
    let press = theme::Press::track(ui, &resp);
    press.wash(ui, rect, 6, false);
    let body = rect.translate(press.offset());
    icons::paint(ui, rtl::start_rect(body, 6.0, (height - 18.0) / 2.0, vec2(18.0, 18.0)), icon, 17.0, t.icon);
    ui.painter().text(rtl::start_center(body, 30.0), rtl::flip(Align2::LEFT_CENTER), label, font, t.text);
    theme::hand(resp)
}

/// A search-field lookalike that opens the command palette. The `placeholder` is cut in logical
/// order when it does not fit, so its "…" is at the end of what is read.
pub fn search_box(ui: &mut egui::Ui, placeholder: &crate::i18n::Logical<'_>, width: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 32.0), Sense::click());
    // The accessible name and the tooltip are the label as the catalog gives it, in display order.
    let named = if rtl::mirrored() { crate::bidi::display_rtl(placeholder.as_str()) } else { placeholder.as_str().to_owned() };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &named));
    let press = theme::Press::track(ui, &resp);
    let fill = press.fill(t.field, t.hover, t.pressed);
    let edge = t.border.lerp_to_gamma(t.text_muted, press.hover * 0.35);
    ui.painter().rect(rect, CornerRadius::same(16), fill, Stroke::new(1.0, edge), egui::StrokeKind::Inside);
    icons::paint(ui, rtl::start_rect(rect, 10.0, 8.0, vec2(16.0, 16.0)), "search", 15.0, t.text_muted);
    let shortcut = crate::commands::command_shortcut_label(ui.ctx(), "view.palette");
    let shortcut = ui.painter().layout_no_wrap(shortcut, theme::regular(11.5), t.text_faint);
    // The shortcut hint sits at the end of the box, the icon and the text at the start.
    let shortcut_left = if rtl::mirrored() { rect.left() + 12.0 } else { rect.right() - 12.0 - shortcut.size().x };
    let shortcut_pos = egui::pos2(shortcut_left, rect.center().y - shortcut.size().y * 0.5);
    // Reserve the measured shortcut width in every language; long translations are elided
    // within the remaining space rather than painted underneath the shortcut.
    let (text_left, text_right) = if rtl::mirrored() {
        // Mirrored: the text runs from the shortcut's right edge (plus a gap) to the icon column.
        let text_right = (rect.right() - 34.0).max(shortcut_pos.x + shortcut.size().x);
        ((shortcut_pos.x + shortcut.size().x + 10.0).min(text_right), text_right)
    } else {
        let text_right = (shortcut_pos.x - 10.0).max(rect.left());
        ((rect.left() + 34.0).min(text_right), text_right)
    };
    let text_rect = Rect::from_min_max(egui::pos2(text_left, rect.top()), egui::pos2(text_right, rect.bottom()));
    // The placeholder starts at the start edge of its room: flush right when mirrored.
    let elided = paint_start(ui, rtl::start_center(text_rect, 0.0), placeholder, theme::regular(13.0), t.text_faint, text_rect.width());
    ui.painter().with_clip_rect(rect).galley(shortcut_pos, shortcut, t.text_faint);
    let resp = resp.on_hover_cursor(egui::CursorIcon::Text);
    if elided { resp.on_hover_text(&named) } else { resp }
}

pub fn menu_item(ui: &mut egui::Ui, label: &str, shortcut: &str) -> Response {
    menu_entry(ui, true, label, Some(shortcut))
}

/// An entry of a menu: `label` at the start edge and the `shortcut` (`None` for an entry that has
/// no place for one) at the end.
///
/// In English that is `Button::new(label)`, with `.shortcut_text(shortcut)` for a `Some` (an empty
/// one too: it keeps the entry as wide as its siblings that have a shortcut). Mirrored, the label is flush
/// right and the shortcut at the left end, and the accessible name stays "label shortcut". The
/// button fills the menu's width either way, so the whole row is the hit rectangle.
pub fn menu_entry(ui: &mut egui::Ui, enabled: bool, label: &str, shortcut: Option<&str>) -> Response {
    if !rtl::mirrored() {
        let button = egui::Button::new(label);
        return ui.add_enabled(
            enabled,
            match shortcut {
                Some(shortcut) => button.shortcut_text(shortcut),
                None => button,
            },
        );
    }
    let mut atoms = egui::Atoms::new(label);
    atoms.push_left(egui::Atom::grow());
    let shortcut = shortcut.unwrap_or_default();
    if !shortcut.is_empty() {
        atoms.push_left(egui::RichText::new(shortcut).weak());
    }
    let resp = ui.add_enabled(enabled, egui::Button::new(atoms));
    let name = if shortcut.is_empty() { label.to_owned() } else { format!("{label} {shortcut}") };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &name));
    resp
}

/// A [`menu_entry`] with the icon `icon` (15 points, in the text colour) at its start edge, before
/// the label. A mirrored icon that points somewhere is flipped by [`icons::image`].
pub fn menu_icon_entry(ui: &mut egui::Ui, enabled: bool, icon: &str, label: &str) -> Response {
    let image = icons::image(icon, 15.0, ui.visuals().text_color());
    if !rtl::mirrored() {
        return ui.add_enabled(enabled, egui::Button::image_and_text(image, label));
    }
    let mut atoms = egui::Atoms::new(label);
    atoms.push_left(egui::Atom::grow());
    atoms.push_right(image);
    let resp = ui.add_enabled(enabled, egui::Button::new(atoms));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    resp
}

/// A line of a menu that is not a button (a check box, a radio button, a section label), at the
/// start edge of the menu. In English that is `add(ui)` as it is. Mirrored, the line is justified
/// against the right edge, so its label is flush right as a [`menu_entry`]'s is. Where the menu
/// has no width yet (an unbounded one) it is also `add(ui)` as it is.
pub fn menu_choice<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    if rtl::mirrored() && ui.available_width().is_finite() { ui.with_layout(Layout::top_down_justified(Align::Max), add).inner } else { add(ui) }
}

/// A submenu button inside a menu (a top-level menu button elsewhere). Mirrored, its label is
/// flush right with the arrow at the left; the submenu itself still opens to the right of the
/// button, or to the left where there is no room (egui fixes its side).
pub fn sub_menu<R>(ui: &mut egui::Ui, label: &str, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<Option<R>> {
    use egui::containers::menu::{SubMenuButton, is_in_menu};
    if !(rtl::mirrored() && is_in_menu(ui)) {
        return ui.menu_button(label, add_contents);
    }
    let mut atoms = egui::Atoms::new(label);
    atoms.push_left(egui::Atom::grow());
    atoms.push_left("\u{23F4}");
    let (resp, inner) = SubMenuButton::from_button(egui::Button::new(atoms)).ui(ui, add_contents);
    let name = format!("{label} {}", SubMenuButton::RIGHT_ARROW);
    let enabled = ui.is_enabled();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &name));
    egui::InnerResponse::new(inner.map(|i| i.inner), resp)
}

/// Corner radius of a checkbox's box. With the widgets' radius (`Tokens::radius`, 6) egui's 14 px
/// box is a circle, the shape of a radio button, and View ▸ Page display has both.
const CHECKBOX_RADIUS: u8 = 3;

/// A checkbox whose box is an outlined rounded square, never mistaken for a radio button. Every
/// checkbox goes through here: clippy.toml disallows `ui.checkbox` and `egui::Checkbox`.
pub fn checkbox<'a>(checked: &'a mut bool, atoms: impl egui::IntoAtoms<'a>) -> impl egui::Widget + 'a {
    #[expect(clippy::disallowed_methods, reason = "the one place that styles the box")]
    let widget = egui::Checkbox::new(checked, atoms);
    toggle(widget, Some(CHECKBOX_RADIUS))
}

/// [`checkbox`] without a label: the selection box on Organize Pages thumbnails.
pub fn checkbox_without_text(checked: &mut bool) -> impl egui::Widget + '_ {
    #[expect(clippy::disallowed_methods, reason = "the one place that styles the box")]
    let widget = egui::Checkbox::without_text(checked);
    toggle(widget, Some(CHECKBOX_RADIUS))
}

/// A radio button with an outline and a fill, like [`checkbox`]. Every radio button goes through
/// here or [`radio_value`]: clippy.toml disallows `ui.radio`, `ui.radio_value` and
/// `egui::RadioButton`.
pub fn radio<'a>(selected: bool, atoms: impl egui::IntoAtoms<'a>) -> impl egui::Widget + 'a {
    #[expect(clippy::disallowed_methods, reason = "the one place that styles the circle")]
    let widget = egui::RadioButton::new(selected, atoms);
    toggle(widget, None)
}

/// [`radio`] selected while `*current == alternative`; a click selects it (`Ui::radio_value`).
pub fn radio_value<'a, V: PartialEq>(ui: &mut egui::Ui, current: &mut V, alternative: V, atoms: impl egui::IntoAtoms<'a>) -> Response {
    let mut response = ui.add(radio(*current == alternative, atoms));
    if response.clicked() && *current != alternative {
        *current = alternative;
        response.mark_changed();
    }
    response
}

/// The box of a checkbox or the circle of a radio button. egui draws it with the widgets' visuals
/// and has no style of its own for it, so it is set in a scope around the widget, keeping what
/// the caller set (dialogs and the search panel already outline theirs):
/// - an outline, darker on hover;
/// - on the light theme, the plain fill (`Tokens::hover`), as dialogs have: the field colour is
///   the panels' white, so the box would vanish;
/// - for a checkbox, a corner radius small enough to keep the box square.
fn toggle<'a>(widget: impl egui::Widget + 'a, radius: Option<u8>) -> impl egui::Widget + 'a {
    move |ui: &mut egui::Ui| {
        ui.scope(|ui| {
            let t = Tokens::get(ui.ctx());
            let w = &mut ui.visuals_mut().widgets;
            if let Some(radius) = radius {
                for v in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
                    v.corner_radius = CornerRadius::same(radius);
                }
            }
            if !t.dark() && w.inactive.bg_fill == t.field {
                w.inactive.bg_fill = t.hover;
            }
            if w.inactive.bg_stroke.is_empty() {
                w.inactive.bg_stroke = Stroke::new(1.0, t.border);
            }
            if w.hovered.bg_stroke.is_empty() {
                w.hovered.bg_stroke = Stroke::new(1.0, t.text_muted);
            }
            ui.add(widget)
        })
        .inner
    }
}

/// The title of a dialog, at its start edge (the right when mirrored). Every dialog body sets its
/// width first ([`show_modal`]), which a title against the right edge needs.
pub fn dialog_title(ui: &mut egui::Ui, text: impl Into<String>) -> Response {
    rtl::page(ui, |ui| ui.label(egui::RichText::new(text).font(theme::semibold(18.0))))
}

pub fn section_title(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(10.0);
    // Section titles across every panel go through here, so one translation point covers them.
    ui.label(egui::RichText::new(crate::i18n::upper_label(text)).font(theme::semibold(10.5)).color(t.text_faint).extra_letter_spacing(0.6));
    ui.add_space(2.0);
}

/// Paint one line of `text` left-aligned and vertically centred on `pos`, cut with "…" so it is
/// at most `max_width` wide: a long file name or path stays inside its row (#425).
pub fn row_text(ui: &egui::Ui, pos: egui::Pos2, text: impl Into<String>, font: egui::FontId, color: Color32, max_width: f32) {
    row_text_at(ui, pos, text, font, color, max_width, Align2::LEFT_CENTER);
}

/// [`row_text`] anchored at the start edge: `pos` is the middle of the line's start edge, the
/// left end in English and the right end when mirrored.
pub fn row_text_start(ui: &egui::Ui, pos: egui::Pos2, text: impl Into<String>, font: egui::FontId, color: Color32, max_width: f32) {
    row_text_at(ui, pos, text, font, color, max_width, rtl::flip(Align2::LEFT_CENTER));
}

fn row_text_at(ui: &egui::Ui, pos: egui::Pos2, text: impl Into<String>, font: egui::FontId, color: Color32, max_width: f32, anchor: Align2) {
    let mut job = egui::text::LayoutJob::simple_singleline(text.into(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_width.max(0.0));
    let galley = ui.painter().layout_job(job);
    let rect = anchor.anchor_size(pos, galley.size());
    ui.painter().galley(rect.min, galley, color);
}

/// Text wrapped to the available width, for text people write (comments, replies). A paragraph
/// in a right-to-left script (Persian, Arabic, Hebrew) reads right to left and sits against the
/// right edge ([`crate::bidi::wrap`]); text without one is an ordinary wrapping label.
/// `truncate` keeps the first line only, with an ellipsis where the text goes on.
pub fn wrapped_text(ui: &mut egui::Ui, text: &str, font: egui::FontId, color: Color32, truncate: bool) -> Response {
    wrapped_text_impl(ui, text, font, color, truncate, false)
}

/// [`wrapped_text`], optionally shrunk to its text. With `fit`, right-to-left text takes the width
/// of its widest line instead of all the room, as egui's own label does in a tooltip; its lines
/// still sit against the right edge of that width. Without `fit` it takes the whole width.
fn wrapped_text_impl(ui: &mut egui::Ui, text: &str, font: egui::FontId, color: Color32, truncate: bool, fit: bool) -> Response {
    if !crate::bidi::has_rtl(text) {
        let mut label = egui::Label::new(egui::RichText::new(text).font(font).color(color)).wrap();
        if truncate {
            label = label.truncate();
        }
        return ui.add(label);
    }
    let available = ui.available_width().max(1.0);
    let lines = wrapped_lines(ui, text, &font, available, truncate);
    let galleys: Vec<_> = lines.iter().map(|line| (line.rtl, ui.painter().layout_no_wrap(line.text.clone(), font.clone(), color))).collect();
    let height = galleys.iter().map(|(_, g)| g.size().y).sum();
    let widest = galleys.iter().map(|(_, g)| g.size().x).fold(0.0, f32::max);
    let width = if fit { widest.min(available).max(1.0) } else { available };
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    // Painted text only: the accessibility name keeps the logical order.
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
    let mut y = rect.top();
    for (rtl, galley) in galleys {
        let x = if rtl { rect.right() - galley.size().x } else { rect.left() };
        let h = galley.size().y;
        ui.painter().galley(egui::pos2(x, y), galley, color);
        y += h;
    }
    resp
}

/// A tooltip for `resp` from translated text in LOGICAL order. Takes the output of
/// [`tl_logical!`](crate::tl_logical) and [`fmt_logical`](crate::i18n::fmt_logical), never of
/// `tl!`: display-order text would be reordered a second time and read backwards.
///
/// egui breaks a long right-to-left tooltip at the wrong end, so those are wrapped here with the
/// bidirectional algorithm and set against the right edge. Every other language gets egui's own
/// [`Response::on_hover_text`].
pub fn hover_text(resp: Response, text: &crate::i18n::Logical<'_>) -> Response {
    if !crate::bidi::has_rtl(text) {
        return resp.on_hover_text(text.as_str());
    }
    resp.on_hover_ui(|ui| tooltip_body(ui, text))
}

/// [`hover_text`] for a disabled widget ([`Response::on_disabled_hover_text`]). LOGICAL-order text
/// only: use [`tl_logical!`](crate::tl_logical), never `tl!` output.
pub fn disabled_hover_text(resp: Response, text: &crate::i18n::Logical<'_>) -> Response {
    if !crate::bidi::has_rtl(text) {
        return resp.on_disabled_hover_text(text.as_str());
    }
    resp.on_disabled_hover_ui(|ui| tooltip_body(ui, text))
}

/// The right-to-left tooltip's text, wrapped to a tooltip's width and shrunk to fit it.
fn tooltip_body(ui: &mut egui::Ui, text: &crate::i18n::Logical<'_>) {
    ui.set_max_width(ui.spacing().tooltip_width);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().text_color();
    wrapped_text_impl(ui, &crate::bidi::isolate_shortcuts(text), font, color, false, true);
}

/// A paragraph of translated body text (a dialog's explanation) in LOGICAL order. Takes the
/// output of [`tl_logical!`](crate::tl_logical) and [`fmt_logical`](crate::i18n::fmt_logical),
/// never of `tl!`: display-order text would be reordered a second time and read backwards.
///
/// A right-to-left paragraph is wrapped here and sits against the right edge of the available
/// width; any other language gets an ordinary label.
pub fn body_text(ui: &mut egui::Ui, text: &crate::i18n::Logical<'_>) -> Response {
    if !crate::bidi::has_rtl(text) {
        return ui.label(text.as_str());
    }
    let font = egui::TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().text_color();
    wrapped_text(ui, &crate::bidi::isolate_shortcuts(text), font, color, false)
}

/// [`body_text`] with a font and colour of the caller's choosing, for notes (a small, muted
/// line under a control). LOGICAL-order text only: use [`tl_logical!`](crate::tl_logical) and
/// [`fmt_logical`](crate::i18n::fmt_logical), never `tl!` output. Other languages get a label
/// in that font and colour.
pub fn styled_text(ui: &mut egui::Ui, text: &crate::i18n::Logical<'_>, font: egui::FontId, color: Color32) -> Response {
    if !crate::bidi::has_rtl(text) {
        return ui.label(egui::RichText::new(text.as_str()).font(font).color(color));
    }
    wrapped_text(ui, &crate::bidi::isolate_shortcuts(text), font, color, false)
}

/// [`styled_text`] in the small font: a note under a control. LOGICAL-order text only
/// ([`tl_logical!`](crate::tl_logical), [`fmt_logical`](crate::i18n::fmt_logical)), never `tl!`
/// output.
pub fn small_text(ui: &mut egui::Ui, text: &crate::i18n::Logical<'_>, color: Color32) -> Response {
    let font = egui::TextStyle::Small.resolve(ui.style());
    styled_text(ui, text, font, color)
}

const ELLIPSIS: &str = "\u{2026}";

/// Wrapped right-to-left text kept between frames. Wrapping measures many candidate lines, so
/// re-wrapping every comment on every frame would make a panel of long comments slow; the
/// result depends only on the text, font, width, scale and collapsed state, so it is looked up.
#[derive(Clone, Default)]
struct WrapCache {
    lines: std::collections::HashMap<u64, std::sync::Arc<[crate::bidi::Line]>>,
}

/// How many wrapped texts the cache holds before it starts over (each costs its text's size).
const WRAP_CACHE_MAX: usize = 256;

/// A collapsed comment shows one line, so no more than this many characters are wrapped for it:
/// a long comment doesn't cost a full wrap just to show its start.
const COLLAPSED_WRAP_CHARS: usize = 1024;

/// The lines `wrapped_text` paints for a right-to-left `text` (and the Bookmarks panel's titles,
/// which lay their lines out themselves), from the cache when the same text was wrapped the same
/// way before.
pub(crate) fn wrapped_lines(ui: &egui::Ui, text: &str, font: &egui::FontId, width: f32, truncate: bool) -> std::sync::Arc<[crate::bidi::Line]> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (text, font, width.to_bits(), ui.ctx().pixels_per_point().to_bits(), truncate).hash(&mut hasher);
    let key = hasher.finish();
    let cache_id = egui::Id::new("pdfcraft-rtl-wrap-cache");
    if let Some(hit) = ui.ctx().data_mut(|d| d.get_temp_mut_or_default::<WrapCache>(cache_id).lines.get(&key).cloned()) {
        return hit;
    }
    // Collapsed: only the start is wrapped (cut at a char boundary, so the slice can't fail).
    let (measured, cut) = match text.char_indices().nth(COLLAPSED_WRAP_CHARS) {
        Some((at, _)) if truncate => (text.get(..at).unwrap_or(text), true),
        _ => (text, false),
    };
    // The direction controls (the isolates around a shortcut) are never painted, so they have no width.
    let measure = |f: &mut egui::epaint::text::FontsView<'_>, s: &str| {
        let shown: String = s.chars().filter(|c| !crate::bidi::is_bidi_control(*c)).collect();
        f.layout_no_wrap(shown, font.clone(), Color32::WHITE).size().x
    };
    let mut lines = ui.fonts_mut(|f| {
        let room = if truncate { width - measure(f, ELLIPSIS) } else { width };
        crate::bidi::wrap(measured, |s| measure(f, s) <= room)
    });
    if truncate && (lines.len() > 1 || cut) {
        lines.truncate(1);
        if let Some(line) = lines.first_mut() {
            // The ellipsis goes where the line's reading ends: the left of a right-to-left line.
            if line.rtl {
                line.text.insert_str(0, ELLIPSIS);
            } else {
                line.text.push_str(ELLIPSIS);
            }
        }
    }
    let lines: std::sync::Arc<[crate::bidi::Line]> = lines.into();
    ui.ctx().data_mut(|d| {
        let cache = &mut d.get_temp_mut_or_default::<WrapCache>(cache_id).lines;
        if cache.len() >= WRAP_CACHE_MAX {
            cache.clear();
        }
        cache.insert(key, lines.clone());
    });
    lines
}

/// Whether document text painted by an egui label needs [`wrapped_text`] to read right: it
/// holds right-to-left script and the interface is left to right. The Arabic interface's own
/// catalog strings are already in display order, so a sentence built from one (which may hold
/// a document's name) is left to the ordinary label there; reordering it again would undo it.
pub fn needs_bidi(text: &str) -> bool {
    !crate::i18n::current().rtl() && crate::bidi::has_rtl(text)
}

/// A wrapping label for document text that may hold right-to-left script (an Arabic file or
/// field name in a popup or a detail line; a notice is a [`logical_label`]): [`wrapped_text`] in
/// a column as wide as the text, up to the room there is, when [`needs_bidi`]; otherwise an
/// ordinary wrapping label.
pub fn wrapped_label(ui: &mut egui::Ui, text: &str, font: egui::FontId, color: Color32) -> Response {
    if !needs_bidi(text) {
        return ui.add(egui::Label::new(egui::RichText::new(text).font(font).color(color)).wrap());
    }
    // `wrapped_text` fills the width it is given; give it the text's own width, measured the way
    // it wraps (each paragraph in logical order), plus a point so rounding never breaks a line.
    let natural = text.split('\n').map(|paragraph| text_width(ui, paragraph, &font)).fold(0.0_f32, f32::max);
    let width = if natural.is_finite() { (natural + 1.0).min(ui.available_width()).max(1.0) } else { ui.available_width().max(1.0) };
    ui.allocate_ui(vec2(width, 0.0), |ui| wrapped_text(ui, text, font, color, false)).inner
}

/// A wrapping label for a notice or a banner: a sentence from the catalog in LOGICAL order
/// ([`tl_logical!`](crate::tl_logical), [`fmt_logical`](crate::i18n::fmt_logical)), which may
/// hold a document's name. Text with right-to-left script is wrapped and reordered here with the
/// bidirectional algorithm and sits against the right edge of a column as wide as its longest
/// line, up to the room there is ([`wrapped_text_impl`] fitted); in the Arabic interface as in
/// any other. Text without it is an ordinary label that wraps, or, with `truncate`, keeps its
/// first line and ends it with an ellipsis.
pub fn logical_label(ui: &mut egui::Ui, text: &crate::i18n::Logical<'_>, font: egui::FontId, color: Color32, truncate: bool) -> Response {
    wrapped_text_impl(ui, text, font, color, truncate, true)
}

/// [`wrapped_text`] as a selectable, clickable row, for lists of document text (findings,
/// changes): a right-to-left paragraph sits against the right edge, and the row gets the
/// selected or hovered background of [`egui::Button::selectable`]. Text without right-to-left
/// script is that button. The accessible name stays the logical `text`.
pub fn wrapped_text_clickable(ui: &mut egui::Ui, text: &str, font: egui::FontId, color: Color32, selected: bool) -> Response {
    if !crate::bidi::has_rtl(text) {
        return ui.add(egui::Button::selectable(selected, egui::RichText::new(text).font(font).color(color)).wrap());
    }
    let pad = ui.spacing().button_padding;
    let width = ui.available_width().max(1.0);
    let lines = wrapped_lines(ui, text, &font, (width - 2.0 * pad.x).max(1.0), false);
    let galleys: Vec<_> = lines.iter().map(|line| (line.rtl, ui.painter().layout_no_wrap(line.text.clone(), font.clone(), color))).collect();
    let height: f32 = galleys.iter().map(|(_, g)| g.size().y).sum();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height + 2.0 * pad.y), Sense::click());
    let enabled = ui.is_enabled();
    // Painted text only: the accessibility name keeps the logical order.
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, text));
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    if selected || resp.hovered() || resp.has_focus() {
        let visuals = ui.style().interact_selectable(&resp, selected);
        ui.painter().rect(rect.expand(visuals.expansion), visuals.corner_radius, visuals.weak_bg_fill, visuals.bg_stroke, egui::StrokeKind::Inside);
    }
    let mut y = rect.top() + pad.y;
    for (rtl, galley) in galleys {
        let x = if rtl { rect.right() - pad.x - galley.size().x } else { rect.left() + pad.x };
        let h = galley.size().y;
        ui.painter().galley(egui::pos2(x, y), galley, color);
        y += h;
    }
    resp
}

/// Transient message at the bottom centre.
pub fn toast(app: &mut PdfCraftApp, ctx: &egui::Context) {
    let Some((msg, start)) = app.toast.clone() else { return };
    let now = ctx.input(|i| i.time);
    let start = if start == 0.0 { now } else { start };
    app.toast = Some((msg.clone(), start));
    if now - start > 3.5 {
        app.toast = None;
        return;
    }
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    // Above the progress notice while a job runs.
    let card = if app.progress_notice.is_some() { ctx.data(|d| d.get_temp::<f32>(egui::Id::new(PROGRESS_HEIGHT))).unwrap_or(0.0) } else { 0.0 };
    let lift = if card > 0.0 { 28.0 + card + 10.0 } else { 28.0 };
    egui::Area::new(egui::Id::new("toast"))
        .order(egui::Order::Tooltip)
        .pivot(Align2::CENTER_BOTTOM)
        .fixed_pos(screen.center_bottom() - vec2(0.0, lift))
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(if t.dark() { Color32::from_rgb(0xEC, 0xEC, 0xEF) } else { Color32::from_rgb(0x2A, 0x2A, 0x2F) })
                .corner_radius(CornerRadius::same(8))
                .inner_margin(egui::Margin::symmetric(16, 10))
                .show(ui, |ui| {
                    // An Area remembers its previous content width. A short notice
                    // must not force the next validation message into a narrow column.
                    // Allow natural short labels; wrap longer ones within the viewport,
                    // including the frame's 32 points and 16-point outside margins.
                    ui.set_max_width((screen.width() - 64.0).clamp(1.0, 560.0));
                    let ink = if t.dark() { Color32::from_rgb(0x22, 0x22, 0x26) } else { Color32::WHITE };
                    // The message is in logical order: an Arabic notice (or one naming an Arabic
                    // file) is wrapped and reordered here; any other is an ordinary label.
                    let font = egui::TextStyle::Body.resolve(ui.style());
                    logical_label(ui, &msg, font, ink, false);
                });
        });
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
}

/// A background job's progress, shown at the bottom centre until the job is done.
#[derive(Clone, Debug, PartialEq)]
pub struct ProgressNotice {
    /// What is happening ("Optimizing… image 3 of 40").
    pub label: String,
    /// From 0 to 1.
    pub fraction: f32,
    /// Show a Cancel button.
    pub cancellable: bool,
}

/// Where the progress notice keeps its height this frame, for placing the toast above it.
const PROGRESS_HEIGHT: &str = "progress-notice-height";

/// Draw [`PdfCraftApp::progress_notice`]: the label, a bar that eases towards the fraction with
/// a light sweeping across it, the percentage and Cancel. Returns true when Cancel was clicked.
/// With animation effects off ([`crate::motion`]) the bar steps to the fraction and has no sweep.
pub fn progress_notice(app: &PdfCraftApp, ctx: &egui::Context) -> bool {
    let Some(p) = &app.progress_notice else { return false };
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    // The same inverted card as the toast.
    let (fill, text) = if t.dark() {
        (Color32::from_rgb(0xEC, 0xEC, 0xEF), Color32::from_rgb(0x22, 0x22, 0x26))
    } else {
        (Color32::from_rgb(0x2A, 0x2A, 0x2F), Color32::WHITE)
    };
    let fraction = if p.fraction.is_finite() { p.fraction.clamp(0.0, 1.0) } else { 0.0 };
    let animated = crate::motion::animated(ctx);
    let shown = ctx.animate_value_with_time(egui::Id::new("progress-notice-bar"), fraction, if animated { 0.3 } else { 0.0 });
    let now = ctx.input(|i| i.time);
    let mut cancel = false;
    let area = egui::Area::new(egui::Id::new("progress-notice"))
        .order(egui::Order::Tooltip)
        .pivot(Align2::CENTER_BOTTOM)
        .fixed_pos(screen.center_bottom() - vec2(0.0, 28.0))
        .show(ctx, |ui| {
            egui::Frame::NONE.fill(fill).corner_radius(CornerRadius::same(10)).inner_margin(egui::Margin::symmetric(16, 12)).show(ui, |ui| {
                let width = 340.0_f32.min(screen.width() - 64.0).max(160.0);
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&p.label).color(text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(format!("{:.0}%", shown * 100.0)).font(theme::medium(12.0)).color(text.gamma_multiply(0.7)));
                    });
                });
                ui.add_space(6.0);
                let (track, resp) = ui.allocate_exact_size(vec2(width, 6.0), Sense::hover());
                // The label above names the job; the bar carries the percentage.
                resp.widget_info(|| {
                    let mut info = egui::WidgetInfo::new(egui::WidgetType::ProgressIndicator);
                    info.value = Some(f64::from(fraction) * 100.0);
                    info
                });
                let painter = ui.painter();
                painter.rect_filled(track, CornerRadius::same(3), text.gamma_multiply(0.18));
                let done = Rect::from_min_size(track.min, vec2(track.width() * shown, track.height()));
                if done.width() > 0.5 {
                    painter.rect_filled(done, CornerRadius::same(3), t.accent);
                    // A soft light sweeping along the filled part says the job is alive between
                    // updates.
                    if animated {
                        let band = 48.0;
                        let x = done.left() - band + ((now * 160.0) % f64::from(done.width() + band)) as f32;
                        let clip = painter.with_clip_rect(done);
                        for (i, a) in [0.10_f32, 0.22, 0.10].into_iter().enumerate() {
                            let r = Rect::from_min_size(egui::pos2(x + i as f32 * band / 3.0, done.top()), vec2(band / 3.0, done.height()));
                            clip.rect_filled(r, CornerRadius::ZERO, Color32::WHITE.gamma_multiply(a));
                        }
                    }
                }
                if p.cancellable {
                    ui.add_space(4.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let b = egui::Button::new(egui::RichText::new(tl!("Cancel")).font(theme::medium(12.5)).color(text)).frame(false);
                        cancel = ui.add(b).clicked();
                    });
                }
            });
        });
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(PROGRESS_HEIGHT), area.response.rect.height()));
    ctx.request_repaint_after(std::time::Duration::from_millis(33));
    cancel
}

/// The ArtCraft wordmark (Storyteller's brand, docs/brand/; not open source), sized to `height`.
pub fn artcraft_logo(ui: &mut egui::Ui, height: f32) -> Response {
    let dark = ui.visuals().dark_mode;
    let (uri, bytes): (&str, &'static [u8]) = if dark {
        ("bytes://artcraft-logo-white.svg", include_bytes!("../../../docs/brand/artcraft-logo-white.svg"))
    } else {
        ("bytes://artcraft-logo.svg", include_bytes!("../../../docs/brand/artcraft-logo.svg"))
    };
    ui.add(egui::Image::from_bytes(uri, bytes).max_height(height).alt_text("ArtCraft"))
}

/// The ArtCraft mark (brand blue, works on light and dark), `size` points square.
pub fn artcraft_mark(ui: &mut egui::Ui, size: f32) -> Response {
    ui.add(
        egui::Image::from_bytes("bytes://artcraft-mark.svg", include_bytes!("../../../docs/brand/artcraft-mark.svg"))
            .fit_to_exact_size(vec2(size, size))
            .alt_text("ArtCraft"),
    )
}

/// Buttons for every community link (`pdfcraft_engine::links`), Discord first and prominent.
/// Returns the registry command of the one clicked.
pub fn community_links(ui: &mut egui::Ui) -> Option<&'static str> {
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        for (i, l) in pdfcraft_engine::links::LINKS.iter().enumerate() {
            let resp = if i == 0 { icon_pill(ui, l.icon, tl!("Join our Discord"), true) } else { icon_pill(ui, l.icon, tl!(l.label), false) };
            if resp.on_hover_text(l.url).clicked() {
                clicked = Some(l.command);
            }
        }
    });
    clicked
}

/// [`icon_pill`] with a ▾ part at its end that opens a menu of related choices. Returns the
/// main button's response and the ▾'s (give it to `egui::Popup::menu`).
pub fn split_pill(ui: &mut egui::Ui, icon: &str, label: &str, more: &str) -> (Response, Response) {
    let t = Tokens::get(ui.ctx());
    let font = theme::medium(12.5);
    let w = ui.fonts_mut(|f| f.layout_no_wrap(label.to_owned(), font.clone(), t.text).size().x);
    let (rect, _) = ui.allocate_exact_size(vec2(w + 46.0 + 28.0, 30.0), Sense::hover());
    let (main_rect, more_rect) = rect.split_left_right_at_x(rect.right() - 28.0);
    let main = ui.interact(main_rect, ui.id().with(("split-pill", label)), Sense::click());
    let arrow = ui.interact(more_rect, ui.id().with(("split-pill-more", label)), Sense::click());
    main.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label));
    arrow.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), more));
    let radius = CornerRadius::same(15);
    ui.painter().rect(rect, radius, t.card, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let left = CornerRadius { nw: 15, sw: 15, ne: 0, se: 0 };
    let right = CornerRadius { nw: 0, sw: 0, ne: 15, se: 15 };
    let main_press = theme::Press::track(ui, &main);
    let arrow_press = theme::Press::track(ui, &arrow);
    for (r, press, corners) in [(main_rect, main_press, left), (more_rect, arrow_press, right)] {
        let fill = press.fill(Color32::TRANSPARENT, t.hover, t.pressed);
        if fill.a() > 0 {
            ui.painter().rect_filled(r.shrink(1.2), corners, fill);
        }
    }
    let body = rect.translate(main_press.offset());
    let more_body = more_rect.translate(arrow_press.offset());
    ui.painter().vline(more_rect.left(), rect.y_range().shrink(7.0), Stroke::new(1.0, t.text_muted));
    crate::icons::paint(ui, Rect::from_min_size(body.min + vec2(12.0, 7.0), vec2(16.0, 16.0)), icon, 15.0, t.text);
    ui.painter().text(body.left_center() + vec2(34.0, 0.0), Align2::LEFT_CENTER, label, font, t.text);
    crate::icons::paint(ui, Rect::from_center_size(more_body.center(), vec2(14.0, 14.0)), "chevron-down", 13.0, t.text);
    (theme::hand(main).on_hover_text(label), theme::hand(arrow).on_hover_text(more))
}

/// A pill button with an icon (primary = filled accent).
pub fn icon_pill(ui: &mut egui::Ui, icon: &str, label: &str, primary: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let font = theme::medium(12.5);
    let w = ui.fonts_mut(|f| f.layout_no_wrap(label.to_owned(), font.clone(), t.text).size().x);
    let (rect, resp) = ui.allocate_exact_size(vec2(w + 46.0, 30.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label));
    let press = theme::Press::track(ui, &resp);
    let (fill, stroke, text) = if primary {
        (press.fill(t.accent, t.accent_text, t.accent.lerp_to_gamma(Color32::BLACK, 0.22)), Stroke::NONE, Color32::WHITE)
    } else {
        (press.fill(t.card, t.hover, t.pressed), Stroke::new(1.0, t.border), t.text)
    };
    ui.painter().rect(rect, CornerRadius::same(15), fill, stroke, egui::StrokeKind::Inside);
    let body = rect.translate(press.offset());
    crate::icons::paint(ui, Rect::from_min_size(body.min + vec2(12.0, 7.0), vec2(16.0, 16.0)), icon, 15.0, text);
    ui.painter().text(body.left_center() + vec2(34.0, 0.0), Align2::LEFT_CENTER, label, font, text);
    theme::hand(resp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Pos2;

    #[test]
    fn search_placeholder_never_overlaps_the_shortcut() {
        let previous = crate::i18n::current();
        for (code, expected) in [("en", "Ctrl+K"), ("de", "Strg+K")] {
            crate::i18n::set_current(crate::i18n::Lang::from_code(code).unwrap());
            for width in [260.0, 160.0, 90.0] {
                let ctx = egui::Context::default();
                ctx.set_os(egui::os::OperatingSystem::Nix);
                ctx.set_fonts(theme::font_definitions());
                let placeholder = "Werkzeuge und Befehle suchen";
                let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    search_box(ui, &crate::i18n::Logical::from(placeholder), width);
                });
                let text_shapes: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| if let egui::Shape::Text(text) = &shape.shape { Some((shape.clip_rect, text)) } else { None })
                    .collect();
                let (_, shortcut) = text_shapes.iter().find(|(_, text)| text.galley.job.text.contains('K')).unwrap();
                assert_eq!(shortcut.galley.job.text, expected);
                // With no room left beside the shortcut nothing is painted at all.
                let Some((clip, hint)) = text_shapes.iter().find(|(_, text)| text.galley.job.text.starts_with("Werk")) else {
                    assert!(width < 100.0, "the hint is painted at width {width}");
                    output.drop_without_applying_deltas();
                    continue;
                };
                assert!(clip.right() <= shortcut.pos.x - 9.0, "hint must leave a gap before the shortcut at width {width}");
                assert!(
                    hint.galley.job.text.ends_with('\u{2026}') && hint.galley.job.text != placeholder,
                    "the long original German hint must be cut with an ellipsis at width {width}: {:?}",
                    hint.galley.job.text
                );
                assert_eq!(hint.galley.rows.len(), 1);
                output.drop_without_applying_deltas();
            }
        }
        crate::i18n::set_current(previous);
    }

    /// Mirrored, the box has its icon and placeholder at the right and the shortcut hint at the
    /// left, and the placeholder still never runs under the hint.
    #[test]
    fn mirrored_search_box_keeps_the_hint_at_the_left_and_the_placeholder_clear_of_it() {
        let _arabic = ArabicUi::on();
        for width in [260.0, 160.0, 90.0] {
            let ctx = egui::Context::default();
            ctx.set_os(egui::os::OperatingSystem::Nix);
            ctx.set_fonts(theme::font_definitions());
            let placeholder = "Werkzeuge und Befehle suchen";
            let mut rect = Rect::NOTHING;
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                rect = search_box(ui, &crate::i18n::Logical::from(placeholder), width).rect;
            });
            let texts: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| if let egui::Shape::Text(text) = &shape.shape { Some((shape.clip_rect, text)) } else { None })
                .collect();
            let (_, shortcut) = texts.iter().find(|(_, text)| text.galley.job.text.contains('K')).unwrap();
            assert!(shortcut.pos.x < rect.center().x, "the shortcut is at the left at width {width}");
            let Some((clip, hint)) = texts.iter().find(|(_, text)| text.galley.job.text.starts_with("Werk")) else {
                assert!(
                    width < 100.0,
                    "the placeholder is painted at width {width}: {:?}",
                    texts.iter().map(|t| t.1.galley.job.text.clone()).collect::<Vec<_>>()
                );
                output.drop_without_applying_deltas();
                continue;
            };
            assert!(clip.left() >= shortcut.pos.x + shortcut.galley.size().x + 9.0, "a gap after the shortcut at width {width}");
            assert!(clip.right() <= rect.right() - 33.0, "room for the icon at the right at width {width}");
            assert!(hint.galley.rows.len() == 1);
            output.drop_without_applying_deltas();
        }
    }

    /// View ▸ Page display mixes radio buttons and checkboxes. With the theme's 6 px widget radius
    /// egui's 14 px checkbox box drew as a circle, and on the light theme an unticked box or an
    /// unselected radio button was white on white with no outline, so it couldn't be seen.
    #[test]
    fn checkboxes_and_radio_buttons_are_outlined_filled_and_distinct() {
        for kind in [theme::ThemeKind::Light, theme::ThemeKind::Dark] {
            let ctx = egui::Context::default();
            theme::apply(&ctx, kind);
            let t = Tokens::for_kind(kind);
            let side = ctx.global_style().spacing.icon_width;
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.add(checkbox(&mut false, "Show cover page in two-page view"));
                ui.add(radio(false, "Single page"));
            });
            let boxes: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Rect(r) if (r.rect.width() - side).abs() < 0.5 && (r.rect.height() - side).abs() < 0.5 => Some(r),
                    _ => None,
                })
                .collect();
            let circles: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Circle(c) if (c.radius * 2.0 - side).abs() < 1.0 => Some(c),
                    _ => None,
                })
                .collect();
            assert_eq!((boxes.len(), circles.len()), (1, 1), "{kind:?}: one checkbox box and one radio circle");
            let r = boxes[0].corner_radius;
            let radius = r.nw.max(r.ne).max(r.sw).max(r.se);
            assert!(f32::from(radius) <= side / 4.0, "{kind:?}: a {side} px box with corner radius {radius} looks like a radio button");
            for (what, fill, edge) in [("checkbox", boxes[0].fill, boxes[0].stroke), ("radio button", circles[0].fill, circles[0].stroke)] {
                assert!(fill != t.panel && fill != t.card, "{kind:?}: an unselected {what} is filled like the panel behind it");
                assert!(
                    edge.width >= 1.0 && edge.color != t.panel && edge.color != t.card,
                    "{kind:?}: an unselected {what} needs an outline, got {edge:?}"
                );
            }
            output.drop_without_applying_deltas();
        }
        // An outline the caller set (dialogs) is kept.
        let ctx = egui::Context::default();
        let own = Stroke::new(1.5, Color32::RED);
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.visuals_mut().widgets.inactive.bg_stroke = own;
            ui.add(checkbox(&mut true, "Collate"));
            ui.add(radio(false, "All"));
        });
        let kept = output
            .shapes
            .iter()
            .filter(|s| match &s.shape {
                egui::Shape::Rect(r) => r.stroke == own,
                egui::Shape::Circle(c) => c.stroke == own,
                _ => false,
            })
            .count();
        assert_eq!(kept, 2, "the checkbox and the radio button keep the caller's outline");
        output.drop_without_applying_deltas();
    }

    fn toast_rect(app: &mut PdfCraftApp, ctx: &egui::Context, width: f32, now: &mut f64) -> Rect {
        // Let the Area settle after a notice/viewport change, without advancing to
        // expiry. No native rendering or renderer worker is needed for this layout.
        for _ in 0..3 {
            *now += 0.016;
            let output = ctx.run_ui(
                egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 900.0))), time: Some(*now), ..Default::default() },
                |ui| toast(app, ui.ctx()),
            );
            output.drop_without_applying_deltas();
        }
        ctx.memory(|memory| memory.area_rect(egui::Id::new("toast")).unwrap())
    }

    #[test]
    fn toast_width_adapts_to_long_notices_and_narrow_viewports() {
        let ctx = egui::Context::default();
        let mut app = PdfCraftApp::new();
        let mut now = 1.0;
        app.notify("Saved");
        let short = toast_rect(&mut app, &ctx, 1280.0, &mut now);
        assert!(short.width() < 160.0, "short notices retain their natural width: {short:?}");
        app.notify("Fill in name failed: The value entered is not valid for the field [ name ]");
        let long = toast_rect(&mut app, &ctx, 1280.0, &mut now);
        assert!(long.width() > 300.0, "a previous short Area must not squeeze validation feedback: {long:?}");
        assert!(long.height() < 85.0, "ordinary feedback must not become a seven-line column: {long:?}");
        for width in [320.0, 180.0] {
            let rect = toast_rect(&mut app, &ctx, width, &mut now);
            assert!(rect.width() <= width - 32.0 + 1.0, "frame and viewport margins must fit: {rect:?}");
            assert!(rect.left() >= 15.0 && rect.right() <= width - 15.0, "notice remains horizontally on screen: {rect:?}");
        }
        app.notify("Saved");
        let again = toast_rect(&mut app, &ctx, 1280.0, &mut now);
        assert!((again.width() - short.width()).abs() < 1.0, "long notices must not impose a permanent minimum width");
    }

    /// The text of every galley the toast paints after settling for a few frames.
    fn toast_texts(message: &str) -> Vec<String> {
        let ctx = egui::Context::default();
        ctx.set_fonts(theme::font_definitions());
        let mut app = PdfCraftApp::new();
        app.notify(message.to_owned());
        let mut now = 1.0;
        let mut texts = Vec::new();
        for _ in 0..3 {
            now += 0.016;
            let output = ctx.run_ui(
                egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), time: Some(now), ..Default::default() },
                |ui| toast(&mut app, ui.ctx()),
            );
            texts = output
                .shapes
                .iter()
                .filter_map(|s| if let egui::Shape::Text(t) = &s.shape { Some(t.galley.job.text.clone()) } else { None })
                .collect();
            output.drop_without_applying_deltas();
        }
        texts
    }

    #[test]
    fn a_notice_naming_an_arabic_file_reads_in_display_order_in_either_interface() {
        let previous = crate::i18n::current();
        let logical = "Could not open تقرير 2024 Report";
        // Left-to-right interface: the document's name is reordered inside the notice (the Arabic
        // word and the number after it form one right-to-left stretch: the number comes first).
        crate::i18n::set_current(crate::i18n::Lang::EN);
        assert!(toast_texts(logical).iter().any(|t| t == "Could not open 2024 تقرير Report"), "{:?}", toast_texts(logical));
        // Text without right-to-left script is the plain label, as before.
        assert!(toast_texts("Saved").iter().any(|t| t == "Saved"));
        // The Arabic interface's notices are in logical order too: this one is a right-to-left
        // paragraph, so the Latin name and the number come out in front of the Arabic word.
        crate::i18n::set_current(crate::i18n::Lang::from_code("ar").unwrap());
        assert!(toast_texts("تقرير 2024 Report").iter().any(|t| t == "Report 2024 تقرير"), "{:?}", toast_texts("تقرير 2024 Report"));
        // An English engine message in the Arabic interface is the plain label.
        assert!(toast_texts("Saved").iter().any(|t| t == "Saved"));
        crate::i18n::set_current(previous);
    }

    /// The text shapes (position, galley rectangle, text) the toast paints on a screen `width`
    /// wide after settling for a few frames.
    fn toast_shapes(message: crate::i18n::Logical<'static>, width: f32) -> Vec<(Pos2, Rect, String)> {
        let ctx = egui::Context::default();
        ctx.set_fonts(theme::font_definitions());
        let mut app = PdfCraftApp::new();
        app.notify(message);
        let mut now = 1.0;
        let mut shapes = Vec::new();
        for _ in 0..3 {
            now += 0.016;
            let output = ctx.run_ui(
                egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 600.0))), time: Some(now), ..Default::default() },
                |ui| toast(&mut app, ui.ctx()),
            );
            shapes = output
                .shapes
                .iter()
                .filter_map(|s| if let egui::Shape::Text(t) = &s.shape { Some((t.pos, t.galley.rect, t.galley.job.text.clone())) } else { None })
                .collect();
            output.drop_without_applying_deltas();
        }
        shapes
    }

    /// A long notice under the Arabic interface wraps in typing order: its first line holds its
    /// first words, the lines sit against the right edge, and the last line holds the last words.
    /// A left-to-right notice is the plain label, as before.
    #[test]
    fn an_arabic_notice_wraps_from_its_first_words_against_the_right_edge() {
        let previous = crate::i18n::current();
        crate::i18n::set_current(crate::i18n::Lang::from_code("ar").unwrap());
        let logical = tl_logical!("This file was damaged and has been repaired. Saving keeps the repaired version.");
        assert!(crate::bidi::has_rtl(&logical), "the Arabic catalog has this notice: {logical:?}");
        let words: Vec<&str> = logical.split_whitespace().collect();
        // The closing full stop moves to the other end of the last word when it is painted.
        let (first, last) = (words[0], words[words.len() - 1].trim_end_matches('.'));
        let shapes = toast_shapes(logical.clone(), 300.0);
        crate::i18n::set_current(previous);
        assert!(shapes.len() >= 2, "a narrow screen wraps the notice: {shapes:?}");
        let top = shapes.iter().min_by(|a, b| a.0.y.total_cmp(&b.0.y)).expect("a line");
        let bottom = shapes.iter().max_by(|a, b| a.0.y.total_cmp(&b.0.y)).expect("a line");
        assert!(has_word(&top.2, first) && !top.2.contains(last), "the first line holds the first words: {shapes:?}");
        assert!(bottom.2.contains(last), "the last line holds the last words: {shapes:?}");
        let right = shapes.iter().map(|(pos, rect, _)| pos.x + rect.width()).fold(0.0, f32::max);
        for (pos, rect, text) in &shapes {
            assert!((pos.x + rect.width() - right).abs() < 0.5, "{text:?} is right-aligned");
        }
        // A Latin notice is one plain label whatever the width.
        let latin = toast_shapes(crate::i18n::Logical::from("Saved"), 300.0);
        assert_eq!(latin.iter().map(|s| s.2.as_str()).collect::<Vec<_>>(), ["Saved"]);
    }

    /// The toast keeps the catalog's logical text: the wrapper reorders it once, so the app must
    /// not hand it display-order text (which `tl!` gives under Arabic).
    #[test]
    fn notices_carry_logical_text_under_the_arabic_interface() {
        let previous = crate::i18n::current();
        crate::i18n::set_current(crate::i18n::Lang::from_code("ar").unwrap());
        let mut app = PdfCraftApp::new();
        let toast = |app: &PdfCraftApp| app.toast.as_ref().map(|t| t.0.clone()).expect("a notice");
        app.notify_tr("Open a document first");
        assert_eq!(toast(&app), tl_logical!("Open a document first"));
        assert_ne!(toast(&app), crate::i18n::t("Open a document first"), "not the display-order text");
        app.notify_fmt("{name} is read-only", &[("name", "تقرير.pdf")]);
        assert_eq!(toast(&app), crate::i18n::fmt_logical(&tl_logical!("{name} is read-only"), &[("name", "تقرير.pdf")]));
        app.notify_error("disk full");
        assert_eq!(toast(&app), "disk full");
        crate::i18n::set_current(previous);
        // English: the text as written.
        app.notify_fmt("{name} is read-only", &[("name", "a.pdf")]);
        assert_eq!(toast(&app), "a.pdf is read-only");
    }

    /// The text shapes `wrapped_text` paints in a 120 pt wide column at x = 0.
    fn wrapped_shapes(text: &str, truncate: bool) -> Vec<(Pos2, Rect, String)> {
        let ctx = egui::Context::default();
        ctx.set_fonts(theme::font_definitions());
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let column = Rect::from_min_size(Pos2::ZERO, vec2(120.0, 400.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(column), |ui| {
                wrapped_text(ui, text, theme::regular(13.0), Color32::WHITE, truncate);
            });
        });
        let shapes = output
            .shapes
            .iter()
            .filter_map(|shape| if let egui::Shape::Text(t) = &shape.shape { Some((t.pos, t.galley.rect, t.galley.job.text.clone())) } else { None })
            .collect();
        output.drop_without_applying_deltas();
        shapes
    }

    /// Right-to-left text is wrapped once and looked up on later frames; the cache stays bounded
    /// however many comments go by, and a collapsed comment of any length still shows one line.
    #[test]
    fn rtl_wrapping_is_cached_and_bounded() {
        let ctx = egui::Context::default();
        ctx.set_fonts(theme::font_definitions());
        let cache_len =
            |ctx: &egui::Context| ctx.data_mut(|d| d.get_temp_mut_or_default::<WrapCache>(egui::Id::new("pdfcraft-rtl-wrap-cache")).lines.len());
        let persian = "این یک نظر طولانی به زبان فارسی است که باید در چند خط شکسته شود";
        let long = persian.repeat(2_000);
        let mut first = None;
        for _ in 0..3 {
            ctx.run_ui(egui::RawInput::default(), |ui| {
                let lines = wrapped_lines(ui, persian, &theme::regular(13.0), 120.0, false);
                assert!(lines.len() > 1);
                if let Some(prev) = &first {
                    assert!(std::sync::Arc::ptr_eq(prev, &lines), "the second frame reuses the first frame's wrap");
                }
                first = Some(lines);
                let collapsed = wrapped_lines(ui, &long, &theme::regular(13.0), 120.0, true);
                assert_eq!(collapsed.len(), 1);
                assert!(collapsed[0].text.starts_with(ELLIPSIS), "{:?}", collapsed[0].text);
            })
            .drop_without_applying_deltas();
        }
        assert_eq!(cache_len(&ctx), 2, "one entry per text and layout");
        ctx.run_ui(egui::RawInput::default(), |ui| {
            for i in 0..WRAP_CACHE_MAX + 50 {
                wrapped_lines(ui, &format!("{persian} {i}"), &theme::regular(13.0), 120.0, false);
            }
        })
        .drop_without_applying_deltas();
        assert!(cache_len(&ctx) <= WRAP_CACHE_MAX, "bounded: {}", cache_len(&ctx));
    }

    #[test]
    fn rtl_comment_wraps_against_the_right_edge() {
        let persian = "این یک نظر طولانی به زبان فارسی است که باید در چند خط شکسته شود";
        let lines = wrapped_shapes(persian, false);
        assert!(lines.len() > 1, "{lines:?}");
        let right = lines.iter().map(|(pos, rect, _)| pos.x + rect.width()).fold(0.0, f32::max);
        for (pos, rect, text) in &lines {
            assert!(pos.x >= -0.5 && (pos.x + rect.width() - right).abs() < 0.5, "{text:?} at {pos:?} must be right-aligned");
        }
        assert!(right <= 120.5, "lines stay inside the column: {right}");
        // The first words read on the first line, at the top.
        let top = lines.iter().min_by(|a, b| a.0.y.total_cmp(&b.0.y)).unwrap();
        assert!(top.2.contains("این"), "{top:?}");
        // Collapsed: one line, its ellipsis on the left where right-to-left reading ends.
        let collapsed = wrapped_shapes(persian, true);
        assert_eq!(collapsed.len(), 1);
        assert!(collapsed[0].2.starts_with('\u{2026}'), "{collapsed:?}");
        // Text without right-to-left script is a plain label, as before.
        let latin = wrapped_shapes("A short note", false);
        assert_eq!(latin.len(), 1);
        assert_eq!(latin[0].2, "A short note");
        assert!(latin[0].0.x.abs() < 0.5);
    }

    /// What `run` paints in a 120 pt wide column at x = 0: its response rectangle and the text
    /// shapes (position, galley rectangle, painted text).
    fn painted(run: impl FnOnce(&mut egui::Ui) -> Response) -> (Rect, Vec<(Pos2, Rect, String)>) {
        let ctx = egui::Context::default();
        ctx.set_fonts(theme::font_definitions());
        let mut response = Rect::NOTHING;
        let mut run = Some(run);
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let column = Rect::from_min_size(Pos2::ZERO, vec2(120.0, 400.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(column), |ui| {
                if let Some(run) = run.take() {
                    response = run(ui).rect;
                }
            });
        });
        let shapes = output
            .shapes
            .iter()
            .filter_map(|shape| if let egui::Shape::Text(t) = &shape.shape { Some((t.pos, t.galley.rect, t.galley.job.text.clone())) } else { None })
            .collect();
        output.drop_without_applying_deltas();
        (response, shapes)
    }

    fn has_word(line: &str, word: &str) -> bool {
        line.split_whitespace().any(|w| w == word)
    }

    /// Text that egui wraps has to be wrapped in typing order: the first line of an Arabic
    /// paragraph holds its first words and the last line its last words. The catalog's display
    /// order text wrapped the same way reads from the last line up (the bug this pins).
    #[test]
    fn logical_arabic_wraps_from_its_first_words() {
        let words = ["احفظ", "الملف", "الحالي", "باسم", "جديد", "قبل", "إغلاق", "التطبيق", "نهائيا"];
        let (first, last) = (words[0], words[words.len() - 1]);
        let logical = words.join(" ");
        let (_, shapes) = painted(|ui| wrapped_text(ui, &logical, theme::regular(13.0), Color32::WHITE, false));
        assert!(shapes.len() > 2, "narrow column: {shapes:?}");
        let top = shapes.iter().min_by(|a, b| a.0.y.total_cmp(&b.0.y)).expect("a line");
        let bottom = shapes.iter().max_by(|a, b| a.0.y.total_cmp(&b.0.y)).expect("a line");
        assert!(has_word(&top.2, first), "the first words are on the first line: {shapes:?}");
        assert!(has_word(&bottom.2, last), "the last words are on the last line: {shapes:?}");
        // Lines are right-aligned.
        let right = shapes.iter().map(|(pos, rect, _)| pos.x + rect.width()).fold(0.0, f32::max);
        for (pos, rect, text) in &shapes {
            assert!((pos.x + rect.width() - right).abs() < 0.5, "{text:?} must be right-aligned");
        }
        // The display-order text, wrapped the same way, starts with the last words.
        let display = crate::bidi::display_rtl(&logical);
        assert_ne!(display, logical);
        let (_, shapes) = painted(|ui| wrapped_text(ui, &display, theme::regular(13.0), Color32::WHITE, false));
        let top = shapes.iter().min_by(|a, b| a.0.y.total_cmp(&b.0.y)).expect("a line");
        let bottom = shapes.iter().max_by(|a, b| a.0.y.total_cmp(&b.0.y)).expect("a line");
        assert!(has_word(&top.2, last) && !has_word(&top.2, first), "display order puts the last words first: {shapes:?}");
        assert!(has_word(&bottom.2, first), "display order puts the first words last: {shapes:?}");
    }

    /// A tooltip shrinks to its text; body text takes the whole width.
    #[test]
    fn fitted_right_to_left_text_is_no_wider_than_its_lines() {
        let long = "احفظ الملف الحالي باسم جديد قبل إغلاق التطبيق نهائيا";
        for text in [long, "احفظ الملف"] {
            let (rect, shapes) = painted(|ui| wrapped_text_impl(ui, text, theme::regular(13.0), Color32::WHITE, false, true));
            let widest = shapes.iter().map(|(_, g, _)| g.width()).fold(0.0, f32::max);
            assert!(rect.width() <= 120.0 + 0.5, "no wider than the room: {rect:?}");
            assert!(rect.width() <= widest + 0.5 && rect.width() + 0.5 >= widest, "as wide as the widest line {widest}: {rect:?}");
            // Lines stay right-aligned inside the shrunk rectangle.
            for (pos, g, line) in &shapes {
                assert!((pos.x + g.width() - rect.right()).abs() < 0.5, "{line:?} at {pos:?} in {rect:?}");
            }
            let (full, _) = painted(|ui| wrapped_text_impl(ui, text, theme::regular(13.0), Color32::WHITE, false, false));
            assert!((full.width() - 120.0).abs() < 0.5, "unfitted text takes the whole width: {full:?}");
        }
        let (short, _) = painted(|ui| wrapped_text_impl(ui, "احفظ", theme::regular(13.0), Color32::WHITE, false, true));
        assert!(short.width() < 60.0, "{short:?}");
    }

    /// The isolates around a shortcut order its symbols and are neither painted nor measured.
    #[test]
    fn tooltip_keeps_a_shortcut_whole() {
        let tip = crate::i18n::Logical::from("احفظ الملف الحالي ⇧⌘S");
        let (_, shapes) = painted(|ui| {
            tooltip_body(ui, &tip);
            ui.label("")
        });
        let painted_text: Vec<&str> = shapes.iter().map(|(_, _, t)| t.as_str()).collect();
        assert!(painted_text.iter().any(|t| t.contains("⇧⌘S")), "{painted_text:?}");
        assert!(painted_text.iter().all(|t| !t.chars().any(crate::bidi::is_bidi_control)), "{painted_text:?}");
        // The same in a column too narrow for one line.
        let narrow = crate::i18n::Logical::from("احفظ الملف الحالي باسم جديد قبل إغلاق التطبيق نهائيا ⇧⌘S");
        let (_, shapes) = painted(|ui| body_text(ui, &narrow));
        assert!(shapes.iter().any(|(_, _, t)| t.contains("⇧⌘S")), "{shapes:?}");
        // Left-to-right text is an ordinary label.
        let (_, shapes) = painted(|ui| body_text(ui, &crate::i18n::Logical::from("Save the file")));
        assert_eq!(shapes.iter().map(|s| s.2.as_str()).collect::<Vec<_>>(), ["Save the file"]);
        let (_, shapes) = painted(|ui| styled_text(ui, &crate::i18n::Logical::from("Note"), theme::regular(11.0), Color32::GRAY));
        assert_eq!(shapes.iter().map(|s| s.2.as_str()).collect::<Vec<_>>(), ["Note"]);
        let (_, shapes) =
            painted(|ui| styled_text(ui, &crate::i18n::Logical::from("ملاحظة طويلة جدا للاختبار هنا"), theme::regular(11.0), Color32::GRAY));
        assert!(shapes.len() >= 2, "wrapped: {shapes:?}");
    }

    /// One frame of `wrapped_text_clickable` in a 120 pt wide column at x = 0 with `events`:
    /// the row's rect, whether it was clicked, whether it senses clicks, the widget info of a
    /// click and the shapes it painted.
    struct ClickableFrame {
        rect: Rect,
        clicked: bool,
        senses_click: bool,
        info: Option<egui::WidgetInfo>,
        shapes: Vec<egui::Shape>,
    }

    fn clickable_frame(ctx: &egui::Context, text: &str, selected: bool, events: Vec<egui::Event>) -> ClickableFrame {
        let (mut rect, mut clicked, mut senses_click) = (Rect::NOTHING, false, false);
        let output = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            let column = Rect::from_min_size(Pos2::ZERO, vec2(120.0, 400.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(column), |ui| {
                let resp = wrapped_text_clickable(ui, text, theme::regular(13.0), Color32::WHITE, selected);
                (rect, clicked, senses_click) = (resp.rect, resp.clicked(), resp.sense.senses_click());
            });
        });
        let info = output
            .platform_output
            .events
            .iter()
            .find_map(|e| if let egui::output::OutputEvent::Clicked(info) = e { Some(info.clone()) } else { None });
        let shapes = output.shapes.iter().map(|s| s.shape.clone()).collect();
        output.drop_without_applying_deltas();
        ClickableFrame { rect, clicked, senses_click, info, shapes }
    }

    fn click(ctx: &egui::Context, text: &str, selected: bool) -> ClickableFrame {
        let at = clickable_frame(ctx, text, selected, vec![]).rect.center();
        let button =
            |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        clickable_frame(ctx, text, selected, vec![egui::Event::PointerMoved(at)]);
        clickable_frame(ctx, text, selected, vec![button(true)]);
        clickable_frame(ctx, text, selected, vec![button(false)])
    }

    #[test]
    fn clickable_rtl_text_sits_at_the_right_edge_senses_clicks_and_keeps_its_logical_name() {
        let persian = "این یک یافته طولانی به زبان فارسی است که باید در چند خط شکسته شود";
        let ctx = egui::Context::default();
        ctx.set_fonts(theme::font_definitions());
        let frame = clickable_frame(&ctx, persian, false, vec![]);
        assert!(frame.senses_click, "a row of findings is clickable");
        assert!(frame.rect.width() <= 120.5, "the row stays inside the column: {:?}", frame.rect);
        let texts: Vec<(Pos2, f32, String)> = frame
            .shapes
            .iter()
            .filter_map(|s| if let egui::Shape::Text(t) = s { Some((t.pos, t.galley.rect.width(), t.galley.job.text.clone())) } else { None })
            .collect();
        assert!(texts.len() > 1, "wrapped over several lines: {texts:?}");
        let padding = egui::Style::default().spacing.button_padding.x;
        for (pos, width, text) in &texts {
            assert!((pos.x + width - (frame.rect.right() - padding)).abs() < 0.5, "{text:?} at {pos:?} must be right-aligned");
        }
        assert!(texts.iter().all(|(_, _, text)| text != persian), "painted in display order");
        // Not hovered, not selected: no background yet.
        assert!(!frame.shapes.iter().any(|s| matches!(s, egui::Shape::Rect(_))), "an idle row is unframed");
        // A click reports the logical text as the accessible name, and the selected state.
        let clicked = click(&ctx, persian, true);
        assert!(clicked.clicked);
        let info = clicked.info.expect("the click is announced");
        assert_eq!(info.label.as_deref(), Some(persian));
        assert_eq!(info.typ, egui::WidgetType::Button);
        assert_eq!(info.selected, Some(true));
        // Selected (and hovered): the row has its background.
        assert!(clicked.shapes.iter().any(|s| matches!(s, egui::Shape::Rect(_))), "a selected row is framed");
    }

    #[test]
    fn clickable_text_without_rtl_script_is_a_selectable_button() {
        let ctx = egui::Context::default();
        ctx.set_fonts(theme::font_definitions());
        let clicked = click(&ctx, "A short finding", false);
        assert!(clicked.clicked && clicked.senses_click);
        let info = clicked.info.expect("the click is announced");
        assert_eq!(info.label.as_deref(), Some("A short finding"));
        assert_eq!(info.selected, Some(false));
    }

    #[test]
    fn line_field_is_at_least_thirty_points_tall() {
        let ctx = egui::Context::default();
        let mut text = String::new();
        let mut height = 0.0;
        ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 200.0))), ..Default::default() }, |ui| {
            begin_dialog(ui);
            height = ui.add(line(&mut text)).rect.height();
        })
        .drop_without_applying_deltas();
        assert!(height >= FIELD_MIN_HEIGHT, "field height {height}");
    }

    /// Draws in Arabic for the test's lifetime (the language is per thread), then restores it.
    struct ArabicUi;
    impl ArabicUi {
        fn on() -> Self {
            crate::i18n::set_current(crate::i18n::Lang::from_code("ar").expect("ar registered"));
            Self
        }
    }
    impl Drop for ArabicUi {
        fn drop(&mut self) {
            crate::i18n::set_current(crate::i18n::Lang::EN);
        }
    }

    /// The line `text` painted by [`paint_start`] in a room `max_width` wide, and whether it was cut.
    fn painted_start_line(text: &str, max_width: f32) -> (String, bool) {
        let ctx = egui::Context::default();
        ctx.set_os(egui::os::OperatingSystem::Nix);
        ctx.set_fonts(theme::font_definitions());
        let mut cut = false;
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            cut = paint_start(ui, egui::pos2(300.0, 50.0), &crate::i18n::Logical::from(text), theme::regular(13.0), Color32::WHITE, max_width);
        });
        let painted = output
            .shapes
            .iter()
            .find_map(|s| if let egui::Shape::Text(t) = &s.shape { Some(t.galley.job.text.clone()) } else { None })
            .unwrap_or_default();
        output.drop_without_applying_deltas();
        (painted, cut)
    }

    /// A line that does not fit is cut where the reader stops: the "…" ends English text and
    /// begins (at the left of) Arabic, which keeps the first words of the sentence.
    #[test]
    fn a_cut_line_has_its_ellipsis_where_the_reader_stops() {
        let (english, cut) = painted_start_line("Find tools and commands in this document", 90.0);
        assert!(cut && english.starts_with("Find") && english.ends_with('\u{2026}'), "{english:?}");
        let (whole, cut) = painted_start_line("Find tools", 300.0);
        assert!(!cut && whole == "Find tools", "{whole:?}");

        let _arabic = ArabicUi::on();
        let sentence = "ابحث عن الأدوات والأوامر في هذا المستند";
        let (arabic, cut) = painted_start_line(sentence, 90.0);
        assert!(cut, "the sentence does not fit 90 points");
        assert!(arabic.starts_with('\u{2026}'), "the ellipsis is at the left of the painted line: {arabic:?}");
        let first_word = crate::bidi::display_rtl("ابحث");
        assert!(arabic.contains(first_word.as_str()), "and the sentence's first word survives at the right: {arabic:?}");
    }

    #[test]
    fn a_field_aligns_to_the_script_it_holds_or_the_start_edge() {
        use egui::Align;
        assert_eq!(field_align(""), Align::Min);
        assert_eq!(field_align("Report"), Align::Min);
        assert_eq!(field_align("تقرير"), Align::Max, "right-to-left script sits against the right edge, even in English");
        assert_eq!(field_align("  12 تقرير"), Align::Max, "the first strong character decides, not digits or spaces");
        assert_eq!(field_align("2024"), Align::Min);
        let _arabic = ArabicUi::on();
        assert_eq!(field_align(""), Align::Max, "an empty field starts at the right in Arabic");
        assert_eq!(field_align("2024"), Align::Max);
        assert_eq!(field_align("Report"), Align::Min, "Latin text typed in an Arabic interface is left aligned");
        assert_eq!(field_align("تقرير"), Align::Max);
    }

    /// Where a single-line field `width` wide draws `text`: its response rectangle, and the
    /// rectangle of the painted text.
    fn field_geometry(text: &str, width: f32) -> (Rect, Rect) {
        let ctx = egui::Context::default();
        ctx.set_fonts(theme::font_definitions());
        let mut geometry = (Rect::NOTHING, Rect::NOTHING);
        let mut buffer = text.to_owned();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            begin_dialog(ui);
            let out = line(&mut buffer).desired_width(width).show(ui);
            // A right-aligned galley's rectangle starts left of its origin; the field draws it
            // shifted back by that (`TextEdit::show`).
            let at = vec2(out.galley_pos.x - out.galley.rect.left(), out.galley_pos.y);
            geometry = (out.response.rect, out.galley.rect.translate(at));
        });
        output.drop_without_applying_deltas();
        geometry
    }

    #[test]
    fn typed_arabic_sits_against_the_right_edge_of_a_field() {
        let (field, text) = field_geometry("تقرير", 200.0);
        let inset = FIELD_MARGIN.right as f32;
        assert!((text.right() - (field.right() - inset)).abs() < 1.5, "right aligned: field {field:?} text {text:?}");
        let (field, text) = field_geometry("Report", 200.0);
        assert!((text.left() - (field.left() + FIELD_MARGIN.left as f32)).abs() < 1.5, "left aligned: field {field:?} text {text:?}");
        let _arabic = ArabicUi::on();
        let (field, text) = field_geometry("", 200.0);
        assert_eq!(field.width(), 200.0);
        assert!(text.width() >= 0.0);
    }

    /// Text longer than the field still scrolls inside it when it is right aligned: the field keeps
    /// its width, the text is not pushed outside it, and nothing is NaN.
    #[test]
    fn long_right_aligned_text_stays_inside_its_field() {
        let long = "تقرير ".repeat(60);
        let (field, text) = field_geometry(&long, 120.0);
        assert_eq!(field.width(), 120.0);
        assert!(text.width() > field.width(), "the text overflows the field");
        assert!(text.left().is_finite() && text.right().is_finite());
        assert!(text.left() <= field.left() + FIELD_MARGIN.left as f32 + 0.5, "it starts at the field's inner edge: {field:?} {text:?}");
    }

    #[test]
    fn fit_line_keeps_short_text_and_ellipsizes_the_rest() {
        let width = |s: &str| s.chars().count() as f32;
        assert_eq!(fit_line("localhost", 20.0, width), "localhost");
        let long = "Apple Worldwide Developer Relations Certification Authority";
        let fitted = fit_line(long, 24.0, width);
        assert!(fitted.ends_with('\u{2026}'), "{fitted}");
        assert!(fitted.chars().count() <= 24, "{fitted}");
        assert!(long.starts_with(fitted.trim_end_matches('\u{2026}')));
        assert_eq!(fit_line(long, 0.0, width), "");
        assert_eq!(fit_line("ééé", 2.0, width), "é\u{2026}");
    }
}
