//! Arabic in text made into a PDF (`text_shaped`). Most tests run with a stand-in for the
//! craft-fonts Arabic face (one boxed glyph per character), so they run in any build; the one at
//! the end uses the real face and skips without it.

use pdfcraft_fonts::shaped_text::{Shaper, set_test_shaper};
use pdfcraft_fonts::{GlyphError, GlyphOutline, ShapedCluster, ShapedScript, helvetica_width};

use super::*;
use crate::tests::{pages, reopen};

// ---- the stand-in face ----

fn fake_shape(script: ShapedScript, text: &str, rtl: bool) -> Result<Vec<ShapedCluster>, GlyphError> {
    if text.contains('\u{FDFD}') {
        return Err(GlyphError::Missing);
    }
    let mut out: Vec<ShapedCluster> = text
        .chars()
        .map(|c| ShapedCluster { script, glyphs: vec![(u32::from(c), [0.0, 0.0])], advance: if c == ' ' { 0.25 } else { 0.5 }, text: c.to_string() })
        .collect();
    if rtl {
        out.reverse();
    }
    Ok(out)
}

fn fake_has(script: ShapedScript, c: char) -> bool {
    script == ShapedScript::Arabic && (c == ' ' || c.is_ascii_punctuation() || pdfcraft_fonts::shaped_text::is_arabic(c))
}

fn fake_outline(_: ShapedScript, _: u32) -> Result<GlyphOutline, GlyphError> {
    Ok(GlyphOutline { contours: vec![vec![[0.0, 0.0], [0.5, 0.0], [0.5, 0.5], [0.0, 0.5]]], width: 0.5, bbox: [0.0, 0.0, 0.5, 0.5] })
}

const FAKE: Shaper = Shaper { shape: fake_shape, has: fake_has, outline: fake_outline, arabic_available: || true };

fn no_shape(_: ShapedScript, _: &str, _: bool) -> Result<Vec<ShapedCluster>, GlyphError> {
    Err(GlyphError::NoFont)
}

fn no_outline(_: ShapedScript, _: u32) -> Result<GlyphOutline, GlyphError> {
    Err(GlyphError::NoFont)
}

/// A build with no Arabic face.
const NO_FACE: Shaper = Shaper { shape: no_shape, has: |_, _| false, outline: no_outline, arabic_available: || false };

/// Uses a shaper on this thread until dropped.
struct Using;

impl Using {
    fn shaper(shaper: Shaper) -> Using {
        set_test_shaper(Some(shaper));
        Using
    }
}

impl Drop for Using {
    fn drop(&mut self) {
        set_test_shaper(None);
    }
}

// ---- reading what was written ----

/// One line as written: where its text matrix puts it, how far it runs, and its `/ActualText`.
#[derive(Debug)]
struct Written {
    x: f64,
    extent: f64,
    actual: Option<String>,
    /// The fonts it shows text with.
    fonts: Vec<String>,
    /// Latin text shown in the body font.
    latin: String,
}

fn content(doc: &Document, page: usize) -> String {
    let p = &pages(doc)[page];
    let Object::Stream(s) = &*doc.resolve(p.get(b"Contents").expect("contents")) else { panic!("a stream") };
    String::from_utf8_lossy(&s.decoded().expect("decodes")).into_owned()
}

fn page_fonts(doc: &Document, page: usize) -> Dict {
    let p = &pages(doc)[page];
    let res = doc.resolve(p.get(b"Resources").expect("resources")).as_dict().cloned().expect("dict");
    doc.resolve(res.get(b"Font").expect("fonts")).as_dict().cloned().expect("font dict")
}

/// The bytes of a string operand: hex, or literal with its escapes undone.
fn literal_bytes(token: &str) -> Vec<u8> {
    if let Some(hex) = token.strip_prefix('<').and_then(|t| t.strip_suffix('>')) {
        return (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex")).collect();
    }
    let inner = token.strip_prefix('(').and_then(|t| t.strip_suffix(')')).expect("a literal string");
    let b = inner.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' {
            out.push(b[i]);
            i += 1;
            continue;
        }
        match b.get(i + 1) {
            Some(d) if d.is_ascii_digit() => {
                let oct = std::str::from_utf8(&b[i + 1..i + 4]).expect("octal");
                out.push(u8::from_str_radix(oct, 8).expect("an octal code"));
                i += 4;
            }
            Some(b'n') => (out.push(b'\n'), i += 2).0,
            Some(b'r') => (out.push(b'\r'), i += 2).0,
            Some(b't') => (out.push(b'\t'), i += 2).0,
            Some(c) => (out.push(*c), i += 2).0,
            None => break,
        }
    }
    out
}

fn utf16_hex(hex: &str) -> String {
    let units: Vec<u16> = (0..hex.len()).step_by(4).map(|i| u16::from_str_radix(&hex[i..i + 4], 16).expect("hex")).collect();
    String::from_utf16(&units).expect("utf-16")
}

/// The lines (one per `Tm`) of page `page`, widths read from the fonts' own `/Widths` and
/// Helvetica's metrics: independent of how the writer measured.
fn written(doc: &Document, page: usize) -> Vec<Written> {
    let fonts = page_fonts(doc, page);
    let mut out: Vec<Written> = Vec::new();
    let (mut font, mut size) = (String::new(), 0.0);
    for line in content(doc, page).lines() {
        if let Some(args) = line.strip_suffix(" Tm") {
            let n: Vec<f64> = args.split(' ').map(|v| v.parse().unwrap_or_else(|_| panic!("a number in {line:?}"))).collect();
            out.push(Written { x: n[4], extent: 0.0, actual: None, fonts: Vec::new(), latin: String::new() });
        } else if let Some(args) = line.strip_suffix(" Tf") {
            let mut parts = args.trim_start_matches("BT ").split(' ');
            font = parts.next().expect("a font").trim_start_matches('/').to_owned();
            size = parts.next().expect("a size").parse().expect("a number");
        } else if line.ends_with(" BDC") {
            let at = line.find("<FEFF").expect("ActualText");
            let hex = &line[at + 5..at + 5 + line[at + 5..].find('>').expect("closed")];
            out.last_mut().expect("Tm first").actual = Some(utf16_hex(hex));
        } else if let Some(token) = line.strip_suffix(" Tj").filter(|t| t.starts_with(['(', '<'])) {
            let bytes = literal_bytes(token);
            let w = out.last_mut().expect("Tm first");
            if font == "F1" {
                let text: String = bytes.iter().map(|b| char::from(*b)).collect();
                w.extent += helvetica_width(&text, size);
                w.latin.push_str(&text);
            } else {
                let f = doc.resolve(fonts.get(font.as_bytes()).unwrap_or_else(|| panic!("font {font} in the resources")));
                let widths = f.as_dict().and_then(|d| d.get(b"Widths")).map(|w| doc.resolve(w)).and_then(|w| w.as_array().cloned()).expect("Widths");
                w.extent += bytes.iter().map(|c| widths[usize::from(*c) - 1].as_f64().expect("a width") / 1000.0 * size).sum::<f64>();
            }
            w.fonts.push(font.clone());
        }
    }
    out
}

fn actual_texts(doc: &Document, page: usize) -> Vec<String> {
    written(doc, page).into_iter().filter_map(|l| l.actual).collect()
}

// ---- tests ----

const PARAGRAPH: &str = "مرحبا بالعالم هذا نص عربي طويل يكفي ليلتف على عدة أسطر داخل الصفحة لأنه يحتوي على كلمات كثيرة جدا ومتنوعة وكل كلمة منها \
                         تأخذ مكانها من السطر حتى ينتهي السطر ويبدأ سطر جديد في الفقرة نفسها ثم تنتهي الفقرة بكلمة أخيرة";

#[test]
fn an_arabic_paragraph_is_wrapped_on_shaped_widths_right_aligned_and_reads_back() {
    let _face = Using::shaper(FAKE);
    let doc = reopen(&from_text("t", PARAGRAPH, LETTER, 11.0).expect("shaped, not refused"));
    let lines = written(&doc, 0);
    assert!(lines.len() >= 3, "wrapped: {lines:?}");
    // Every line is a span in typing order; joined, they are the paragraph again.
    let actual = actual_texts(&doc, 0);
    assert_eq!(actual.len(), lines.len());
    assert_eq!(actual.join(" "), PARAGRAPH);
    // Wrapped on the shaped width (468 pt of text), and each line ends at the right margin.
    for line in &lines {
        assert!(line.extent <= 468.0 + 0.01, "{line:?}");
        assert!((line.x + line.extent - 540.0).abs() < 0.01, "right-aligned: {line:?}");
        assert!(line.fonts.iter().all(|f| f.starts_with("PCTxAr")), "{line:?}");
    }
    // The first lines are nearly full: wrapping used the width, not a Helvetica estimate.
    assert!(lines[0].extent > 400.0, "{lines:?}");
    // The generated fonts map back to Unicode and are in the page's resources.
    let fonts = page_fonts(&doc, 0);
    assert!(fonts.contains(b"F1"));
    let shaped: Vec<_> = fonts.iter().filter(|(n, _)| n.starts_with(b"PCTxAr")).collect();
    assert!(!shaped.is_empty());
    for (name, font) in shaped {
        let font = doc.resolve(font);
        let font = font.as_dict().expect("font");
        assert_eq!(font.name(b"Subtype"), Some(b"Type3".as_slice()), "{}", String::from_utf8_lossy(name));
        assert!(font.get(b"ToUnicode").is_some());
    }
}

#[test]
fn a_left_to_right_paragraph_with_arabic_starts_at_the_margin_and_plain_pages_are_unchanged() {
    let _face = Using::shaper(FAKE);
    let text = "Title: مرحبا\nplain (line)\n\u{c}Second page, Latin only";
    let doc = reopen(&from_text("t", text, LETTER, 11.0).expect("shaped"));
    let lines = written(&doc, 0);
    // Three lines on the page: the Arabic one, a plain line and the empty one the last line break ends.
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(lines[2].latin.is_empty() && lines[2].actual.is_none(), "{lines:?}");
    assert!((lines[0].x - 72.0).abs() < 0.01 && lines[0].actual.as_deref() == Some("Title: مرحبا"), "{lines:?}");
    // Latin pieces of the line are set in the body font, between shaped runs, in the same span.
    assert!(lines[0].latin.starts_with("Title:"), "{lines:?}");
    // A plain line on a page with shaped text is set at the margin in the body font.
    assert!((lines[1].x - 72.0).abs() < 0.01 && lines[1].latin == "plain (line)" && lines[1].actual.is_none(), "{lines:?}");
    // The page without Arabic is written as it always was.
    let second = content(&doc, 1);
    assert!(second.starts_with("BT /F1 11 Tf 13.750 TL 72 ") && second.contains("(Second page, Latin only) Tj T*"), "{second}");
    assert!(page_fonts(&doc, 1).iter().all(|(n, _)| !n.starts_with(b"PCTxAr")));
}

#[test]
fn text_without_arabic_is_written_exactly_as_before() {
    let _face = Using::shaper(FAKE);
    let doc = reopen(&from_text("t", "    indented\n\ttabbed\nplain", LETTER, 11.0).expect("converts"));
    let c = content(&doc, 0);
    assert!(c.contains("(    indented) Tj T*") && c.contains("(    tabbed) Tj T*") && c.contains("(plain) Tj T*"), "{c}");
    assert!(!c.contains("Tm") && !c.contains("BDC"), "{c}");
}

#[test]
fn arabic_next_to_text_nothing_can_draw_is_still_refused_with_its_line() {
    let _face = Using::shaper(FAKE);
    for (text, line, c) in [("fine\nمرحبا Ελληνικά", 2, 'Ε'), ("مرحبا 日本", 1, '日'), ("مرحبا שלום", 1, 'ש')] {
        let err = from_text("t", text, LETTER, 11.0).unwrap_err();
        let CreateError::Invalid(msg) = &err else { panic!("{err:?}") };
        assert!(msg.contains(&format!("(line {line})")) && msg.contains(&format!("(U+{:04X})", u32::from(c))), "{msg}");
    }
}

#[test]
fn without_the_face_arabic_is_refused_or_escaped_as_before() {
    let _face = Using::shaper(NO_FACE);
    let err = from_text("t", "ok\nمرحبا", LETTER, 11.0).unwrap_err();
    let CreateError::Invalid(msg) = &err else { panic!("{err:?}") };
    assert!(msg.contains("(line 2)") && msg.contains("\"م\" (U+0645)") && msg.contains("Western European"), "{msg}");
    let (doc, escaped) = from_text_escaped("t", "مرحبا ok", LETTER, 11.0).expect("escaped");
    assert_eq!(escaped, 5);
    let c = content(&reopen(&doc), 0);
    assert!(c.contains("[U+0645][U+0631][U+062D][U+0628][U+0627] ok") && c.contains("U+XXXX") && !c.contains("BDC"), "{c}");
}

#[test]
fn generated_reports_keep_arabic_readable_and_escape_the_rest() {
    let _face = Using::shaper(FAKE);
    // Nothing to escape: no note, and the Arabic is shaped.
    let (doc, escaped) = from_text_escaped("Summary", "Comment: مرحبا بالعالم\nPage 2", LETTER, 11.0).expect("shaped");
    assert_eq!(escaped, 0);
    let doc = reopen(&doc);
    assert_eq!(actual_texts(&doc, 0), ["Comment: مرحبا بالعالم"]);
    let c = content(&doc, 0);
    assert!(!c.contains("U+XXXX") && !c.contains("[U+0645]") && c.contains("(Page 2) Tj"), "{c}");
    // Characters the faces lack are still written as code points, in a paragraph that is shaped.
    let (doc, escaped) = from_text_escaped("S", "مرحبا 签名\nΩ plain 日", LETTER, 11.0).expect("shaped");
    assert_eq!(escaped, 4);
    let doc = reopen(&doc);
    let c = content(&doc, 0);
    assert!(c.contains(ESCAPED_NOTE), "{c}");
    let texts = actual_texts(&doc, 0);
    assert_eq!(texts, ["مرحبا [U+7B7E][U+540D]"]);
    assert!(c.contains("[U+03A9] plain [U+65E5]"), "{c}");
    // Direction marks stay with the paragraphs they steer, and are escaped in the others.
    let (_, escaped) = from_text_escaped("S", "ا\u{200F}ب\nplain\u{200F}", LETTER, 11.0).expect("converts");
    assert_eq!(escaped, 1);
}

#[test]
fn arabic_the_planner_cannot_lay_out_falls_back_to_code_points_in_reports() {
    let _face = Using::shaper(FAKE);
    // The stand-in can't shape U+FDFD: the report is written with the Arabic as code points.
    let text = "ok مرحبا \u{FDFD}";
    assert!(from_text("t", text, LETTER, 11.0).is_err());
    let (doc, escaped) = from_text_escaped("t", text, LETTER, 11.0).expect("not lost");
    assert_eq!(escaped, 6);
    let c = content(&reopen(&doc), 0);
    assert!(c.contains("[U+0645]") && c.contains("[U+FDFD]") && c.contains(ESCAPED_NOTE) && !c.contains("BDC"), "{c}");
}

#[test]
fn odd_and_huge_arabic_text_never_panics_and_stays_bounded() {
    let _face = Using::shaper(FAKE);
    for text in [
        "\u{064B}",
        "\u{202E}ب\u{064B}\u{064B} (]",
        "ا\u{200F}\u{2067}b\u{2069}",
        "ا\n\nب\tc\r\nd\u{c}\u{c}ه",
        "(((ا)))",
        " ا",
        "ا ".repeat(5).as_str(),
    ] {
        for size in [f64::NAN, 4.0, 72.0] {
            if let Ok(doc) = from_text("t", text, (100.0, 100.0), size) {
                reopen(&doc);
            }
            let (doc, _) = from_text_escaped("t", text, (100.0, 100.0), size).expect("reports are never refused");
            reopen(&doc);
        }
    }
    // A long text with no line breaks and a long word are cut and planned in groups of pages.
    let word = "ب".repeat(60_000);
    let doc = reopen(&from_text("t", &word, LETTER, 11.0).expect("shaped in pieces"));
    let count = pages(&doc).len();
    assert!(count > 4, "{count} pages");
    let mut fonts = std::collections::HashSet::new();
    for page in 0..count {
        for line in written(&doc, page) {
            assert!(line.extent <= 468.0 + 0.01, "{line:?}");
        }
        fonts.extend(page_fonts(&doc, page).iter().filter(|(n, _)| n.starts_with(b"PCTxAr")).filter_map(|(_, f)| match f {
            Object::Ref(r) => Some(r.num),
            _ => None,
        }));
    }
    // Several groups of pages, each with fonts of its own.
    assert!(fonts.len() > 1, "{} fonts", fonts.len());
    // And together they read back as the word.
    let back: String = (0..count).flat_map(|p| actual_texts(&doc, p)).collect();
    assert_eq!(back, word);
}

/// With the real craft-fonts Arabic face.
#[test]
fn the_real_face_sets_an_arabic_paragraph() {
    if pdfcraft_fonts::document_arabic_font().is_none() {
        eprintln!("skipping the real-face checks: built without a craft-fonts Arab face (set CRAFT_FONTS_DIR)");
        return;
    }
    let doc = reopen(&from_text("t", PARAGRAPH, LETTER, 11.0).expect("shaped"));
    let lines = written(&doc, 0);
    assert!(lines.len() >= 2, "{lines:?}");
    assert_eq!(actual_texts(&doc, 0).join(" "), PARAGRAPH);
    for line in &lines {
        assert!(line.extent <= 468.0 + 0.5 && (line.x + line.extent - 540.0).abs() < 0.01, "{line:?}");
    }
    // A comment summary that quotes Arabic is readable, not code points.
    let (doc, escaped) = from_text_escaped("Summary", "Comment: مرحبا بالعالم", LETTER, 11.0).expect("shaped");
    assert_eq!(escaped, 0);
    assert_eq!(actual_texts(&reopen(&doc), 0), ["Comment: مرحبا بالعالم"]);
}
