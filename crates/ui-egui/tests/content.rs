//! Edit a PDF ▸ Add content in the real shell (egui_kittest): type text onto a page, move it,
//! restyle it from the panel, add an image, delete with the keyboard.

use egui::{Pos2, pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::{PdfCraftApp, QuickTool};

fn harness() -> Harness<'static, PdfCraftApp> {
    harness_with_step_dt(0.25)
}

fn harness_with_step_dt(step_dt: f32) -> Harness<'static, PdfCraftApp> {
    harness_bytes_with_step_dt(include_bytes!("data/form.pdf").to_vec(), step_dt)
}

fn harness_bytes(bytes: Vec<u8>) -> Harness<'static, PdfCraftApp> {
    harness_bytes_with_step_dt(bytes, 0.25)
}

fn harness_bytes_with_step_dt(bytes: Vec<u8>, step_dt: f32) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_step_dt(step_dt).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("form.pdf", None, bytes.clone()).unwrap();
        app.set_option("zoom", "150").unwrap();
        app
    });
    for _ in 0..60 {
        h.run_steps(2);
        if !h.state().render_pending() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    h
}

fn at(h: &Harness<'static, PdfCraftApp>, x: f32, y: f32) -> Pos2 {
    let r = h.state().views[0].page_screen_rect(0).expect("on screen");
    let k = r.width() / 300.0;
    r.min + egui::vec2(x * k, y * k)
}

fn click(h: &mut Harness<'static, PdfCraftApp>, p: Pos2) {
    h.hover_at(p);
    h.run_steps(1);
    h.drag_at(p);
    h.run_steps(1);
    h.drop_at(p);
    h.run_steps(3);
}

fn added(h: &Harness<'static, PdfCraftApp>) -> Vec<pdfcraft_engine::Added> {
    let s = h.state();
    s.session.get(s.views[0].id).unwrap().added().to_vec()
}

/// A click whose press and release arrive in the same frame, as a quick real mouse click does.
fn quick_click(h: &mut Harness<'static, PdfCraftApp>, p: Pos2) {
    h.hover_at(p);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
    }
    h.run_steps(3);
}

fn texts(h: &Harness<'static, PdfCraftApp>) -> Vec<String> {
    added(h)
        .into_iter()
        .filter_map(|a| match a.content {
            pdfcraft_engine::AddedContent::Text(t) => Some(t.text),
            _ => None,
        })
        .collect()
}

#[test]
fn text_tool_drafts_grow_for_latin_and_arabic() {
    for text in ["Wide words ".repeat(8), "سلام عليكم ".repeat(8)] {
        let mut h = harness();
        assert!(h.state_mut().execute("edit.text"));
        h.run_steps(2);
        let point = at(&h, 40.0, 300.0);
        click(&mut h, point);
        let before = h.state().views[0].content.draft.as_ref().unwrap().rect;
        h.event(egui::Event::Text(text.clone()));
        h.run_steps(4);
        let draft = h.state().views[0].content.draft.as_ref().unwrap();
        assert!(draft.rect[2] - draft.rect[0] > before[2] - before[0] + 10.0, "the editor grows before wrapping: {}", text);
        assert!(draft.rect[2] <= 300.0 && draft.rect[1] < before[1], "the frame stays on the page and grows vertically");
        if let Ok(dir) = std::env::var("PDFCRAFT_SHOTS") {
            let language = if text.starts_with("Wide") { "latin" } else { "arabic" };
            h.render().unwrap().save(format!("{dir}/growing-{language}-text.png")).unwrap();
        }
        h.get_by_label("Done adding text").click();
        h.run_steps(3);
        if text.starts_with("سلام") && pdfcraft_fonts::document_arabic_font().is_none() {
            assert!(h.state().views[0].content.draft.is_some(), "without a document font the draft remains available");
            assert!(added(&h).is_empty());
            continue;
        }
        let pdfcraft_engine::AddedContent::Text(saved) = &added(&h)[0].content else { panic!() };
        assert_eq!(saved.text, text.trim_end());
        assert!(saved.rect[2] - saved.rect[0] > before[2] - before[0] + 10.0, "Save uses the grown width");
    }
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

/// Typed Arabic shows in the Add text box as it will be on the page: the words in display order, the
/// first word to the right of the second, while the draft keeps the typing order, and the saved text is
/// what was typed, with the page's copy of it (`/ActualText`) in typing order too.
#[test]
fn typed_arabic_in_the_add_text_box_shows_in_display_order_and_is_saved_in_typing_order() {
    if pdfcraft_fonts::document_arabic_font().is_none() {
        return;
    }
    let input = "سلام عليكم";
    for zoom in ["100", "150", "300"] {
        let mut h = harness();
        h.state_mut().set_option("zoom", zoom).unwrap();
        h.run_steps(4);
        assert!(h.state_mut().execute("edit.text"));
        h.run_steps(2);
        let point = at(&h, 40.0, 100.0);
        click(&mut h, point);
        h.event(egui::Event::Text(input.into()));
        h.run_steps(4);
        let draft = h.state().views[0].content.draft.as_ref().expect("the editor is open");
        assert_eq!(draft.text, input, "the draft keeps the typing order at {zoom}%");
        // What is painted: the second word first from the left, the letters of each word in the order
        // the shaper joins them, so the first word's glyphs lie to the right of the second word's.
        let galleys = painted_galleys(&h);
        let (_, galley) =
            galleys.iter().find(|(_, g)| g.job.text.contains('\u{633}')).unwrap_or_else(|| panic!("the box paints the Arabic text at {zoom}%"));
        assert_eq!(galley.job.text, "عليكم سلام", "the words are in display order at {zoom}%");
        let glyphs: Vec<(char, f32)> = galley.rows.iter().flat_map(|row| row.glyphs.iter().map(|g| (g.chr, g.pos.x))).collect();
        assert_eq!(glyphs.len(), "عليكم سلام".chars().count(), "a glyph for every character at {zoom}%");
        let (second, first) = glyphs.split_at("عليكم ".chars().count());
        let second_right = second.iter().map(|g| g.1).fold(f32::MIN, f32::max);
        let first_left = first.iter().map(|g| g.1).fold(f32::MAX, f32::min);
        assert!(first_left > second_right, "the first word is right of the second at {zoom}%: {first_left} against {second_right}");
        if let Ok(dir) = std::env::var("PDFCRAFT_SHOTS") {
            h.render().unwrap().save(format!("{dir}/add-text-arabic-{zoom}.png")).unwrap();
        }
        h.get_by_label("Done adding text").click();
        h.run_steps(3);
        assert_eq!(texts(&h), [input], "the text is saved as typed at {zoom}%");
        let actual: String = input.encode_utf16().map(|u| format!("{u:04X}")).collect();
        let marked = saved_streams_with(&h, &format!("/ActualText <FEFF{actual}>"));
        assert!(!marked.is_empty(), "the page keeps the typing order for copy and search at {zoom}%");
    }
}

/// The rows of the Add text box sit where the text's alignment puts them, in the box as on the page:
/// Arabic with the default alignment (left) starts at the box's left edge, with Align right it ends at
/// the right edge; Latin text follows the alignment too.
#[test]
fn rows_of_the_add_text_box_follow_the_texts_alignment() {
    // The text typed, and the text the box paints (the words of Arabic in display order).
    for (text, painted, needs_arabic) in [("سلام عليكم", "عليكم سلام", true), ("Hello there", "Hello there", false)] {
        if needs_arabic && pdfcraft_fonts::document_arabic_font().is_none() {
            continue;
        }
        for align in [pdfcraft_engine::TextAlign::Left, pdfcraft_engine::TextAlign::Center, pdfcraft_engine::TextAlign::Right] {
            let mut h = harness();
            assert!(h.state_mut().execute("edit.text"));
            h.run_steps(2);
            let point = at(&h, 40.0, 100.0);
            click(&mut h, point);
            h.state_mut().views[0].content.draft.as_mut().expect("the editor is open").style.align = align;
            h.event(egui::Event::Text(text.into()));
            h.run_steps(4);
            let galleys = painted_galleys(&h);
            let (pos, galley) =
                galleys.iter().find(|(_, g)| g.job.text == painted).unwrap_or_else(|| panic!("the box paints {text:?} as {painted:?}"));
            let row = &galley.rows[0];
            let (left, right) = (pos.x + row.pos.x, pos.x + row.pos.x + row.row.size.x);
            let (area_left, area_right) = (pos.x, pos.x + galley.rect.width());
            let what = format!("{text:?} aligned {align:?}: the row is {left}..{right} in {area_left}..{area_right}");
            match align {
                pdfcraft_engine::TextAlign::Left => assert!((left - area_left).abs() <= 1.0, "{what}"),
                pdfcraft_engine::TextAlign::Right => assert!((right - area_right).abs() <= 1.0, "{what}"),
                _ => assert!(((left - area_left) - (area_right - right)).abs() <= 1.5, "{what}"),
            }
            assert!(right > left && right <= area_right + 1.0 && left >= area_left - 1.0, "{what}");
        }
    }
}

/// The rows the Add text box paints for `probe` (a word of the text), without the spaces a wrapped row keeps at its end:
/// one string per row, as the galley holds the characters (Arabic words keep the letters in typing order).
fn editor_rows(h: &Harness<'static, PdfCraftApp>, probe: &str) -> Vec<String> {
    let galleys = painted_galleys(h);
    let (_, galley) = galleys.iter().find(|(_, g)| g.job.text.contains(probe)).unwrap_or_else(|| panic!("the box paints {probe:?}"));
    galley.rows.iter().map(|row| row.glyphs.iter().map(|g| g.chr).collect::<String>().trim().to_owned()).collect()
}

/// The lines of the saved document's page that are not among `before` (the text lines the page had), top to bottom.
fn new_page_lines(h: &Harness<'static, PdfCraftApp>, before: &[String]) -> Vec<String> {
    let mut left = before.to_vec();
    let state = h.state();
    let mut lines = state.session.get(state.views[0].id).unwrap().text_lines(0);
    lines.sort_by(|a, b| b.rect[3].total_cmp(&a.rect[3]));
    lines
        .into_iter()
        .map(|l| l.text)
        .filter(|text| match left.iter().position(|old| old == text) {
            Some(i) => {
                left.remove(i);
                false
            }
            None => true,
        })
        .collect()
}

/// The Add text box breaks its lines where the page it is saved to does, whichever font it is typed in:
/// Latin is measured in the standard font the box is written in (Helvetica) and Arabic by the shaped face, not
/// by the interface font, and the box's width is the field's inner width (no margin). Typed near the page's
/// right edge the box is narrow and the text wraps; the rows of the editor are the lines of the saved page.
#[test]
fn the_add_text_box_wraps_where_the_saved_page_does() {
    let latin = "The quick brown fox jumps over the lazy dog while nine wizards quickly box jumpers and wrap lines at the edge";
    let arabic = "سلام عليكم ورحمة الله وبركاته مع أطيب التحيات من الجميع هنا اليوم وفي كل مكان";
    // Arabic with Latin words and digits in it: its lines hold the same words, in the order the bidirectional algorithm gives.
    let mixed = "سلام PDF عليكم ورحمة الله Acrobat وبركاته 2026 مع أطيب التحيات من الجميع هنا Reader اليوم وفي كل مكان";
    for (input, probe, needs_arabic) in [(latin, "quick", false), (arabic, "\u{633}\u{644}\u{627}\u{645}", true), (mixed, "Acrobat", true)] {
        if needs_arabic && pdfcraft_fonts::document_arabic_font().is_none() {
            continue;
        }
        // Two widths of box: the page's right edge leaves the text 100 and then 160 points.
        for (zoom, x) in [("150", 200.0), ("150", 140.0), ("100", 200.0), ("200", 140.0)] {
            let mut h = harness();
            h.state_mut().set_option("zoom", zoom).unwrap();
            h.run_steps(4);
            let before: Vec<String> = h.state().session.get(h.state().views[0].id).unwrap().text_lines(0).into_iter().map(|l| l.text).collect();
            assert!(h.state_mut().execute("edit.text"));
            h.run_steps(2);
            let point = at(&h, x, 100.0);
            click(&mut h, point);
            h.event(egui::Event::Text(input.into()));
            h.run_steps(4);
            let draft = h.state().views[0].content.draft.as_ref().expect("the editor is open");
            let box_width = draft.rect[2] - draft.rect[0];
            assert!(box_width <= 300.0 - x as f64 + 1.0, "the box stops at the page's edge: {box_width}");
            let rows = editor_rows(&h, probe);
            h.get_by_label("Done adding text").click();
            h.run_steps(3);
            assert_eq!(texts(&h), [input]);
            let saved = new_page_lines(&h, &before);
            let what = format!("zoom {zoom}, box {box_width:.1} pt wide, at x {x}");
            assert!(saved.len() >= 3, "the text wraps over several lines ({what}): {saved:?}");
            assert_eq!(rows.len(), saved.len(), "the editor has the saved lines ({what}): {rows:?} against {saved:?}");
            for (i, (row, line)) in rows.iter().zip(&saved).enumerate() {
                // The glyphs of a right-to-left row are in display order: all its characters are the saved line's, last to first.
                if input == mixed {
                    // The same words on the row, whatever order their letters are in.
                    let words = |text: &str| {
                        let mut words: Vec<Vec<char>> = text
                            .split(' ')
                            .map(|w| {
                                let mut chars: Vec<char> = w.chars().collect();
                                chars.sort_unstable();
                                chars
                            })
                            .collect();
                        words.sort();
                        words
                    };
                    assert_eq!(words(row), words(line), "row {i} ({what})");
                    continue;
                }
                let read: String = if needs_arabic { row.chars().rev().collect() } else { row.clone() };
                assert_eq!(&read, line, "row {i} ({what})");
            }
        }
    }
}

/// The painted rows of the Add text box for `probe`: for each, where its ink begins and ends on screen (the spaces a wrapped row keeps are
/// no ink) and where its top is.
fn editor_row_extents(h: &Harness<'static, PdfCraftApp>, probe: &str) -> Vec<(f32, f32, f32)> {
    let galleys = painted_galleys(h);
    let (pos, galley) = galleys.iter().find(|(_, g)| g.job.text.contains(probe)).unwrap_or_else(|| panic!("the box paints {probe:?}"));
    galley
        .rows
        .iter()
        .map(|row| {
            let ink: Vec<_> = row.glyphs.iter().filter(|g| g.chr != ' ').collect();
            let left = ink.iter().map(|g| g.pos.x).fold(f32::MAX, f32::min);
            let right = ink.iter().map(|g| g.pos.x + g.advance_width).fold(f32::MIN, f32::max);
            (pos.x + row.pos.x + left, pos.x + row.pos.x + right, pos.y + row.pos.y)
        })
        .collect()
}

/// The rows of a wrapped Latin Add text box sit where the saved page writes its lines, at every alignment (a justified text with its
/// spaces stretched): each row's ink begins and ends
/// where the page line's does (the interface font is fitted to the standard font's width) and the rows are as far apart as the page's lines.
#[test]
fn the_rows_of_the_add_text_box_sit_where_the_page_writes_them() {
    let input = "The quick brown fox jumps over the lazy dog while nine wizards quickly box jumpers and wrap lines at the edge";
    use pdfcraft_engine::TextAlign::{Center, Justify, Left, Right};
    // The interface font's rows are 1.21 em apart, the page's 1.2: a large size shows the pitch.
    let cases = [("100", Left, 12.0), ("100", Center, 12.0), ("100", Right, 12.0), ("150", Left, 12.0), ("150", Center, 12.0), ("150", Right, 12.0)]
        .into_iter()
        .chain([("200", Left, 12.0), ("200", Center, 12.0), ("200", Right, 12.0), ("150", Left, 72.0), ("100", Right, 36.0)])
        // Justified: every row but the last is stretched to the box's width, as the page writes it.
        .chain([("100", Justify, 12.0), ("150", Justify, 12.0), ("200", Justify, 12.0), ("150", Justify, 36.0)]);
    for (zoom, align, size) in cases {
        {
            let mut h = harness();
            h.state_mut().set_option("zoom", zoom).unwrap();
            h.run_steps(4);
            let before: Vec<String> = h.state().session.get(h.state().views[0].id).unwrap().text_lines(0).into_iter().map(|l| l.text).collect();
            assert!(h.state_mut().execute("edit.text"));
            h.run_steps(2);
            let point = at(&h, 140.0, 100.0);
            click(&mut h, point);
            let style = &mut h.state_mut().views[0].content.draft.as_mut().expect("the editor is open").style;
            (style.align, style.size) = (align, size);
            h.event(egui::Event::Text(input.into()));
            h.run_steps(4);
            let rows = editor_row_extents(&h, "quick");
            h.get_by_label("Done adding text").click();
            h.run_steps(3);
            let state = h.state();
            let mut lines = state.session.get(state.views[0].id).unwrap().text_lines(0);
            lines.retain(|l| !before.contains(&l.text));
            lines.sort_by(|a, b| b.rect[3].total_cmp(&a.rect[3]));
            let what = format!("zoom {zoom}, {align:?}, {size} pt");
            assert_eq!(rows.len(), lines.len(), "{what}");
            assert!(lines.len() >= 3, "{what}");
            // The pitch adds up over the rows: the first and the last row are as far apart as the page's first and last lines.
            if let (Some((_, _, first)), Some((_, _, last)), Some(top), Some(bottom)) = (rows.first(), rows.last(), lines.first(), lines.last()) {
                let page_span = (at(&h, 0.0, top.rect[3] as f32).y - at(&h, 0.0, bottom.rect[3] as f32).y).abs();
                // (half a pixel a row for the whole-pixel heights egui gives rows)
                let slack = 1.5 + 0.5 * rows.len() as f32;
                assert!(((last - first) - page_span).abs() <= slack, "the rows span {} against the page's {page_span} ({what})", last - first);
            }
            for (i, ((left, right, top), line)) in rows.iter().zip(&lines).enumerate() {
                let (page_left, page_right) = (at(&h, line.rect[0] as f32, 0.0).x, at(&h, line.rect[2] as f32, 0.0).x);
                assert!((left - page_left).abs() <= 2.0, "row {i} begins at {left}, the page's line at {page_left} ({what})");
                assert!((right - page_right).abs() <= 2.0, "row {i} ends at {right}, the page's line at {page_right} ({what})");
                if let (Some((_, _, next)), Some(below)) = (rows.get(i + 1), lines.get(i + 1)) {
                    let page_pitch = (at(&h, 0.0, line.rect[3] as f32).y - at(&h, 0.0, below.rect[3] as f32).y).abs();
                    assert!(
                        (next - top - page_pitch).abs() <= 1.0,
                        "rows {i} and {} are {} apart, the page's lines {page_pitch} ({what})",
                        i + 1,
                        next - top
                    );
                }
            }
        }
    }
}

#[test]
fn text_tool_single_click_retypes_added_text_without_another_item() {
    for command in ["edit.text", "edit.edit_text"] {
        for (original, replacement) in [("Original", "Replacement"), ("سلام", "سلام عليكم ورحمة الله")] {
            if original == "سلام" && pdfcraft_fonts::document_arabic_font().is_none() {
                continue;
            }
            let mut h = harness();
            assert!(h.state_mut().apply_edit(pdfcraft_engine::Edit::AddText {
                page: 0,
                text: pdfcraft_engine::AddedText { rect: [40.0, 195.0, 140.0, 210.0], text: original.into(), ..Default::default() },
            }));
            assert!(h.state_mut().execute(command));
            h.run_steps(3);
            let point = at(&h, 45.0, 195.0);
            click(&mut h, point);
            let draft = h.state().views[0].content.draft.as_ref().expect("one click edits the existing item");
            assert_eq!(draft.index, Some(0));
            assert_eq!(draft.text, original);
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
            h.event(egui::Event::Text(replacement.into()));
            h.run_steps(3);
            h.get_by_label("Done adding text").click();
            h.run_steps(3);
            assert_eq!(texts(&h), [replacement]);
            let state = h.state();
            let bytes = state.session.save_bytes(state.views[0].id).unwrap();
            let mut reopened = pdfcraft_engine::Session::new();
            let id = reopened.open("again.pdf", None, bytes, None).unwrap();
            assert_eq!(reopened.get(id).unwrap().added().len(), 1);
            let pdfcraft_engine::AddedContent::Text(text) = &reopened.get(id).unwrap().added()[0].content else { panic!() };
            assert_eq!(text.text, replacement);
        }
    }
}

#[test]
fn text_tool_deleting_a_draft_hides_the_old_added_glyphs_and_cancel_restores_them() {
    let mut h = harness();
    assert!(h.state_mut().apply_edit(pdfcraft_engine::Edit::AddText {
        page: 0,
        text: pdfcraft_engine::AddedText { rect: [40.0, 195.0, 240.0, 220.0], text: "Original glyphs".into(), size: 18.0, ..Default::default() },
    }));
    let bytes = h.state().session.get(h.state().views[0].id).unwrap().bytes.clone();
    assert!(h.state_mut().execute("edit.edit_text"));
    h.run_steps(3);
    let point = at(&h, 45.0, 185.0);
    click(&mut h, point);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.key_press(egui::Key::Backspace);
    for _ in 0..100 {
        h.run_steps(2);
        if !h.state().render_pending() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(h.state().views[0].content.draft.as_ref().unwrap().text.is_empty());
    let shot = h.render().unwrap();
    for x in 105..140 {
        for y in 185..197 {
            let pixel = at(&h, x as f32, y as f32);
            let [r, g, b, a] = shot.get_pixel(pixel.x as u32, pixel.y as u32).0;
            assert!(r > 200 && g > 200 && b > 200 && a == 255, "old glyph ink remains at {x},{y}: {r},{g},{b},{a}");
        }
    }
    if let Ok(dir) = std::env::var("PDFCRAFT_SHOTS") {
        shot.save(format!("{dir}/cleared-added-text.png")).unwrap();
    }
    assert_eq!(h.state().session.get(h.state().views[0].id).unwrap().bytes, bytes, "previewing does not edit the document");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(texts(&h), ["Original glyphs"]);
    assert!(h.state().views[0].content.draft.is_none());
}

#[test]
fn clicking_elsewhere_keeps_the_text_and_starts_a_new_box() {
    // #74: a click elsewhere on the page threw away the text being typed and opened a new box.
    let mut h = harness();
    assert!(h.state_mut().execute("edit.text"));
    h.run_steps(2);
    let p = at(&h, 40.0, 100.0);
    click(&mut h, p);
    h.event(egui::Event::Text("First".into()));
    h.run_steps(2);
    // A slow click (press and release in different frames)…
    let p = at(&h, 40.0, 200.0);
    click(&mut h, p);
    assert_eq!(texts(&h), ["First"]);
    assert!(h.state().views[0].content.draft.is_some(), "a new box opened");
    assert_eq!(h.state().quick_tool, QuickTool::AddText, "still adding text");
    h.event(egui::Event::Text("Second".into()));
    h.run_steps(2);
    // …and a quick one (both in one frame, as a real mouse click usually arrives).
    let p = at(&h, 40.0, 300.0);
    quick_click(&mut h, p);
    assert_eq!(texts(&h), ["First", "Second"]);
    // Discard throws away only the box being typed.
    h.event(egui::Event::Text("Third".into()));
    h.run_steps(2);
    h.get_by_label("Discard this text (Esc)").click();
    h.run_steps(3);
    assert_eq!(texts(&h), ["First", "Second"]);
    assert!(h.state().views[0].content.draft.is_none());
}

#[test]
fn typing_moving_styling_and_deleting_added_content() {
    let mut h = harness();
    assert!(h.state_mut().execute("edit.text"));
    h.run_steps(2);
    h.get_by_label("Edit a PDF");
    let p = at(&h, 40.0, 300.0);
    click(&mut h, p);
    assert!(h.state().views[0].content.draft.is_some(), "the editor opened");
    h.event(egui::Event::Text("Reviewed".into()));
    h.run_steps(2);
    // Done keeps the text and goes back to Select, with the new text selected.
    h.get_by_label("Done adding text").click();
    h.run_steps(3);
    let a = added(&h);
    assert_eq!(a.len(), 1);
    let pdfcraft_engine::AddedContent::Text(t) = &a[0].content else { panic!() };
    assert_eq!(t.text, "Reviewed");
    assert_eq!(h.state().quick_tool, QuickTool::Select);
    h.run_steps(2);
    assert_eq!(h.state().views[0].content.selected, Some((0, 0)), "the new text is selected");
    // Bold from the Format panel: one undoable change.
    h.get_by_label("B").click();
    h.run_steps(3);

    let pdfcraft_engine::AddedContent::Text(t) = &added(&h)[0].content else { panic!() };
    assert!(t.bold);
    {
        let s = h.state();
        assert_eq!(s.session.get(s.views[0].id).unwrap().can_undo(), Some("Edit content"));
    }
    // Delete.
    h.key_press(egui::Key::Delete);
    h.run_steps(3);
    assert!(added(&h).is_empty());
    // An image lands in the middle of the page, selected.
    let mut png = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png, 30, 20);
        enc.set_color(png::ColorType::Rgb);
        let mut w = enc.write_header().unwrap();
        w.write_image_data(&[90u8; 1800]).unwrap();
    }
    h.state_mut().add_image("logo.png".into(), png);
    h.run_steps(3);
    let a = added(&h);
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].content.rect(), [135.0, 190.0, 165.0, 210.0]);
    assert_eq!(h.state().views[0].content.selected, Some((0, 0)));
    // Edit image: rotate clockwise turns the box around its centre.
    h.get_by_label("Rotate clockwise").click();
    h.run_steps(3);
    let a = added(&h);
    let pdfcraft_engine::AddedContent::Image(img) = &a[0].content else { panic!() };
    assert_eq!((img.rotation, img.rect), (3, [140.0, 185.0, 160.0, 215.0]));
    h.get_by_label("Flip horizontal").click();
    h.run_steps(3);
    let pdfcraft_engine::AddedContent::Image(img) = &added(&h)[0].content else { panic!() };
    assert!(img.flip_h);
}

/// #125: text the standard fonts can't draw stays in the editor with a warning instead of being
/// written to the page as `?` (or lost).
#[test]
fn text_the_standard_fonts_cant_draw_stays_in_the_editor() {
    let mut h = harness();
    assert!(h.state_mut().execute("edit.text"));
    h.run_steps(2);
    let p = at(&h, 40.0, 300.0);
    click(&mut h, p);
    h.event(egui::Event::Text("Hello 世界".into()));
    h.run_steps(2);
    h.get_by_label_contains("can't draw “世” (U+4E16)");
    if let Ok(dir) = std::env::var("PDFCRAFT_SHOTS") {
        h.render().unwrap().save(format!("{dir}/undrawable-text.png")).unwrap();
    }
    h.get_by_label("Done adding text").click();
    h.run_steps(3);
    assert!(texts(&h).is_empty(), "nothing was written to the page");
    let draft = h.state().views[0].content.draft.as_ref().map(|d| d.text.clone());
    assert_eq!(draft.as_deref(), Some("Hello 世界"), "the typed text is kept");
    // A click elsewhere on the page doesn't finish it, or replace it with a new box.
    let elsewhere = at(&h, 40.0, 150.0);
    click(&mut h, elsewhere);
    let draft = h.state().views[0].content.draft.as_ref().map(|d| d.text.clone());
    assert_eq!(draft.as_deref(), Some("Hello 世界"), "clicking elsewhere keeps the typed text");
    assert!(texts(&h).is_empty());
    // Once the text can be drawn, Done adds it (Done took the focus, so edit the draft directly).
    if let Some(d) = h.state_mut().views[0].content.draft.as_mut() {
        d.text = "Hello world".into();
    }
    h.run_steps(2);
    h.get_by_label("Done adding text").click();
    h.run_steps(3);
    assert_eq!(texts(&h), ["Hello world"]);
}

/// Text kept because it can't be drawn goes with its page when the page is deleted, so nothing
/// is left blocking Save out of sight.
#[test]
fn a_kept_draft_goes_with_its_deleted_page() {
    let mut h = harness();
    assert!(h.state_mut().apply_edit(pdfcraft_engine::Edit::InsertBlankPage { at: 1, width: 612.0, height: 792.0 }));
    h.run_steps(2);
    assert!(h.state_mut().execute("edit.text"));
    h.state_mut().views[0].go_to_page(1);
    h.run_steps(4);
    let p = h.state().views[0].page_screen_rect(1).expect("page 2 on screen").min + egui::vec2(40.0, 40.0);
    click(&mut h, p);
    h.event(egui::Event::Text("日本語".into()));
    h.run_steps(2);
    assert_eq!(h.state().views[0].content.draft.as_ref().map(|d| (d.page, d.text.as_str())), Some((1, "日本語")));
    assert!(h.state_mut().apply_edit(pdfcraft_engine::Edit::DeletePages { pages: vec![1] }));
    h.run_steps(2);
    assert!(h.state().views[0].content.draft.is_none());
}

#[test]
fn typing_thai_text_keeps_marks_and_commits_to_page() {
    if pdfcraft_fonts::document_thai_font().is_none() {
        return;
    }
    let fonts = pdfcraft_ui_egui::theme::font_definitions();
    let face = pdfcraft_fonts::document_thai_font().unwrap();
    assert!(fonts.families[&egui::FontFamily::Proportional].contains(&face.name()));
    let mut h = harness();
    assert!(h.state_mut().execute("edit.text"));
    h.run_steps(2);
    let p = at(&h, 40.0, 100.0);
    click(&mut h, p);
    h.event(egui::Event::Text("น้ำ กิ่ ผู้ใช้ Hello".into()));
    h.run_steps(3);
    let p = at(&h, 40.0, 200.0);
    click(&mut h, p);
    assert_eq!(texts(&h), ["น้ำ กิ่ ผู้ใช้ Hello"]);
}

#[test]
fn finished_text_over_form_field_keeps_selection_and_drag_ownership() {
    let mut h = harness();
    assert!(h.state_mut().execute("edit.text"));
    h.run_steps(2);
    let p = at(&h, 60.0, 202.0);
    click(&mut h, p);
    h.event(egui::Event::Text("Reviewed".into()));
    h.run_steps(2);
    h.get_by_label("Done adding text").click();
    h.run_steps(3);
    assert_eq!(texts(&h), ["Reviewed"]);
    let rect = added(&h)[0].content.rect();
    let mid_x = ((rect[0] + rect[2]) / 2.0) as f32;
    let mid_y = ((rect[1] + rect[3]) / 2.0) as f32;
    assert!((20.0..280.0).contains(&mid_x) && (180.0..200.0).contains(&mid_y), "fixture text must overlap city: {rect:?}");
    let center = at(&h, mid_x, 400.0 - mid_y);
    click(&mut h, center);
    assert!(h.state().views[0].forms.focus.is_none(), "selecting finished text must not focus the field underneath");
    assert_eq!(h.state().views[0].content.selected, Some((0, 0)));
    h.run_steps(60);
    let start = at(&h, mid_x, 400.0 - mid_y);
    let end = at(&h, mid_x, 400.0 - mid_y - 80.0);
    h.drag_at(start);
    h.run_steps(1);
    for step in 1..=40 {
        h.hover_at(start + (end - start) * (step as f32 / 40.0));
        h.run_steps(1);
    }
    h.drop_at(end);
    h.run_steps(4);
    let moved = added(&h)[0].content.rect();
    for (actual, expected) in moved.into_iter().zip([rect[0], rect[1] + 80.0, rect[2], rect[3] + 80.0]) {
        assert!((actual - expected).abs() < 1.0, "text should move upward: {rect:?} -> {moved:?}");
    }
    assert!(h.state().views[0].forms.focus.is_none());
    let bare = at(&h, 270.0, 210.0);
    click(&mut h, bare);
    assert_eq!(h.state().views[0].forms.focus.as_ref().map(|f| f.name.as_str()), Some("city"));
}

fn text_over_city() -> Harness<'static, PdfCraftApp> {
    let mut h = harness_with_step_dt(1.0 / 60.0);
    assert!(h.state_mut().execute("edit.text"));
    h.run_steps(2);
    let p = at(&h, 60.0, 202.0);
    click(&mut h, p);
    h.event(egui::Event::Text("Reviewed".into()));
    h.run_steps(2);
    h.get_by_label("Done adding text").click();
    h.run_steps(3);
    h
}

#[test]
fn finished_text_over_field_keeps_move_and_resize_cursors() {
    let mut h = text_over_city();
    let rect = added(&h)[0].content.rect();
    let center = at(&h, ((rect[0] + rect[2]) / 2.0) as f32, (400.0 - (rect[1] + rect[3]) / 2.0) as f32);
    h.hover_at(center);
    h.run_steps(2);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Move);
    // The upper-right handle overlaps the city field; it retains its resize gesture.
    let corner = at(&h, rect[2] as f32, (400.0 - rect[3]) as f32);
    h.hover_at(corner);
    h.run_steps(2);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::ResizeNeSw);
    let end = corner + egui::vec2(30.0, -20.0);
    h.drag_at(corner);
    h.run_steps(1);
    for step in 1..=20 {
        h.hover_at(corner + (end - corner) * (step as f32 / 20.0));
        h.run_steps(1);
    }
    h.drop_at(end);
    h.run_steps(4);
    let resized = added(&h)[0].content.rect();
    assert!(resized[2] > rect[2] + 1.0 && resized[3] > rect[3] + 1.0, "resize must retain ownership: {rect:?} -> {resized:?}");
    assert!(h.state().views[0].forms.focus.is_none());
}

#[test]
fn finished_text_over_field_still_opens_its_editor() {
    let mut h = text_over_city();
    let rect = added(&h)[0].content.rect();
    let center = at(&h, ((rect[0] + rect[2]) / 2.0) as f32, (400.0 - (rect[1] + rect[3]) / 2.0) as f32);
    h.run_steps(60);
    h.hover_at(center);
    h.run_steps(1);
    for _ in 0..2 {
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: center, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        }
        h.run_steps(1);
    }
    h.run_steps(2);
    assert!(h.state().views[0].content.draft.as_ref().is_some_and(|d| d.index == Some(0)), "double click edits the added text");
    assert!(h.state().views[0].forms.focus.is_none());
}

#[test]
fn closing_edit_panel_leaves_form_filling_available() {
    let mut h = text_over_city();
    h.state_mut().set_option("left", "closed").unwrap();
    h.run_steps(3);
    let p = at(&h, 100.0, 210.0);
    click(&mut h, p);
    assert_eq!(h.state().views[0].forms.focus.as_ref().map(|f| f.name.as_str()), Some("city"));
}

/// The screen rectangle of an added item's display-space box (y up, from the page's bottom).
fn item_on_screen(h: &Harness<'static, PdfCraftApp>, r: [f64; 4]) -> egui::Rect {
    let s = h.state();
    let ph = s.session.get(s.views[0].id).unwrap().info.pages[0].height;
    let page = s.views[0].page_screen_rect(0).expect("on screen");
    let k = page.width() / 300.0;
    let p = |x: f64, y: f64| page.min + egui::vec2(x as f32 * k, (ph - y as f32) * k);
    egui::Rect::from_two_pos(p(r[0], r[1]), p(r[2], r[3]))
}

/// Dark pixels in `r`: glyph ink (the selection outline is blue, the page white).
fn ink(pixels: &image::RgbaImage, r: egui::Rect) -> usize {
    let (x0, y0, x1, y1) = (r.left().max(0.0) as u32, r.top().max(0.0) as u32, r.right() as u32, r.bottom() as u32);
    (y0..y1.min(pixels.height()))
        .flat_map(|y| (x0..x1.min(pixels.width())).map(move |x| (x, y)))
        .filter(|(x, y)| pixels.get_pixel(*x, *y).0[..3].iter().all(|v| *v < 90))
        .count()
}

/// Only the left part of the glyphs: kittest paints its cursor down and right of the pointer.
fn probe(r: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_size(r.min, egui::vec2(r.width() * 0.6, r.height()))
}

/// Ink in the probes of the text's first and dragged places.
fn ink_at(h: &mut Harness<'static, PdfCraftApp>, from: egui::Rect, to: egui::Rect) -> (usize, usize) {
    let pixels = h.render().unwrap();
    (ink(&pixels, probe(from)), ink(&pixels, probe(to)))
}

/// Types text, finishes it, and drags it up without releasing, until the drag preview shows.
/// Returns the text's box, its screen rectangles before and after, and the pointer.
fn drag_added_text(h: &mut Harness<'static, PdfCraftApp>) -> ([f64; 4], egui::Rect, egui::Rect, Pos2) {
    assert!(h.state_mut().execute("edit.text"));
    h.run_steps(2);
    let p = at(h, 40.0, 300.0);
    click(h, p);
    h.event(egui::Event::Text("MMMMMMMM".into()));
    h.run_steps(2);
    h.get_by_label("Done adding text").click();
    h.run_steps(3);
    let rect = added(h)[0].content.rect();
    let from = item_on_screen(h, rect);
    let mut shown = 0;
    for _ in 0..100 {
        h.run_steps(2);
        shown = ink(&h.render().unwrap(), probe(from));
        if shown > 20 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(shown > 20, "the added text is on the page ({shown} dark pixels)");
    let grab = pos2(from.right() - 12.0, from.center().y);
    let shift = egui::vec2(0.0, -150.0);
    h.hover_at(grab);
    h.run_steps(1);
    h.drag_at(grab);
    h.run_steps(1);
    h.hover_at(grab + shift);
    let to = from.translate(shift);
    for _ in 0..100 {
        h.run_steps(2);
        let (old, new) = ink_at(h, from, to);
        if old == 0 && new > 20 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    (rect, from, to, grab + shift)
}

/// #1488: while a finished text box is dragged, its text follows the box instead of staying put
/// until the button is released.
#[test]
fn dragged_text_follows_the_pointer_before_release() {
    let mut h = harness();
    let (rect, from, to, end) = drag_added_text(&mut h);
    if let Ok(dir) = std::env::var("PDFCRAFT_SHOTS") {
        h.render().unwrap().save(format!("{dir}/added-text-drag.png")).unwrap();
    }
    let (old, new) = ink_at(&mut h, from, to);
    assert!(new > 20, "the text is drawn where the box was dragged ({new} dark pixels)");
    assert_eq!(old, 0, "and no longer where it was");
    assert_eq!(added(&h)[0].content.rect(), rect, "nothing is applied before release");
    h.drop_at(end);
    h.run_steps(3);
    assert_ne!(added(&h)[0].content.rect(), rect, "releasing moves it");
}

/// After release the text stays in its new place while the page is re-rendered with the move,
/// instead of jumping back to where it was for those frames.
#[test]
fn released_text_does_not_jump_back_before_the_page_is_redrawn() {
    // A page slow enough to render that the frames before its new raster can be looked at: many
    // pale squares, too light to count as ink.
    let mut content = String::from("0.9 g\n");
    for i in 0..60_000 {
        content.push_str(&format!("{} {} 1 1 re f\n", i % 290, (i / 290) % 390));
    }
    let pdf = format!(
        "%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >> endobj\n3 0 obj << /Type /Page /Parent 2 0 R /Contents 4 0 R >> endobj\n4 0 obj << /Length {} >> stream\n{content}\nendstream endobj\ntrailer << /Root 1 0 R >>\n%%EOF",
        content.len()
    );
    let mut h = harness_bytes(pdf.into_bytes());
    let (rect, from, to, end) = drag_added_text(&mut h);
    h.drop_at(end);
    // Frame by frame until the page has been re-rendered. The one frame that swaps in the new
    // raster reads back blank here (kittest renders it after its old texture is freed).
    let mut shown = Vec::new();
    while h.state().render_pending() || shown.is_empty() {
        h.run_steps(1);
        assert!(shown.len() < 500, "the page is re-rendered");
        let (old, new) = ink_at(&mut h, from, to);
        assert_eq!(old, 0, "frame {} after release: the text is back where it was", shown.len() + 1);
        shown.push(new);
    }
    assert!(shown.len() > 2, "frames before the new raster: {shown:?}");
    assert!(shown.iter().filter(|n| **n <= 20).count() <= 1, "the text stays where it was released: {shown:?}");
    assert_ne!(added(&h)[0].content.rect(), rect, "the move is applied");
    h.run_steps(2);
    let (old, new) = ink_at(&mut h, from, to);
    assert!(new > 20 && old == 0, "the redrawn page has the text in its new place ({old} old, {new} new)");
}

/// A justified Arabic Add text box stretches the spaces of its rows but the last to the box's width, as the page writes the text: the rows
/// that the saved lines span the box are as wide in the editor (their width is the box's plus the space a wrap leaves at the row's left
/// end), the last row is as long as its words.
#[test]
fn a_justified_arabic_add_text_box_stretches_its_rows_to_the_boxs_width() {
    if pdfcraft_fonts::document_arabic_font().is_none() {
        return;
    }
    let input = "سلام عليكم ورحمة الله وبركاته مع أطيب التحيات من الجميع هنا اليوم وفي كل مكان";
    for zoom in ["100", "200"] {
        let mut h = harness();
        h.state_mut().set_option("zoom", zoom).unwrap();
        h.run_steps(4);
        let before: Vec<String> = h.state().session.get(h.state().views[0].id).unwrap().text_lines(0).into_iter().map(|l| l.text).collect();
        assert!(h.state_mut().execute("edit.text"));
        h.run_steps(2);
        let point = at(&h, 140.0, 100.0);
        click(&mut h, point);
        h.state_mut().views[0].content.draft.as_mut().expect("the editor is open").style.align = pdfcraft_engine::TextAlign::Justify;
        h.event(egui::Event::Text(input.into()));
        h.run_steps(4);
        let galleys = painted_galleys(&h);
        let (_, galley) = galleys.iter().find(|(_, g)| g.job.text.contains("\u{633}\u{644}\u{627}\u{645}")).expect("the box paints the draft");
        let widths: Vec<f32> = galley.rows.iter().map(|row| row.row.size.x).collect();
        // The width of a space where the rows' break spaces are: the box's own font.
        let space = h.ctx.fonts_mut(|f| {
            f.layout_no_wrap(
                " ".into(),
                egui::FontId::new(12.0 * zoom.parse::<f32>().unwrap() / 100.0 * 4.0 / 3.0, egui::FontFamily::Proportional),
                egui::Color32::WHITE,
            )
            .size()
            .x
        });
        h.get_by_label("Done adding text").click();
        h.run_steps(3);
        let mut lines = h.state().session.get(h.state().views[0].id).unwrap().text_lines(0);
        lines.retain(|l| !before.contains(&l.text));
        assert!(lines.len() >= 3 && lines.len() == widths.len(), "{} rows, {} lines ({zoom}%)", widths.len(), lines.len());
        lines.sort_by(|a, b| b.rect[3].total_cmp(&a.rect[3]));
        for (i, (width, line)) in widths.iter().zip(&lines).enumerate().take(lines.len() - 1) {
            let page = at(&h, line.rect[2] as f32, 0.0).x - at(&h, line.rect[0] as f32, 0.0).x;
            assert!((width - space - page).abs() <= 3.0, "row {i} is {width} wide (a space is {space}), the page's line {page} ({zoom}%)");
        }
        let last = lines.len() - 1;
        let page_last = at(&h, lines[last].rect[2] as f32, 0.0).x - at(&h, lines[last].rect[0] as f32, 0.0).x;
        assert!(widths[last] <= page_last + space + 3.0 && widths[last] < widths[0] * 0.9, "the last row is not stretched ({zoom}%)");
    }
}
