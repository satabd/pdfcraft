//! Mirroring the interface for a right-to-left language (Arabic).
//!
//! The shell is laid out from its start edge: the left in English, the right in Arabic. These
//! helpers answer "which edge is the start?" in one place so a panel, a row or a painted icon
//! does not carry its own `if rtl`. Mirroring changes positions only. The labels of the Arabic
//! catalog are already in display order ([`crate::bidi::display_rtl`]), so nothing here (and no
//! caller) puts them through `bidi::visual*` again.
//!
//! Lines the code assembles from translated pieces are ordered the same way: [`join`] and
//! [`caption_value`] follow the reading order of the language.
//!
//! Every helper is the identity when [`mirrored`] is false, so the English layout cannot change.
//! The captions of a dialog form sit at the right of their fields ([`crate::widgets::form_grid`]
//! builds the rows by hand, because `egui::Grid` cannot sit against a right edge), and the
//! entries of the main menu put their label at the right and their shortcut at the left
//! ([`crate::widgets::menu_entry`]).
//!
//! What stays physical: the window controls, scrollbars, the document canvas and its overlays,
//! the side a submenu opens on, the order of the document tabs, and the explicit "Tools panel
//! side" preference. What is not mirrored yet: the other `egui::Grid` users (panels, tool pages,
//! the Preferences and signing dialogs) and context menus built from plain `ui.button`s.

use egui::{Align, Align2, InnerResponse, Layout, Margin, Pos2, Rect, Ui, UiBuilder, Vec2, pos2, vec2};

/// Whether the interface language is written right to left ([`crate::i18n::current`], set at the
/// start of every frame from the language preference).
pub fn mirrored() -> bool {
    crate::i18n::current().rtl()
}

/// Whether the language preference `pref` ([`crate::PdfCraftApp::language`], possibly Auto) is
/// written right to left, for code that runs outside a frame (a control-channel command).
pub fn of_pref(pref: &str) -> bool {
    crate::i18n::Lang::from_pref(pref).rtl()
}

/// Which side of the window a panel belongs to, in reading order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The side where reading starts: the left in English, the right in Arabic. No panel sits
    /// there yet (the Tools pane is placed by the docking workspace).
    #[cfg_attr(not(test), expect(dead_code, reason = "the complete pair; only the end side has a panel today"))]
    Start,
    /// The opposite side.
    End,
}

/// A horizontal layout whose first item sits at the start edge.
pub fn start_layout(cross: Align) -> Layout {
    if mirrored() { Layout::right_to_left(cross) } else { Layout::left_to_right(cross) }
}

/// A horizontal layout whose first item sits at the end edge (an end-aligned cluster).
pub fn end_layout(cross: Align) -> Layout {
    if mirrored() { Layout::left_to_right(cross) } else { Layout::right_to_left(cross) }
}

/// The layout of a panel or dialog body: items are placed against the start edge.
pub fn page_layout() -> Layout {
    if mirrored() { Layout::top_down(Align::Max) } else { Layout::top_down(Align::Min) }
}

/// The start alignment of text and of a text field.
pub fn text_align() -> Align {
    if mirrored() { Align::Max } else { Align::Min }
}

/// Run `add_contents` with the [`page_layout`] (a panel or page body: items against the start
/// edge). In a left-to-right language the content runs in `ui` as it is, with no extra scope.
///
/// Never around anything that may hold an `egui::Grid`, and never around a window whose width is
/// not set yet: both measure infinite when aligned to the right edge.
pub fn page<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    if mirrored() { ui.with_layout(page_layout(), add_contents).inner } else { add_contents(ui) }
}

/// `ui.vertical`, with its items against the start edge.
pub fn vertical<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let width = ui.available_size_before_wrap().x;
    if mirrored() && width.is_finite() {
        if ui.layout().is_horizontal() {
            // In a row a block of no height yet would be centred on the row's middle line and grow
            // down from there. Put it where `ui.vertical` does: at the row's top.
            return ui.scope_builder(UiBuilder::new().layout(page_layout()), add_contents);
        }
        ui.allocate_ui_with_layout(vec2(width, 0.0), page_layout(), add_contents)
    } else {
        ui.vertical(add_contents)
    }
}

/// `ui.horizontal`, with the first item at the start edge.
///
/// Where the row has no width to sit against (a scroll area that scrolls sideways, a window that
/// fits its contents) it is an ordinary left-to-right row: an end against infinity has no place.
pub fn horizontal<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let width = ui.available_size_before_wrap().x;
    if mirrored() && width.is_finite() {
        // `Ui::horizontal` only follows the parent's direction; this does not depend on it.
        ui.allocate_ui_with_layout(vec2(width, ui.spacing().interact_size.y), start_layout(Align::Center), add_contents)
    } else {
        ui.horizontal(add_contents)
    }
}

/// `ui.horizontal_centered`, with the first item at the start edge (and, like [`horizontal`],
/// an ordinary row where the width is unbounded).
pub fn horizontal_centered<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let size = ui.available_size_before_wrap();
    if mirrored() && size.is_finite() {
        ui.allocate_ui_with_layout(size, start_layout(Align::Center), add_contents)
    } else {
        ui.horizontal_centered(add_contents)
    }
}

/// A cluster pushed to the end edge of the row (the replacement for
/// `with_layout(Layout::right_to_left(Align::Center), ..)`).
pub fn end<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    ui.with_layout(end_layout(Align::Center), add_contents)
}

/// The panel at the `side` of the window: Start is the left in English and the right in Arabic.
pub fn panel(id: impl Into<egui::Id>, side: Side) -> egui::Panel {
    match (side, mirrored()) {
        (Side::Start, false) | (Side::End, true) => egui::Panel::left(id),
        (Side::Start, true) | (Side::End, false) => egui::Panel::right(id),
    }
}

/// The x of a thing `off` from the start edge of `rect`.
pub fn start(rect: Rect, off: f32) -> f32 {
    if mirrored() { rect.right() - off } else { rect.left() + off }
}

/// The left edge of a block `width` wide that starts at `start_x` (a point on the start edge,
/// from [`start`]): `start_x` itself in English, `width` further left when mirrored.
pub fn block_left(start_x: f32, width: f32) -> f32 {
    if mirrored() { start_x - width } else { start_x }
}

/// The x of a thing `off` from the end edge of `rect`.
pub fn end_x(rect: Rect, off: f32) -> f32 {
    if mirrored() { rect.left() + off } else { rect.right() - off }
}

/// The point `off` from the start edge, level with the middle of `rect`.
pub fn start_center(rect: Rect, off: f32) -> Pos2 {
    pos2(start(rect, off), rect.center().y)
}

/// The point `off` from the end edge, level with the middle of `rect`.
pub fn end_center(rect: Rect, off: f32) -> Pos2 {
    pos2(end_x(rect, off), rect.center().y)
}

/// A box of `size` whose start edge is `off` in from the start edge of `rect` and whose top is
/// `top` below the top of `rect` (an icon at the start of a row).
pub fn start_rect(rect: Rect, off: f32, top: f32, size: Vec2) -> Rect {
    let x = if mirrored() { rect.right() - off - size.x } else { rect.left() + off };
    Rect::from_min_size(pos2(x, rect.top() + top), size)
}

/// `align` flipped horizontally when mirrored: the anchor of text painted at a start or end
/// position (`LEFT_CENTER` becomes `RIGHT_CENTER`).
pub fn flip(align: Align2) -> Align2 {
    if !mirrored() {
        return align;
    }
    Align2([
        match align.x() {
            Align::Min => Align::Max,
            Align::Center => Align::Center,
            Align::Max => Align::Min,
        },
        align.y(),
    ])
}

/// A margin with `start` against the start edge and `end` against the end edge.
pub fn margin(start: i8, end: i8, top: i8, bottom: i8) -> Margin {
    if mirrored() { Margin { left: end, right: start, top, bottom } } else { Margin { left: start, right: end, top, bottom } }
}

/// Where a popup opens relative to a start-side button: below it, reaching away from its edge.
pub fn menu_align() -> egui::RectAlign {
    if mirrored() { egui::RectAlign::BOTTOM_END } else { egui::RectAlign::BOTTOM_START }
}

/// `parts` joined by `sep` in the interface's reading order, for one line painted as it is: the
/// first part at the start edge. In English that is `parts.join(sep)`. Mirrored, the line runs
/// right to left and its parts are in display order already ([`crate::bidi::display_rtl`]), so
/// their order is reversed: `A · B · C` is painted `C · B · A`, with `A` at the right.
///
/// Only for a list the code assembles from translated pieces. A list of document text (tags,
/// folder paths), or one that goes through a wrapping helper in logical order, is not reversed.
pub fn join(parts: &[&str], sep: &str) -> String {
    if mirrored() { parts.iter().rev().copied().collect::<Vec<_>>().join(sep) } else { parts.join(sep) }
}

/// A translated `caption` (display order, with the colon and the space the code appends after
/// it: `"Issued by: "`) and the `value` it captions, as one painted line. In English that is
/// `caption` followed by `value`. Mirrored, the caption is at the start edge (the right) and the
/// value follows it in reading order, which is to its left on screen; the colon goes to the
/// caption's far end so that it still follows the caption's last word (`value :caption`).
pub fn caption_value(caption: &str, value: &str) -> String {
    if !mirrored() {
        return format!("{caption}{value}");
    }
    let caption = caption.trim_end();
    match caption.strip_suffix(':') {
        Some(words) => format!("{value} :{words}"),
        None => format!("{value} {caption}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Lang, set_current};

    /// Draws in Arabic for the test's lifetime (the language is per thread), then restores it.
    struct Arabic;
    impl Arabic {
        fn on() -> Self {
            set_current(Lang::from_code("ar").expect("ar registered"));
            Self
        }
    }
    impl Drop for Arabic {
        fn drop(&mut self) {
            set_current(Lang::EN);
        }
    }

    const ROW: Rect = Rect { min: pos2(100.0, 40.0), max: pos2(400.0, 74.0) };

    #[test]
    fn english_is_the_identity() {
        assert!(!mirrored());
        assert_eq!(start_layout(Align::Center), Layout::left_to_right(Align::Center));
        assert_eq!(end_layout(Align::Center), Layout::right_to_left(Align::Center));
        assert_eq!(page_layout(), Layout::top_down(Align::Min));
        assert_eq!(text_align(), Align::Min);
        assert_eq!(block_left(106.0, 50.0), 106.0);
        assert_eq!(start(ROW, 6.0), 106.0);
        assert_eq!(end_x(ROW, 6.0), 394.0);
        assert_eq!(start_center(ROW, 6.0), pos2(106.0, 57.0));
        assert_eq!(end_center(ROW, 10.0), pos2(390.0, 57.0));
        assert_eq!(start_rect(ROW, 6.0, 7.0, vec2(20.0, 20.0)), Rect::from_min_size(pos2(106.0, 47.0), vec2(20.0, 20.0)));
        assert_eq!(flip(Align2::LEFT_CENTER), Align2::LEFT_CENTER);
        assert_eq!(flip(Align2::RIGHT_TOP), Align2::RIGHT_TOP);
        assert_eq!(margin(14, 10, 12, 8), Margin { left: 14, right: 10, top: 12, bottom: 8 });
        assert_eq!(menu_align(), egui::RectAlign::BOTTOM_START);
    }

    #[test]
    fn arabic_mirrors_positions() {
        let _arabic = Arabic::on();
        assert!(mirrored());
        assert_eq!(start_layout(Align::Center), Layout::right_to_left(Align::Center));
        assert_eq!(end_layout(Align::Center), Layout::left_to_right(Align::Center));
        assert_eq!(page_layout(), Layout::top_down(Align::Max));
        assert_eq!(text_align(), Align::Max);
        assert_eq!(block_left(394.0, 50.0), 344.0);
        assert_eq!(start(ROW, 6.0), 394.0);
        assert_eq!(end_x(ROW, 6.0), 106.0);
        assert_eq!(start_center(ROW, 6.0), pos2(394.0, 57.0));
        assert_eq!(end_center(ROW, 10.0), pos2(110.0, 57.0));
        assert_eq!(start_rect(ROW, 6.0, 7.0, vec2(20.0, 20.0)), Rect::from_min_size(pos2(374.0, 47.0), vec2(20.0, 20.0)));
        assert_eq!(flip(Align2::LEFT_CENTER), Align2::RIGHT_CENTER);
        assert_eq!(flip(Align2::RIGHT_TOP), Align2::LEFT_TOP);
        assert_eq!(flip(Align2::CENTER_CENTER), Align2::CENTER_CENTER);
        assert_eq!(margin(14, 10, 12, 8), Margin { left: 10, right: 14, top: 12, bottom: 8 });
        assert_eq!(menu_align(), egui::RectAlign::BOTTOM_END);
    }

    #[test]
    fn a_start_box_and_an_end_point_stay_inside_the_row_on_opposite_sides() {
        for arabic in [false, true] {
            let _guard = arabic.then(Arabic::on);
            let a = start_rect(ROW, 6.0, 7.0, vec2(20.0, 20.0));
            let b = end_center(ROW, 10.0);
            assert!(ROW.contains_rect(a) && ROW.contains(b));
            assert_eq!(a.center().x < b.x, !arabic, "the start box is left of the end point only in English");
        }
    }

    #[test]
    fn panels_swap_sides() {
        // The panel's side is not readable back, so check it through a laid-out frame.
        fn left_edge(arabic: bool) -> bool {
            let _guard = arabic.then(Arabic::on);
            let ctx = egui::Context::default();
            let mut at_left = false;
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let shown = panel("probe", Side::Start).exact_size(50.0).show(ui, |_| {});
                at_left = shown.response.rect.center().x < ui.max_rect().center().x;
            });
            out.textures_delta.clear();
            at_left
        }
        assert!(left_edge(false), "the start panel is on the left in English");
        assert!(!left_edge(true), "and on the right in Arabic");
    }

    #[test]
    fn a_row_with_no_width_to_sit_against_stays_finite() {
        let _arabic = Arabic::on();
        let ctx = egui::Context::default();
        let mut rects = Vec::new();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                // A region as wide as the world, as a window that fits its contents offers.
                let world = Rect::from_min_size(pos2(0.0, 0.0), vec2(f32::INFINITY, 400.0));
                ui.scope_builder(egui::UiBuilder::new().max_rect(world), |ui| {
                    rects.push(horizontal(ui, |ui| ui.label("a")).response.rect);
                    rects.push(horizontal_centered(ui, |ui| ui.label("b")).response.rect);
                    rects.push(vertical(ui, |ui| ui.label("c")).response.rect);
                });
            });
        });
        out.textures_delta.clear();
        assert_eq!(rects.len(), 3);
        for rect in rects {
            assert!(rect.min.is_finite() && rect.max.is_finite(), "{rect:?}");
        }
    }

    #[test]
    fn horizontal_puts_the_first_item_at_the_start() {
        for arabic in [false, true] {
            let _guard = arabic.then(Arabic::on);
            let ctx = egui::Context::default();
            let mut xs = (0.0, 0.0);
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    horizontal(ui, |ui| {
                        let a = ui.allocate_space(vec2(20.0, 20.0)).1.center().x;
                        let b = ui.allocate_space(vec2(20.0, 20.0)).1.center().x;
                        xs = (a, b);
                    });
                });
            });
            out.textures_delta.clear();
            assert_eq!(xs.0 < xs.1, !arabic, "first item left of the second only in English: {xs:?}");
        }
    }

    /// A block in a row grows down from the row's top, as `ui.vertical` does, not from the middle
    /// line of the row.
    #[test]
    fn a_block_in_a_row_starts_at_the_top_of_the_row() {
        for arabic in [false, true] {
            let _guard = arabic.then(Arabic::on);
            let ctx = egui::Context::default();
            let mut tops = (0.0, 0.0);
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let row = Rect::from_min_size(ui.cursor().min, vec2(300.0, 90.0));
                    ui.scope_builder(UiBuilder::new().max_rect(row).layout(start_layout(Align::Center)), |ui| {
                        tops.0 = row.top();
                        tops.1 = vertical(ui, |ui| ui.allocate_space(vec2(20.0, 60.0))).response.rect.top();
                    });
                });
            });
            out.textures_delta.clear();
            assert!((tops.1 - tops.0).abs() < 0.5, "arabic {arabic}: the block starts at {} in a row from {}", tops.1, tops.0);
        }
    }

    #[test]
    fn lists_and_captions_are_the_identity_in_english() {
        assert_eq!(join(&["A", "B", "C"], " · "), "A · B · C");
        assert_eq!(join(&["only"], " · "), "only");
        assert_eq!(join(&[], " · "), "");
        assert_eq!(join(&["A", "", "C"], "-"), "A--C", "as slice::join does");
        assert_eq!(caption_value("Issued by: ", "Contoso CA"), "Issued by: Contoso CA");
        assert_eq!(caption_value("", "x"), "x");
        assert_eq!(caption_value("Issued by: ", ""), "Issued by: ");
    }

    #[test]
    fn lists_run_from_the_start_edge_when_mirrored() {
        let _arabic = Arabic::on();
        assert_eq!(join(&["A", "B", "C"], " · "), "C · B · A");
        assert_eq!(join(&["only"], " · "), "only");
        assert_eq!(join(&[], " · "), "");
        // The catalog's own pieces, as a card blurb joins them: the first at the right.
        let (a, b) = (crate::i18n::t("Save"), crate::i18n::t("Open"));
        assert_ne!(a, "Save", "Save is translated");
        assert_eq!(join(&[a, b], " · "), format!("{b} · {a}"));
        // A caption's value follows it in reading order, which is on its left, and the colon
        // follows the caption's last word.
        assert_eq!(caption_value("صادرة عن: ", "Contoso"), "Contoso :صادرة عن");
        assert_eq!(caption_value("صادرة عن", "Contoso"), "Contoso صادرة عن");
        assert_eq!(caption_value("صادرة عن: ", ""), " :صادرة عن");
    }

    #[test]
    fn the_signing_dialog_captions_read_value_after_caption_when_mirrored() {
        let _arabic = Arabic::on();
        let issued = crate::i18n::t("Issued by: ");
        let line = caption_value(issued, "Contoso");
        assert!(line.starts_with("Contoso :") && !line.ends_with(' '), "{line:?} from {issued:?}");
    }
}
