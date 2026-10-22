//! What `text_edit_layout` says about a draft besides its rows (`row_rects`, `shaping_faces`): the boxes of the rows the
//! rewrite writes, and the page fonts it shapes Arabic in, in place.
//!
//! The face tests need a complete, static Arabic face: `PDFCRAFT_TEST_ARABIC_FONT=<font file>`, or a craft-fonts build whose
//! face is static; they return without one.

use crate::added::Align;
use crate::tests::{page_content_bytes, reopen, text_page};
use crate::text::{self, BlockStyle};

/// The bytes of a complete static Arabic face for tests, if the build has one.
fn static_face() -> Option<Vec<u8>> {
    let bytes = match std::env::var_os("PDFCRAFT_TEST_ARABIC_FONT") {
        Some(path) => std::fs::read(path).ok()?,
        None => pdfcraft_fonts::document_arabic_font()?.bytes.to_vec(),
    };
    if pdfcraft_fonts::embedding::validate(&bytes).is_err() {
        eprintln!("the Arabic face is variable: set PDFCRAFT_TEST_ARABIC_FONT to a static one to run the face checks");
        return None;
    }
    Some(bytes)
}

const SALAM: &str = "\u{633}\u{644}\u{627}\u{645} \u{639}\u{644}\u{64A}\u{643}\u{645} \u{645}\u{631}\u{62D}\u{628}\u{627}";

/// A page whose paragraph is `rows` of Arabic set in the complete face `bytes` the way a producer that embeds a whole font does:
/// the glyphs of each row (shaped, in drawing order) shown by code, the row against the right edge at x = 300, the text of
/// each row behind them as `/ActualText`.
fn page_in_the_face(bytes: &[u8], rows: &[&str]) -> pdfcraft_cos::Document {
    use pdfcraft_cos::{Dict, Object, Stream};
    let mut doc = text_page("BT /F1 12 Tf 100 700 Td (x) Tj ET");
    let resource = text::register_text_font(&mut doc, 0, bytes).expect("the face registers");
    // The code of a glyph is the first CID of the font that is mapped to it.
    let page = pdfcraft_model::pages(&doc)[0].clone();
    let fonts = doc.dict(doc.dict(page.dict.get(b"Resources").expect("resources")).expect("dict").get(b"Font").expect("fonts")).expect("fonts");
    let top = doc.dict(fonts.get(resource.as_bytes()).expect("the face")).expect("font");
    let descendant = match top.get(b"DescendantFonts").map(|d| doc.resolve(d)).as_deref() {
        Some(Object::Array(a)) => doc.dict(a.first().expect("a descendant")).expect("descendant"),
        other => panic!("descendant fonts: {other:?}"),
    };
    let map = match &*doc.resolve(descendant.get(b"CIDToGIDMap").expect("a glyph map")) {
        Object::Stream(s) => s.decoded().expect("decodes"),
        other => panic!("{other:?}"),
    };
    let gids: Vec<u16> = map.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
    let mut content = String::new();
    for (i, row) in rows.iter().enumerate() {
        let shaped = pdfcraft_fonts::source_shaping::shape(bytes, row).expect("the face shapes the row");
        let hex: String = shaped
            .glyphs
            .iter()
            .map(|g| {
                let gid = u16::try_from(g.id.to_u32()).expect("a glyph id");
                let cid = gids.iter().position(|v| *v == gid && gid != 0).expect("the font maps the glyph");
                format!("{cid:04X}")
            })
            .collect();
        let logical: String = row.encode_utf16().map(|u| format!("{u:04X}")).collect();
        let x = 300.0 - shaped.advance * 12.0;
        let y = 700.0 - 15.0 * i as f64;
        content += &format!("/Span <</ActualText <FEFF{logical}>>> BDC BT /{resource} 12 Tf 1 0 0 1 {x:.3} {y} Tm <{hex}> Tj ET EMC\n");
    }
    let stream = doc.add(Object::Stream(Stream::flate(Dict::new(), content.as_bytes())));
    doc.update_dict(page.obj, |d| d.set(b"Contents".to_vec(), Object::Ref(stream))).expect("page updates");
    reopen(&doc)
}

fn layout(doc: &pdfcraft_cos::Document, text: &str, style: &BlockStyle) -> text::TextEditLayout {
    text::text_edit_layout(doc, 0, 0, text, style).expect("the draft lays out")
}

/// The faces of a draft the rewrite may refuse (no face to draw it in): none then, as Apply writes nothing.
fn faces(doc: &pdfcraft_cos::Document, text: &str, style: &BlockStyle) -> Vec<(usize, String)> {
    text::text_edit_layout(doc, 0, 0, text, style).map(|l| l.shaping_faces).unwrap_or_default()
}

/// The sfnt bytes of the page font `resource`.
fn program(doc: &pdfcraft_cos::Document, resource: &str) -> Option<Vec<u8>> {
    let p = &pdfcraft_model::pages(doc)[0];
    let resources = doc.dict(p.dict.get(b"Resources")?)?;
    let fonts = doc.dict(resources.get(b"Font")?)?;
    let font = doc.dict(fonts.get(resource.as_bytes())?)?;
    pdfcraft_fonts::pdf::embedded_sfnt(doc, &font)
}

#[test]
fn a_paragraph_in_a_complete_face_is_shaped_in_that_face_whatever_is_typed() {
    let Some(face) = static_face() else { return };
    let doc = page_in_the_face(&face, &[SALAM]);
    let block = &text::text_blocks(&doc, 0).expect("blocks")[0];
    assert_eq!(block.editor_text, SALAM, "the page reads the paragraph back");
    // As it is, a retyped draft and a draft of other Arabic letters: the page's face, which holds the program.
    for draft in [SALAM, "\u{643}\u{62A}\u{627}\u{628} \u{62C}\u{62F}\u{64A}\u{62F}", &format!("{SALAM} {SALAM} {SALAM}")] {
        let l = layout(&doc, draft, &BlockStyle::default());
        assert!(!l.shaping_faces.is_empty(), "{draft:?} is shaped in place");
        for (index, resource) in &l.shaping_faces {
            assert!(block.lines.contains(index), "a source run of the paragraph: {index}");
            assert_eq!(program(&doc, resource).as_deref(), Some(face.as_slice()), "{resource} holds the face");
        }
        // The face is the one the rewrite really writes: its lines are drawn in that program after Apply.
        if draft != SALAM {
            let mut written = doc.clone();
            text::rewrite_block(&mut written, 0, 0, Some(draft), &BlockStyle::default()).expect("applies");
            let after = text::text_lines(&written, 0).expect("lines");
            assert!(after.iter().all(|line| program(&written, &line.font).as_deref() == Some(face.as_slice())), "drawn in the face");
        }
    }
}

#[test]
fn the_rewrite_leaves_the_page_face_when_it_cannot_shape_in_it() {
    let Some(face) = static_face() else { return };
    let doc = page_in_the_face(&face, &[SALAM]);
    // A letter the face lacks (the rewrite refuses the draft), a change of family or weight, a document font chosen: another
    // route, or none.
    let mut lacks = SALAM.to_owned();
    lacks.push('\u{FDFD}');
    if !pdfcraft_fonts::embedding::covers(&face, &lacks) {
        assert!(faces(&doc, &lacks, &BlockStyle::default()).is_empty(), "a letter the face lacks");
    }
    let bold = BlockStyle { bold: Some(true), ..Default::default() };
    assert!(faces(&doc, SALAM, &bold).is_empty(), "a weight change");
    let family = BlockStyle { family: Some((crate::Family::Times, false, false)), ..Default::default() };
    assert!(faces(&doc, SALAM, &family).is_empty(), "a family change");
    let chosen = BlockStyle { resource: Some("PCFull0".into()), ..Default::default() };
    assert!(faces(&doc, SALAM, &chosen).is_empty(), "a document font chosen for the paragraph");
    // Latin text, Hebrew and an empty draft have no Arabic to shape.
    assert!(faces(&doc, "Hello world", &BlockStyle::default()).is_empty(), "no Arabic in the draft");
    assert!(faces(&doc, "\u{5E9}\u{5DC}\u{5D5}\u{5DD}", &BlockStyle::default()).is_empty(), "Hebrew");
    assert!(faces(&doc, "", &BlockStyle::default()).is_empty(), "an empty draft");
}

#[test]
fn arabic_on_a_page_without_the_face_is_not_shaped_in_place() {
    // Helvetica and a subset font: nothing complete to shape in, whatever the build has.
    for content in ["BT /F1 12 Tf 100 700 Td (Hello wide world) Tj ET", "BT /F2 12 Tf 100 700 Td (abab) Tj ET"] {
        let doc = text_page(content);
        let l = text::text_edit_layout(&doc, 0, 0, SALAM, &BlockStyle::default());
        let faces = l.map(|l| l.shaping_faces).unwrap_or_default();
        assert!(faces.is_empty(), "{content}: {faces:?}");
    }
}

#[test]
fn the_rows_of_an_unchanged_paragraph_have_the_boxes_of_its_lines() {
    let src = "BT /F1 10 Tf 12 TL 100 700 Td (One two three four five six seven) Tj T* (eight nine ten eleven twelve) Tj T* (end) Tj ET";
    let doc = text_page(src);
    let block = text::text_blocks(&doc, 0).expect("blocks").remove(0);
    let lines = text::text_lines(&doc, 0).expect("lines");
    let l = text::text_edit_layout(&doc, 0, 0, &block.editor_text, &BlockStyle::default()).expect("layout");
    assert_eq!(l.rows.len(), l.row_rects.len(), "a box to every row");
    let expected: Vec<[f64; 4]> = block.rows.iter().map(|row| lines[row[0]].rect).collect();
    assert_eq!(l.row_rects, expected);
}

#[test]
fn the_boxes_of_justified_rows_span_the_paragraph_and_the_last_row_does_not() {
    let src = "BT /F1 10 Tf 12 TL 100 700 Td (One two three four five six seven) Tj T* (eight nine ten eleven twelve) Tj T* (end) Tj ET";
    let doc = text_page(src);
    let block = text::text_blocks(&doc, 0).expect("blocks").remove(0);
    let justified = BlockStyle { align: Some(Align::Justify), ..Default::default() };
    // (The text without the rows' line breaks: a draft with explicit breaks is written by the styled rewrite, which does not justify.)
    let draft = format!("{} and some more words to fill a third row of this text", block.text);
    let l = text::text_edit_layout(&doc, 0, 0, &draft, &justified).expect("layout");
    assert!(l.rows.len() >= 3, "{:?}", l.rows);
    for rect in &l.row_rects[..l.row_rects.len() - 1] {
        assert!((rect[2] - block.rect[2]).abs() < 1.0, "a justified row ends at the paragraph's right edge: {rect:?} against {:?}", block.rect);
    }
    let last = l.row_rects.last().expect("a last row");
    assert!(last[2] < block.rect[2] - 10.0, "the last row stays short: {last:?}");
    // The layout is the rewrite's: the lines of the document after Apply are these rows.
    let mut applied = doc.clone();
    text::rewrite_block(&mut applied, 0, 0, Some(&draft), &justified).expect("applies");
    let after = text::text_lines(&applied, 0).expect("lines");
    assert_eq!(after.len(), l.row_rects.len());
    for (line, rect) in after.iter().zip(&l.row_rects) {
        assert!(line.rect.iter().zip(rect).all(|(a, b)| (a - b).abs() < 1e-6), "{:?} against {rect:?}", line.rect);
    }
    assert!(!page_content_bytes(&applied, 0).is_empty());
}

const RETYPED: &str = "هذا نص عربي طويل يكتب في حقل ضيق ليلتف على عدة أسطر ويبقى مطابقا لما تكتبه الصفحة";

#[test]
fn a_retyped_arabic_paragraph_in_its_face_keeps_its_right_edge_like_the_single_style_rewrite() {
    let Some(face) = static_face() else { return };
    let doc = page_in_the_face(&face, &[SALAM, "\u{643}\u{62A}\u{627}\u{628}"]);
    let block = text::text_blocks(&doc, 0).expect("blocks").remove(0);
    let right = block.rect[2];
    // Every row of the rewritten paragraph ends at the paragraph's right edge, whichever rows the page had.
    let l = layout(&doc, RETYPED, &BlockStyle::default());
    assert!(l.rows.len() >= 3, "{:?}", l.rows);
    for rect in &l.row_rects {
        assert!((rect[2] - right).abs() < 0.01, "{rect:?} against the right edge {right}");
    }
    // A single row grows to the left, keeping its right edge.
    let single = page_in_the_face(&face, &[SALAM]);
    let single_block = text::text_blocks(&single, 0).expect("blocks").remove(0);
    let right = single_block.rect[2];
    let l = text::text_edit_layout(&single, 0, 0, "هذا نص عربي طويل يكتب في حقل", &BlockStyle::default()).expect("layout");
    assert_eq!(l.rows.len(), 1, "{:?}", l.rows);
    assert!((l.row_rects[0][2] - right).abs() < 0.01 && l.row_rects[0][0] < single_block.rect[0] - 1.0, "{:?}", l.row_rects);
    // An alignment asked for wins.
    let left = BlockStyle { align: Some(Align::Left), ..Default::default() };
    let l = text::text_edit_layout(&doc, 0, 0, RETYPED, &left).expect("layout");
    assert!(l.row_rects.iter().all(|r| (r[0] - block.rect[0]).abs() < 0.01), "{:?}", l.row_rects);
}
