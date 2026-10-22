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
//! - [`Measure`]: the width of a line as it will be drawn, for wrapping and alignment.
//!
//! Operators are returned as [`TextOp`] (an operator and its operands) because this crate sits
//! below the content-stream crate; callers turn them into their own operator type.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use pdfcraft_cos::{Dict, Document, ObjRef, Object, PdfString, Stream};
use unicode_bidi::{Level, ParagraphBidiInfo};

use crate::{GlyphError, GlyphOutline, ShapedCluster, ShapedScript, shape_arabic, shape_hebrew, shape_thai, shaped_glyph};

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
    /// One directional run of a script, clusters in drawing order.
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
        ShapedScript::Arabic => shape_arabic(text, rtl),
        ShapedScript::Hebrew => shape_hebrew(text, rtl),
        ShapedScript::Thai => shape_thai(text),
    }
}

fn face_has(script: ShapedScript, c: char) -> bool {
    match script {
        ShapedScript::Arabic => crate::arabic_has(c),
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
        ShapedScript::Arabic => arabic_error(e, text),
        ShapedScript::Hebrew => hebrew_error(e, text),
        ShapedScript::Thai => thai_error(e, text),
    }
}

// ---- layout ----

/// The pieces of `line` in drawing order: Latin text for the caller's own font, and shaped clusters
/// for Arabic, Hebrew and Thai. `rtl` is the paragraph's direction. Widths are the caller's to
/// measure, so each writer uses its own fonts for the Latin.
pub fn shaped_pieces(line: &str, rtl: bool, shaper: &Shaper) -> Result<Vec<Piece>, ShapedTextError> {
    // unicode-bidi indexes the first level of a line, so an empty one would panic.
    if line.is_empty() {
        return Ok(Vec::new());
    }
    let base = if rtl { Level::rtl() } else { Level::ltr() };
    let info = ParagraphBidiInfo::new(line, Some(base));
    let (levels, runs) = info.visual_runs(0..line.len());
    let mut pieces = Vec::new();
    for run in runs {
        let run_rtl = levels.get(run.start).is_some_and(Level::is_rtl);
        // Runs lie on character boundaries; a missing slice is skipped, never a panic.
        let Some(text) = line.get(run) else { continue };
        // Arabic and Hebrew letters take their face; so do spaces and punctuation between them
        // (Hebrew's when they follow Hebrew), which the shaper then mirrors. Latin text (and what
        // the faces lack) keeps the caller's font.
        let mut segments: Vec<(Option<ShapedScript>, String)> = Vec::new();
        for c in text.chars().filter(|c| !is_bidi_control(*c)) {
            let after_hebrew = segments.last().is_some_and(|(kind, _)| *kind == Some(ShapedScript::Hebrew));
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
                Some((kind, s)) if *kind == script => s.push(c),
                _ => segments.push((script, c.to_string())),
            }
        }
        if run_rtl {
            segments.reverse();
        }
        for (script, s) in segments {
            pieces.push(if let Some(script) = script {
                let shaped = (shaper.shape)(script, &s, run_rtl).map_err(|e| script_error(script, e, &s))?;
                Piece::Shaped(shaped)
            } else if run_rtl {
                Piece::Latin(s.chars().rev().map(mirrored).collect())
            } else {
                Piece::Latin(s)
            });
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

/// Measures lines of shaped text, in text space before the horizontal scale.
pub struct Measure<'a> {
    shaper: Shaper,
    rtl: bool,
    size: f64,
    /// The width of Latin text in the font that draws it.
    latin: &'a dyn Fn(&str) -> f64,
    cache: RefCell<HashMap<String, f64>>,
}

impl<'a> Measure<'a> {
    /// Lines of a paragraph that runs `rtl` or not, at `size`; `latin` measures what the caller's
    /// own font draws (at the same size).
    pub fn new(rtl: bool, size: f64, latin: &'a dyn Fn(&str) -> f64) -> Self {
        Measure { shaper: current_shaper(), rtl, size, latin, cache: RefCell::new(HashMap::new()) }
    }

    /// The width of `line` laid out as it will be drawn. A line that can't be shaped is measured as
    /// Latin (drawing then reports why).
    pub fn line(&self, line: &str) -> f64 {
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
    pub fn approx(&self, candidate: &str) -> f64 {
        if candidate.len() <= EXACT_MEASURE {
            return self.line(candidate);
        }
        let words: f64 = candidate.split(' ').map(|w| self.line(w)).sum();
        words + candidate.matches(' ').count() as f64 * self.line(" ")
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
    let family = match clusters.first().map(|c| c.script).filter(|s| clusters.iter().all(|c| c.script == *s)) {
        Some(script) => script.font().map_or(
            match script {
                ShapedScript::Arabic => "Arabic",
                ShapedScript::Hebrew => "Hebrew",
                ShapedScript::Thai => "Thai",
            },
            |f| f.family,
        ),
        None => "PdfCraft Shaped Text",
    };
    let mut descriptor = Dict::new();
    descriptor.set(b"Type".to_vec(), Object::name("FontDescriptor"));
    descriptor.set(b"FontName".to_vec(), Object::name(&family.replace(' ', "")));
    descriptor.set(b"FontFamily".to_vec(), Object::String(PdfString::literal(family.as_bytes().to_vec())));
    descriptor.set(b"Flags".to_vec(), Object::Int(4));
    descriptor.set(b"ItalicAngle".to_vec(), Object::Int(0));
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
    pub show: &'a dyn Fn(&str) -> Result<Vec<TextOp>, ShapedTextError>,
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
    /// Operators that show Latin text in the Latin font, and the text they show.
    Latin(Vec<TextOp>, String),
    /// Codes of the generated font at this index of `names`, and for each code whether it draws a space between words
    /// (a gap [`ShapedPlan::line_ops_stretched`] widens).
    Shaped(usize, Vec<u8>, Vec<bool>),
}

struct PlannedLine {
    /// The line in typing order: its `/ActualText`.
    logical: String,
    runs: Vec<Run>,
}

/// Wrapped lines laid out, with the fonts they need already added to the document.
pub struct ShapedPlan {
    /// The generated fonts' resource names.
    names: Vec<String>,
    latin_font: String,
    lines: Vec<PlannedLine>,
    family: String,
}

enum Staged {
    Latin(Vec<TextOp>, String),
    /// Indexes into the distinct clusters, and whether each is a space.
    Shaped(Vec<(usize, bool)>),
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
    let shaper = current_shaper();
    let mut keys: Vec<&ShapedCluster> = Vec::new();
    let mut index: HashMap<ClusterKey, usize> = HashMap::new();
    let mut staged: Vec<Vec<Staged>> = Vec::with_capacity(laid.len());
    for (_, pieces) in &laid {
        let mut row = Vec::with_capacity(pieces.len());
        for piece in pieces {
            match piece {
                Piece::Latin(s) => row.push(Staged::Latin((latin.show)(s)?, s.clone())),
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
                entry.insert((shaper.outline)(c.script, *id).map_err(|e| script_error(c.script, e, &c.text))?);
            }
        }
    }

    let per_font = fonts.max_glyphs.clamp(1, GLYPHS_PER_FONT);
    let mut taken: HashSet<Vec<u8>> = fonts_res.iter().map(|(name, _)| name.clone()).collect();
    let mut names = Vec::new();
    for chunk in keys.chunks(per_font) {
        let font = type3_shaped(doc, chunk, &outlines, fonts.font_name);
        let mut name = format!("{}{}", fonts.name_prefix, font.num);
        let mut suffix = 0usize;
        while taken.contains(name.as_bytes()) {
            suffix = suffix.saturating_add(1);
            name = format!("{}{}_{suffix}", fonts.name_prefix, font.num);
        }
        taken.insert(name.clone().into_bytes());
        fonts_res.set(name.clone().into_bytes(), Object::Ref(font));
        names.push(name);
    }

    let mut planned = Vec::with_capacity(laid.len());
    for ((line, _), row) in laid.iter().zip(staged) {
        let mut runs: Vec<Run> = Vec::new();
        for item in row {
            match item {
                Staged::Latin(ops, text) => runs.push(Run::Latin(ops, text)),
                Staged::Shaped(ks) => {
                    for (k, space) in ks {
                        let font = k / per_font;
                        // Codes start at 1; `k % per_font + 1` is at most 240.
                        let code = u8::try_from(k % per_font + 1).unwrap_or(1);
                        match runs.last_mut() {
                            Some(Run::Shaped(f, codes, spaces)) if *f == font => {
                                codes.push(code);
                                spaces.push(space);
                            }
                            _ => runs.push(Run::Shaped(font, vec![code], vec![space])),
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
    pub fn family(&self) -> &str {
        &self.family
    }

    /// The operators that draw planned line `i` at the current text position: its span, the Latin
    /// and shaped runs in drawing order. `current` is the font the text state holds, and holds the
    /// last one used afterwards. A line with nothing to draw gives no operators.
    pub fn line_ops(&self, i: usize, size: f64, current: &mut String) -> Vec<TextOp> {
        let Some(line) = self.lines.get(i).filter(|l| !l.runs.is_empty()) else { return Vec::new() };
        let mut ops = vec![Self::span(line)];
        ops.extend(self.run_ops(line, size, current));
        ops.push(TextOp::new("EMC", vec![]));
        ops
    }

    fn span(line: &PlannedLine) -> TextOp {
        let mut bytes = vec![0xFE, 0xFF];
        for unit in line.logical.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        let mut properties = Dict::new();
        properties.set(b"ActualText".to_vec(), Object::String(PdfString { bytes, hex: true }));
        TextOp::new("BDC", vec![Object::name("Span"), Object::Dict(properties)])
    }

    /// How many spaces between words planned line `i` draws: the gaps [`Self::line_ops_stretched`] widens (a space of
    /// Latin text, or a shaped space).
    pub fn gaps(&self, i: usize) -> usize {
        let Some(line) = self.lines.get(i) else { return 0 };
        line.runs
            .iter()
            .map(|run| match run {
                Run::Latin(_, text) => text.matches(' ').count(),
                Run::Shaped(_, _, spaces) => spaces.iter().filter(|s| **s).count(),
            })
            .sum()
    }

    /// [`Self::line_ops`] with every gap (see [`Self::gaps`]) widened by `gap`, in the text space units of a space's
    /// advance before the horizontal scale (as word spacing is), so a justified line fills its width. Word spacing
    /// reaches only a single-byte code 32, which these fonts' codes are not, so each gap is a `TJ` adjustment after
    /// the space: the shaped codes are shown as the strings of a `TJ` array, and Latin text is shown by `show` word by
    /// word (up to and including each space) followed by the adjustment. A `gap` that is not positive, a `size` that is
    /// not, or no gap in the line gives the operators of [`Self::line_ops`].
    pub fn line_ops_stretched(
        &self,
        i: usize,
        size: f64,
        current: &mut String,
        gap: f64,
        show: &dyn Fn(&str) -> Result<Vec<TextOp>, ShapedTextError>,
    ) -> Result<Vec<TextOp>, ShapedTextError> {
        let adjustment = -1000.0 * gap / size;
        if !(gap > 0.0 && size > 0.0 && adjustment.is_finite()) || self.gaps(i) == 0 {
            return Ok(self.line_ops(i, size, current));
        }
        let Some(line) = self.lines.get(i).filter(|l| !l.runs.is_empty()) else { return Ok(Vec::new()) };
        let mut ops = vec![Self::span(line)];
        let mut select = |ops: &mut Vec<TextOp>, font: &str| {
            if current != font {
                ops.push(TextOp::new("Tf", vec![Object::name(font), number(size)]));
                *current = font.to_owned();
            }
        };
        for run in &line.runs {
            match run {
                Run::Latin(shown, text) => {
                    select(&mut ops, &self.latin_font);
                    if text.contains(' ') {
                        for word in text.split_inclusive(' ') {
                            ops.extend(show(word)?);
                            if word.ends_with(' ') {
                                ops.push(TextOp::new("TJ", vec![Object::Array(vec![number(adjustment)])]));
                            }
                        }
                    } else {
                        ops.extend(shown.iter().cloned());
                    }
                }
                Run::Shaped(font, codes, spaces) => {
                    let Some(name) = self.names.get(*font) else { continue };
                    select(&mut ops, name);
                    if spaces.iter().any(|s| *s) {
                        let mut array = Vec::new();
                        let mut word = Vec::new();
                        for (code, space) in codes.iter().zip(spaces) {
                            word.push(*code);
                            if *space {
                                array.push(Object::String(PdfString::literal(std::mem::take(&mut word))));
                                array.push(number(adjustment));
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
        ops.push(TextOp::new("EMC", vec![]));
        Ok(ops)
    }

    /// [`Self::line_ops`] without the span: the operators that show planned line `i`, for a caller
    /// that wraps several of them (the cells of a comb field) in one span of its own.
    pub fn bare_ops(&self, i: usize, size: f64, current: &mut String) -> Vec<TextOp> {
        self.lines.get(i).map(|line| self.run_ops(line, size, current)).unwrap_or_default()
    }

    fn run_ops(&self, line: &PlannedLine, size: f64, current: &mut String) -> Vec<TextOp> {
        let mut ops = Vec::new();
        let mut select = |ops: &mut Vec<TextOp>, font: &str| {
            if current != font {
                ops.push(TextOp::new("Tf", vec![Object::name(font), number(size)]));
                *current = font.to_owned();
            }
        };
        for run in &line.runs {
            match run {
                Run::Latin(shown, _) => {
                    select(&mut ops, &self.latin_font);
                    ops.extend(shown.iter().cloned());
                }
                Run::Shaped(font, codes, _) => {
                    let Some(name) = self.names.get(*font) else { continue };
                    select(&mut ops, name);
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
}
