//! Edit text with Latin text in the real shell (egui_kittest): the editor breaks the lines of a paragraph where the page
//! will write them (the rows the rewrite that Apply runs lays the draft out in), in a font the page does not embed.

use egui::{Key, Modifiers, Pos2};
use egui_kittest::Harness;
use pdfcraft_ui_egui::PdfCraftApp;

/// A three-row Helvetica 12 pt paragraph at (20, 250) of a 220 × 300 pt page (the font is not embedded).
fn page() -> Vec<u8> {
    let body =
        "BT /F1 12 Tf 14.4 TL 20 250 Td (The quick brown fox jumps over) Tj T* (the lazy dog while nine wizards) Tj T* (quickly box jumpers) Tj ET";
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [4 0 R] /Count 1 /MediaBox [0 0 220 300] >>".to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /Font << /F1 3 0 R >> >> >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len()),
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
    let zoom = zoom.to_owned();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("doc.pdf", None, page()).expect("fixture opens");
        app.set_option("zoom", &zoom).unwrap();
        app
    });
    h.run_steps(4);
    h
}

fn block(h: &Harness<'static, PdfCraftApp>) -> pdfcraft_engine::TextBlock {
    let s = h.state();
    s.session.get(s.views[0].id).unwrap().text_blocks(0).remove(0)
}

fn at(h: &Harness<'static, PdfCraftApp>, x: f64, y: f64) -> Pos2 {
    let r = h.state().views[0].page_screen_rect(0).expect("on screen");
    Pos2::new(r.left() + x as f32 / 220.0 * r.width(), r.top() + (300.0 - y as f32) / 300.0 * r.height())
}

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

/// The rows the editor paints for the draft, without the spaces a wrapped row keeps.
fn editor_rows(h: &Harness<'static, PdfCraftApp>) -> Vec<String> {
    let galleys = painted_galleys(h);
    let (_, g) = galleys.iter().find(|(_, g)| g.job.text.contains("quick")).expect("the editor paints the draft");
    g.rows.iter().map(|r| r.glyphs.iter().map(|g| g.chr).collect::<String>().trim().to_owned()).collect()
}

/// The rows of the page's first paragraph, as the text of the runs of each.
fn page_rows(h: &Harness<'static, PdfCraftApp>) -> Vec<String> {
    let b = block(h);
    let lines = h.state().session.get(h.state().views[0].id).unwrap().text_lines(0);
    b.rows.iter().map(|row| row.iter().map(|i| lines[*i].text.clone()).collect::<Vec<_>>().join(" ")).collect()
}

/// What Apply would write for the open draft, without applying it.
fn layout_rows(h: &Harness<'static, PdfCraftApp>) -> Vec<String> {
    let ed = h.state().views[0].line_editor.as_ref().expect("the editor is open");
    h.state().session.get(h.state().views[0].id).unwrap().text_edit_layout(0, 0, &ed.text, &ed.style()).expect("the draft is measured").rows
}

const LONG: &str =
    "The quick brown fox jumps over the lazy dog while nine wizards quickly box jumpers and wrap lines at the edge of this narrow paragraph";

/// The editor breaks a retyped Latin paragraph into the rows the page will have after Apply, at every zoom: the paragraph is set in
/// Helvetica, which the file does not embed, so the editor's own font (wider, with its spacing fitted to the source rows) breaks
/// it elsewhere.
#[test]
fn the_edit_text_box_breaks_latin_lines_where_apply_writes_them() {
    for zoom in ["100", "150", "200"] {
        let mut h = harness(zoom);
        open_editor(&mut h);
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.event(egui::Event::Text(LONG.into()));
        h.run_steps(4);
        let rows = editor_rows(&h);
        let planned = layout_rows(&h);
        h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
        h.run_steps(4);
        let saved = page_rows(&h);
        assert!(saved.len() >= 4, "the paragraph wraps ({zoom}%): {saved:?}");
        assert_eq!(rows, saved, "the editor's rows are the page's after Apply ({zoom}%)");
        assert_eq!(planned, saved, "and what the measuring rewrite says ({zoom}%)");
    }
}

/// Typing a word at a time keeps the editor's rows the rewrite's, and so does changing the size in the Format panel (which changes
/// the page's lines while the text stays the same).
#[test]
fn the_rows_follow_the_draft_as_it_is_typed_and_restyled() {
    let mut h = harness("150");
    open_editor(&mut h);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    let mut typed = String::new();
    for word in LONG.split(' ') {
        let piece = if typed.is_empty() { word.to_owned() } else { format!(" {word}") };
        typed.push_str(&piece);
        h.event(egui::Event::Text(piece));
        h.run_steps(3);
        if typed.contains("quick") {
            assert_eq!(editor_rows(&h), layout_rows(&h), "after typing {typed:?}");
        }
    }
    assert_eq!(h.state().views[0].line_editor.as_ref().map(|ed| ed.text.clone()), Some(LONG.to_owned()));
    // A bigger size: the same words break into other rows.
    let before = editor_rows(&h);
    h.state_mut().views[0].line_editor.as_mut().expect("the editor is open").look.size = 18.0;
    h.run_steps(4);
    let after = editor_rows(&h);
    assert_ne!(after, before, "the rows change with the size");
    assert_eq!(after, layout_rows(&h), "and are the rewrite's for it");
    assert_eq!(h.state().views[0].line_editor.as_ref().map(|ed| ed.text.clone()), Some(LONG.to_owned()), "the text is not touched");
}
