//! Arabic in form fields: text, combo and list appearances are drawn shaped from Type 3 fonts, with
//! the typed order in `/ActualText`, right-aligned when nothing states an alignment, and with no
//! `?`, mark or `/NeedAppearances` when the Arabic face drew them. Most tests run with a stand-in
//! for the craft-fonts Arabic face (one cluster per character, half an em wide), so they run in any
//! build; the ones at the end use the real face and skip without it.

use pdfcraft_content::Op;
use pdfcraft_cos::{Dict, Document, Object, SaveOptions, write_full};
use pdfcraft_fonts::shaped_text::{Shaper, set_test_shaper};
use pdfcraft_fonts::{GlyphError, GlyphOutline, ShapedCluster, ShapedScript, helvetica_width};

use crate::tests::{document, field, reopen};
use crate::*;

// ---- the stand-in face ----

fn fake_shape(script: ShapedScript, text: &str, rtl: bool) -> Result<Vec<ShapedCluster>, GlyphError> {
    if script != ShapedScript::Arabic {
        return Err(GlyphError::Missing);
    }
    let mut out: Vec<ShapedCluster> = text
        .chars()
        .enumerate()
        .map(|(i, c)| {
            let advance = if matches!(u32::from(c), 0x064B..=0x065F) {
                0.0
            } else if c == '\u{0600}' {
                // Every one of these is a different shape: a hostile text can need thousands.
                0.5 + i as f64 * 1e-4
            } else {
                0.5
            };
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

// ---- the form ----

fn hex16(text: &str) -> String {
    text.encode_utf16().map(|u| format!("{u:04X}")).collect()
}

const GREETING: &str = "مرحبا بالعالم";

/// A form with fields drawn in 12 pt Helvetica: `t` (200 × 20), `m` (multiline, 100 × 100),
/// `c` (combo), `l` (list), `left` (`/Q 0`), `centre` (`/Q 1`), `comb` (a 5-cell comb, 200 × 20)
/// and `grp.kid` under a parent that states `/Q 0`. `form` is added to the AcroForm dictionary.
fn form(form: &str) -> Document {
    let da = "/DA (/Helv 12 Tf 0 g)";
    let opts = format!("/Opt [[(ar) <FEFF{}>] [(en) (English)]]", hex16("بالعالم"));
    let objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 600 800] >>".into(),
        "<< /Type /Page /Parent 2 0 R /Annots [6 0 R 7 0 R 8 0 R 9 0 R 10 0 R 11 0 R 13 0 R 15 0 R] >>".into(),
        format!(
            "<< /Fields [6 0 R 7 0 R 8 0 R 9 0 R 10 0 R 11 0 R 12 0 R 15 0 R] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 5 0 R /Cour 14 0 R >> >> {form} >>"
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".into(),
        format!("<< /Type /Annot /Subtype /Widget /FT /Tx /T (t) {da} /Rect [50 700 250 720] /P 3 0 R >>"),
        format!("<< /Type /Annot /Subtype /Widget /FT /Tx /T (m) /Ff 4096 {da} /Rect [50 500 150 600] /P 3 0 R >>"),
        format!("<< /Type /Annot /Subtype /Widget /FT /Ch /T (c) /Ff 131072 {da} {opts} /Rect [50 400 250 420] /P 3 0 R >>"),
        format!("<< /Type /Annot /Subtype /Widget /FT /Ch /T (l) {da} {opts} /Rect [50 300 150 360] /P 3 0 R >>"),
        format!("<< /Type /Annot /Subtype /Widget /FT /Tx /T (left) /Q 0 {da} /Rect [50 200 250 220] /P 3 0 R >>"),
        format!("<< /Type /Annot /Subtype /Widget /FT /Tx /T (centre) /Q 1 {da} /Rect [50 150 250 170] /P 3 0 R >>"),
        "<< /T (grp) /Q 0 /Kids [13 0 R] >>".into(),
        format!("<< /Type /Annot /Subtype /Widget /FT /Tx /T (kid) /Parent 12 0 R {da} /Rect [50 100 250 120] /P 3 0 R >>"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>".into(),
        format!("<< /Type /Annot /Subtype /Widget /FT /Tx /T (comb) /Ff 16777216 /MaxLen 5 {da} /Rect [50 50 250 70] /P 3 0 R >>"),
    ];
    document(&objs)
}

fn set(doc: &mut Document, name: &str, value: &str) {
    set_value(doc, name, &FieldValue::Text(value.into())).unwrap();
}

/// The normal appearance's content operators and `/Font` resources of the first widget of `name`.
fn appearance_of(doc: &Document, name: &str) -> (Vec<Op>, Dict) {
    let all = fields(doc);
    let w = &all.iter().find(|f| f.name == name).unwrap().widgets[0];
    let wd = doc.get(w.obj).as_dict().cloned().unwrap();
    let n = wd.get(b"AP").unwrap().as_dict().unwrap().reference(b"N").expect("normal appearance stream");
    let Object::Stream(s) = &*doc.get(n) else { panic!("an appearance stream") };
    let res = doc.resolve(s.dict.get(b"Resources").unwrap());
    let fonts = doc.resolve(res.as_dict().unwrap().get(b"Font").unwrap()).as_dict().cloned().unwrap();
    (pdfcraft_content::parse(&s.decoded().unwrap()).ops, fonts)
}

/// One drawn line: the `Tm` x it starts at, the typing-order text of its span (none for plain
/// text), its drawn width in points and the Latin strings shown in standard fonts.
#[derive(Debug)]
struct Line {
    x: f64,
    span: Option<String>,
    width: f64,
    latin: Vec<String>,
    fonts: Vec<String>,
    /// The text of the generated fonts' glyphs shown, read from their `ToUnicode` maps.
    glyphs: String,
}

/// Code → text of a generated Type 3 font's `ToUnicode` map (`<01> <0627>` lines).
fn to_unicode_of(doc: &Document, font: &Dict) -> std::collections::HashMap<u8, String> {
    let mut map = std::collections::HashMap::new();
    let Some(r) = font.get(b"ToUnicode") else { return map };
    let Object::Stream(s) = &*doc.resolve(r) else { return map };
    let text = String::from_utf8_lossy(&s.decoded().unwrap()).into_owned();
    for line in text.lines() {
        let hex: Vec<&str> = line.split(['<', '>']).map(str::trim).filter(|p| !p.is_empty()).collect();
        if let [code, unicode] = hex[..]
            && code.len() == 2
            && unicode.len() >= 4
            && let Ok(code) = u8::from_str_radix(code, 16)
        {
            let units: Vec<u16> = (0..unicode.len() / 4).filter_map(|i| u16::from_str_radix(unicode.get(i * 4..i * 4 + 4)?, 16).ok()).collect();
            map.insert(code, String::from_utf16_lossy(&units));
        }
    }
    map
}

fn lines_of(doc: &Document, ops: &[Op], fonts: &Dict) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::new();
    let (mut font, mut size) = (String::new(), 0.0);
    for op in ops {
        if op.is("Tm") {
            out.push(Line { x: op.num(4).unwrap(), span: None, width: 0.0, latin: Vec::new(), fonts: Vec::new(), glyphs: String::new() });
        } else if op.is("Tf") {
            font = String::from_utf8_lossy(op.name(0).unwrap()).into_owned();
            size = op.num(1).unwrap();
        } else if op.is("BDC") {
            let props = op.operands[1].as_dict().expect("inline properties");
            let bytes = &props.get(b"ActualText").unwrap().as_string().unwrap().bytes;
            assert_eq!(&bytes[..2], [0xFE, 0xFF]);
            let units: Vec<u16> = bytes[2..].chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            out.last_mut().expect("a Tm before the span").span = Some(String::from_utf16(&units).unwrap());
        } else if op.is("Tj") {
            let line = out.last_mut().expect("a Tm before the text");
            let bytes = &op.operands[0].as_string().unwrap().bytes;
            let font_obj = doc.resolve(fonts.get(font.as_bytes()).expect("the font is a resource"));
            let fd = font_obj.as_dict().unwrap();
            line.fonts.push(font.clone());
            if fd.name(b"Subtype") == Some(b"Type3") {
                let first = fd.get(b"FirstChar").and_then(Object::as_int).unwrap();
                let widths = fd.get(b"Widths").and_then(Object::as_array).unwrap();
                let to_unicode = to_unicode_of(doc, fd);
                for b in bytes {
                    let w = widths.get(usize::from(*b) - first as usize).and_then(Object::as_f64).unwrap();
                    line.width += w / 1000.0 * size;
                    line.glyphs.push_str(to_unicode.get(b).map_or("", String::as_str));
                }
            } else {
                let text: String = bytes.iter().map(|b| char::from(*b)).collect();
                line.width += helvetica_width(&text, size);
                line.latin.push(text);
            }
        }
    }
    out
}

fn fonts_named(fonts: &Dict, prefix: &str) -> Vec<String> {
    fonts.iter().map(|(k, _)| String::from_utf8_lossy(k).into_owned()).filter(|k| k.starts_with(prefix)).collect()
}

fn assert_unmarked(doc: &Document) {
    assert!(undrawn_fields(doc).is_empty(), "{:?}", undrawn_fields(doc));
    assert!(!need_appearances(doc), "no viewer is asked to redraw");
}

// ---- tests ----

fn right_aligned_shaped_text_field() {
    let mut doc = form("");
    set(&mut doc, "t", GREETING);
    assert_eq!(field(&fields(&doc), "t").value, [GREETING], "the value is stored as typed");
    assert_unmarked(&doc);
    let (ops, fonts) = appearance_of(&doc, "t");
    let drawn = fonts_named(&fonts, "PCFmAr");
    assert!(!drawn.is_empty(), "Type 3 fonts next to Helv: {:?}", fonts.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>());
    assert!(fonts.contains(b"Helv"));
    for name in &drawn {
        let font = doc.resolve(fonts.get(name.as_bytes()).unwrap());
        assert_eq!(font.as_dict().unwrap().name(b"Subtype"), Some(&b"Type3"[..]));
        assert!(font.as_dict().unwrap().contains(b"ToUnicode"));
    }
    let lines = lines_of(&doc, &ops, &fonts);
    assert_eq!(lines.len(), 1, "{lines:?}");
    let line = &lines[0];
    assert_eq!(line.span.as_deref(), Some(GREETING), "/ActualText holds the typed order");
    assert!(line.latin.is_empty() && line.fonts.iter().all(|f| f.starts_with("PCFmAr")), "{line:?}");
    assert!(line.width > 20.0, "{line:?}");
    // 200 pt wide, 2 pt of padding: the text ends at the right edge.
    assert!((line.x + line.width - 198.0).abs() < 0.01, "right-aligned: {line:?}");
    // No `?` shown in place of letters.
    assert!(ops.iter().filter(|o| o.is("Tj")).all(|o| o.operands[0].as_string().unwrap().bytes.iter().any(|b| *b != b'?')));
    // Kept through a save and reopen: the fonts are real objects.
    let doc = reopen(&doc);
    let (ops, fonts) = appearance_of(&doc, "t");
    assert_eq!(lines_of(&doc, &ops, &fonts)[0].span.as_deref(), Some(GREETING));
    assert_unmarked(&doc);
}

#[test]
fn a_text_field_with_arabic_is_drawn_shaped_and_right_aligned() {
    let _face = Using::shaper(FAKE);
    right_aligned_shaped_text_field();
}

#[test]
fn a_stated_alignment_is_kept() {
    let _face = Using::shaper(FAKE);
    let mut doc = form("");
    for name in ["left", "centre", "grp.kid", "t"] {
        set(&mut doc, name, GREETING);
    }
    let x_of = |doc: &Document, name: &str| {
        let (ops, fonts) = appearance_of(doc, name);
        let line = lines_of(doc, &ops, &fonts).remove(0);
        assert_eq!(line.span.as_deref(), Some(GREETING), "{name}");
        (line.x, line.width)
    };
    // `/Q 0` on the widget, and on the parent field: left, even though the text runs right to left.
    assert!((x_of(&doc, "left").0 - 2.0).abs() < 0.01);
    assert!((x_of(&doc, "grp.kid").0 - 2.0).abs() < 0.01);
    // `/Q 1`: centred.
    let (x, w) = x_of(&doc, "centre");
    assert!((x - (2.0 + (196.0 - w) / 2.0)).abs() < 0.01, "{x} {w}");
    // No `/Q` anywhere: right. An AcroForm that states `/Q 0` keeps it left.
    assert!(x_of(&doc, "t").0 > 50.0);
    let mut stated = form("/Q 0");
    set(&mut stated, "t", GREETING);
    assert!((x_of(&stated, "t").0 - 2.0).abs() < 0.01);
    assert_unmarked(&stated);
    // Left-to-right text is left-aligned without a `/Q`, shaped lines or not.
    set(&mut doc, "t", "Hello");
    let (ops, fonts) = appearance_of(&doc, "t");
    let lines = lines_of(&doc, &ops, &fonts);
    assert_eq!((lines[0].span.clone(), lines[0].x), (None, 2.0));
    assert_eq!(lines[0].latin, ["Hello"]);
    assert!(fonts_named(&fonts, "PCFmAr").is_empty(), "plain text adds no fonts");
}

#[test]
fn latin_text_and_digits_stay_in_place_in_arabic() {
    let _face = Using::shaper(FAKE);
    let mut doc = form("");
    let typed = "رقم 42 Report";
    set(&mut doc, "t", typed);
    assert_unmarked(&doc);
    let (ops, fonts) = appearance_of(&doc, "t");
    let lines = lines_of(&doc, &ops, &fonts);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].span.as_deref(), Some(typed), "ActualText is the logical order");
    // In a right-to-left paragraph the Latin words read left to right among themselves, with the
    // later ones to the left: "Report" is drawn before "42".
    assert_eq!(lines[0].latin, ["Report", "42"]);
    assert!(lines[0].fonts.iter().any(|f| f == "Helv") && lines[0].fonts.iter().any(|f| f.starts_with("PCFmAr")));
    assert!((lines[0].x + lines[0].width - 198.0).abs() < 0.01, "{:?}", lines[0]);
    // A left-to-right paragraph with Arabic in it starts at the left.
    set(&mut doc, "t", "Name: مرحبا");
    let (ops, fonts) = appearance_of(&doc, "t");
    let lines = lines_of(&doc, &ops, &fonts);
    assert_eq!(lines[0].span.as_deref(), Some("Name: مرحبا"));
    assert_eq!(lines[0].x, 2.0);
    // The form's own font draws the Latin when it holds it.
    let mut courier = form("");
    courier.update_dict(field(&fields(&courier), "t").obj, |d| d.set(b"DA".to_vec(), PdfString::text("/Cour 12 Tf 0 g"))).unwrap();
    set(&mut courier, "t", "رقم 42");
    let (ops, fonts) = appearance_of(&courier, "t");
    let lines = lines_of(&courier, &ops, &fonts);
    assert_eq!(lines[0].fonts.iter().filter(|f| !f.starts_with("PCFmAr")).collect::<Vec<_>>(), ["Cour"]);
}

#[test]
fn a_multiline_field_wraps_arabic_into_spans_that_fit() {
    let _face = Using::shaper(FAKE);
    let mut doc = form("");
    let typed = "مرحبا بالعالم الجميل في هذا اليوم المشمس الجميل";
    set(&mut doc, "m", typed);
    assert_unmarked(&doc);
    let (ops, fonts) = appearance_of(&doc, "m");
    let lines = lines_of(&doc, &ops, &fonts);
    assert!(lines.len() >= 3, "{lines:?}");
    // 100 pt wide, 2 pt of padding each side.
    for l in &lines {
        assert!(l.width <= 96.0 + 0.01, "{l:?}");
        assert!((l.x + l.width - 98.0).abs() < 0.01, "each row ends at the right edge: {l:?}");
    }
    let spans: Vec<&str> = lines.iter().map(|l| l.span.as_deref().unwrap()).collect();
    assert_eq!(spans.join(" "), typed, "the spans are the typed text, row by row");
    // Paragraphs are laid out on their own: Latin and Arabic, one each.
    set(&mut doc, "m", "Hello there\nمرحبا بالعالم");
    let (ops, fonts) = appearance_of(&doc, "m");
    let lines = lines_of(&doc, &ops, &fonts);
    assert_eq!(lines.len(), 2);
    assert_eq!((lines[0].span.clone(), lines[0].x), (None, 2.0));
    assert_eq!(lines[1].span.as_deref(), Some("مرحبا بالعالم"));
    assert!((lines[1].x + lines[1].width - 98.0).abs() < 0.01, "{:?}", lines[1]);
}

#[test]
fn combo_and_list_options_with_arabic_are_drawn_shaped() {
    let _face = Using::shaper(FAKE);
    let mut doc = form("");
    set_value(&mut doc, "c", &FieldValue::Choice(vec!["ar".into()])).unwrap();
    set_value(&mut doc, "l", &FieldValue::Choice(vec!["ar".into()])).unwrap();
    assert_unmarked(&doc);
    // The combo shows the selected option's display text, right-aligned.
    let (ops, fonts) = appearance_of(&doc, "c");
    let lines = lines_of(&doc, &ops, &fonts);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].span.as_deref(), Some("بالعالم"));
    assert!((lines[0].x + lines[0].width - 198.0).abs() < 0.01, "{:?}", lines[0]);
    // The list draws every option: the Arabic one shaped and right-aligned, the other plain.
    let (ops, fonts) = appearance_of(&doc, "l");
    let lines = lines_of(&doc, &ops, &fonts);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].span.as_deref(), Some("بالعالم"));
    assert!((lines[0].x + lines[0].width - 98.0).abs() < 0.01, "{:?}", lines[0]);
    assert_eq!((lines[1].span.clone(), lines[1].latin.clone(), lines[1].x), (None, vec!["English".to_string()], 2.0));
    // Choosing the Latin option again draws it plain.
    set_value(&mut doc, "c", &FieldValue::Choice(vec!["en".into()])).unwrap();
    let (ops, fonts) = appearance_of(&doc, "c");
    let lines = lines_of(&doc, &ops, &fonts);
    assert_eq!((lines[0].span.clone(), lines[0].latin.clone()), (None, vec!["English".to_string()]));
    assert_unmarked(&doc);
}

#[test]
fn hostile_arabic_values_never_panic_and_fall_back_to_the_marked_drawing() {
    let _face = Using::shaper(FAKE);
    let long: String = "مرحبا ".repeat(3400);
    assert!(long.chars().count() > 20_000);
    let many_shapes: String = "\u{0600}".repeat(5000);
    let cases: Vec<(&str, String)> = vec![
        ("lone marks", "\u{064E}\u{064E}\u{0651}".into()),
        ("controls only", "\u{202E}\u{200F}\u{061C}\u{2067}".into()),
        ("controls and letters", "\u{202E}ب\u{200F}\u{061C}ا\u{2067}".into()),
        ("NUL and tab", "ب\0ا\tب".into()),
        ("a lone surrogate-adjacent char", "ب\u{FFFD}\u{10FFFF}".into()),
        ("20,000 characters", long.clone()),
        ("an outline that is too complex", "ب\u{06E5}ب".into()),
        ("more than 4,096 shapes", many_shapes),
        ("Arabic next to CJK", "ب日本".into()),
    ];
    for (what, text) in cases {
        for name in ["t", "m", "kid"] {
            let mut doc = form("");
            let name = if name == "kid" { "grp.kid" } else { name };
            let value = FieldValue::Text(text.clone());
            let before = write_full(&doc, &SaveOptions::default()).unwrap();
            set_value(&mut doc, name, &value).unwrap_or_else(|e| panic!("{what} in {name}: {e}"));
            let all = fields(&doc);
            assert!(!field(&all, name).widgets.is_empty());
            // Whatever was drawn, the document still saves and reads back.
            let again = reopen(&doc);
            assert_eq!(fields(&again).len(), all.len(), "{what} in {name}");
            assert_ne!(write_full(&doc, &SaveOptions::default()).unwrap(), before);
        }
    }
    // A shaping failure is reported like any undrawable text and adds nothing to the document.
    for text in ["ب\u{06E5}ب", "ب日本"] {
        let mut doc = form("");
        let f = field(&fields(&doc), "t").clone();
        let before = write_full(&doc, &SaveOptions::default()).unwrap();
        let (_, missing) = appearance::field_appearance_drawn(&mut doc, &f, &f.widgets[0], &[text.to_owned()], true);
        assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before, "no fonts are left behind");
        assert!(missing.is_some(), "{text}: the first character drawn as `?` is reported");
        set(&mut doc, "t", text);
        assert_eq!(undrawn_fields(&doc).len(), 1, "{text}");
        assert!(need_appearances(&doc));
    }
    // The 20,000-character value is shaped (a person could paste it): one span, nothing marked.
    let mut doc = form("");
    set(&mut doc, "m", &long);
    assert_unmarked(&doc);
}

#[test]
fn without_the_arabic_face_the_value_is_kept_marked_and_redrawn_elsewhere() {
    let _face = Using::shaper(NO_FACE);
    let mut doc = form("");
    set(&mut doc, "t", GREETING);
    assert_eq!(field(&fields(&doc), "t").value, [GREETING]);
    assert_eq!(undrawn_fields(&doc), [("t".to_string(), 'م')]);
    assert!(need_appearances(&doc));
    let (ops, fonts) = appearance_of(&doc, "t");
    assert!(fonts_named(&fonts, "PCFmAr").is_empty());
    let lines = lines_of(&doc, &ops, &fonts);
    assert_eq!((lines[0].span.clone(), lines[0].latin.clone()), (None, vec!["????? ???????".to_string()]));
}

#[test]
fn refilling_with_latin_text_clears_the_shaped_appearance() {
    let _face = Using::shaper(FAKE);
    let mut doc = form("");
    set(&mut doc, "t", GREETING);
    set(&mut doc, "t", "Plain");
    let (ops, fonts) = appearance_of(&doc, "t");
    assert!(fonts_named(&fonts, "PCFmAr").is_empty());
    let lines = lines_of(&doc, &ops, &fonts);
    assert_eq!((lines[0].span.clone(), lines[0].latin.clone()), (None, vec!["Plain".to_string()]));
    assert_unmarked(&doc);
}

#[test]
fn redrawing_a_shaped_field_keeps_it_shaped() {
    let _face = Using::shaper(FAKE);
    let mut doc = form("");
    set(&mut doc, "t", GREETING);
    redraw_field(&mut doc, "t").unwrap();
    let (ops, fonts) = appearance_of(&doc, "t");
    let names = fonts_named(&fonts, "PCFmAr");
    assert_eq!(names.len(), 1, "{names:?}");
    assert_eq!(lines_of(&doc, &ops, &fonts)[0].span.as_deref(), Some(GREETING));
    assert_unmarked(&doc);
}

#[test]
fn the_real_arabic_face_shapes_a_form_field() {
    if pdfcraft_fonts::document_arabic_font().is_none() {
        eprintln!("set CRAFT_FONTS_DIR to exercise Arabic in form fields");
        return;
    }
    right_aligned_shaped_text_field();
    let mut doc = form("");
    let typed = "رقم 42 (Report) سَلَام";
    set(&mut doc, "t", typed);
    set(&mut doc, "m", "مرحبا بالعالم الجميل في هذا اليوم المشمس الجميل");
    set_value(&mut doc, "c", &FieldValue::Choice(vec!["ar".into()])).unwrap();
    assert_unmarked(&doc);
    let (ops, fonts) = appearance_of(&doc, "t");
    let lines = lines_of(&doc, &ops, &fonts);
    assert_eq!(lines[0].span.as_deref(), Some(typed));
    assert!((lines[0].x + lines[0].width - 198.0).abs() < 0.01, "{:?}", lines[0]);
    assert!(lines[0].latin.contains(&"42".to_string()) && lines[0].latin.iter().any(|s| s.contains("Report")), "{:?}", lines[0].latin);
    let (ops, fonts) = appearance_of(&doc, "m");
    let rows = lines_of(&doc, &ops, &fonts);
    assert!(rows.len() >= 3);
    assert!(rows.iter().all(|r| r.width <= 96.0 + 0.01));
    // A comb field and a button caption with the real face: five cells for five letters, one span.
    set(&mut doc, "comb", "مرحبا");
    button(&mut doc, "موافق");
    assert_unmarked(&doc);
    let (ops, fonts) = appearance_of(&doc, "comb");
    let cells = lines_of(&doc, &ops, &fonts);
    assert_eq!(cells.len(), 5, "{cells:?}");
    assert_eq!(cells.iter().filter_map(|l| l.span.as_deref()).collect::<Vec<_>>(), ["مرحبا"]);
    assert!(cells.windows(2).all(|w| w[0].x < w[1].x) && cells.iter().all(|l| l.latin.is_empty()));
    let (ops, fonts) = appearance_of(&doc, "btn");
    assert_eq!(lines_of(&doc, &ops, &fonts)[0].span.as_deref(), Some("موافق"));
    // Hostile input with the real face: no panic, whatever is drawn.
    for text in ["\u{064E}\u{064E}", "\u{202E}\u{200F}\u{061C}", "ب\0ا", "\u{FDFD}", &"ب".repeat(20_000)] {
        set(&mut doc, "t", text);
        force(&mut doc, "comb", text);
        let _ = reopen(&doc);
    }
}

// ---- comb fields ----

/// Store `text` as the value of `name` without the checks `set_value` makes (a value longer than a
/// comb's cells, as another program may have written it) and redraw the field.
fn force(doc: &mut Document, name: &str, text: &str) {
    let obj = field(&fields(doc), name).obj;
    doc.update_dict(obj, |d| d.set(b"V".to_vec(), PdfString::text(text))).unwrap();
    redraw_field(doc, name).unwrap();
}

/// The glyph texts of a comb field's cells from left to right, with the cell each starts in.
fn comb_cells_of(doc: &Document, name: &str) -> (Vec<(usize, String)>, Vec<Line>) {
    let (ops, fonts) = appearance_of(doc, name);
    let lines = lines_of(doc, &ops, &fonts);
    // 200 pt wide, five cells of 40 pt.
    let cells = lines.iter().map(|l| ((l.x / 40.0) as usize, l.glyphs.clone())).collect();
    (cells, lines)
}

#[test]
fn an_arabic_comb_value_draws_one_cluster_per_cell_right_to_left() {
    let _face = Using::shaper(FAKE);
    let mut doc = form("");
    set(&mut doc, "comb", "مرحبا");
    assert_eq!(field(&fields(&doc), "comb").value, ["مرحبا"], "the value is stored as typed");
    assert_unmarked(&doc);
    let (ops, fonts) = appearance_of(&doc, "comb");
    assert!(!fonts_named(&fonts, "PCFmAr").is_empty(), "drawn from generated fonts");
    let lines = lines_of(&doc, &ops, &fonts);
    assert_eq!(lines.len(), 5, "{lines:?}");
    // Left to right the cells hold the letters in drawing order: the first typed letter is in the
    // rightmost cell.
    assert_eq!(lines.iter().map(|l| l.glyphs.as_str()).collect::<Vec<_>>(), ["ا", "ب", "ح", "ر", "م"]);
    for (i, l) in lines.iter().enumerate() {
        // Centred in its 40 pt cell: a half-em cluster at 12 pt is 6 pt wide.
        assert!((l.x - (i as f64 * 40.0 + 17.0)).abs() < 0.01, "cell {i}: {l:?}");
        assert!(l.latin.is_empty(), "no `?` or other Latin text: {l:?}");
    }
    // One span for the whole value, in typing order.
    let spans: Vec<&str> = lines.iter().filter_map(|l| l.span.as_deref()).collect();
    assert_eq!(spans, ["مرحبا"]);
    let content = String::from_utf8_lossy(&pdfcraft_content::serialize_ops(&ops)).into_owned();
    assert_eq!(content.matches("BDC").count(), 1, "{content}");
    assert_eq!(content.matches("EMC").count(), 2, "the span and the field's own marked content: {content}");
    // Kept through a save and reopen.
    let again = reopen(&doc);
    let (ops, fonts) = appearance_of(&again, "comb");
    assert_eq!(lines_of(&again, &ops, &fonts).len(), 5);
    assert_unmarked(&again);
}

#[test]
fn a_short_arabic_comb_value_fills_cells_from_the_right() {
    let _face = Using::shaper(FAKE);
    let mut doc = form("");
    set(&mut doc, "comb", "مرح");
    assert_unmarked(&doc);
    let (cells, lines) = comb_cells_of(&doc, "comb");
    assert_eq!(cells, [(2, "ح".to_string()), (3, "ر".to_string()), (4, "م".to_string())]);
    assert_eq!(lines.iter().filter_map(|l| l.span.as_deref()).collect::<Vec<_>>(), ["مرح"]);
    // More characters than cells: the first five are kept, as for Latin text.
    force(&mut doc, "comb", "مرحباب");
    let (cells, lines) = comb_cells_of(&doc, "comb");
    assert_eq!(cells.len(), 5);
    assert_eq!(cells.last(), Some(&(4, "م".to_string())));
    assert_eq!(lines.iter().filter_map(|l| l.span.as_deref()).collect::<Vec<_>>(), ["مرحبا"]);
    assert_unmarked(&doc);
    // Digits and Latin letters in the value fill cells of their own, read left to right.
    set(&mut doc, "comb", "مر12");
    assert_unmarked(&doc);
    let (_, lines) = comb_cells_of(&doc, "comb");
    assert_eq!(lines.len(), 4);
    assert_eq!(
        lines.iter().map(|l| (l.glyphs.as_str(), l.latin.concat())).collect::<Vec<_>>(),
        [("", "1".to_string()), ("", "2".to_string()), ("ر", String::new()), ("م", String::new())]
    );
    assert_eq!(lines.iter().filter_map(|l| l.span.as_deref()).collect::<Vec<_>>(), ["مر12"]);
}

#[test]
fn a_latin_comb_value_is_drawn_as_before() {
    let _face = Using::shaper(FAKE);
    let mut doc = form("");
    set(&mut doc, "comb", "ABCDE");
    assert_unmarked(&doc);
    let (ops, fonts) = appearance_of(&doc, "comb");
    assert!(fonts_named(&fonts, "PCFmAr").is_empty(), "plain text adds no fonts");
    let lines = lines_of(&doc, &ops, &fonts);
    assert_eq!(lines.len(), 5);
    for (i, (l, ch)) in lines.iter().zip("ABCDE".chars()).enumerate() {
        assert_eq!((l.span.clone(), l.latin.clone()), (None, vec![ch.to_string()]));
        let expected = i as f64 * 40.0 + (40.0 - helvetica_width(&ch.to_string(), 12.0)) / 2.0;
        assert!((l.x - expected).abs() < 0.01, "cell {i}: {l:?}");
    }
    // Unwritten cells stay empty, and the text is left to right from the first cell.
    set(&mut doc, "comb", "AB");
    let (_, lines) = comb_cells_of(&doc, "comb");
    assert_eq!(lines.len(), 2);
    assert!(lines[0].x < 40.0 && lines[1].x < 80.0);
}

#[test]
fn hostile_comb_values_never_panic() {
    let _face = Using::shaper(FAKE);
    let long: String = "مرحبا ".repeat(3400);
    let many_shapes: String = "\u{0600}".repeat(5000);
    let cases: Vec<(&str, String)> = vec![
        ("lone marks", "\u{064E}\u{064E}\u{0651}".into()),
        ("controls only", "\u{202E}\u{200F}\u{061C}\u{2067}".into()),
        ("controls and letters", "\u{202E}ب\u{200F}\u{061C}ا\u{2067}".into()),
        ("NUL and tab", "ب\0ا\tب".into()),
        ("20,000 characters", long),
        ("an outline that is too complex", "ب\u{06E5}ب".into()),
        ("more than 4,096 shapes", many_shapes),
        ("Arabic next to CJK", "ب日本".into()),
        ("marks first", "\u{064E}ب".into()),
    ];
    for (what, text) in cases {
        let mut doc = form("");
        force(&mut doc, "comb", &text);
        let again = reopen(&doc);
        assert_eq!(fields(&again).len(), fields(&doc).len(), "{what}");
        let (ops, fonts) = appearance_of(&doc, "comb");
        let lines = lines_of(&doc, &ops, &fonts);
        assert!(lines.len() <= 5, "{what}: {} cells", lines.len());
    }
    // What the face can't draw falls back to `?` in the cells and is reported, as before.
    for text in ["ب\u{06E5}ب", "ب日本"] {
        let mut doc = form("");
        force(&mut doc, "comb", text);
        assert_eq!(undrawn_fields(&doc).len(), 1, "{text}");
        let (ops, fonts) = appearance_of(&doc, "comb");
        assert!(fonts_named(&fonts, "PCFmAr").is_empty());
        assert!(lines_of(&doc, &ops, &fonts).iter().all(|l| l.span.is_none()));
    }
}

#[test]
fn without_the_arabic_face_a_comb_value_is_kept_marked_and_drawn_with_question_marks() {
    let _face = Using::shaper(NO_FACE);
    let mut doc = form("");
    set(&mut doc, "comb", "مرحبا");
    assert_eq!(field(&fields(&doc), "comb").value, ["مرحبا"]);
    assert_eq!(undrawn_fields(&doc), [("comb".to_string(), 'م')]);
    let (_, lines) = comb_cells_of(&doc, "comb");
    assert_eq!(lines.iter().map(|l| l.latin.concat()).collect::<Vec<_>>(), ["?"; 5]);
}

// ---- push-button captions ----

fn button(doc: &mut Document, caption: &str) {
    add_field(doc, 0, [50.0, 20.0, 200.0, 50.0], &NewField::Button { caption: caption.into() }, Some("btn")).unwrap();
}

#[test]
fn an_arabic_button_caption_is_shaped_centred_and_unmarked() {
    let _face = Using::shaper(FAKE);
    let mut doc = form("");
    button(&mut doc, "موافق");
    assert_unmarked(&doc);
    let (ops, fonts) = appearance_of(&doc, "btn");
    assert!(!fonts_named(&fonts, "PCFmAr").is_empty(), "drawn from a generated font");
    let lines = lines_of(&doc, &ops, &fonts);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].span.as_deref(), Some("موافق"), "the caption in typing order");
    assert!(lines[0].latin.is_empty(), "no `?`: {:?}", lines[0]);
    assert_eq!(lines[0].glyphs.chars().rev().collect::<String>(), "موافق");
    // Centred in the 150 pt button.
    assert!((lines[0].x + lines[0].width / 2.0 - 75.0).abs() < 0.01, "{:?}", lines[0]);
    // Through a save, and when the caption is changed with the field's properties.
    let mut doc = reopen(&doc);
    assert_eq!(lines_of(&doc, &appearance_of(&doc, "btn").0, &appearance_of(&doc, "btn").1)[0].span.as_deref(), Some("موافق"));
    set_props(&mut doc, "btn", &FieldProps { caption: Some((0, "إلغاء".into())), ..Default::default() }).unwrap();
    let (ops, fonts) = appearance_of(&doc, "btn");
    assert_eq!(lines_of(&doc, &ops, &fonts)[0].span.as_deref(), Some("إلغاء"));
    assert_unmarked(&doc);
    // A Latin caption is drawn as before, and a mixed one keeps its Latin letters in Helvetica.
    set_props(&mut doc, "btn", &FieldProps { caption: Some((0, "OK".into())), ..Default::default() }).unwrap();
    let (ops, fonts) = appearance_of(&doc, "btn");
    let lines = lines_of(&doc, &ops, &fonts);
    assert_eq!((lines[0].span.clone(), lines[0].latin.clone()), (None, vec!["OK".to_string()]));
    assert!(fonts_named(&fonts, "PCFmAr").is_empty());
    set_props(&mut doc, "btn", &FieldProps { caption: Some((0, "OK موافق".into())), ..Default::default() }).unwrap();
    let (ops, fonts) = appearance_of(&doc, "btn");
    let lines = lines_of(&doc, &ops, &fonts);
    assert_eq!(lines[0].span.as_deref(), Some("OK موافق"));
    assert!(lines[0].latin.iter().any(|s| s.contains("OK")));
    assert_unmarked(&doc);
}

#[test]
fn hostile_or_undrawable_button_captions_never_panic() {
    let _face = Using::shaper(FAKE);
    let long = "مرحبا ".repeat(3400);
    for text in ["\u{064E}\u{064E}", "\u{202E}\u{200F}\u{061C}", "ب\0ا", "ب\u{06E5}ب", "ب日本", &long, ""] {
        let mut doc = form("");
        button(&mut doc, text);
        let _ = reopen(&doc);
    }
    // The face can't draw it: `?` and the mark, as before.
    let mut doc = form("");
    button(&mut doc, "ب日本");
    assert_eq!(undrawn_fields(&doc).len(), 1);
    let (ops, fonts) = appearance_of(&doc, "btn");
    assert!(lines_of(&doc, &ops, &fonts).iter().all(|l| l.span.is_none()));
    let _no_face = Using::shaper(NO_FACE);
    let mut doc = form("");
    button(&mut doc, "موافق");
    assert_eq!(undrawn_fields(&doc), [("btn".to_string(), 'م')]);
}
