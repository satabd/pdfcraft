//! Two-column forms (a caption and the field it names) for dialogs, in either reading direction.
//!
//! In a left-to-right language [`form_grid`] is an `egui::Grid`, and the rows go in as they
//! always did. Mirrored, `egui::Grid` cannot sit against the right edge (it measures infinite in
//! a layout aligned there), so the rows are laid out by hand: the caption in a column of its own
//! at the right, as wide as the widest caption, and the field to its left, with the same row
//! heights. Hit rectangles are the painted ones: the cells are ordinary layouts, not painted
//! over a left-to-right layout.

use egui::{Align, Layout, Rect, Response, RichText, Sense, Ui, UiBuilder, Vec2, pos2, vec2};

use crate::rtl;

/// The widest share of the form a caption column may take, so a long caption leaves its field a
/// usable width.
const CAPTION_SHARE: f32 = 0.6;

/// The most rows whose height a mirrored form remembers; later rows are laid out at the minimum
/// height the first time and are never stored.
const MAX_REMEMBERED_ROWS: usize = 4096;

/// What a mirrored form remembers from the previous frame, as `egui::Grid` does: the caption
/// column's width and each row's height are only known once the rows have been laid out.
#[derive(Clone, Debug, Default, PartialEq)]
struct FormState {
    caption_width: f32,
    row_heights: Vec<f32>,
}

/// A mirrored form's geometry while its rows are added.
struct Mirrored {
    left: f32,
    right: f32,
    /// Gaps between the cells and between the rows.
    spacing: Vec2,
    /// The item spacing the cells' contents keep, the same as in an `egui::Grid` cell.
    cell_spacing: Vec2,
    min_row_height: f32,
    min_caption_width: f32,
    previous: FormState,
    widest_caption: f32,
    /// Rows added so far (also those past [`MAX_REMEMBERED_ROWS`]).
    rows: usize,
    row_heights: Vec<f32>,
}

impl Mirrored {
    /// Count a finished row of height `used` and remember the height for the next frame.
    fn remember_row(&mut self, used: f32) {
        self.rows = self.rows.saturating_add(1);
        if self.row_heights.len() < MAX_REMEMBERED_ROWS {
            self.row_heights.push(finite_or(used, self.min_row_height));
        }
    }
}

/// The rows of a [`form_grid`].
pub struct FormRows<'a> {
    ui: &'a mut Ui,
    /// `None` inside an `egui::Grid`.
    mirrored: Option<Mirrored>,
}

/// A two-column form. `spacing` is the gap between cells and between rows; `min_col_width` the
/// narrowest the caption column may be (`None`: the style's `interact_size.x`).
///
/// Fill it with [`FormRows::row`], [`FormRows::row_styled`], [`FormRows::row_with`] and
/// [`FormRows::heading`]. A form needs a finite width; where there is none (a window that fits
/// its contents) it is the plain left-to-right grid even in a right-to-left language.
pub fn form_grid<R>(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    spacing: impl Into<Vec2>,
    min_col_width: Option<f32>,
    add_rows: impl FnOnce(&mut FormRows<'_>) -> R,
) -> R {
    form_grid_of(ui, id, spacing, min_col_width, 2, add_rows)
}

/// [`form_grid`] whose `egui::Grid` is told it has `columns` columns, for a form whose rows have a
/// third cell ([`FormRows::row_with_note`]). `egui::Grid` sizes the last column from the room that
/// is left, so the count it is given is part of how a left-to-right form is laid out. Mirrored, a
/// form has the two columns of its captions and fields.
pub fn form_grid_of<R>(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    spacing: impl Into<Vec2>,
    min_col_width: Option<f32>,
    columns: usize,
    add_rows: impl FnOnce(&mut FormRows<'_>) -> R,
) -> R {
    let spacing = spacing.into();
    let min_col_width = min_col_width.filter(|width| width.is_finite());
    if !(rtl::mirrored() && ui.available_width().is_finite()) {
        let mut grid = egui::Grid::new(id).num_columns(columns).spacing(spacing);
        if let Some(width) = min_col_width {
            grid = grid.min_col_width(width);
        }
        return grid.show(ui, |ui| add_rows(&mut FormRows { ui, mirrored: None })).inner;
    }
    let spacing = vec2(finite_or(spacing.x, 0.0).max(0.0), finite_or(spacing.y, 0.0).max(0.0));
    let id = ui.make_persistent_id(id);
    let previous = ui.data(|d| d.get_temp::<FormState>(id));
    let known = previous.is_some();
    if !known && ui.is_visible() {
        // The first frame does not know the caption column yet; show the next one instead.
        ui.ctx().request_discard("new form_grid");
    }
    let previous = previous.unwrap_or_default();
    let cell_spacing = ui.spacing().item_spacing;
    let min_row_height = finite_or(ui.spacing().interact_size.y, 0.0);
    let min_caption_width = finite_or(min_col_width.unwrap_or(ui.spacing().interact_size.x), 0.0);
    ui.scope(|ui| {
        // The rows are spaced by the form's own gap; the cells restore the style's.
        ui.spacing_mut().item_spacing = spacing;
        let avail = ui.available_rect_before_wrap();
        let mirrored = Mirrored {
            left: avail.left(),
            right: avail.right(),
            spacing,
            cell_spacing,
            min_row_height,
            min_caption_width,
            previous: previous.clone(),
            widest_caption: 0.0,
            rows: 0,
            row_heights: Vec::new(),
        };
        let mut rows = FormRows { ui, mirrored: Some(mirrored) };
        let result = add_rows(&mut rows);
        if let Some(m) = rows.mirrored.take() {
            let state = FormState { caption_width: m.widest_caption.max(m.min_caption_width), row_heights: m.row_heights };
            if !known || state != previous {
                // The next frame lays the rows out with what this one measured.
                rows.ui.ctx().request_repaint();
                rows.ui.data_mut(|d| d.insert_temp(id, state));
            }
        }
        result
    })
    .inner
}

/// `value`, or `fallback` when it is NaN or infinite.
fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

/// A caption as the mirrored form paints it. The catalog keeps a short caption's closing colon
/// at the visual right ([`crate::bidi::display_rtl`]), next to a field that used to sit there.
/// The caption is now at the right of its field, so the colon belongs at the visual left of the
/// caption's last word, where the Unicode bidi algorithm puts it: `"الاسم:"` becomes `":الاسم"`.
/// Anything else, and every caption in a left-to-right language, comes back as it is.
fn caption_text(caption: &str, mirrored: bool) -> std::borrow::Cow<'_, str> {
    if mirrored
        && let Some(words) = caption.trim_end().strip_suffix(':')
        && !words.is_empty()
    {
        return std::borrow::Cow::Owned(format!(":{words}"));
    }
    std::borrow::Cow::Borrowed(caption)
}

impl FormRows<'_> {
    /// A row: the `caption` and the field `add_field` puts after it. The field gets the caption's
    /// response, for `labelled_by`.
    pub fn row(&mut self, caption: &str, add_field: impl FnOnce(&mut Ui, &Response)) {
        self.row_styled(caption, |text| text, add_field);
    }

    /// [`row`](Self::row) with the caption styled by `style` (a colour, a font).
    pub fn row_styled(&mut self, caption: &str, style: impl FnOnce(RichText) -> RichText, add_field: impl FnOnce(&mut Ui, &Response)) {
        let text = caption_text(caption, self.mirrored.is_some());
        self.row_with(move |ui| ui.label(style(RichText::new(text))), add_field);
    }

    /// A row whose caption is whatever `add_caption` adds (an icon, a custom label) and returns
    /// the response of.
    pub fn row_with(&mut self, add_caption: impl FnOnce(&mut Ui) -> Response, add_field: impl FnOnce(&mut Ui, &Response)) {
        let Self { ui, mirrored } = self;
        let Some(m) = mirrored else {
            let caption = add_caption(ui);
            add_field(ui, &caption);
            ui.end_row();
            return;
        };
        let row = m.rows;
        let top = ui.cursor().top();
        let width = (m.right - m.left).max(0.0);
        let caption_width = finite_or(m.previous.caption_width, 0.0).max(0.0).min(width * CAPTION_SHARE);
        let height = m.previous.row_heights.get(row).copied().map_or(m.min_row_height, |h| finite_or(h, 0.0).max(m.min_row_height));
        let caption_rect = Rect::from_min_max(pos2(m.right - caption_width, top), pos2(m.right, top + height));
        let field_right = (m.right - caption_width - m.spacing.x).max(m.left);
        let field_rect = Rect::from_min_max(pos2(m.left, top), pos2(field_right, top + height));
        let mut caption_ui = cell(ui, caption_rect, ("caption", row), m.cell_spacing);
        // A caption is measured whole, never wrapped to the column it is measured for.
        caption_ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        let caption = add_caption(&mut caption_ui);
        let caption_used = caption_ui.min_rect();
        let mut field_ui = cell(ui, field_rect, ("field", row), m.cell_spacing);
        add_field(&mut field_ui, &caption);
        let used = height.max(caption_used.height()).max(field_ui.min_rect().height());
        m.widest_caption = m.widest_caption.max(finite_or(caption_used.width(), 0.0));
        m.remember_row(used);
        ui.allocate_rect(Rect::from_min_max(pos2(m.left, top), pos2(m.right, top + used)), Sense::hover());
    }

    /// A row whose caption is whatever `add_caption` adds, then the field, then a note about the
    /// field (`add_note`: a "Mixed" tag). In the `egui::Grid` the note is the cell of a third
    /// column; the mirrored form has two columns, so the note goes in the field's cell, after the
    /// field in reading order (at its left).
    pub fn row_with_note(
        &mut self,
        add_caption: impl FnOnce(&mut Ui) -> Response,
        add_field: impl FnOnce(&mut Ui, &Response),
        add_note: impl FnOnce(&mut Ui),
    ) {
        if self.mirrored.is_some() {
            self.row_with(add_caption, |ui, caption| {
                add_field(ui, caption);
                add_note(ui);
            });
            return;
        }
        let caption = add_caption(self.ui);
        add_field(self.ui, &caption);
        add_note(self.ui);
        self.ui.end_row();
    }

    /// A row of one cell across the form, against its start edge: a section title, a row of
    /// buttons. (In the `egui::Grid` it is the cell of the first column, and the row ends.)
    pub fn full_row(&mut self, add_cell: impl FnOnce(&mut Ui)) {
        let Self { ui, mirrored } = self;
        let Some(m) = mirrored else {
            add_cell(ui);
            ui.end_row();
            return;
        };
        let row = m.rows;
        let top = ui.cursor().top();
        let rect = Rect::from_min_max(pos2(m.left, top), pos2(m.right, top + m.min_row_height));
        let mut cell_ui = cell(ui, rect, ("full", row), m.cell_spacing);
        add_cell(&mut cell_ui);
        let used = m.min_row_height.max(finite_or(cell_ui.min_rect().height(), 0.0));
        m.remember_row(used);
        ui.allocate_rect(Rect::from_min_max(pos2(m.left, top), pos2(m.right, top + used)), Sense::hover());
    }

    /// A section title across the form.
    pub fn heading(&mut self, text: RichText) {
        self.full_row(|ui| _ = ui.label(text));
    }
}

/// What [`start_block`] remembers from the previous frame: how wide the block was.
#[derive(Clone, Copy, Debug, PartialEq)]
struct BlockWidth(f32);

/// `add_contents` as a block that sits against the start edge of the space it is given, for
/// content that can only be laid out from its own left edge (an `egui::Grid` of swatches, say)
/// inside a field of a mirrored [`form_grid`]. In a left-to-right language, and where the width
/// is unbounded, it is `add_contents(ui)` as it is.
///
/// Mirrored, the block is measured in one frame and placed against the right edge in the next
/// (the first frame of a new block is discarded, as for a [`form_grid`]).
pub fn start_block<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    if !(rtl::mirrored() && ui.available_width().is_finite()) {
        return add_contents(ui);
    }
    let id = ui.next_auto_id();
    ui.skip_ahead_auto_ids(1);
    let previous = ui.data(|d| d.get_temp::<BlockWidth>(id));
    if previous.is_none() && ui.is_visible() {
        ui.ctx().request_discard("new start_block");
    }
    let avail = ui.available_rect_before_wrap();
    let width = previous.map_or(avail.width(), |BlockWidth(w)| finite_or(w, 0.0).clamp(0.0, avail.width().max(0.0)));
    let rect = Rect::from_min_max(pos2((avail.right() - width).max(avail.left()), avail.top()), avail.right_bottom());
    let mut block = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)).id_salt(id));
    let result = add_contents(&mut block);
    let used = block.min_rect();
    // Sit against the right edge even when the block is narrower than it was measured.
    let used = Rect::from_min_max(pos2((avail.right() - used.width()).max(avail.left()), used.top()), pos2(avail.right(), used.bottom()));
    ui.allocate_rect(used, Sense::hover());
    let measured = BlockWidth(finite_or(block.min_rect().width(), 0.0));
    if previous != Some(measured) {
        ui.ctx().request_repaint();
        ui.data_mut(|d| d.insert_temp(id, measured));
    }
    result
}

/// A cell of a mirrored form: a right-to-left row centred in `rect`, whose contents keep the
/// style's item spacing.
fn cell(ui: &mut Ui, rect: Rect, salt: (&'static str, usize), item_spacing: Vec2) -> Ui {
    let mut cell = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::right_to_left(Align::Center)).id_salt(salt));
    cell.spacing_mut().item_spacing = item_spacing;
    cell
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Lang, set_current};

    /// Draws in Arabic for the test's lifetime (the language is per thread), then restores it.
    struct ArabicUi;
    impl ArabicUi {
        fn on() -> Self {
            set_current(Lang::from_code("ar").expect("ar registered"));
            Self
        }
    }
    impl Drop for ArabicUi {
        fn drop(&mut self) {
            set_current(Lang::EN);
        }
    }

    const WIDTH: f32 = 400.0;

    /// Where each caption and field of a two-row form ended up, in the last of a few frames.
    #[derive(Debug, Default)]
    struct Placed {
        captions: Vec<Rect>,
        fields: Vec<Rect>,
    }

    /// The same two rows through `egui::Grid` (`grid`) or [`form_grid`].
    fn run(grid: bool) -> Placed {
        let ctx = egui::Context::default();
        let mut placed = Placed::default();
        for _ in 0..4 {
            placed = Placed::default();
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(WIDTH, 300.0))), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.spacing_mut().interact_size.y = 30.0;
                    let mut text = String::from("Ada");
                    if grid {
                        egui::Grid::new("form").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                            placed.captions.push(ui.label("Name").rect);
                            placed.fields.push(ui.add(egui::TextEdit::singleline(&mut text).desired_width(120.0)).rect);
                            ui.end_row();
                            placed.captions.push(ui.label("Pages").rect);
                            placed.fields.push(ui.button("All").rect);
                            ui.end_row();
                        });
                    } else {
                        form_grid(ui, "form", [12.0, 8.0], None, |rows| {
                            rows.row("Name", |ui, caption| {
                                placed.captions.push(caption.rect);
                                placed.fields.push(ui.add(egui::TextEdit::singleline(&mut text).desired_width(120.0)).rect);
                            });
                            rows.row("Pages", |ui, caption| {
                                placed.captions.push(caption.rect);
                                placed.fields.push(ui.button("All").rect);
                            });
                        });
                    }
                });
            });
            out.textures_delta.clear();
        }
        placed
    }

    /// The caption, field and note of two rows, in the last of a few frames: through a three-column
    /// `egui::Grid` (`grid`) or [`form_grid_of`] with [`FormRows::row_with_note`].
    fn run_notes(grid: bool) -> Vec<[Rect; 3]> {
        let ctx = egui::Context::default();
        let mut placed = Vec::new();
        for _ in 0..4 {
            placed.clear();
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(WIDTH, 300.0))), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    if grid {
                        egui::Grid::new("notes").num_columns(3).spacing([12.0, 8.0]).show(ui, |ui| {
                            for (caption, note) in [("Name", "Mixed"), ("Pages", "")] {
                                let cells = [ui.label(caption).rect, ui.button("All").rect, ui.label(note).rect];
                                placed.push(cells);
                                ui.end_row();
                            }
                        });
                    } else {
                        form_grid_of(ui, "notes", [12.0, 8.0], None, 3, |rows| {
                            for (caption, note) in [("Name", "Mixed"), ("Pages", "")] {
                                let cells = [Rect::NOTHING; 3].map(std::cell::Cell::new);
                                rows.row_with_note(
                                    |ui| {
                                        let label = ui.label(caption);
                                        cells[0].set(label.rect);
                                        label
                                    },
                                    |ui, _| cells[1].set(ui.button("All").rect),
                                    |ui| cells[2].set(ui.label(note).rect),
                                );
                                let cells = cells.map(|cell| cell.get());
                                placed.push(cells);
                            }
                        });
                    }
                });
            });
            out.textures_delta.clear();
        }
        placed
    }

    #[test]
    fn english_notes_are_the_cells_of_a_third_grid_column() {
        let (grid, form) = (run_notes(true), run_notes(false));
        assert_eq!(grid.len(), 2);
        assert_eq!(format!("{grid:?}"), format!("{form:?}"));
        for [caption, field, note] in &form {
            assert!(caption.right() <= field.left() && field.right() <= note.left(), "{caption:?} {field:?} {note:?}");
        }
    }

    #[test]
    fn a_note_is_left_of_its_field_in_arabic() {
        let _arabic = ArabicUi::on();
        let form = run_notes(false);
        assert_eq!(form.len(), 2);
        for [caption, field, note] in &form {
            assert!(field.right() <= caption.left(), "the field {field:?} is left of its caption {caption:?}");
            assert!(note.right() <= field.left() + 0.5, "and its note {note:?} is left of the field {field:?}");
            assert!(caption.right() <= WIDTH && note.left() >= 0.0, "inside the form: {caption:?} {note:?}");
        }
    }

    #[test]
    fn english_is_the_egui_grid() {
        let (grid, form) = (run(true), run(false));
        assert_eq!(grid.captions.len(), 2);
        assert_eq!(format!("{:?}", grid.captions), format!("{:?}", form.captions));
        assert_eq!(format!("{:?}", grid.fields), format!("{:?}", form.fields));
    }

    #[test]
    fn the_caption_is_right_of_its_field_in_arabic() {
        let english = run(false);
        let arabic = ArabicUi::on();
        let form = run(false);
        drop(arabic);
        assert_eq!(form.captions.len(), 2);
        for (caption, field) in form.captions.iter().zip(&form.fields) {
            assert!(caption.left() >= field.right(), "the caption {caption:?} is right of its field {field:?}");
            assert!((caption.center().y - field.center().y).abs() < 0.5, "and level with it: {caption:?} {field:?}");
            assert!(caption.right() <= WIDTH, "inside the form: {caption:?}");
        }
        // Both captions start at the same right edge, and the fields start at the same edge.
        assert!((form.captions[0].right() - form.captions[1].right()).abs() < 0.5, "{:?}", form.captions);
        assert!((form.fields[0].right() - form.fields[1].right()).abs() < 0.5, "{:?}", form.fields);
        // The rows are as tall, and as far apart, as in English.
        let pitch = |p: &Placed| p.fields[1].center().y - p.fields[0].center().y;
        assert!((pitch(&form) - pitch(&english)).abs() < 0.5, "arabic {} english {}", pitch(&form), pitch(&english));
        assert!((form.fields[0].height() - english.fields[0].height()).abs() < 0.5);
    }

    #[test]
    fn a_tall_field_makes_its_row_taller_and_the_caption_follows_its_centre() {
        let _arabic = ArabicUi::on();
        let ctx = egui::Context::default();
        let mut got = (Rect::NOTHING, Rect::NOTHING, Rect::NOTHING);
        for _ in 0..4 {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(WIDTH, 400.0))), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    form_grid(ui, "tall", [12.0, 8.0], None, |rows| {
                        rows.row("Show", |ui, caption| {
                            got.0 = caption.rect;
                            got.1 = rtl::vertical(ui, |ui| {
                                for _ in 0..3 {
                                    ui.label("line");
                                    ui.add_space(20.0);
                                }
                            })
                            .response
                            .rect;
                        });
                        rows.row("Next", |_, caption| got.2 = caption.rect);
                    });
                });
            });
            out.textures_delta.clear();
        }
        assert!(got.1.height() > 60.0, "{:?}", got.1);
        assert!((got.0.center().y - got.1.center().y).abs() < 1.0, "caption {:?} against field {:?}", got.0, got.1);
        assert!(got.2.top() >= got.1.bottom(), "the next row starts below the tall one");
    }

    #[test]
    fn a_caption_wider_than_the_form_or_a_form_without_width_stays_finite() {
        let _arabic = ArabicUi::on();
        let ctx = egui::Context::default();
        let long = "a very long caption ".repeat(30);
        let mut rects = Vec::new();
        for width in [WIDTH, f32::INFINITY, 0.0] {
            for _ in 0..3 {
                let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let world = Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 400.0));
                        ui.scope_builder(UiBuilder::new().max_rect(world), |ui| {
                            form_grid(ui, ("wide", width.to_bits()), [12.0, 8.0], Some(f32::NAN), |rows| {
                                rows.row(&long, |ui, caption| {
                                    rects.push(caption.rect);
                                    rects.push(ui.label("field").rect);
                                });
                                rows.heading(RichText::new("Section"));
                            });
                        });
                    });
                });
                out.textures_delta.clear();
            }
        }
        for rect in rects {
            assert!(rect.min.is_finite() && rect.max.is_finite() && !rect.any_nan(), "{rect:?}");
        }
    }

    /// A block holding an `egui::Grid`, in a field of a form: where the grid sat in the first
    /// frame, and the caption next to it.
    fn run_block(arabic: bool) -> (Rect, Rect, Rect) {
        let _arabic = arabic.then(ArabicUi::on);
        let ctx = egui::Context::default();
        let mut got = (Rect::NOTHING, Rect::NOTHING, Rect::NOTHING);
        for _ in 0..5 {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(WIDTH, 300.0))), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    got.0 = ui.available_rect_before_wrap();
                    form_grid(ui, "block", [12.0, 8.0], None, |rows| {
                        rows.row("Colour", |ui, caption| {
                            got.1 = caption.rect;
                            got.2 = start_block(ui, |ui| {
                                egui::Grid::new("swatches")
                                    .show(ui, |ui| {
                                        for row in 0..2 {
                                            for _ in 0..3 {
                                                ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                                            }
                                            if row == 0 {
                                                ui.end_row();
                                            }
                                        }
                                    })
                                    .response
                                    .rect
                            });
                        });
                    });
                });
            });
            out.textures_delta.clear();
        }
        got
    }

    #[test]
    fn a_start_block_keeps_a_grid_beside_its_caption_in_arabic_and_is_the_plain_grid_in_english() {
        let (panel, caption, grid) = run_block(true);
        assert!(grid.min.is_finite() && grid.max.is_finite(), "{grid:?}");
        assert!(grid.width() > 60.0, "the grid keeps its natural width: {grid:?}");
        assert!(grid.right() <= caption.left(), "the grid {grid:?} is left of its caption {caption:?}");
        assert!(caption.right() <= panel.right() + 0.5, "inside the form: {caption:?} {panel:?}");
        assert!(caption.left() - grid.right() < 40.0, "and next to it, not at the far left: {grid:?} {caption:?}");
        let (panel, caption, grid) = run_block(false);
        assert!(caption.right() <= grid.left() && (grid.left() - caption.right()) < 40.0, "{caption:?} {grid:?}");
        assert!(grid.left() >= panel.left());
    }

    /// A short caption's closing colon moves to the visual left of the caption (the side of the
    /// field), in display order; nothing else changes.
    #[test]
    fn the_caption_colon_moves_to_the_side_of_the_field() {
        assert_eq!(caption_text("الاسم:", true), ":الاسم");
        assert_eq!(caption_text("عن صادرة: ", true), ":عن صادرة");
        assert_eq!(caption_text("Show:", true), ":Show");
        assert_eq!(caption_text("الاسم", true), "الاسم");
        assert_eq!(caption_text(":", true), ":", "a lone colon has no caption to move it from");
        assert_eq!(caption_text("", true), "");
        assert_eq!(caption_text(":استخدامه تريد الذي", true), ":استخدامه تريد الذي", "a colon already at the left stays");
        assert_eq!(caption_text("Direction:", false), "Direction:");
        assert_eq!(caption_text("الاسم:", false), "الاسم:");
    }

    #[test]
    fn a_form_caption_paints_its_colon_at_the_left_in_arabic() {
        let _arabic = ArabicUi::on();
        let direction = crate::i18n::t("Direction:");
        assert!(direction.ends_with(':'), "the catalog keeps the colon at the visual right: {direction:?}");
        let ctx = egui::Context::default();
        let mut painted = Vec::new();
        for _ in 0..3 {
            let mut out = ctx.run_ui(
                egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(WIDTH, 200.0))), ..Default::default() },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        form_grid(ui, "colon", [12.0, 8.0], None, |rows| rows.row(direction, |ui, _| _ = ui.label("x")));
                    });
                },
            );
            painted =
                out.shapes.iter().filter_map(|c| if let egui::Shape::Text(t) = &c.shape { Some(t.galley.text().to_owned()) } else { None }).collect();
            out.textures_delta.clear();
        }
        let expected = format!(":{}", direction.trim_end().trim_end_matches(':'));
        assert!(painted.contains(&expected), "{expected:?} in {painted:?}");
        assert!(!painted.iter().any(|t| t == direction), "not the catalog string as it is: {painted:?}");
    }
}
