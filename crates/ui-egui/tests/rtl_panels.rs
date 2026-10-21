//! Document text in right-to-left script (Arabic, Hebrew, Persian) is painted in display order
//! in the panels and dialogs, while the accessible names keep the logical text.
//!
//! egui lays the runs of a line out left to right in logical order, so a mixed-direction string
//! from a PDF would show its words swapped. The panels put it through `bidi::visual_auto`
//! (`bidi` is private, so the expected strings are written out here).
//!
//! The expected display string for the logical title "تقرير 2024 Report" is
//! "Report 2024 تقرير". It follows from the bidirectional algorithm (UAX #9) for a paragraph
//! that starts with an Arabic letter, so reads right to left: the Arabic word is rightmost, the
//! number (Arabic digits' rule W2) to its left, the Latin word leftmost. The Arabic word itself
//! stays in logical order, because egui's shaper lays out a run of Arabic script right to left
//! on its own. The unit tests of `bidi.rs` (`single_line_with_its_own_direction`) pin the same
//! rules for other strings.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use pdfcraft_ui_egui::PdfCraftApp;

/// The logical text of an Arabic word, a number and a Latin word, as a PDF stores it.
const LOGICAL: &str = "تقرير 2024 Report";
/// What egui must be handed to show it right.
const DISPLAY: &str = "Report 2024 تقرير";

/// `s` as a PDF text string: UTF-16BE with a byte-order mark, in hex.
fn pdf_text(s: &str) -> String {
    let mut hex = String::from("FEFF");
    for unit in s.encode_utf16() {
        hex.push_str(&format!("{unit:04X}"));
    }
    format!("<{hex}>")
}

/// A one-page document with one bookmark and a creator, both named `LOGICAL`.
fn fixture() -> Vec<u8> {
    format!(
        "%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >> endobj
3 0 obj << /Type /Page /Parent 2 0 R >> endobj
4 0 obj << /Type /Outlines /First 5 0 R /Last 5 0 R /Count 1 >> endobj
5 0 obj << /Title {title} /Parent 4 0 R /Dest [3 0 R /Fit] >> endobj
6 0 obj << /Title (Plain title) /Creator {title} >> endobj
trailer << /Root 1 0 R /Info 6 0 R >>
%%EOF",
        title = pdf_text(LOGICAL)
    )
    .into_bytes()
}

fn harness(setup: impl FnOnce(&mut PdfCraftApp) + 'static) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("rtl.pdf", None, fixture()).expect("opens");
        app.set_option("left", "closed").unwrap();
        setup(&mut app);
        app
    });
    h.run_steps(6);
    h
}

/// The text of every galley painted this frame.
fn painted(h: &Harness<'static, PdfCraftApp>) -> Vec<String> {
    h.output()
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_bookmark_with_arabic_script_is_painted_in_display_order() {
    let h = harness(|app| app.set_option("panel", "bookmarks").unwrap());
    // The accessible name is the logical title.
    h.get_by_label(LOGICAL);
    let shown = painted(&h);
    assert!(shown.iter().any(|s| s == DISPLAY), "the title is painted in display order: {shown:?}");
    assert!(!shown.iter().any(|s| s == LOGICAL), "and not in logical order: {shown:?}");
}

#[test]
fn properties_show_arabic_script_in_display_order() {
    // Title, Author and the others are editable fields for a document that can be modified, which
    // keep the logical text; Application is always a row of values.
    let h = harness(|app| app.set_option("dialog", "properties").unwrap());
    // The row's accessible name is the logical text (the Bookmarks panel behind the dialog has a
    // button of the same name).
    assert!(h.query_all_by_label(LOGICAL).any(|n| n.accesskit_node().role() == egui::accesskit::Role::Label));
    let shown = painted(&h);
    assert!(shown.iter().any(|s| s == DISPLAY), "the creator is painted in display order: {shown:?}");
    assert!(!shown.iter().any(|s| s == LOGICAL), "and not in logical order: {shown:?}");
}
