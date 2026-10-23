//! Shaped text for PDF content: Arabic, Hebrew and Thai laid out line by line and drawn from
//! Type 3 fonts built from the craft-fonts faces' outlines (no font file is embedded).
//!
//! Shared by every writer that draws such text with its own hands: Add text and Edit text
//! (`pdfcraft-edit`), generated form-field appearances (`pdfcraft-forms`) and anything later that
//! needs a line of text a viewer's own fonts can't draw. The pieces are:
//!
//! - [`shaped_pieces`]: one line in the Unicode bidirectional order (UAX #9), cut into Latin
//!   pieces (drawn by the caller's font) and shaped clusters (joined forms, marks, ligatures);
//! - [`type3_shaped`]: one Type 3 glyph per distinct cluster, with a `ToUnicode` map holding the
//!   cluster's text, so copy and search read each character once;
//! - [`plan`] / [`ShapedPlan::line_ops`]: the lines of a text laid out, every outline read before
//!   the document is touched, the fonts added in chunks of [`GLYPHS_PER_FONT`] glyphs, and for
//!   each line the content-stream operators of a `/Span <</ActualText …>> BDC … EMC` that holds
//!   the line in typing order;
//! - [`Measure`]: the width of a line as it will be drawn, for wrapping and alignment;
//! - lines drawn in several looks (sizes, fill colours, Latin fonts): [`shaped_pieces_looked`], [`Measure::with_looks`],
//!   [`plan_looked`] and [`ShapedPlan::looked_line_ops`], each piece and run in one look, the text state kept in a [`Pen`],
//!   and a span for each look holding its characters in typing order where the looks are drawn one after another and
//!   [`reading_order`] (the order page text extraction reads a line's spans in) gives the line back as typed;
//! - bold Arabic: each look has a [`Weight`] ([`shaped_pieces_weighted`], [`Measure::weighted`], [`plan_looked`]), and
//!   a bold look's Arabic is shaped, measured and drawn at the Arabic face's bold instance (its `wght` axis at 700:
//!   [`ShapedScript::ArabicBold`]), in Type 3 fonts of its own. A face without a `wght` axis draws it regular.
//!
//! Operators are returned as [`TextOp`] (an operator and its operands) because this crate sits
//! below the content-stream crate; callers turn them into their own operator type.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use pdfcraft_cos::{Dict, Document, ObjRef, Object, PdfString, Stream};
use unicode_bidi::{Level, ParagraphBidiInfo};

use crate::{GlyphError, GlyphOutline, ShapedCluster, ShapedScript, Weight, shape_arabic_at, shape_hebrew, shape_thai, shaped_glyph};

/// Most glyphs in one Type 3 font: its codes are single bytes, 1–240. A text gets one such font
/// per 240 distinct glyphs it needs.
pub const GLYPHS_PER_FONT: usize = 240;

/// Most distinct shapes (a letter in a joining form with its marks) one [`plan`] may draw: each is
/// a glyph of a generated font, and a text that needs more is not one a person typed.
pub const MAX_CLUSTERS: usize = 4096;

/// Longest line measured exactly in one go while wrapping; longer candidates are measured word by
/// word, which keeps wrapping linear in the text's length.
const EXACT_MEASURE: usize = 400;

/// Why text can't be shaped: the message says what was wrong and what is needed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShapedTextError(pub String);

impl std::fmt::Display for ShapedTextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ShapedTextError {}

/// One content-stream operator with its operands (the writer's own type lives in a higher crate).
#[derive(Clone, Debug, PartialEq)]
pub struct TextOp {
    pub operator: Vec<u8>,
    pub operands: Vec<Object>,
}

impl TextOp {
    pub fn new(operator: &str, operands: Vec<Object>) -> Self {
        TextOp { operator: operator.as_bytes().to_vec(), operands }
    }
}

// ---- character classes ----

fn is_thai(c: char) -> bool {
    matches!(u32::from(c), 0x0E00..=0x0E7F)
}

/// Arabic letters, marks, digits and punctuation (and the presentation forms): text with any of
/// them is shaped with the craft-fonts Arabic face.
pub fn is_arabic(c: char) -> bool {
    matches!(u32::from(c), 0x0600..=0x06FF | 0x0750..=0x077F | 0x0870..=0x08FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFC)
}

/// Hebrew letters, points, cantillation marks and punctuation (and the presentation forms).
pub fn is_hebrew(c: char) -> bool {
    matches!(u32::from(c), 0x0591..=0x05F4 | 0xFB1D..=0xFB4F)
}

/// Direction marks, embeddings, overrides and isolates: they steer the order and are not drawn.
pub fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Whether `c` is an Arabic letter, mark, digit or sign the Arabic face draws (direction marks are
/// steering characters, not drawn).
pub fn is_arabic_script(c: char) -> bool {
    is_arabic(c) && !is_bidi_control(c)
}

/// Whether `text` has Arabic-script characters, which the Arabic face draws.
pub fn has_arabic(text: &str) -> bool {
    text.chars().any(is_arabic_script)
}

/// The bracket drawn for `c` in a right-to-left run (UAX #9 L4).
pub fn mirrored(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        other => other,
    }
}

/// `text` without what the Arabic face draws, and without the direction marks: what is left for
/// the caller's Latin font.
pub fn latin_of(text: &str) -> String {
    text.chars().filter(|c| !is_arabic_script(*c) && !is_bidi_control(*c)).collect()
}

/// `text` followed by `text` with its brackets mirrored: right-to-left runs draw mirrored brackets,
/// so a font that is to show the Latin pieces must have both.
pub fn with_mirrors(text: &str) -> String {
    let mut out = text.to_owned();
    out.extend(text.chars().map(mirrored));
    out
}

/// Whether the paragraph `text` runs right to left: its first strong character (UAX #9 P2/P3) is
/// a right-to-left letter. Text with no strong character runs left to right.
pub fn first_strong_is_rtl(text: &str) -> bool {
    unicode_bidi::get_base_direction(text) == unicode_bidi::Direction::Rtl
}

// ---- the faces ----

/// A piece of a line with shaped text, in drawing order.
#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
    /// Shown in the caller's own (Latin) font.
    Latin(String),
    Shaped(Vec<ShapedCluster>),
}

/// Where shaped text gets its shapes and outlines: the craft-fonts faces, or (in tests) a stand-in
/// that needs no face. Plain function pointers, so a swap is one value and costs nothing.
#[derive(Clone, Copy)]
pub struct Shaper {
    /// One directional run of a script, clusters in drawing order. [`ShapedScript::ArabicBold`] asks for bold Arabic,
    /// whose clusters say which instance drew them (the faces give [`ShapedScript::Arabic`] ones when there is no bold).
    pub shape: fn(ShapedScript, &str, bool) -> Result<Vec<ShapedCluster>, GlyphError>,
    /// Whether the script's face has a glyph for the character.
    pub has: fn(ShapedScript, char) -> bool,
    /// The outline of a glyph of the script's face.
    pub outline: fn(ShapedScript, u32) -> Result<GlyphOutline, GlyphError>,
    /// Whether the Arabic face exists at all.
    pub arabic_available: fn() -> bool,
}

fn face_shape(script: ShapedScript, text: &str, rtl: bool) -> Result<Vec<ShapedCluster>, GlyphError> {
    match script {
        ShapedScript::Arabic | ShapedScript::ArabicBold => shape_arabic_at(text, rtl, script.weight()),
        ShapedScript::Hebrew => shape_hebrew(text, rtl),
        ShapedScript::Thai => shape_thai(text),
    }
}

fn face_has(script: ShapedScript, c: char) -> bool {
    match script {
        ShapedScript::Arabic | ShapedScript::ArabicBold => crate::arabic_has(c),
        ShapedScript::Hebrew => crate::hebrew_has(c),
        ShapedScript::Thai => crate::thai_has(c),
    }
}

fn arabic_face_available() -> bool {
    crate::document_arabic_font().is_some()
}

impl Shaper {
    /// The craft-fonts faces.
    pub const FACES: Shaper = Shaper { shape: face_shape, has: face_has, outline: shaped_glyph, arabic_available: arabic_face_available };
}

#[cfg(any(test, feature = "test-shaper"))]
thread_local! {
    static TEST_SHAPER: std::cell::Cell<Option<Shaper>> = const { std::cell::Cell::new(None) };
}

/// Tests: use `shaper` (a stand-in for the faces) on this thread, or the real faces again with
/// `None`. Only built for tests (`test-shaper` feature, enabled by the dev-dependencies that use it).
#[cfg(any(test, feature = "test-shaper"))]
pub fn set_test_shaper(shaper: Option<Shaper>) {
    TEST_SHAPER.with(|s| s.set(shaper));
}

/// Where shaped text is drawn from now: the craft-fonts faces (a test may have replaced them on
/// its thread).
pub fn current_shaper() -> Shaper {
    #[cfg(any(test, feature = "test-shaper"))]
    if let Some(shaper) = TEST_SHAPER.with(std::cell::Cell::get) {
        return shaper;
    }
    Shaper::FACES
}

// ---- errors ----

/// Why the Arabic face can't show `text`.
pub fn arabic_error(e: GlyphError, text: &str) -> ShapedTextError {
    ShapedTextError(match e {
        GlyphError::NoFont => "Arabic text needs PdfCraft's Arabic font, which this build doesn't include \
                               (to build it in, set CRAFT_FONTS_DIR to a craft-fonts checkout that has an Arab face)"
            .into(),
        GlyphError::Missing => format!("the Arabic font can't show \"{text}\""),
        GlyphError::TooComplex => format!("the Arabic font's glyphs for \"{text}\" are too complex"),
    })
}

fn hebrew_error(e: GlyphError, text: &str) -> ShapedTextError {
    ShapedTextError(match e {
        GlyphError::NoFont => "Hebrew text needs PdfCraft's Hebrew font, which this build doesn't include \
                               (to build it in, set CRAFT_FONTS_DIR to a craft-fonts checkout that has a Hebr face)"
            .into(),
        GlyphError::Missing => format!("the Hebrew font can't show \"{text}\""),
        GlyphError::TooComplex => format!("the Hebrew font's glyphs for \"{text}\" are too complex"),
    })
}

fn thai_error(e: GlyphError, text: &str) -> ShapedTextError {
    ShapedTextError(match e {
        GlyphError::NoFont => {
            "Thai text needs a Thai font in this build; rebuild with CRAFT_FONTS_DIR pointing to craft-fonts with a Thai face".into()
        }
        GlyphError::Missing => format!("the Thai font can't show \"{text}\""),
        GlyphError::TooComplex => "the Thai glyphs are too complex; use a shorter text box".into(),
    })
}

/// Why the face of `script` can't show `text`.
pub fn script_error(script: ShapedScript, e: GlyphError, text: &str) -> ShapedTextError {
    match script {
        ShapedScript::Arabic | ShapedScript::ArabicBold => arabic_error(e, text),
        ShapedScript::Hebrew => hebrew_error(e, text),
        ShapedScript::Thai => thai_error(e, text),
    }
}

// ---- layout ----

/// The pieces of `line` in drawing order: Latin text for the caller's own font, and shaped clusters
/// for Arabic, Hebrew and Thai. `rtl` is the paragraph's direction. Widths are the caller's to
/// measure, so each writer uses its own fonts for the Latin.
pub fn shaped_pieces(line: &str, rtl: bool, shaper: &Shaper) -> Result<Vec<Piece>, ShapedTextError> {
    Ok(shaped_pieces_looked(line, &[], rtl, shaper)?.into_iter().map(|(_, piece)| piece).collect())
}

/// [`shaped_pieces`] for a line drawn in several looks (sizes, colours, fonts): `looks` holds the look of each
/// character of `line` (a character past its end has look 0), and a piece ends where the look changes, so each
/// piece is drawn in one look, given with it. A look that changes inside a word of a script that joins cuts the
/// word in two shaped runs; a caller keeps the looks of such a word one (Edit text does).
pub fn shaped_pieces_looked(line: &str, looks: &[usize], rtl: bool, shaper: &Shaper) -> Result<Vec<(usize, Piece)>, ShapedTextError> {
    shaped_pieces_weighted(line, looks, &[], rtl, shaper)
}

/// The weight of look `look` among `weights` (a look past the end has the first's, and no weight at all is regular).
fn weight_of(weights: &[Weight], look: usize) -> Weight {
    weights.get(look).or_else(|| weights.first()).copied().unwrap_or_default()
}

/// [`shaped_pieces_looked`] with each look at a weight: the Arabic of look `i` is shaped at `weights[i]` (a look past
/// the end at the first's; none at all, regular), so a bold look's clusters are [`ShapedScript::ArabicBold`]'s when the
/// face has a bold instance. Hebrew and Thai have one weight.
pub fn shaped_pieces_weighted(
    line: &str,
    looks: &[usize],
    weights: &[Weight],
    rtl: bool,
    shaper: &Shaper,
) -> Result<Vec<(usize, Piece)>, ShapedTextError> {
    Ok(placed_pieces(line, looks, weights, rtl, shaper)?.into_iter().map(|(look, _, piece)| (look, piece)).collect())
}

/// [`shaped_pieces_weighted`] with where each piece starts in `line`: the byte offset of its first character in typing
/// order.
fn placed_pieces(line: &str, looks: &[usize], weights: &[Weight], rtl: bool, shaper: &Shaper) -> Result<Vec<(usize, usize, Piece)>, ShapedTextError> {
    // unicode-bidi indexes the first level of a line, so an empty one would panic.
    if line.is_empty() {
        return Ok(Vec::new());
    }
    // Where each character starts, with its look, to find a run's characters by their bytes (none: one look).
    let starts: Vec<(usize, usize)> = if looks.is_empty() {
        Vec::new()
    } else {
        line.char_indices().map(|(at, _)| at).zip(looks.iter().copied().chain(std::iter::repeat(0))).collect()
    };
    let base = if rtl { Level::rtl() } else { Level::ltr() };
    let info = ParagraphBidiInfo::new(line, Some(base));
    let (levels, runs) = info.visual_runs(0..line.len());
    let mut pieces = Vec::new();
    for run in runs {
        let run_rtl = levels.get(run.start).is_some_and(Level::is_rtl);
        let mut next = starts.partition_point(|(at, _)| *at < run.start);
        let run_start = run.start;
        // Runs lie on character boundaries; a missing slice is skipped, never a panic.
        let Some(text) = line.get(run) else { continue };
        // Arabic and Hebrew letters take their face; so do spaces and punctuation between them
        // (Hebrew's when they follow Hebrew), which the shaper then mirrors. Latin text (and what
        // the faces lack) keeps the caller's font.
        let mut segments: Vec<(Option<ShapedScript>, usize, String, usize)> = Vec::new();
        for (offset, c) in text.char_indices() {
            let look = starts.get(next).map_or(0, |(_, look)| *look);
            next = next.saturating_add(1);
            if is_bidi_control(c) {
                continue;
            }
            let after_hebrew = segments.last().is_some_and(|(kind, ..)| *kind == Some(ShapedScript::Hebrew));
            let script = if is_thai(c) {
                Some(ShapedScript::Thai)
            } else if is_hebrew(c) || (run_rtl && after_hebrew && !c.is_alphanumeric() && (shaper.has)(ShapedScript::Hebrew, c)) {
                Some(ShapedScript::Hebrew)
            } else if is_arabic(c) || (run_rtl && !c.is_alphanumeric() && (shaper.has)(ShapedScript::Arabic, c)) {
                Some(ShapedScript::Arabic)
            } else {
                None
            };
            match segments.last_mut() {
                Some((kind, was, s, _)) if *kind == script && *was == look => s.push(c),
                _ => segments.push((script, look, c.to_string(), run_start.saturating_add(offset))),
            }
        }
        if run_rtl {
            segments.reverse();
        }
        for (script, look, s, start) in segments {
            pieces.push((
                look,
                start,
                if let Some(script) = script {
                    let script = script.weighted(weight_of(weights, look));
                    let shaped = (shaper.shape)(script, &s, run_rtl).map_err(|e| script_error(script, e, &s))?;
                    Piece::Shaped(shaped)
                } else if run_rtl {
                    Piece::Latin(s.chars().rev().map(mirrored).collect())
                } else {
                    Piece::Latin(s)
                },
            ));
        }
    }
    Ok(pieces)
}

/// A cluster's advance in em as the generated font's `/Widths` states it (whole thousandths).
fn width_em(c: &ShapedCluster) -> f64 {
    let em = (c.advance * 1000.0).round() / 1000.0;
    if em.is_finite() { em } else { 0.0 }
}

/// A cluster's advance in em, as [`Measure`] counts it and the generated fonts state it.
pub fn cluster_width(c: &ShapedCluster) -> f64 {
    width_em(c)
}

/// `line` as the cells of a comb field hold it: one piece per cell, in drawing order (left to
/// right) in the paragraph's direction. A shaped cluster (a joined letter with its marks, a
/// ligature) fills one cell, and so does each character of Latin text. In a paragraph that runs
/// right to left the first typed letter is the last cell. Joined forms are those of the whole
/// line, so a letter keeps the shape it has in the word. Widths are the caller's to measure.
pub fn comb_cells(line: &str, rtl: bool, shaper: &Shaper) -> Result<Vec<Piece>, ShapedTextError> {
    let mut cells = Vec::new();
    for piece in shaped_pieces(line, rtl, shaper)? {
        match piece {
            Piece::Latin(s) => cells.extend(s.chars().map(|c| Piece::Latin(c.to_string()))),
            Piece::Shaped(clusters) => cells.extend(clusters.into_iter().map(|c| Piece::Shaped(vec![c]))),
        }
    }
    Ok(cells)
}

/// The width of Latin text in the font that draws it (at the size it is drawn).
pub type LatinWidth<'a> = &'a dyn Fn(&str) -> f64;

/// The operators that show Latin text in a font, or why the font can't.
pub type LatinOps<'a> = &'a dyn Fn(&str) -> Result<Vec<TextOp>, ShapedTextError>;

/// A line measured, and its looks as runs of characters of one look (`(count, look)`; none for one look).
type MeasureKey = (String, Vec<(usize, usize)>);

/// Measures lines of shaped text, in text space before the horizontal scale.
pub struct Measure<'a> {
    shaper: Shaper,
    rtl: bool,
    /// Each look's size, and the width of Latin text in the font that draws it (at that size).
    looks: Vec<(f64, LatinWidth<'a>)>,
    /// Each look's weight (see [`shaped_pieces_weighted`]; none: regular).
    weights: Vec<Weight>,
    /// Widths by line and its looks.
    cache: RefCell<HashMap<MeasureKey, f64>>,
}

impl<'a> Measure<'a> {
    /// Lines of a paragraph that runs `rtl` or not, at `size`; `latin` measures what the caller's
    /// own font draws (at the same size).
    pub fn new(rtl: bool, size: f64, latin: LatinWidth<'a>) -> Self {
        Self::with_looks(rtl, vec![(size, latin)])
    }

    /// Lines drawn in several looks (see [`shaped_pieces_looked`]): look `i` is drawn at `looks[i].0`, its Latin text
    /// measured by `looks[i].1` (at that size). A look past the end is measured as look 0, and with no look at all a
    /// line measures nothing.
    pub fn with_looks(rtl: bool, looks: Vec<(f64, LatinWidth<'a>)>) -> Self {
        Measure { shaper: current_shaper(), rtl, looks, weights: Vec::new(), cache: RefCell::new(HashMap::new()) }
    }

    /// These measures with look `i`'s Arabic at `weights[i]` (a look past the end at the first's; see
    /// [`shaped_pieces_weighted`]): a bold look is measured at the bold advances. Regular without it.
    pub fn weighted(mut self, weights: Vec<Weight>) -> Self {
        self.weights = weights;
        if let Ok(mut cache) = self.cache.try_borrow_mut() {
            cache.clear();
        }
        self
    }

    fn look(&self, look: usize) -> Option<&(f64, LatinWidth<'a>)> {
        self.looks.get(look).or_else(|| self.looks.first())
    }

    /// The width of `line` laid out as it will be drawn. A line that can't be shaped is measured as
    /// Latin (drawing then reports why).
    pub fn line(&self, line: &str) -> f64 {
        self.line_looked(line, &[])
    }

    /// [`Self::line`] for a line drawn in several looks: `looks` holds the look of each character (see
    /// [`shaped_pieces_looked`]; empty for look 0 throughout).
    pub fn line_looked(&self, line: &str, looks: &[usize]) -> f64 {
        // The looks as runs, `(characters, look)`: the cache's key with the line.
        let mut runs: Vec<(usize, usize)> = Vec::new();
        for look in looks {
            match runs.last_mut() {
                Some((count, was)) if was == look => *count = count.saturating_add(1),
                _ => runs.push((1, *look)),
            }
        }
        let key: MeasureKey = (line.to_owned(), runs);
        if let Ok(cache) = self.cache.try_borrow()
            && let Some(w) = cache.get(&key)
        {
            return *w;
        }
        let latin = |look: usize, s: &str| self.look(look).map_or(0.0, |(_, latin)| latin(s));
        let w = match shaped_pieces_weighted(line, looks, &self.weights, self.rtl, &self.shaper) {
            Ok(pieces) => pieces
                .iter()
                .map(|(look, p)| match p {
                    Piece::Latin(s) => latin(*look, s),
                    Piece::Shaped(clusters) => clusters.iter().map(width_em).sum::<f64>() * self.look(*look).map_or(0.0, |(size, _)| *size),
                })
                .sum(),
            Err(_) if key.1.is_empty() => latin(0, line),
            // Each run of one look as Latin in its own font.
            Err(_) => {
                let mut chars = line.chars();
                key.1.iter().map(|(count, look)| latin(*look, &chars.by_ref().take(*count).collect::<String>())).sum()
            }
        };
        let w = if w.is_finite() { w } else { 0.0 };
        if let Ok(mut cache) = self.cache.try_borrow_mut() {
            // Bounded: a hostile text has as many distinct candidates as it has words.
            if cache.len() >= 50_000 {
                cache.clear();
            }
            cache.insert(key, w);
        }
        w
    }

    /// The width of a candidate line while wrapping: exact for ordinary lines, the sum of its words
    /// for very long ones.
    pub fn approx(&self, candidate: &str) -> f64 {
        self.approx_looked(candidate, &[])
    }

    /// [`Self::approx`] for a candidate drawn in several looks (see [`Self::line_looked`]): each word and each space in its own.
    pub fn approx_looked(&self, candidate: &str, looks: &[usize]) -> f64 {
        if candidate.len() <= EXACT_MEASURE {
            return self.line_looked(candidate, looks);
        }
        // The character each word starts at.
        let mut at = 0usize;
        let words: f64 = candidate
            .split(' ')
            .map(|word| {
                let count = word.chars().count();
                let w = self.line_looked(word, looks.get(at..at.saturating_add(count)).unwrap_or_default());
                at = at.saturating_add(count).saturating_add(1);
                w
            })
            .sum();
        let spaces: f64 = if looks.is_empty() {
            candidate.matches(' ').count() as f64 * self.line(" ")
        } else {
            candidate.chars().zip(looks).filter(|(c, _)| *c == ' ').map(|(_, look)| self.line_looked(" ", std::slice::from_ref(look))).sum()
        };
        words + spaces
    }
}

// ---- the fonts ----

/// What makes two clusters the same Type 3 glyph: text, glyphs, offsets and advance (bit for bit;
/// equal clusters come from the same font units).
pub type ClusterKey<'a> = (ShapedScript, &'a str, Vec<(u32, u64, u64)>, u64);

pub fn cluster_key(c: &ShapedCluster) -> ClusterKey<'_> {
    (c.script, c.text.as_str(), c.glyphs.iter().map(|(id, [x, y])| (*id, x.to_bits(), y.to_bits())).collect(), c.advance.to_bits())
}

/// Format a number for a content stream (up to three decimals).
fn n(v: f64) -> String {
    let s = format!("{:.3}", if v.abs() < 5e-4 { 0.0 } else { v });
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// A number operand, written as an integer when it is one.
fn number(v: f64) -> Object {
    if v.fract() == 0.0 && v.abs() < 1e15 { Object::Int(v as i64) } else { Object::Real((v * 10_000.0).round() / 10_000.0) }
}

/// A Type 3 font with `clusters` as codes 1…, each drawn from its script face's glyph `outlines`.
/// `name` is the font's `/Name` (Edit text marks its fonts so it recognises them again); Add text
/// leaves it out.
pub fn type3_shaped(
    doc: &mut Document,
    clusters: &[&ShapedCluster],
    outlines: &HashMap<(ShapedScript, u32), GlyphOutline>,
    name: Option<&str>,
) -> ObjRef {
    let mut charprocs = Dict::new();
    let mut differences = vec![Object::Int(1)];
    let mut widths = Vec::with_capacity(clusters.len());
    let mut bbox = [0.0f64; 4];
    let mut to_unicode = Vec::new();
    for (i, c) in clusters.iter().enumerate() {
        let code = i + 1;
        // Glyph space is 1000 units per em.
        let mut paths = Vec::new();
        let mut ink: Option<[f64; 4]> = None;
        for (id, [dx, dy]) in &c.glyphs {
            let Some(g) = outlines.get(&(c.script, *id)) else { continue };
            for contour in &g.contours {
                for (j, p) in contour.iter().enumerate() {
                    paths.extend(format!("{} {} {}\n", n((p[0] + dx) * 1000.0), n((p[1] + dy) * 1000.0), if j == 0 { "m" } else { "l" }).bytes());
                }
                paths.extend_from_slice(b"h\n");
                let b = [g.bbox[0] + dx, g.bbox[1] + dy, g.bbox[2] + dx, g.bbox[3] + dy];
                ink = Some(ink.map_or(b, |a| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]));
            }
        }
        // Rounded outwards so no ink is clipped.
        let [x0, y0, x1, y1] =
            ink.map_or([0.0; 4], |b| [(b[0] * 1000.0).floor(), (b[1] * 1000.0).floor(), (b[2] * 1000.0).ceil(), (b[3] * 1000.0).ceil()]);
        bbox = [bbox[0].min(x0), bbox[1].min(y0), bbox[2].max(x1), bbox[3].max(y1)];
        let mut proc = format!("{} 0 {} {} {} {} d1\n", n(c.advance * 1000.0), n(x0), n(y0), n(x1), n(y1)).into_bytes();
        if !paths.is_empty() {
            proc.extend(paths);
            proc.extend_from_slice(b"f\n");
        }
        let glyph = format!("g{code:02X}");
        charprocs.set(glyph.clone().into_bytes(), Object::Ref(doc.add(Object::Stream(Stream::flate(Dict::new(), &proc)))));
        differences.push(Object::name(&glyph));
        widths.push(Object::Real((c.advance * 1000.0).round()));
        if !c.text.is_empty() {
            let hex: String = c.text.encode_utf16().map(|u| format!("{u:04X}")).collect();
            to_unicode.push(format!("<{code:02X}> <{hex}>\n"));
        }
    }
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CMapType 2 def\n1 begincodespacerange\n<01> <FF>\nendcodespacerange\n",
    );
    // A CMap block holds at most 100 entries.
    for block in to_unicode.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n{}endbfchar\n", block.len(), block.concat()));
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    let cmap = doc.add(Object::Stream(Stream::flate(Dict::new(), cmap.as_bytes())));
    // The face's family when every glyph comes from one script; without the face, the script's
    // name (as Arabic items were always named).
    let single = clusters.first().map(|c| c.script).filter(|s| clusters.iter().all(|c| c.script == *s));
    let family = match single {
        Some(script) => script.font().map_or(
            match script {
                ShapedScript::Arabic | ShapedScript::ArabicBold => "Arabic",
                ShapedScript::Hebrew => "Hebrew",
                ShapedScript::Thai => "Thai",
            },
            |f| f.family,
        ),
        None => "PdfCraft Shaped Text",
    };
    // A font of bold glyphs says so (its name and weight), so a reader, a later Edit text among them, sees bold text.
    let bold = single.is_some_and(|s| s.weight() == Weight::Bold);
    let mut descriptor = Dict::new();
    descriptor.set(b"Type".to_vec(), Object::name("FontDescriptor"));
    let font_name = family.replace(' ', "");
    descriptor.set(b"FontName".to_vec(), Object::name(&if bold { format!("{font_name}-Bold") } else { font_name }));
    descriptor.set(b"FontFamily".to_vec(), Object::String(PdfString::literal(family.as_bytes().to_vec())));
    descriptor.set(b"Flags".to_vec(), Object::Int(4));
    descriptor.set(b"ItalicAngle".to_vec(), Object::Int(0));
    if bold {
        descriptor.set(b"FontWeight".to_vec(), Object::Int(700));
    }
    let mut encoding = Dict::new();
    encoding.set(b"Type".to_vec(), Object::name("Encoding"));
    encoding.set(b"Differences".to_vec(), Object::Array(differences));
    let mut font = Dict::new();
    font.set(b"Type".to_vec(), Object::name("Font"));
    font.set(b"Subtype".to_vec(), Object::name("Type3"));
    if let Some(name) = name {
        font.set(b"Name".to_vec(), Object::name(name));
    }
    font.set(b"FontDescriptor".to_vec(), Object::Ref(doc.add(Object::Dict(descriptor))));
    font.set(b"FontBBox".to_vec(), Object::Array(bbox.iter().map(|v| Object::Real(*v)).collect()));
    font.set(
        b"FontMatrix".to_vec(),
        Object::Array(vec![Object::Real(0.001), Object::Int(0), Object::Int(0), Object::Real(0.001), Object::Int(0), Object::Int(0)]),
    );
    font.set(b"FirstChar".to_vec(), Object::Int(1));
    font.set(b"LastChar".to_vec(), Object::Int(i64::try_from(clusters.len()).unwrap_or(i64::MAX)));
    font.set(b"Widths".to_vec(), Object::Array(widths));
    font.set(b"Encoding".to_vec(), Object::Dict(encoding));
    font.set(b"CharProcs".to_vec(), Object::Dict(charprocs));
    font.set(b"ToUnicode".to_vec(), Object::Ref(cmap));
    doc.add(Object::Dict(font))
}

// ---- outlines as paths ----

/// A coordinate for a content stream, to hundredths of a point.
fn coord(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() { "0".into() } else { s.into() }
}

/// The start of the marked-content span that holds one line of text in typing order:
/// `/Span <</ActualText <FEFF…>>> BDC` and a line break. End it with `EMC`. Viewers copy and search
/// that text rather than the glyphs drawn inside.
pub fn actual_text_bdc(logical: &str) -> Vec<u8> {
    let mut out = b"/Span <</ActualText <FEFF".to_vec();
    for unit in logical.encode_utf16() {
        out.extend(format!("{unit:04X}").bytes());
    }
    out.extend_from_slice(b">>> BDC\n");
    out
}

/// Append the path operators that fill `clusters` with their outlines to `out`, for drawing
/// shaped text where no font can be added (an appearance stream built from a dictionary alone):
/// the pen starts at (`x`, `y`) on the baseline, and one `f` ends each cluster. Returns where the
/// pen ends, advanced by each cluster's width as [`Measure`] counts it.
///
/// Every outline is read before `out` is touched, so a failure appends nothing. Coordinates are
/// in hundredths of a point; a glyph holds at most a few thousand points.
pub fn cluster_paths(out: &mut Vec<u8>, clusters: &[ShapedCluster], shaper: &Shaper, x: f64, y: f64, size: f64) -> Result<f64, ShapedTextError> {
    if !(x.is_finite() && y.is_finite() && size.is_finite()) {
        return Err(ShapedTextError("the text's position or size is not a number".into()));
    }
    let mut outlines: HashMap<(ShapedScript, u32), GlyphOutline> = HashMap::new();
    for c in clusters {
        for (id, _) in &c.glyphs {
            if let std::collections::hash_map::Entry::Vacant(entry) = outlines.entry((c.script, *id)) {
                entry.insert((shaper.outline)(c.script, *id).map_err(|e| script_error(c.script, e, &c.text))?);
            }
        }
    }
    let mut pen = x;
    let mut text = String::new();
    for c in clusters {
        let mut any = false;
        for (id, [dx, dy]) in &c.glyphs {
            let Some(g) = outlines.get(&(c.script, *id)) else { continue };
            for contour in &g.contours {
                for (j, p) in contour.iter().enumerate() {
                    let (px, py) = (pen + (p[0] + dx) * size, y + (p[1] + dy) * size);
                    if !(px.is_finite() && py.is_finite()) {
                        return Err(ShapedTextError("the text's position or size is out of range".into()));
                    }
                    text.push_str(&format!("{} {} {}\n", coord(px), coord(py), if j == 0 { "m" } else { "l" }));
                }
                text.push_str("h\n");
                any = true;
            }
        }
        if any {
            text.push_str("f\n");
        }
        pen += width_em(c) * size;
    }
    out.extend_from_slice(text.as_bytes());
    Ok(pen)
}

// ---- planning lines ----

/// How the Latin pieces of a line are drawn: in the resource `font`, with the operators `show`
/// gives (or an error when the font can't show a piece).
pub struct LatinShow<'a> {
    pub font: &'a str,
    pub show: LatinOps<'a>,
}

/// What the generated fonts are called.
#[derive(Clone, Copy, Debug)]
pub struct PlanFonts<'a> {
    /// Most glyphs one generated font holds (at most [`GLYPHS_PER_FONT`]; tests lower it).
    pub max_glyphs: usize,
    /// The start of the fonts' resource names, followed by the font's object number (and a suffix
    /// if that name is taken): a name nothing on the page has, since a full save renumbers objects
    /// but keeps resource names.
    pub name_prefix: &'a str,
    /// The `/Name` of the Type 3 fonts, if the writer wants to recognise them again.
    pub font_name: Option<&'a str>,
}

enum Run {
    /// Operators that show Latin text in the Latin font of a look, the look, and the text they show.
    Latin(usize, Vec<TextOp>, String),
    /// A look, codes of the generated font at this index of `names`, and for each code whether it draws a space between
    /// words (a gap [`ShapedPlan::line_ops_stretched`] widens).
    Shaped(usize, usize, Vec<u8>, Vec<bool>),
}

impl Run {
    fn look(&self) -> usize {
        match self {
            Run::Latin(look, ..) | Run::Shaped(look, ..) => *look,
        }
    }
}

struct PlannedLine {
    /// The line in typing order: its `/ActualText`.
    logical: String,
    /// The runs in drawing order, each with the part of the line it draws (see [`parts_of`]).
    runs: Vec<(usize, Run)>,
    /// The line's spans when it has more than one ([`split_spans`]): the runs each holds and its `/ActualText`. Empty:
    /// one span of every run, holding `logical`.
    spans: Vec<(std::ops::Range<usize>, String)>,
}

/// Wrapped lines laid out, with the fonts they need already added to the document.
pub struct ShapedPlan {
    /// The generated fonts' resource names.
    names: Vec<String>,
    /// The resource name of the font each look shows its Latin text in.
    latin_fonts: Vec<String>,
    lines: Vec<PlannedLine>,
    family: String,
}

enum Staged {
    /// A look, the part of the line, the operators that show the text and the text.
    Latin(usize, usize, Vec<TextOp>, String),
    /// A look, the part of the line, and indexes into the distinct clusters with whether each is a space.
    Shaped(usize, usize, Vec<(usize, bool)>),
}

/// The parts of `line` drawn in one look each, as byte ranges in typing order: the longest stretches of characters of one
/// look (`looks` holds each character's, a character past its end has look 0). A line of one look is one part.
fn parts_of(line: &str, looks: &[usize]) -> Vec<std::ops::Range<usize>> {
    let mut parts: Vec<(std::ops::Range<usize>, usize)> = Vec::new();
    for ((at, c), look) in line.char_indices().zip(looks.iter().copied().chain(std::iter::repeat(0))) {
        let end = at.saturating_add(c.len_utf8());
        match parts.last_mut() {
            Some((range, was)) if *was == look => range.end = end,
            _ => parts.push((at..end, look)),
        }
    }
    parts.into_iter().map(|(range, _)| range).collect()
}

/// The order a reader of the page puts the pieces of one line in, each piece drawn after the one before it along the
/// baseline (in visual order) with its text already in typing order (a span's `/ActualText`): indexes into `texts` in
/// reading order. The simplified bidirectional pass (UAX #9) that page text extraction runs over a line's glyphs, a
/// span counting as one glyph: the line runs right to left when more of its pieces are right-to-left text than
/// left-to-right; numbers keep their left-to-right order, and neutrals (spaces, punctuation) take the direction of their
/// neighbours, or the line's between opposite ones.
pub fn reading_order(texts: &[&str]) -> Vec<usize> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Bidi {
        L,
        R,
        Num,
        N,
    }
    let rtl_char = |c: char| matches!(u32::from(c), 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x1_0800..=0x1_0FFF | 0x1_E800..=0x1_EFFF);
    let class = |text: &str| {
        if text.chars().any(|c| rtl_char(c) && !c.is_numeric()) {
            Bidi::R
        } else if text.chars().any(char::is_numeric) {
            Bidi::Num
        } else if text.chars().any(char::is_alphabetic) {
            Bidi::L
        } else {
            Bidi::N
        }
    };
    let mut classes: Vec<Bidi> = texts.iter().map(|t| class(t)).collect();
    let count = |classes: &[Bidi], b: Bidi| classes.iter().filter(|c| **c == b).count();
    let para = if count(&classes, Bidi::R) > count(&classes, Bidi::L) { Bidi::R } else { Bidi::L };
    let text = |k: usize| texts.get(k).copied().unwrap_or("");
    let at = |classes: &[Bidi], k: usize| classes.get(k).copied().unwrap_or(Bidi::N);
    let n = classes.len();
    // W4: a single separator between two numbers belongs to them.
    for k in 1..n.saturating_sub(1) {
        let sep = matches!(text(k), "." | "," | ":" | "/" | "+" | "-" | "\u{066B}" | "\u{066C}");
        if sep
            && at(&classes, k - 1) == Bidi::Num
            && at(&classes, k + 1) == Bidi::Num
            && let Some(c) = classes.get_mut(k)
        {
            *c = Bidi::Num;
        }
    }
    // W5: terminators (percent, currency) next to a number belong to it.
    let term =
        |s: &str| !s.is_empty() && s.chars().all(|c| matches!(c, '%' | '‰' | '#' | '$' | '¢'..='¥' | '°' | '\u{066A}' | '\u{20A0}'..='\u{20CF}'));
    for k in 1..n {
        if at(&classes, k) == Bidi::N
            && at(&classes, k - 1) == Bidi::Num
            && term(text(k))
            && let Some(c) = classes.get_mut(k)
        {
            *c = Bidi::Num;
        }
    }
    for k in (0..n.saturating_sub(1)).rev() {
        if at(&classes, k) == Bidi::N
            && at(&classes, k + 1) == Bidi::Num
            && term(text(k))
            && let Some(c) = classes.get_mut(k)
        {
            *c = Bidi::Num;
        }
    }
    // W7: a number after left-to-right text (or at the start of a left-to-right line) is left-to-right.
    let mut strong = para;
    for c in &mut classes {
        match *c {
            Bidi::L | Bidi::R => strong = *c,
            Bidi::Num if strong == Bidi::L => *c = Bidi::L,
            _ => {}
        }
    }
    // N1/N2: neutrals between text of one direction take it (numbers count as right-to-left), else the line's.
    let dir = |b: Bidi| if b == Bidi::L { Bidi::L } else { Bidi::R };
    let mut k = 0;
    while k < n {
        if at(&classes, k) != Bidi::N {
            k += 1;
            continue;
        }
        let mut e = k;
        while e < n && at(&classes, e) == Bidi::N {
            e += 1;
        }
        let before = if k == 0 { para } else { dir(at(&classes, k - 1)) };
        let after = if e == n { para } else { dir(at(&classes, e)) };
        let resolved = if before == after { before } else { para };
        for c in classes.get_mut(k..e).unwrap_or_default() {
            *c = resolved;
        }
        k = e;
    }
    // Levels (I1/I2), then L2: every run at or above each level reversed, the highest first.
    let rtl_para = para == Bidi::R;
    let levels: Vec<u8> = classes
        .iter()
        .map(|c| match c {
            Bidi::R => 1,
            Bidi::Num => 2,
            _ if rtl_para => 2,
            _ => 0,
        })
        .collect();
    let level_of = |i: usize| levels.get(i).copied().unwrap_or(0);
    let mut order: Vec<usize> = (0..n).collect();
    for level in (1..=levels.iter().copied().max().unwrap_or(0)).rev() {
        let mut k = 0;
        while k < n {
            if order.get(k).is_none_or(|i| level_of(*i) < level) {
                k += 1;
                continue;
            }
            let mut e = k;
            while order.get(e).is_some_and(|i| level_of(*i) >= level) {
                e += 1;
            }
            if let Some(run) = order.get_mut(k..e) {
                run.reverse();
            }
            k = e;
        }
    }
    order
}

/// The spans of a planned line drawn in several looks: one for each part of the line (see [`parts_of`]), holding the runs
/// that draw it and the part's text, so that each look reads back as a run of its own. A line gets them only when the
/// runs of each part are drawn one after another and the parts, read in [`reading_order`], give the line as it was typed;
/// otherwise (and for a line of one part) none, and the line is one span.
fn split_spans(logical: &str, parts: &[std::ops::Range<usize>], runs: &[(usize, Run)]) -> Vec<(std::ops::Range<usize>, String)> {
    if parts.len() < 2 {
        return Vec::new();
    }
    // The runs of each part, one after another.
    let mut groups: Vec<(std::ops::Range<usize>, usize)> = Vec::new();
    for (i, (part, _)) in runs.iter().enumerate() {
        match groups.last_mut() {
            Some((range, was)) if *was == *part => range.end = i.saturating_add(1),
            _ => groups.push((i..i.saturating_add(1), *part)),
        }
    }
    if groups.len() < 2 {
        return Vec::new();
    }
    let mut seen = vec![false; parts.len()];
    let mut spans = Vec::with_capacity(groups.len());
    for (range, part) in groups {
        let Some(slot) = seen.get_mut(part).filter(|s| !**s) else { return Vec::new() };
        *slot = true;
        let Some(text) = parts.get(part).and_then(|r| logical.get(r.clone())) else { return Vec::new() };
        spans.push((range, text.to_owned()));
    }
    let texts: Vec<&str> = spans.iter().map(|(_, t)| t.as_str()).collect();
    let read: String = reading_order(&texts).into_iter().filter_map(|k| texts.get(k).copied()).collect();
    if read == logical { spans } else { Vec::new() }
}

/// The look a run of a planned line is drawn in (see [`ShapedPlan::looked_line_ops`]): its font size and the
/// operators that set its fill colour (none: the colour the text state has).
#[derive(Clone, Copy, Debug)]
pub struct RunLook<'a> {
    pub size: f64,
    pub fill: &'a [TextOp],
}

/// The text state while planned lines are drawn: the font and size the last `Tf` set, and the fill colour operators
/// last given (those the text state was given before the lines, to start with).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pen {
    pub font: String,
    pub size: f64,
    pub fill: Vec<TextOp>,
}

impl Pen {
    /// Brings the text state to `font` in `look`: the look's fill where it differs from the one given last, and a `Tf`
    /// where the font or the size does.
    fn select(&mut self, ops: &mut Vec<TextOp>, font: &str, look: &RunLook) {
        if self.fill.as_slice() != look.fill {
            ops.extend(look.fill.iter().cloned());
            self.fill = look.fill.to_vec();
        }
        if self.font != font || self.size.to_bits() != look.size.to_bits() {
            ops.push(TextOp::new("Tf", vec![Object::name(font), number(look.size)]));
            font.clone_into(&mut self.font);
            self.size = look.size;
        }
    }
}

/// Why a paragraph with more than one script can't be drawn (Arabic is the only script drawn here
/// next to Latin so far).
pub fn mixed_scripts_error() -> ShapedTextError {
    ShapedTextError("Arabic and Hebrew in one paragraph can't be written yet; edit them as separate paragraphs".into())
}

/// [`plan_directed`] for lines that all run `rtl` or not.
pub fn plan(
    doc: &mut Document,
    fonts_res: &mut Dict,
    lines: &[String],
    rtl: bool,
    fonts: PlanFonts,
    latin: &LatinShow,
) -> Result<ShapedPlan, ShapedTextError> {
    let directed: Vec<(String, bool)> = lines.iter().map(|l| (l.clone(), rtl)).collect();
    plan_directed(doc, fonts_res, &directed, fonts, latin)
}

/// Lay out `lines` (each in typing order, with its paragraph's direction), read every outline and
/// add the fonts to `doc` and `fonts_res`. Everything that can fail does so before `doc` is
/// touched.
pub fn plan_directed(
    doc: &mut Document,
    fonts_res: &mut Dict,
    lines: &[(String, bool)],
    fonts: PlanFonts,
    latin: &LatinShow,
) -> Result<ShapedPlan, ShapedTextError> {
    let shaper = current_shaper();
    let mut laid = Vec::with_capacity(lines.len());
    for (line, rtl) in lines {
        laid.push((line.clone(), shaped_pieces(line, *rtl, &shaper)?));
    }
    plan_pieces(doc, fonts_res, laid, fonts, latin)
}

/// [`plan`] for lines drawn in several looks: each line is its text in typing order with the look of each of its
/// characters (see [`shaped_pieces_looked`]), look `i` shows its Latin text with `latins[i]` and its Arabic is drawn at
/// `weights[i]` (a look past the end of either with the first's; no weight at all, regular). A bold look's Arabic is
/// drawn from Type 3 fonts of its own (the resource name's prefix and object number followed by `B`), built from the
/// face's bold outlines. Everything that can fail does so before `doc` is touched.
///
/// A line of several looks is drawn in a span for each look (each part of it in one look, see `parts_of`), holding the
/// look's characters in typing order, so that a reader takes each look as a run of its own. It is one span, as a line of
/// one look is, when the runs of a look are not drawn one after another or the spans read in [`reading_order`] would not
/// give the line as it was typed.
pub fn plan_looked(
    doc: &mut Document,
    fonts_res: &mut Dict,
    lines: &[(String, Vec<usize>)],
    rtl: bool,
    fonts: PlanFonts,
    latins: &[LatinShow],
    weights: &[Weight],
) -> Result<ShapedPlan, ShapedTextError> {
    let shaper = current_shaper();
    let mut laid = Vec::with_capacity(lines.len());
    for (line, looks) in lines {
        let parts = parts_of(line, looks);
        let pieces = placed_pieces(line, looks, weights, rtl, &shaper)?
            .into_iter()
            .map(|(look, start, piece)| (look, parts.partition_point(|r| r.end <= start), piece))
            .collect();
        laid.push((line.clone(), parts, pieces));
    }
    plan_looked_pieces(doc, fonts_res, laid, fonts, latins)
}

/// [`plan_directed`] for lines already laid out: each is its text in typing order and its pieces
/// in drawing order (from [`shaped_pieces`], or one cell at a time from [`comb_cells`]). Pieces
/// with a script other than Arabic are refused; everything that can fail does so before `doc` is
/// touched.
pub fn plan_pieces(
    doc: &mut Document,
    fonts_res: &mut Dict,
    laid: Vec<(String, Vec<Piece>)>,
    fonts: PlanFonts,
    latin: &LatinShow,
) -> Result<ShapedPlan, ShapedTextError> {
    let laid = laid.into_iter().map(|(line, pieces)| (line, Vec::new(), pieces.into_iter().map(|piece| (0, 0, piece)).collect())).collect();
    plan_looked_pieces(doc, fonts_res, laid, fonts, std::slice::from_ref(latin))
}

/// A line laid out: its text in typing order, its parts (see [`parts_of`]; none: one), and its pieces in drawing order,
/// each with its look and the part it draws.
type Laid = (String, Vec<std::ops::Range<usize>>, Vec<(usize, usize, Piece)>);

/// [`plan_pieces`] with each piece in a look of its own (see [`plan_looked`]).
fn plan_looked_pieces(
    doc: &mut Document,
    fonts_res: &mut Dict,
    laid: Vec<Laid>,
    fonts: PlanFonts,
    latins: &[LatinShow],
) -> Result<ShapedPlan, ShapedTextError> {
    let shaper = current_shaper();
    let mut keys: Vec<&ShapedCluster> = Vec::new();
    let mut index: HashMap<ClusterKey, usize> = HashMap::new();
    let mut staged: Vec<Vec<Staged>> = Vec::with_capacity(laid.len());
    for (_, _, pieces) in &laid {
        let mut row = Vec::with_capacity(pieces.len());
        for (look, part, piece) in pieces {
            match piece {
                Piece::Latin(s) => {
                    let latin =
                        latins.get(*look).or_else(|| latins.first()).ok_or_else(|| ShapedTextError("no font to show Latin text in".into()))?;
                    row.push(Staged::Latin(*look, *part, (latin.show)(s)?, s.clone()));
                }
                Piece::Shaped(clusters) => {
                    let mut ks = Vec::with_capacity(clusters.len());
                    for c in clusters {
                        if !c.script.is_arabic() {
                            return Err(mixed_scripts_error());
                        }
                        let k = match index.get(&cluster_key(c)) {
                            Some(k) => *k,
                            None => {
                                if keys.len() >= MAX_CLUSTERS {
                                    return Err(ShapedTextError(format!(
                                        "this Arabic text needs more than {MAX_CLUSTERS} different letter shapes; split it into smaller parts"
                                    )));
                                }
                                keys.push(c);
                                let k = keys.len().saturating_sub(1);
                                index.insert(cluster_key(c), k);
                                k
                            }
                        };
                        ks.push((k, c.text == " "));
                    }
                    row.push(Staged::Shaped(*look, *part, ks));
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
                entry.insert((shaper.outline)(c.script, *id).map_err(|e| script_error(c.script, e, &c.text))?);
            }
        }
    }

    let per_font = fonts.max_glyphs.clamp(1, GLYPHS_PER_FONT);
    // Each face instance (regular, bold) has fonts of its own, the one seen first first: a text of one weight is chunked
    // as it always was.
    let mut groups: Vec<(ShapedScript, Vec<usize>)> = Vec::new();
    for (k, c) in keys.iter().enumerate() {
        match groups.iter_mut().find(|(script, _)| *script == c.script) {
            Some((_, group)) => group.push(k),
            None => groups.push((c.script, vec![k])),
        }
    }
    // Each distinct cluster's font (an index into `names`) and code in it.
    let mut slots: Vec<(usize, u8)> = vec![(0, 1); keys.len()];
    let mut taken: HashSet<Vec<u8>> = fonts_res.iter().map(|(name, _)| name.clone()).collect();
    let mut names = Vec::new();
    for (script, group) in &groups {
        let weight = if script.weight() == Weight::Bold { "B" } else { "" };
        for chunk in group.chunks(per_font) {
            let clusters: Vec<&ShapedCluster> = chunk.iter().filter_map(|k| keys.get(*k).copied()).collect();
            let font = type3_shaped(doc, &clusters, &outlines, fonts.font_name);
            let mut name = format!("{}{}{weight}", fonts.name_prefix, font.num);
            let mut suffix = 0usize;
            while taken.contains(name.as_bytes()) {
                suffix = suffix.saturating_add(1);
                name = format!("{}{}{weight}_{suffix}", fonts.name_prefix, font.num);
            }
            taken.insert(name.clone().into_bytes());
            fonts_res.set(name.clone().into_bytes(), Object::Ref(font));
            for (i, k) in chunk.iter().enumerate() {
                // Codes start at 1; a chunk holds at most 240.
                if let Some(slot) = slots.get_mut(*k) {
                    *slot = (names.len(), u8::try_from(i.saturating_add(1)).unwrap_or(1));
                }
            }
            names.push(name);
        }
    }

    let mut planned = Vec::with_capacity(laid.len());
    for ((line, parts, _), row) in laid.iter().zip(staged) {
        let mut runs: Vec<(usize, Run)> = Vec::new();
        for item in row {
            match item {
                Staged::Latin(look, part, ops, text) => runs.push((part, Run::Latin(look, ops, text))),
                Staged::Shaped(look, part, ks) => {
                    for (k, space) in ks {
                        let Some(&(font, code)) = slots.get(k) else { continue };
                        match runs.last_mut() {
                            Some((p, Run::Shaped(l, f, codes, spaces))) if *f == font && *l == look && *p == part => {
                                codes.push(code);
                                spaces.push(space);
                            }
                            _ => runs.push((part, Run::Shaped(look, font, vec![code], vec![space]))),
                        }
                    }
                }
            }
        }
        let spans = split_spans(line, parts, &runs);
        planned.push(PlannedLine { logical: line.clone(), runs, spans });
    }
    let family = ShapedScript::Arabic.font().map_or("Arabic", |f| f.family).to_owned();
    Ok(ShapedPlan { names, latin_fonts: latins.iter().map(|l| l.font.to_owned()).collect(), lines: planned, family })
}

impl ShapedPlan {
    /// The family of the face the glyphs come from.
    pub fn family(&self) -> &str {
        &self.family
    }

    /// The operators that draw planned line `i` at the current text position: its span, the Latin
    /// and shaped runs in drawing order. `current` is the font the text state holds, and holds the
    /// last one used afterwards. A line with nothing to draw gives no operators.
    pub fn line_ops(&self, i: usize, size: f64, current: &mut String) -> Vec<TextOp> {
        let mut pen = Pen { font: std::mem::take(current), size, fill: Vec::new() };
        let ops = self.looked_line_ops(i, &[RunLook { size, fill: &[] }], &mut pen);
        *current = pen.font;
        ops
    }

    /// [`Self::line_ops`] for a plan of several looks ([`plan_looked`]): each run is drawn in `looks[its look]` (a look
    /// past the end in the first), its fill colour and size set where they change, in a span for each look where the
    /// plan gave the line those (see [`plan_looked`]). `pen` is the text state, and holds the
    /// last font, size and fill afterwards. No look at all draws nothing.
    pub fn looked_line_ops(&self, i: usize, looks: &[RunLook], pen: &mut Pen) -> Vec<TextOp> {
        let Some(line) = self.lines.get(i).filter(|l| !l.runs.is_empty()) else { return Vec::new() };
        if looks.is_empty() {
            return Vec::new();
        }
        let mut ops = Vec::new();
        for (runs, text) in Self::spans(line) {
            ops.push(Self::span(text));
            ops.extend(self.run_ops(runs, looks, pen));
            ops.push(TextOp::new("EMC", vec![]));
        }
        ops
    }

    /// The spans line `line` is drawn in, in drawing order: the runs each holds and its `/ActualText` (one span of the
    /// whole line unless [`split_spans`] gave it several).
    fn spans(line: &PlannedLine) -> Vec<(&[(usize, Run)], &str)> {
        if line.spans.is_empty() {
            return vec![(line.runs.as_slice(), line.logical.as_str())];
        }
        line.spans.iter().map(|(range, text)| (line.runs.get(range.clone()).unwrap_or_default(), text.as_str())).collect()
    }

    fn span(text: &str) -> TextOp {
        let mut bytes = vec![0xFE, 0xFF];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        let mut properties = Dict::new();
        properties.set(b"ActualText".to_vec(), Object::String(PdfString { bytes, hex: true }));
        TextOp::new("BDC", vec![Object::name("Span"), Object::Dict(properties)])
    }

    /// How many spaces between words planned line `i` draws: the gaps [`Self::line_ops_stretched`] widens (a space of
    /// Latin text, or a shaped space), whatever their looks.
    pub fn gaps(&self, i: usize) -> usize {
        let Some(line) = self.lines.get(i) else { return 0 };
        line.runs
            .iter()
            .map(|(_, run)| match run {
                Run::Latin(_, _, text) => text.matches(' ').count(),
                Run::Shaped(_, _, _, spaces) => spaces.iter().filter(|s| **s).count(),
            })
            .sum()
    }

    /// [`Self::line_ops`] with every gap (see [`Self::gaps`]) widened by `gap`, in the text space units of a space's
    /// advance before the horizontal scale (as word spacing is), so a justified line fills its width. Word spacing
    /// reaches only a single-byte code 32, which these fonts' codes are not, so each gap is a `TJ` adjustment after
    /// the space: the shaped codes are shown as the strings of a `TJ` array, and Latin text is shown by `show` word by
    /// word (up to and including each space) followed by the adjustment. A `gap` that is not positive, a `size` that is
    /// not, or no gap in the line gives the operators of [`Self::line_ops`].
    pub fn line_ops_stretched(&self, i: usize, size: f64, current: &mut String, gap: f64, show: LatinOps) -> Result<Vec<TextOp>, ShapedTextError> {
        let mut pen = Pen { font: std::mem::take(current), size, fill: Vec::new() };
        let ops = self.looked_line_ops_stretched(i, &[RunLook { size, fill: &[] }], &mut pen, gap, &[show]);
        *current = pen.font;
        ops
    }

    /// [`Self::line_ops_stretched`] for a plan of several looks (see [`Self::looked_line_ops`]): the gap is the same in
    /// every look, each adjustment in the size of the run it follows, and the Latin text of look `i` is shown by
    /// `shows[i]` (a look past the end by the first). A look whose size is not positive gives the plain line.
    pub fn looked_line_ops_stretched(
        &self,
        i: usize,
        looks: &[RunLook],
        pen: &mut Pen,
        gap: f64,
        shows: &[LatinOps],
    ) -> Result<Vec<TextOp>, ShapedTextError> {
        let adjustment = |size: f64| -1000.0 * gap / size;
        let stretches = gap > 0.0 && !looks.is_empty() && looks.iter().all(|l| l.size > 0.0 && adjustment(l.size).is_finite());
        if !stretches || self.gaps(i) == 0 {
            return Ok(self.looked_line_ops(i, looks, pen));
        }
        let Some(line) = self.lines.get(i).filter(|l| !l.runs.is_empty()) else { return Ok(Vec::new()) };
        let mut ops = Vec::new();
        for (runs, text) in Self::spans(line) {
            ops.push(Self::span(text));
            self.stretched_runs(&mut ops, runs, looks, pen, &adjustment, shows)?;
            ops.push(TextOp::new("EMC", vec![]));
        }
        Ok(ops)
    }

    /// The operators of [`Self::looked_line_ops_stretched`] that draw `runs`, each gap widened by `adjustment` of the size
    /// of the run it is in.
    fn stretched_runs(
        &self,
        ops: &mut Vec<TextOp>,
        runs: &[(usize, Run)],
        looks: &[RunLook],
        pen: &mut Pen,
        adjustment: &dyn Fn(f64) -> f64,
        shows: &[LatinOps],
    ) -> Result<(), ShapedTextError> {
        for (_, run) in runs {
            // (`looks` is not empty, checked above)
            let Some(look) = looks.get(run.look()).or_else(|| looks.first()) else { continue };
            match run {
                Run::Latin(look_index, shown, text) => {
                    pen.select(ops, self.latin_font(*look_index), look);
                    if text.contains(' ') {
                        let show = shows
                            .get(*look_index)
                            .or_else(|| shows.first())
                            .ok_or_else(|| ShapedTextError("no font to show Latin text in".into()))?;
                        for word in text.split_inclusive(' ') {
                            ops.extend(show(word)?);
                            if word.ends_with(' ') {
                                ops.push(TextOp::new("TJ", vec![Object::Array(vec![number(adjustment(look.size))])]));
                            }
                        }
                    } else {
                        ops.extend(shown.iter().cloned());
                    }
                }
                Run::Shaped(_, font, codes, spaces) => {
                    let Some(name) = self.names.get(*font) else { continue };
                    pen.select(ops, name, look);
                    if spaces.iter().any(|s| *s) {
                        let mut array = Vec::new();
                        let mut word = Vec::new();
                        for (code, space) in codes.iter().zip(spaces) {
                            word.push(*code);
                            if *space {
                                array.push(Object::String(PdfString::literal(std::mem::take(&mut word))));
                                array.push(number(adjustment(look.size)));
                            }
                        }
                        if !word.is_empty() {
                            array.push(Object::String(PdfString::literal(word)));
                        }
                        ops.push(TextOp::new("TJ", vec![Object::Array(array)]));
                    } else {
                        ops.push(TextOp::new("Tj", vec![Object::String(PdfString::literal(codes.clone()))]));
                    }
                }
            }
        }
        Ok(())
    }

    /// [`Self::line_ops`] without the span: the operators that show planned line `i`, for a caller
    /// that wraps several of them (the cells of a comb field) in one span of its own.
    pub fn bare_ops(&self, i: usize, size: f64, current: &mut String) -> Vec<TextOp> {
        let Some(line) = self.lines.get(i) else { return Vec::new() };
        let mut pen = Pen { font: std::mem::take(current), size, fill: Vec::new() };
        let ops = self.run_ops(&line.runs, &[RunLook { size, fill: &[] }], &mut pen);
        *current = pen.font;
        ops
    }

    /// The font look `look` shows its Latin text in (a look past the end, the first's).
    fn latin_font(&self, look: usize) -> &str {
        self.latin_fonts.get(look).or_else(|| self.latin_fonts.first()).map_or("", String::as_str)
    }

    fn run_ops(&self, runs: &[(usize, Run)], looks: &[RunLook], pen: &mut Pen) -> Vec<TextOp> {
        let mut ops = Vec::new();
        for (_, run) in runs {
            let Some(look) = looks.get(run.look()).or_else(|| looks.first()) else { continue };
            match run {
                Run::Latin(look_index, shown, _) => {
                    pen.select(&mut ops, self.latin_font(*look_index), look);
                    ops.extend(shown.iter().cloned());
                }
                Run::Shaped(_, font, codes, _) => {
                    let Some(name) = self.names.get(*font) else { continue };
                    pen.select(&mut ops, name, look);
                    ops.push(TextOp::new("Tj", vec![Object::String(PdfString::literal(codes.clone()))]));
                }
            }
        }
        ops
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    fn square(_: ShapedScript, id: u32) -> Result<GlyphOutline, GlyphError> {
        if id == 99 {
            return Err(GlyphError::TooComplex);
        }
        Ok(GlyphOutline { contours: vec![vec![[0.0, 0.0], [0.5, 0.0], [0.5, 0.25]]], width: 0.5, bbox: [0.0, 0.0, 0.5, 0.25] })
    }

    const SQUARES: Shaper = Shaper { shape: |_, _, _| Ok(Vec::new()), has: |_, _| true, outline: square, arabic_available: || true };

    fn cluster(id: u32, advance: f64, offset: [f64; 2]) -> ShapedCluster {
        ShapedCluster { script: ShapedScript::Arabic, glyphs: vec![(id, offset)], advance, text: "x".into() }
    }

    #[test]
    fn clusters_become_filled_paths_at_the_pen() {
        let mut out = b"q\n".to_vec();
        let clusters = [cluster(1, 0.5, [0.0, 0.0]), cluster(2, 0.0, [0.1, 0.2])];
        let pen = cluster_paths(&mut out, &clusters, &SQUARES, 10.0, 20.0, 10.0).unwrap();
        assert_eq!(pen, 15.0, "the pen advances by the first cluster's width only");
        assert_eq!(String::from_utf8(out).unwrap(), "q\n10 20 m\n15 20 l\n15 22.5 l\nh\nf\n16 22 m\n21 22 l\n21 24.5 l\nh\nf\n");
    }

    #[test]
    fn a_glyph_that_cannot_be_read_appends_nothing() {
        let mut out = b"q\n".to_vec();
        let clusters = [cluster(1, 0.5, [0.0, 0.0]), cluster(99, 0.5, [0.0, 0.0])];
        assert!(cluster_paths(&mut out, &clusters, &SQUARES, 0.0, 0.0, 12.0).is_err());
        assert_eq!(out, b"q\n");
        for (x, size) in [(f64::NAN, 12.0), (0.0, f64::INFINITY), (f64::MAX, 1e308)] {
            assert!(cluster_paths(&mut out, &clusters[..1], &SQUARES, x, 0.0, size).is_err(), "{x} {size}");
            assert_eq!(out, b"q\n", "{x} {size}");
        }
        assert_eq!(cluster_paths(&mut out, &[], &SQUARES, 3.0, 0.0, 12.0), Ok(3.0));
    }

    #[test]
    fn the_span_holds_the_typed_text_as_utf16() {
        assert_eq!(actual_text_bdc("اب"), b"/Span <</ActualText <FEFF06270628>>> BDC\n");
        assert_eq!(actual_text_bdc("a\u{1F600}"), b"/Span <</ActualText <FEFF0061D83DDE00>>> BDC\n");
        assert_eq!(actual_text_bdc(""), b"/Span <</ActualText <FEFF>>> BDC\n");
    }
}

#[cfg(test)]
mod stretch_tests {
    use super::*;

    /// Each Arabic letter is a cluster of its own, and so is each space (its text is `" "`), as the faces shape them.
    fn letters(_: ShapedScript, text: &str, _: bool) -> Result<Vec<ShapedCluster>, GlyphError> {
        Ok(text
            .chars()
            .map(|c| ShapedCluster {
                script: ShapedScript::Arabic,
                glyphs: vec![(u32::from(c), [0.0, 0.0])],
                advance: if c == ' ' { 0.25 } else { 0.5 },
                text: c.to_string(),
            })
            .collect())
    }

    fn box_outline(_: ShapedScript, _: u32) -> Result<GlyphOutline, GlyphError> {
        Ok(GlyphOutline { contours: vec![vec![[0.0, 0.0], [0.5, 0.0], [0.5, 0.5]]], width: 0.5, bbox: [0.0, 0.0, 0.5, 0.5] })
    }

    const LETTERS: Shaper = Shaper { shape: letters, has: |_, c| c == ' ', outline: box_outline, arabic_available: || true };

    fn latin(s: &str) -> Result<Vec<TextOp>, ShapedTextError> {
        Ok(vec![TextOp::new("Tj", vec![Object::String(PdfString::literal(s.as_bytes().to_vec()))])])
    }

    fn planned(lines: &[&str]) -> ShapedPlan {
        set_test_shaper(Some(LETTERS));
        let mut doc = Document::new_empty();
        let mut fonts = Dict::new();
        let names = PlanFonts { max_glyphs: GLYPHS_PER_FONT, name_prefix: "PCT", font_name: None };
        let lines: Vec<String> = lines.iter().map(|l| (*l).to_owned()).collect();
        let plan = plan(&mut doc, &mut fonts, &lines, true, names, &LatinShow { font: "F9", show: &latin }).expect("plans");
        set_test_shaper(None);
        plan
    }

    fn name(op: &TextOp) -> String {
        String::from_utf8_lossy(&op.operator).into_owned()
    }

    #[test]
    fn a_stretched_line_has_an_adjustment_after_every_space() {
        // (Arabic letters, a space, a Latin word, a space, more Arabic: three gaps would be two spaces and the Latin's own.)
        let plan = planned(&["\u{627}\u{628} \u{627}\u{628} \u{627}", "\u{627}\u{628}"]);
        assert_eq!(plan.gaps(0), 2);
        assert_eq!(plan.gaps(1), 0);
        assert_eq!(plan.gaps(9), 0, "no such line");
        let mut current = String::new();
        let ops = plan.line_ops_stretched(0, 10.0, &mut current, 2.0, &latin).expect("stretches");
        let tj: Vec<&TextOp> = ops.iter().filter(|op| op.operator == b"TJ").collect();
        assert_eq!(tj.len(), 1, "{ops:?}");
        // [(ab ) -200 (ab ) -200 (a)]: 2 text units on a 10 point size is 200 thousandths.
        let Some(Object::Array(items)) = tj[0].operands.first() else { panic!("an array: {:?}", tj[0]) };
        let adjustments: Vec<f64> = items.iter().filter_map(Object::as_f64).collect();
        assert_eq!(adjustments, [-200.0, -200.0], "{items:?}");
        assert_eq!(items.iter().filter(|i| matches!(i, Object::String(_))).count(), 3);
        // It is the line's span, its font set once, and nothing else but the strings.
        assert_eq!(name(&ops[0]), "BDC");
        assert_eq!(name(ops.last().expect("an end")), "EMC");
        assert_eq!(ops.iter().filter(|op| op.operator == b"Tf").count(), 1);
    }

    #[test]
    fn a_line_with_nothing_to_widen_or_no_room_to_widen_it_is_the_plain_line() {
        let plan = planned(&["\u{627}\u{628} \u{627}\u{628}", "\u{627}\u{628}"]);
        for (gap, size) in [(0.0, 10.0), (-1.0, 10.0), (f64::NAN, 10.0), (2.0, 0.0), (f64::INFINITY, 10.0)] {
            let ops = plan.line_ops_stretched(0, size, &mut String::new(), gap, &latin).expect("plain");
            assert_eq!(ops, plan.line_ops(0, size, &mut String::new()), "gap {gap}, size {size}");
        }
        // (A size that is not a number gives the plain line too, though NaN is not equal to itself.)
        assert_eq!(plan.line_ops_stretched(0, f64::NAN, &mut String::new(), 2.0, &latin).expect("plain").len(), 4);
        assert!(plan.line_ops_stretched(9, 10.0, &mut String::new(), 2.0, &latin).expect("no line").is_empty());
        // A line without a space is the plain line whatever the gap.
        assert_eq!(plan.line_ops_stretched(1, 10.0, &mut String::new(), 2.0, &latin).expect("plain"), plan.line_ops(1, 10.0, &mut String::new()));
    }

    fn looked(lines: &[(&str, Vec<usize>)]) -> ShapedPlan {
        set_test_shaper(Some(LETTERS));
        let mut doc = Document::new_empty();
        let mut fonts = Dict::new();
        let names = PlanFonts { max_glyphs: GLYPHS_PER_FONT, name_prefix: "PCT", font_name: None };
        let lines: Vec<(String, Vec<usize>)> = lines.iter().map(|(l, looks)| ((*l).to_owned(), looks.clone())).collect();
        let latins = [LatinShow { font: "F9", show: &latin }, LatinShow { font: "F8", show: &latin }];
        let plan = plan_looked(&mut doc, &mut fonts, &lines, true, names, &latins, &[]).expect("plans");
        set_test_shaper(None);
        plan
    }

    /// The `/ActualText` of each span `ops` open, in order.
    fn spans_of(ops: &[TextOp]) -> Vec<String> {
        ops.iter()
            .filter(|op| op.operator == b"BDC")
            .filter_map(|op| match op.operands.get(1) {
                Some(Object::Dict(d)) => match d.get(b"ActualText") {
                    Some(Object::String(s)) => Some(s.to_text()),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    #[test]
    fn pieces_read_in_the_order_page_text_extraction_puts_them_in() {
        assert!(reading_order(&[]).is_empty());
        assert_eq!(reading_order(&["ab", "cd"]), [0, 1]);
        // Right-to-left pieces drawn left to right are read right to left, with the Latin ones among them left to right.
        assert_eq!(reading_order(&["\u{628}", "\u{627} "]), [1, 0]);
        assert_eq!(reading_order(&["\u{628}", "ab", "cd ", "\u{627} ", "\u{62A} "]), [4, 3, 1, 2, 0]);
        // As many pieces of each: the line is left to right, as extraction takes it.
        assert_eq!(reading_order(&["ab ", "\u{627}"]), [0, 1]);
        // Numbers keep their order; a separator between two belongs to them.
        assert_eq!(reading_order(&["\u{628} ", "12", ".", "5", " \u{627}"]), [4, 1, 2, 3, 0]);
        let hostile: Vec<String> = (0..5_000).map(|i| if i % 3 == 0 { "\u{627}".to_owned() } else { format!("{i}") }).collect();
        let texts: Vec<&str> = hostile.iter().map(String::as_str).collect();
        let mut order = reading_order(&texts);
        order.sort_unstable();
        assert_eq!(order, (0..5_000).collect::<Vec<_>>());
    }

    #[test]
    fn a_line_whose_looks_interleave_as_drawn_keeps_one_span() {
        // `اب AB` in look 0 and ` CD اب` in look 1, right to left: drawn `اب`, `AB`, ` CD`, `اب`, the looks in turn.
        let line = "\u{627}\u{628} AB CD \u{627}\u{628}";
        let plan = looked(&[(line, vec![0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1])]);
        let looks = [RunLook { size: 10.0, fill: &[] }, RunLook { size: 20.0, fill: &[] }];
        let ops = plan.looked_line_ops(0, &looks, &mut Pen::default());
        assert_eq!(spans_of(&ops), [line]);
        // A line of one look is one span whatever its scripts.
        let plan = looked(&[(line, Vec::new())]);
        assert_eq!(spans_of(&plan.looked_line_ops(0, &looks, &mut Pen::default())), [line]);
    }

    #[test]
    fn pieces_end_where_the_look_changes_and_one_look_cuts_nothing() {
        let shaper = LETTERS;
        let latin = |pieces: Vec<(usize, Piece)>| -> Vec<(usize, String)> {
            pieces.into_iter().filter_map(|(look, p)| if let Piece::Latin(s) = p { Some((look, s)) } else { None }).collect()
        };
        assert_eq!(latin(shaped_pieces_looked("abcd", &[0, 0, 1, 1], false, &shaper).expect("lays out")), [(0, "ab".into()), (1, "cd".into())]);
        // A right-to-left line draws the later look first; missing looks are look 0.
        let pieces = shaped_pieces_looked("\u{627}\u{628} \u{627}\u{628}", &[0, 0, 0, 1], true, &shaper).expect("lays out");
        assert_eq!(pieces.iter().map(|(look, _)| *look).collect::<Vec<_>>(), [0, 1, 0]);
        assert_eq!(
            shaped_pieces_looked("ab \u{627}", &[], false, &shaper).expect("lays out").into_iter().map(|(_, p)| p).collect::<Vec<_>>(),
            shaped_pieces("ab \u{627}", false, &shaper).expect("lays out")
        );
        assert!(shaped_pieces_looked("", &[3], true, &shaper).expect("nothing").is_empty());
    }

    #[test]
    fn a_line_of_two_looks_sets_each_looks_size_and_fill_where_it_changes() {
        // (The second word in look 1: drawn first, right to left.)
        let plan = looked(&[("\u{627}\u{628} \u{627}\u{628} ab", vec![0, 0, 0, 1, 1, 1, 1, 1])]);
        let (black, red) = ([TextOp::new("g", vec![Object::Int(0)])], [TextOp::new("rg", vec![Object::Int(1), Object::Int(0), Object::Int(0)])]);
        let looks = [RunLook { size: 10.0, fill: &black }, RunLook { size: 20.0, fill: &red }];
        let mut pen = Pen { font: String::new(), size: 10.0, fill: black.to_vec() };
        let ops = plan.looked_line_ops(0, &looks, &mut pen);
        let states: Vec<String> = ops
            .iter()
            .filter(|op| matches!(op.operator.as_slice(), b"Tf" | b"g" | b"rg"))
            .map(|op| format!("{} {:?}", name(op), op.operands.iter().map(|o| o.as_f64().unwrap_or(-1.0)).collect::<Vec<_>>()))
            .collect();
        // The Latin word of look 1 in its font F8 at 20, in red; its shaped word; then look 0 at 10 in black.
        assert_eq!(states, ["rg [1.0, 0.0, 0.0]", "Tf [-1.0, 20.0]", "Tf [-1.0, 20.0]", "g [0.0]", "Tf [-1.0, 10.0]"], "{ops:?}");
        assert!(pen.font.starts_with("PCT") && pen.size == 10.0 && pen.fill == black, "{pen:?}");
        // A span for each look, in drawing order, each holding its part of the line in typing order; the gaps of the whole
        // line, whatever their looks.
        assert_eq!(spans_of(&ops), ["\u{627}\u{628} ab", "\u{627}\u{628} "]);
        assert_eq!(plan.gaps(0), 2);
        // Stretched, each gap is the same distance: an adjustment in the size of the run it follows.
        let mut pen = Pen { font: String::new(), size: 10.0, fill: black.to_vec() };
        let ops = plan.looked_line_ops_stretched(0, &looks, &mut pen, 2.0, &[&latin, &latin]).expect("stretches");
        let adjustments: Vec<f64> = ops
            .iter()
            .filter(|op| op.operator == b"TJ")
            .flat_map(|op| match op.operands.first() {
                Some(Object::Array(items)) => items.iter().filter_map(Object::as_f64).collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect();
        assert_eq!(adjustments, [-100.0, -200.0], "{ops:?}");
    }
}

#[cfg(test)]
mod weight_tests {
    use super::*;

    /// Each letter a cluster of its own; a bold one (asked for as [`ShapedScript::ArabicBold`]) a fifth wider.
    fn weighted_letters(script: ShapedScript, text: &str, _: bool) -> Result<Vec<ShapedCluster>, GlyphError> {
        let wider = if script == ShapedScript::ArabicBold { 1.2 } else { 1.0 };
        Ok(text
            .chars()
            .map(|c| ShapedCluster { script, glyphs: vec![(u32::from(c), [0.0, 0.0])], advance: 0.5 * wider, text: c.to_string() })
            .collect())
    }

    /// The faces without a bold instance: bold is asked for and regular comes back.
    fn regular_only(script: ShapedScript, text: &str, rtl: bool) -> Result<Vec<ShapedCluster>, GlyphError> {
        weighted_letters(script.weighted(Weight::Regular), text, rtl)
    }

    fn outline(script: ShapedScript, _: u32) -> Result<GlyphOutline, GlyphError> {
        let w = if script == ShapedScript::ArabicBold { 0.6 } else { 0.5 };
        Ok(GlyphOutline { contours: vec![vec![[0.0, 0.0], [w, 0.0], [w, 0.5]]], width: w, bbox: [0.0, 0.0, w, 0.5] })
    }

    const WEIGHTED: Shaper = Shaper { shape: weighted_letters, has: |_, c| c == ' ', outline, arabic_available: || true };
    const REGULAR_ONLY: Shaper = Shaper { shape: regular_only, has: |_, c| c == ' ', outline, arabic_available: || true };

    fn latin(s: &str) -> Result<Vec<TextOp>, ShapedTextError> {
        Ok(vec![TextOp::new("Tj", vec![Object::String(PdfString::literal(s.as_bytes().to_vec()))])])
    }

    /// `lines` planned with `shaper` at `weights`: the document, its font resources, the plan and line 0's operators.
    fn planned(shaper: Shaper, lines: &[(&str, Vec<usize>)], weights: &[Weight]) -> (Document, Dict, Vec<TextOp>) {
        set_test_shaper(Some(shaper));
        let mut doc = Document::new_empty();
        let mut fonts = Dict::new();
        let names = PlanFonts { max_glyphs: GLYPHS_PER_FONT, name_prefix: "PCT", font_name: None };
        let lines: Vec<(String, Vec<usize>)> = lines.iter().map(|(l, looks)| ((*l).to_owned(), looks.clone())).collect();
        let latins = [LatinShow { font: "F9", show: &latin }, LatinShow { font: "F8", show: &latin }];
        let plan = plan_looked(&mut doc, &mut fonts, &lines, true, names, &latins, weights).expect("plans");
        set_test_shaper(None);
        let looks = [RunLook { size: 10.0, fill: &[] }, RunLook { size: 10.0, fill: &[] }];
        let ops = plan.looked_line_ops(0, &looks, &mut Pen::default());
        (doc, fonts, ops)
    }

    /// The decoded bytes of the stream `o` is (or refers to).
    fn decoded(doc: &Document, o: &Object) -> Option<Vec<u8>> {
        match &*doc.resolve(o) {
            Object::Stream(s) => s.decoded().ok(),
            _ => None,
        }
    }

    fn font_names(fonts: &Dict) -> Vec<String> {
        fonts.iter().map(|(name, _)| String::from_utf8_lossy(name).into_owned()).collect()
    }

    /// The fonts selected by the line's `Tf`s, in order, each with the string it shows next.
    fn shown(ops: &[TextOp]) -> Vec<(String, Vec<u8>)> {
        let mut font = String::new();
        let mut out = Vec::new();
        for op in ops {
            match (op.operator.as_slice(), op.operands.first()) {
                (b"Tf", Some(Object::Name(name))) => font = String::from_utf8_lossy(name).into_owned(),
                (b"Tj", Some(Object::String(s))) => out.push((font.clone(), s.bytes.clone())),
                _ => {}
            }
        }
        out
    }

    #[test]
    fn a_bold_look_is_drawn_from_fonts_of_its_own() {
        // The first word in look 0 (regular), the second in look 1 (bold); right to left, the second is drawn first.
        let line = ("\u{627}\u{628} \u{628}\u{627}", vec![0, 0, 0, 1, 1]);
        let (doc, fonts, ops) = planned(WEIGHTED, std::slice::from_ref(&line), &[Weight::Regular, Weight::Bold]);
        let names = font_names(&fonts);
        assert_eq!(names.len(), 2, "{names:?}");
        let bold: Vec<&String> = names.iter().filter(|n| n.ends_with('B')).collect();
        assert_eq!(bold.len(), 1, "one font of bold glyphs, its name marked: {names:?}");
        // The bold word is shown in the bold font, the regular word (and its space) in the other.
        let drawn = shown(&ops);
        assert_eq!(drawn.len(), 2, "{ops:?}");
        assert_eq!(&drawn[0].0, bold[0]);
        assert!(!drawn[1].0.ends_with('B'));
        // The bold font says it is bold, and draws the bold outlines at the bold advances.
        let font = |name: &str| doc.resolve(fonts.get(name.as_bytes()).expect("a font")).as_dict().cloned().expect("a dict");
        let descriptor = |name: &str| doc.resolve(font(name).get(b"FontDescriptor").expect("a descriptor")).as_dict().cloned().expect("a dict");
        let regular_name = names.iter().find(|n| !n.ends_with('B')).expect("a regular font");
        assert_eq!(descriptor(bold[0]).get(b"FontWeight").and_then(Object::as_f64), Some(700.0));
        assert!(descriptor(bold[0]).name(b"FontName").is_some_and(|n| n.ends_with(b"-Bold")));
        assert_eq!(descriptor(regular_name).get(b"FontWeight"), None);
        let widths = |name: &str| match font(name).get(b"Widths") {
            Some(Object::Array(w)) => w.iter().filter_map(Object::as_f64).collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        assert_eq!(widths(bold[0]), [600.0, 600.0]);
        assert_eq!(widths(regular_name), [500.0, 500.0, 500.0]);
        let procs = |name: &str| -> Vec<Vec<u8>> {
            let Some(Object::Dict(procs)) = font(name).get(b"CharProcs").cloned() else { return Vec::new() };
            procs.iter().filter_map(|(_, p)| decoded(&doc, p)).collect()
        };
        let (heavy, light) = (procs(bold[0]), procs(regular_name));
        assert!(heavy.iter().all(|p| !light.contains(p)), "{heavy:?} {light:?}");
        // Measured at the bold advances too.
        let zero = |_: &str| 0.0;
        set_test_shaper(Some(WEIGHTED));
        let measure = Measure::with_looks(true, vec![(10.0, &zero), (10.0, &zero)]);
        let regular_width = measure.line_looked(line.0, &line.1);
        let measure = measure.weighted(vec![Weight::Regular, Weight::Bold]);
        let bold_width = measure.line_looked(line.0, &line.1);
        let all_bold = Measure::new(true, 10.0, &zero).weighted(vec![Weight::Bold]).line(line.0);
        set_test_shaper(None);
        assert!(
            (regular_width - 25.0).abs() < 1e-9 && (bold_width - 27.0).abs() < 1e-9 && (all_bold - 30.0).abs() < 1e-9,
            "{regular_width} {bold_width} {all_bold}"
        );
    }

    #[test]
    fn regular_is_planned_as_before_and_bold_without_a_bold_instance_too() {
        let lines = [("\u{627}\u{628} \u{628}\u{627}", vec![0, 0, 0, 1, 1]), ("\u{628}", vec![1])];
        let bytes = |(doc, fonts, ops): (Document, Dict, Vec<TextOp>)| {
            let objects: Vec<String> = doc.object_numbers().into_iter().map(|n| format!("{n} {:?}", doc.try_get(n).ok())).collect();
            (objects, font_names(&fonts), format!("{ops:?}"))
        };
        // No weight, all regular, and bold that the faces can't draw (no `wght` axis) are one plan.
        let before = bytes(planned(WEIGHTED, &lines, &[]));
        assert_eq!(bytes(planned(WEIGHTED, &lines, &[Weight::Regular, Weight::Regular])), before);
        assert_eq!(bytes(planned(REGULAR_ONLY, &lines, &[Weight::Regular, Weight::Bold])), before);
        assert_eq!(bytes(planned(REGULAR_ONLY, &lines, &[Weight::Bold])), before);
        // (And bold that can be drawn is another.)
        assert_ne!(bytes(planned(WEIGHTED, &lines, &[Weight::Regular, Weight::Bold])), before);
    }

    #[test]
    fn the_real_face_draws_a_bold_word_wider_from_other_glyph_procedures() {
        if crate::document_arabic_font().is_none() {
            eprintln!("skipping the bold plan check: built without a craft-fonts Arab face (set CRAFT_FONTS_DIR)");
            return;
        }
        let word = "بالعالم";
        let plan_at = |weight: Weight| {
            let mut doc = Document::new_empty();
            let mut fonts = Dict::new();
            let names = PlanFonts { max_glyphs: GLYPHS_PER_FONT, name_prefix: "PCT", font_name: None };
            plan_looked(&mut doc, &mut fonts, &[(word.to_owned(), Vec::new())], true, names, &[LatinShow { font: "F9", show: &latin }], &[weight])
                .expect("plans");
            let (name, font) = fonts.iter().next().map(|(n, f)| (String::from_utf8_lossy(n).into_owned(), f.clone())).expect("a font");
            let font = doc.resolve(&font).as_dict().cloned().expect("a dict");
            let widths: f64 = match font.get(b"Widths") {
                Some(Object::Array(w)) => w.iter().filter_map(Object::as_f64).sum(),
                _ => 0.0,
            };
            let Some(Object::Dict(procs)) = font.get(b"CharProcs").cloned() else { panic!("glyph procedures") };
            let procs: Vec<Vec<u8>> = procs.iter().filter_map(|(_, p)| decoded(&doc, p)).collect();
            (name, widths, procs)
        };
        let (regular_name, regular_widths, regular_procs) = plan_at(Weight::Regular);
        let (bold_name, bold_widths, bold_procs) = plan_at(Weight::Bold);
        assert!(!regular_name.ends_with('B') && bold_name.ends_with('B'), "{regular_name} {bold_name}");
        assert!(bold_widths > regular_widths * 1.02, "{bold_widths} against {regular_widths}");
        assert_eq!(bold_procs.len(), regular_procs.len());
        assert!(bold_procs.iter().zip(&regular_procs).all(|(b, r)| b != r));
        let zero = |_: &str| 0.0;
        let (light, heavy) = (Measure::new(true, 1.0, &zero).line(word), Measure::new(true, 1.0, &zero).weighted(vec![Weight::Bold]).line(word));
        // (A line measures every cluster; a font's widths count each distinct one once.)
        let at = |weight| crate::shape_arabic_at(word, true, weight).expect("shapes").iter().map(cluster_width).sum::<f64>();
        assert!((light - at(Weight::Regular)).abs() < 1e-9 && (heavy - at(Weight::Bold)).abs() < 1e-9 && heavy > light * 1.02, "{light} {heavy}");
    }
}
