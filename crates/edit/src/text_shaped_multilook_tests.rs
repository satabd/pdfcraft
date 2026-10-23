//! Shaped Arabic in a paragraph of several looks (`text_looks`): retyped, each character keeps the look (size, colour,
//! font) of the stored character it came from, a stretch of right-to-left script the look of its first letter. Most tests
//! use the stand-in face of `text_shaped_tests`; the last uses the real one and skips without it.

use pdfcraft_content::Op;
use pdfcraft_cos::{Document, Object};

use crate::added::Align;
use crate::tests::{build, page_content_bytes, reopen, stream};
use crate::text::{self, BlockStyle};
use crate::text_shaped_tests::fake;

/// A page whose one font resource per look is a subset that has only `B`/`A` (م/س), `X` (lam-alef) and the space: `F1`
/// regular and `F2` bold, both drawn as `content` says.
fn looks_page(content: &str) -> Document {
    let cmap = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapType 2 def \
                1 begincodespacerange <00> <FF> endcodespacerange 4 beginbfchar \
                <20> <0020> <41> <0633> <42> <0645> <58> <FEFB> \
                endbfchar endcmap CMapName currentdict /CMap defineresource pop end end";
    let widths: Vec<_> = (32..=88).map(|c| if c == 32 { "250" } else { "500" }).collect();
    let font = |name: &str| {
        format!("<< /Type /Font /Subtype /Type1 /BaseFont /{name} /FirstChar 32 /LastChar 88 /Widths [{}] /ToUnicode 6 0 R >>", widths.join(" "))
    };
    build(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Contents 4 0 R /Resources << /Font << /F1 5 0 R /F2 7 0 R >> >> >>",
        &stream("", content),
        &font("FakeArabic"),
        &stream("", cmap),
        &font("FakeArabic-Bold"),
    ])
}

/// One row from x = 300 of two runs, `مسم` at 12 pt and `مس ` at 18 pt (each run is read right to left, and the row's
/// runs left to right: the paragraph reads `مسممس `).
fn two_sizes() -> Document {
    looks_page("BT /F1 12 Tf 300 700 Td (BAB) Tj /F1 18 Tf ( AB) Tj ET")
}

/// The same row with its second run in red and bold.
fn bold_red() -> Document {
    looks_page("BT /F1 12 Tf 300 700 Td (BAB) Tj 1 0 0 rg /F2 12 Tf ( AB) Tj ET")
}

/// A heading at 18 pt over a row at 12 pt, both from x = 300.
fn heading() -> Document {
    looks_page("BT /F1 18 Tf 300 720 Td (BAB) Tj /F1 12 Tf 0 -24 Td (ABA) Tj ET")
}

/// The text of the paragraph as the editor has it.
fn editor_text(doc: &Document) -> String {
    text::text_blocks(doc, 0).expect("blocks").remove(0).editor_text
}

/// `text` with its runs of white space as one space.
fn words(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn ops(doc: &Document) -> Vec<Op> {
    pdfcraft_content::parse(&page_content_bytes(doc, 0)).ops
}

/// The `/ActualText` of every span on the page.
fn actual_texts(doc: &Document) -> Vec<String> {
    ops(doc)
        .iter()
        .filter(|o| o.is("BDC"))
        .filter_map(|o| match o.operands.get(1) {
            Some(Object::Dict(d)) => match d.get(b"ActualText") {
                Some(Object::String(s)) => Some(s.to_text()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// What each string the paragraph shows is drawn in, in drawing order: the font resource, the size and the fill colour
/// operator in force (`""` for the initial black).
fn shown(doc: &Document) -> Vec<(String, f64, String)> {
    let (mut font, mut size, mut fill) = (String::new(), 0.0, String::new());
    let mut out = Vec::new();
    for op in ops(doc) {
        if op.is("Tf") {
            font = String::from_utf8_lossy(op.name(0).unwrap_or_default()).into_owned();
            size = op.num(1).unwrap_or(0.0);
        } else if op.is("rg") || op.is("g") {
            fill = format!("{} {:?}", String::from_utf8_lossy(&op.op), op.operands.iter().filter_map(Object::as_f64).collect::<Vec<_>>());
        } else if op.is("Tj") || op.is("TJ") {
            out.push((font.clone(), size, fill.clone()));
        }
    }
    out
}

/// The rows of the page as they read back: their text and box, the runs on one baseline being one row.
fn rows(doc: &Document) -> Vec<(String, [f64; 4])> {
    let mut rows: Vec<(f64, String, [f64; 4])> = Vec::new();
    for line in text::text_lines(&reopen(doc), 0).expect("lines") {
        match rows.last_mut() {
            Some((baseline, text, rect)) if (*baseline - line.origin_baseline()).abs() < 1.0 => {
                text.push_str(&line.text);
                *rect = [rect[0].min(line.rect[0]), rect[1].min(line.rect[1]), rect[2].max(line.rect[2]), rect[3].max(line.rect[3])];
            }
            _ => rows.push((line.origin_baseline(), line.text.clone(), line.rect)),
        }
    }
    rows.into_iter().map(|(_, text, rect)| (text, rect)).collect()
}

/// The sizes the generated Arabic glyphs are shown at, in drawing order, one per change.
fn arabic_sizes(doc: &Document) -> Vec<f64> {
    let mut sizes: Vec<f64> = Vec::new();
    for (font, size, _) in shown(doc) {
        if font.starts_with("PCEdAr") && sizes.last() != Some(&size) {
            sizes.push(size);
        }
    }
    sizes
}

#[test]
fn a_retyped_paragraph_of_two_sizes_keeps_each_words_size() {
    let _face = fake();
    let mut doc = two_sizes();
    // A word typed after the 18 pt run's space takes its look; the word that joins the two runs' letters is drawn in one look,
    // its first letter's (12 pt).
    let typed = format!("{}بالعالم", editor_text(&doc));
    assert_eq!(typed, "مسممس بالعالم");
    text::replace_block(&mut doc, 0, 0, &typed).expect("writes");
    // Drawn right to left: the last word typed is drawn first, at 18 pt (with its space), then the first at 12.
    assert_eq!(arabic_sizes(&doc), [18.0, 12.0], "{:?}", shown(&doc));
    // One span, the line in typing order.
    assert_eq!(actual_texts(&doc), [typed.as_str()]);
    // It reads back as typed: the generated fonts' ToUnicode maps give back every letter once.
    assert_eq!(text::text_blocks(&reopen(&doc), 0).expect("blocks")[0].text, typed);
    // The row keeps its right edge, as a right-to-left row does.
    let (_, rect) = rows(&doc).pop().expect("a row");
    let before = rows(&two_sizes()).pop().expect("a row").1;
    assert!((rect[2] - before[2]).abs() < 0.5, "{rect:?} against {before:?}");
}

#[test]
fn a_larger_first_row_keeps_its_size_and_the_rows_after_it_theirs() {
    let _face = fake();
    let mut doc = heading();
    let source = editor_text(&doc);
    let (first, second) = source.split_once('\n').expect("two rows");
    // (A word typed in the first row would make the whole draft one change, which takes the first row's look.)
    let typed = format!("{first}\n{second} بالعالم");
    // (A paragraph of several rows keeps its width unless it is given one.)
    let style = BlockStyle { width: Some(200.0), ..Default::default() };
    text::rewrite_block(&mut doc, 0, 0, Some(&typed), &style).expect("writes");
    assert_eq!(arabic_sizes(&doc), [18.0, 12.0], "{:?}", shown(&doc));
    assert_eq!(actual_texts(&doc), typed.split('\n').collect::<Vec<_>>());
    let rows = rows(&doc);
    assert_eq!(rows.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>(), typed.split('\n').collect::<Vec<_>>());
    // The heading is half again as tall as the row under it, as 18 pt letters are to 12 pt ones.
    let height = |rect: [f64; 4]| rect[3] - rect[1];
    assert!((height(rows[0].1) / height(rows[1].1) - 1.5).abs() < 0.01, "{rows:?}");
}

#[test]
fn a_bold_red_run_keeps_its_colour_and_its_latin_text_its_weight() {
    let _face = fake();
    let mut doc = bold_red();
    // The stored text with a Latin word typed at its end: it takes the look of the bold red space before it.
    let typed = format!("{}PDF", editor_text(&doc));
    text::replace_block(&mut doc, 0, 0, &typed).expect("writes");
    let drawn = shown(&doc);
    // The Latin word is in the run's own bold font (which has the letters), in red.
    let latin: Vec<_> = drawn.iter().filter(|(font, _, _)| !font.starts_with("PCEdAr")).collect();
    assert_eq!(latin, [&("F2".to_owned(), 12.0, "rg [1.0, 0.0, 0.0]".to_owned())], "{drawn:?}");
    // The first word is black, the kept bold word red: the colour goes back to black after a red run.
    let fills: Vec<&str> = drawn.iter().filter(|(font, _, _)| font.starts_with("PCEdAr")).map(|(_, _, fill)| fill.as_str()).collect();
    assert!(fills.iter().any(|f| f.starts_with("rg [1.0, 0.0, 0.0]")) && fills.iter().any(|f| f.starts_with("g [0.0]")), "{drawn:?}");
    assert_eq!(actual_texts(&doc), [typed.as_str()]);
    assert_eq!(text::text_blocks(&reopen(&doc), 0).expect("blocks")[0].text, typed);
}

#[test]
fn a_paragraph_of_one_look_is_written_as_before() {
    let _face = fake();
    // The same draft over a paragraph whose two runs share a look writes no more than one look's operators.
    let mut one = looks_page("BT /F1 12 Tf 300 700 Td (BAB) Tj ( AB) Tj ET");
    let typed = format!("{}بالعالم", editor_text(&one));
    text::replace_block(&mut one, 0, 0, &typed).expect("writes");
    assert_eq!(arabic_sizes(&one), [12.0]);
    assert!(!ops(&one).iter().any(|o| o.is("g") || o.is("rg")), "{:?}", shown(&one));
    // Black set out loud on the second row is the initial black of the first: still one look.
    let rows = |fill: &str| looks_page(&format!("BT /F1 12 Tf 300 720 Td (BAB) Tj {fill} 0 -16 Td (ABA) Tj ET"));
    let typed = format!("{} بالعالم", editor_text(&rows("")));
    let (mut plain, mut black) = (rows(""), rows("0 g"));
    assert_eq!(text::text_lines(&black, 0).expect("lines").len(), 2);
    text::replace_block(&mut plain, 0, 0, &typed).expect("writes");
    text::replace_block(&mut black, 0, 0, &typed).expect("writes");
    assert_eq!(shown(&black), shown(&plain));
}

#[test]
fn wrapping_measures_each_word_in_its_own_size() {
    let _face = fake();
    // Twelve words at 30 pt after a first word at 12 pt, in a box 100 pt wide: a 30 pt word is about 45 pt wide.
    let mut doc = looks_page("BT /F1 12 Tf 300 700 Td (BAB) Tj /F1 30 Tf ( AB) Tj ET");
    let typed = format!("{}{}", editor_text(&doc), ["سم"; 12].join(" "));
    let style = BlockStyle { width: Some(100.0), ..Default::default() };
    text::rewrite_block(&mut doc, 0, 0, Some(&typed), &style).expect("writes");
    let rows = rows(&doc);
    assert!(rows.len() >= 5, "{rows:?}");
    for (text, rect) in &rows {
        assert!(rect[2] - rect[0] <= 100.5, "a row no wider than the box: {text:?} {rect:?}");
    }
    assert_eq!(actual_texts(&doc).join(" "), typed);
}

#[test]
fn a_justified_paragraph_of_two_looks_is_stretched_to_its_width() {
    let _face = fake();
    let mut doc = two_sizes();
    // Words typed after the 18 pt run's space take its size; those before it are the first run's 12 pt.
    let typed = format!("مرحبا بالعالم مرحبا بالعالم {}{}", editor_text(&doc), ["سم"; 6].join(" "));
    let style = BlockStyle { width: Some(120.0), align: Some(Align::Justify), ..Default::default() };
    text::rewrite_block(&mut doc, 0, 0, Some(&typed), &style).expect("writes");
    let rows = rows(&doc);
    assert!(rows.len() >= 3, "{rows:?}");
    let (last, stretched) = rows.split_last().expect("a last row");
    for (text, rect) in stretched {
        assert!((rect[0] - 300.0).abs() < 0.5 && (rect[2] - 420.0).abs() < 0.5, "{text:?} fills the width: {rect:?}");
    }
    assert!((last.1[2] - 420.0).abs() < 0.5 && last.1[0] > 300.5, "the last row is right-aligned: {last:?}");
    // Some row has words of both sizes, stretched by adjustments in each run's own size.
    assert!(arabic_sizes(&doc).len() >= 2, "{:?}", shown(&doc));
    assert!(ops(&doc).iter().any(|o| o.is("TJ")));
    // The text is what it was: the lines in typing order.
    assert_eq!(actual_texts(&doc).join(" "), words(&typed));
    // The layout the editor is given reports those rows.
    let draft = two_sizes();
    let layout = text::text_edit_layout(&draft, 0, 0, &typed, &style).expect("layout");
    assert_eq!(layout.rows.len(), rows.len(), "{:?}", layout.rows);
    for ((text, rect), laid) in rows.iter().zip(&layout.row_rects) {
        assert!(rect.iter().zip(laid).all(|(a, b)| (a - b).abs() < 0.01), "{text:?}: {rect:?} against {laid:?}");
    }
}

#[test]
fn a_look_never_changes_inside_a_word_of_arabic() {
    let _face = fake();
    // The space at the end deleted and a letter typed: the word `مسممسب` has letters of both runs (`مسم` at 12 pt, `مسب`
    // at 18), and it is drawn in one look, its first letter's, as the editor shows it.
    let mut doc = two_sizes();
    let typed = format!("{}ب", editor_text(&doc).trim_end());
    assert_eq!(typed, "مسممسب");
    text::replace_block(&mut doc, 0, 0, &typed).expect("writes");
    assert_eq!(arabic_sizes(&doc), [12.0], "{:?}", shown(&doc));
    assert_eq!(actual_texts(&doc), [typed.as_str()]);
    // A word after a space is a stretch of its own, and takes its own look again.
    let mut doc = two_sizes();
    let typed = format!("{}ب سم", editor_text(&doc).trim_end());
    text::replace_block(&mut doc, 0, 0, &typed).expect("writes");
    assert_eq!(arabic_sizes(&doc), [18.0, 12.0], "{:?}", shown(&doc));
}

#[test]
fn a_look_change_at_every_character_stays_bounded_and_never_panics() {
    let _face = fake();
    // 4,000 runs: each letter at 12 pt, each space after it at 18 pt and red.
    let mut content = String::from("BT /F1 12 Tf 30 700 Td ");
    for i in 0..2_000 {
        content.push_str(if i % 2 == 0 { "0 g /F1 12 Tf (B) Tj " } else { "0 g /F1 12 Tf (A) Tj " });
        content.push_str("1 0 0 rg /F2 18 Tf ( ) Tj ");
    }
    content.push_str("ET");
    let source = looks_page(&content);
    let source_text = editor_text(&source);
    assert_eq!(source_text.chars().count(), 4_000);
    let mut written = 0;
    for typed in [format!("{source_text}سم PDF"), format!("x{}", source_text.replace(' ', "1")), "ب".repeat(4_100)] {
        for style in [BlockStyle::default(), BlockStyle { width: Some(200.0), align: Some(Align::Justify), ..Default::default() }] {
            let mut doc = source.clone();
            let started = std::time::Instant::now();
            match text::rewrite_block(&mut doc, 0, 0, Some(&typed), &style) {
                Ok(_) => {
                    written += 1;
                    let bytes = page_content_bytes(&doc, 0).len();
                    assert!(bytes < 4_000_000, "{bytes} bytes for {} characters", typed.chars().count());
                    let read: String = actual_texts(&doc).concat();
                    assert_eq!(read.split_whitespace().collect::<String>(), typed.split_whitespace().collect::<String>());
                }
                Err(e) => assert!(!e.to_string().is_empty()),
            }
            assert!(started.elapsed().as_secs() < 30, "{:?}", started.elapsed());
        }
    }
    // (They are written, not refused: a test of refusals would prove nothing about the looks.)
    assert_eq!(written, 6);
}

#[test]
fn the_real_face_keeps_two_sizes() {
    if pdfcraft_fonts::document_arabic_font().is_none() {
        eprintln!("built without a craft-fonts Arabic face: the real-face check needs CRAFT_FONTS_DIR");
        return;
    }
    let mut doc = two_sizes();
    let typed = format!("{}بالعالم", editor_text(&doc));
    text::replace_block(&mut doc, 0, 0, &typed).expect("writes");
    assert_eq!(arabic_sizes(&doc), [18.0, 12.0], "{:?}", shown(&doc));
    assert_eq!(actual_texts(&doc), [typed.as_str()]);
    assert_eq!(text::text_blocks(&reopen(&doc), 0).expect("blocks")[0].text, typed);
}
