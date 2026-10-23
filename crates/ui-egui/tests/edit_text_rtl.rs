//! Edit a PDF ▸ Edit text with Arabic and Hebrew in the real shell (egui_kittest): the paragraph's editor
//! shows the text in display order, a right-to-left row against the right edge of the paragraph, while the
//! draft, the edit that is applied and the page's copy of the text (`/ActualText`) stay in typing order.
//!
//! The tests need the Arabic face (a build with `CRAFT_FONTS_DIR`); without one they return at once.

use egui::{Key, Modifiers, Pos2};
use egui_kittest::Harness;
use pdfcraft_engine::{BlockStyle, Edit};
use pdfcraft_ui_egui::PdfCraftApp;

/// "Hello wide world friends" in Helvetica 12 pt, at (20, 150) of a 200 × 300 pt page.
fn page() -> Vec<u8> {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [4 0 R] /Count 1 /MediaBox [0 0 200 300] >>".to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /Font << /F1 3 0 R >> >> >>".to_owned(),
        {
            let body = "BT /F1 12 Tf 20 150 Td (Hello wide world friends) Tj ET";
            format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len())
        },
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

fn harness(zoom: &str) -> Harness<'static, PdfCraftApp> {
    harness_with(zoom, page())
}

fn harness_with(zoom: &str, bytes: Vec<u8>) -> Harness<'static, PdfCraftApp> {
    let zoom = zoom.to_owned();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("doc.pdf", None, bytes).expect("fixture opens");
        app.set_option("zoom", &zoom).unwrap();
        app
    });
    h.run_steps(4);
    h
}

/// The page's paragraph is `text` now (written by the engine, as an applied edit would).
fn write(h: &mut Harness<'static, PdfCraftApp>, text: &str) -> bool {
    h.state_mut().apply_edit(Edit::EditTextBlock { page: 0, block: 0, text: text.into(), style: BlockStyle::default() })
}

fn block(h: &Harness<'static, PdfCraftApp>) -> pdfcraft_engine::TextBlock {
    let s = h.state();
    s.session.get(s.views[0].id).unwrap().text_blocks(0).remove(0)
}

/// Where the user-space point `(x, y)` of the page is on screen.
fn at(h: &Harness<'static, PdfCraftApp>, x: f64, y: f64) -> Pos2 {
    let r = h.state().views[0].page_screen_rect(0).expect("on screen");
    Pos2::new(r.left() + x as f32 / 200.0 * r.width(), r.top() + (300.0 - y as f32) / 300.0 * r.height())
}

/// Opens the paragraph for editing with a click on it.
fn open_editor(h: &mut Harness<'static, PdfCraftApp>) {
    assert!(h.state_mut().execute("edit.edit_text"));
    h.run_steps(3);
    let b = block(h);
    let point = at(h, b.rect[0] + 6.0, (b.rect[1] + b.rect[3]) / 2.0);
    h.hover_at(point);
    h.run_steps(1);
    h.drag_at(point);
    h.run_steps(1);
    h.drop_at(point);
    h.run_steps(4);
    assert!(h.state().views[0].line_editor.is_some(), "the paragraph opens for editing");
}

fn editor_text(h: &Harness<'static, PdfCraftApp>) -> String {
    h.state().views[0].line_editor.as_ref().expect("the editor is open").text.clone()
}

/// The galleys of every text the last frame painted, with where each is painted.
fn painted_galleys(h: &Harness<'static, PdfCraftApp>) -> Vec<(Pos2, std::sync::Arc<egui::Galley>)> {
    fn walk(shape: &egui::Shape, out: &mut Vec<(Pos2, std::sync::Arc<egui::Galley>)>) {
        match shape {
            egui::Shape::Text(text) => out.push((text.pos, std::sync::Arc::clone(&text.galley))),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in &h.output().shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

/// The galley the editor paints: the one that holds `display` (the draft in display order, one line).
fn editor_galley(h: &Harness<'static, PdfCraftApp>, display: &str) -> (Pos2, std::sync::Arc<egui::Galley>) {
    let galleys = painted_galleys(h);
    let seen: Vec<String> = galleys.iter().map(|(_, g)| g.job.text.clone()).collect();
    galleys.into_iter().find(|(_, g)| g.job.text == display).unwrap_or_else(|| panic!("the editor paints {display:?}; painted: {seen:?}"))
}

/// The decoded streams of the saved document that mention `needle`.
fn saved_streams_with(h: &Harness<'static, PdfCraftApp>, needle: &str) -> Vec<String> {
    let state = h.state();
    let bytes = state.session.save_bytes(state.views[0].id).unwrap();
    let doc = pdfcraft_cos::Document::open(bytes).unwrap();
    doc.object_numbers()
        .into_iter()
        .filter_map(|n| match &*doc.get(pdfcraft_cos::ObjRef::new(n, 0)) {
            pdfcraft_cos::Object::Stream(stream) => stream.decoded().ok(),
            _ => None,
        })
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .filter(|text| text.contains(needle))
        .collect()
}

/// The `/ActualText` marker the page carries for `text`.
fn actual_text(text: &str) -> String {
    let hex: String = text.encode_utf16().map(|u| format!("{u:04X}")).collect();
    format!("/ActualText <FEFF{hex}>")
}

const SALAM_ALAYKUM: &str = "\u{633}\u{644}\u{627}\u{645} \u{639}\u{644}\u{64A}\u{643}\u{645}";
/// ... in display order: the words are swapped.
const SALAM_ALAYKUM_DISPLAYED: &str = "\u{639}\u{644}\u{64A}\u{643}\u{645} \u{633}\u{644}\u{627}\u{645}";

/// Arabic text the page has opens in the editor in typing order, drawn in display order with its row against
/// the right edge of the paragraph, at any zoom.
#[test]
fn an_arabic_paragraph_opens_in_display_order_against_its_right_edge() {
    if pdfcraft_fonts::document_arabic_font().is_none() {
        return;
    }
    for zoom in ["100", "150", "300"] {
        let mut h = harness(zoom);
        assert!(write(&mut h, SALAM_ALAYKUM), "the engine writes the Arabic text");
        h.run_steps(3);
        let b = block(&h);
        assert_eq!(b.editor_text, SALAM_ALAYKUM, "the page reads it back in typing order");
        let written = h.state().session.get(h.state().views[0].id).unwrap().bytes.clone();
        open_editor(&mut h);
        assert_eq!(editor_text(&h), b.editor_text, "the editor holds the typing order at {zoom}%");
        let (pos, galley) = editor_galley(&h, SALAM_ALAYKUM_DISPLAYED);
        // The first word is to the right of the second.
        let glyphs: Vec<(char, f32)> = galley.rows.iter().flat_map(|row| row.glyphs.iter().map(|g| (g.chr, g.pos.x))).collect();
        assert_eq!(glyphs.len(), SALAM_ALAYKUM.chars().count(), "a glyph for every character at {zoom}%");
        let (second, first) = glyphs.split_at("\u{639}\u{644}\u{64A}\u{643}\u{645} ".chars().count());
        let second_right = second.iter().map(|g| g.1).fold(f32::MIN, f32::max);
        let first_left = first.iter().map(|g| g.1).fold(f32::MAX, f32::min);
        assert!(first_left > second_right, "the first word is right of the second at {zoom}%: {first_left} against {second_right}");
        // The row ends where the paragraph's right edge is on the page.
        let row = &galley.rows[0];
        let right = pos.x + row.pos.x + row.row.size.x;
        let expected = at(&h, b.rect[2], b.rect[1]).x;
        assert!((right - expected).abs() <= 2.0, "the row ends at {right}, the paragraph at {expected} ({zoom}%)");
        if let Ok(dir) = std::env::var("PDFCRAFT_SHOTS") {
            h.render().unwrap().save(format!("{dir}/edit-text-arabic-{zoom}.png")).unwrap();
        }
        // Nothing changed: leaving applies nothing.
        h.key_press(Key::Escape);
        h.run_steps(3);
        assert!(h.state().views[0].line_editor.is_none());
        assert_eq!(h.state().session.get(h.state().views[0].id).unwrap().bytes, written, "nothing was applied at {zoom}%");
    }
}

/// Typing and retyping Arabic in the editor: the draft grows in typing order, shows in display order (the
/// newest word at the left), and what is applied and what the page keeps for copy and search is the draft.
#[test]
fn typed_arabic_in_the_edit_text_box_is_applied_and_marked_in_typing_order() {
    if pdfcraft_fonts::document_arabic_font().is_none() {
        return;
    }
    let mut h = harness("150");
    assert!(write(&mut h, SALAM_ALAYKUM));
    h.run_steps(3);
    open_editor(&mut h);
    let before = block(&h);
    // A third word at the end of the text.
    let marhaba = "\u{645}\u{631}\u{62D}\u{628}\u{627}";
    h.event(egui::Event::Text(format!(" {marhaba}")));
    h.run_steps(4);
    let typed = format!("{SALAM_ALAYKUM} {marhaba}");
    assert_eq!(editor_text(&h), typed, "the draft keeps the typing order");
    let displayed = format!("{marhaba} {SALAM_ALAYKUM_DISPLAYED}");
    let (pos, galley) = editor_galley(&h, &displayed);
    assert!(h.state().views[0].line_editor.as_ref().is_some_and(|ed| ed.has_unsaved_text()));
    // The line grew to the left: its right edge stays where the paragraph's is.
    let row = &galley.rows[0];
    let (left, right) = (pos.x + row.pos.x, pos.x + row.pos.x + row.row.size.x);
    assert!((right - at(&h, before.rect[2], before.rect[1]).x).abs() <= 2.0, "the right edge stays at the paragraph's: {right}");
    assert!(left < at(&h, before.rect[0], before.rect[1]).x - 2.0, "the line grew to the left: {left}");
    // Apply: the page's paragraph is the draft, in typing order, with its copy marked as such.
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run_steps(4);
    assert!(h.state().views[0].line_editor.is_none(), "applied");
    assert_eq!(block(&h).editor_text, typed, "the page reads back what was typed");
    assert!(!saved_streams_with(&h, &actual_text(&typed)).is_empty(), "the page keeps the typing order for copy and search");
    // Retype all of it.
    open_editor(&mut h);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    let selected = h.state().views[0].line_editor.as_ref().and_then(|ed| ed.selection.clone());
    assert_eq!(selected, Some(0..typed.len()), "the selection is the whole draft, in bytes of the draft");
    // ("a new text": narrower than the three words the paragraph has by now)
    let kitab = "\u{646}\u{635} \u{62C}\u{62F}\u{64A}\u{62F}";
    h.event(egui::Event::Text(kitab.into()));
    h.run_steps(4);
    assert_eq!(editor_text(&h), kitab);
    let displayed = "\u{62C}\u{62F}\u{64A}\u{62F} \u{646}\u{635}";
    editor_galley(&h, displayed);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run_steps(4);
    assert_eq!(block(&h).editor_text, kitab);
    assert!(!saved_streams_with(&h, &actual_text(kitab)).is_empty(), "retyped text is marked in typing order");
}

/// The editor breaks a retyped Arabic paragraph into the rows the page will have after Apply (the rewrite measures the words
/// shaped, in the first row's font and size): the rows the editor paints are the page's, read the other way round.
#[test]
fn the_edit_text_box_breaks_arabic_lines_where_apply_writes_them() {
    if pdfcraft_fonts::document_arabic_font().is_none() {
        return;
    }
    let first = "سلام عليكم ورحمة الله وبركاته مع أطيب التحيات من الجميع هنا اليوم وفي كل مكان";
    let second = "هذا نص عربي طويل يكتب في حقل ضيق ليلتف على عدة أسطر ويبقى مطابقا لما تكتبه الصفحة عند التطبيق";
    for zoom in ["100", "150", "300"] {
        let mut h = harness(zoom);
        assert!(write(&mut h, first), "the engine writes the Arabic paragraph");
        h.run_steps(3);
        assert!(block(&h).rows.len() >= 2, "the paragraph has several rows at {zoom}%");
        open_editor(&mut h);
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.event(egui::Event::Text(second.into()));
        h.run_steps(4);
        let galleys = painted_galleys(&h);
        let (_, galley) =
            galleys.iter().find(|(_, g)| g.job.text.contains('\u{647}') && g.job.text.contains('\n')).expect("the editor paints the draft");
        let rows: Vec<String> =
            galley.rows.iter().map(|r| r.glyphs.iter().map(|g| g.chr).collect::<String>().trim().chars().rev().collect()).collect();
        h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
        h.run_steps(4);
        let b = block(&h);
        let lines = h.state().session.get(h.state().views[0].id).unwrap().text_lines(0);
        let page: Vec<String> = b.rows.iter().map(|row| row.iter().map(|i| lines[*i].text.clone()).collect::<Vec<_>>().join(" ")).collect();
        assert!(page.len() >= 3, "the retyped paragraph wraps at {zoom}%: {page:?}");
        assert_eq!(rows, page, "the editor's rows are the page's at {zoom}%");
    }
}

/// What the editor and the page make of a paragraph of Arabic written `bold` or not, with a word typed at its end: the width of
/// the editor's row, of the row the engine lays the draft out as (`text_edit_layout`), and of the page's row after Apply, in
/// screen points at 150%.
fn typed_row_widths(bold: bool) -> (f32, f32, f32) {
    let mut h = harness("150");
    let style = BlockStyle { bold: Some(bold), ..Default::default() };
    assert!(h.state_mut().apply_edit(Edit::EditTextBlock { page: 0, block: 0, text: SALAM_ALAYKUM.into(), style }));
    h.run_steps(3);
    assert_eq!(block(&h).bold, bold, "the page reads the paragraph back as bold: {bold}");
    open_editor(&mut h);
    assert_eq!(h.state().views[0].line_editor.as_ref().expect("the editor is open").look.bold, bold, "the editor's look");
    let marhaba = "\u{645}\u{631}\u{62D}\u{628}\u{627}";
    h.event(egui::Event::Text(format!(" {marhaba}")));
    h.run_steps(4);
    let (_, galley) = editor_galley(&h, &format!("{marhaba} {SALAM_ALAYKUM_DISPLAYED}"));
    assert_eq!(galley.rows.len(), 1, "one row");
    let shown = galley.rows[0].row.size.x;
    let px_per_pt = h.state().views[0].page_screen_rect(0).expect("on screen").width() / 200.0;
    let measured = {
        let state = h.state();
        let ed = state.views[0].line_editor.as_ref().expect("the editor is open");
        let layout = state.session.get(state.views[0].id).unwrap().text_edit_layout(0, 0, &ed.text, &ed.style()).expect("the draft is measured");
        assert_eq!(layout.rows.len(), 1, "the engine lays the draft out in one row: {:?}", layout.rows);
        let r = layout.row_rects[0];
        (r[2] - r[0]) as f32 * px_per_pt
    };
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run_steps(4);
    assert!(h.state().views[0].line_editor.is_none(), "applied");
    let page = page_ink(&h);
    assert_eq!(page.len(), 1, "one row on the page");
    (shown, measured, page[0].1 - page[0].0)
}

/// A paragraph whose Arabic the page draws bold opens in the editor at the Arabic face's bold instance: its row is wider than the
/// same words regular, and as wide as the row the engine lays out and Apply writes (the box drew bold Arabic at the face's
/// regular instance before, an eighth narrower than the page).
#[test]
fn bold_arabic_is_as_wide_in_the_edit_text_box_as_on_the_page() {
    if !pdfcraft_fonts::arabic_bold_available() {
        return;
    }
    let (regular, regular_measured, regular_page) = typed_row_widths(false);
    let (bold, measured, page) = typed_row_widths(true);
    assert!(page > regular_page * 1.05, "the page's bold row is wider: {page} against {regular_page}");
    assert!(bold > regular * 1.05, "the editor's bold row is wider than its regular one: {bold} against {regular}");
    for (weight, shown, measured, page) in [("regular", regular, regular_measured, regular_page), ("bold", bold, measured, page)] {
        assert!((shown - measured).abs() <= measured * 0.02, "{weight}: the editor's row is {shown} wide, the engine's {measured}");
        assert!((shown - page).abs() <= page * 0.02, "{weight}: the editor's row is {shown} wide, the page's after Apply {page}");
    }
}

/// Latin typed into a block that holds Arabic reads in its own direction inside the right-to-left row, and
/// Escape leaves the page as it was.
#[test]
fn escape_leaves_an_arabic_paragraph_as_it_was() {
    if pdfcraft_fonts::document_arabic_font().is_none() {
        return;
    }
    let mut h = harness("150");
    assert!(write(&mut h, SALAM_ALAYKUM));
    h.run_steps(3);
    let before = h.state().session.get(h.state().views[0].id).unwrap().bytes.clone();
    open_editor(&mut h);
    h.event(egui::Event::Text(" abc".into()));
    h.run_steps(3);
    assert_eq!(editor_text(&h), format!("{SALAM_ALAYKUM} abc"));
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state().views[0].line_editor.is_none());
    assert_eq!(h.state().session.get(h.state().views[0].id).unwrap().bytes, before, "cancelling changes nothing");
}

/// Hebrew is edited in display order too. (The draft is set here, as other tests do: writing Hebrew to the
/// page needs the Hebrew face, which a build without craft-fonts' Hebrew does not have.)
#[test]
fn a_hebrew_draft_is_edited_in_display_order() {
    let mut h = harness("150");
    open_editor(&mut h);
    let hebrew = "\u{5E9}\u{5DC}\u{5D5}\u{5DD} \u{5E2}\u{5D5}\u{5DC}\u{5DD}";
    h.state_mut().views[0].line_editor.as_mut().expect("the editor is open").text = hebrew.into();
    h.run_steps(3);
    let displayed = "\u{5E2}\u{5D5}\u{5DC}\u{5DD} \u{5E9}\u{5DC}\u{5D5}\u{5DD}";
    let (pos, galley) = editor_galley(&h, displayed);
    let b = block(&h);
    let row = &galley.rows[0];
    let right = pos.x + row.pos.x + row.row.size.x;
    assert!((right - at(&h, b.rect[2], b.rect[1]).x).abs() <= 2.0, "the row ends at the paragraph's right edge: {right}");
    h.event(egui::Event::Text(" \u{5D0}\u{5D1}".into()));
    h.run_steps(3);
    assert_eq!(editor_text(&h), format!("{hebrew} \u{5D0}\u{5D1}"), "typed at the end, in typing order");
    editor_galley(&h, &format!("\u{5D0}\u{5D1} {displayed}"));
}

/// A centred two-row Arabic paragraph opens with each row where the page has it (they end at different
/// places), and once its text changes the rows are against the right edge of the paragraph, as the engine
/// right-aligns right-to-left text.
#[test]
fn the_rows_of_a_centred_arabic_paragraph_open_where_the_page_has_them() {
    if pdfcraft_fonts::document_arabic_font().is_none() {
        return;
    }
    let mut h = harness("150");
    let text = format!("{SALAM_ALAYKUM} {SALAM_ALAYKUM}\n\u{633}\u{644}\u{627}\u{645}");
    let centred = BlockStyle { align: Some(pdfcraft_engine::TextAlign::Center), ..Default::default() };
    assert!(h.state_mut().apply_edit(Edit::EditTextBlock { page: 0, block: 0, text: text.clone(), style: centred }));
    h.run_steps(3);
    let b = block(&h);
    assert_eq!(b.rows.len(), 2, "two rows on the page");
    let lines = h.state().session.get(h.state().views[0].id).unwrap().text_lines(0);
    let page_rights: Vec<f32> = b.rows.iter().map(|row| at(&h, lines[row[0]].rect[2], 0.0).x).collect();
    assert!((page_rights[0] - page_rights[1]).abs() > 3.0, "the page's rows end at different places: {page_rights:?}");
    open_editor(&mut h);
    assert_eq!(editor_text(&h), b.editor_text);
    let displayed = "\u{639}\u{644}\u{64A}\u{643}\u{645} \u{633}\u{644}\u{627}\u{645} \u{639}\u{644}\u{64A}\u{643}\u{645} \u{633}\u{644}\u{627}\u{645}\n\u{633}\u{644}\u{627}\u{645}";
    let (pos, galley) = editor_galley(&h, displayed);
    assert_eq!(galley.rows.len(), 2);
    for (i, row) in galley.rows.iter().enumerate() {
        let right = pos.x + row.pos.x + row.row.size.x;
        assert!((right - page_rights[i]).abs() <= 2.5, "row {i} ends at {right}, the page's row at {}", page_rights[i]);
    }
    // A letter more: the rows are against the right edge of the paragraph.
    h.event(egui::Event::Text("\u{628}".into()));
    h.run_steps(4);
    let galley = painted_galleys(&h).into_iter().find(|(_, g)| g.job.text.contains('\n') && g.job.text.contains('\u{633}')).expect("the editor");
    let paragraph_right = at(&h, b.rect[2], 0.0).x;
    for (i, row) in galley.1.rows.iter().enumerate() {
        let right = galley.0.x + row.pos.x + row.row.size.x;
        assert!((right - paragraph_right).abs() <= 2.5, "row {i} ends at {right}, the paragraph at {paragraph_right}");
    }
}

/// The block reads in the direction the page will write it, not the one its first letter says: more Arabic
/// letters than Latin ones make it right to left though it starts with an English word.
#[test]
fn a_block_with_more_arabic_than_latin_reads_right_to_left_whatever_it_starts_with() {
    if pdfcraft_fonts::document_arabic_font().is_none() {
        return;
    }
    let mut h = harness("150");
    open_editor(&mut h);
    let draft = format!("Hi {SALAM_ALAYKUM}");
    h.state_mut().views[0].line_editor.as_mut().expect("the editor is open").text = draft.clone();
    h.run_steps(3);
    // The line is right to left: "Hi", read first, is at the right, then the first Arabic word, then the second.
    let (pos, galley) = editor_galley(&h, &format!("{SALAM_ALAYKUM_DISPLAYED} Hi"));
    let row = &galley.rows[0];
    let b = block(&h);
    let right = pos.x + row.pos.x + row.row.size.x;
    assert!((right - at(&h, b.rect[2], b.rect[1]).x).abs() <= 2.5, "the row is against the right edge: {right}");
}

// ---- the page's own face ----

/// A complete static Arabic face for the tests: the one `PDFCRAFT_TEST_ARABIC_FONT` names, else the craft-fonts face if it is
/// static (the bundled Noto Sans Arabic is a variable font, which the rewrite cannot embed). `None` skips the test.
fn static_face() -> Option<Vec<u8>> {
    let bytes = match std::env::var_os("PDFCRAFT_TEST_ARABIC_FONT") {
        Some(path) => std::fs::read(path).ok()?,
        None => pdfcraft_fonts::document_arabic_font()?.bytes.to_vec(),
    };
    if pdfcraft_fonts::embedding::validate(&bytes).is_err() {
        eprintln!("skipping: the Arabic face is variable; set PDFCRAFT_TEST_ARABIC_FONT to a static face to run this test");
        return None;
    }
    Some(bytes)
}

/// A 200 × 300 pt page whose Arabic `rows` are set in the complete face `face` as a producer that embeds a whole font does: the
/// glyphs of each row (shaped, in drawing order) shown by code at 12 pt, the rows against the right edge at x = 180 and 15 pt
/// apart from y = 200, with the row's text behind them as `/ActualText`.
fn face_page(face: &[u8], rows: &[&str]) -> Vec<u8> {
    use pdfcraft_cos::{Dict, Object, SaveOptions, Stream};
    let mut doc = pdfcraft_cos::Document::open(std::sync::Arc::new(page())).expect("the page opens");
    let (_, postscript) = pdfcraft_fonts::embedding::names(face).expect("the face has names");
    let mut fonts = Dict::new();
    let resource = pdfcraft_fonts::embedding::embed(&mut doc, &mut fonts, face, &postscript).expect("the face embeds");
    // A glyph is shown by the first code of the font that is mapped to it.
    let top = doc.dict(fonts.get(resource.as_bytes()).expect("the font")).expect("the font dictionary");
    let Some(Object::Array(descendants)) = top.get(b"DescendantFonts").map(|d| doc.resolve(d).as_ref().clone()) else { panic!("descendant fonts") };
    let descendant = doc.dict(descendants.first().expect("a descendant")).expect("the descendant font");
    let Object::Stream(map) = &*doc.resolve(descendant.get(b"CIDToGIDMap").expect("a glyph map")) else { panic!("a glyph map stream") };
    let gids: Vec<u16> = map.decoded().expect("decodes").as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
    let mut content = String::new();
    for (i, row) in rows.iter().enumerate() {
        let shaped = pdfcraft_fonts::source_shaping::shape(face, row).expect("the face shapes the row");
        let codes: String = shaped
            .glyphs
            .iter()
            .map(|g| {
                let gid = u16::try_from(g.id.to_u32()).expect("a glyph id");
                format!("{:04X}", gids.iter().position(|v| *v == gid && gid != 0).expect("the font maps the glyph"))
            })
            .collect();
        let logical: String = row.encode_utf16().map(|u| format!("{u:04X}")).collect();
        let (x, y) = (180.0 - shaped.advance * 12.0, 200.0 - 15.0 * i as f64);
        content += &format!("/Span <</ActualText <FEFF{logical}>>> BDC BT /{resource} 12 Tf 1 0 0 1 {x:.3} {y} Tm <{codes}> Tj ET EMC\n");
    }
    let page_ref = pdfcraft_model::pages(&doc)[0].obj;
    let mut resources = Dict::new();
    resources.set(b"Font".to_vec(), Object::Dict(fonts));
    let contents = doc.add(Object::Stream(Stream::flate(Dict::new(), content.as_bytes())));
    doc.update_dict(page_ref, |d| {
        d.set(b"Resources".to_vec(), Object::Dict(resources));
        d.set(b"Contents".to_vec(), Object::Ref(contents));
    })
    .expect("the page updates");
    pdfcraft_cos::write_full(&doc, &SaveOptions::default()).expect("the page saves")
}

const FIRST_ROW: &str = "\u{633}\u{644}\u{627}\u{645} \u{639}\u{644}\u{64A}\u{643}\u{645} \u{645}\u{631}\u{62D}\u{628}\u{627} \u{628}\u{643}\u{645}";
const SECOND_ROW: &str = "\u{643}\u{62A}\u{627}\u{628} \u{62C}\u{62F}\u{64A}\u{62F}";
/// Words the paragraph does not have (a retyped word that is one of its words takes the look of its row, and a row of two
/// source rows' looks is refused by the rewrite), long enough to wrap in the 160 pt box.
const RETYPED: &str = "هذا نص عربي طويل يكتب في حقل ضيق ليلتف على عدة أسطر ويبقى مطابقا لما تكتبه الصفحة";

/// The ink of the rows of the editor's galley as screen `(left, right)` pairs: the row without the space at either end (the
/// break space a wrapped row keeps, `space` wide: the galley's glyphs of a right-to-left row are aligned to the characters, so
/// their advances are not measures of the space).
fn editor_ink(pos: Pos2, galley: &egui::Galley, space: f32) -> Vec<(f32, f32)> {
    galley
        .rows
        .iter()
        .map(|placed| {
            let glyphs = &placed.row.glyphs;
            let lead = if glyphs.first().is_some_and(|g| g.chr == ' ') { space } else { 0.0 };
            let trail = if glyphs.last().is_some_and(|g| g.chr == ' ') { space } else { 0.0 };
            (pos.x + placed.pos.x + lead, pos.x + placed.pos.x + placed.row.size.x - trail)
        })
        .collect()
}

/// The width of a space of the face, in screen pixels.
fn space_px(h: &Harness<'static, PdfCraftApp>, face: &[u8]) -> f32 {
    let page = h.state().views[0].page_screen_rect(0).expect("on screen");
    (pdfcraft_fonts::source_shaping::shape(face, " ").expect("shapes").advance * 12.0) as f32 * page.width() / 200.0
}

/// The screen `(left, right)` of the page's rows of its first paragraph.
fn page_ink(h: &Harness<'static, PdfCraftApp>) -> Vec<(f32, f32)> {
    let b = block(h);
    let lines = h.state().session.get(h.state().views[0].id).unwrap().text_lines(0);
    b.rows
        .iter()
        .map(|row| {
            let rect = row.iter().map(|i| lines[*i].rect).reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]).unwrap();
            (at(h, rect[0], rect[1]).x, at(h, rect[2], rect[1]).x)
        })
        .collect()
}

/// The galley the editor paints for a draft of Arabic rows (it holds a newline, or `needle`).
fn arabic_editor_galley(h: &Harness<'static, PdfCraftApp>, needle: char) -> (Pos2, std::sync::Arc<egui::Galley>) {
    painted_galleys(h)
        .into_iter()
        .find(|(_, g)| g.job.text.contains(needle) && g.rows.iter().any(|r| r.glyphs.len() > 3))
        .expect("the editor paints the draft")
}

/// A paragraph set in a complete Arabic face is edited in that face: the rows of the editor are as wide as the page's
/// (the face is not the interface's, whose letters are another width), whether the paragraph is as the page has it or retyped
/// and about to be applied, at every zoom.
#[test]
fn arabic_in_a_complete_page_face_is_drawn_in_that_face_so_the_rows_are_the_pages_width() {
    let Some(face) = static_face() else { return };
    for zoom in ["100", "250"] {
        let mut h = harness_with(zoom, face_page(&face, &[FIRST_ROW, SECOND_ROW]));
        let b = block(&h);
        assert_eq!(b.rows.len(), 2, "the page has two rows at {zoom}%");
        assert_eq!(b.editor_text, format!("{FIRST_ROW}\n{SECOND_ROW}"), "read back in typing order");
        let saved = h.state().session.get(h.state().views[0].id).unwrap().bytes.clone();
        open_editor(&mut h);
        // As the page has it: each row ends where the page's does and begins where it begins.
        let (pos, galley) = arabic_editor_galley(&h, '\n');
        let (ink, page) = (editor_ink(pos, &galley, space_px(&h, &face)), page_ink(&h));
        assert_eq!(ink.len(), page.len(), "a row to a row at {zoom}%");
        for (i, ((left, right), (page_left, page_right))) in ink.iter().zip(&page).enumerate() {
            assert!((right - page_right).abs() <= 1.0, "row {i} ends at {right}, the page's at {page_right} ({zoom}%)");
            assert!((left - page_left).abs() <= 1.0, "row {i} begins at {left}, the page's at {page_left} ({zoom}%)");
        }
        // Retyped, in other words of the same face: the page after Apply has the rows the editor showed.
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        let typed = RETYPED.to_owned();
        h.event(egui::Event::Text(typed.clone()));
        h.run_steps(4);
        let (pos, galley) = arabic_editor_galley(&h, '\u{647}');
        let shown = editor_ink(pos, &galley, space_px(&h, &face));
        assert!(shown.len() >= 2, "the retyped paragraph wraps at {zoom}%: {shown:?}");
        h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
        h.run_steps(4);
        assert_eq!(block(&h).editor_text.replace('\n', " "), typed, "applied in typing order at {zoom}%");
        let page = page_ink(&h);
        assert_eq!(page.len(), shown.len(), "as many rows as the editor showed at {zoom}%");
        for (i, ((left, right), (page_left, page_right))) in shown.iter().zip(&page).enumerate() {
            assert!((right - page_right).abs() <= 1.0, "row {i} ends at {right}, the page's at {page_right} ({zoom}%)");
            assert!((left - page_left).abs() <= 1.0, "row {i} begins at {left}, the page's at {page_left} ({zoom}%)");
        }
        assert_ne!(h.state().session.get(h.state().views[0].id).unwrap().bytes, saved, "something was applied at {zoom}%");
    }
}
