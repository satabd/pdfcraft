//! Arabic replacement text in Edit text (`text_shaped`). Most tests run with a stand-in for the
//! craft-fonts Arabic face (joined forms from the neighbours, lam-alef as one cluster, boxed
//! outlines), so they run in any build; the ones at the end use the real face and skip without it.

use pdfcraft_cos::{Document, Object, SaveOptions, write_full};
use pdfcraft_fonts::{GlyphError, GlyphOutline, ShapedCluster, ShapedScript};

use crate::EditError;
use crate::added::{Align, Shaper};
use crate::tests::{arabic_ligature_page, hebrew_page, page_content_bytes, reopen, text_page};
use crate::text::{self, BlockStyle};

// ---- the stand-in face ----

fn is_mark(c: char) -> bool {
    matches!(u32::from(c), 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06ED)
}

fn is_right_joiner(c: char) -> bool {
    matches!(u32::from(c), 0x0622..=0x0625 | 0x0627 | 0x0629 | 0x062F..=0x0632 | 0x0648 | 0x0698)
}

/// Letters that join on both sides.
fn is_dual(c: char) -> bool {
    c.is_alphabetic() && crate::added::is_arabic(c) && !is_mark(c) && !is_right_joiner(c)
}

fn joins_next_letter(c: char) -> bool {
    is_dual(c) || is_right_joiner(c)
}

/// The stand-in's bold is a fifth wider than its regular, so a bold word measures wider (as the real face's does).
const FAKE_BOLD_WIDER: f64 = 1.2;

fn fake_shape(script: ShapedScript, text: &str, rtl: bool) -> Result<Vec<ShapedCluster>, GlyphError> {
    assert!(script.is_arabic(), "only Arabic runs are shaped here");
    let wider = if script == ShapedScript::ArabicBold { FAKE_BOLD_WIDER } else { 1.0 };
    if text.contains('\u{FDFD}') {
        return Err(GlyphError::Missing);
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<ShapedCluster> = Vec::new();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        let before = chars[..i].iter().rev().find(|c| !is_mark(**c)).copied();
        let mut next = i + 1;
        // Lam followed by alef is one ligature glyph.
        if c == '\u{0644}' && matches!(chars.get(i + 1), Some('\u{0627}' | '\u{0622}' | '\u{0623}' | '\u{0625}')) {
            next = i + 2;
            let joined = before.is_some_and(is_dual);
            let id = 0xFEFB + u32::from(joined) * 0x0100_0000;
            let text: String = chars[i..next].iter().collect();
            out.push(ShapedCluster { script, glyphs: vec![(id, [0.0, 0.0])], advance: 0.6 * wider, text });
            i = next;
            continue;
        }
        let after = chars[next..].iter().find(|c| !is_mark(**c)).copied();
        let form = u32::from(before.is_some_and(is_dual)) | u32::from(is_dual(c) && after.is_some_and(joins_next_letter)) << 1;
        let advance = if is_mark(c) {
            0.0
        } else if c == ' ' {
            0.25
        } else if c.is_alphabetic() {
            0.5 + 0.02 * f64::from(form)
        } else {
            0.4
        };
        out.push(ShapedCluster { script, glyphs: vec![(u32::from(c) | form << 24, [0.0, 0.0])], advance: advance * wider, text: c.to_string() });
        i = next;
    }
    if rtl {
        out.reverse();
    }
    Ok(out)
}

fn fake_has(script: ShapedScript, c: char) -> bool {
    script.is_arabic() && (c == ' ' || c.is_ascii_punctuation() || crate::added::is_arabic(c))
}

fn fake_outline(script: ShapedScript, id: u32) -> Result<GlyphOutline, GlyphError> {
    // One letter whose outline is too complex.
    if id & 0x00FF_FFFF == 0x06E5 {
        return Err(GlyphError::TooComplex);
    }
    let advance = if script == ShapedScript::ArabicBold { 0.5 * FAKE_BOLD_WIDER } else { 0.5 };
    Ok(GlyphOutline { contours: vec![vec![[0.0, 0.0], [advance, 0.0], [advance, 0.5], [0.0, 0.5]]], width: advance, bbox: [0.0, 0.0, advance, 0.5] })
}

pub(crate) const FAKE: Shaper = Shaper { shape: fake_shape, has: fake_has, outline: fake_outline, arabic_available: || true };

fn no_shape(_: ShapedScript, _: &str, _: bool) -> Result<Vec<ShapedCluster>, GlyphError> {
    Err(GlyphError::NoFont)
}

fn no_outline(_: ShapedScript, _: u32) -> Result<GlyphOutline, GlyphError> {
    Err(GlyphError::NoFont)
}

/// A build with no Arabic face.
pub(crate) const NO_FACE: Shaper = Shaper { shape: no_shape, has: |_, _| false, outline: no_outline, arabic_available: || false };

/// Uses a shaper on this thread until dropped.
pub(crate) struct Using;

impl Using {
    pub(crate) fn shaper(shaper: Shaper) -> Using {
        pdfcraft_fonts::shaped_text::set_test_shaper(Some(shaper));
        Using
    }
}

impl Drop for Using {
    fn drop(&mut self) {
        pdfcraft_fonts::shaped_text::set_test_shaper(None);
    }
}

pub(crate) fn fake() -> Using {
    Using::shaper(FAKE)
}

// ---- reading what an edit wrote ----

/// A page with one short line in a subset font that has only three glyphs (`B`/`A` are س/م and `X`
/// lam-alef), drawn from x = 300: nothing a typed text could be encoded with.
fn page() -> Document {
    arabic_ligature_page("BT /F1 12 Tf 300 700 Td (BAB) Tj ET", false)
}

fn ops(doc: &Document) -> Vec<pdfcraft_content::Op> {
    pdfcraft_content::parse(&page_content_bytes(doc, 0)).ops
}

fn utf16_hex(text: &str) -> String {
    text.encode_utf16().map(|u| format!("{u:04X}")).collect()
}

/// The `/ActualText` of every span on the page, decoded.
fn actual_texts(doc: &Document) -> Vec<String> {
    ops(doc)
        .iter()
        .filter(|o| o.is("BDC"))
        .filter_map(|o| match o.operands.get(1) {
            Some(Object::Dict(d)) => match d.get(b"ActualText") {
                Some(Object::String(s)) => {
                    assert!(s.hex && s.bytes.starts_with(&[0xFE, 0xFF]), "UTF-16BE with a byte order mark, as a hex string");
                    Some(s.to_text())
                }
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// The font and bytes of every string the page shows, in drawing order.
fn shown(doc: &Document) -> Vec<(String, Vec<u8>)> {
    let mut font = String::new();
    let mut out = Vec::new();
    for op in ops(doc) {
        if op.is("Tf") {
            font = String::from_utf8_lossy(op.name(0).unwrap_or_default()).into_owned();
        } else if op.is("Tj")
            && let Some(Object::String(s)) = op.operands.first()
        {
            out.push((font.clone(), s.bytes.clone()));
        }
    }
    out
}

/// The page's font resources.
fn fonts(doc: &Document) -> pdfcraft_cos::Dict {
    let page = pdfcraft_model::pages(doc).swap_remove(0);
    let res = doc.resolve(page.dict.get(b"Resources").expect("resources"));
    let fonts = doc.resolve(res.as_dict().expect("resources dict").get(b"Font").expect("fonts"));
    fonts.as_dict().expect("font dict").clone()
}

/// The generated Arabic fonts: resource name and font dictionary.
fn arabic_fonts(doc: &Document) -> Vec<(String, pdfcraft_cos::Dict)> {
    fonts(doc)
        .iter()
        .filter(|(name, _)| name.starts_with(b"PCEdAr"))
        .map(|(name, font)| (String::from_utf8_lossy(name).into_owned(), doc.resolve(font).as_dict().expect("font").clone()))
        .collect()
}

fn bytes_of(doc: &Document) -> Vec<u8> {
    write_full(doc, &SaveOptions::default()).expect("saves")
}

/// The first line of the page, read back.
fn line(doc: &Document) -> text::TextLine {
    text::text_lines(doc, 0).expect("lines").swap_remove(0)
}

/// The decoded ToUnicode map of a generated font.
fn to_unicode(doc: &Document, font: &pdfcraft_cos::Dict) -> String {
    let Object::Stream(cmap) = &*doc.resolve(font.get(b"ToUnicode").expect("ToUnicode")) else { panic!("a stream") };
    String::from_utf8(cmap.decoded().expect("decodes")).expect("text")
}

// ---- tests ----

const TYPED: [&str; 8] =
    ["مرحبا بالعالم", "سَلَام 123 (PDF)", "English مرحبا ١٢٣", "PDFمرحبا", "لا", "لأ", "السلام عليكم ورحمة الله", "عدد (٣) من [أصل] ٥"];

#[test]
fn arabic_replacement_on_a_subset_font_is_shaped_type3_with_actual_text() {
    let _face = fake();
    let mut doc = page();
    let before = line(&doc).rect;
    let typed = "مرحبا بالعالم";
    let edit = text::replace_block(&mut doc, 0, 0, typed).expect("shaped, not refused");
    assert!(edit.substituted.as_deref().is_some_and(|s| s.ends_with(" Type3")), "{edit:?}");

    // One span in typing order; its glyphs come from generated fonts, none from the page's font.
    assert_eq!(actual_texts(&doc), [typed]);
    let content = String::from_utf8_lossy(&page_content_bytes(&doc, 0)).into_owned();
    assert!(content.contains(&format!("<FEFF{}>", utf16_hex(typed))) && content.contains("EMC"), "{content}");
    let drawn = shown(&doc);
    assert!(!drawn.is_empty() && drawn.iter().all(|(font, _)| font.starts_with("PCEdAr")), "{drawn:?}");
    let generated = arabic_fonts(&doc);
    assert!(!generated.is_empty());
    for (name, font) in &generated {
        assert_eq!(font.name(b"Subtype"), Some(b"Type3".as_slice()), "{name}");
        assert_eq!(font.name(b"Name"), Some(b"PCArabic".as_slice()), "{name}");
        assert!(font.get(b"ToUnicode").is_some(), "{name}");
    }

    // It reads back as typed, with its right edge where the old line ended.
    let doc = reopen(&doc);
    let lines = text::text_lines(&doc, 0).expect("lines");
    assert_eq!(lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), [typed]);
    assert_eq!(text::text_blocks(&doc, 0).expect("blocks")[0].text, typed);
    assert!((lines[0].rect[2] - before[2]).abs() < 0.5, "right edge {} → {}", before[2], lines[0].rect[2]);
    assert!(lines[0].rect[0] < before[0] - 1.0, "longer text grows to the left: {:?} vs {before:?}", lines[0].rect);
}

#[test]
fn applying_unchanged_arabic_text_keeps_the_producers_glyphs() {
    let _face = fake();
    let mut doc = page();
    let before = bytes_of(&doc);
    let block = text::text_blocks(&doc, 0).expect("blocks").swap_remove(0);
    assert!(crate::text_shaped::has_arabic(&block.text), "{:?}", block.text);
    for draft in [None, Some(block.text.as_str()), Some(block.editor_text.as_str())] {
        let edit = text::rewrite_block(&mut doc, 0, 0, draft, &BlockStyle::default()).expect("nothing to do");
        assert_eq!(edit.substituted, None);
        assert_eq!(bytes_of(&doc), before, "{draft:?}");
    }
    // A change of style is not nothing: the paragraph is drawn again, shaped.
    let style = BlockStyle { size: Some(20.0), ..Default::default() };
    text::rewrite_block(&mut doc, 0, 0, None, &style).expect("restyled");
    assert_eq!(actual_texts(&doc), [block.text]);
}

#[test]
fn replace_line_shapes_arabic_instead_of_writing_isolated_letters() {
    let _face = fake();
    let mut doc = page();
    let typed = "مرحبا بالعالم";
    let edit = text::replace_line(&mut doc, 0, 0, typed).expect("shaped");
    assert!(edit.substituted.as_deref().is_some_and(|s| s.ends_with(" Type3")), "{edit:?}");
    assert_eq!(actual_texts(&doc), [typed]);
    let drawn = shown(&doc);
    assert!(!drawn.is_empty() && drawn.iter().all(|(font, _)| font.starts_with("PCEdAr")), "no letter is drawn from the page's font: {drawn:?}");
    // The line's own font is selected again for the text after it.
    let tf: Vec<_> = ops(&doc).into_iter().filter(|o| o.is("Tf")).filter_map(|o| o.name(0).map(<[u8]>::to_vec)).collect();
    assert_eq!(tf.last().map(Vec::as_slice), Some(b"F1".as_slice()), "{tf:?}");
    let doc = reopen(&doc);
    assert_eq!(line(&doc).text, typed);
    assert_eq!(text::text_blocks(&doc, 0).expect("blocks")[0].text, typed);
}

#[test]
fn replace_line_keeps_spacing_out_of_the_joined_letters_and_restores_it() {
    let _face = fake();
    let mut doc = arabic_ligature_page("BT /F1 12 Tf 1.5 Tc 2 Tw 300 700 Td (BAB) Tj ET", false);
    text::replace_line(&mut doc, 0, 0, "مرحبا").expect("shaped");
    let spacing: Vec<(String, f64)> = ops(&doc)
        .iter()
        .filter(|o| o.is("Tc") || o.is("Tw"))
        .map(|o| (String::from_utf8_lossy(&o.op).into_owned(), o.num(0).unwrap_or(f64::NAN)))
        .collect();
    // Zero while the line is drawn, then the page's own again.
    let pos = |op: &str, v: f64| spacing.iter().rposition(|s| *s == (op.to_string(), v));
    assert!(pos("Tc", 0.0) < pos("Tc", 1.5) && pos("Tc", 0.0).is_some(), "{spacing:?}");
    assert!(pos("Tw", 0.0) < pos("Tw", 2.0) && pos("Tw", 0.0).is_some(), "{spacing:?}");
}

#[test]
fn typed_arabic_round_trips_through_block_and_line_edits() {
    let _face = fake();
    for typed in TYPED {
        for by_line in [false, true] {
            let mut doc = page();
            if by_line {
                text::replace_line(&mut doc, 0, 0, typed).expect(typed);
            } else {
                text::replace_block(&mut doc, 0, 0, typed).expect(typed);
            }
            assert_eq!(actual_texts(&doc), [typed], "{typed} (line: {by_line})");
            for doc in [&doc, &reopen(&doc)] {
                let lines = text::text_lines(doc, 0).expect("lines");
                assert_eq!(lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), [typed], "{typed} (line: {by_line})");
                assert_eq!(text::text_blocks(doc, 0).expect("blocks")[0].text, typed, "{typed} (line: {by_line})");
            }
        }
    }
    // A line with no Arabic is not this path's business.
    let mut doc = page();
    text::replace_block(&mut doc, 0, 0, "Latin only").expect("latin");
    assert!(actual_texts(&doc).is_empty() && arabic_fonts(&doc).is_empty());
    assert_eq!(line(&reopen(&doc)).text, "Latin only");
}

#[test]
fn lam_alef_is_one_glyph_with_both_letters_in_its_to_unicode() {
    let _face = fake();
    let mut doc = page();
    text::replace_block(&mut doc, 0, 0, "لا").expect("lam-alef");
    assert_eq!(shown(&doc).iter().map(|(_, bytes)| bytes.len()).sum::<usize>(), 1);
    let (_, font) = arabic_fonts(&doc).swap_remove(0);
    let cmap = to_unicode(&doc, &font);
    assert!(cmap.contains("<01> <06440627>"), "{cmap}");
}

#[test]
fn latin_and_digits_use_the_page_font_or_helvetica_beside_the_shaped_runs() {
    let _face = fake();
    // A left-to-right paragraph (more Latin than Arabic): the Latin comes first.
    let mut doc = page();
    text::replace_block(&mut doc, 0, 0, "English مرحبا ١٢٣").expect("mixed");
    let drawn = shown(&doc);
    assert_eq!(drawn.first(), Some(&("PCEdHelv".to_string(), b"English ".to_vec())), "{drawn:?}");
    assert!(drawn[1..].iter().all(|(font, _)| font.starts_with("PCEdAr")), "{drawn:?}");
    // A right-to-left paragraph with a Latin word: it is drawn after (right of) the Arabic.
    let mut doc = page();
    text::replace_block(&mut doc, 0, 0, "PDFمرحبا").expect("mixed");
    let drawn = shown(&doc);
    // (The page's font has P, D and F; it lacks the lower-case letters of "English".)
    assert_eq!(drawn.last(), Some(&("F1".to_string(), b"PDF".to_vec())), "{drawn:?}");
    assert!(drawn[..drawn.len() - 1].iter().all(|(font, _)| font.starts_with("PCEdAr")), "{drawn:?}");
    let mut doc = page();
    text::replace_block(&mut doc, 0, 0, "مرحبا (PDF)").expect("brackets");
    assert_eq!(actual_texts(&doc), ["مرحبا (PDF)"]);

    // The page's own font shows the Latin it has: Helvetica here, so no substitute is made.
    let mut doc = text_page("BT /F1 12 Tf 72 700 Td (Hello) Tj ET");
    text::replace_block(&mut doc, 0, 0, "Hello مرحبا").expect("page font");
    let drawn = shown(&doc);
    assert_eq!(drawn.first(), Some(&("F1".to_string(), b"Hello ".to_vec())), "{drawn:?}");
    assert_eq!(text::text_blocks(&reopen(&doc), 0).expect("blocks")[0].text, "Hello مرحبا");
}

#[test]
fn wrapped_arabic_lines_each_get_their_own_span_and_fit_the_width() {
    let _face = fake();
    let mut doc = page();
    let typed = "مرحبا بالعالم مرحبا بالعالم مرحبا بالعالم مرحبا بالعالم";
    let style = BlockStyle { width: Some(100.0), ..Default::default() };
    text::rewrite_block(&mut doc, 0, 0, Some(typed), &style).expect("wrapped");
    let spans = actual_texts(&doc);
    assert!(spans.len() >= 3, "{spans:?}");
    assert_eq!(spans.join(" "), typed);
    let doc = reopen(&doc);
    let lines = text::text_lines(&doc, 0).expect("lines");
    assert_eq!(lines.iter().map(|l| l.text.clone()).collect::<Vec<_>>(), spans);
    for l in &lines {
        assert!(l.rect[2] - l.rect[0] <= 100.5, "{:?} is {} wide", l.text, l.rect[2] - l.rect[0]);
    }
    let blocks = text::text_blocks(&doc, 0).expect("blocks");
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].text, typed);
    // Right to left: every line ends at the same right edge.
    assert!(lines.iter().all(|l| (l.rect[2] - lines[0].rect[2]).abs() < 0.5), "{:?}", lines.iter().map(|l| l.rect).collect::<Vec<_>>());
}

#[test]
fn editing_an_edited_arabic_paragraph_never_reuses_its_generated_fonts_by_unicode() {
    let _face = fake();
    let mut doc = page();
    text::replace_block(&mut doc, 0, 0, "مرحبا بالعالم").expect("first edit");
    let mut doc = reopen(&doc);
    let old: Vec<String> = arabic_fonts(&doc).into_iter().map(|(name, _)| name).collect();
    assert!(!old.is_empty());
    // The generated fonts' ToUnicode repeats a base letter for each of its forms: asking them to
    // encode text would pick the wrong glyph, so the paragraph is shaped again.
    for typed in ["السلام عليكم", "مرحبا PDF", "بب"] {
        text::replace_block(&mut doc, 0, 0, typed).expect(typed);
        assert_eq!(actual_texts(&doc), [typed]);
        let drawn = shown(&doc);
        assert!(drawn.iter().all(|(font, _)| font.starts_with("PCEdAr") || font == "PCEdHelv"), "{typed}: {drawn:?}");
        assert!(drawn.iter().all(|(font, _)| !old.contains(font)), "{typed}: the old fonts are not drawn from: {drawn:?}");
        doc = reopen(&doc);
        assert_eq!(line(&doc).text, typed);
        // One paragraph: the old span and its glyphs are gone.
        assert_eq!(ops(&doc).iter().filter(|o| o.is("BDC")).count(), 1);
        assert_eq!(ops(&doc).iter().filter(|o| o.is("EMC")).count(), 1);
    }
    // The paragraph's own text restyled (no new text) is shaped again, not re-encoded.
    let style = BlockStyle { size: Some(20.0), ..Default::default() };
    text::rewrite_block(&mut doc, 0, 0, None, &style).expect("restyled");
    assert_eq!(actual_texts(&doc), ["بب"]);
    assert!(shown(&doc).iter().all(|(font, _)| font.starts_with("PCEdAr")));
}

#[test]
fn a_small_font_limit_splits_the_glyphs_over_several_fonts_in_one_line() {
    let _face = fake();
    let mut doc = page();
    let typed = "مرحبا بالعالم الجميل";
    text::rewrite_block_with_limit(&mut doc, 0, 0, Some(typed), &BlockStyle::default(), 3).expect("split");
    let generated = arabic_fonts(&doc);
    assert!(generated.len() >= 3, "{} fonts", generated.len());
    for (name, font) in &generated {
        let Some(Object::Dict(procs)) = font.get(b"CharProcs") else { panic!("{name} has no CharProcs") };
        assert!(procs.len() <= 3, "{name} holds {} glyphs", procs.len());
    }
    // Still one span, one line.
    assert_eq!(actual_texts(&doc), [typed]);
    // The names are distinct from Add text's `PCAr` fonts, and from each other.
    let names: std::collections::HashSet<_> = generated.iter().map(|(n, _)| n.clone()).collect();
    assert_eq!(names.len(), generated.len());
    // Every code drawn is defined by its font.
    for (font, bytes) in shown(&doc) {
        let (_, dict) = generated.iter().find(|(n, _)| *n == font).expect("a generated font");
        let Some(Object::Array(widths)) = dict.get(b"Widths") else { panic!("widths") };
        assert!(bytes.iter().all(|b| usize::from(*b) >= 1 && usize::from(*b) <= widths.len()), "{font}: {bytes:?}");
    }
    let doc = reopen(&doc);
    assert_eq!(text::text_lines(&doc, 0).expect("lines").iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), [typed]);
}

#[test]
fn a_build_without_the_arabic_face_says_so_and_changes_nothing() {
    let _face = Using::shaper(NO_FACE);
    let mut doc = page();
    let before = bytes_of(&doc);
    for typed in ["مرحبا بالعالم", "Hello مرحبا"] {
        let block = text::replace_block(&mut doc, 0, 0, typed);
        let Err(EditError::Invalid(msg)) = block else { panic!("expected a clear error, got {block:?}") };
        assert!(msg.contains("Arabic") && msg.contains("CRAFT_FONTS_DIR"), "{msg}");
        let line = text::replace_line(&mut doc, 0, 0, typed);
        let Err(EditError::Invalid(msg)) = line else { panic!("expected a clear error, got {line:?}") };
        assert!(msg.contains("Arabic") && msg.contains("CRAFT_FONTS_DIR"), "{msg}");
        assert_eq!(bytes_of(&doc), before, "{typed}");
    }
    // Latin text is unaffected.
    text::replace_block(&mut doc, 0, 0, "Hello").expect("latin");
}

#[test]
fn arabic_and_hebrew_in_one_paragraph_is_a_clear_error() {
    let _face = fake();
    let mut doc = hebrew_page("BT /F1 12 Tf 300 700 Td (EDCBA) Tj ET");
    let before = bytes_of(&doc);
    let block = text::replace_block(&mut doc, 0, 0, "שלום مرحبا");
    let Err(EditError::Invalid(msg)) = block else { panic!("expected an error, got {block:?}") };
    assert!(msg.contains("Arabic") && msg.contains("Hebrew"), "{msg}");
    let line = text::replace_line(&mut doc, 0, 0, "שלום مرحبا");
    assert!(matches!(&line, Err(EditError::Invalid(m)) if m.contains("Hebrew")), "{line:?}");
    assert_eq!(bytes_of(&doc), before);
}

#[test]
fn other_scripts_the_fonts_lack_next_to_arabic_are_refused_before_anything_changes() {
    let _face = fake();
    let mut doc = page();
    let before = bytes_of(&doc);
    for typed in ["مرحبا 日本語", "مرحبا Привет", "مرحبا 😀"] {
        let block = text::replace_block(&mut doc, 0, 0, typed);
        assert!(matches!(&block, Err(EditError::Invalid(_))), "{typed}: {block:?}");
        let line = text::replace_line(&mut doc, 0, 0, typed);
        assert!(matches!(&line, Err(EditError::Invalid(_))), "{typed}: {line:?}");
        assert_eq!(bytes_of(&doc), before, "{typed}");
    }
}

#[test]
fn formatting_part_of_arabic_text_keeps_the_styled_path_and_a_refusal_changes_nothing() {
    let _face = fake();
    let mut doc = page();
    let before = bytes_of(&doc);
    let style = BlockStyle {
        ranges: vec![text::RangeStyle {
            range: 0..2,
            family: None,
            resource: None,
            size: Some(20.0),
            color: None,
            underline: None,
            char_spacing: None,
            scale: None,
        }],
        ..Default::default()
    };
    let r = text::rewrite_block(&mut doc, 0, 0, Some("مرحبا"), &style);
    assert!(r.is_err(), "{r:?}");
    assert_eq!(bytes_of(&doc), before);
}

const WORDS: &str = "مرحبا بالعالم مرحبا بالعالم مرحبا بالعالم مرحبا بالعالم";

/// What a justified draft wrote: the page's lines (rects in points), `/ActualText` of each line and the generated fonts'
/// `ToUnicode` maps, for the draft in `style`.
fn written(typed: &str, style: &BlockStyle) -> (Document, Vec<text::TextLine>) {
    let mut doc = page();
    text::rewrite_block(&mut doc, 0, 0, Some(typed), style).expect("writes");
    let lines = text::text_lines(&reopen(&doc), 0).expect("lines");
    (doc, lines)
}

#[test]
fn justified_arabic_fills_every_row_it_breaks_with_adjustments_after_its_spaces() {
    let _face = fake();
    let style = BlockStyle { width: Some(100.0), align: Some(Align::Justify), char_spacing: Some(2.0), ..Default::default() };
    let (doc, lines) = written(WORDS, &style);
    assert!(lines.len() >= 3, "{:?}", lines.iter().map(|l| l.rect).collect::<Vec<_>>());
    let (last, rows) = lines.split_last().expect("a last row");
    // The box is 100 pt wide from x = 300: the rows the wrap broke fill it from edge to edge.
    for l in rows {
        assert!((l.rect[0] - 300.0).abs() < 0.5 && (l.rect[2] - 400.0).abs() < 0.5, "{:?}: {:?}", l.text, l.rect);
    }
    // The last row is as the page left it unjustified: right-to-left text ends at the right edge.
    assert!((last.rect[2] - 400.0).abs() < 0.5 && last.rect[0] > 300.5, "{:?}", last.rect);
    let ops = ops(&doc);
    // Word spacing does not reach the generated fonts' codes, nor does character spacing belong to joined letters.
    assert!(ops.iter().any(|o| o.is("TJ")));
    assert!(ops.iter().filter(|o| o.is("Tw") || o.is("Tc")).all(|o| o.num(0) == Some(0.0)), "{ops:?}");
    // What the text says is what it said unjustified: the same lines in typing order, the same glyph maps.
    let (plain, plain_lines) = written(WORDS, &BlockStyle { width: Some(100.0), ..Default::default() });
    assert_eq!(actual_texts(&doc), actual_texts(&plain));
    assert_eq!(lines.iter().map(|l| &l.text).collect::<Vec<_>>(), plain_lines.iter().map(|l| &l.text).collect::<Vec<_>>());
    let maps = |doc: &Document| arabic_fonts(doc).iter().map(|(_, f)| to_unicode(doc, f)).collect::<Vec<_>>();
    assert_eq!(maps(&doc), maps(&plain));
    // (An unjustified paragraph is ragged: the rows end at the right edge but begin where their width puts them.)
    assert!(plain_lines.iter().any(|l| l.rect[0] > 300.5));
}

#[test]
fn a_justified_arabic_row_of_one_word_or_before_a_line_break_is_not_stretched() {
    let _face = fake();
    let style = BlockStyle { width: Some(100.0), align: Some(Align::Justify), ..Default::default() };
    // One word of twelve letters (72 pt) fills most of a row, and the next, of eight, does not fit after it.
    let (doc, lines) = written("ببببببببببببب ببببببب مرحبا بالعالم مرحبا بالعالم مرحبا بالعالم", &style);
    assert!(lines.len() >= 3, "{:?}", lines.iter().map(|l| l.rect).collect::<Vec<_>>());
    let first = lines.first().expect("a first row");
    assert!(!actual_texts(&doc).first().expect("a span").contains(' '), "{:?}", actual_texts(&doc));
    assert!(first.rect[2] - first.rect[0] < 99.0 && (first.rect[2] - 400.0).abs() < 0.5, "a single word keeps its width: {:?}", first.rect);
    // A line break ends a row that the wrap did not break: it keeps its width too.
    let (doc, lines) = written("مرحبا بالعالم\nمرحبا بالعالم مرحبا بالعالم مرحبا بالعالم مرحبا بالعالم", &style);
    assert!(lines.len() >= 3, "{:?}", lines.iter().map(|l| l.rect).collect::<Vec<_>>());
    assert_eq!(actual_texts(&doc).first().map(String::as_str), Some("مرحبا بالعالم"));
    let first = lines.first().expect("a first row");
    assert!(first.rect[2] - first.rect[0] < 90.0 && (first.rect[2] - 400.0).abs() < 0.5, "{:?}", first.rect);
    // The rows the wrap broke after it still fill the width.
    assert!(
        lines[1..lines.len() - 1].iter().all(|l| (l.rect[0] - 300.0).abs() < 0.5 && (l.rect[2] - 400.0).abs() < 0.5),
        "{:?}",
        lines.iter().map(|l| l.rect).collect::<Vec<_>>()
    );
}

#[test]
fn a_justified_arabic_row_with_latin_words_in_it_is_stretched_to_the_width_too() {
    let _face = fake();
    let style = BlockStyle { width: Some(120.0), align: Some(Align::Justify), ..Default::default() };
    let (_, lines) = written("مرحبا hello world بالعالم مرحبا by the way بالعالم مرحبا the end بالعالم", &style);
    assert!(lines.len() >= 3, "{:?}", lines.iter().map(|l| l.rect).collect::<Vec<_>>());
    for l in &lines[..lines.len() - 1] {
        assert!((l.rect[0] - 300.0).abs() < 0.5 && (l.rect[2] - 420.0).abs() < 0.5, "{:?}: {:?}", l.text, l.rect);
    }
}

#[test]
fn hostile_arabic_input_gives_a_result_and_never_a_panic() {
    let _face = fake();
    let long_word = "ب".repeat(50_000);
    let many_words = "ب ".repeat(20_000);
    // Each letter in its isolated, initial, final and medial context.
    let mut every_shape = String::new();
    for c in ('\u{0600}'..='\u{06FF}')
        .chain('\u{0750}'..='\u{077F}')
        .chain('\u{0870}'..='\u{08FF}')
        .chain('\u{FB50}'..='\u{FDFC}')
        .chain('\u{FE70}'..='\u{FEFC}')
    {
        every_shape.push_str(&format!("{c} {c}ب ب{c} ب{c}ب "));
    }
    let cases: Vec<&str> = vec![
        "\u{064B}",
        "ب\u{064B}\u{064B}\u{064B}",
        "\u{0640}\u{0640}\u{0640}",
        "ب\u{200D}ب",
        "ب\u{200C}ب",
        "\u{200D}",
        "\u{202E}ب\u{064B}\u{064B} (]",
        "ا\u{200F}\u{2067}b\u{2069}",
        "\u{0600}\u{0601}\u{0602}\u{0603}ب",
        "\u{FDFD}",
        "ب \u{FDFD}",
        "ﷲ\u{FB50}\u{FE70}ﻻ",
        "ب\u{0001}ب",
        "ب\u{0000}",
        "ب😀",
        "ب\u{E000}",
        "ب\u{06E5}ب",
        "ب\n\nب\tc",
        "ﷺ ١٢٣ 456",
        "لا لا لا لا لا لا",
        &long_word,
        &many_words,
        &every_shape,
    ];
    for (i, typed) in cases.iter().enumerate() {
        for width in [None, Some(30.0), Some(14_400.0)] {
            let style = BlockStyle { width, ..Default::default() };
            let mut doc = page();
            let before = bytes_of(&doc);
            match text::rewrite_block(&mut doc, 0, 0, Some(typed), &style) {
                Ok(_) => {
                    let again = reopen(&doc);
                    text::text_lines(&again, 0).expect("reads back");
                    text::text_blocks(&again, 0).expect("reads back");
                }
                Err(_) => assert_eq!(bytes_of(&doc), before, "case {i} {width:?}: a refused edit changes nothing"),
            }
            if width.is_none() {
                let mut doc = page();
                match text::replace_line(&mut doc, 0, 0, typed) {
                    Ok(_) => {
                        text::text_lines(&reopen(&doc), 0).expect("reads back");
                    }
                    Err(_) => assert_eq!(bytes_of(&doc), before, "case {i}: a refused line edit changes nothing"),
                }
            }
        }
    }
    // More distinct shapes than a rewrite may draw is refused by name.
    let mut doc = page();
    let before = bytes_of(&doc);
    let style = BlockStyle { width: Some(14_400.0), ..Default::default() };
    let r = text::rewrite_block(&mut doc, 0, 0, Some(&every_shape), &style);
    let Err(EditError::Invalid(msg)) = r else { panic!("expected the shape cap, got {:?}", r.map(|_| ())) };
    assert!(msg.contains("letter shapes"), "{msg}");
    assert_eq!(bytes_of(&doc), before);
    // Text no person types is refused before anything is shaped.
    let huge = "ب".repeat(600_000);
    let r = text::replace_block(&mut doc, 0, 0, &huge);
    assert!(matches!(&r, Err(EditError::Invalid(m)) if m.contains("too long")), "{:?}", r.map(|_| ()));
    let r = text::replace_line(&mut doc, 0, 0, &huge);
    assert!(matches!(&r, Err(EditError::Invalid(m)) if m.contains("too long")), "{:?}", r.map(|_| ()));
    assert_eq!(bytes_of(&doc), before);
    // A glyph whose outline can't be read leaves the document as it was.
    let r = text::replace_block(&mut doc, 0, 0, "ب\u{06E5}ب");
    assert!(matches!(&r, Err(EditError::Invalid(m)) if m.contains("too complex")), "{r:?}");
    assert_eq!(bytes_of(&doc), before);
}

#[test]
fn a_paragraph_of_thousands_of_words_wraps_in_reasonable_time() {
    let _face = fake();
    let mut doc = page();
    let typed = "مرحبا ".repeat(5_000);
    let started = std::time::Instant::now();
    text::rewrite_block(&mut doc, 0, 0, Some(&typed), &BlockStyle { width: Some(14_400.0), ..Default::default() }).expect("long paragraph");
    assert!(started.elapsed().as_secs() < 60, "took {:?}", started.elapsed());
}

#[test]
fn shaped_lines_are_read_in_typing_order_without_reordering() {
    // The span is in typing order, so the reader must not reorder it again.
    let _face = fake();
    let typed = "Hello مرحبا 123 (PDF)";
    let mut doc = page();
    text::replace_block(&mut doc, 0, 0, typed).expect("mixed");
    assert_eq!(line(&reopen(&doc)).text, typed);
}

// ---- where the rows go ----

/// A paragraph of `BAB` rows (مسم) at 12 pt from x = 300, one on each of `baselines`, top to bottom.
fn rows_at(baselines: &[f64]) -> Document {
    let mut content = String::from("BT /F1 12 Tf");
    let mut at = 0.0;
    for (i, baseline) in baselines.iter().enumerate() {
        let (x, y) = if i == 0 { (300.0, *baseline) } else { (0.0, baseline - at) };
        content.push_str(&format!(" {x} {y} Td (BAB) Tj"));
        at = *baseline;
    }
    content.push_str(" ET");
    arabic_ligature_page(&content, false)
}

/// The baseline of each row the page reads back, top to bottom (the runs on one baseline being one row).
fn baselines(doc: &Document) -> Vec<f64> {
    let mut out: Vec<f64> = Vec::new();
    for line in text::text_lines(&reopen(doc), 0).expect("lines") {
        if out.last().is_none_or(|b| (b - line.origin_baseline()).abs() > 1.0) {
            out.push(line.origin_baseline());
        }
    }
    out
}

/// Each row's box as the page reads it back (the runs on one baseline being one row).
fn row_boxes(doc: &Document) -> Vec<[f64; 4]> {
    let mut out: Vec<(f64, [f64; 4])> = Vec::new();
    for line in text::text_lines(&reopen(doc), 0).expect("lines") {
        match out.last_mut() {
            Some((baseline, r)) if (*baseline - line.origin_baseline()).abs() <= 1.0 => {
                *r = [r[0].min(line.rect[0]), r[1].min(line.rect[1]), r[2].max(line.rect[2]), r[3].max(line.rect[3])];
            }
            _ => out.push((line.origin_baseline(), line.rect)),
        }
    }
    out.into_iter().map(|(_, r)| r).collect()
}

/// The steps down from each of `heights` to the next.
fn steps_of(heights: &[f64]) -> Vec<f64> {
    heights.windows(2).map(|w| w[0] - w[1]).collect()
}

/// Whether the rows `text_edit_layout` reported are the rows the page reads back, a step apart as `want`'s baselines are.
fn reported_where_written(row_rects: &[[f64; 4]], doc: &Document, want: &[f64]) -> bool {
    let bottoms: Vec<f64> = row_rects.iter().map(|r| r[1]).collect();
    let read = row_boxes(doc);
    read.len() == row_rects.len()
        && read.iter().zip(row_rects).all(|(a, b)| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.01))
        && close(&steps_of(&bottoms), &steps_of(want))
}

fn close(got: &[f64], want: &[f64]) -> bool {
    got.len() == want.len() && got.iter().zip(want).all(|(g, w)| (g - w).abs() < 0.01)
}

/// A paragraph whose larger first gap (22 pt, then 16) is a heading's leading.
const UNEVEN: [f64; 3] = [720.0, 698.0, 682.0];

/// (Wide enough that no row wraps.)
fn wide() -> BlockStyle {
    BlockStyle { width: Some(200.0), ..Default::default() }
}

#[test]
fn a_retyped_paragraph_of_uneven_rows_writes_each_row_on_its_source_rows_baseline() {
    let _face = fake();
    let mut doc = rows_at(&UNEVEN);
    let blocks = text::text_blocks(&doc, 0).expect("blocks");
    assert_eq!((blocks.len(), blocks[0].rows.len()), (1, 3), "one paragraph of three rows");
    let typed = "مرحبا\nبالعالم\nسلام";
    // What the editor is told: the rows where the rewrite writes them.
    let layout = text::text_edit_layout(&doc, 0, 0, typed, &wide()).expect("layout");
    text::rewrite_block(&mut doc, 0, 0, Some(typed), &wide()).expect("writes");
    assert_eq!(actual_texts(&doc), typed.split('\n').collect::<Vec<_>>());
    let written = baselines(&doc);
    assert!(close(&written, &UNEVEN), "{written:?}");
    assert_eq!(layout.rows, typed.split('\n').collect::<Vec<_>>());
    assert!(reported_where_written(&layout.row_rects, &doc, &UNEVEN), "{:?} against {:?}", layout.row_rects, row_boxes(&doc));
}

#[test]
fn a_row_added_past_the_source_rows_continues_at_the_last_two_rows_step() {
    let _face = fake();
    let mut doc = rows_at(&UNEVEN);
    let typed = "مرحبا\nبالعالم\nسلام\nمرحبا";
    let layout = text::text_edit_layout(&doc, 0, 0, typed, &wide()).expect("layout");
    text::rewrite_block(&mut doc, 0, 0, Some(typed), &wide()).expect("writes");
    // The fourth row is 16 pt under the third, as the third is under the second (not 22, the first step).
    let want = [720.0, 698.0, 682.0, 666.0];
    assert!(close(&baselines(&doc), &want), "{:?}", baselines(&doc));
    assert!(reported_where_written(&layout.row_rects, &doc, &want), "{:?} against {:?}", layout.row_rects, row_boxes(&doc));
    // Two more rows past them step on at 16 pt each.
    let mut doc = rows_at(&UNEVEN);
    text::rewrite_block(&mut doc, 0, 0, Some("مرحبا\nبالعالم\nسلام\nمرحبا\nسلام"), &wide()).expect("writes");
    assert!(close(&baselines(&doc), &[720.0, 698.0, 682.0, 666.0, 650.0]), "{:?}", baselines(&doc));
}

#[test]
fn a_row_removed_leaves_the_rows_kept_on_their_source_baselines() {
    let _face = fake();
    let source = [720.0, 698.0, 682.0, 670.0];
    let mut doc = rows_at(&source);
    assert_eq!(text::text_blocks(&doc, 0).expect("blocks")[0].rows.len(), 4);
    let typed = "مرحبا\nبالعالم\nسلام";
    let layout = text::text_edit_layout(&doc, 0, 0, typed, &wide()).expect("layout");
    text::rewrite_block(&mut doc, 0, 0, Some(typed), &wide()).expect("writes");
    assert!(close(&baselines(&doc), &source[..3]), "{:?}", baselines(&doc));
    assert!(reported_where_written(&layout.row_rects, &doc, &source[..3]), "{:?} against {:?}", layout.row_rects, row_boxes(&doc));
}

#[test]
fn a_line_spacing_asked_for_still_spaces_every_row_evenly() {
    let _face = fake();
    let mut doc = rows_at(&UNEVEN);
    let style = BlockStyle { line_spacing: Some(2.0), ..wide() };
    text::rewrite_block(&mut doc, 0, 0, Some("مرحبا\nبالعالم\nسلام"), &style).expect("writes");
    assert!(close(&baselines(&doc), &[720.0, 696.0, 672.0]), "{:?}", baselines(&doc));
}

#[test]
fn row_steps_stop_at_the_first_step_that_is_not_a_sane_number() {
    let steps = |across: &[Option<f64>], limit| text::row_steps(across.iter().copied(), 1.0, 12.0, 12.0, limit);
    assert_eq!(steps(&[Some(720.0), Some(698.0), Some(682.0)], 10), [22.0, 16.0]);
    // As many steps as rows are written, less one.
    assert_eq!(steps(&[Some(720.0), Some(698.0), Some(682.0)], 1), [22.0]);
    assert!(steps(&[Some(720.0), Some(698.0)], 0).is_empty());
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e300, -1e300, 698.0 - 14_400.5, 720.0, 730.0] {
        assert_eq!(steps(&[Some(720.0), Some(698.0), Some(bad), Some(670.0)], 10), [22.0], "{bad}");
    }
    assert!(steps(&[Some(f64::NAN), Some(698.0)], 10).is_empty());
    assert!(steps(&[Some(720.0), None, Some(682.0)], 10).is_empty());
    // A zero or non-finite scale or size gives no steps rather than a non-number.
    for (k, size, old) in [(0.0, 12.0, 12.0), (1.0, 12.0, 0.0), (f64::NAN, 12.0, 12.0), (1.0, f64::INFINITY, 12.0), (1.0, 0.0, 12.0)] {
        assert!(text::row_steps([Some(720.0), Some(698.0)].into_iter(), k, size, old, 10).is_empty(), "{k} {size} {old}");
    }
    // A step is scaled with the size: 22 pt at 12 pt is 33 at 18.
    assert_eq!(text::row_steps([Some(720.0), Some(698.0)].into_iter(), 1.0, 18.0, 12.0, 10), [33.0]);
    // Bounded by the rows written, not by the source's.
    let many = (0..1_000_000).map(|i| Some(-f64::from(i) * 12.0));
    assert_eq!(text::row_steps(many, 1.0, 12.0, 12.0, 300).len(), 300);
}

#[test]
fn a_paragraph_of_hundreds_of_rows_with_absurd_moves_retypes_bounded_and_never_panics() {
    let _face = fake();
    // 400 rows stepping 12 and 14 pt in turn, an absurd move after the 200th (a paragraph of its own then follows, at a
    // baseline that rounds away every step), and a row in a zero-size font.
    let mut content = String::from("BT /F1 12 Tf 300 790 Td");
    for i in 0..400 {
        let step = match i {
            200 => -1e300,
            _ if i % 2 == 0 => -12.0,
            _ => -14.0,
        };
        content.push_str(&format!(" (BAB) Tj 0 {step} Td"));
    }
    content.push_str(" /F1 0 Tf (BAB) Tj ET");
    let page = || arabic_ligature_page(&content, false);
    let blocks = text::text_blocks(&page(), 0).expect("blocks");
    let rows = blocks.first().map_or(0, |b| b.rows.len());
    assert!(rows >= 100, "{rows} rows");
    let started = std::time::Instant::now();
    for count in [1, 2, rows / 2, rows, rows + 50] {
        let typed = vec!["مرحبا"; count].join("\n");
        for style in [wide(), BlockStyle { size: Some(30.0), ..wide() }, BlockStyle { line_spacing: Some(1.0), ..wide() }] {
            let mut doc = page();
            text::rewrite_block(&mut doc, 0, 0, Some(&typed), &style).expect("writes");
            // Every move written is a number.
            let moves: Vec<f64> = ops(&doc).iter().filter(|o| o.is("Td")).flat_map(|o| o.operands.iter().filter_map(Object::as_f64)).collect();
            assert!(moves.iter().all(|v| v.is_finite()), "{count} {style:?}");
            let written = baselines(&doc);
            assert!(written.len() >= count.min(rows), "{count} {style:?}: {}", written.len());
            if style == wide() {
                // The rows the source has keep its steps: 12 and 14 pt in turn from 790.
                let want: Vec<f64> = (0..count.min(rows)).map(|i| 790.0 - (i / 2) as f64 * 26.0 - (i % 2) as f64 * 12.0).collect();
                assert!(close(written.get(..want.len()).unwrap_or_default(), &want), "{count}: {:?}", written.get(..4));
            }
        }
        for block in 1..blocks.len().min(4) {
            let mut doc = page();
            let _ = text::rewrite_block(&mut doc, 0, block, Some(&typed), &wide());
        }
    }
    assert!(started.elapsed().as_secs() < 120, "took {:?}", started.elapsed());
}

// ---- the real face ----

fn real_face() -> bool {
    if pdfcraft_fonts::document_arabic_font().is_none() {
        eprintln!("built without a craft-fonts Arabic face: the real-face checks need CRAFT_FONTS_DIR");
        return false;
    }
    true
}

#[test]
fn without_the_real_face_arabic_replacement_text_is_refused_by_name() {
    if pdfcraft_fonts::document_arabic_font().is_some() {
        return;
    }
    let mut doc = page();
    let before = bytes_of(&doc);
    let r = text::replace_block(&mut doc, 0, 0, "مرحبا بالعالم");
    let Err(EditError::Invalid(msg)) = r else { panic!("expected a clear error, got {r:?}") };
    assert!(msg.contains("Arabic") && msg.contains("CRAFT_FONTS_DIR"), "{msg}");
    assert_eq!(bytes_of(&doc), before);
    let r = text::replace_line(&mut doc, 0, 0, "مرحبا بالعالم");
    assert!(matches!(&r, Err(EditError::Invalid(m)) if m.contains("CRAFT_FONTS_DIR")), "{r:?}");
    assert_eq!(bytes_of(&doc), before);
}

#[test]
fn the_real_face_draws_lam_alef_with_both_letters_in_its_to_unicode() {
    if !real_face() {
        return;
    }
    let mut doc = page();
    let before = bytes_of(&doc);
    let edit = text::replace_block(&mut doc, 0, 0, "لا").expect("shaped with the real face");
    assert_eq!(edit.substituted.as_deref(), Some("Noto Sans Arabic Type3"));
    assert_ne!(bytes_of(&doc), before);
    assert_eq!(actual_texts(&doc), ["لا"]);
    let doc = reopen(&doc);
    let generated = arabic_fonts(&doc);
    assert_eq!(generated.len(), 1);
    let font = &generated[0].1;
    let Some(Object::Dict(procs)) = font.get(b"CharProcs") else { panic!("char procs") };
    // The face draws it as the final alef joined to an initial lam: one glyph per letter, each with
    // its own character in the ToUnicode map, whichever way the face forms the pair.
    assert!(procs.len() <= 2, "{} glyphs", procs.len());
    let Some(Object::Array(widths)) = font.get(b"Widths") else { panic!("widths") };
    assert!(widths.iter().all(|w| w.as_f64().is_some_and(|w| w > 0.0)), "{widths:?}");
    for (_, glyph) in procs.iter() {
        let Object::Stream(s) = &*doc.resolve(glyph) else { panic!("glyph stream") };
        let path = String::from_utf8(s.decoded().expect("decodes")).expect("text");
        assert!(path.contains(" d1") && path.contains(" l\n") && path.contains('f'), "an outline, not a box: {path}");
    }
    let cmap = to_unicode(&doc, font);
    let entries = cmap.split("beginbfchar").nth(1).unwrap_or_default().split("endbfchar").next().unwrap_or_default();
    let mapped: String = entries.lines().filter_map(|l| l.split_once("> <")).map(|(_, to)| to.trim_end_matches('>').to_owned()).collect();
    assert!(mapped == "06440627" || mapped == "06270644", "{cmap}");
    assert_eq!(line(&doc).text, "لا");
}

#[test]
fn the_real_face_round_trips_typed_text_by_block_and_line() {
    if !real_face() {
        return;
    }
    for typed in TYPED.into_iter().chain(["سَلَام", "كتاب 2026 جديد", "لا إله إلا الله"]) {
        for by_line in [false, true] {
            let mut doc = page();
            let edit = if by_line { text::replace_line(&mut doc, 0, 0, typed) } else { text::replace_block(&mut doc, 0, 0, typed) };
            edit.unwrap_or_else(|e| panic!("{typed} (line: {by_line}): {e}"));
            let doc = reopen(&doc);
            assert_eq!(line(&doc).text, typed, "{typed} (line: {by_line})");
            assert_eq!(text::text_blocks(&doc, 0).expect("blocks")[0].text, typed);
            for (_, font) in arabic_fonts(&doc) {
                let Some(Object::Array(w)) = font.get(b"Widths") else { panic!("widths") };
                assert!(w.iter().all(|w| w.as_f64().is_some_and(|w| w >= 0.0)));
            }
        }
    }
}

#[test]
fn the_real_face_justifies_a_wrapped_paragraph_to_its_width() {
    if !real_face() {
        return;
    }
    let style = BlockStyle { width: Some(120.0), align: Some(Align::Justify), ..Default::default() };
    let typed = "هذا نص عربي طويل يكتب في حقل ضيق ليلتف على عدة أسطر ويبقى مطابقا لما تكتبه الصفحة وهذا سطر آخر";
    let (doc, lines) = written(typed, &style);
    assert!(lines.len() >= 4, "{:?}", lines.iter().map(|l| l.rect).collect::<Vec<_>>());
    for l in &lines[..lines.len() - 1] {
        assert!((l.rect[0] - 300.0).abs() < 0.6 && (l.rect[2] - 420.0).abs() < 0.6, "{:?}: {:?}", l.text, l.rect);
    }
    let last = lines.last().expect("a last row");
    assert!((last.rect[2] - 420.0).abs() < 0.6 && last.rect[0] > 300.6, "{:?}", last.rect);
    // What the page says is the typed text, as the unjustified paragraph says it.
    assert_eq!(text::text_blocks(&reopen(&doc), 0).expect("blocks")[0].text.split_whitespace().collect::<Vec<_>>().join(" "), typed);
}

/// Three right-aligned rows (every glyph 6 pt wide at 12 pt) whose first, one glyph, starts 12 pt to the right of
/// the other two (a short title row): the paragraph's left edge (300) is the longer rows', not the first run's origin.
fn rows_with_a_short_first_row() -> Document {
    arabic_ligature_page("BT /F1 12 Tf 312 720 Td (B) Tj -12 -22 Td (BAB) Tj 0 -16 Td (BAB) Tj ET", false)
}

#[test]
fn a_retyped_paragraph_whose_first_row_starts_right_of_its_left_edge_keeps_its_right_edge() {
    let _face = fake();
    // A draft that fits the paragraph's own 18 pt width, and one in a width asked for.
    for (style, typed) in [(BlockStyle::default(), "\u{633}\u{644}"), (wide(), "مرحبا بالعالم")] {
        let mut doc = rows_with_a_short_first_row();
        let block = text::text_blocks(&doc, 0).expect("blocks").remove(0);
        assert_eq!(block.rows.len(), 3, "one paragraph of three rows");
        let (left, right) = (block.rect[0], block.rect[2]);
        assert!((left - 300.0).abs() < 0.5 && (right - 318.0).abs() < 0.5, "the paragraph's edges are the longer rows' ({left}, {right})");
        let layout = text::text_edit_layout(&doc, 0, 0, typed, &style).expect("layout");
        text::rewrite_block(&mut doc, 0, 0, Some(typed), &style).expect("writes");
        let rows = row_boxes(&doc);
        assert_eq!(rows.len(), 1, "{rows:?}");
        // The right-aligned row ends where the paragraph ended (or where the width asked for ends, from the
        // paragraph's left edge), not 60 pt further right, past the first run's origin.
        let want_right = style.width.map_or(right, |w| left + w);
        assert!((rows[0][2] - want_right).abs() < 0.5, "row {:?} should end at {want_right} (style width {:?})", rows[0], style.width);
        assert!(rows[0][0] >= left - 0.5, "row {:?} starts inside the paragraph ({left})", rows[0]);
        assert!(
            layout.row_rects.iter().zip(&rows).all(|(a, b)| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.01)),
            "{:?} vs {rows:?}",
            layout.row_rects
        );
    }
}
