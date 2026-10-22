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
