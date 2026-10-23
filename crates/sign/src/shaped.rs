//! Shaped Arabic in the visible signature: the signer's name, the reason, the location, the
//! contact and the labels.
//!
//! The appearance is built with the document in hand, so Arabic is drawn from generated Type 3
//! fonts (`pdfcraft_fonts::shaped_text`, shared with Add text, form fields and stamped text),
//! listed in the appearance's own `/Resources /Font` beside `/Helv`, which still draws the Latin
//! letters, digits and punctuation. A line is laid out in the Unicode bidirectional order (UAX #9),
//! with joined forms and marks, wrapped on its shaped width, and drawn inside one
//! `/Span <</ActualText …>> BDC … EMC` that holds it in typing order, so viewers copy and search
//! the text rather than the glyphs. A paragraph that runs right to left (its first strong
//! character is an Arabic letter) is right-aligned in its column.
//!
//! Text without Arabic is not touched: it is measured and drawn in Helvetica exactly as before,
//! which matters because the bytes of the appearance are signed. Nothing here is random: the fonts'
//! names and glyph order follow the text.
//!
//! Without the Arabic face in the build, or with a character neither font has, the text is
//! refused, as it always was ([`first_undrawable`] names the character).

use pdfcraft_cos::{Dict, Document, Object, PdfString};
use pdfcraft_fonts::shaped_text::{self, LatinShow, Measure, PlanFonts, ShapedPlan, ShapedTextError, TextOp};
use pdfcraft_fonts::{ShapedScript, first_non_win_ansi, helvetica_width, undrawable_message, win_ansi, wrap_with};

use crate::SignError;

/// The start of the generated fonts' resource names, followed by the font's object number: a name
/// `/Helv` never has.
const FONT_PREFIX: &str = "PCSgAr";

/// The resource name of the Helvetica that draws everything else.
const LATIN_FONT: &str = "Helv";

/// Most characters of one field drawn shaped: the name, a reason or a location is a few words, and
/// laying out more is slow for no reader's benefit.
pub(crate) const MAX_CHARS: usize = 2000;

/// The first character of `text` that the visible signature can't draw. With Arabic in it: the
/// first Arabic letter the face lacks (any, without the face) and the first other character
/// Helvetica can't show. Otherwise the first character Helvetica can't show.
pub(crate) fn first_undrawable(text: &str) -> Option<char> {
    if !shaped_text::has_arabic(text) {
        return pdfcraft_fonts::first_undrawable(text);
    }
    let shaper = shaped_text::current_shaper();
    let face = (shaper.arabic_available)();
    text.chars()
        .find(|c| shaped_text::is_arabic_script(*c) && !(face && (shaper.has)(ShapedScript::Arabic, *c)))
        .or_else(|| pdfcraft_fonts::first_undrawable(&shaped_text::latin_of(text)))
}

/// Whether this build can draw Arabic at all.
pub(crate) fn face_available() -> bool {
    (shaped_text::current_shaper().arabic_available)()
}

/// Why `text` is not drawn: it has more Arabic than a field of a signature holds.
pub(crate) fn too_long(text: &str) -> Option<SignError> {
    (shaped_text::has_arabic(text) && text.chars().take(MAX_CHARS + 1).count() > MAX_CHARS).then(|| {
        SignError::Unsupported(format!(
            "Arabic text in the visible signature can have at most {MAX_CHARS} characters; shorten it, or sign without a visible appearance"
        ))
    })
}

/// Why shaped text can't be drawn, as the error of signing.
pub(crate) fn refused(e: ShapedTextError) -> SignError {
    SignError::Unsupported(format!("{}; sign without a visible appearance, or leave that text out of it", e.0))
}

/// One wrapped line.
#[derive(Clone, Debug)]
pub(crate) struct Wrapped {
    pub text: String,
    /// Whether its paragraph runs right to left.
    pub rtl: bool,
    /// Whether it is drawn from the generated fonts (its paragraph has Arabic).
    pub shaped: bool,
}

/// Measures and wraps lines of a visible signature, shaped where they have Arabic.
pub(crate) struct Setter<'a> {
    /// Lines of a paragraph that runs left to right, at one point.
    ltr: Measure<'a>,
    /// Lines of a paragraph that runs right to left, at one point.
    rtl: Measure<'a>,
}

impl<'a> Setter<'a> {
    /// `latin` is the width of Helvetica text at one point.
    pub(crate) fn new(latin: &'a dyn Fn(&str) -> f64) -> Self {
        Setter { ltr: Measure::new(false, 1.0, latin), rtl: Measure::new(true, 1.0, latin) }
    }

    fn measure(&self, rtl: bool) -> &Measure<'a> {
        if rtl { &self.rtl } else { &self.ltr }
    }

    /// The width in ems of `text` as one line, drawn as [`Self::wrap`] would draw it.
    pub(crate) fn em(&self, text: &str) -> f64 {
        if shaped_text::has_arabic(text) { self.measure(shaped_text::first_strong_is_rtl(text)).line(text) } else { helvetica_width(text, 1.0) }
    }

    /// The width in ems of one wrapped line.
    pub(crate) fn em_of(&self, line: &Wrapped) -> f64 {
        if line.shaped { self.measure(line.rtl).line(&line.text) } else { helvetica_width(&line.text, 1.0) }
    }

    /// `text` wrapped to `width` points at `size`, a paragraph at a time, each in its own direction.
    /// Paragraphs without Arabic are wrapped in Helvetica as ever.
    pub(crate) fn wrap(&self, text: &str, size: f64, width: f64) -> Vec<Wrapped> {
        let mut out = Vec::new();
        for paragraph in text.split(['\n', '\r']) {
            if !shaped_text::has_arabic(paragraph) {
                out.extend(wrap_with(paragraph, size, width, helvetica_width).into_iter().map(|text| Wrapped { text, rtl: false, shaped: false }));
                continue;
            }
            let rtl = shaped_text::first_strong_is_rtl(paragraph);
            let measure = self.measure(rtl);
            out.extend(wrap_with(paragraph, size, width, |s, sz| measure.approx(s) * sz).into_iter().map(|text| Wrapped { text, rtl, shaped: true }));
        }
        out
    }
}

/// How a Latin piece next to Arabic is shown: its bytes in `/Helv`, or the character Helvetica
/// can't hold.
fn show_latin(s: &str) -> Result<Vec<TextOp>, ShapedTextError> {
    if let Some(c) = first_non_win_ansi(s) {
        return Err(ShapedTextError(undrawable_message("the visible signature", c)));
    }
    Ok(vec![TextOp::new("Tj", vec![Object::String(PdfString::literal(win_ansi(s)))])])
}

/// Lay out `lines` (each in typing order with its paragraph's direction), read every outline and
/// add the generated fonts to `doc` and `fonts`. Everything that can fail does so before `doc` is
/// touched.
pub(crate) fn plan(doc: &mut Document, fonts: &mut Dict, lines: &[(String, bool)]) -> Result<ShapedPlan, SignError> {
    let latin = LatinShow { font: LATIN_FONT, show: &show_latin };
    let names = PlanFonts { max_glyphs: shaped_text::GLYPHS_PER_FONT, name_prefix: FONT_PREFIX, font_name: None };
    shaped_text::plan_directed(doc, fonts, lines, names, &latin).map_err(refused)
}

/// The content-stream bytes that draw planned line `i` at the current text position in `size`,
/// ending in a line break. The font is selected again in every line, whatever the last one used.
pub(crate) fn line_bytes(plan: &ShapedPlan, i: usize, size: f64) -> Vec<u8> {
    let mut current = String::new();
    let mut out = Vec::new();
    for op in plan.line_ops(i, size, &mut current) {
        for operand in &op.operands {
            pdfcraft_cos::serialize(operand, &mut out);
            out.push(b' ');
        }
        out.extend_from_slice(&op.operator);
        out.push(b' ');
    }
    out.push(b'\n');
    out
}
