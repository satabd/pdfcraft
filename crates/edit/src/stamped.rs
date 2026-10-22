//! Shaped Arabic in page marks: header & footer, Bates numbers and watermarks.
//!
//! These marks are drawn in Helvetica (`/PCHelv`, WinAnsi), which has no Arabic. A line with Arabic
//! in it is laid out like Add text lays it out: the Unicode bidirectional order (UAX #9), the
//! craft-fonts Arabic face's joined forms, glyphs drawn from Type 3 fonts built from that face's
//! outlines (`pdfcraft_fonts::shaped_text`), the line in one marked-content span whose
//! `/ActualText` holds it in typing order. Latin text, digits and the tokens' expansions stay in
//! `/PCHelv` between the shaped runs, in the same span. A line without Arabic is drawn exactly as
//! before.
//!
//! One run (a header on many pages, a watermark) plans every distinct Arabic line **once**, before
//! the document is touched, so a failure leaves the document as it was and the pages share one set
//! of generated fonts (`/PCStAr<n>` in each page's `/Resources /Font`). The width of a line, for
//! the left, centre and right slots and the watermark's centring, is measured as it is drawn.
//!
//! Without the face the text is refused as it always was ([`first_undrawable`] names the first
//! Arabic letter).

use std::collections::{HashMap, HashSet};

use pdfcraft_content::{Op, serialize_ops};
use pdfcraft_cos::{Dict, Document, Object, PdfString};
use pdfcraft_fonts::shaped_text::{self, LatinShow, Measure, PlanFonts, ShapedPlan, ShapedTextError, TextOp};
use pdfcraft_fonts::{ShapedScript, first_non_win_ansi, helvetica_width, win_ansi};

use crate::{EditError, n, undrawable_message};

/// The start of the resource names of the fonts generated for marks: a name nothing on the page
/// has (a full save renumbers objects but keeps resource names), and not Add text's `PCAr` or
/// Edit text's `PCEdAr`, which those tools match by prefix.
pub(crate) const FONT_PREFIX: &str = "PCStAr";

/// The resource name of the Helvetica that draws everything else.
const HELV: &str = "PCHelv";

/// Most bytes of Arabic-bearing lines planned in one run: every character becomes a shaped cluster
/// in memory while the run is laid out.
const MAX_TEXT: usize = 1_000_000;

/// Longest single line shaped: a line of a header is a few words, and a watermark line a word or
/// two; more is not something a person typed into the dialog.
const MAX_LINE: usize = 20_000;

/// The first character of `text` that marks can't draw: with Arabic in it, the first Arabic letter
/// the Arabic face lacks (or any, without the face) and the first other character Helvetica can't
/// show; otherwise the first character Helvetica can't show.
pub(crate) fn first_undrawable(text: &str) -> Option<char> {
    if !shaped_text::has_arabic(text) {
        return first_non_win_ansi(text);
    }
    let shaper = shaped_text::current_shaper();
    let face = (shaper.arabic_available)();
    text.chars()
        .find(|c| shaped_text::is_arabic_script(*c) && !(face && (shaper.has)(ShapedScript::Arabic, *c)))
        .or_else(|| first_non_win_ansi(&shaped_text::latin_of(text)))
}

/// The width of `line` at `size` as it is drawn: Helvetica's, or for a line with Arabic the shaped
/// layout's.
pub(crate) fn line_width(line: &str, size: f64) -> f64 {
    if !shaped_text::has_arabic(line) {
        return helvetica_width(line, size);
    }
    let latin = |s: &str| helvetica_width(s, size);
    Measure::new(shaped_text::first_strong_is_rtl(line), size, &latin).line(line)
}

/// The Type 3 fonts and planned lines of one run of marks.
pub(crate) struct Shaped {
    plan: ShapedPlan,
    /// Planned line index of every line with Arabic.
    lines: HashMap<String, usize>,
    /// The generated fonts: resource name and font.
    fonts: Vec<(Vec<u8>, Object)>,
}

/// A Latin piece of a line in `/PCHelv`: its `Tj`, or the reason Helvetica can't show it.
fn show_latin(s: &str) -> Result<Vec<TextOp>, ShapedTextError> {
    match first_non_win_ansi(s) {
        Some(c) => Err(ShapedTextError(undrawable_message(c))),
        None => Ok(vec![TextOp::new("Tj", vec![Object::String(PdfString::literal(win_ansi(s)))])]),
    }
}

impl Shaped {
    /// Plan the lines of a run over `pages` (0-based page indexes). `None` when no line has
    /// Arabic. Everything that can fail does so before `doc` is touched.
    pub(crate) fn prepare(
        doc: &mut Document,
        pages: &[usize],
        lines: impl IntoIterator<Item = Result<String, EditError>>,
    ) -> Result<Option<Shaped>, EditError> {
        let mut index: HashMap<String, usize> = HashMap::new();
        let mut directed: Vec<(String, bool)> = Vec::new();
        let mut total = 0usize;
        for line in lines {
            let line = line?;
            if !shaped_text::has_arabic(&line) || index.contains_key(&line) {
                continue;
            }
            total = total.saturating_add(line.len());
            if line.len() > MAX_LINE || total > MAX_TEXT {
                return Err(EditError::Invalid("the Arabic text is too long to shape in one go".into()));
            }
            index.insert(line.clone(), directed.len());
            let rtl = shaped_text::first_strong_is_rtl(&line);
            directed.push((line, rtl));
        }
        if directed.is_empty() {
            return Ok(None);
        }
        // The names the pages' fonts already have, so a generated font never replaces one.
        let all = crate::page_list(doc);
        let mut scratch = Dict::new();
        for page in pages.iter().filter_map(|i| all.get(*i)) {
            let Some(res) = page.dict.get(b"Resources").map(|r| doc.resolve(r)) else { continue };
            let Some(fonts) = res.as_dict().and_then(|r| r.get(b"Font")).map(|f| doc.resolve(f)) else { continue };
            if let Some(fonts) = fonts.as_dict() {
                for (name, _) in fonts.iter() {
                    scratch.set(name.clone(), Object::Null);
                }
            }
        }
        let taken: HashSet<Vec<u8>> = scratch.iter().map(|(name, _)| name.clone()).collect();
        let latin = LatinShow { font: HELV, show: &show_latin };
        let names = PlanFonts { max_glyphs: shaped_text::GLYPHS_PER_FONT, name_prefix: FONT_PREFIX, font_name: None };
        let plan = shaped_text::plan_directed(doc, &mut scratch, &directed, names, &latin)?;
        let fonts = scratch.iter().filter(|(name, _)| !taken.contains(*name)).map(|(name, font)| (name.clone(), font.clone())).collect();
        Ok(Some(Shaped { plan, lines: index, fonts }))
    }

    /// Make the generated fonts available to `page` (after `add_resources` has given it its own).
    pub(crate) fn add_fonts(&self, doc: &mut Document, page: &pdfcraft_model::Page) -> Result<(), EditError> {
        let mut res = page.dict.get(b"Resources").map(|r| doc.resolve(r)).and_then(|r| r.as_dict().cloned()).unwrap_or_default();
        let mut fonts = res.get(b"Font").map(|f| doc.resolve(f)).and_then(|f| f.as_dict().cloned()).unwrap_or_default();
        for (name, font) in &self.fonts {
            fonts.set(name.clone(), font.clone());
        }
        res.set(b"Font".to_vec(), Object::Dict(fonts));
        doc.update_dict(page.obj, |d| d.set(b"Resources".to_vec(), Object::Dict(res)))?;
        Ok(())
    }

    /// The operators of `line`'s span, if it was planned. `current` is the font the text state
    /// holds, and holds the last one used afterwards.
    fn ops(&self, line: &str, size: f64, current: &mut String) -> Option<Vec<Op>> {
        let i = *self.lines.get(line)?;
        let ops = self.plan.line_ops(i, size, current);
        Some(ops.into_iter().map(|op| Op { op: op.operator, operands: op.operands, inline: None, span: 0..0 }).collect())
    }
}

/// Draw `line` with its text origin at `(x, y)` (inside `BT … ET` whose font is `current`): shaped
/// when the run planned it, in Helvetica otherwise (selecting it first if a generated font was the
/// last one used).
pub(crate) fn draw(out: &mut Vec<u8>, shaped: Option<&Shaped>, x: f64, y: f64, line: &str, size: f64, current: &mut String) {
    if let Some(ops) = shaped.and_then(|s| s.ops(line, size, current)) {
        out.extend(format!("1 0 0 1 {} {} Tm ", n(x), n(y)).bytes());
        out.extend(serialize_ops(&ops));
        out.push(b'\n');
        return;
    }
    if current != HELV {
        out.extend(format!("/{HELV} {} Tf\n", n(size)).bytes());
        *current = HELV.to_owned();
    }
    out.extend(crate::text_op(x, y, line));
}

/// The names of the generated fonts a mark's content shows text with.
pub(crate) fn font_names(content: &[u8]) -> Vec<Vec<u8>> {
    content
        .split(u8::is_ascii_whitespace)
        .filter_map(|w| w.strip_prefix(b"/"))
        .filter(|w| w.starts_with(FONT_PREFIX.as_bytes()))
        .map(<[u8]>::to_vec)
        .collect()
}

/// Remove `names` from the `/Font` resources of `page` (the fonts of marks that were removed).
pub(crate) fn drop_fonts(doc: &mut Document, page: &pdfcraft_model::Page, names: &HashSet<Vec<u8>>) -> Result<(), EditError> {
    if names.is_empty() {
        return Ok(());
    }
    let Some(mut res) = page.dict.get(b"Resources").map(|r| doc.resolve(r)).and_then(|r| r.as_dict().cloned()) else { return Ok(()) };
    let Some(fonts) = res.get(b"Font").map(|f| doc.resolve(f)).and_then(|f| f.as_dict().cloned()) else { return Ok(()) };
    let mut kept = Dict::new();
    let mut dropped = false;
    for (name, font) in fonts.iter() {
        if names.contains(name) {
            dropped = true;
        } else {
            kept.set(name.clone(), font.clone());
        }
    }
    if !dropped {
        return Ok(());
    }
    res.set(b"Font".to_vec(), Object::Dict(kept));
    doc.update_dict(page.obj, |d| d.set(b"Resources".to_vec(), Object::Dict(res)))?;
    Ok(())
}
