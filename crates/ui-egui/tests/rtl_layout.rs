//! The interface is mirrored for a right-to-left language (Arabic): the Tools pane and the
//! navigation panes swap sides, the rail moves to the left edge, and the toolbar, the tab strip
//! and the rows start at the right. English is laid out as before.
//!
//! Catalog labels are already in display order, so these tests look labels up with `i18n::tr`
//! and only compare positions.

use craft_ui::docking::{Action, Placement, Zone};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;
use pdfcraft_ui_egui::docking::{Panel, Workspace};
use pdfcraft_ui_egui::i18n::{self, Lang};

const WIDTH: f32 = 1400.0;

/// The logical text of an Arabic word, a number and a Latin word, as a PDF stores it.
const LOGICAL: &str = "تقرير 2024 Report";

/// `s` as a PDF text string: UTF-16BE with a byte-order mark, in hex.
fn pdf_text(s: &str) -> String {
    let mut hex = String::from("FEFF");
    for unit in s.encode_utf16() {
        hex.push_str(&format!("{unit:04X}"));
    }
    format!("<{hex}>")
}

/// A one-page document with one bookmark named `LOGICAL`.
fn fixture() -> Vec<u8> {
    format!(
        "%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >> endobj
3 0 obj << /Type /Page /Parent 2 0 R >> endobj
4 0 obj << /Type /Outlines /First 5 0 R /Last 5 0 R /Count 1 >> endobj
5 0 obj << /Title {title} /Parent 4 0 R /Dest [3 0 R /Fit] >> endobj
trailer << /Root 1 0 R >>
%%EOF",
        title = pdf_text(LOGICAL)
    )
    .into_bytes()
}

fn harness(language: &'static str, setup: impl FnOnce(&mut PdfCraftApp) + 'static) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(WIDTH, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", language).unwrap();
        app.open_bytes("rtl.pdf", None, fixture()).expect("opens");
        setup(&mut app);
        app
    });
    h.run_steps(6);
    h
}

fn ar(english: &str) -> &str {
    i18n::tr(Lang::from_code("ar").unwrap(), english)
}

/// The centre x of the one button called `label`.
fn button_x(h: &Harness<'static, PdfCraftApp>, label: &str) -> f32 {
    h.get_by_role_and_label(Role::Button, label).rect().center().x
}

/// The centre x of the button called `label` on the tab strip (the top 40 points).
fn strip_x(h: &Harness<'static, PdfCraftApp>, label: &str) -> f32 {
    let mut found = h.query_all_by_role_and_label(Role::Button, label).filter(|n| n.rect().top() < 40.0);
    let first = found.next().unwrap_or_else(|| panic!("no {label:?} button on the tab strip"));
    assert!(found.next().is_none(), "two {label:?} buttons on the tab strip");
    first.rect().center().x
}

#[test]
fn the_navigation_pane_is_on_the_left_in_arabic_and_on_the_right_in_english() {
    let h = harness("ar", |app| {
        app.set_option("left", "closed").unwrap();
        app.set_option("panel", "bookmarks").unwrap();
    });
    assert!(h.get_by_label(LOGICAL).rect().center().x < WIDTH / 2.0, "the pane sits at the start side of the document");
    let h = harness("en", |app| {
        app.set_option("left", "closed").unwrap();
        app.set_option("panel", "bookmarks").unwrap();
    });
    assert!(h.get_by_label(LOGICAL).rect().center().x > WIDTH / 2.0);
}

#[test]
fn the_tools_pane_is_on_the_right_in_arabic_and_on_the_left_in_english() {
    let row = |h: &Harness<'static, PdfCraftApp>, lang: &str| {
        let label = i18n::tr(Lang::from_code(lang).unwrap(), "Organize pages").to_owned();
        h.get_by_role_and_label(Role::Button, &label).rect().center().x
    };
    let h = harness("ar", |_| {});
    assert!(row(&h, "ar") > WIDTH / 2.0, "the Arabic tool list is at the right");
    assert!(h.state().docking.tools_on_right());
    let h = harness("en", |_| {});
    assert!(row(&h, "en") < WIDTH / 2.0);
    assert!(!h.state().docking.tools_on_right());
}

#[test]
fn the_rail_moves_to_the_left_edge_in_arabic() {
    let h = harness("ar", |_| {});
    assert!(button_x(&h, ar("Comments")) < 60.0, "the rail is at the left edge");
    let h = harness("en", |_| {});
    assert!(button_x(&h, "Comments") > WIDTH - 60.0);
}

#[test]
fn the_toolbar_starts_at_the_right_in_arabic() {
    let h = harness("ar", |_| {});
    assert!(button_x(&h, ar("All tools")) > button_x(&h, ar("E-Sign")), "the first mode tab is the rightmost");
    assert!(button_x(&h, ar("Find tools and commands")) < button_x(&h, ar("E-Sign")), "search is at the end side");
    assert!(button_x(&h, ar("Menu")) > WIDTH / 2.0, "the menu button is at the start side");
    assert!(button_x(&h, ar("Home")) > WIDTH / 2.0, "Home is the first thing on the tab strip");
    assert!(button_x(&h, ar("Open")) < button_x(&h, ar("Home")), "Open follows the tabs, to the left of Home");
    assert!(button_x(&h, "Discord") < button_x(&h, ar("Open")), "the community, help and theme buttons are at the far left");
    assert!(button_x(&h, ar("Keyboard shortcuts")) < button_x(&h, "Discord"), "help is outside the community button");
    let h = harness("en", |_| {});
    assert!(button_x(&h, "All tools") < button_x(&h, "E-Sign"));
    assert!(button_x(&h, "Find tools and commands") > button_x(&h, "E-Sign"));
    assert!(button_x(&h, "Menu") < WIDTH / 2.0);
    assert!(button_x(&h, "Home") < WIDTH / 2.0);
    assert!(button_x(&h, "Open") > button_x(&h, "Home"));
    assert!(button_x(&h, "Discord") > button_x(&h, "Open"));
    assert!(button_x(&h, "Keyboard shortcuts") > button_x(&h, "Discord"));
}

/// The window buttons stay where the desktop put them, in the desktop's order, whatever the
/// language: only the rest of the strip is mirrored.
#[test]
fn the_window_buttons_stay_at_the_physical_edge() {
    use pdfcraft_ui_egui::window_controls::Layout;
    for (language, home) in [("en", "Home"), ("ar", ar("Home"))] {
        let h = harness(language, |app| app.window_controls = Some(Layout::default()));
        let label = |english: &str| if language == "ar" { ar(english).to_owned() } else { english.to_owned() };
        let (min, max, close) = (strip_x(&h, &label("Minimize")), strip_x(&h, &label("Maximize")), strip_x(&h, &label("Close")));
        assert!(min < max && max < close, "{language}: the buttons read in the desktop's order");
        assert!(close > WIDTH - 60.0, "{language}: and sit at the right edge");
        let home = strip_x(&h, home);
        if language == "ar" {
            assert!(home < min && min - home < 100.0, "ar: Home is the first thing after the window buttons");
        } else {
            assert!(home < 100.0, "en: Home is at the left edge");
        }
    }
}

#[test]
fn the_standard_layout_follows_the_language_and_a_customised_one_stays() {
    let mut h = harness("ar", |_| {});
    assert!(h.state().docking.tools_on_right());
    h.state_mut().set_option("language", "en").unwrap();
    h.run_steps(4);
    assert!(!h.state().docking.tools_on_right(), "the standard layout mirrors back");
    // The stock arrangement again, with the Inspector closed as at the start (no side panel).
    let mut stock = Workspace::default_for(false);
    stock.set_visible(Panel::Inspector, h.state().right.is_some()).unwrap();
    assert_eq!(serde_json::to_value(&h.state().docking).unwrap(), serde_json::to_value(&stock).unwrap());

    // The user moved Tools to the right in English: Arabic keeps what they chose.
    h.state_mut().set_option("tools-side", "right").unwrap();
    h.state_mut().set_option("language", "ar").unwrap();
    h.run_steps(4);
    assert!(h.state().docking.tools_on_right());
    // And a layout that is not the stock shape is never swapped.
    let mut custom = Workspace::default_for(false);
    custom
        .apply(Action::Move { panel: Panel::Inspector, anchor: Panel::Tools, placement: Placement::Split(Zone::Left) })
        .expect("the Inspector moves to the far left");
    let before = serde_json::to_value(&custom).unwrap();
    h.state_mut().docking = custom;
    h.state_mut().set_option("language", "en").unwrap();
    h.run_steps(4);
    h.state_mut().set_option("language", "ar").unwrap();
    h.run_steps(4);
    assert_eq!(h.state().docking.layout, serde_json::from_value::<Workspace>(before).unwrap().layout);
}

#[test]
fn the_mirror_flag_persists_with_the_layout() {
    let h = harness("ar", |_| {});
    let saved = h.state().persist();
    let mut restored = PdfCraftApp::new();
    restored.restore(&saved);
    assert!(restored.docking.tools_on_right());
    assert!(saved.contains("\"layout_rtl\":true"));
    let h = harness("en", |_| {});
    assert!(h.state().persist().contains("\"layout_rtl\":false"));
}

#[test]
fn the_english_default_layout_is_unchanged() {
    // Pinned before the mirrored layout existed.
    assert_eq!(serde_json::to_string(&Workspace::default()).unwrap(), GOLDEN_DEFAULT);
    assert_eq!(serde_json::to_string(&Workspace::default_for(false)).unwrap(), GOLDEN_DEFAULT);
}

const GOLDEN_DEFAULT: &str = r#"{"layout":{"root":{"Split":{"axis":"Horizontal","size":{"FixedFirst":272.0},"first":{"Tabs":{"panels":["tools"],"active":0}},"second":{"Split":{"axis":"Horizontal","size":{"FixedSecond":330.0},"first":{"Tabs":{"panels":["canvas"],"active":0}},"second":{"Tabs":{"panels":["inspector"],"active":0}}}}}},"floating":[]},"hidden":[]}"#;

/// The rectangle of each text galley painted this frame, with its text.
fn painted_text(h: &Harness<'static, PdfCraftApp>) -> Vec<(String, egui::Rect)> {
    h.output()
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => Some((t.galley.text().to_owned(), t.galley.rect.translate(t.pos.to_vec2()))),
            _ => None,
        })
        .collect()
}

/// A bookmark row starts at its start edge with the title and keeps the destination page in its
/// own column at the end: reversed between the two languages.
#[test]
fn a_bookmark_row_runs_from_the_start_edge() {
    for (language, display) in [("ar", "Report 2024 تقرير"), ("en", "Report 2024 تقرير")] {
        let h = harness(language, |app| {
            app.set_option("left", "closed").unwrap();
            app.set_option("panel", "bookmarks").unwrap();
        });
        let shown = painted_text(&h);
        let title = shown.iter().find(|(text, _)| text == display).unwrap_or_else(|| panic!("{language}: the title is painted: {shown:?}")).1;
        // The page label "1" painted in the bookmark row.
        let label = shown.iter().filter(|(text, r)| text == "1" && (r.center().y - title.center().y).abs() < 20.0).map(|(_, r)| *r).next();
        let label = label.unwrap_or_else(|| panic!("{language}: the page label is painted beside the title: {shown:?}"));
        if language == "ar" {
            // The pane is at the left of the window (x 48 to 378): the title ends at its right
            // edge and the page label sits at its left edge.
            assert!(title.center().x > label.center().x, "ar: the title is at the start (right) of the label");
            assert!(label.left() < 100.0 && title.right() > 250.0, "ar: label {label:?} title {title:?}");
        } else {
            assert!(title.center().x < label.center().x, "en: the title is at the start (left) of the label");
            assert!(label.right() > 1250.0 && title.right() < label.left(), "en: label {label:?} title {title:?}");
        }
    }
}

/// Every node that has an extent lies inside the window: a mirrored row that ran off the edge
/// (a run laid out against the wrong side) would put controls at negative x.
fn assert_on_screen(h: &Harness<'static, PdfCraftApp>, what: &str) {
    use egui_kittest::kittest::NodeT;
    use egui_kittest::kittest::Queryable as _;
    let mut off = Vec::new();
    for n in h.query_all(egui_kittest::kittest::By::new()) {
        let Some(r) = n.accesskit_node().raw_bounds() else { continue };
        // The centred zoom field's text run overhangs its field by a few points in either language.
        if n.accesskit_node().role() == Role::TextRun {
            continue;
        }
        if r.width() > 0.0 && r.height() > 0.0 && (r.x0 < -1.0 || r.x1 > f64::from(WIDTH) + 1.0) {
            off.push(format!("{:?} {:?} {r:?}", n.accesskit_node().role(), n.accesskit_node().label()));
        }
    }
    assert!(off.is_empty(), "{what}: nodes outside the window: {off:#?}");
}

#[test]
fn every_screen_stays_inside_the_window() {
    for lang in ["ar", "en"] {
        let mut h = harness(lang, |app| {
            app.set_option("panel", "bookmarks").unwrap();
        });
        assert_on_screen(&h, &format!("{lang} bookmarks"));
        for panel in ["comments", "pages", "fields", "layers", "attachments", "signatures", "tags", "search", "compare", "accessibility"] {
            h.state_mut().set_option("panel", panel).unwrap();
            h.run_steps(3);
            assert_on_screen(&h, &format!("{lang} {panel}"));
        }
        for dialog in [
            "shortcuts",
            "preferences",
            "about",
            "properties",
            "security",
            "split",
            "protect",
            "page-boxes",
            "word-count",
            "action-wizard",
            "watermark",
        ] {
            h.state_mut().set_option("dialog", dialog).unwrap();
            h.run_steps(4);
            assert_on_screen(&h, &format!("{lang} {dialog}"));
            h.key_press(egui::Key::Escape);
            h.run_steps(3);
        }
        h.state_mut().set_option("panel", "none").unwrap();
        for group in pdfcraft_engine::catalog::TOOL_GROUPS {
            h.state_mut().set_option("tool", group.id).unwrap();
            h.run_steps(3);
            assert_on_screen(&h, &format!("{lang} tool {}", group.id));
        }
        h.state_mut().set_option("palette", "").unwrap();
        h.run_steps(4);
        assert_on_screen(&h, &format!("{lang} palette"));
        h.key_press(egui::Key::Escape);
        h.state_mut().palette_open = false;
        h.state_mut().active = None;
        h.run_steps(4);
        assert_on_screen(&h, &format!("{lang} home"));
    }
}
