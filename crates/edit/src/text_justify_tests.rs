//! Justified drafts in the styled rewrite (a paragraph of several looks, or a draft with a line break): every row the wrap
//! breaks is stretched to the paragraph's width by widening the spaces between its words, the last row of a paragraph, a
//! row before a line break and a row of one word are not, and a text that is not justified is written as it always was.

use pdfcraft_cos::{Dict, Document, Object, Stream};

use crate::added::Align;
use crate::tests::{hebrew_page, page_content_bytes, reopen, text_page};
use crate::text::{self, BlockStyle};

fn justified() -> BlockStyle {
    BlockStyle { align: Some(Align::Justify), ..Default::default() }
}

/// Three rows in two looks (the end of the first row is blue): the styled rewrite writes the paragraph.
fn two_looks() -> Document {
    text_page(
        "BT /F1 10 Tf 12 TL 100 700 Td (One two three four five six) Tj 0 0 1 rg ( seven) Tj 0 g T* (eight nine ten eleven twelve) Tj T* (end) Tj ET",
    )
}

const MORE: &str = "and some more words to fill a third row of this text and then a fourth row to end with";

/// The text of the paragraph without its rows' line breaks, and more words.
fn long_draft(doc: &Document) -> (text::TextBlock, String) {
    let block = text::text_blocks(doc, 0).expect("blocks").remove(0);
    let draft = format!("{} {MORE}", block.editor_text.replace('\n', " "));
    (block, draft)
}

/// The boxes of the rows of `lines`, the runs on one baseline being one row.
fn rows_of(lines: &[text::TextLine]) -> Vec<[f64; 4]> {
    let mut rows: Vec<(f64, [f64; 4])> = Vec::new();
    for line in lines {
        match rows.last_mut() {
            Some((baseline, rect)) if (*baseline - line.origin_baseline()).abs() < 1.0 => {
                *rect = [rect[0].min(line.rect[0]), rect[1].min(line.rect[1]), rect[2].max(line.rect[2]), rect[3].max(line.rect[3])];
            }
            _ => rows.push((line.origin_baseline(), line.rect)),
        }
    }
    rows.into_iter().map(|(_, rect)| rect).collect()
}

fn layout(doc: &Document, draft: &str, style: &BlockStyle) -> text::TextEditLayout {
    text::text_edit_layout(doc, 0, 0, draft, style).expect("layout")
}

#[test]
fn a_justified_paragraph_of_two_looks_fills_the_width_with_every_row_but_the_last() {
    let doc = two_looks();
    let (block, draft) = long_draft(&doc);
    let l = layout(&doc, &draft, &justified());
    assert!(l.rows.len() >= 4, "{:?}", l.rows);
    let (last, rows) = l.row_rects.split_last().expect("a last row");
    for rect in rows {
        assert!((rect[2] - block.rect[2]).abs() < 0.5, "a justified row ends at the paragraph's right edge: {rect:?} against {:?}", block.rect);
        assert!((rect[0] - block.rect[0]).abs() < 0.5, "and begins at its left edge: {rect:?}");
    }
    assert!(last[2] < block.rect[2] - 10.0 && (last[0] - block.rect[0]).abs() < 0.5, "the last row stays short: {last:?}");
    // It is the rewrite's own: Apply writes these rows, in both looks, with the text as typed.
    let mut applied = doc.clone();
    text::rewrite_block(&mut applied, 0, 0, Some(&draft), &justified()).expect("applies");
    let after = text::text_lines(&reopen(&applied), 0).expect("lines");
    assert!(after.iter().any(|line| line.color == [0.0, 0.0, 1.0]) && after.iter().any(|line| line.color == [0.0, 0.0, 0.0]), "{after:?}");
    let written = rows_of(&after);
    assert_eq!(written.len(), l.row_rects.len());
    for (written, rect) in written.iter().zip(&l.row_rects) {
        assert!(written.iter().zip(rect).all(|(a, b)| (a - b).abs() < 1e-6), "{written:?} against {rect:?}");
    }
    let flat = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    assert_eq!(flat(&text::text_blocks(&applied, 0).expect("blocks")[0].editor_text), flat(&draft));
    // Word spacing reaches the spaces of a simple font: no glyph is moved by hand.
    let ops = pdfcraft_content::parse(&page_content_bytes(&applied, 0)).ops;
    assert!(ops.iter().any(|o| o.is("Tw") && o.num(0).is_some_and(|w| w > 0.0)), "{ops:?}");
}

#[test]
fn a_paragraph_that_is_not_justified_is_ragged_and_written_with_no_word_spacing() {
    let doc = two_looks();
    let (block, draft) = long_draft(&doc);
    for style in [BlockStyle::default(), BlockStyle { align: Some(Align::Left), ..Default::default() }] {
        let l = layout(&doc, &draft, &style);
        let rows = &l.row_rects[..l.row_rects.len() - 1];
        assert!(rows.iter().any(|r| r[2] < block.rect[2] - 0.6), "ragged: {rows:?} against {:?}", block.rect);
        let mut applied = doc.clone();
        text::rewrite_block(&mut applied, 0, 0, Some(&draft), &style).expect("applies");
        let ops = pdfcraft_content::parse(&page_content_bytes(&applied, 0)).ops;
        assert!(ops.iter().filter(|o| o.is("Tw")).all(|o| o.num(0) == Some(0.0)), "{ops:?}");
        assert!(ops.iter().all(|o| !o.is("TJ")), "{ops:?}");
    }
}

#[test]
fn a_justified_row_of_one_word_is_not_stretched() {
    let doc = two_looks();
    let block = text::text_blocks(&doc, 0).expect("blocks").remove(0);
    // A long word that fills most of a row, and one after it that does not fit beside it.
    let draft = "Wonderfullylongword Extraordinarilyunusualwordtwo and some more words to fill a third row of this text and more";
    let l = layout(&doc, draft, &justified());
    assert_eq!(l.rows.first().map(String::as_str), Some("Wonderfullylongword"), "{:?}", l.rows);
    let first = l.row_rects[0];
    assert!(
        first[2] < block.rect[2] - 3.0 && (first[0] - block.rect[0]).abs() < 0.5,
        "a single word keeps its width: {first:?} against {:?}",
        block.rect
    );
    // The rows after it are stretched.
    assert!(l.rows.len() >= 4 && (l.row_rects[2][2] - block.rect[2]).abs() < 0.5, "{:?} {:?}", l.rows, l.row_rects);
}

#[test]
fn a_justified_row_before_a_line_break_is_not_stretched_and_the_rows_after_it_are() {
    let doc = two_looks();
    let block = text::text_blocks(&doc, 0).expect("blocks").remove(0);
    let draft = format!("A short row\n{MORE} and {MORE}");
    let l = layout(&doc, &draft, &justified());
    assert_eq!(l.rows.first().map(String::as_str), Some("A short row"), "{:?}", l.rows);
    assert!(l.row_rects[0][2] < block.rect[2] - 40.0, "{:?}", l.row_rects[0]);
    let (_, rest) = l.row_rects[1..].split_last().expect("rows after it");
    assert!(rest.len() >= 2 && rest.iter().all(|r| (r[2] - block.rect[2]).abs() < 0.5), "{rest:?}");
    // A row that ends at a line break in the middle of the text is a paragraph's last row: it keeps its width too.
    let l = layout(&doc, &format!("{MORE}\n{MORE}"), &justified());
    let at = l.rows.iter().position(|row| row.ends_with("to end with")).expect("the row before the break");
    assert!(l.row_rects[at][2] < block.rect[2] - 5.0, "{:?}", l.row_rects[at]);
    assert!(l.row_rects[..at].iter().all(|r| (r[2] - block.rect[2]).abs() < 0.5), "{:?}", l.row_rects);
}

#[test]
fn a_justified_paragraph_with_leading_or_trailing_spaces_in_a_row_leaves_that_row_alone() {
    let doc = two_looks();
    let block = text::text_blocks(&doc, 0).expect("blocks").remove(0);
    // A row that begins with a space (the first row, after a line break) has spaces that are not between words.
    let draft = format!("{MORE}\n  {MORE} {MORE}");
    let l = layout(&doc, &draft, &justified());
    assert!(l.rows.iter().any(|row| row.starts_with(' ')), "{:?}", l.rows);
    let indented = l.rows.iter().position(|row| row.starts_with(' ')).expect("an indented row");
    assert!(l.row_rects[indented][2] < block.rect[2] - 0.6, "{:?}", l.row_rects[indented]);
}

/// The page of `justified_text_in_a_composite_font_fills_each_row_with_adjustments_after_its_spaces`, its rows in two
/// colours: two-byte codes, so word spacing (single-byte code 32 only) can't stretch the spaces.
fn composite_two_looks() -> Document {
    let hex = |s: &str| s.bytes().map(|b| format!("00{b:02x}")).collect::<String>();
    let src = format!(
        "BT /F1 10 Tf 12 TL 100 700 Td <{}> Tj 0 0 1 rg <{}> Tj 0 g T* <{}> Tj T* <{}> Tj ET",
        hex("One two three four five six"),
        hex(" seven"),
        hex("eight nine ten eleven twelve"),
        hex("and then the end of it")
    );
    let mut doc = text_page(&src);
    let unicode = doc.add(Object::Stream(Stream::flate(
        Dict::new(),
        b"1 begincodespacerange <0000> <ffff> endcodespacerange 1 beginbfrange <0020> <007e> <0020> endbfrange",
    )));
    let font = Dict::from_iter([
        (b"Type".to_vec(), Object::name("Font")),
        (b"Subtype".to_vec(), Object::name("Type0")),
        (b"BaseFont".to_vec(), Object::name("SyntheticCID")),
        (b"Encoding".to_vec(), Object::name("Identity-H")),
        (b"ToUnicode".to_vec(), Object::Ref(unicode)),
        (
            b"DescendantFonts".to_vec(),
            Object::Array(vec![Object::Dict(Dict::from_iter([
                (b"Subtype".to_vec(), Object::name("CIDFontType2")),
                (b"DW".to_vec(), Object::Int(500)),
            ]))]),
        ),
    ]);
    let p = pdfcraft_model::pages(&doc)[0].obj;
    let resources = Dict::from_iter([(b"Font".to_vec(), Object::Dict(Dict::from_iter([(b"F1".to_vec(), Object::Dict(font))])))]);
    doc.update_dict(p, |d| d.set(b"Resources".to_vec(), Object::Dict(resources))).unwrap();
    doc
}

#[test]
fn a_justified_paragraph_in_a_composite_font_of_two_looks_stretches_with_adjustments_after_its_spaces() {
    let doc = composite_two_looks();
    let (block, draft) = long_draft(&doc);
    let mut applied = doc.clone();
    text::rewrite_block(&mut applied, 0, 0, Some(&draft), &justified()).expect("applies");
    let lines = text::text_lines(&reopen(&applied), 0).expect("lines");
    let rows = rows_of(&lines);
    assert!(rows.len() >= 4, "{lines:?}");
    for rect in &rows[..rows.len() - 1] {
        assert!((rect[2] - block.rect[2]).abs() < 0.5 && (rect[0] - block.rect[0]).abs() < 0.5, "{rect:?} against {:?}", block.rect);
    }
    let ops = pdfcraft_content::parse(&page_content_bytes(&applied, 0)).ops;
    assert!(ops.iter().any(|o| o.is("TJ")), "{ops:?}");
    assert!(ops.iter().filter(|o| o.is("Tw")).all(|o| o.num(0) == Some(0.0)), "word spacing does not reach two-byte codes: {ops:?}");
}

#[test]
fn a_justified_hebrew_paragraph_that_grows_to_the_left_keeps_its_right_edge() {
    // One short right-to-left line, which grows to the left as far as the page's margin (36 pt) when it wraps.
    let mut doc = hebrew_page("BT /F1 12 Tf 300 700 Td (EDCBA) Tj ET");
    let before = text::text_lines(&doc, 0).expect("lines")[0].rect;
    let draft = ["שלום בדיוק"; 8].join(" ");
    text::rewrite_block(&mut doc, 0, 0, Some(&draft), &justified()).expect("applies");
    let lines = text::text_lines(&reopen(&doc), 0).expect("lines");
    assert!(lines.len() >= 2, "{lines:?}");
    let (last, rows) = lines.split_last().expect("a last row");
    for l in rows {
        assert!((l.rect[2] - before[2]).abs() < 0.5 && (l.rect[0] - 36.0).abs() < 0.5, "a stretched row spans the room the line has: {:?}", l.rect);
    }
    assert!((last.rect[2] - before[2]).abs() < 0.5 && last.rect[0] > 36.5, "the last row ends at the right edge: {:?}", last.rect);
}

#[test]
fn a_justified_hebrew_line_before_a_line_break_is_not_stretched() {
    // (Hebrew is written by the single-style rewrite even with a line break in the draft.)
    let mut doc = hebrew_page("BT /F1 12 Tf 300 700 Td (EDCBA) Tj ET");
    let before = text::text_lines(&doc, 0).expect("lines")[0].rect;
    let draft = format!("שלום בדיוק\n{}", ["שלום בדיוק"; 8].join(" "));
    text::rewrite_block(&mut doc, 0, 0, Some(&draft), &justified()).expect("applies");
    let lines = text::text_lines(&reopen(&doc), 0).expect("lines");
    assert!(lines.len() >= 3, "{lines:?}");
    // It keeps its natural width (two words, ten letters and a space) at the right edge.
    assert!((lines[0].rect[2] - before[2]).abs() < 0.5 && lines[0].rect[2] - lines[0].rect[0] < 80.0, "{:?}", lines[0].rect);
    assert!(
        lines[1].rect[0] < 40.0 && (lines[1].rect[2] - before[2]).abs() < 0.5,
        "the next row, which the wrap broke, spans the room: {:?}",
        lines[1].rect
    );
}
