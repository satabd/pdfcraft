//! Typing right-to-left script into the text fields of the real shell (egui_kittest): the string a
//! field stores and the value it announces stay in logical order, whatever order the letters
//! are drawn in.

use egui::accesskit::Role;
use egui::{Event, Key, Pos2, pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::{PdfCraftApp, QuickTool};

/// One 300×200 page.
const FIXTURE: &[u8] = b"%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >> endobj
4 0 obj << /Length 55 >> stream
BT /F1 14 Tf 20 150 Td (The quick brown fox) Tj ET
endstream endobj
5 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj
trailer << /Root 1 0 R >>
%%EOF";

/// "ab", then the letters of "ali" (ain, lam, ya).
const AB_ALI: &str = "ab\u{639}\u{644}\u{64A}";

fn harness() -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("text.pdf", None, FIXTURE.to_vec()).expect("opens");
        app.set_option("left", "closed").unwrap();
        app.set_option("author", "Tester").unwrap();
        app
    });
    h.run_steps(4);
    for _ in 0..200 {
        h.run_steps(2);
        if !h.state().render_pending() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    h
}

/// A point on page 1 in PDF user space → screen (no rotation; MediaBox 300×200).
fn at(h: &Harness<'static, PdfCraftApp>, x: f32, y: f32) -> Pos2 {
    let r = h.state().views[0].page_screen_rect(0).expect("page 1 on screen");
    pos2(r.left() + x / 300.0 * r.width(), r.top() + (200.0 - y) / 200.0 * r.height())
}

fn drag(h: &mut Harness<'static, PdfCraftApp>, from: Pos2, to: Pos2) {
    h.hover_at(from);
    h.run_steps(1);
    h.drag_at(from);
    h.run_steps(1);
    for k in 1..=4 {
        h.hover_at(from + (to - from) * (k as f32 / 4.0));
        h.run_steps(1);
    }
    h.drop_at(to);
    h.run_steps(3);
}

/// The comment properties dialog of a new rectangle, on its General tab.
fn author_dialog() -> Harness<'static, PdfCraftApp> {
    let mut h = harness();
    h.state_mut().set_option("quick", "square").unwrap();
    assert!(matches!(h.state().quick_tool, QuickTool::Comment(_)));
    let (a, b) = (at(&h, 40.0, 100.0), at(&h, 140.0, 40.0));
    drag(&mut h, a, b);
    h.state_mut().open_comment_props(0, 0);
    h.run_steps(2);
    h.get_by_label("General").click();
    h.run_steps(2);
    h
}

fn author(h: &Harness<'static, PdfCraftApp>) -> String {
    h.state().comment_props.as_ref().expect("the dialog is open").edited.author.clone()
}

/// Where egui's cursor is in the focused field, as a character index in the order the letters are drawn.
fn cursor(h: &Harness<'static, PdfCraftApp>) -> usize {
    let id = h.ctx.memory(|m| m.focused()).expect("a field has the focus");
    egui::TextEdit::load_state(&h.ctx, id).and_then(|s| s.cursor.char_range()).expect("a cursor").primary.index.0
}

/// The author field's announced value.
fn announced(h: &Harness<'static, PdfCraftApp>) -> Option<String> {
    h.get_by_role_and_label(Role::TextInput, "Author").value()
}

#[test]
fn arabic_typed_into_the_comment_author_field_is_stored_and_announced_in_logical_order() {
    let mut h = author_dialog();
    h.get_by_role_and_label(Role::TextInput, "Author").click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    h.run_steps(1);
    // One key at a time, as typed.
    for typed in ["a", "b", "\u{639}", "\u{644}", "\u{64A}"] {
        h.event(Event::Text(typed.to_owned()));
        h.run_steps(2);
    }
    assert_eq!(author(&h), AB_ALI, "stored as typed");
    assert_eq!(announced(&h).as_deref(), Some(AB_ALI), "announced as typed");
    // egui's cursor counts the letters as they are drawn, "ab" then the word reversed: it stays
    // after the "b", where the next letter of the word appears, not at the end of the stored text.
    assert_eq!(cursor(&h), 2);

    // Backspace takes the letter typed last, which is the one drawn first after the "b".
    h.key_press(Key::Backspace);
    h.run_steps(2);
    assert_eq!(author(&h), "ab\u{639}\u{644}");
    assert_eq!(announced(&h).as_deref(), Some("ab\u{639}\u{644}"));
    assert_eq!(cursor(&h), 2);

    // A whole word at once, as pasted or sent by an input method.
    h.event(Event::Text("\u{64A}".to_owned()));
    h.run_steps(2);
    h.event(Event::Paste(" \u{645}\u{646}".to_owned()));
    h.run_steps(2);
    assert_eq!(author(&h), "ab\u{639}\u{644}\u{64A} \u{645}\u{646}");
}

#[test]
fn latin_typed_into_the_author_field_behaves_as_it_always_did() {
    let mut h = author_dialog();
    h.get_by_role_and_label(Role::TextInput, "Author").click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    h.run_steps(1);
    h.event(Event::Text("Ada Lovelace".to_owned()));
    h.run_steps(2);
    assert_eq!(author(&h), "Ada Lovelace");
    assert_eq!(announced(&h).as_deref(), Some("Ada Lovelace"));
    h.key_press(Key::Backspace);
    h.run_steps(2);
    assert_eq!(author(&h), "Ada Lovelac");
}

#[test]
fn arabic_typed_into_the_command_palette_and_the_find_bar_is_kept_in_typing_order() {
    let mut h = harness();
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::K);
    h.run_steps(3);
    assert!(h.state().palette_open);
    for typed in ["a", "b", "\u{639}", "\u{644}", "\u{64A}"] {
        h.event(Event::Text(typed.to_owned()));
        h.run_steps(2);
    }
    assert_eq!(h.state().palette_query, AB_ALI);
    assert_eq!(cursor(&h), 2);
    h.key_press(Key::Backspace);
    h.run_steps(2);
    assert_eq!(h.state().palette_query, "ab\u{639}\u{644}");
    h.key_press(Key::Escape);
    h.run_steps(2);

    h.state_mut().run_command("edit.find");
    h.run_steps(4);
    for typed in ["\u{639}", "\u{644}", "\u{64A}"] {
        h.event(Event::Text(typed.to_owned()));
        h.run_steps(2);
    }
    let query = h.state().views[0].find.as_ref().map(|f| f.query.clone());
    assert_eq!(query.as_deref(), Some("\u{639}\u{644}\u{64A}"));
    // Right to left from the first letter: the caret is at the left end, after the last letter.
    assert_eq!(cursor(&h), 0);
}

/// Every galley painted in the last frame with its position, nested shapes included.
fn painted_galleys(h: &Harness<'static, PdfCraftApp>) -> Vec<(Pos2, std::sync::Arc<egui::Galley>)> {
    fn collect(shape: &egui::Shape, out: &mut Vec<(Pos2, std::sync::Arc<egui::Galley>)>) {
        match shape {
            egui::Shape::Text(text) => out.push((text.pos, std::sync::Arc::clone(&text.galley))),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in &h.output().shapes {
        collect(&clipped.shape, &mut out);
    }
    out
}

#[test]
fn a_two_line_arabic_comment_typed_into_the_composer_is_stored_in_order_and_drawn_from_the_right() {
    let mut h = harness();
    h.state_mut().set_option("quick", "note").unwrap();
    h.run_steps(2);
    let p = at(&h, 250.0, 180.0);
    h.hover_at(p);
    h.run_steps(1);
    h.drag_at(p);
    h.run_steps(1);
    h.drop_at(p);
    h.run_steps(3);
    assert!(h.state().views[0].comments.composer.is_some(), "the composer opened");
    let editor = || h.query_all_by(|n| n.placeholder() == Some("Add a comment")).next().expect("the composer's editor");
    editor().click();
    h.run_steps(2);

    // A paragraph long enough for two rows of the composer, a line break, a short second paragraph.
    let first = "هذا تعليق عربي طويل يكتب في مربع التعليق ليلتف على أكثر من سطر واحد في المربع";
    let second = "السطر الثاني";
    let mut typed = String::new();
    for word in first.split_inclusive(' ') {
        h.event(Event::Text(word.to_owned()));
        h.run_steps(2);
        typed.push_str(word);
    }
    h.key_press_modifiers(egui::Modifiers::SHIFT, Key::Enter);
    h.run_steps(2);
    typed.push('\n');
    for word in second.split_inclusive(' ') {
        h.event(Event::Text(word.to_owned()));
        h.run_steps(2);
        typed.push_str(word);
    }
    // Stored as typed: the soft wrap is no line break of the comment.
    let stored = h.state().views[0].comments.composer.as_ref().expect("the composer is open").text.clone();
    assert_eq!(stored, typed);
    assert_eq!(stored.matches('\n').count(), 1);
    let editor_node = h.query_all_by(|n| n.role() == egui::accesskit::Role::MultilineTextInput).next().expect("the editor");
    assert_eq!(editor_node.value().as_deref(), Some(typed.as_str()), "announced as typed");
    let field = editor_node.rect();

    // Painted: the editor's text is one galley, a row for each line, every row against the right edge.
    let galley = painted_galleys(&h)
        .into_iter()
        .filter(|(_, g)| g.job.text.contains('\n') && g.job.text.contains('\u{639}'))
        .max_by_key(|(_, g)| g.rows.len())
        .expect("the editor's text is painted");
    let (pos, galley) = galley;
    assert!(galley.rows.len() >= 3, "two rows for the first paragraph and one for the second: {}", galley.rows.len());
    // Reading starts at the right: the first row's rightmost letter is the first word's first.
    assert_eq!(galley.rows[0].glyphs.last().map(|g| g.chr), Some('\u{647}'), "row 0 is {:?}", galley.rows[0].text());
    // Every row ends at the same edge (a left-aligned paragraph is ragged on the right), the right
    // edge of the editor's text area.
    let rights: Vec<f32> = galley.rows.iter().map(|placed| pos.x + placed.pos.x + placed.row.size.x).collect();
    let (least, most) = rights.iter().fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(*x), hi.max(*x)));
    assert!(most - least < 2.0, "the rows end at {rights:?}");
    let margin = 10.0;
    assert!((most - (field.right() - margin)).abs() < 2.0, "the rows end at {most}, the field's text area at {}", field.right() - margin);
}

/// The painted galley whose text holds `c` and has the most rows (a field's, not a label's).
fn galley_holding(h: &Harness<'static, PdfCraftApp>, c: char) -> (Pos2, std::sync::Arc<egui::Galley>) {
    painted_galleys(h).into_iter().filter(|(_, g)| g.job.text.contains(c)).max_by_key(|(_, g)| g.rows.len()).expect("the field's text is painted")
}

/// The characters of `row` of `galley`, left to right as drawn.
fn row_chars(galley: &egui::Galley, row: usize) -> String {
    galley.rows.get(row).map(|placed| placed.glyphs.iter().map(|g| g.chr).collect()).unwrap_or_default()
}

/// "ali" (ain, lam, ya), a space and "abc": a line that starts right to left and ends in a Latin word.
const ALI_ABC: &str = "\u{639}\u{644}\u{64A} abc";
/// The same line as drawn: read from the right, so the Latin word is at the left end and the ain at the right.
const ALI_ABC_DRAWN: &str = "abc \u{64A}\u{644}\u{639}";

#[test]
fn arabic_typed_into_the_fill_and_sign_text_box_is_stored_in_typing_order_and_drawn_from_the_right() {
    let mut h = harness();
    h.state_mut().execute("sign.fill.text");
    h.run_steps(2);
    let p = at(&h, 50.0, 80.0);
    h.hover_at(p);
    h.run_steps(1);
    h.drag_at(p);
    h.run_steps(1);
    h.drop_at(p);
    h.run_steps(3);
    assert!(h.state().views[0].fill_text.is_some(), "the type box opened");
    for typed in ["\u{639}", "\u{644}", "\u{64A}", " ", "a", "b", "c"] {
        h.event(Event::Text(typed.to_owned()));
        h.run_steps(2);
    }
    assert_eq!(h.state().views[0].fill_text.as_ref().map(|t| t.text.as_str()), Some(ALI_ABC), "stored as typed");
    let field = h.query_all_by_role(Role::TextInput).find(|n| n.is_focused()).expect("the type box has the focus");
    assert_eq!(field.value().as_deref(), Some(ALI_ABC), "announced as typed");
    // Drawn in display order: the line reads from the right, its Latin word at the left end.
    let (_, galley) = galley_holding(&h, '\u{639}');
    assert_eq!(row_chars(&galley, 0), ALI_ABC_DRAWN);
    // The caret is after the "c", where the next Latin letter goes.
    assert_eq!(cursor(&h), 3);
    // Backspace takes the letter typed last.
    h.key_press(Key::Backspace);
    h.run_steps(2);
    assert_eq!(h.state().views[0].fill_text.as_ref().map(|t| t.text.as_str()), Some("\u{639}\u{644}\u{64A} ab"));
    h.event(Event::Text("c".to_owned()));
    h.run_steps(2);
    // Placed on the page as typed.
    h.key_press(Key::Enter);
    h.run_steps(3);
    let s = h.state();
    let contents: Vec<Option<String>> =
        s.session.get(s.views[0].id).expect("the document").info.annotations.iter().map(|a| a.contents.clone()).collect();
    assert!(contents.iter().any(|c| c.as_deref() == Some(ALI_ABC)), "{contents:?}");
}

#[test]
fn arabic_typed_into_the_search_and_redact_word_list_is_stored_in_typing_order_and_each_phrase_drawn_from_the_right() {
    let mut h = harness();
    h.state_mut().run_command("redact.search");
    h.run_steps(3);
    h.state_mut().redact_search.mode = pdfcraft_ui_egui::RedactSearchMode::Words;
    h.run_steps(2);
    let editor = || h.query_all_by(|n| n.placeholder() == Some("One word or phrase per line")).next().expect("the word list");
    editor().click();
    h.run_steps(2);
    // Two phrases, one per line, a letter at a time.
    let second = "\u{628}\u{628} 12";
    for (i, line) in [ALI_ABC, second].into_iter().enumerate() {
        if i > 0 {
            h.key_press(Key::Enter);
            h.run_steps(2);
        }
        for typed in line.chars() {
            h.event(Event::Text(typed.to_string()));
            h.run_steps(2);
        }
    }
    let typed = format!("{ALI_ABC}\n{second}");
    assert_eq!(h.state().redact_search.words, typed, "stored as typed");
    let node = h.query_all_by(|n| n.role() == Role::MultilineTextInput).next().expect("the word list");
    assert_eq!(node.value().as_deref(), Some(typed.as_str()), "announced as typed");
    // Drawn in display order, a row for each phrase, each read from the right and set against the right edge.
    let (pos, galley) = galley_holding(&h, '\u{628}');
    assert_eq!(galley.rows.len(), 2, "{:?}", galley.job.text);
    assert_eq!(row_chars(&galley, 0), ALI_ABC_DRAWN);
    assert_eq!(row_chars(&galley, 1), "12 \u{628}\u{628}");
    let rights: Vec<f32> = galley.rows.iter().map(|placed| pos.x + placed.pos.x + placed.row.size.x).collect();
    assert!((rights[0] - rights[1]).abs() < 2.0, "both phrases end at the right edge: {rights:?}");
    // The list the search uses is the stored text's.
    assert_eq!(pdfcraft_engine::redact_word_list(&h.state().redact_search.words), [ALI_ABC, second]);
}
