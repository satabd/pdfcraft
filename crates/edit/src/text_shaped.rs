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
//! between the shaped runs, in the same span.
//!
//! The paragraph's text state keeps no character or word spacing here (spacing would pull joined
//! letters apart), and a justified right-to-left paragraph is right-aligned instead.

use std::cell::RefCell;
use std::collections::HashMap;

use pdfcraft_content::Op;
use pdfcraft_cos::{Dict, Document, Object, PdfString};
use pdfcraft_fonts::{GlyphError, ShapedCluster, ShapedScript};

use crate::EditError;
use crate::added::{
    ClusterKey, GLYPHS_PER_FONT, Piece, Shaper, arabic_error, cluster_key, is_arabic, is_bidi_control, mirrored, shaped_pieces, type3_shaped,
};

/// The `/Name` of the Type 3 fonts generated for Arabic text.
pub(crate) const ARABIC_FONT_NAME: &str = "PCArabic";

/// Most distinct shapes (a letter in a joining form with its marks) one rewrite may draw: each is
/// a glyph of a generated font, and a text that needs more is not one a person typed.
const MAX_CLUSTERS: usize = 4096;

/// Longest line measured exactly in one go while wrapping; longer candidates are measured word by
/// word, which keeps wrapping linear in the text's length.
const EXACT_MEASURE: usize = 400;

#[cfg(test)]
thread_local! {
    /// Tests replace the Arabic face with a stand-in on their own thread.
    pub(crate) static TEST_SHAPER: std::cell::Cell<Option<Shaper>> = const { std::cell::Cell::new(None) };
}

/// Where Arabic gets its shapes and outlines: the craft-fonts face.
pub(crate) fn shaper() -> Shaper {
    #[cfg(test)]
    if let Some(shaper) = TEST_SHAPER.with(std::cell::Cell::get) {
        return shaper;
    }
    Shaper::FACES
}

/// Whether `c` is an Arabic letter, mark, digit or sign the Arabic face draws (direction marks are
/// steering characters, not drawn).
pub(crate) fn is_arabic_script(c: char) -> bool {
    is_arabic(c) && !is_bidi_control(c)
}

/// Whether `text` has Arabic-script characters, which Edit text draws with the Arabic face.
pub(crate) fn has_arabic(text: &str) -> bool {
    text.chars().any(is_arabic_script)
}

/// `text` without what the Arabic face draws, and without the direction marks: what is left for
/// the page's font or Helvetica.
pub(crate) fn latin_of(text: &str) -> String {
    text.chars().filter(|c| !is_arabic_script(*c) && !is_bidi_control(*c)).collect()
}

/// `text` followed by `text` with its brackets mirrored: right-to-left runs draw mirrored brackets,
/// so a font that is to show the Latin pieces must have both.
pub(crate) fn with_mirrors(text: &str) -> String {
    let mut out = text.to_owned();
    out.extend(text.chars().map(mirrored));
    out
}

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
    EditError::Invalid("Arabic and Hebrew in one paragraph can't be written yet; edit them as separate paragraphs".into())
}

/// The characters of `latin` that neither the page's font nor Helvetica can show next to Arabic.
pub(crate) fn unsupported_with_arabic(latin: &str) -> EditError {
    EditError::Invalid(format!(
        "\"{latin}\" has characters that can't be drawn next to Arabic text: only Arabic, Latin letters, digits and punctuation the page font or Helvetica can show are supported"
    ))
}

/// A cluster's advance in em as the generated font's `/Widths` states it (whole thousandths).
fn width_em(c: &ShapedCluster) -> f64 {
    let em = (c.advance * 1000.0).round() / 1000.0;
    if em.is_finite() { em } else { 0.0 }
}

/// Measures lines of shaped text, in text space before the horizontal scale.
pub(crate) struct Measure<'a> {
    shaper: Shaper,
    rtl: bool,
    size: f64,
    /// The width of Latin text in the font that draws it.
    latin: &'a dyn Fn(&str) -> f64,
    cache: RefCell<HashMap<String, f64>>,
}

impl<'a> Measure<'a> {
    pub(crate) fn new(rtl: bool, size: f64, latin: &'a dyn Fn(&str) -> f64) -> Self {
        Measure { shaper: shaper(), rtl, size, latin, cache: RefCell::new(HashMap::new()) }
    }

    /// The width of `line` laid out as it will be drawn. A line that can't be shaped is measured as
    /// Latin (drawing then reports why).
    pub(crate) fn line(&self, line: &str) -> f64 {
        if let Ok(cache) = self.cache.try_borrow()
            && let Some(w) = cache.get(line)
        {
            return *w;
        }
        let w = match shaped_pieces(line, self.rtl, &self.shaper) {
            Ok(pieces) => pieces
                .iter()
                .map(|p| match p {
                    Piece::Latin(s) => (self.latin)(s),
                    Piece::Shaped(clusters) => clusters.iter().map(width_em).sum::<f64>() * self.size,
                })
                .sum(),
            Err(_) => (self.latin)(line),
        };
        let w = if w.is_finite() { w } else { 0.0 };
        if let Ok(mut cache) = self.cache.try_borrow_mut() {
            // Bounded: a hostile text has as many distinct candidates as it has words.
            if cache.len() >= 50_000 {
                cache.clear();
            }
            cache.insert(line.to_owned(), w);
        }
        w
    }

    /// The width of a candidate line while wrapping: exact for ordinary lines, the sum of its words
    /// for very long ones.
    pub(crate) fn approx(&self, candidate: &str) -> f64 {
        if candidate.len() <= EXACT_MEASURE {
            return self.line(candidate);
        }
        let words: f64 = candidate.split(' ').map(|w| self.line(w)).sum();
        words + candidate.matches(' ').count() as f64 * self.line(" ")
    }
}

/// How the Latin pieces of a line are drawn: in the resource `font`, with the operators `show`
/// gives (or an error when the font can't show a piece).
pub(crate) struct LatinShow<'a> {
    pub font: &'a str,
    pub show: &'a dyn Fn(&str) -> Result<Vec<Op>, EditError>,
}

enum Run {
    /// Operators that show Latin text in the Latin font.
    Latin(Vec<Op>),
    /// Codes of the generated font at this index of `names`.
    Shaped(usize, Vec<u8>),
}

struct PlannedLine {
    /// The line in typing order: its `/ActualText`.
    logical: String,
    runs: Vec<Run>,
}

/// Wrapped lines laid out, with the fonts they need already added to the document.
pub(crate) struct ShapedPlan {
    /// The generated fonts' resource names.
    names: Vec<String>,
    latin_font: String,
    lines: Vec<PlannedLine>,
    family: String,
}

enum Staged {
    Latin(Vec<Op>),
    /// Indexes into the distinct clusters.
    Shaped(Vec<usize>),
}

/// Lay out `lines` (in typing order, in a paragraph that runs `rtl` or not), read every outline
/// and add the fonts to `doc` and `fonts_res`. Everything that can fail does so before `doc` is
/// touched. `max_glyphs` is the most glyphs one generated font holds (tests lower it).
pub(crate) fn plan(
    doc: &mut Document,
    fonts_res: &mut Dict,
    lines: &[String],
    rtl: bool,
    max_glyphs: usize,
    latin: &LatinShow,
) -> Result<ShapedPlan, EditError> {
    let shaper = shaper();
    let mut keys: Vec<&ShapedCluster> = Vec::new();
    let mut index: HashMap<ClusterKey, usize> = HashMap::new();
    let mut laid = Vec::with_capacity(lines.len());
    for line in lines {
        laid.push(shaped_pieces(line, rtl, &shaper)?);
    }
    let mut staged: Vec<Vec<Staged>> = Vec::with_capacity(laid.len());
    for pieces in &laid {
        let mut row = Vec::with_capacity(pieces.len());
        for piece in pieces {
            match piece {
                Piece::Latin(s) => row.push(Staged::Latin((latin.show)(s)?)),
                Piece::Shaped(clusters) => {
                    let mut ks = Vec::with_capacity(clusters.len());
                    for c in clusters {
                        if c.script != ShapedScript::Arabic {
                            return Err(mixed_scripts_error());
                        }
                        let k = match index.get(&cluster_key(c)) {
                            Some(k) => *k,
                            None => {
                                if keys.len() >= MAX_CLUSTERS {
                                    return Err(EditError::Invalid(format!(
                                        "this Arabic text needs more than {MAX_CLUSTERS} different letter shapes; edit it in smaller parts"
                                    )));
                                }
                                keys.push(c);
                                index.insert(cluster_key(c), keys.len() - 1);
                                keys.len() - 1
                            }
                        };
                        ks.push(k);
                    }
                    row.push(Staged::Shaped(ks));
                }
            }
        }
        staged.push(row);
    }

    // Every outline is read before anything is written, so a failure leaves the document as it was.
    let mut outlines = HashMap::new();
    for c in &keys {
        for (id, _) in &c.glyphs {
            if let std::collections::hash_map::Entry::Vacant(entry) = outlines.entry((c.script, *id)) {
                entry.insert((shaper.outline)(c.script, *id).map_err(|e| arabic_error(e, &c.text))?);
            }
        }
    }

    let per_font = max_glyphs.clamp(1, GLYPHS_PER_FONT);
    let mut names = Vec::new();
    for chunk in keys.chunks(per_font) {
        let font = type3_shaped(doc, chunk, &outlines, Some(ARABIC_FONT_NAME));
        // A name nothing on the page has: a full save renumbers objects but keeps resource names,
        // and Add text's `PCAr` fonts are matched by their prefix.
        let mut name = format!("PCEdAr{}", font.num);
        let mut suffix = 0usize;
        while fonts_res.contains(name.as_bytes()) {
            suffix = suffix.saturating_add(1);
            name = format!("PCEdAr{}_{suffix}", font.num);
        }
        fonts_res.set(name.clone().into_bytes(), Object::Ref(font));
        names.push(name);
    }

    let mut planned = Vec::with_capacity(lines.len());
    for (line, row) in lines.iter().zip(staged) {
        let mut runs: Vec<Run> = Vec::new();
        for item in row {
            match item {
                Staged::Latin(ops) => runs.push(Run::Latin(ops)),
                Staged::Shaped(ks) => {
                    for k in ks {
                        let font = k / per_font;
                        // Codes start at 1; `k % per_font + 1` is at most 240.
                        let code = u8::try_from(k % per_font + 1).unwrap_or(1);
                        match runs.last_mut() {
                            Some(Run::Shaped(f, codes)) if *f == font => codes.push(code),
                            _ => runs.push(Run::Shaped(font, vec![code])),
                        }
                    }
                }
            }
        }
        planned.push(PlannedLine { logical: line.clone(), runs });
    }
    let family = ShapedScript::Arabic.font().map_or("Arabic", |f| f.family).to_owned();
    Ok(ShapedPlan { names, latin_font: latin.font.to_owned(), lines: planned, family })
}

impl ShapedPlan {
    /// The family of the face the glyphs come from.
    pub(crate) fn family(&self) -> &str {
        &self.family
    }

    /// The operators that draw wrapped line `i` at the current text position: its span, the Latin
    /// and shaped runs in drawing order. `current` is the font the text state holds, and holds the
    /// last one used afterwards.
    pub(crate) fn line_ops(&self, i: usize, size: f64, current: &mut String) -> Vec<Op> {
        let Some(line) = self.lines.get(i).filter(|l| !l.runs.is_empty()) else { return Vec::new() };
        let mut bytes = vec![0xFE, 0xFF];
        for unit in line.logical.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        let mut properties = Dict::new();
        properties.set(b"ActualText".to_vec(), Object::String(PdfString { bytes, hex: true }));
        let mut ops = vec![Op::new("BDC", vec![Object::name("Span"), Object::Dict(properties)])];
        let mut select = |ops: &mut Vec<Op>, font: &str| {
            if current != font {
                ops.push(Op::new("Tf", vec![Object::name(font), pdfcraft_content::num(size)]));
                *current = font.to_owned();
            }
        };
        for run in &line.runs {
            match run {
                Run::Latin(shown) => {
                    select(&mut ops, &self.latin_font);
                    ops.extend(shown.iter().cloned());
                }
                Run::Shaped(font, codes) => {
                    let Some(name) = self.names.get(*font) else { continue };
                    select(&mut ops, name);
                    ops.push(Op::new("Tj", vec![Object::String(PdfString::literal(codes.clone()))]));
                }
            }
        }
        ops.push(Op::new("EMC", vec![]));
        ops
    }
}
