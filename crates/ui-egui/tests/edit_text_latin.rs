//! Edit text with Latin text in the real shell (egui_kittest): the editor breaks the lines of a paragraph where the page
//! will write them (the rows the rewrite that Apply runs lays the draft out in), in a font the page does not embed.

use egui::{Key, Modifiers, Pos2};
use egui_kittest::Harness;
use pdfcraft_ui_egui::PdfCraftApp;

/// A three-row Helvetica 12 pt paragraph at (20, 250) of a 220 × 300 pt page (the font is not embedded).
fn page() -> Vec<u8> {
    page_of(
        "BT /F1 12 Tf 14.4 TL 20 250 Td (The quick brown fox jumps over) Tj T* (the lazy dog while nine wizards) Tj T* (quickly box jumpers) Tj ET",
    )
}

/// The same paragraph with the end of its first row blue: two looks, which the styled rewrite writes.
fn two_looks_page() -> Vec<u8> {
    page_of(
        "BT /F1 12 Tf 14.4 TL 20 250 Td (The quick brown fox jumps ) Tj 0 0 1 rg (over) Tj 0 g T* (the lazy dog while nine wizards) Tj T* (quickly box jumpers) Tj ET",
    )
}

/// A page of 220 × 300 pt with `body` as its content (the font /F1 is Helvetica, not embedded).
fn page_of(body: &str) -> Vec<u8> {
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
    harness_of(zoom, page())
}

fn harness_of(zoom: &str, pdf: Vec<u8>) -> Harness<'static, PdfCraftApp> {
    let zoom = zoom.to_owned();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("doc.pdf", None, pdf.clone()).expect("fixture opens");
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

/// The screen `(left, right)` of the ink of each row the editor paints for the draft (without the spaces a wrapped row keeps).
fn editor_ink(h: &Harness<'static, PdfCraftApp>) -> Vec<(f32, f32)> {
    let galleys = painted_galleys(h);
    let (pos, g) = galleys.iter().find(|(_, g)| g.job.text.contains("quick")).expect("the editor paints the draft");
    g.rows
        .iter()
        .map(|row| {
            let ink: Vec<_> = row.glyphs.iter().filter(|g| g.chr != ' ').collect();
            let left = ink.iter().map(|g| g.pos.x).fold(f32::MAX, f32::min);
            let right = ink.iter().map(|g| g.pos.x + g.advance_width).fold(f32::MIN, f32::max);
            (pos.x + row.pos.x + left, pos.x + row.pos.x + right)
        })
        .collect()
}

/// The screen `(left, right)` of each row of the page's first paragraph, and the left edges of the words of each.
fn page_ink(h: &Harness<'static, PdfCraftApp>) -> Vec<((f32, f32), Vec<f32>)> {
    let b = block(h);
    let lines = h.state().session.get(h.state().views[0].id).unwrap().text_lines(0);
    b.rows
        .iter()
        .map(|row| {
            let rects: Vec<[f64; 4]> = row.iter().map(|i| lines[*i].rect).collect();
            let left = rects.iter().map(|r| r[0]).fold(f64::MAX, f64::min);
            let right = rects.iter().map(|r| r[2]).fold(f64::MIN, f64::max);
            let words = row.iter().flat_map(|i| lines[*i].ink.iter().map(|w| at(h, w[0], 0.0).x)).collect();
            ((at(h, left, 0.0).x, at(h, right, 0.0).x), words)
        })
        .collect()
}

/// Justify chosen in the Format panel for a retyped paragraph: the editor stretches the spaces of every row but the last to the
/// width the page writes the justified rows at, so each of those rows ends where the page's line does (and begins where it
/// begins), at every zoom. The rows are the rewrite's own; the last row stays as long as its words.
#[test]
fn a_justified_paragraph_is_shown_with_its_spaces_stretched_to_the_pages_rows() {
    for zoom in ["100", "200"] {
        let mut h = harness(zoom);
        open_editor(&mut h);
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.event(egui::Event::Text(LONG.into()));
        h.run_steps(4);
        let natural = editor_ink(&h);
        h.state_mut().views[0].line_editor.as_mut().expect("the editor is open").look.align = pdfcraft_engine::TextAlign::Justify;
        h.run_steps(4);
        let shown = editor_ink(&h);
        let planned = layout_rows(&h);
        assert_eq!(shown.len(), natural.len(), "the same rows when justified ({zoom}%)");
        assert!(shown.len() >= 4, "the paragraph wraps ({zoom}%): {planned:?}");
        // Before Apply, from the engine: every row but the last is as wide as the paragraph.
        h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
        h.run_steps(4);
        let page = page_ink(&h);
        assert_eq!(page.len(), shown.len(), "as many rows as the editor showed ({zoom}%)");
        for (i, ((left, right), ((page_left, page_right), _))) in shown.iter().zip(&page).enumerate().take(shown.len() - 1) {
            assert!((right - page_right).abs() <= 1.5, "row {i} ends at {right}, the page's line at {page_right} ({zoom}%)");
            assert!((left - page_left).abs() <= 1.5, "row {i} begins at {left}, the page's line at {page_left} ({zoom}%)");
            // (the stretch is what moved it: the row was shorter before)
            assert!(natural[i].1 < *right - 1.0 || (natural[i].1 - right).abs() <= 1.0, "row {i} was {natural:?}");
        }
        let (last_left, last_right) = shown[shown.len() - 1];
        let ((_, page_last_right), _) = page[page.len() - 1];
        assert!(
            (last_right - page_last_right).abs() <= 2.0,
            "the last row ends where its words do: {last_right} against {page_last_right} ({zoom}%)"
        );
        assert!(last_right - last_left < (shown[0].1 - shown[0].0) * 0.95, "and is not stretched ({zoom}%)");
    }
}

/// `rows` with the runs of a row joined by one space (`page_rows` puts one between runs the rewrite writes without).
fn spaced(rows: &[String]) -> Vec<String> {
    rows.iter().map(|row| row.split_whitespace().collect::<Vec<_>>().join(" ")).collect()
}

/// A retyped paragraph with Justify chosen in the Format panel, in the editor; `apply` also applies it.
fn justified_draft(zoom: &str, apply: bool) -> Harness<'static, PdfCraftApp> {
    let mut h = harness(zoom);
    open_editor(&mut h);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(egui::Event::Text(LONG.into()));
    h.run_steps(4);
    h.state_mut().views[0].line_editor.as_mut().expect("the editor is open").look.align = pdfcraft_engine::TextAlign::Justify;
    h.run_steps(4);
    if apply {
        h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
        h.run_steps(4);
    }
    h
}

/// Over stretched spaces the caret and clicks are still on the letters: a click on the first letter of a word of a justified row (the
/// second row, whose words the stretch has moved furthest from where the interface font puts them) puts the caret before it, and
/// typing goes there. The places clicked are those of the page's words after Apply.
#[test]
fn clicks_over_stretched_spaces_land_on_the_letters_they_are_on() {
    for zoom in ["100", "200"] {
        let applied = justified_draft(zoom, true);
        let page = page_ink(&applied);
        let rows = page_rows(&applied);
        let words: Vec<&str> = rows[1].split(' ').collect();
        let line = applied.state().session.get(applied.state().views[0].id).unwrap().text_lines(0);
        let b = block(&applied);
        let second = &line[b.rows[1][0]];
        let y = at(&applied, 0.0, (second.rect[1] + second.rect[3]) / 2.0).y;
        assert_eq!(page[1].1.len(), words.len(), "a box for each word of the second row ({zoom}%)");
        for w in [2, words.len() - 2, words.len() - 1] {
            let mut h = justified_draft(zoom, false);
            let click = Pos2::new(page[1].1[w] + 2.0, y);
            h.hover_at(click);
            h.run_steps(1);
            h.drag_at(click);
            h.run_steps(1);
            h.drop_at(click);
            h.run_steps(2);
            h.event(egui::Event::Text("X".into()));
            h.run_steps(3);
            let at_word: usize = rows[0].len() + 1 + words[..w].iter().map(|word| word.len() + 1).sum::<usize>();
            let expected = format!("{}X{}", &LONG[..at_word], &LONG[at_word..]);
            let typed = h.state().views[0].line_editor.as_ref().map(|ed| ed.text.clone());
            assert_eq!(typed, Some(expected), "word {w} of the second row ({zoom}%)");
        }
    }
}

/// Justify chosen for a retyped paragraph of two looks, which the styled rewrite writes (one look is the single-style
/// rewrite's): the editor's rows are stretched to the widths the page's lines have after Apply, as for one look, and a
/// line break ends a row that neither stretches. The paragraph is `first`, a line break and `second`.
fn justified_two_looks(zoom: &str, first: &str, second: Option<&str>) -> (Harness<'static, PdfCraftApp>, Vec<(f32, f32)>) {
    let mut h = harness_of(zoom, two_looks_page());
    open_editor(&mut h);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(egui::Event::Text(first.into()));
    if let Some(second) = second {
        h.key_press(Key::Enter);
        h.event(egui::Event::Text(second.into()));
    }
    h.run_steps(4);
    let natural = editor_ink(&h);
    h.state_mut().views[0].line_editor.as_mut().expect("the editor is open").look.align = pdfcraft_engine::TextAlign::Justify;
    h.run_steps(4);
    (h, natural)
}

#[test]
fn a_justified_paragraph_of_two_looks_is_shown_stretched_to_the_pages_rows_after_apply() {
    for zoom in ["100", "200"] {
        let (mut h, natural) = justified_two_looks(zoom, LONG, None);
        let shown = editor_ink(&h);
        assert_eq!(shown.len(), natural.len(), "the same rows when justified ({zoom}%)");
        assert!(shown.len() >= 4, "the paragraph wraps ({zoom}%): {:?}", layout_rows(&h));
        let planned = layout_rows(&h);
        h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
        h.run_steps(4);
        let page = page_ink(&h);
        assert_eq!(spaced(&planned), spaced(&page_rows(&h)), "the rewrite's rows are the page's ({zoom}%)");
        assert_eq!(page.len(), shown.len(), "as many rows as the editor showed ({zoom}%)");
        let lines = h.state().session.get(h.state().views[0].id).unwrap().text_lines(0);
        assert!(lines.iter().any(|l| l.color == [0.0, 0.0, 1.0]), "the blue look is kept: {lines:?}");
        // Every row but the last runs from the page's left edge of the paragraph to its right edge, in the editor and on the page.
        for (i, ((left, right), ((page_left, page_right), _))) in shown.iter().zip(&page).enumerate().take(shown.len() - 1) {
            assert!((right - page_right).abs() <= 1.5, "row {i} ends at {right}, the page's line at {page_right} ({zoom}%)");
            assert!((left - page_left).abs() <= 1.5, "row {i} begins at {left}, the page's line at {page_left} ({zoom}%)");
            assert!(natural[i].1 < *right - 1.0, "the stretch is what moved it: row {i} was {:?} ({zoom}%)", natural[i]);
        }
        let (last_left, last_right) = shown[shown.len() - 1];
        let ((_, page_last_right), _) = page[page.len() - 1];
        assert!((last_right - page_last_right).abs() <= 2.0, "the last row ends where its words do ({zoom}%)");
        assert!(last_right - last_left < (shown[0].1 - shown[0].0) * 0.95, "and is not stretched ({zoom}%)");
    }
}

#[test]
fn a_justified_row_before_a_line_break_is_shown_unstretched_like_the_page() {
    const FIRST: &str = "The quick brown fox jumps over the lazy dog while nine wizards quickly box jumpers";
    const SECOND: &str = "and wrap lines at the edge of this narrow paragraph so that it takes several more rows to end";
    let (mut h, _) = justified_two_looks("150", FIRST, Some(SECOND));
    let shown = editor_ink(&h);
    let planned = layout_rows(&h);
    let before_break = planned.iter().position(|row| row.ends_with("jumpers")).expect("the row before the break");
    assert!(before_break >= 2 && planned.len() > before_break + 2, "{planned:?}");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run_steps(4);
    let page = page_ink(&h);
    assert_eq!(spaced(&page_rows(&h)), spaced(&planned), "the rewrite's rows are the page's");
    assert_eq!(page.len(), shown.len());
    let full = page[0].0.1;
    for (i, ((left, right), ((page_left, page_right), _))) in shown.iter().zip(&page).enumerate() {
        let stretched = i != before_break && i + 1 != shown.len();
        // (The rows the paragraph had not have their letters fitted to the page's more loosely: the last is a row the source lacked.)
        let tolerance = if stretched {
            1.5
        } else if i == before_break {
            2.0
        } else {
            8.0
        };
        assert!((right - page_right).abs() <= tolerance, "row {i} ends at {right}, the page's line at {page_right}");
        assert!((left - page_left).abs() <= tolerance, "row {i} begins at {left}, the page's line at {page_left}");
        if stretched {
            assert!((page_right - full).abs() <= 1.0, "the page's row {i} fills the width");
        }
    }
    assert!(page[before_break].0.1 < full - 3.0, "the page leaves the row before the break short: {page:?}");
}
