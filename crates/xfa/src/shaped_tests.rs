//! Arabic in a dynamic form's own text (draws, captions, read-only values): laid out on shaped
//! widths, right-aligned when the paragraph runs right to left unless the template states an
//! alignment, and drawn from generated Type 3 fonts with one `/ActualText` span a line. Most
//! tests run with a stand-in for the craft-fonts Arabic face (one cluster per character, half an
//! em wide), so they run in any build; the one at the end uses the real face and skips without it.

use std::sync::Arc;

use pdfcraft_cos::{Dict, Document, Object};
use pdfcraft_fonts::shaped_text::{Shaper, set_test_shaper};
use pdfcraft_fonts::{GlyphError, GlyphOutline, ShapedCluster, ShapedScript};

use super::*;
use crate::fixtures::shell;
use crate::layout::Item;

// ---- the stand-in face ----

fn fake_shape(script: ShapedScript, text: &str, rtl: bool) -> Result<Vec<ShapedCluster>, GlyphError> {
    if script != ShapedScript::Arabic {
        return Err(GlyphError::Missing);
    }
    let mut out: Vec<ShapedCluster> =
        text.chars().map(|c| ShapedCluster { script, glyphs: vec![(u32::from(c), [0.0, 0.0])], advance: 0.5, text: c.to_string() }).collect();
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

// ---- forms ----

const GREETING: &str = "مرحبا بالعالم";

/// A form of draws, each 3 in wide (216 pt) at the left of the content area (x = 36 pt).
fn form_of(draws: &str) -> String {
    format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<xdp:xdp xmlns:xdp="http://ns.adobe.com/xdp/"><template xmlns="http://www.xfa.org/schema/xfa-template/3.3/"><subform name="form" layout="tb">
<pageSet><pageArea name="front"><contentArea x="0.5in" y="0.5in" w="7.5in" h="10in"/><medium short="8.5in" long="11in"/></pageArea></pageSet>
{draws}
</subform></template></xdp:xdp>"##
    )
}

fn draw(text: &str, extra: &str) -> String {
    format!(r#"<draw w="3in" h="0.3in"><value><text>{text}</text></value><font typeface="Arial" size="12pt"/>{extra}</draw>"#)
}

fn spans(form: &Form) -> Vec<&crate::text::Span> {
    form.pages.iter().flat_map(|p| &p.items).filter_map(|i| if let Item::Text(s) = i { Some(s) } else { None }).collect()
}

/// The form rendered into a document: the document, the report and page 1's content and fonts.
struct Rendered {
    doc: Document,
    report: Report,
    content: String,
    fonts: Dict,
}

fn render(xdp: &str) -> Rendered {
    let mut doc = Document::open(Arc::new(shell(xdp))).unwrap();
    let report = render_into(&mut doc).unwrap();
    let root = doc.get(doc.root().unwrap());
    let pages = doc.resolve(root.as_dict().unwrap().get(b"Pages").unwrap());
    let kids = doc.resolve(pages.as_dict().unwrap().get(b"Kids").unwrap()).as_array().unwrap().clone();
    let page = doc.resolve(&kids[0]);
    let page = page.as_dict().unwrap();
    let Object::Stream(s) = &*doc.resolve(page.get(b"Contents").unwrap()) else { panic!("contents") };
    let content = String::from_utf8_lossy(&s.decoded().unwrap()).into_owned();
    let resources = doc.resolve(page.get(b"Resources").unwrap());
    let fonts = doc.resolve(resources.as_dict().unwrap().get(b"Font").unwrap()).as_dict().unwrap().clone();
    Rendered { doc, report, content, fonts }
}

fn actual_text_hex(text: &str) -> String {
    let mut s = String::from("<FEFF");
    for u in text.encode_utf16() {
        s.push_str(&format!("{u:04X}"));
    }
    s.push('>');
    s
}

fn generated_fonts(r: &Rendered) -> Vec<String> {
    r.fonts.iter().map(|(n, _)| String::from_utf8_lossy(n).into_owned()).filter(|n| n.starts_with("PCXfAr")).collect()
}

fn type3(r: &Rendered, name: &str) -> Dict {
    let font = r.doc.resolve(r.fonts.get(name.as_bytes()).unwrap());
    font.as_dict().unwrap().clone()
}

// ---- tests ----

#[test]
fn arabic_text_is_drawn_shaped_with_one_actual_text_span_and_no_question_marks() {
    let _face = Using::shaper(FAKE);
    let r = render(&form_of(&draw(GREETING, "")));
    assert!(r.content.contains(&format!("/Span <</ActualText {}>> BDC", actual_text_hex(GREETING))), "{}", r.content);
    assert!(r.content.contains("EMC"), "{}", r.content);
    assert!(!r.content.contains('?'), "no ? is written: {}", r.content);
    assert!(!r.report.warnings.iter().any(|w| w.contains("standard fonts")), "{:?}", r.report.warnings);
    let names = generated_fonts(&r);
    assert_eq!(names.len(), 1, "{names:?}");
    let font = type3(&r, &names[0]);
    assert_eq!(font.name(b"Subtype"), Some(&b"Type3"[..]));
    assert!(font.contains(b"ToUnicode"), "copy and search read each letter once");
    assert!(r.content.contains(&format!("/{} 12 Tf", names[0])), "{}", r.content);
}

#[test]
fn a_paragraph_that_runs_right_to_left_is_right_aligned_unless_the_template_states_a_side() {
    let _face = Using::shaper(FAKE);
    // The box is 36..252 pt. The stand-in is 6 pt a character at 12 pt.
    let width = GREETING.chars().count() as f64 * 6.0;
    let form = layout_xml(&form_of(&draw(GREETING, ""))).unwrap();
    let s = spans(&form).into_iter().next().unwrap();
    assert!(s.rtl);
    assert!((s.x + width - 252.0).abs() < 0.01, "right edge at {}", s.x + width);
    // Stated left: it stays at the left.
    let form = layout_xml(&form_of(&draw(GREETING, r#"<para hAlign="left"/>"#))).unwrap();
    let s = spans(&form).into_iter().next().unwrap();
    assert!((s.x - 36.0).abs() < 0.01, "{}", s.x);
    // Stated centre, and stated right.
    let form = layout_xml(&form_of(&draw(GREETING, r#"<para hAlign="center"/>"#))).unwrap();
    assert!((spans(&form)[0].x - (36.0 + (216.0 - width) / 2.0)).abs() < 0.01);
    let form = layout_xml(&form_of(&draw(GREETING, r#"<para hAlign="right"/>"#))).unwrap();
    assert!((spans(&form)[0].x + width - 252.0).abs() < 0.01);
    // Latin text is not moved.
    let form = layout_xml(&form_of(&draw("Hello", ""))).unwrap();
    assert!((spans(&form)[0].x - 36.0).abs() < 0.01);
    assert!(!spans(&form)[0].rtl);
}

#[test]
fn a_latin_paragraph_with_arabic_in_it_stays_left_to_right() {
    let _face = Using::shaper(FAKE);
    let form = layout_xml(&form_of(&draw("Name: مرحبا", ""))).unwrap();
    let s = spans(&form).into_iter().next().unwrap();
    assert!(!s.rtl);
    assert!((s.x - 36.0).abs() < 0.01, "{}", s.x);
}

#[test]
fn latin_words_and_digits_inside_arabic_are_drawn_in_place() {
    let _face = Using::shaper(FAKE);
    let text = "مرحبا 123 abc";
    let r = render(&form_of(&draw(text, "")));
    assert!(r.content.contains(&format!("/ActualText {}", actual_text_hex(text))), "{}", r.content);
    // Right to left, the Latin word and the digits come out in Unicode's visual order (abc, a
    // space, 123, then the Arabic), each drawn by Helvetica as typed.
    let abc = r.content.find("(abc) Tj").expect("abc");
    let digits = r.content.find("(123) Tj").expect("123");
    assert!(abc < digits, "{}", r.content);
    assert!(r.content.contains("/Helv "), "{}", r.content);
    assert!(!r.content.contains('?'));
}

#[test]
fn long_arabic_text_wraps_on_its_shaped_width_one_span_a_line() {
    let _face = Using::shaper(FAKE);
    // 216 pt holds 36 stand-in characters; ten 5-letter words need three lines.
    let text = ["مرحبا"; 10].join(" ");
    let xdp = form_of(&format!(r#"<draw w="3in" h="1in"><value><text>{text}</text></value><font typeface="Arial" size="12pt"/></draw>"#));
    let form = layout_xml(&xdp).unwrap();
    let lines = spans(&form);
    assert!(lines.len() >= 2, "wrapped: {}", lines.len());
    for s in &lines {
        let w = s.face.width_in(&s.text, true);
        assert!(w <= 216.0 + 0.01, "{w} fits the box: {:?}", s.text);
        assert!((s.x + w - 252.0).abs() < 7.0, "right-aligned within a trailing space: {} {}", s.x, w);
    }
    let r = render(&xdp);
    assert_eq!(r.content.matches("/ActualText").count(), lines.len(), "{}", r.content);
    // Together the spans hold every word once, in typing order.
    let joined: Vec<&str> = lines.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(joined.concat().split_whitespace().count(), 10, "{joined:?}");
}

#[test]
fn a_string_of_one_word_longer_than_a_line_is_broken_by_character() {
    let _face = Using::shaper(FAKE);
    let word = "ب".repeat(100);
    let form =
        layout_xml(&form_of(&format!(r#"<draw w="3in" h="1in"><value><text>{word}</text></value><font typeface="Arial" size="12pt"/></draw>"#)))
            .unwrap();
    let lines = spans(&form);
    assert!(lines.len() >= 3, "{}", lines.len());
    assert_eq!(lines.iter().map(|s| s.text.chars().count()).sum::<usize>(), 100);
    assert!(lines.iter().all(|s| s.face.width_in(&s.text, true) <= 216.01));
}

#[test]
fn an_underline_spans_the_shaped_width() {
    let _face = Using::shaper(FAKE);
    let xdp = form_of(&format!(
        r#"<draw w="3in" h="0.3in"><value><text>{GREETING}</text></value><font typeface="Arial" size="12pt" underline="1"/></draw>"#
    ));
    let form = layout_xml(&xdp).unwrap();
    let width = GREETING.chars().count() as f64 * 6.0;
    let line = form.pages[0].items.iter().find_map(|i| if let Item::Line { from, to, .. } = i { Some((from.0, to.0)) } else { None }).unwrap();
    assert!((line.1 - line.0 - width).abs() < 0.01, "{line:?}");
}

#[test]
fn a_form_with_both_scripts_draws_each_text_by_its_own_font() {
    let _face = Using::shaper(FAKE);
    let xdp = form_of(&format!("{}{}{}", draw("Name", ""), draw(GREETING, ""), draw("مرحبا", r#"<para hAlign="left"/>"#)));
    let r = render(&xdp);
    assert_eq!(r.content.matches("/ActualText").count(), 2, "{}", r.content);
    assert!(r.content.contains("(Name) Tj"), "Latin-only text is drawn as ever: {}", r.content);
    assert_eq!(generated_fonts(&r).len(), 1, "one set of glyph fonts for the page");
}

#[test]
fn a_letter_the_face_cannot_draw_keeps_the_old_behaviour_and_adds_no_fonts() {
    let _face = Using::shaper(FAKE);
    // U+06E5's outline is unreadable in the stand-in.
    let xdp = form_of(&format!("{}{}", draw("ab\u{06E5}", ""), draw("Name", "")));
    let r = render(&xdp);
    assert!(r.report.warnings.iter().any(|w| w.contains("Arabic") && w.contains("can't be drawn")), "{:?}", r.report.warnings);
    assert!(r.report.warnings.iter().any(|w| w.contains("standard fonts can't draw")), "{:?}", r.report.warnings);
    assert!(generated_fonts(&r).is_empty(), "nothing was added for the refused text");
    assert!(!r.content.contains("ActualText"));
    assert!(r.content.contains("(Name) Tj"));
    // The plan failed before the document was touched: the same form with a plain `?` for the
    // refused letter has exactly as many objects.
    let plain = render(&form_of(&format!("{}{}", draw("ab?", ""), draw("Name", ""))));
    assert_eq!(r.doc.object_numbers().len(), plain.doc.object_numbers().len());
}

#[test]
fn without_the_arabic_face_the_text_is_reported_and_shown_as_question_marks_as_before() {
    let _face = Using::shaper(NO_FACE);
    let r = render(&form_of(&draw(GREETING, "")));
    let hits: Vec<&String> = r.report.warnings.iter().filter(|w| w.contains("standard fonts can't draw")).collect();
    assert_eq!(hits.len(), 1, "{:?}", r.report.warnings);
    assert!(r.report.warnings.iter().any(|w| w.contains("Arabic text needs PdfCraft's Arabic font")), "{:?}", r.report.warnings);
    assert!(generated_fonts(&r).is_empty());
    assert!(!r.content.contains("ActualText"));
    assert!(r.content.contains('?'), "{}", r.content);
}

#[test]
fn hostile_arabic_text_does_not_panic() {
    let _face = Using::shaper(FAKE);
    // A very long text without spaces, one with only marks and controls, and an empty one.
    let long = "ب".repeat(60_000);
    for text in [long.as_str(), "\u{064B}\u{064B}\u{200F}\u{202E}\u{202C}", "\u{200F}", "(((مرحبا)))", "\u{0600}"] {
        let xdp = form_of(&draw(text, ""));
        let mut doc = Document::open(Arc::new(shell(&xdp))).unwrap();
        // Either laid out or refused with an error; never a panic.
        let _ = render_into(&mut doc);
        let _ = layout_xml(&xdp);
    }
}

#[test]
fn a_stated_alignment_of_a_text_field_is_kept_on_its_widget_and_an_unstated_one_is_not_written() {
    let _face = Using::shaper(FAKE);
    let field = |para: &str| {
        form_of(&format!(r#"<field name="f" w="3in" h="0.3in"><ui><textEdit/></ui>{para}<value><text>{GREETING}</text></value></field>"#))
    };
    let q = |xdp: &str| {
        let r = render(xdp);
        let root = r.doc.get(r.doc.root().unwrap());
        let acro = r.doc.resolve(root.as_dict().unwrap().get(b"AcroForm").unwrap());
        let fields = r.doc.resolve(acro.as_dict().unwrap().get(b"Fields").unwrap()).as_array().unwrap().clone();
        let f = r.doc.resolve(&fields[0]);
        f.as_dict().unwrap().int(b"Q")
    };
    assert_eq!(q(&field("")), None, "the forms layer right-aligns Arabic when no side is stated");
    assert_eq!(q(&field(r#"<para hAlign="left"/>"#)), Some(0), "an explicit left is kept");
    assert_eq!(q(&field(r#"<para hAlign="right"/>"#)), Some(2));
}

/// With the real Arabic face: the greeting is shaped, right-aligned and drawn from glyph outlines.
#[test]
fn the_real_arabic_face_draws_a_form_s_arabic_text() {
    if pdfcraft_fonts::document_arabic_font().is_none() {
        eprintln!("set CRAFT_FONTS_DIR to exercise Arabic XFA text");
        return;
    }
    let xdp = form_of(&draw(GREETING, ""));
    let form = layout_xml(&xdp).unwrap();
    let s = spans(&form).into_iter().next().unwrap();
    let w = s.face.width_in(&s.text, true);
    assert!(w > 20.0 && w < 216.0, "{w}");
    assert!((s.x + w - 252.0).abs() < 0.01, "right-aligned: {}", s.x + w);
    let r = render(&xdp);
    assert!(r.content.contains(&format!("/ActualText {}", actual_text_hex(GREETING))), "{}", r.content);
    assert!(!r.content.contains('?'));
    let names = generated_fonts(&r);
    assert_eq!(names.len(), 1);
    assert!(r.report.warnings.iter().all(|w| !w.contains("Arabic")), "{:?}", r.report.warnings);
}
