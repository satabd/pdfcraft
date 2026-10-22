//! Arabic in FreeText annotations (text boxes, callouts, Typewriter, Fill & Sign text): the text is
//! drawn shaped as filled paths inside `/Span <</ActualText …>> BDC … EMC`, right-aligned when its
//! paragraph runs right to left and nothing states an alignment, with no `?` and no mark. Most tests
//! run with a stand-in for the craft-fonts Arabic face (one cluster per character, half an em wide,
//! a half-em square for an outline), so they run in any build; the last one uses the real face and
//! skips without it.

use pdfcraft_cos::{Dict, Document, Object, PdfString};
use pdfcraft_fonts::shaped_text::{Shaper, set_test_shaper};
use pdfcraft_fonts::{GlyphError, GlyphOutline, ShapedCluster, ShapedScript, helvetica_width};

use crate::tests::{fixture, list, meta, new, reopen};
use crate::*;

// ---- the stand-in face ----

fn fake_shape(script: ShapedScript, text: &str, rtl: bool) -> Result<Vec<ShapedCluster>, GlyphError> {
    if script != ShapedScript::Arabic {
        return Err(GlyphError::Missing);
    }
    let mut out: Vec<ShapedCluster> = text
        .chars()
        .map(|c| {
            let advance = if matches!(u32::from(c), 0x064B..=0x065F) { 0.0 } else { 0.5 };
            ShapedCluster { script, glyphs: vec![(u32::from(c), [0.0, 0.0])], advance, text: c.to_string() }
        })
        .collect();
    if rtl {
        out.reverse();
    }
    Ok(out)
}

fn fake_has(script: ShapedScript, c: char) -> bool {
    script == ShapedScript::Arabic && (c == ' ' || c.is_ascii_punctuation() || pdfcraft_fonts::shaped_text::is_arabic(c))
}

fn fake_outline(_: ShapedScript, id: u32) -> Result<GlyphOutline, GlyphError> {
    // One letter whose outline is too complex.
    if id == 0x06E5 {
        return Err(GlyphError::TooComplex);
    }
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

// ---- reading an appearance ----

const GREETING: &str = "مرحبا بالعالم";

/// One `/Span <</ActualText …>> BDC … EMC` of an appearance: its text, the x of every path point
/// and the Latin strings shown (with the x each starts at).
#[derive(Debug, Default)]
struct Span {
    actual: String,
    path_x: Vec<f64>,
    latin: Vec<(f64, String)>,
    /// How many invisible runs (render mode 3) the span holds, for text extraction to attach to.
    invisible: usize,
}

impl Span {
    fn left(&self) -> f64 {
        self.path_x.iter().copied().chain(self.latin.iter().map(|l| l.0)).fold(f64::INFINITY, f64::min)
    }

    fn path_right(&self) -> f64 {
        self.path_x.iter().copied().fold(f64::NEG_INFINITY, f64::max)
    }

    fn latin_text(&self) -> String {
        self.latin.iter().map(|l| l.1.as_str()).collect::<Vec<_>>().join("|")
    }
}

fn unhex(hex: &str) -> String {
    let units: Vec<u16> = (0..hex.len() / 4).filter_map(|i| u16::from_str_radix(hex.get(i * 4..i * 4 + 4)?, 16).ok()).collect();
    String::from_utf16_lossy(&units)
}

/// The spans of a content stream, and the whole stream as text.
fn read(stream: &pdfcraft_cos::Stream) -> (Vec<Span>, String) {
    let content = String::from_utf8_lossy(&stream.decoded().expect("decodes")).into_owned();
    let mut spans = Vec::new();
    let mut open: Option<Span> = None;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("/Span <</ActualText <FEFF") {
            let hex = rest.split('>').next().unwrap_or_default();
            open = Some(Span { actual: unhex(hex), ..Span::default() });
        } else if line == "EMC" {
            spans.extend(open.take());
        } else if let Some(span) = open.as_mut() {
            if line.starts_with("q 3 Tr ") {
                // The invisible space the span's text is attached to.
                span.invisible += 1;
                continue;
            }
            let words: Vec<&str> = line.split_whitespace().collect();
            if matches!(words.last(), Some(&"m") | Some(&"l")) {
                span.path_x.extend(words.first().and_then(|x| x.parse::<f64>().ok()));
            } else if let Some(at) = line.find(" Tm ") {
                let before: Vec<&str> = line[..at].split_whitespace().collect();
                let x = before.len().checked_sub(2).and_then(|i| before.get(i)).and_then(|x| x.parse::<f64>().ok()).unwrap_or(f64::NAN);
                let open_paren = line.find('(').unwrap_or(0);
                let close_paren = line.rfind(')').unwrap_or(0);
                span.latin.push((x, line.get(open_paren + 1..close_paren).unwrap_or_default().to_owned()));
            }
        }
    }
    (spans, content)
}

fn nums(v: &[f64]) -> Object {
    Object::Array(v.iter().map(|x| Object::Real(*x)).collect())
}

/// A FreeText dictionary in 12 pt Helvetica.
fn free_text(contents: &str, q: Option<i64>, rect: [f64; 4]) -> Dict {
    let mut d = Dict::new();
    d.set(b"Type".to_vec(), Object::name("Annot"));
    d.set(b"Subtype".to_vec(), Object::name("FreeText"));
    d.set(b"Rect".to_vec(), nums(&rect));
    d.set(b"Contents".to_vec(), PdfString::text(contents));
    d.set(b"DA".to_vec(), PdfString::literal(b"0 0 0 rg /Helv 12 Tf".to_vec()));
    if let Some(q) = q {
        d.set(b"Q".to_vec(), Object::Int(q));
    }
    d
}

const WIDE: [f64; 4] = [100.0, 600.0, 300.0, 640.0];

fn built(d: &Dict) -> (Vec<Span>, String) {
    read(&appearance::build(d).expect("a FreeText has an appearance"))
}

// ---- tests ----

#[test]
fn arabic_text_is_drawn_shaped_and_right_aligned() {
    let _face = Using::shaper(FAKE);
    let d = free_text(GREETING, None, WIDE);
    let (spans, content) = built(&d);
    assert_eq!(spans.len(), 1, "{content}");
    assert_eq!(spans[0].actual, GREETING);
    assert!(!content.contains('?'), "no placeholder is drawn: {content}");
    assert!(spans[0].latin.is_empty(), "all of it is paths: {content}");
    assert_eq!(spans[0].invisible, 1, "one invisible run carries the text: {content}");
    assert!(content.contains(" Tz "), "{content}");
    assert!(content.contains(" f\n") || content.contains("\nf\n"), "the glyphs are filled: {content}");
    // 13 characters of half an em at 12 pt, flush with the right edge less the 2 pt padding.
    let width = 13.0 * 0.5 * 12.0;
    assert!((spans[0].path_right() - (WIDE[2] - 2.0)).abs() < 0.01, "{:?}", spans[0]);
    assert!((spans[0].left() - (WIDE[2] - 2.0 - width)).abs() < 0.01, "{:?}", spans[0]);
    assert!(appearance::undrawable_text(&d).is_none(), "shaped text is not undrawable");
    // The text sits inside the box, on a baseline below its top.
    assert!(content.contains("re W n"), "clipped to the box: {content}");
}

#[test]
fn an_explicit_alignment_is_kept() {
    let _face = Using::shaper(FAKE);
    let width = 13.0 * 0.5 * 12.0;
    let (left, _) = built(&free_text(GREETING, Some(0), WIDE));
    assert!((left[0].left() - (WIDE[0] + 2.0)).abs() < 0.01, "/Q 0 stays left: {:?}", left[0]);
    let (centre, _) = built(&free_text(GREETING, Some(1), WIDE));
    let middle = (WIDE[0] + WIDE[2]) / 2.0;
    assert!((centre[0].left() - (middle - width / 2.0)).abs() < 0.01, "{:?}", centre[0]);
    let (right, _) = built(&free_text(GREETING, Some(2), WIDE));
    assert!((right[0].path_right() - (WIDE[2] - 2.0)).abs() < 0.01);
    // Left-to-right text without a /Q is left, as ever.
    let (latin_first, content) = built(&free_text("Hello", None, WIDE));
    assert!(latin_first.is_empty(), "plain Latin text is not shaped: {content}");
    assert!(content.contains("(Hello) Tj") && content.contains("1 0 0 1 102 "), "{content}");
}

#[test]
fn latin_and_digits_stay_in_place_next_to_arabic() {
    let _face = Using::shaper(FAKE);
    let typed = "رقم 42 Report";
    let (spans, content) = built(&free_text(typed, None, WIDE));
    assert_eq!(spans.len(), 1, "{content}");
    assert_eq!(spans[0].actual, typed);
    // UAX #9 in a right-to-left paragraph: the Latin word and the number read right to left as
    // words ("Report 42"), each left to right, to the left of the Arabic.
    assert_eq!(spans[0].latin_text(), "Report|42", "{content}");
    let latin_x: Vec<f64> = spans[0].latin.iter().map(|l| l.0).collect();
    assert!(latin_x[0] < latin_x[1], "{latin_x:?}");
    assert!(
        spans[0].path_x.iter().filter(|x| **x >= latin_x[1] + helvetica_width("42", 12.0) - 0.01).count() > 4,
        "the Arabic is right of the Latin: {:?}",
        spans[0]
    );
    // "رقم" and the two spaces between the words are shaped (5 clusters).
    let width = helvetica_width("Report", 12.0) + helvetica_width("42", 12.0) + 5.0 * 0.5 * 12.0;
    assert!((spans[0].left() - (WIDE[2] - 2.0 - width)).abs() < 0.01, "{:?} vs {width}", spans[0]);
    assert!((spans[0].path_right() - (WIDE[2] - 2.0)).abs() < 0.01, "{:?}", spans[0]);
    // A Latin paragraph with a little Arabic stays left-aligned.
    let (spans, _) = built(&free_text("Report رقم", None, WIDE));
    assert_eq!(spans[0].actual, "Report رقم");
    assert!((spans[0].left() - (WIDE[0] + 2.0)).abs() < 0.01, "{:?}", spans[0]);
}

#[test]
fn long_arabic_text_wraps_on_its_shaped_width() {
    let _face = Using::shaper(FAKE);
    let rect = [100.0, 500.0, 200.0, 640.0];
    let text = "مرحبا بالعالم الجميل في هذا اليوم المشمس الجميل\nسطر ثان";
    let (spans, content) = built(&free_text(text, None, rect));
    assert!(spans.len() >= 5, "wrapped into several lines: {content}");
    let room = rect[2] - rect[0] - 4.0;
    for span in &spans {
        let drawn = span.path_right() - span.left();
        assert!(drawn <= room + 0.01, "{drawn} fits {room}: {span:?}");
        assert!((span.path_right() - (rect[2] - 2.0)).abs() < 0.01, "every line is flush right: {span:?}");
    }
    // The lines, in typing order, are the words of the text.
    let words = |s: &str| s.split_whitespace().map(str::to_owned).collect::<Vec<_>>();
    let joined: Vec<String> = spans.iter().flat_map(|s| words(&s.actual)).collect();
    assert_eq!(joined, words(text));
    // Lines go down the page.
    let ys: Vec<f64> = content.lines().filter(|l| l.ends_with(" m")).filter_map(|l| l.split_whitespace().nth(1)?.parse().ok()).collect();
    assert!(ys.windows(2).all(|w| w[1] <= w[0] + 1e-9), "{ys:?}");
}

#[test]
fn hostile_text_never_panics() {
    let _face = Using::shaper(FAKE);
    let long = "ب".repeat(20_000);
    let clusters: String = (0x0620..0x0670).map(|u| char::from_u32(u).unwrap_or('ب')).collect::<String>().repeat(100);
    let hostile: Vec<&str> =
        vec!["", "\u{064E}\u{064E}", "\u{202E}\u{200F}\u{061C}", "ب\0ا", "\u{FDFD}", "ب\u{06E5}ب", "ب日本", "\r\n\r\n", &long, &clusters];
    for text in hostile {
        for q in [None, Some(0), Some(1), Some(2), Some(99), Some(-3)] {
            for rect in [WIDE, [0.0, 0.0, 3.0, 3.0], [10.0, 10.0, 10.0, 10.0]] {
                let d = free_text(text, q, rect);
                let _ = appearance::undrawable_text(&d);
                if let Some(stream) = appearance::build(&d) {
                    assert!(stream.decoded().is_ok());
                }
            }
        }
    }
    // Through the document too: adding, saving and reopening.
    let mut doc = fixture();
    for text in ["\u{064E}\u{064E}", "ب\u{06E5}ب", &long] {
        let mut n = new(0, Shape::TextBox { rect: [100.0, 600.0, 300.0, 640.0], font_size: 12.0 });
        n.contents = text.to_owned();
        let _ = add_annotation(&mut doc, &n, &meta("hostile"));
    }
    let _ = reopen(&doc);
}

#[test]
fn text_the_face_or_the_latin_font_cannot_draw_keeps_the_marked_placeholder() {
    let _face = Using::shaper(FAKE);
    // A letter whose outline is too complex, and text in a script neither font has.
    for (text, missing) in [("ب\u{06E5}ب", None), ("مرحبا 日本", Some('日'))] {
        let d = free_text(text, None, WIDE);
        let (spans, content) = built(&d);
        assert!(spans.is_empty(), "{text}: {content}");
        assert!(content.contains('?'), "{text}: {content}");
        let reported = appearance::undrawable_text(&d);
        assert!(reported.is_some(), "{text}");
        if let Some(c) = missing {
            assert_eq!(reported.map(|r| r.1), Some(c));
        }
    }
}

#[test]
fn without_the_arabic_face_the_text_is_drawn_with_question_marks_and_marked() {
    let _face = Using::shaper(NO_FACE);
    let d = free_text(GREETING, None, WIDE);
    let (spans, content) = built(&d);
    assert!(spans.is_empty(), "{content}");
    assert!(content.contains("(????? ???????) Tj"), "{content}");
    assert_eq!(appearance::undrawable_text(&d).map(|r| r.1), Some('م'));
    // Added to a page, the annotation is marked, as before.
    let mut doc = fixture();
    let mut n = new(0, Shape::TextBox { rect: [100.0, 600.0, 300.0, 640.0], font_size: 12.0 });
    n.contents = GREETING.into();
    let index = add_annotation(&mut doc, &n, &meta("t")).unwrap();
    let all = list(&doc, 0);
    assert_eq!(pdfcraft_fonts::undrawn_mark(&all[index]), Some('م'));
}

#[test]
fn added_text_boxes_and_typewriter_text_are_shaped_and_unmarked() {
    let _face = Using::shaper(FAKE);
    let rect = [100.0, 600.0, 300.0, 640.0];
    for shape in [Shape::TextBox { rect, font_size: 12.0 }, Shape::Typewriter { rect, font_size: 12.0 }] {
        let mut doc = fixture();
        let mut n = new(0, shape);
        n.contents = GREETING.into();
        let index = add_annotation(&mut doc, &n, &meta("t")).unwrap();
        let doc = reopen(&doc);
        let all = list(&doc, 0);
        let d = &all[index];
        assert_eq!(pdfcraft_fonts::undrawn_mark(d), None, "shaped text is not marked");
        assert!(!d.contains(b"Q"), "no alignment is stated for right-to-left text, so it is right-aligned");
        let ap = d.get(b"AP").and_then(Object::as_dict).and_then(|ap| ap.reference(b"N")).expect("an appearance");
        let Object::Stream(stream) = &*doc.get(ap) else { panic!("a stream") };
        let (spans, content) = read(stream);
        assert_eq!(spans.len(), 1, "{content}");
        assert_eq!(spans[0].actual, GREETING);
        // The 2 pt border and 2 pt padding.
        assert!((spans[0].path_right() - (rect[2] - 4.0)).abs() < 0.01, "{:?}", spans[0]);
        // Rich text says which way the paragraph runs for editors that read it.
        let rc = d.get(b"RC").and_then(Object::as_string).map(PdfString::to_text).unwrap_or_default();
        assert!(rc.contains("dir=\"rtl\"") && rc.contains("text-align:right"), "{rc}");
        // A restyle redraws it shaped again.
        let mut doc = doc;
        let page = page_refs(&doc).unwrap()[0];
        let r = annots(&doc, page)[index].as_ref().unwrap();
        set_appearance(&mut doc, r).unwrap();
        let after = list(&doc, 0);
        assert_eq!(pdfcraft_fonts::undrawn_mark(&after[index]), None);
    }
    // Latin text keeps the explicit left alignment it always had.
    let mut doc = fixture();
    let index = add_annotation(&mut doc, &new(0, Shape::TextBox { rect, font_size: 12.0 }), &meta("l")).unwrap();
    assert_eq!(list(&doc, 0)[index].int(b"Q"), Some(0));
}

#[test]
fn the_real_arabic_face_draws_a_free_text() {
    if pdfcraft_fonts::document_arabic_font().is_none() {
        eprintln!("set CRAFT_FONTS_DIR to exercise Arabic in FreeText annotations");
        return;
    }
    let typed = "رقم 42 (Report) سَلَام";
    let d = free_text(typed, None, WIDE);
    let (spans, content) = built(&d);
    assert_eq!(spans.len(), 1, "{content}");
    assert_eq!(spans[0].actual, typed);
    assert!(!content.contains('?'));
    assert!(appearance::undrawable_text(&d).is_none());
    assert!((spans[0].path_right() - (WIDE[2] - 2.0)).abs() < 3.0, "flush right (within the last glyph's side bearing): {:?}", spans[0]);
    assert!(spans[0].left() > WIDE[0], "{:?}", spans[0]);
    assert!(spans[0].latin_text().contains("42") && spans[0].latin_text().contains("Report"), "{:?}", spans[0]);
    assert!(spans[0].path_x.len() > 100, "real outlines have many points");
    // Wrapped on real widths.
    let rect = [100.0, 500.0, 200.0, 640.0];
    let (rows, _) = built(&free_text("مرحبا بالعالم الجميل في هذا اليوم المشمس الجميل", None, rect));
    assert!(rows.len() >= 3);
    assert!(rows.iter().all(|r| r.path_right() - r.left() <= 96.0 + 3.0), "{rows:?}");
    // Hostile input with the real face: no panic, whatever is drawn.
    for text in ["\u{064E}\u{064E}", "\u{202E}\u{200F}\u{061C}", "ب\0ا", "\u{FDFD}", &"ب".repeat(20_000)] {
        let d = free_text(text, None, WIDE);
        let _ = appearance::undrawable_text(&d);
        let _ = appearance::build(&d);
    }
    // And it survives a save.
    let mut doc: Document = fixture();
    let mut n = new(0, Shape::TextBox { rect: WIDE, font_size: 12.0 });
    n.contents = typed.into();
    let index = add_annotation(&mut doc, &n, &meta("real")).unwrap();
    let doc = reopen(&doc);
    assert_eq!(pdfcraft_fonts::undrawn_mark(&list(&doc, 0)[index]), None);
}

#[test]
fn undrawable_text_agrees_with_what_is_drawn() {
    let _face = Using::shaper(FAKE);
    let at_limit = "ب ".repeat(750);
    let over_limit = "ب ".repeat(751);
    let cases = [GREETING, "رقم 42 Report", "Plain", "ب\u{06E5}ب", "مرحبا 日本", "\u{064E}", "مرحبا\nHello\nبالعالم", &at_limit, &over_limit, ""];
    for text in cases {
        let d = free_text(text, None, [0.0, 0.0, 400.0, 20_000.0]);
        let (spans, content) = built(&d);
        let unmarked = appearance::undrawable_text(&d).is_none();
        assert_eq!(unmarked, !content.contains('?'), "{text:.20}: the mark says whether a `?` was drawn");
        assert_eq!(unmarked && pdfcraft_fonts::shaped_text::has_arabic(text), !spans.is_empty(), "{text:.20}: Arabic that is not marked is shaped");
    }
    // The longest shaped text is one span per line, not one per character.
    let (spans, _) = built(&free_text(&at_limit, None, [0.0, 0.0, 400.0, 20_000.0]));
    assert!(spans.len() > 20 && spans.iter().all(|s| !s.actual.is_empty()));
}

#[test]
fn a_callouts_arabic_text_is_shaped_inside_its_text_box() {
    let _face = Using::shaper(FAKE);
    let tb = [100.0, 600.0, 300.0, 640.0];
    let shape = Shape::Callout { rect: tb, knee: [60.0, 620.0], point: [20.0, 700.0], font_size: 12.0, ending: LineEnding::OpenArrow };
    let mut doc = fixture();
    let mut n = new(0, shape);
    n.contents = GREETING.into();
    let index = add_annotation(&mut doc, &n, &meta("callout")).unwrap();
    let doc = reopen(&doc);
    let all = list(&doc, 0);
    assert_eq!(pdfcraft_fonts::undrawn_mark(&all[index]), None);
    let ap = all[index].get(b"AP").and_then(Object::as_dict).and_then(|ap| ap.reference(b"N")).expect("an appearance");
    let Object::Stream(stream) = &*doc.get(ap) else { panic!("a stream") };
    let (spans, content) = read(stream);
    assert_eq!(spans.len(), 1, "{content}");
    assert_eq!(spans[0].actual, GREETING);
    // Flush right inside the text box (less the border and padding), not the leader's bounding box.
    assert!((spans[0].path_right() - (tb[2] - 4.0)).abs() < 0.01, "{:?}", spans[0]);
    assert!(content.contains(" l\nS\n") || content.contains("S\n"), "the leader is still drawn: {content}");
}

// ---- stamp "By" lines and measurement labels ----

/// The dictionary of a stamp added to the first page of the fixture (a dynamic stamp, which has a
/// "By" line) and read back.
fn stamp_dict(by: Option<&str>) -> Dict {
    let mut doc = fixture();
    let shape = Shape::Stamp { rect: [100.0, 100.0, 260.0, 142.0], stamp: StampKind::DynApproved, by: by.map(str::to_owned) };
    let mut n = new(0, shape);
    n.author = "A".into();
    let index = add_annotation(&mut doc, &n, &meta("stamp")).unwrap();
    let doc = reopen(&doc);
    list(&doc, 0)[index].clone()
}

/// The dictionary of a line annotation with a measurement label (as the measure tools write it).
fn measured_line(label: &str) -> Dict {
    let mut doc = fixture();
    let n = new(0, Shape::Line { from: [100.0, 400.0], to: [300.0, 400.0], start: LineEnding::None, end: LineEnding::None });
    let index = add_annotation(&mut doc, &n, &meta("measure")).unwrap();
    let mut d = list(&doc, 0)[index].clone();
    d.set(b"PCMeasureValue".to_vec(), PdfString::text(label));
    d
}

fn rect_of(d: &Dict) -> [f64; 4] {
    let v: Vec<f64> = d.get(b"Rect").and_then(Object::as_array).unwrap().iter().filter_map(Object::as_f64).collect();
    [v[0], v[1], v[2], v[3]]
}

#[test]
fn an_arabic_by_line_on_a_stamp_is_shaped_centred_and_unmarked() {
    let _face = Using::shaper(FAKE);
    for by in ["أحمد", "By أحمد at 2:14 pm, Oct 02, 2026"] {
        let d = stamp_dict(Some(by));
        assert_eq!(pdfcraft_fonts::undrawn_mark(&d), None, "{by}: shaped text is not marked");
        assert_eq!(appearance::undrawable_text(&d), None, "{by}");
        let (spans, content) = built(&d);
        assert_eq!(spans.len(), 1, "{by}: {content}");
        assert_eq!(spans[0].actual, by, "{by}: the span holds the typed text");
        assert!(spans[0].invisible >= 1, "{by}: text extraction has a glyph to attach the span to");
        assert!(!content.contains('?'), "{by}: {content}");
        // The label is still the stamp's own, in bold Helvetica; the By line is centred under it.
        assert!(content.contains("(APPROVED) Tj"), "{content}");
        let rect = rect_of(&d);
        let centre = (rect[0] + rect[2]) / 2.0;
        // (Only Arabic is measured by its paths; the Latin runs of a mixed line are text.)
        if by == "أحمد" {
            assert!(((spans[0].left() + spans[0].path_right()) / 2.0 - centre).abs() < 0.5, "{by}: {:?} around {centre}", spans[0]);
        } else {
            assert_eq!(spans[0].latin_text(), "By | at 2:14 pm, Oct 02, 2026");
        }
    }
    // A stamp's line too long for the stamp is shrunk to fit, as Latin text is.
    let d = stamp_dict(Some(&"أحمد ".repeat(30)));
    let (spans, _) = built(&d);
    assert_eq!(spans.len(), 1);
    let rect = rect_of(&d);
    assert!(spans[0].left() >= rect[0] && spans[0].path_right() <= rect[2], "{:?} in {rect:?}", spans[0]);
}

#[test]
fn latin_by_lines_and_stamps_without_one_are_drawn_as_before() {
    let _face = Using::shaper(FAKE);
    for by in [Some("By Ada at 2:14 pm"), None] {
        let d = stamp_dict(by);
        let (spans, content) = built(&d);
        assert!(spans.is_empty(), "{content}");
        assert_eq!(content.contains("(By Ada at 2:14 pm) Tj"), by.is_some(), "{content}");
        assert_eq!(appearance::undrawable_text(&d), None);
    }
}

#[test]
fn without_the_face_an_arabic_by_line_is_drawn_with_question_marks_and_marked() {
    let _face = Using::shaper(NO_FACE);
    let d = stamp_dict(Some("أحمد"));
    let (spans, content) = built(&d);
    assert!(spans.is_empty());
    assert!(content.contains("(????) Tj"), "{content}");
    assert_eq!(appearance::undrawable_text(&d).map(|r| r.1), Some('أ'));
    assert_eq!(pdfcraft_fonts::undrawn_mark(&d), Some('أ'));
}

#[test]
fn an_arabic_measurement_label_is_shaped_and_centred() {
    let _face = Using::shaper(FAKE);
    let label = "١٢٫٥ سم";
    let d = measured_line(label);
    assert_eq!(appearance::undrawable_text(&d), None);
    let (spans, content) = built(&d);
    assert_eq!(spans.len(), 1, "{content}");
    assert_eq!(spans[0].actual, label);
    assert!(!content.contains('?'), "{content}");
    let rect = rect_of(&d);
    let centre = (rect[0] + rect[2]) / 2.0;
    assert!(((spans[0].left() + spans[0].path_right()) / 2.0 - centre).abs() < 0.5, "{:?} around {centre}", spans[0]);
    // A Latin label is drawn as before.
    let d = measured_line("12.5 cm");
    let (spans, content) = built(&d);
    assert!(spans.is_empty() && content.contains("(12.5 cm) Tj"), "{content}");
    // Without the face the label is `?` and the caller marks it, as before.
    let _none = Using::shaper(NO_FACE);
    let d = measured_line(label);
    assert!(appearance::undrawable_text(&d).is_some());
    assert!(built(&d).1.contains('?'));
}

#[test]
fn hostile_labels_never_panic_and_the_mark_agrees_with_the_drawing() {
    let _face = Using::shaper(FAKE);
    let long = "ب".repeat(5000);
    let texts = ["", "\u{064E}\u{064E}", "\u{202E}\u{200F}\u{061C}", "ب\0ا", "ب\u{06E5}ب", "ب日本", "أ\nب\rج", &long, "١٢٫٥ سم", "12 م²"];
    for text in texts {
        // A measurement label over 256 characters is not drawn at all.
        let mut dicts = vec![stamp_dict(Some(text))];
        if text.chars().count() <= 256 {
            dicts.push(measured_line(text));
        }
        for d in dicts {
            let (spans, content) = built(&d);
            let unmarked = appearance::undrawable_text(&d).is_none();
            assert_eq!(unmarked, !content.contains('?'), "{text:.20}: the mark says whether a `?` was drawn");
            assert_eq!(
                unmarked && pdfcraft_fonts::shaped_text::has_arabic(text),
                !spans.is_empty(),
                "{text:.20}: Arabic that is not marked is shaped"
            );
        }
    }
}

#[test]
fn the_real_arabic_face_draws_a_by_line_and_a_label() {
    if pdfcraft_fonts::document_arabic_font().is_none() {
        eprintln!("set CRAFT_FONTS_DIR to exercise Arabic in stamps and measurements");
        return;
    }
    let d = stamp_dict(Some("أحمد"));
    assert_eq!(pdfcraft_fonts::undrawn_mark(&d), None);
    let (spans, _) = built(&d);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].actual, "أحمد");
    let d = measured_line("١٢٫٥ سم");
    assert_eq!(appearance::undrawable_text(&d), None);
    let (spans, _) = built(&d);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].actual, "١٢٫٥ سم");
}
