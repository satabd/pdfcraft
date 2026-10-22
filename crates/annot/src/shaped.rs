//! Shaped Arabic in FreeText annotations (text boxes, callouts, Typewriter, Fill & Sign text).
//!
//! [`crate::appearance::build`] is pure: it has the annotation dictionary and nothing else, and
//! the same code draws display-only copies on a shared `&Document`. It can therefore add no font to
//! the document, so shaped glyphs are drawn as filled paths inside the appearance stream, one
//! `/Span <</ActualText …>> BDC … EMC` per line holding the line in typing order (copy and search
//! read that, not the glyphs). Latin letters, digits and punctuation next to the Arabic stay
//! `/Helv` text runs. The layout is `pdfcraft_fonts::shaped_text`'s, shared with Add text, Edit text
//! and form fields: Unicode bidirectional order (UAX #9), joined forms, marks, wrapping on shaped
//! widths.
//!
//! Text this can't draw (no Arabic face in the build, a character neither font has, more than
//! [`MAX_SHAPED_CHARS`] characters) is not shaped here: the caller draws it as before, with `?`.
//! [`drawable`] answers for [`crate::appearance::undrawable_text`], so a shaped annotation is not
//! marked and a `?` one still is.

use std::collections::HashSet;

use pdfcraft_fonts::shaped_text::{self, Measure, Piece, ShapedTextError, Shaper};
use pdfcraft_fonts::{ShapedScript, first_undrawable, helvetica_width, literal, win_ansi, wrap_with};

use crate::n;

/// Most characters of one annotation's text drawn shaped: every glyph becomes a path of up to a few
/// thousand points in the stream, so a longer text (not one a person typed into a box) is stored
/// as typed and drawn with `?` and the mark, as without the face.
pub(crate) const MAX_SHAPED_CHARS: usize = 1500;

/// Whether `text` is to be drawn shaped: it has Arabic script, is of a size a person types, and
/// the build has the Arabic face.
pub(crate) fn wanted(text: &str) -> bool {
    shaped_text::has_arabic(text)
        && text.chars().take(MAX_SHAPED_CHARS + 1).count() <= MAX_SHAPED_CHARS
        && (shaped_text::current_shaper().arabic_available)()
}

/// Whether `text` is an Arabic paragraph: it has Arabic script and its first strong character runs
/// right to left. Such text with no `/Q` is right-aligned.
pub(crate) fn rtl_text(text: &str) -> bool {
    shaped_text::has_arabic(text) && shaped_text::first_strong_is_rtl(text)
}

/// One wrapped line, its pieces in drawing order and its width in points.
struct Line {
    /// The line in typing order.
    text: String,
    /// Whether its paragraph runs right to left.
    rtl: bool,
    width: f64,
    pieces: Vec<Piece>,
}

/// `text` wrapped to `width` points at `size`, a paragraph at a time, each laid out in its own
/// direction. Fails when a piece can't be drawn: shaping fails, the script is not Arabic, or the
/// Latin font has no glyph for a character.
fn lay_out(text: &str, size: f64, width: f64) -> Result<Vec<Line>, ShapedTextError> {
    let shaper = shaped_text::current_shaper();
    let latin = |s: &str| helvetica_width(s, 1.0);
    let measures = [Measure::new(false, 1.0, &latin), Measure::new(true, 1.0, &latin)];
    let mut lines = Vec::new();
    for paragraph in text.split(['\n', '\r']) {
        let rtl = shaped_text::first_strong_is_rtl(paragraph);
        let Some(measure) = measures.get(usize::from(rtl)) else { continue };
        for line in wrap_with(paragraph, size, width, |s, sz| measure.approx(s) * sz) {
            let pieces = shaped_text::shaped_pieces(&line, rtl, &shaper)?;
            for piece in &pieces {
                match piece {
                    Piece::Latin(s) => {
                        if let Some(c) = first_undrawable(s) {
                            return Err(ShapedTextError(format!("\"{c}\" can't be drawn next to Arabic text in a text box")));
                        }
                    }
                    Piece::Shaped(clusters) => {
                        if clusters.iter().any(|c| c.script != ShapedScript::Arabic) {
                            return Err(shaped_text::mixed_scripts_error());
                        }
                    }
                }
            }
            lines.push(Line { width: measure.line(&line) * size, text: line, rtl, pieces });
        }
    }
    Ok(lines)
}

/// Whether [`body`] can draw `text`, whatever the box: it is wanted, every line shapes, the Latin
/// font has what is left and the face has an outline for every glyph. What
/// [`crate::appearance::undrawable_text`] asks, so the two agree on which text is `?`.
pub(crate) fn drawable(text: &str) -> bool {
    if !wanted(text) {
        return false;
    }
    let Ok(lines) = lay_out(text, 1.0, 1e9) else { return false };
    let shaper = shaped_text::current_shaper();
    let mut seen = HashSet::new();
    lines.iter().flat_map(|l| &l.pieces).all(|piece| match piece {
        Piece::Latin(_) => true,
        Piece::Shaped(clusters) => clusters
            .iter()
            .flat_map(|c| c.glyphs.iter().map(move |(id, _)| (c.script, *id)))
            .all(|key| !seen.insert(key) || (shaper.outline)(key.0, key.1).is_ok()),
    })
}

/// A space in invisible text (render mode 3), stretched over `width` points from `x`: paths are not
/// text, so without a glyph inside the span a viewer or PdfCraft's own text extraction has nothing
/// to attach the span's `/ActualText` to, and the line couldn't be copied or found. Nothing is
/// drawn. A line with no width (a lone mark) gets none.
fn invisible_run(buf: &mut Vec<u8>, width: f64, x: f64, y: f64, size: f64) {
    // A Helvetica space is 0.278 em wide; `Tz` scales it to the line.
    let scale = width / (0.278 * size) * 100.0;
    if scale.is_finite() && scale >= 0.01 {
        buf.extend(format!("q 3 Tr BT /Helv {} Tf {} Tz 1 0 0 1 {} {} Tm ( ) Tj ET Q\n", n(size), n(scale.min(1e7)), n(x), n(y)).bytes());
    }
}

/// Draw one line with its baseline at `y`, starting at `x`: Latin pieces as `/Helv` runs, shaped
/// pieces as filled paths, the whole line (if it has shaped text) in a span that holds its text.
fn draw_line(out: &mut Vec<u8>, line: &Line, x: f64, y: f64, size: f64, shaper: &Shaper) -> Result<(), ShapedTextError> {
    let shaped = line.pieces.iter().any(|p| matches!(p, Piece::Shaped(_)));
    let mut buf = Vec::new();
    if shaped {
        buf.extend(shaped_text::actual_text_bdc(&line.text));
        invisible_run(&mut buf, line.width, x, y, size);
    }
    let mut pen = x;
    for piece in &line.pieces {
        match piece {
            Piece::Latin(s) => {
                if s.is_empty() {
                    continue;
                }
                buf.extend(format!("BT /Helv {} Tf 1 0 0 1 {} {} Tm ", n(size), n(pen), n(y)).bytes());
                buf.extend(literal(&win_ansi(s)));
                buf.extend_from_slice(b" Tj ET\n");
                pen += helvetica_width(s, size);
            }
            Piece::Shaped(clusters) => pen = shaped_text::cluster_paths(&mut buf, clusters, shaper, pen, y, size)?,
        }
    }
    if shaped {
        buf.extend_from_slice(b"EMC\n");
    }
    out.extend(buf);
    Ok(())
}

/// The operators that draw `text` shaped in the box `rect` (padded by `pad`, `width` wide inside):
/// from the top, lines `1.2 × size` apart, and right-aligned when a paragraph runs right to left
/// and the annotation states no alignment (`q` is its `/Q`). The fill colour is the caller's.
/// `None` when the text is not drawn shaped (see [`wanted`], [`drawable`]), so the caller draws it
/// the old way.
pub(crate) fn body(text: &str, size: f64, rect: [f64; 4], pad: f64, width: f64, q: Option<i64>) -> Option<Vec<u8>> {
    if !wanted(text) || !size.is_finite() || size <= 0.0 {
        return None;
    }
    let lines = lay_out(text, size, width).ok()?;
    let shaper = shaped_text::current_shaper();
    let mut out = Vec::new();
    let mut y = rect[3] - pad - size * 0.9;
    for line in &lines {
        if y < rect[1] - size {
            break;
        }
        let align = match q {
            Some(q) => q,
            None if line.rtl => 2,
            None => 0,
        };
        let x = match align {
            1 => rect[0] + pad + (width - line.width) / 2.0,
            2 => rect[2] - pad - line.width,
            _ => rect[0] + pad,
        };
        draw_line(&mut out, line, x, y, size, &shaper).ok()?;
        y -= size * 1.2;
    }
    Some(out)
}

/// The width in points of the widest paragraph of `text` at `size`, as [`body`] draws it
/// (shaped), or `None` when the text is not drawn shaped.
pub(crate) fn longest_line(text: &str, size: f64) -> Option<f64> {
    if !wanted(text) || !size.is_finite() || size <= 0.0 {
        return None;
    }
    let lines = lay_out(text, size, f64::MAX).ok()?;
    Some(lines.iter().map(|l| l.width).fold(0.0, f64::max))
}

/// How many lines `text` wraps into within `width` points at `size`, as [`body`] draws it
/// (shaped), or `None` when the text is not drawn shaped.
pub(crate) fn line_count(text: &str, size: f64, width: f64) -> Option<usize> {
    if !wanted(text) || !size.is_finite() || size <= 0.0 {
        return None;
    }
    Some(lay_out(text, size, width).ok()?.len())
}
