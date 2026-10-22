//! Right-to-left text in the exports. The writers are XML, HTML and RTF text writers, so Arabic and
//! Hebrew need no font: the text is written as it is (UTF-8, or `\uN` in RTF), and each paragraph
//! whose first strong character runs right to left says so where its format has a way to
//! (`<w:bidi/>` and `<w:rtl/>` in Word, `rtl="1"` in PowerPoint, `dir="rtl"` in HTML, `\rtlpar` in
//! RTF), so the paragraph is right-aligned and its digits and Latin words land in order.

use super::*;
use crate::tests::part;

const ARABIC: &str = "مرحبا بالعالم";
const HEBREW: &str = "שלום עולם";

fn block(text: &str, y: f64, size: f64, bold: bool) -> Block {
    Block { text: text.into(), rect: [72.0, y, 500.0, y + size], size, bold, italic: false, color: [0.0; 3], rotation: 0.0 }
}

/// A page of body paragraphs: an Arabic one, a Latin one, a Latin one with Arabic in it, and an
/// Arabic one with digits and Latin words.
fn page() -> Page {
    Page {
        width: 612.0,
        height: 792.0,
        blocks: vec![
            block(ARABIC, 700.0, 11.0, false),
            block("Plain English text of the body.", 650.0, 11.0, false),
            block("Name: مرحبا", 600.0, 11.0, false),
            block("مرحبا 123 abc", 550.0, 11.0, false),
        ],
        images: Vec::new(),
    }
}

/// The paragraphs of a Word body: the `<w:p>…</w:p>` strings.
fn paragraphs(xml: &str) -> Vec<&str> {
    xml.split("<w:p>").skip(1).collect()
}

fn paragraph_with<'a>(xml: &'a str, needle: &str) -> &'a str {
    paragraphs(xml).into_iter().find(|p| p.contains(needle)).unwrap_or_else(|| panic!("{needle} missing in {xml}"))
}

#[test]
fn direction_follows_the_first_strong_character() {
    assert!(is_rtl(ARABIC));
    assert!(is_rtl(HEBREW));
    assert!(is_rtl("123 مرحبا"), "digits are weak");
    assert!(is_rtl("(مرحبا) abc"));
    assert!(!is_rtl("Name: مرحبا"));
    assert!(!is_rtl("123"));
    assert!(!is_rtl(""));
    assert!(is_rtl("\u{200F}"), "a right-to-left mark is strong");
}

#[test]
fn word_marks_right_to_left_paragraphs_and_runs() {
    let xml = part(&docx(&[page()], "Report"), "word/document.xml");
    let arabic = paragraph_with(&xml, ARABIC);
    assert!(arabic.starts_with("<w:pPr><w:bidi/></w:pPr>"), "{arabic}");
    assert!(arabic.contains("<w:rtl/>"), "{arabic}");
    assert!(arabic.contains("<w:szCs w:val=\"22\"/>"), "complex-script size follows the size: {arabic}");
    assert!(arabic.contains(">مرحبا بالعالم</w:t>"), "the text is written as it is: {arabic}");
    assert!(!arabic.contains('?'));
    // Mixed text starting with Arabic is right to left too.
    let mixed = paragraph_with(&xml, "123 abc");
    assert!(mixed.contains("<w:bidi/>") && mixed.contains("<w:rtl/>"), "{mixed}");
    // Latin paragraphs, including one with Arabic later in it, are untouched.
    for text in ["Plain English", "Name: مرحبا"] {
        let p = paragraph_with(&xml, text);
        assert!(!p.contains("<w:bidi/>") && !p.contains("<w:rtl/>") && !p.contains("szCs"), "{p}");
    }
}

#[test]
fn word_keeps_the_element_order_the_schema_asks_for() {
    let mut p = page();
    p.blocks.push(block("عنوان كبير", 750.0, 24.0, true));
    let xml = part(&docx(&[p], "Report"), "word/document.xml");
    let title = paragraph_with(&xml, "عنوان");
    // pStyle, then bidi in the paragraph; b, bCs, sz, szCs, rtl in the run.
    assert!(title.starts_with("<w:pPr><w:pStyle w:val=\"Heading1\"/><w:bidi/></w:pPr>"), "{title}");
    let run: Vec<usize> = ["<w:b/>", "<w:bCs/>", "<w:sz ", "<w:szCs ", "<w:rtl/>"]
        .iter()
        .map(|t| title.find(t).unwrap_or_else(|| panic!("{t} missing in {title}")))
        .collect();
    assert!(run.windows(2).all(|w| w[0] < w[1]), "{run:?} {title}");
}

#[test]
fn word_marks_right_to_left_cells_and_text_boxes() {
    let cell = |t: &str, x: f64, y: f64| Block { rect: [x, y, x + 100.0, y + 12.0], ..block(t, y, 11.0, false) };
    let table = Page {
        width: 612.0,
        height: 792.0,
        blocks: vec![
            cell(ARABIC, 72.0, 620.0),
            cell("Unit", 232.0, 620.0),
            cell("مرحبا", 72.0, 590.0),
            cell("kWh", 232.0, 590.0),
            block("Notes follow the table and are long enough to be the body text size of the page here.", 500.0, 11.0, false),
        ],
        images: Vec::new(),
    };
    let xml = part(&docx(&[table], "Table"), "word/document.xml");
    assert!(xml.contains("<w:tbl>"), "{xml}");
    assert!(paragraph_with(&xml, ARABIC).contains("<w:bidi/>"));
    assert!(!paragraph_with(&xml, ">Unit<").contains("<w:bidi/>"));
    // A page kept as positioned text boxes (rotated text).
    let mut rotated = page();
    rotated.blocks[0].rotation = 45.0;
    let xml = part(&docx(&[rotated], "Shapes"), "word/document.xml");
    assert!(xml.contains("position:absolute"), "{xml}");
    let boxed = paragraph_with(&xml, ARABIC);
    assert!(boxed.starts_with("<w:pPr><w:bidi/><w:spacing "), "bidi comes before spacing: {boxed}");
}

#[test]
fn powerpoint_paragraphs_run_right_to_left_and_right_aligned() {
    let mut p = page();
    p.blocks.insert(0, block("عنوان", 760.0, 28.0, true));
    let xml = part(&pptx(&[p], "Deck"), "ppt/slides/slide1.xml");
    for text in ["عنوان", ARABIC, "مرحبا 123 abc"] {
        let at = xml.find(&format!("<a:t>{text}")).unwrap_or_else(|| panic!("{text} missing in {xml}"));
        let para = &xml[xml[..at].rfind("<a:p>").unwrap()..at];
        assert!(para.starts_with("<a:p><a:pPr algn=\"r\" rtl=\"1\"/>"), "{para}");
    }
    for text in ["Plain English", "Name: مرحبا"] {
        let at = xml.find(&format!("<a:t>{text}")).unwrap();
        let para = &xml[xml[..at].rfind("<a:p>").unwrap()..at];
        assert!(para.starts_with("<a:p><a:r>"), "{para}");
    }
    assert!(!xml.contains("<a:t>?") && !xml.contains("?</a:t>"), "{xml}");
}

#[test]
fn powerpoint_table_cells_run_right_to_left() {
    let cell = |t: &str, x: f64, y: f64| Block { rect: [x, y, x + 100.0, y + 12.0], ..block(t, y, 11.0, false) };
    let page = Page {
        width: 720.0,
        height: 540.0,
        blocks: vec![
            cell("Title text for the slide", 72.0, 500.0),
            cell(ARABIC, 72.0, 420.0),
            cell("Unit", 232.0, 420.0),
            cell("مرحبا", 72.0, 390.0),
            cell("kWh", 232.0, 390.0),
            block("Notes follow the table and are long enough to be the body text size of the page here.", 300.0, 11.0, false),
        ],
        images: Vec::new(),
    };
    let xml = part(&pptx(&[page], "Deck"), "ppt/slides/slide1.xml");
    assert!(xml.contains("<a:tbl>"), "{xml}");
    assert!(xml.contains("<a:p><a:pPr algn=\"r\" rtl=\"1\"/><a:r>"), "{xml}");
    assert_eq!(xml.matches("rtl=\"1\"").count(), 2, "only the two Arabic cells: {xml}");
}

#[test]
fn html_gives_right_to_left_paragraphs_a_direction() {
    let mut p = page();
    p.blocks.push(block("عنوان كبير", 760.0, 24.0, true));
    p.blocks.push(block(HEBREW, 500.0, 11.0, false));
    let h = html(&[p], "Report");
    assert!(h.contains(&format!("<p dir=\"rtl\">{ARABIC}</p>")), "{h}");
    assert!(h.contains(&format!("<p dir=\"rtl\">{HEBREW}</p>")), "{h}");
    assert!(h.contains("<h1 dir=\"rtl\">عنوان كبير</h1>"), "{h}");
    assert!(h.contains("<p>Plain English text of the body.</p>"), "{h}");
    assert!(h.contains("<p>Name: مرحبا</p>"), "{h}");
    assert!(h.contains("<meta charset=\"utf-8\">"));
}

#[test]
fn html_table_cells_get_a_direction_and_keep_their_colour_style() {
    let cell = |t: &str, x: f64, y: f64| Block { rect: [x, y, x + 100.0, y + 12.0], color: [1.0, 0.0, 0.0], ..block(t, y, 11.0, false) };
    let page = Page {
        width: 612.0,
        height: 792.0,
        blocks: vec![
            cell(ARABIC, 72.0, 620.0),
            cell("Unit", 232.0, 620.0),
            cell("مرحبا", 72.0, 590.0),
            cell("kWh", 232.0, 590.0),
            block("Notes follow the table and are long enough to be the body text size of the page here.", 500.0, 11.0, false),
        ],
        images: Vec::new(),
    };
    let h = html(&[page], "Table");
    assert!(h.contains(&format!("<td dir=\"rtl\" style=\"color:#FF0000\">{ARABIC}</td>")), "{h}");
    assert!(h.contains("<td style=\"color:#FF0000\">Unit</td>"), "{h}");
}

#[test]
fn rtf_writes_the_text_as_unicode_escapes_in_right_to_left_paragraphs() {
    let r = rtf(&[page()]);
    // م is U+0645 = 1605.
    assert!(r.contains("\\u1605?"), "{r}");
    let arabic = r.lines().find(|l| l.contains("\\u1605?\\u1585?")).unwrap_or_else(|| panic!("{r}"));
    assert!(arabic.starts_with("{\\pard\\rtlpar\\qr\\fs22\\afs22 "), "{arabic}");
    assert!(arabic.ends_with("\\par}"), "{arabic}");
    let latin = r.lines().find(|l| l.contains("Plain English")).unwrap();
    assert!(!latin.contains("rtlpar") && !latin.contains("\\afs"), "{latin}");
    let mixed_latin_first = r.lines().find(|l| l.contains("Name: ")).unwrap();
    assert!(!mixed_latin_first.contains("rtlpar"), "{mixed_latin_first}");
    // Bold and italic carry over to the complex-script formatting.
    let mut p = page();
    p.blocks[0].bold = true;
    p.blocks[0].italic = true;
    let r = rtf(&[p]);
    let arabic = r.lines().find(|l| l.contains("\\u1605?\\u1585?")).unwrap();
    assert!(arabic.starts_with("{\\pard\\rtlpar\\qr\\fs22\\afs22\\b\\ab\\i\\ai "), "{arabic}");
}

#[test]
fn rtf_table_cells_run_right_to_left() {
    let cell = |t: &str, x: f64, y: f64| Block { rect: [x, y, x + 100.0, y + 12.0], ..block(t, y, 11.0, false) };
    let page = Page {
        width: 612.0,
        height: 792.0,
        blocks: vec![
            cell(ARABIC, 72.0, 620.0),
            cell("Unit", 232.0, 620.0),
            cell("مرحبا", 72.0, 590.0),
            cell("kWh", 232.0, 590.0),
            block("Notes follow the table and are long enough to be the body text size of the page here.", 500.0, 11.0, false),
        ],
        images: Vec::new(),
    };
    let r = rtf(&[page]);
    assert!(r.contains("{\\intbl\\rtlpar\\qr\\fs22\\afs22 \\u1605?"), "{r}");
    assert!(r.contains("{\\intbl\\fs22 Unit}\\cell"), "{r}");
}

#[test]
fn hostile_text_is_written_without_panicking() {
    let long = "ب".repeat(200_000);
    for text in [long.as_str(), "\u{202E}\u{202C}", "\u{200F}", "\u{0000}مرحبا\u{FFFF}", "", " "] {
        let p = Page { width: 612.0, height: 792.0, blocks: vec![block(text, 700.0, 11.0, false)], images: Vec::new() };
        let _ = docx(std::slice::from_ref(&p), "t");
        let _ = pptx(std::slice::from_ref(&p), "t");
        let _ = html(std::slice::from_ref(&p), "t");
        let _ = rtf(std::slice::from_ref(&p));
    }
    // Control characters the formats forbid are still dropped from right-to-left text.
    let p = Page { width: 612.0, height: 792.0, blocks: vec![block("\u{0001}مرحبا", 700.0, 11.0, false)], images: Vec::new() };
    let xml = part(&docx(&[p], "t"), "word/document.xml");
    assert!(xml.contains(">مرحبا</w:t>") && !xml.contains('\u{1}'), "{xml}");
}
