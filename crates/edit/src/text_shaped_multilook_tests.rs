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
    fonts_page(content, &["FakeArabic", "FakeArabic-Bold"])
}

/// [`looks_page`] with a font resource for each of `base_fonts`, `F1` on (the first two are objects 5 and 7, the ToUnicode
/// map between them, and any others follow from 8).
fn fonts_page(content: &str, base_fonts: &[&str]) -> Document {
    let cmap = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapType 2 def \
                1 begincodespacerange <00> <FF> endcodespacerange 4 beginbfchar \
                <20> <0020> <41> <0633> <42> <0645> <58> <FEFB> \
                endbfchar endcmap CMapName currentdict /CMap defineresource pop end end";
    let widths: Vec<_> = (32..=88).map(|c| if c == 32 { "250" } else { "500" }).collect();
    let font = |name: &str| {
        format!("<< /Type /Font /Subtype /Type1 /BaseFont /{name} /FirstChar 32 /LastChar 88 /Widths [{}] /ToUnicode 6 0 R >>", widths.join(" "))
    };
    let numbers = |i: usize| if i == 0 { 5 } else { i + 6 };
    let resources: Vec<String> = (0..base_fonts.len()).map(|i| format!("/F{} {} 0 R", i + 1, numbers(i))).collect();
    let page =
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Contents 4 0 R /Resources << /Font << {} >> >> >>", resources.join(" "));
    let mut objects =
        vec!["<< /Type /Catalog /Pages 2 0 R >>".to_owned(), "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(), page, stream("", content)];
    for (i, base) in base_fonts.iter().enumerate() {
        objects.push(font(base));
        if i == 0 {
            objects.push(stream("", cmap));
        }
    }
    build(&objects.iter().map(String::as_str).collect::<Vec<_>>())
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

/// The `/ActualText` of each line the page draws (the spans after one move of the text position), its spans read in the
/// order page text extraction reads them ([`pdfcraft_fonts::shaped_text::reading_order`]): the line as copy and search
/// take it.
fn read_lines(doc: &Document) -> Vec<String> {
    let mut lines: Vec<Vec<String>> = Vec::new();
    let mut moved = true;
    for op in ops(doc) {
        if ["BT", "Td", "TD", "Tm", "T*"].iter().any(|name| op.is(name)) {
            moved = true;
        } else if op.is("BDC")
            && let Some(Object::Dict(d)) = op.operands.get(1)
            && let Some(Object::String(text)) = d.get(b"ActualText")
        {
            if std::mem::take(&mut moved) {
                lines.push(Vec::new());
            }
            if let Some(line) = lines.last_mut() {
                line.push(text.to_text());
            }
        }
    }
    lines
        .iter()
        .map(|spans| {
            let texts: Vec<&str> = spans.iter().map(String::as_str).collect();
            pdfcraft_fonts::shaped_text::reading_order(&texts).into_iter().map(|k| texts[k]).collect()
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
    // A span for each look, in drawing order, each in typing order: read right to left, the line as typed.
    assert_eq!(actual_texts(&doc), [" بالعالم", "مسممس"]);
    assert_eq!(read_lines(&doc), [typed.as_str()]);
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
    assert_eq!(read_lines(&doc).join(" "), typed);
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
    assert_eq!(read_lines(&doc).join(" "), words(&typed));
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
                    let read: String = read_lines(&doc).concat();
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
    assert_eq!(actual_texts(&doc), [" بالعالم", "مسممس"]);
    assert_eq!(read_lines(&doc), [typed.as_str()]);
    assert_eq!(text::text_blocks(&reopen(&doc), 0).expect("blocks")[0].text, typed);
}

// ---- bold ----

/// The generated Arabic fonts the page shows, in drawing order, one per change: a bold one's name ends in `B`.
fn arabic_fonts(doc: &Document) -> Vec<String> {
    let mut fonts: Vec<String> = Vec::new();
    for (font, _, _) in shown(doc) {
        if font.starts_with("PCEdAr") && fonts.last() != Some(&font) {
            fonts.push(font);
        }
    }
    fonts
}

/// The font resource `name` of page 0's resources, and its descriptor.
fn font_and_descriptor(doc: &Document, name: &str) -> (pdfcraft_cos::Dict, pdfcraft_cos::Dict) {
    let page = pdfcraft_model::pages(doc)[0].dict.clone();
    let resources = doc.dict(page.get(b"Resources").expect("resources")).expect("a dict");
    let fonts = doc.dict(resources.get(b"Font").expect("fonts")).expect("a dict");
    let font = doc.dict(fonts.get(name.as_bytes()).expect("the font")).expect("a dict");
    let descriptor = doc.dict(font.get(b"FontDescriptor").expect("a descriptor")).expect("a dict");
    (font, descriptor)
}

#[test]
fn a_bold_run_retyped_in_arabic_is_drawn_from_bold_fonts_and_the_regular_run_from_regular_ones() {
    let _face = fake();
    let mut doc = bold_red();
    // A word typed after the bold red run's space takes its look: bold.
    let typed = format!("{}بالعالم", editor_text(&doc));
    text::replace_block(&mut doc, 0, 0, &typed).expect("writes");
    let drawn = shown(&doc);
    let red_arabic: Vec<_> = drawn.iter().filter(|(font, _, fill)| font.starts_with("PCEdAr") && fill.starts_with("rg")).collect();
    let black_arabic: Vec<_> = drawn.iter().filter(|(font, _, fill)| font.starts_with("PCEdAr") && fill.starts_with("g")).collect();
    assert!(!red_arabic.is_empty() && !black_arabic.is_empty(), "{drawn:?}");
    // The bold (red) run's Arabic in a font of the bold glyphs, the regular (black) run's in a regular one.
    assert!(red_arabic.iter().all(|(font, _, _)| font.ends_with('B')), "{drawn:?}");
    assert!(black_arabic.iter().all(|(font, _, _)| !font.ends_with('B')), "{drawn:?}");
    let (bold_font, bold_descriptor) = font_and_descriptor(&doc, &red_arabic[0].0);
    let (regular_font, regular_descriptor) = font_and_descriptor(&doc, &black_arabic[0].0);
    assert_eq!(bold_descriptor.get(b"FontWeight").and_then(Object::as_f64), Some(700.0));
    assert_eq!(regular_descriptor.get(b"FontWeight"), None);
    // Its glyphs are the bold ones: wider (the stand-in's bold is a fifth wider).
    let widths = |font: &pdfcraft_cos::Dict| match font.get(b"Widths") {
        Some(Object::Array(w)) => w.iter().filter_map(Object::as_f64).collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    // (The stand-in's regular advances: a space, a sign, a letter in its four forms, lam-alef.)
    let regular_advances = [250.0, 400.0, 500.0, 520.0, 540.0, 560.0, 600.0];
    let (bold_widths, regular_widths) = (widths(&bold_font), widths(&regular_font));
    assert!(!bold_widths.is_empty() && bold_widths.iter().all(|w| regular_advances.contains(&(w / 1.2).round())), "{bold_widths:?}");
    assert!(regular_widths.iter().all(|w| regular_advances.contains(w)), "{regular_widths:?}");
    // The text is what it was: a span for each look, read in typing order, and the ToUnicode maps give back every letter once.
    assert_eq!(actual_texts(&doc), [" بالعالم", "مسممس"]);
    assert_eq!(read_lines(&doc), [typed.as_str()]);
    assert_eq!(text::text_blocks(&reopen(&doc), 0).expect("blocks")[0].text, typed);
}

#[test]
fn a_whole_bold_paragraph_is_bold_on_the_one_look_path_and_a_regular_one_regular() {
    let _face = fake();
    for (font, bold) in [("F2", true), ("F1", false)] {
        let mut doc = looks_page(&format!("BT /{font} 12 Tf 300 700 Td (BAB) Tj ( AB) Tj ET"));
        let typed = format!("{}بالعالم", editor_text(&doc));
        text::replace_block(&mut doc, 0, 0, &typed).expect("writes");
        let fonts = arabic_fonts(&doc);
        assert_eq!(fonts.len(), 1, "{font}: {:?}", shown(&doc));
        assert_eq!(fonts[0].ends_with('B'), bold, "{font}: {fonts:?}");
        assert_eq!(actual_texts(&doc), [typed.as_str()]);
        let reopened = reopen(&doc);
        assert_eq!(text::text_blocks(&reopened, 0).expect("blocks")[0].text, typed);
        // Read back, the generated font says whether it is bold (its name and weight), so a second edit keeps the weight.
        let lines = text::text_lines(&reopened, 0).expect("lines");
        assert!(!lines.is_empty() && lines.iter().all(|l| l.bold == bold), "{:?}", lines.iter().map(|l| (&l.font, l.bold)).collect::<Vec<_>>());
    }
    // A weight asked for wins over the run's: bold asked for over a regular paragraph, regular over a bold one.
    for (font, asked) in [("F1", true), ("F2", false)] {
        let mut doc = looks_page(&format!("BT /{font} 12 Tf 300 700 Td (BAB) Tj ( AB) Tj ET"));
        let typed = format!("{}بالعالم", editor_text(&doc));
        text::rewrite_block(&mut doc, 0, 0, Some(&typed), &BlockStyle { bold: Some(asked), ..Default::default() }).expect("writes");
        let fonts = arabic_fonts(&doc);
        assert!(!fonts.is_empty() && fonts.iter().all(|f| f.ends_with('B') == asked), "{font} {asked}: {fonts:?}");
    }
}

#[test]
fn a_weight_change_at_every_word_stays_bounded() {
    let _face = fake();
    // 4,000 runs: Arabic words of one letter, regular and bold in turn, the spaces between them in the weight before.
    let mut content = String::from("BT /F1 12 Tf 30 700 Td ");
    for i in 0..2_000 {
        content.push_str(if i % 2 == 0 { "/F1 12 Tf (B ) Tj " } else { "/F2 12 Tf (A ) Tj " });
    }
    content.push_str("ET");
    let source = looks_page(&content);
    let source_text = editor_text(&source);
    assert!(source_text.chars().count() >= 3_999, "{}", source_text.chars().count());
    let mut written = 0;
    for typed in [format!("{source_text}سم PDF"), format!("{source_text}{}", "ب ".repeat(2_000))] {
        for style in [BlockStyle::default(), BlockStyle { width: Some(200.0), align: Some(Align::Justify), ..Default::default() }] {
            let mut doc = source.clone();
            let started = std::time::Instant::now();
            match text::rewrite_block(&mut doc, 0, 0, Some(&typed), &style) {
                Ok(_) => {
                    written += 1;
                    let bytes = page_content_bytes(&doc, 0).len();
                    assert!(bytes < 4_000_000, "{bytes} bytes for {} characters", typed.chars().count());
                    // Both weights are drawn, each from fonts of its own.
                    let fonts = arabic_fonts(&doc);
                    assert!(fonts.iter().any(|f| f.ends_with('B')) && fonts.iter().any(|f| !f.ends_with('B')), "{:?}", &fonts[..fonts.len().min(8)]);
                    let read: String = read_lines(&doc).concat();
                    assert_eq!(read.split_whitespace().collect::<String>(), typed.split_whitespace().collect::<String>());
                }
                Err(e) => assert!(!e.to_string().is_empty()),
            }
            assert!(started.elapsed().as_secs() < 30, "{:?}", started.elapsed());
        }
    }
    assert_eq!(written, 4);
}

#[test]
fn the_real_face_draws_a_bold_run_at_its_bold_instance() {
    if pdfcraft_fonts::document_arabic_font().is_none() {
        eprintln!("built without a craft-fonts Arabic face: the real-face check needs CRAFT_FONTS_DIR");
        return;
    }
    // The same word typed into a bold paragraph and a regular one: the bold one is drawn wider, from other glyphs.
    let written = |font: &str| {
        let mut doc = looks_page(&format!("BT /{font} 12 Tf 300 700 Td (BAB) Tj ( AB) Tj ET"));
        let typed = "بالعالم";
        text::replace_block(&mut doc, 0, 0, typed).expect("writes");
        assert_eq!(actual_texts(&doc), [typed]);
        assert_eq!(text::text_blocks(&reopen(&doc), 0).expect("blocks")[0].text, typed);
        let fonts = arabic_fonts(&doc);
        assert_eq!(fonts.len(), 1, "{fonts:?}");
        let (font_dict, _) = font_and_descriptor(&doc, &fonts[0]);
        let width: f64 = match font_dict.get(b"Widths") {
            Some(Object::Array(w)) => w.iter().filter_map(Object::as_f64).sum(),
            _ => 0.0,
        };
        (fonts[0].clone(), width)
    };
    let (regular, regular_width) = written("F1");
    let (bold, bold_width) = written("F2");
    assert!(!regular.ends_with('B') && bold.ends_with('B'), "{regular} {bold}");
    assert!(bold_width > regular_width * 1.02, "{bold_width} against {regular_width}");
}

// ---- read back ----

/// Each run of paragraph 0 of `doc` as it reads back, in reading order: its text in the editor, and the size, weight and
/// fill colour of the run it comes from.
fn read_looks(doc: &Document) -> Vec<(String, f64, bool, [f64; 3])> {
    let lines = text::text_lines(doc, 0).expect("lines");
    let block = text::text_blocks(doc, 0).expect("blocks").remove(0);
    block
        .spans
        .iter()
        .map(|(range, line)| {
            let line = &lines[*line];
            (block.editor_text[range.clone()].to_owned(), line.size, line.bold, line.color)
        })
        .collect()
}

/// What each string the page shows is drawn in, as [`shown`] gives it, with each generated Arabic font as its weight
/// (their names follow object numbers, which a second edit changes).
fn drawn_looks(doc: &Document) -> Vec<(String, f64, String)> {
    shown(doc)
        .into_iter()
        .map(|(font, size, fill)| {
            let font = match font {
                f if f.starts_with("PCEdAr") && f.ends_with('B') => "Arabic bold".to_owned(),
                f if f.starts_with("PCEdAr") => "Arabic".to_owned(),
                f => f,
            };
            (font, size, fill)
        })
        .collect()
}

/// The text of page 0 as copy and Find read it.
fn page_text(doc: &Document) -> pdfcraft_render::PageText {
    let bytes = pdfcraft_cos::write_incremental(doc, &pdfcraft_cos::SaveOptions::default()).expect("saves");
    let mut renderer = pdfcraft_render::PageRenderer::new(std::sync::Arc::new(bytes), pdfcraft_render::RenderConfig::default());
    renderer.filtered_text(0, pdfcraft_render::text::TextFilter::default()).expect("page text")
}

#[test]
fn a_paragraph_of_two_looks_reads_back_with_its_looks() {
    let _face = fake();
    for (source, second) in [(two_sizes(), (18.0, false, [0.0; 3])), (bold_red(), (12.0, true, [1.0, 0.0, 0.0]))] {
        let mut doc = source;
        let typed = format!("{}بالعالم", editor_text(&doc));
        text::replace_block(&mut doc, 0, 0, &typed).expect("writes");
        let doc = reopen(&doc);
        // The first word in the paragraph's first look, the space and the typed word in the second run's.
        assert_eq!(
            read_looks(&doc),
            [("مسممس".to_owned(), 12.0, false, [0.0; 3]), (" بالعالم".to_owned(), second.0, second.1, second.2)],
            "{:?}",
            shown(&doc)
        );
        let block = text::text_blocks(&doc, 0).expect("blocks").remove(0);
        assert_eq!((block.editor_text.as_str(), block.text.as_str()), (typed.as_str(), typed.as_str()));
    }
}

#[test]
fn a_paragraph_of_two_looks_retyped_twice_keeps_both() {
    let _face = fake();
    for source in [two_sizes(), bold_red()] {
        let mut doc = source;
        let typed = format!("{}بالعالم", editor_text(&doc));
        text::replace_block(&mut doc, 0, 0, &typed).expect("writes");
        let mut doc = reopen(&doc);
        let once = drawn_looks(&doc);
        let looks = read_looks(&doc);
        // The second word changed: it keeps its look, and the first word its own.
        let again = typed.replace("بالعالم", "سلام");
        text::replace_block(&mut doc, 0, 0, &again).expect("writes again");
        let doc = reopen(&doc);
        assert_eq!(drawn_looks(&doc), once, "{:?}", shown(&doc));
        let read: Vec<_> = read_looks(&doc).into_iter().map(|(_, size, bold, color)| (size, bold, color)).collect();
        assert_eq!(read, looks.iter().map(|(_, size, bold, color)| (*size, *bold, *color)).collect::<Vec<_>>());
        assert_eq!(text::text_blocks(&doc, 0).expect("blocks")[0].text, again);
        assert_eq!(read_lines(&doc), [again.as_str()]);
    }
}

#[test]
fn page_text_reads_a_paragraph_of_two_looks_as_it_reads_one_of_one_look() {
    let _face = fake();
    let typed = "مسممس بالعالم";
    let written = |source: Document| {
        let mut doc = source;
        text::replace_block(&mut doc, 0, 0, typed).expect("writes");
        doc
    };
    let (two, one) = (written(two_sizes()), written(looks_page("BT /F1 12 Tf 300 700 Td (BAB) Tj ( AB) Tj ET")));
    assert_eq!((actual_texts(&two).len(), actual_texts(&one).len()), (2, 1));
    let (two_text, one_text) = (page_text(&two), page_text(&one));
    assert_eq!(two_text.plain_text().trim(), typed);
    assert_eq!(two_text.plain_text(), one_text.plain_text());
    // Find finds each word, and the two across the look change.
    for word in ["مسممس", "بالعالم", typed] {
        assert_eq!(two_text.find(word).len(), 1, "{word}");
    }
    // Bold and red too, and a line whose looks are a Latin and an Arabic piece, which extraction would read the wrong
    // way round as two spans: it keeps one, and reads as typed.
    for (draft, spans) in [(typed.to_owned(), 2), (format!("{}PDF", editor_text(&bold_red())), 1)] {
        let mut doc = bold_red();
        text::replace_block(&mut doc, 0, 0, &draft).expect("writes");
        assert_eq!(actual_texts(&doc).len(), spans, "{draft}");
        assert_eq!(page_text(&doc).plain_text().trim(), draft);
        assert_eq!(text::text_blocks(&reopen(&doc), 0).expect("blocks")[0].text, draft);
    }
}

#[test]
fn the_glyphs_of_a_paragraph_of_two_looks_still_read_as_typed_without_its_spans() {
    let _face = fake();
    let mut doc = two_sizes();
    let typed = format!("{}بالعالم", editor_text(&doc));
    text::replace_block(&mut doc, 0, 0, &typed).expect("writes");
    // A reader that ignores `/ActualText` reads the generated fonts' ToUnicode maps: every letter once.
    let bare: Vec<Op> = ops(&doc).into_iter().filter(|op| !op.is("BDC") && !op.is("EMC")).collect();
    let mut stripped = doc.clone();
    let page = pdfcraft_model::pages(&stripped)[0].obj;
    let contents = stripped.add(Object::Stream(pdfcraft_cos::Stream::from_raw(pdfcraft_cos::Dict::new(), pdfcraft_content::serialize_ops(&bare))));
    stripped.update_dict(page, |d| d.set(b"Contents".to_vec(), Object::Ref(contents))).expect("updates");
    let letters = |text: &str| {
        let mut letters: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
        letters.sort_unstable();
        letters
    };
    let read: String = text::text_lines(&stripped, 0).expect("lines").iter().map(|l| l.text.clone()).collect();
    assert!(!read.is_empty());
    assert_eq!(letters(&read), letters(&typed), "{read:?}");
}

#[test]
fn a_look_change_at_every_word_reads_back_and_retypes_bounded() {
    let _face = fake();
    // 3,000 runs: Arabic words of one letter, regular and bold in turn, the spaces between them in the weight before.
    let mut content = String::from("BT /F1 12 Tf 30 700 Td ");
    for i in 0..3_000 {
        content.push_str(if i % 2 == 0 { "/F1 12 Tf (B ) Tj " } else { "/F2 12 Tf (A ) Tj " });
    }
    content.push_str("ET");
    let mut doc = looks_page(&content);
    let typed = format!("{}سم", editor_text(&doc));
    let started = std::time::Instant::now();
    text::rewrite_block(&mut doc, 0, 0, Some(&typed), &BlockStyle::default()).expect("writes");
    let doc = reopen(&doc);
    let block = text::text_blocks(&doc, 0).expect("blocks").remove(0);
    assert_eq!(words(&block.text), words(&typed));
    // Both weights read back, a run for each word (with its space), and a second edit keeps them.
    let looks = read_looks(&doc);
    assert!(looks.len() > 2_900 && looks.iter().filter(|l| l.2).count() > 1_400, "{} {:?}", looks.len(), &looks[..looks.len().min(8)]);
    assert!(looks.windows(2).filter(|pair| pair[0].2 != pair[1].2).count() > 2_800, "{:?}", &looks[..looks.len().min(8)]);
    let mut again = doc.clone();
    let retyped = format!("{} بالعالم", block.editor_text);
    text::rewrite_block(&mut again, 0, 0, Some(&retyped), &BlockStyle::default()).expect("writes again");
    let fonts = arabic_fonts(&again);
    assert!(fonts.iter().any(|f| f.ends_with('B')) && fonts.iter().any(|f| !f.ends_with('B')), "{:?}", &fonts[..fonts.len().min(8)]);
    assert_eq!(words(&read_lines(&again).join(" ")), words(&retyped));
    assert!(started.elapsed().as_secs() < 60, "{:?}", started.elapsed());
}

#[test]
fn the_real_face_reads_two_looks_back_and_keeps_them_retyped() {
    if pdfcraft_fonts::document_arabic_font().is_none() {
        eprintln!("built without a craft-fonts Arabic face: the real-face check needs CRAFT_FONTS_DIR");
        return;
    }
    let mut doc = bold_red();
    let typed = format!("{}بالعالم", editor_text(&doc));
    text::replace_block(&mut doc, 0, 0, &typed).expect("writes");
    let doc = reopen(&doc);
    assert_eq!(read_lines(&doc), [typed.as_str()]);
    assert_eq!(page_text(&doc).plain_text().trim(), typed);
    let looks: Vec<_> = read_looks(&doc).into_iter().map(|(_, _, bold, color)| (bold, color)).collect();
    assert_eq!(looks, [(false, [0.0; 3]), (true, [1.0, 0.0, 0.0])]);
    let mut again = doc.clone();
    let retyped = typed.replace("بالعالم", "سلام");
    text::replace_block(&mut again, 0, 0, &retyped).expect("writes again");
    assert_eq!(drawn_looks(&again), drawn_looks(&doc));
    assert_eq!(text::text_blocks(&reopen(&again), 0).expect("blocks")[0].text, retyped);
}

#[test]
fn right_to_left_spans_side_by_side_in_one_look_read_right_to_left() {
    // Two words of one look, each a span of its own in typing order, drawn left to right: the right one is read first,
    // whether they are one run or two.
    let doc = looks_page(
        "BT /F1 12 Tf 300 700 Td /Span <</ActualText <FEFF0633064520>>> BDC (BAB ) Tj EMC /Span <</ActualText <FEFF06450645>>> BDC (BB) Tj EMC ET",
    );
    let block = text::text_blocks(&doc, 0).expect("blocks").remove(0);
    assert_eq!(block.editor_text.trim(), "ممسم");
    let text = page_text(&doc).plain_text();
    assert_eq!(text.trim(), block.editor_text.trim());
}

// ---- italic ----

/// [`looks_page`] with `F3` italic and `F4` bold italic too.
fn slants_page(content: &str) -> Document {
    fonts_page(content, &["FakeArabic", "FakeArabic-Bold", "FakeArabic-Italic", "FakeArabic-BoldItalic"])
}

/// The `FontMatrix` of font resource `name` of page 0.
fn font_matrix(doc: &Document, name: &str) -> Vec<f64> {
    match font_and_descriptor(doc, name).0.get(b"FontMatrix") {
        Some(Object::Array(m)) => m.iter().filter_map(Object::as_f64).collect(),
        _ => Vec::new(),
    }
}

/// A generated font of slanted glyphs: its resource name ends in `I` (`BI` when bold too).
fn slanted(font: &str) -> bool {
    font.ends_with('I')
}

const UPRIGHT_MATRIX: [f64; 6] = [0.001, 0.0, 0.0, 0.001, 0.0, 0.0];
const ITALIC_MATRIX: [f64; 6] = [0.001, 0.0, 0.000_212_6, 0.001, 0.0, 0.0];

/// Whether every line of the page reads back italic (`true`) or upright, as a later Edit text takes it.
fn reads_back_italic(doc: &Document, italic: bool) -> bool {
    let lines = text::text_lines(&reopen(doc), 0).expect("lines");
    !lines.is_empty() && lines.iter().all(|l| l.italic == italic)
}

#[test]
fn an_italic_run_retyped_in_arabic_is_drawn_slanted_and_the_upright_run_upright() {
    let _face = fake();
    let mut doc = slants_page("BT /F1 12 Tf 300 700 Td (BAB) Tj 1 0 0 rg /F3 12 Tf ( AB) Tj ET");
    // A word typed after the italic red run's space takes its look: italic.
    let typed = format!("{}بالعالم", editor_text(&doc));
    text::replace_block(&mut doc, 0, 0, &typed).expect("writes");
    let drawn = shown(&doc);
    let red: Vec<_> = drawn.iter().filter(|(font, _, fill)| font.starts_with("PCEdAr") && fill.starts_with("rg")).collect();
    let black: Vec<_> = drawn.iter().filter(|(font, _, fill)| font.starts_with("PCEdAr") && fill.starts_with("g")).collect();
    assert!(!red.is_empty() && !black.is_empty(), "{drawn:?}");
    assert!(red.iter().all(|(font, _, _)| slanted(font) && !font.ends_with("BI")), "{drawn:?}");
    assert!(black.iter().all(|(font, _, _)| !slanted(font)), "{drawn:?}");
    // The italic run's font slants the glyphs; its widths are the upright stand-in's (a slant changes no advance).
    assert_eq!(font_matrix(&doc, &red[0].0), ITALIC_MATRIX);
    assert_eq!(font_matrix(&doc, &black[0].0), UPRIGHT_MATRIX);
    let (italic_font, italic_descriptor) = font_and_descriptor(&doc, &red[0].0);
    assert_eq!(italic_descriptor.get(b"ItalicAngle").and_then(Object::as_f64), Some(-12.0));
    let regular_advances = [250.0, 400.0, 500.0, 520.0, 540.0, 560.0, 600.0];
    let widths: Vec<f64> = match italic_font.get(b"Widths") {
        Some(Object::Array(w)) => w.iter().filter_map(Object::as_f64).collect(),
        _ => Vec::new(),
    };
    assert!(!widths.is_empty() && widths.iter().all(|w| regular_advances.contains(w)), "{widths:?}");
    // The text is what it was: a span per look, which read in order give the line as typed, and the ToUnicode maps give
    // back every letter once.
    assert_eq!(read_lines(&doc), [typed.as_str()]);
    let reopened = reopen(&doc);
    assert_eq!(text::text_blocks(&reopened, 0).expect("blocks")[0].text, typed);
    // Read back, each run is the slant it was drawn at, so a second edit keeps them.
    let lines = text::text_lines(&reopened, 0).expect("lines");
    let italics: Vec<(bool, bool)> = lines.iter().map(|l| (l.font.starts_with("PCEdAr") && slanted(&l.font), l.italic)).collect();
    assert!(italics.iter().any(|(_, italic)| *italic) && italics.iter().all(|(drawn, italic)| drawn == italic), "{italics:?}");
    let mut again = reopened.clone();
    let retyped = format!("{} سم", editor_text(&again));
    text::replace_block(&mut again, 0, 0, &retyped).expect("writes again");
    let fonts = arabic_fonts(&again);
    assert!(fonts.iter().any(|f| slanted(f)) && fonts.iter().any(|f| !slanted(f)), "{fonts:?}");
    assert_eq!(text::text_blocks(&reopen(&again), 0).expect("blocks")[0].text, retyped);
}

#[test]
fn a_whole_italic_paragraph_is_slanted_on_the_one_look_path_and_keeps_its_slant() {
    let _face = fake();
    for (font, italic, bold) in [("F3", true, false), ("F4", true, true), ("F1", false, false), ("F2", false, true)] {
        let mut doc = slants_page(&format!("BT /{font} 12 Tf 300 700 Td (BAB) Tj ( AB) Tj ET"));
        let typed = format!("{}بالعالم", editor_text(&doc));
        text::replace_block(&mut doc, 0, 0, &typed).expect("writes");
        let fonts = arabic_fonts(&doc);
        assert_eq!(fonts.len(), 1, "{font}: {:?}", shown(&doc));
        assert_eq!((slanted(&fonts[0]), fonts[0].trim_end_matches('I').ends_with('B')), (italic, bold), "{font}: {fonts:?}");
        assert_eq!(font_matrix(&doc, &fonts[0]), if italic { ITALIC_MATRIX } else { UPRIGHT_MATRIX });
        assert_eq!(actual_texts(&doc), [typed.as_str()]);
        assert_eq!(text::text_blocks(&reopen(&doc), 0).expect("blocks")[0].text, typed);
        assert!(reads_back_italic(&doc, italic), "{font}");
        // A second edit reads the slant back from the generated font and draws it again.
        let mut again = reopen(&doc);
        let retyped = format!("{} سم", editor_text(&again));
        text::replace_block(&mut again, 0, 0, &retyped).expect("writes again");
        let fonts = arabic_fonts(&again);
        assert!(!fonts.is_empty() && fonts.iter().all(|f| slanted(f) == italic), "{font}: {fonts:?}");
    }
}

#[test]
fn the_slant_asked_for_in_the_format_panel_wins_over_the_runs() {
    let _face = fake();
    for (font, asked) in [("F1", true), ("F3", false)] {
        let mut doc = slants_page(&format!("BT /{font} 12 Tf 300 700 Td (BAB) Tj ( AB) Tj ET"));
        let typed = format!("{}بالعالم", editor_text(&doc));
        let style = BlockStyle { family: Some((crate::added::Family::Helvetica, false, asked)), ..Default::default() };
        text::rewrite_block(&mut doc, 0, 0, Some(&typed), &style).expect("writes");
        let fonts = arabic_fonts(&doc);
        assert!(!fonts.is_empty() && fonts.iter().all(|f| slanted(f) == asked), "{font} {asked}: {fonts:?}");
        // (Its Latin font, a standard one of that slant, and the Arabic agree.)
        assert!(reads_back_italic(&doc, asked), "{font} {asked}");
    }
    // A weight asked for alone keeps the run's slant.
    let mut doc = slants_page("BT /F3 12 Tf 300 700 Td (BAB) Tj ( AB) Tj ET");
    let typed = format!("{}بالعالم", editor_text(&doc));
    text::rewrite_block(&mut doc, 0, 0, Some(&typed), &BlockStyle { bold: Some(true), ..Default::default() }).expect("writes");
    let fonts = arabic_fonts(&doc);
    assert!(!fonts.is_empty() && fonts.iter().all(|f| f.ends_with("BI")), "{fonts:?}");
}

#[test]
fn a_line_edited_in_arabic_keeps_its_slant() {
    let _face = fake();
    for (font, italic) in [("F3", true), ("F1", false)] {
        let mut doc = slants_page(&format!("BT /{font} 12 Tf 300 700 Td (BAB) Tj ET"));
        text::replace_line(&mut doc, 0, 0, "بالعالم").expect("writes");
        let fonts = arabic_fonts(&doc);
        assert!(!fonts.is_empty() && fonts.iter().all(|f| slanted(f) == italic), "{font}: {fonts:?}");
        assert!(fonts.iter().all(|f| font_matrix(&doc, f) == if italic { ITALIC_MATRIX } else { UPRIGHT_MATRIX }));
        assert_eq!(actual_texts(&doc), ["بالعالم"]);
    }
}

#[test]
fn a_slant_change_at_every_word_stays_bounded() {
    let _face = fake();
    // 4,000 runs: Arabic words of one letter, upright and italic in turn, the spaces between them in the slant before.
    let mut content = String::from("BT /F1 12 Tf 30 700 Td ");
    for i in 0..2_000 {
        content.push_str(if i % 2 == 0 { "/F1 12 Tf (B ) Tj " } else { "/F3 12 Tf (A ) Tj " });
    }
    content.push_str("ET");
    let source = slants_page(&content);
    let source_text = editor_text(&source);
    assert!(source_text.chars().count() >= 3_999, "{}", source_text.chars().count());
    let mut written = 0;
    for typed in [format!("{source_text}سم PDF"), format!("{source_text}{}", "ب ".repeat(2_000))] {
        for style in [BlockStyle::default(), BlockStyle { width: Some(200.0), align: Some(Align::Justify), ..Default::default() }] {
            let mut doc = source.clone();
            let started = std::time::Instant::now();
            match text::rewrite_block(&mut doc, 0, 0, Some(&typed), &style) {
                Ok(_) => {
                    written += 1;
                    let bytes = page_content_bytes(&doc, 0).len();
                    assert!(bytes < 4_000_000, "{bytes} bytes for {} characters", typed.chars().count());
                    let fonts = arabic_fonts(&doc);
                    assert!(fonts.iter().any(|f| slanted(f)) && fonts.iter().any(|f| !slanted(f)), "{:?}", &fonts[..fonts.len().min(8)]);
                    let read: String = read_lines(&doc).concat();
                    assert_eq!(read.split_whitespace().collect::<String>(), typed.split_whitespace().collect::<String>());
                }
                Err(e) => assert!(!e.to_string().is_empty()),
            }
            assert!(started.elapsed().as_secs() < 30, "{:?}", started.elapsed());
        }
    }
    assert_eq!(written, 4);
}

#[test]
fn the_real_face_draws_an_italic_paragraph_from_the_upright_glyphs_sheared() {
    if pdfcraft_fonts::document_arabic_font().is_none() {
        eprintln!("built without a craft-fonts Arabic face: the real-face italic check needs CRAFT_FONTS_DIR");
        return;
    }
    let written = |font: &str| {
        let mut doc = slants_page(&format!("BT /{font} 12 Tf 300 700 Td (BAB) Tj ( AB) Tj ET"));
        let typed = "بالعالم";
        text::replace_block(&mut doc, 0, 0, typed).expect("writes");
        assert_eq!(actual_texts(&doc), [typed]);
        assert_eq!(text::text_blocks(&reopen(&doc), 0).expect("blocks")[0].text, typed);
        let fonts = arabic_fonts(&doc);
        assert_eq!(fonts.len(), 1, "{fonts:?}");
        let (font_dict, _) = font_and_descriptor(&doc, &fonts[0]);
        let procs: Vec<Vec<u8>> = match font_dict.get(b"CharProcs") {
            Some(Object::Dict(procs)) => procs
                .iter()
                .filter_map(|(_, p)| match &*doc.resolve(p) {
                    Object::Stream(s) => s.decoded().ok(),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        (fonts[0].clone(), font_matrix(&doc, &fonts[0]), font_dict.get(b"Widths").cloned(), procs)
    };
    let (upright, upright_matrix, upright_widths, upright_procs) = written("F1");
    let (italic, italic_matrix, italic_widths, italic_procs) = written("F3");
    assert!(!slanted(&upright) && slanted(&italic), "{upright} {italic}");
    assert_eq!((upright_matrix.as_slice(), italic_matrix.as_slice()), (UPRIGHT_MATRIX.as_slice(), ITALIC_MATRIX.as_slice()));
    assert_eq!(italic_widths, upright_widths);
    assert!(!italic_procs.is_empty());
    assert_eq!(italic_procs, upright_procs);
}
