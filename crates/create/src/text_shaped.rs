//! Arabic in text made into a PDF: laid out shaped, right-to-left paragraphs right-aligned.
//!
//! Helvetica has no Arabic, so a paragraph with Arabic in it is set like Add text sets it
//! (`pdfcraft_fonts::shaped_text`): the Unicode bidirectional order (UAX #9) with the paragraph's
//! direction taken from its first strong character, the craft-fonts Arabic face's joined forms,
//! glyphs drawn from Type 3 fonts built from the face's outlines, one marked-content span per
//! line whose `/ActualText` holds the line in typing order (so copy and search read it as it was
//! typed). Wrapping measures the shaped widths; a right-to-left paragraph is right-aligned to the
//! text width. Latin text, digits and punctuation between the Arabic stay in Helvetica (`/F1`).
//!
//! Paragraphs without Arabic are laid out and written exactly as before. Without the face nothing
//! here applies: callers refuse the text, or escape it ([`crate::from_text_escaped`]).

use pdfcraft_cos::{Dict, Document, Object, PdfString};
use pdfcraft_fonts::shaped_text::{self, LatinShow, Measure, PlanFonts, ShapedPlan, ShapedTextError, TextOp};
use pdfcraft_fonts::{ShapedScript, first_non_win_ansi, helvetica_width, win_ansi, wrap, wrap_with};

use crate::CreateError;

/// The resource name of the Helvetica the page text is set in.
pub(crate) const BODY_FONT: &str = "F1";

/// The start of the resource names of the generated Type 3 fonts.
const FONT_PREFIX: &str = "PCTxAr";

/// Most bytes of one paragraph shaped as one piece: a longer one (a file with no line breaks) is
/// cut, at a character boundary, so the layout of any one piece stays small.
const MAX_PARAGRAPH: usize = 50_000;

/// Most bytes of shaped lines planned together. The pages of a longer text are planned in groups
/// that each get their own fonts, which keeps the memory a text needs bounded.
const CHUNK_BYTES: usize = 100_000;

/// Whether this build can shape Arabic at all.
pub(crate) fn face_available() -> bool {
    (shaped_text::current_shaper().arabic_available)()
}

/// Whether `c` is Arabic the face draws, or a direction mark that steers its order.
pub(crate) fn drawn_with_arabic(c: char) -> bool {
    let shaper = shaped_text::current_shaper();
    shaped_text::is_bidi_control(c) || (shaped_text::is_arabic_script(c) && (shaper.arabic_available)() && (shaper.has)(ShapedScript::Arabic, c))
}

/// The first character of `line` that can't be drawn: in a line with Arabic, an Arabic letter the
/// face lacks (any, without the face) or other text Helvetica can't show next to it; otherwise
/// what Helvetica can't show.
pub(crate) fn first_undrawable(line: &str) -> Option<char> {
    if !shaped_text::has_arabic(line) {
        return pdfcraft_fonts::first_undrawable(line);
    }
    line.chars()
        .find(|c| shaped_text::is_arabic_script(*c) && !drawn_with_arabic(*c))
        .or_else(|| pdfcraft_fonts::first_undrawable(&shaped_text::latin_of(line)))
}

/// One laid-out line.
pub(crate) struct Line {
    pub text: String,
    /// Set for a line of a paragraph with Arabic: whether the paragraph runs right to left.
    pub shaped: Option<bool>,
    /// Its width as drawn (shaped lines only).
    pub width: f64,
    /// Its index in the plan of the group of pages it is on (set when planned).
    plan: usize,
}

impl Line {
    fn plain(text: String) -> Line {
        Line { text, shaped: None, width: 0.0, plan: 0 }
    }
}

/// What the layout of a text is made of: lines, and form feeds that start a page.
pub(crate) enum Item {
    Line(Line),
    FormFeed,
}

/// The text cut into wrapped lines at `size` within `width`, a form feed after each part between
/// form feeds (the last one included). With `shaping`, paragraphs with Arabic are laid out shaped.
pub(crate) fn layout(text: &str, size: f64, width: f64, shaping: bool) -> Vec<Item> {
    let mut items = Vec::new();
    let normalized = text.replace("\r\n", "\n").replace('\t', "    ");
    for part in normalized.split('\u{c}') {
        for para in part.split(['\n', '\r']) {
            if shaping && shaped_text::has_arabic(para) {
                shaped_paragraph(para, size, width, &mut items);
            } else {
                items.extend(wrap(para, size, width).into_iter().map(|l| Item::Line(Line::plain(l))));
            }
        }
        items.push(Item::FormFeed);
    }
    items
}

fn shaped_paragraph(para: &str, size: f64, width: f64, items: &mut Vec<Item>) {
    let rtl = shaped_text::first_strong_is_rtl(para);
    let latin = |s: &str| helvetica_width(s, size);
    let meter = Measure::new(rtl, size, &latin);
    let mut rest = para;
    while !rest.is_empty() {
        // Cut an enormous paragraph at a character boundary.
        let mut cut = rest.len().min(MAX_PARAGRAPH);
        while !rest.is_char_boundary(cut) {
            cut = cut.saturating_sub(1);
        }
        let (piece, tail) = rest.split_at(cut);
        rest = tail;
        for line in wrap_with(piece, size, width, |s, _| meter.approx(s)) {
            let w = if line.is_empty() { 0.0 } else { meter.line(&line) };
            items.push(Item::Line(Line { text: line, shaped: Some(rtl), width: w, plan: 0 }));
        }
    }
}

/// A group of pages' shaped lines laid out, with the fonts they need already added to the document.
pub(crate) struct Planned {
    plan: ShapedPlan,
    /// The generated fonts: resource name and font.
    fonts: Dict,
}

fn show_latin(s: &str) -> Result<Vec<TextOp>, ShapedTextError> {
    match first_non_win_ansi(s) {
        Some(c) => Err(ShapedTextError(pdfcraft_fonts::undrawable_message("the text", c))),
        None => Ok(vec![TextOp::new("Tj", vec![Object::String(PdfString::literal(win_ansi(s)))])]),
    }
}

/// Plan the shaped lines of `pages` together, adding the fonts to `doc`; `None` when none is shaped.
pub(crate) fn plan(doc: &mut Document, pages: &mut [Vec<Line>]) -> Result<Option<Planned>, CreateError> {
    let mut directed: Vec<(String, bool)> = Vec::new();
    for line in pages.iter_mut().flat_map(|p| p.iter_mut()) {
        if let (Some(rtl), false) = (line.shaped, line.text.is_empty()) {
            line.plan = directed.len();
            directed.push((line.text.clone(), rtl));
        }
    }
    if directed.is_empty() {
        return Ok(None);
    }
    let mut fonts = Dict::new();
    let names = PlanFonts { max_glyphs: shaped_text::GLYPHS_PER_FONT, name_prefix: FONT_PREFIX, font_name: None };
    let latin = LatinShow { font: BODY_FONT, show: &show_latin };
    let plan = shaped_text::plan_directed(doc, &mut fonts, &directed, names, &latin).map_err(|e| CreateError::Invalid(e.0))?;
    Ok(Some(Planned { plan, fonts }))
}

impl Planned {
    /// Add the generated fonts to a page's font resources.
    pub(crate) fn add_fonts(&self, fonts: &mut Dict) {
        for (name, font) in self.fonts.iter() {
            fonts.set(name.clone(), font.clone());
        }
    }
}

/// Group pages so that each group's shaped text is about [`CHUNK_BYTES`]: the ranges of `pages`.
pub(crate) fn groups(pages: &[Vec<Line>]) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let (mut start, mut bytes) = (0usize, 0usize);
    for (i, page) in pages.iter().enumerate() {
        let here: usize = page.iter().filter(|l| l.shaped.is_some()).map(|l| l.text.len()).sum();
        if bytes > 0 && bytes.saturating_add(here) > CHUNK_BYTES {
            out.push(start..i);
            start = i;
            bytes = 0;
        }
        bytes = bytes.saturating_add(here);
    }
    out.push(start..pages.len());
    out
}

/// The content of a page that has shaped lines: every line placed with its own text matrix
/// (right-to-left paragraphs end at the right margin). `top` is the first baseline.
pub(crate) fn page_content(lines: &[Line], planned: Option<&Planned>, size: f64, line_h: f64, margin: f64, width: f64, top: f64) -> Vec<u8> {
    let mut out = format!("BT /{BODY_FONT} {size} Tf\n").into_bytes();
    let mut current = BODY_FONT.to_owned();
    for (i, line) in lines.iter().enumerate() {
        let y = top - i as f64 * line_h;
        let shaped_ops = planned.filter(|_| line.shaped.is_some() && !line.text.is_empty()).map(|p| p.plan.line_ops(line.plan, size, &mut current));
        match shaped_ops {
            Some(ops) if !ops.is_empty() => {
                let x = if line.shaped == Some(true) { margin + (width - line.width).max(0.0) } else { margin };
                out.extend(format!("1 0 0 1 {x:.3} {y:.3} Tm\n").bytes());
                write_ops(&ops, &mut out);
            }
            _ => {
                if current != BODY_FONT {
                    out.extend(format!("/{BODY_FONT} {size} Tf\n").bytes());
                    current = BODY_FONT.to_owned();
                }
                out.extend(format!("1 0 0 1 {margin:.3} {y:.3} Tm\n").bytes());
                out.extend(pdfcraft_fonts::literal(&win_ansi(&line.text)));
                out.extend_from_slice(b" Tj\n");
            }
        }
    }
    out.extend_from_slice(b"ET\n");
    out
}

fn write_ops(ops: &[TextOp], out: &mut Vec<u8>) {
    for op in ops {
        for operand in &op.operands {
            pdfcraft_cos::serialize(operand, out);
            out.push(b' ');
        }
        out.extend_from_slice(&op.operator);
        out.push(b'\n');
    }
}
