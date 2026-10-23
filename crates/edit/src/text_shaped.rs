//! Arabic replacement text in Edit text: shaped, with the typing order kept for copy and search.
//!
//! The page's own font can't draw Arabic that was typed into Edit text. Producers map a letter's
//! initial, medial and final forms to one Unicode letter, so a subset font can't say which glyph
//! a letter needs, and a font with no Arabic in it has nothing to draw. Replacement text is
//! therefore laid out like Add text lays it out ([`crate::added`]): the Unicode bidirectional
//! algorithm (UAX #9) orders each line, the craft-fonts Arabic face shapes it (joined forms,
//! lam-alef, marks), and the glyphs are drawn from Type 3 fonts built from that face's outlines.
//!
//! Each line is one marked-content span with `/ActualText` holding the line in typing order, so
//! copy, search and a later Edit text read the line as it was typed. Latin text and digits in the
//! line are drawn with the paragraph's own font when it has them and with Helvetica otherwise,
//! between the shaped runs, in the same span. In a paragraph of several looks (sizes, colours, fonts) each line is drawn as
//! runs of one look, each character in the look of the stored character it came from (`text_looks`, [`plan_looked`]).
//! A bold paragraph, or a bold look, has its Arabic drawn at the face's bold instance (`wght` 700), from Type 3 fonts of
//! its own (`PCEdAr…B`); a build whose face has no `wght` axis draws it regular.
//!
//! The paragraph's text state keeps no character or word spacing here (spacing would pull joined
//! letters apart; word spacing reaches only a single-byte code 32, which these fonts' codes are not). A justified
//! paragraph is stretched by `TJ` adjustments after its spaces instead ([`ShapedPlan::line_ops_stretched`]); the
//! rows it does not stretch (the last, one before a line break, one with a single word) of a right-to-left paragraph
//! are right-aligned.

use pdfcraft_content::Op;
use pdfcraft_cos::{Dict, Document};
use pdfcraft_fonts::GlyphError;
use pdfcraft_fonts::shaped_text::{self, PlanFonts, ShapedTextError, TextOp};

use crate::EditError;
use crate::added::{Shaper, arabic_error};

/// The `/Name` of the Type 3 fonts generated for Arabic text.
pub(crate) const ARABIC_FONT_NAME: &str = "PCArabic";

/// The start of the resource names of the fonts generated for Edit text: a name nothing on the
/// page has, since a full save renumbers objects but keeps resource names, and Add text's `PCAr`
/// fonts are matched by their prefix.
const FONT_PREFIX: &str = "PCEdAr";

/// Where Arabic gets its shapes and outlines: the craft-fonts face (a test may have replaced it
/// on its thread, see `shaped_text::set_test_shaper`).
pub(crate) fn shaper() -> Shaper {
    shaped_text::current_shaper()
}

pub(crate) use shaped_text::{has_arabic, latin_of, with_mirrors};

/// Most bytes of replacement text laid out and shaped in one edit: every character becomes a
/// shaped cluster in memory.
const MAX_TEXT: usize = 1_000_000;

/// Whether `draft` can be shaped at all: this build has the Arabic face to draw it with, and the
/// text is of a size a person types. Checked before anything is changed.
pub(crate) fn require_face(draft: &str) -> Result<(), EditError> {
    if !(shaper().arabic_available)() {
        return Err(arabic_error(GlyphError::NoFont, ""));
    }
    if draft.len() > MAX_TEXT {
        return Err(EditError::Invalid("the Arabic text is too long to shape in one edit".into()));
    }
    Ok(())
}

pub(crate) fn mixed_scripts_error() -> EditError {
    shaped_text::mixed_scripts_error().into()
}

/// The characters of `latin` that neither the page's font nor Helvetica can show next to Arabic.
pub(crate) fn unsupported_with_arabic(latin: &str) -> EditError {
    EditError::Invalid(format!(
        "\"{latin}\" has characters that can't be drawn next to Arabic text: only Arabic, Latin letters, digits and punctuation the page font or Helvetica can show are supported"
    ))
}

pub(crate) use pdfcraft_fonts::Weight;
pub(crate) use shaped_text::{LatinWidth, Measure};

/// The operators that show Latin text in a font, or why the font can't.
pub(crate) type ShowLatin<'a> = &'a dyn Fn(&str) -> Result<Vec<Op>, EditError>;

/// How the Latin pieces of a line are drawn: in the resource `font`, with the operators `show`
/// gives (or an error when the font can't show a piece).
pub(crate) struct LatinShow<'a> {
    pub font: &'a str,
    pub show: ShowLatin<'a>,
}

/// Wrapped lines laid out, with the fonts they need already added to the document.
pub(crate) struct ShapedPlan(shaped_text::ShapedPlan);

fn to_text_op(op: &Op) -> TextOp {
    TextOp { operator: op.op.clone(), operands: op.operands.clone() }
}

fn to_op(op: TextOp) -> Op {
    Op { op: op.operator, operands: op.operands, inline: None, span: 0..0 }
}

/// `show`, in the planner's own operator and error types; an error of `show`'s is kept in `failure`.
fn adapted<'a>(
    show: ShowLatin<'a>,
    failure: &'a std::cell::RefCell<Option<EditError>>,
) -> impl Fn(&str) -> Result<Vec<TextOp>, ShapedTextError> + 'a {
    move |s: &str| {
        show(s).map(|ops| ops.iter().map(to_text_op).collect()).map_err(|e| {
            let message = e.to_string();
            if let Ok(mut slot) = failure.try_borrow_mut() {
                *slot = Some(e);
            }
            ShapedTextError(message)
        })
    }
}

/// Lay out `lines` (in typing order, in a paragraph that runs `rtl` or not), read every outline
/// and add the fonts to `doc` and `fonts_res`. Everything that can fail does so before `doc` is
/// touched. `max_glyphs` is the most glyphs one generated font holds (tests lower it). The Arabic is drawn at `weight`.
pub(crate) fn plan(
    doc: &mut Document,
    fonts_res: &mut Dict,
    lines: &[String],
    rtl: bool,
    max_glyphs: usize,
    latin: &LatinShow,
    weight: Weight,
) -> Result<ShapedPlan, EditError> {
    let lines: Vec<(String, Vec<usize>)> = lines.iter().map(|line| (line.clone(), Vec::new())).collect();
    plan_looked(doc, fonts_res, &lines, rtl, max_glyphs, std::slice::from_ref(latin), &[weight])
}

/// [`plan`] for a paragraph of several looks: each line with the look of each of its characters (an index into
/// `latins`, which shows the Latin text of each look, and into `weights`, the weight its Arabic is drawn at; see
/// [`shaped_text::shaped_pieces_weighted`]).
pub(crate) fn plan_looked(
    doc: &mut Document,
    fonts_res: &mut Dict,
    lines: &[(String, Vec<usize>)],
    rtl: bool,
    max_glyphs: usize,
    latins: &[LatinShow],
    weights: &[Weight],
) -> Result<ShapedPlan, EditError> {
    // An error of the writer's own is kept as it is; the planner's is turned into an `EditError`.
    let failure: std::cell::RefCell<Option<EditError>> = std::cell::RefCell::new(None);
    let shows: Vec<_> = latins.iter().map(|latin| adapted(latin.show, &failure)).collect();
    let planner: Vec<shaped_text::LatinShow> =
        latins.iter().zip(&shows).map(|(latin, show)| shaped_text::LatinShow { font: latin.font, show }).collect();
    let fonts = PlanFonts { max_glyphs, name_prefix: FONT_PREFIX, font_name: Some(ARABIC_FONT_NAME) };
    let planned = shaped_text::plan_looked(doc, fonts_res, lines, rtl, fonts, &planner, weights);
    drop(planner);
    drop(shows);
    match planned {
        Ok(p) => Ok(ShapedPlan(p)),
        Err(e) => Err(failure.into_inner().unwrap_or_else(|| e.into())),
    }
}

/// The text state a paragraph of several looks starts drawing from (see [`ShapedPlan::looked_line_ops`]): `font` at
/// `size`, filled as `fill` sets.
pub(crate) fn pen(font: &str, size: f64, fill: &[Op]) -> shaped_text::Pen {
    shaped_text::Pen { font: font.to_owned(), size, fill: fill.iter().map(to_text_op).collect() }
}

/// The look a run of a planned line is drawn in: its size in text space and the operators that set its fill colour.
pub(crate) struct RunLook {
    pub size: f64,
    pub fill: Vec<Op>,
}

impl ShapedPlan {
    /// The family of the face the glyphs come from.
    pub(crate) fn family(&self) -> &str {
        self.0.family()
    }

    /// The operators that draw wrapped line `i` at the current text position: its span, the Latin
    /// and shaped runs in drawing order. `current` is the font the text state holds, and holds the
    /// last one used afterwards.
    pub(crate) fn line_ops(&self, i: usize, size: f64, current: &mut String) -> Vec<Op> {
        self.0.line_ops(i, size, current).into_iter().map(to_op).collect()
    }

    /// How many spaces between words wrapped line `i` draws: the gaps [`Self::line_ops_stretched`] widens.
    pub(crate) fn gaps(&self, i: usize) -> usize {
        self.0.gaps(i)
    }

    /// [`Self::line_ops`] with each gap widened by `gap` (text space units before the horizontal scale, as word spacing is),
    /// the Latin text of the line shown by `show` word by word.
    pub(crate) fn line_ops_stretched(&self, i: usize, size: f64, current: &mut String, gap: f64, show: ShowLatin) -> Result<Vec<Op>, EditError> {
        let failure: std::cell::RefCell<Option<EditError>> = std::cell::RefCell::new(None);
        let ops = self.0.line_ops_stretched(i, size, current, gap, &adapted(show, &failure));
        match ops {
            Ok(ops) => Ok(ops.into_iter().map(to_op).collect()),
            Err(e) => Err(failure.into_inner().unwrap_or_else(|| e.into())),
        }
    }

    /// The operators that draw wrapped line `i` of a plan of several looks ([`plan_looked`]), each run in its look of
    /// `looks`, with each gap widened by `gap` (none when it is not positive) and the Latin text of look `l` then shown
    /// by `shows[l]` word by word. `pen` is the text state (font, size and fill), and holds the last ones afterwards.
    pub(crate) fn looked_line_ops(
        &self,
        i: usize,
        looks: &[RunLook],
        pen: &mut shaped_text::Pen,
        gap: f64,
        shows: &[ShowLatin],
    ) -> Result<Vec<Op>, EditError> {
        let fills: Vec<Vec<TextOp>> = looks.iter().map(|look| look.fill.iter().map(to_text_op).collect()).collect();
        let looks: Vec<shaped_text::RunLook> = looks.iter().zip(&fills).map(|(look, fill)| shaped_text::RunLook { size: look.size, fill }).collect();
        let failure: std::cell::RefCell<Option<EditError>> = std::cell::RefCell::new(None);
        let adapted: Vec<_> = shows.iter().map(|show| adapted(*show, &failure)).collect();
        let shows: Vec<shaped_text::LatinOps> = adapted.iter().map(|show| show as shaped_text::LatinOps).collect();
        let ops = self.0.looked_line_ops_stretched(i, &looks, pen, gap, &shows);
        drop(shows);
        drop(adapted);
        match ops {
            Ok(ops) => Ok(ops.into_iter().map(to_op).collect()),
            Err(e) => Err(failure.into_inner().unwrap_or_else(|| e.into())),
        }
    }
}
