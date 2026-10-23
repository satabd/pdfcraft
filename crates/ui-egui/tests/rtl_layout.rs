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
use pdfcraft_engine::{Edit, NewField, Session};
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
            "recognize-text",
            "compare-files",
            "optimize",
            "pdfa",
            "signature",
            "js-console",
            "document-js",
            "header-footer",
            "export-image",
            "accessibility-options",
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

/// The caption label `caption` and the `field_role` node on its row.
fn row_pair(h: &Harness<'static, PdfCraftApp>, caption: &str, field_role: Role) -> (egui::Rect, egui::Rect) {
    let label = h.get_by_role_and_label(Role::Label, caption).rect();
    let field = h
        .query_all_by_role(field_role)
        .map(|n| n.rect())
        .filter(|r| (r.center().y - label.center().y).abs() < 12.0)
        .min_by(|a, b| (a.center().y - label.center().y).abs().total_cmp(&(b.center().y - label.center().y).abs()))
        .unwrap_or_else(|| panic!("no {field_role:?} on the row of {caption:?} at {label:?}"));
    (label, field)
}

/// A dialog's caption rows read from the start edge: the caption is to the right of its field in
/// Arabic and to the left of it in English.
#[test]
fn a_dialog_caption_sits_at_the_start_side_of_its_field() {
    for (language, caption) in [("ar", ar("Author")), ("en", "Author")] {
        let h = harness(language, |app| app.set_option("dialog", "properties").unwrap());
        let (label, field) = row_pair(&h, caption, Role::TextInput);
        if language == "ar" {
            assert!(label.left() >= field.right(), "ar: the caption {label:?} is right of its field {field:?}");
            assert!(label.right() > WIDTH / 2.0 + 200.0, "ar: and at the right edge of the dialog: {label:?}");
        } else {
            assert!(label.right() <= field.left(), "en: the caption {label:?} is left of its field {field:?}");
            assert!(label.left() < WIDTH / 2.0 - 200.0, "en: and at the left edge of the dialog: {label:?}");
        }
        assert_on_screen(&h, &format!("{language} properties"));
    }
}

/// The main menu's File submenu open, in `language`.
fn file_menu(language: &'static str) -> Harness<'static, PdfCraftApp> {
    let tr = |english: &str| if language == "ar" { ar(english).to_owned() } else { english.to_owned() };
    let mut h = harness(language, |_| {});
    h.get_by_role_and_label(Role::Button, &tr("Menu")).click();
    h.run_steps(4);
    h.get_by_role_and_label(Role::Button, &format!("{} \u{23F5}", tr("File"))).click();
    h.run_steps(6);
    h
}

/// A menu entry has its label at the start edge and its shortcut at the end: the label right of
/// the shortcut in Arabic, left of it in English. Every entry fills the menu's width, so the
/// whole row is clickable, and the accessible name stays "label shortcut".
#[test]
fn a_menu_entry_has_its_label_at_the_start_and_its_shortcut_at_the_end() {
    use egui_kittest::kittest::NodeT as _;
    for language in ["ar", "en"] {
        let h = file_menu(language);
        let open = if language == "ar" { ar("Open…") } else { "Open…" };
        let shown = painted_text(&h);
        let label = shown.iter().find(|(text, _)| text == open).unwrap_or_else(|| panic!("{language}: {open:?} is painted: {shown:?}")).1;
        let shortcut = shown
            .iter()
            .find(|(text, r)| text.ends_with("+O") && (r.center().y - label.center().y).abs() < 4.0)
            .unwrap_or_else(|| panic!("{language}: the shortcut of {open:?} is painted on its row: {shown:?}"))
            .1;
        if language == "ar" {
            assert!(label.left() >= shortcut.right(), "ar: the label {label:?} is right of its shortcut {shortcut:?}");
        } else {
            assert!(label.right() <= shortcut.left(), "en: the label {label:?} is left of its shortcut {shortcut:?}");
        }
        // The entry is one button across the menu, named as in English.
        let entry = h
            .query_all_by_role(Role::Button)
            .find(|n| n.accesskit_node().label().is_some_and(|l| l.starts_with(open) && l.contains("+O")))
            .unwrap_or_else(|| panic!("{language}: a button named {open:?} and its shortcut"))
            .rect();
        assert!(entry.contains_rect(label) && entry.contains_rect(shortcut), "{language}: the entry {entry:?} holds both texts");
        assert!(entry.width() > label.width() + shortcut.width(), "{language}: and spans the menu");
        assert_on_screen(&h, &format!("{language} file menu"));
    }
}

/// The dialogs that moved from `egui::Grid` to `form_grid` read from the start edge too: the
/// caption is to the right of its field in Arabic and to the left of it in English.
#[test]
fn converted_dialogs_put_the_caption_at_the_start_side_of_its_field() {
    for (dialog, caption, role) in [("page-boxes", "Top", Role::SpinButton), ("recognize-text", "Document language", Role::ComboBox)] {
        for (language, name) in [("ar", ar(caption)), ("en", caption)] {
            let h = harness(language, |app| app.set_option("dialog", dialog).unwrap());
            let (label, field) = row_pair(&h, name, role);
            if language == "ar" {
                assert!(label.left() >= field.right(), "ar {dialog}: the caption {label:?} is right of its field {field:?}");
                assert!(label.right() > WIDTH / 2.0, "ar {dialog}: and in the right half of the window: {label:?}");
            } else {
                assert!(label.right() <= field.left(), "en {dialog}: the caption {label:?} is left of its field {field:?}");
                assert!(label.left() < WIDTH / 2.0, "en {dialog}: and in the left half of the window: {label:?}");
            }
            assert_on_screen(&h, &format!("{language} {dialog}"));
        }
    }
}

/// A dialog's title starts at the start edge of the dialog: at its right edge in Arabic (the
/// dialog is 520 points wide and centred in the window) and at its left edge in English.
#[test]
fn a_dialog_title_starts_at_the_start_edge() {
    for (language, title) in [("ar", ar("Set Page Boxes")), ("en", "Set Page Boxes")] {
        let h = harness(language, |app| app.set_option("dialog", "page-boxes").unwrap());
        let title = painted_text(&h).into_iter().find(|(text, _)| text == title).unwrap_or_else(|| panic!("{language}: the title is painted")).1;
        if language == "ar" {
            assert!(title.right() > WIDTH / 2.0 + 200.0, "ar: the title {title:?} ends at the right edge of the dialog");
        } else {
            assert!(title.left() < WIDTH / 2.0 - 200.0, "en: the title {title:?} starts at the left edge of the dialog");
        }
    }
}

/// The Properties tab strip runs from the start edge: the first tab is the rightmost in Arabic.
#[test]
fn the_properties_tab_strip_runs_from_the_start_edge() {
    for language in ["ar", "en"] {
        let h = harness(language, |app| app.set_option("dialog", "properties").unwrap());
        let tr = |english: &str| if language == "ar" { ar(english).to_owned() } else { english.to_owned() };
        let (first, last) = (button_x(&h, &tr("Description")), button_x(&h, &tr("Advanced")));
        if language == "ar" {
            assert!(first > last, "ar: Description ({first}) is right of Advanced ({last})");
        } else {
            assert!(first < last, "en: Description ({first}) is left of Advanced ({last})");
        }
    }
}

/// A tab's context menu: its entries are flush with the start edge of the menu, the right in Arabic.
#[test]
fn a_tab_context_menu_entry_has_its_label_at_the_start_edge() {
    use egui_kittest::kittest::NodeT as _;
    for language in ["ar", "en"] {
        let mut h = harness(language, |_| {});
        let name = "rtl.pdf";
        let tab = h.query_all_by_label_contains(name).find(|n| n.rect().top() < 40.0).unwrap_or_else(|| panic!("{language}: a tab named {name}"));
        tab.click_secondary();
        h.run_steps(6);
        let close = if language == "ar" { ar("Close Tab") } else { "Close Tab" };
        let label = painted_text(&h).into_iter().find(|(text, _)| text == close).unwrap_or_else(|| panic!("{language}: {close:?} is painted")).1;
        let entry = h
            .query_all_by_role(Role::Button)
            .find(|n| n.accesskit_node().label().is_some_and(|l| l.starts_with(close)))
            .unwrap_or_else(|| panic!("{language}: a button named {close:?}"))
            .rect();
        assert!(entry.contains_rect(label), "{language}: the entry {entry:?} holds its label {label:?}");
        let (gap_start, gap_end) = if language == "ar" {
            (entry.right() - label.right(), label.left() - entry.left())
        } else {
            (label.left() - entry.left(), entry.right() - label.right())
        };
        assert!(gap_start < 16.0, "{language}: the label {label:?} is flush with the start of {entry:?}");
        assert!(gap_end > gap_start, "{language}: and the entry spans the menu");
        assert_on_screen(&h, &format!("{language} tab menu"));
    }
}

/// The page's context menu: Copy has its label at the start and its shortcut at the end.
#[test]
fn the_canvas_copy_entry_has_its_label_at_the_start_and_its_shortcut_at_the_end() {
    for language in ["ar", "en"] {
        let mut h = harness(language, |_| {});
        let at = egui::pos2(WIDTH / 2.0, 450.0);
        h.event(egui::Event::PointerMoved(at));
        h.run_steps(2);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE });
            h.run_steps(1);
        }
        h.run_steps(5);
        let copy = if language == "ar" { ar("Copy") } else { "Copy" };
        let shown = painted_text(&h);
        let label = shown.iter().find(|(text, _)| text == copy).unwrap_or_else(|| panic!("{language}: {copy:?} is painted: {shown:?}")).1;
        let shortcut = shown
            .iter()
            .find(|(text, r)| text.ends_with("+C") && (r.center().y - label.center().y).abs() < 4.0)
            .unwrap_or_else(|| panic!("{language}: the shortcut of Copy is painted on its row: {shown:?}"))
            .1;
        if language == "ar" {
            assert!(label.left() >= shortcut.right(), "ar: the label {label:?} is right of its shortcut {shortcut:?}");
        } else {
            assert!(label.right() <= shortcut.left(), "en: the label {label:?} is left of its shortcut {shortcut:?}");
        }
        assert_on_screen(&h, &format!("{language} canvas menu"));
    }
}

/// A mirrored form fills the width it is given, so a dialog that puts something beside its form
/// (the page preview of Set Page Boxes) must give the form a width of its own. The dialog is
/// centred, so its right edge, where the OK button ends, tells how wide it is: as in English.
#[test]
fn a_mirrored_dialog_with_a_preview_is_as_wide_as_the_english_one() {
    let right_edge = |language: &'static str| {
        let h = harness(language, |app| app.set_option("dialog", "page-boxes").unwrap());
        let ok = if language == "ar" { ar("OK") } else { "OK" };
        painted_text(&h).into_iter().find(|(text, _)| text == ok).unwrap_or_else(|| panic!("{language}: OK is painted")).1.right()
    };
    let (ar_right, en_right) = (right_edge("ar"), right_edge("en"));
    assert!((ar_right - en_right).abs() < 40.0, "the OK button ends at {ar_right} in Arabic and at {en_right} in English");
}

/// No dialog is wider mirrored than in English: a form that filled more room than it was given
/// would push the dialog's right edge out. The dialogs are centred, so the right edge of the text
/// in the middle band of the window tells how wide they are.
#[test]
fn a_mirrored_dialog_is_not_wider_than_the_english_one() {
    let right_edge = |language: &'static str, dialog: &'static str| {
        let h = harness(language, |app| app.set_option("dialog", dialog).unwrap());
        painted_text(&h)
            .into_iter()
            .filter(|(_, r)| r.center().y > 100.0 && r.center().y < 800.0 && r.left() > 330.0 && r.right() < 1100.0)
            .map(|(_, r)| r.right())
            .fold(f32::NEG_INFINITY, f32::max)
    };
    for dialog in [
        "properties",
        "page-boxes",
        "recognize-text",
        "compare-files",
        "optimize",
        "word-count",
        "protect",
        "split",
        "number-pages",
        "action-wizard",
        "watermark",
        "pdfa",
        "signature",
        "js-console",
        "document-js",
        "export-image",
        "accessibility-options",
    ] {
        let (ar_right, en_right) = (right_edge("ar", dialog), right_edge("en", dialog));
        assert!(ar_right <= en_right + 40.0, "{dialog}: the text ends at {ar_right} in Arabic and at {en_right} in English");
    }
}

/// Field Properties on a form, in `language`, on the tab called `tab` (an English name).
fn field_properties(language: &'static str, tab: &str) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(WIDTH, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", language).unwrap();
        app.open_bytes("form.pdf", None, include_bytes!("data/form.pdf").to_vec()).unwrap();
        app
    });
    h.run_steps(6);
    h.state_mut().execute("form.prepare");
    h.run_steps(2);
    h.state_mut().open_field_props("city", 0);
    h.run_steps(4);
    if tab != "General" {
        let label = if language == "ar" { ar(tab) } else { tab };
        h.get_by_role_and_label(Role::Button, label).click();
        h.run_steps(5);
    }
    h
}

/// The forms of Field Properties (general, appearance with its colour swatches, and the
/// position pairs side by side) read from the start edge and stay inside the dialog, which is
/// no wider than in English.
#[test]
fn field_properties_forms_are_mirrored_and_keep_the_dialog_width() {
    let mut right_edges = Vec::new();
    for language in ["ar", "en"] {
        // The mirrored form paints a caption's closing colon at its left: ":الاسم".
        let tr = |english: &str| {
            if language != "ar" {
                return english.to_owned();
            }
            let shown = ar(english);
            shown.trim_end().strip_suffix(':').map_or_else(|| shown.to_owned(), |words| format!(":{words}"))
        };
        // General: the caption is at the start side of its field.
        let h = field_properties(language, "General");
        let (label, field) = row_pair(&h, &tr("Name:"), Role::TextInput);
        if language == "ar" {
            assert!(label.left() >= field.right(), "ar: the caption {label:?} is right of its field {field:?}");
        } else {
            assert!(label.right() <= field.left(), "en: the caption {label:?} is left of its field {field:?}");
        }
        assert_on_screen(&h, &format!("{language} field general"));
        // Appearance: the swatch grids sit beside their captions, not at the far side.
        let h = field_properties(language, "Appearance");
        let colour = h.get_by_role_and_label(Role::Label, &tr("Text Color:")).rect();
        let swatches: Vec<_> = h.query_all_by_role_and_label(Role::Button, &tr("Black")).map(|n| n.rect()).collect();
        assert!(!swatches.is_empty(), "{language}: the colour swatches are there");
        let swatch =
            swatches.iter().min_by(|a, b| (a.center().y - colour.center().y).abs().total_cmp(&(b.center().y - colour.center().y).abs())).unwrap();
        if language == "ar" {
            assert!(
                swatch.right() <= colour.left() && colour.left() - swatch.right() < 200.0,
                "ar: swatches {swatch:?} beside the caption {colour:?}"
            );
        } else {
            assert!(
                swatch.left() >= colour.right() && swatch.left() - colour.right() < 200.0,
                "en: swatches {swatch:?} beside the caption {colour:?}"
            );
        }
        assert_on_screen(&h, &format!("{language} field appearance"));
        // Position: Left and Width in one column, Bottom and Height in the other, all inside.
        let h = field_properties(language, "Position");
        for caption in ["Left:", "Bottom:", "Width:", "Height:"] {
            let label = h.get_by_role_and_label(Role::Label, &tr(caption)).rect();
            // Two forms share each row: the field next to the caption, on the side away from its edge.
            let gap = |field: &egui::Rect| if language == "ar" { label.left() - field.right() } else { field.left() - label.right() };
            let nearest = h
                .query_all_by_role(Role::SpinButton)
                .map(|n| n.rect())
                .filter(|r| (r.center().y - label.center().y).abs() < 12.0 && gap(r) >= -0.5)
                .min_by(|a, b| gap(a).total_cmp(&gap(b)))
                .unwrap_or_else(|| panic!("{language} {caption}: no field beside {label:?}"));
            assert!(gap(&nearest) < 30.0, "{language} {caption}: the field {nearest:?} is next to its caption {label:?}");
        }
        let left = h.get_by_role_and_label(Role::Label, &tr("Left:")).rect();
        let bottom = h.get_by_role_and_label(Role::Label, &tr("Bottom:")).rect();
        let width = h.get_by_role_and_label(Role::Label, &tr("Width:")).rect();
        assert!((left.center().y - bottom.center().y).abs() < 2.0, "{language}: Left and Bottom share a row");
        assert!(width.center().y > left.center().y + 10.0, "{language}: Width is on the next row");
        if language == "ar" {
            assert!(left.center().x > bottom.center().x, "ar: Left is in the first column, at the right");
        } else {
            assert!(left.center().x < bottom.center().x, "en: Left is in the first column, at the left");
        }
        assert_on_screen(&h, &format!("{language} field position"));
        let ok = if language == "ar" { ar("OK") } else { "OK" };
        right_edges.push(painted_text(&h).into_iter().find(|(text, _)| text == ok).map(|(_, r)| r.right()));
    }
    let (ar_edge, en_edge) = (right_edges[0].expect("ar OK"), right_edges[1].expect("en OK"));
    assert!((ar_edge - en_edge).abs() < 40.0, "the dialog ends at {ar_edge} in Arabic and at {en_edge} in English");
}

/// The View menu's section labels ("Zoom", "Page navigation") are at the start edge like its
/// entries: the right edges of the label and of the entry below it line up in Arabic, and their
/// left edges in English.
#[test]
fn a_menu_section_label_is_flush_with_the_entries_below_it() {
    for language in ["ar", "en"] {
        let tr = |english: &str| if language == "ar" { ar(english).to_owned() } else { english.to_owned() };
        let mut h = harness(language, |_| {});
        h.get_by_role_and_label(Role::Button, &tr("Menu")).click();
        h.run_steps(4);
        h.get_by_role_and_label(Role::Button, &format!("{} \u{23F5}", tr("View"))).click();
        h.run_steps(6);
        let shown = painted_text(&h);
        let rect = |text: &str| shown.iter().find(|(t, _)| t == text).unwrap_or_else(|| panic!("{language}: {text:?} is painted: {shown:?}")).1;
        let (section, entry) = (rect(&tr("Zoom")), rect(&tr("Actual size")));
        if language == "ar" {
            assert!((section.right() - entry.right()).abs() < 8.0, "ar: the section label {section:?} ends where the entry {entry:?} ends");
        } else {
            assert!((section.left() - entry.left()).abs() < 8.0, "en: the section label {section:?} starts where the entry {entry:?} starts");
        }
        assert_on_screen(&h, &format!("{language} view menu"));
    }
}

/// The rectangle of the text `text` painted this frame.
fn painted_rect(h: &Harness<'static, PdfCraftApp>, language: &str, text: &str) -> egui::Rect {
    let shown = painted_text(h);
    shown.iter().find(|(t, _)| t == text).unwrap_or_else(|| panic!("{language}: {text:?} is painted: {shown:?}")).1
}

/// A submenu opens beside its entry, on the side the entry's arrow points to: the left of it in
/// Arabic (the menu is at the right edge and the entries read from the right), the right of it in
/// English. A submenu of a submenu continues the same way. egui's own submenu always asked for
/// the right, and only moved to the left where the window had no room.
#[test]
fn a_submenu_opens_on_the_side_its_arrow_points_to() {
    for language in ["ar", "en"] {
        let tr = |english: &str| if language == "ar" { ar(english).to_owned() } else { english.to_owned() };
        let mut h = harness(language, |app| {
            app.recent.push(pdfcraft_ui_egui::RecentFile { name: "recent.pdf".into(), path: "/nonexistent/recent.pdf".into(), pages: 1, size: 0 });
        });
        h.get_by_role_and_label(Role::Button, &tr("Menu")).click();
        h.run_steps(4);
        let beside = |inner: egui::Rect, entry: egui::Rect, what: &str| {
            if language == "ar" {
                assert!(inner.right() <= entry.left(), "ar: the {what} submenu ({inner:?}) is left of its entry {entry:?}");
            } else {
                assert!(inner.left() >= entry.right(), "en: the {what} submenu ({inner:?}) is right of its entry {entry:?}");
            }
        };

        // File, opened by a click: its first entry is beside the File entry, not under it.
        let file_name = format!("{} \u{23F5}", tr("File"));
        let file = h.get_by_role_and_label(Role::Button, &file_name).rect();
        h.get_by_role_and_label(Role::Button, &file_name).click();
        h.run_steps(6);
        beside(painted_rect(&h, language, &tr("Open…")), file, "File");
        assert_on_screen(&h, &format!("{language} file submenu"));

        // Open Recent inside it, opened by hovering: the recent file is beside that entry.
        let recent_name = format!("{} \u{23F5}", tr("Open Recent"));
        let recent = h.get_by_role_and_label(Role::Button, &recent_name);
        let recent_rect = recent.rect();
        recent.hover();
        h.run_steps(8);
        beside(painted_rect(&h, language, "recent.pdf"), recent_rect, "Open Recent");
        assert_on_screen(&h, &format!("{language} open recent submenu"));
    }
}

/// A one-page document with `count` bookmarks called "Chapter 1" to "Chapter {count}".
fn many_bookmarks_fixture(count: usize) -> Vec<u8> {
    let mut pdf = format!(
        "%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >> endobj
3 0 obj << /Type /Page /Parent 2 0 R >> endobj
4 0 obj << /Type /Outlines /First 5 0 R /Last {} 0 R /Count {count} >> endobj
",
        4 + count
    );
    for i in 1..=count {
        let (prev, next) = (
            if i > 1 { format!("/Prev {} 0 R", 4 + i - 1) } else { String::new() },
            if i < count { format!("/Next {} 0 R", 4 + i + 1) } else { String::new() },
        );
        pdf.push_str(&format!("{} 0 obj << /Title (Chapter {i}) /Parent 4 0 R /Dest [3 0 R /Fit] {prev} {next} >> endobj\n", 4 + i));
    }
    pdf.push_str("trailer << /Root 1 0 R >>\n%%EOF");
    pdf.into_bytes()
}

/// The scrollbar of a list that outgrows its pane is at the start edge of the pane: the left in
/// Arabic (egui puts its own at the right, so the panel draws a bar of its own), the right in
/// English, where it is egui's.
#[test]
fn the_scrollbar_of_a_long_list_is_at_the_start_edge_of_its_pane() {
    for language in ["ar", "en"] {
        let mut h = Harness::builder().with_size(egui::vec2(WIDTH, 600.0)).build_eframe(move |_cc| {
            let mut app = PdfCraftApp::new();
            app.set_option("language", language).unwrap();
            app.open_bytes("long.pdf", None, many_bookmarks_fixture(80)).expect("opens");
            app.set_option("left", "closed").unwrap();
            app.set_option("panel", "bookmarks").unwrap();
            app
        });
        h.run_steps(8);
        let row = painted_rect(&h, language, "Chapter 1").center().x;
        let bars: Vec<egui::Rect> = h.query_all_by_role(Role::ScrollBar).map(|n| n.rect()).filter(|r| r.height() > 100.0).collect();
        // The document view has a bar of its own, at its start edge (the left in Arabic, just right
        // of the pane, which is x 48 to 378 there; the right in English, where the pane is the right
        // 330 points). The pane has a bar on its start side only.
        let in_pane = |r: &&egui::Rect| if language == "ar" { r.center().x < 378.0 } else { r.center().x > 1030.0 };
        let in_pane: Vec<egui::Rect> = bars.iter().filter(in_pane).copied().collect();
        if language == "ar" {
            assert!(in_pane.iter().any(|r| r.center().x < row), "ar: a bar left of the rows (centre {row}): {bars:?}");
            assert!(in_pane.iter().all(|r| r.center().x < row), "ar: and none at their right: {bars:?}");
        } else {
            assert!(in_pane.iter().any(|r| r.center().x > row), "en: a bar right of the rows (centre {row}): {bars:?}");
        }
        assert_on_screen(&h, &format!("{language} long list"));
    }
}

/// The vertical and the horizontal bar of the document view: the widest horizontal bar (the
/// view's own) and the vertical bars whose centre lies within its width.
fn document_bars(h: &Harness<'static, PdfCraftApp>) -> (Vec<egui::Rect>, egui::Rect) {
    let bars: Vec<egui::Rect> = h.query_all_by_role(Role::ScrollBar).map(|n| n.rect()).collect();
    let horizontal = bars
        .iter()
        .filter(|r| r.height() < 24.0 && r.width() > 300.0)
        .max_by(|a, b| a.width().total_cmp(&b.width()))
        .copied()
        .unwrap_or_else(|| panic!("no horizontal bar for the document view among {bars:?}"));
    let vertical = bars.iter().filter(|r| r.width() < 24.0 && r.height() > 300.0 && horizontal.x_range().contains(r.center().x)).copied().collect();
    (vertical, horizontal)
}

/// The document view's vertical scrollbar is at the left edge of the view when the interface is
/// mirrored (egui puts its own at the right, so the canvas draws the bar), and at the right edge
/// in English, where it is egui's. The horizontal bar is at the bottom in both.
#[test]
fn the_document_views_scrollbar_is_at_the_start_edge() {
    for language in ["ar", "en"] {
        // Three times the size: the page is taller and wider than the view.
        let h = harness(language, |app| {
            app.set_option("left", "closed").unwrap();
            app.set_option("zoom", "300").unwrap();
        });
        let (vertical, horizontal) = document_bars(&h);
        assert_eq!(vertical.len(), 1, "{language}: one vertical bar for the view among {vertical:?} (horizontal {horizontal:?})");
        let bar = vertical[0];
        if language == "ar" {
            assert!((bar.left() - horizontal.left()).abs() < 2.0, "ar: the bar {bar:?} is at the left edge of the view {horizontal:?}");
        } else {
            assert!((bar.right() - horizontal.right()).abs() < 2.0, "en: the bar {bar:?} is at the right edge of the view {horizontal:?}");
        }
        assert!(horizontal.top() > bar.center().y, "{language}: the horizontal bar {horizontal:?} runs along the bottom");
    }
}

/// Scrolling the document view works from the bar at its left edge: dragging the handle down moves
/// the page up, and the horizontal bar still scrolls sideways.
#[test]
fn the_mirrored_document_view_scrolls_from_its_bars() {
    let mut h = harness("ar", |app| {
        app.set_option("left", "closed").unwrap();
        app.set_option("zoom", "300").unwrap();
    });
    // How far the page has moved up and to the left from where it started.
    let start = h.state().views[0].page_screen_rect(0).expect("page 1 on screen").min;
    let moved = |h: &Harness<'static, PdfCraftApp>| start - h.state().views[0].page_screen_rect(0).expect("page 1 on screen").min;
    let (vertical, horizontal) = document_bars(&h);
    let bar = vertical[0];
    let from = egui::pos2(bar.center().x, bar.top() + 12.0);
    let to = from + egui::vec2(0.0, 80.0);
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(2);
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(2);
    for k in 1..=4 {
        h.event(egui::Event::PointerMoved(from.lerp(to, k as f32 / 4.0)));
        h.run_steps(2);
    }
    h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
    assert!(moved(&h).y > 50.0, "the left bar scrolled the page: {:?}", moved(&h));
    // Sideways, along the bottom bar from its left end.
    let from = egui::pos2(horizontal.left() + 12.0, horizontal.center().y);
    let to = from + egui::vec2(60.0, 0.0);
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(2);
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(2);
    for k in 1..=4 {
        h.event(egui::Event::PointerMoved(from.lerp(to, k as f32 / 4.0)));
        h.run_steps(2);
    }
    h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
    assert!(moved(&h).x > 20.0, "the bottom bar scrolled the page sideways: {:?}", moved(&h));
}

/// The rect of the tab called `name` on the tab strip.
fn tab_rect(h: &Harness<'static, PdfCraftApp>, name: &str) -> egui::Rect {
    h.get_by_role_and_label(Role::Tab, name).rect()
}

/// Three documents: `rtl.pdf` (the one every harness opens) first, then `middle.pdf` and `last.pdf`.
fn three_documents(language: &'static str) -> Harness<'static, PdfCraftApp> {
    harness(language, |app| {
        for name in ["middle.pdf", "last.pdf"] {
            app.open_bytes(name, None, fixture()).unwrap();
        }
    })
}

fn drag_tab(h: &mut Harness<'static, PdfCraftApp>, name: &str, to: egui::Pos2) {
    let from = tab_rect(h, name).center();
    h.event(egui::Event::PointerMoved(from));
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    for k in 1..=6 {
        h.event(egui::Event::PointerMoved(from.lerp(to, k as f32 / 6.0)));
        h.run_steps(1);
    }
    h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
}

/// The first document is the rightmost tab when the interface is mirrored, the leftmost in
/// English; the order of the documents is the same in both, and the Combine tab comes after the
/// last document (at the far end of the strip).
#[test]
fn the_first_document_tab_is_at_the_start_edge() {
    for language in ["ar", "en"] {
        let mut h = three_documents(language);
        let ids: Vec<_> = h.state().views.iter().map(|view| view.id).collect();
        let (first, middle, last) = (tab_rect(&h, "rtl.pdf"), tab_rect(&h, "middle.pdf"), tab_rect(&h, "last.pdf"));
        if language == "ar" {
            assert!(first.left() > middle.right() && middle.left() > last.right(), "ar: first {first:?}, middle {middle:?}, last {last:?}");
            assert!(first.center().x < button_x(&h, ar("Home")), "ar: the tabs follow Home, to its left");
        } else {
            assert!(first.right() < middle.left() && middle.right() < last.left(), "en: first {first:?}, middle {middle:?}, last {last:?}");
        }
        h.state_mut().execute("page.combine");
        h.run_steps(3);
        let (last, combine) = (tab_rect(&h, "last.pdf"), tab_rect(&h, i18n::tr(Lang::from_code(language).unwrap(), "Combine files")));
        assert_eq!(combine.center().x < last.center().x, language == "ar", "{language}: Combine after the last document: {combine:?} / {last:?}");
        assert_eq!(h.state().views.iter().map(|view| view.id).collect::<Vec<_>>(), ids, "{language}: the documents keep their order");
    }
}

/// Dragging a tab in the mirrored strip drops it where the pointer lets go, reading from the
/// right: the first document dragged to the left end becomes the last.
#[test]
fn a_dragged_tab_drops_where_the_mirrored_strip_reads() {
    let mut h = three_documents("ar");
    let ids: Vec<_> = h.state().views.iter().map(|view| view.id).collect();
    // rtl.pdf (rightmost) past the last one at the left: the end of the order.
    let to = tab_rect(&h, "last.pdf").left_center() - egui::vec2(12.0, 0.0);
    drag_tab(&mut h, "rtl.pdf", to);
    assert_eq!(h.state().views.iter().map(|view| view.id).collect::<Vec<_>>(), [ids[1], ids[2], ids[0]]);
    let (first, middle, last) = (tab_rect(&h, "rtl.pdf"), tab_rect(&h, "middle.pdf"), tab_rect(&h, "last.pdf"));
    assert!(
        middle.left() > last.right() && last.left() > first.right(),
        "the strip draws the new order from the right: {middle:?} {last:?} {first:?}"
    );
    // last.pdf, now in the middle, past the right end: the front.
    let to = tab_rect(&h, "middle.pdf").right_center() + egui::vec2(12.0, 0.0);
    drag_tab(&mut h, "last.pdf", to);
    assert_eq!(h.state().views.iter().map(|view| view.id).collect::<Vec<_>>(), [ids[2], ids[1], ids[0]]);
    assert!(tab_rect(&h, "last.pdf").left() > tab_rect(&h, "middle.pdf").right());
    // A tab dropped where it was stays.
    let middle = tab_rect(&h, "middle.pdf");
    drag_tab(&mut h, "middle.pdf", middle.center() - egui::vec2(10.0, 0.0));
    assert_eq!(h.state().views.iter().map(|view| view.id).collect::<Vec<_>>(), [ids[2], ids[1], ids[0]]);
}

fn enabled(h: &Harness<'static, PdfCraftApp>, label: &str) -> Option<bool> {
    h.query_by_label(label)?;
    Some(h.query_by(|n| n.label().as_deref() == Some(label) && n.is_disabled()).is_none())
}

/// Tabs that outgrow the strip scroll with arrows, as in English: the arrows keep their physical
/// sides, the strip starts at its right end (the first document), and the newest tab, at the left
/// end, is brought into view when it opens.
#[test]
fn a_mirrored_strip_that_overflows_scrolls_with_its_arrows() {
    fn name(i: usize) -> String {
        format!("Statement {i} for the office records, scanned.pdf")
    }
    let mut h = harness("ar", |app| {
        for i in 0..7 {
            app.open_bytes(&name(i), None, fixture()).unwrap();
        }
    });
    h.run_steps(6);
    let (left, right) = (ar("Scroll tabs left"), ar("Scroll tabs right"));
    let open = button_x(&h, ar("Open"));
    let (left_x, right_x) = (button_x(&h, left), button_x(&h, right));
    assert!(left_x < open && open < right_x - 20.0 || open < left_x, "the arrows are beside the tabs: {left_x} {open} {right_x}");
    assert!(left_x < right_x, "the left arrow is at the left of the tabs, the right one at their right");
    // The newest document (the last, at the left end) is active and in sight, so only the right
    // arrow can go further: more tabs lie to the right.
    assert_eq!((enabled(&h, left), enabled(&h, right)), (Some(false), Some(true)));
    let strip = (left_x, right_x);
    let newest = tab_rect(&h, &name(6));
    assert!(newest.center().x > strip.0 && newest.center().x < strip.1, "the newest tab {newest:?} is in sight between the arrows {strip:?}");
    let before = tab_rect(&h, &name(0));
    h.get_by_label(right).click();
    h.run_steps(4);
    assert!(tab_rect(&h, &name(0)).left() < before.left(), "the right arrow scrolls the strip towards the first documents");
    assert_eq!(enabled(&h, left), Some(true), "and then there is more to the left");
    // Back at the left end.
    for _ in 0..8 {
        if enabled(&h, left) == Some(true) {
            h.get_by_label(left).click();
            h.run_steps(4);
        }
    }
    assert_eq!(enabled(&h, left), Some(false));
}

/// The mirrored strip stays at its right end, where the first documents are, when the window gets
/// narrower: its first tab stays beside the right arrow instead of sliding out of the strip.
#[test]
fn a_mirrored_strip_keeps_its_first_tabs_in_sight_when_the_window_narrows() {
    let mut h = harness("ar", |app| {
        for i in 0..7 {
            app.open_bytes(&format!("Statement {i} for the office records, scanned.pdf"), None, fixture()).unwrap();
        }
        // The first document is active, at the right end of the strip.
        app.active = Some(0);
    });
    h.run_steps(6);
    let beside_the_arrow = |h: &Harness<'static, PdfCraftApp>| {
        let (first, arrow) = (tab_rect(h, "rtl.pdf"), h.get_by_role_and_label(Role::Button, ar("Scroll tabs right")).rect());
        (arrow.left() - first.right() < 12.0 && first.right() <= arrow.left() + 0.5, first, arrow)
    };
    let (beside, first, arrow) = beside_the_arrow(&h);
    assert!(beside, "the first tab {first:?} is beside the right arrow {arrow:?}");
    h.set_size(egui::vec2(WIDTH - 150.0, 900.0));
    h.run_steps(8);
    let (beside, first, arrow) = beside_the_arrow(&h);
    assert!(beside, "after narrowing: the first tab {first:?} is beside the right arrow {arrow:?}");
}

/// A document with `fields` on its first page.
fn form_with(fields: &[(&str, [f64; 4], NewField)]) -> Vec<u8> {
    let mut s = Session::new();
    let bytes = s.create_blank(300.0, 400.0, 1).unwrap();
    let id = s.open("form.pdf", None, bytes, None).unwrap();
    for (name, rect, kind) in fields {
        s.apply(id, Edit::AddField { name: Some((*name).into()), page: 0, rect: *rect, kind: kind.clone() }).unwrap();
    }
    s.save_bytes(id).unwrap().as_ref().clone()
}

/// The shared field properties are a form: the checkbox that names a property is at the right of
/// its editor, and the "Mixed" tag at the left of the editor; English keeps the three columns.
#[test]
fn the_shared_field_properties_run_from_the_start_edge() {
    for language in ["ar", "en"] {
        let mut h = Harness::builder().with_size(egui::vec2(WIDTH, 900.0)).build_eframe(move |_cc| {
            let mut app = PdfCraftApp::new();
            app.set_option("language", language).unwrap();
            app.open_bytes(
                "form.pdf",
                None,
                form_with(&[
                    ("alpha", [40.0, 290.0, 230.0, 314.0], NewField::Text { multiline: false }),
                    ("beta", [40.0, 240.0, 230.0, 264.0], NewField::Text { multiline: false }),
                ]),
            )
            .unwrap();
            app.execute("form.prepare");
            app
        });
        h.run_steps(4);
        // Two fields with different tooltips: Tooltip is mixed.
        for (name, tip) in [("alpha", "Help for alpha"), ("beta", "Help for beta")] {
            let s = h.state_mut();
            let id = s.views[0].id;
            s.session
                .apply(
                    id,
                    Edit::SetFieldProps {
                        name: name.into(),
                        props: Box::new(pdfcraft_engine::FieldProps { tooltip: Some(tip.into()), ..Default::default() }),
                    },
                )
                .unwrap();
        }
        let p = &mut h.state_mut().views[0].prepare;
        p.selected = Some(("alpha".into(), 0));
        p.also = vec![("beta".into(), 0)];
        h.state_mut().execute("form.field.properties");
        h.run_steps(6);
        let tooltip = h.get_by_role_and_label(Role::CheckBox, i18n::tr(Lang::from_code(language).unwrap(), "Tooltip:")).rect();
        let editor = h
            .query_all_by_role(Role::TextInput)
            .map(|n| n.rect())
            .find(|r| (r.center().y - tooltip.center().y).abs() < 20.0 && r.width() > 50.0)
            .unwrap_or_else(|| {
                let inputs: Vec<_> = h.query_all_by_role(Role::TextInput).map(|n| n.rect()).collect();
                panic!("{language}: no editor on the Tooltip row {tooltip:?} among {inputs:?}")
            });
        let mixed = h.get_by_label(i18n::tr(Lang::from_code(language).unwrap(), "Mixed")).rect();
        if language == "ar" {
            assert!(tooltip.center().x > editor.center().x, "ar: the checkbox {tooltip:?} is at the right of its editor {editor:?}");
            assert!(mixed.center().x < editor.center().x, "ar: the Mixed tag {mixed:?} is at the left of the editor {editor:?}");
        } else {
            assert!(tooltip.center().x < editor.center().x && editor.center().x < mixed.center().x, "en: {tooltip:?} {editor:?} {mixed:?}");
        }
        assert_on_screen(&h, &format!("{language} shared field properties"));
    }
}

/// The calendar of a date field runs from the start edge: Sunday is the rightmost column and the
/// arrow at the left goes to the next month when mirrored; English is as it was.
#[test]
fn the_calendar_runs_from_the_start_edge() {
    for language in ["ar", "en"] {
        let mut h = Harness::builder().with_size(egui::vec2(WIDTH, 900.0)).build_eframe(move |_cc| {
            let mut app = PdfCraftApp::new();
            app.set_option("language", language).unwrap();
            app.open_bytes("form.pdf", None, form_with(&[("due", [50.0, 40.0, 200.0, 60.0], NewField::Date)])).unwrap();
            app.set_option("left", "closed").unwrap();
            app.set_option("zoom", "150").unwrap();
            app
        });
        for _ in 0..40 {
            h.run_steps(2);
            if !h.state().render_pending() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let point = {
            let s = h.state();
            let doc = s.session.get(s.views[0].id).unwrap();
            let f = doc.form.iter().find(|f| f.name == "due").unwrap();
            pdfcraft_ui_egui::forms_ui::field_screen_rect(&s.views[0], &doc.info, f, 0).expect("on screen").center()
        };
        h.hover_at(point);
        h.run_steps(1);
        h.drag_at(point);
        h.run_steps(1);
        h.drop_at(point);
        h.run_steps(3);
        let lang = Lang::from_code(language).unwrap();
        let weekday = |english: &str| h.get_by_label(i18n::tr_ctx(lang, "calendar weekday", english)).rect().center().x;
        let (sunday, saturday) = (weekday("Su"), weekday("Sa"));
        assert_eq!(sunday > saturday, language == "ar", "{language}: Sunday {sunday}, Saturday {saturday}");
        // The whole week reads from the start edge.
        let days = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"].map(weekday);
        assert!(days.windows(2).all(|pair| (pair[0] < pair[1]) == (language == "en")), "{language}: the week runs {days:?}");
        let (y, m, _) = h.state().session.today();
        // The arrow at the left of the month name goes to the earlier month in English and to the
        // later one when mirrored.
        let title = |month: u32, year: i64| {
            const MONTHS: [&str; 12] =
                ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
            let name = MONTHS.get((month - 1) as usize).copied().unwrap_or("January");
            i18n::fmt(i18n::tr(lang, "{month} {y}"), &[("month", i18n::tr(lang, name)), ("y", &year.to_string())])
        };
        h.get_by_label(&title(m, y));
        let (later, earlier) = (if m == 12 { (1, y + 1) } else { (m + 1, y) }, if m == 1 { (12, y - 1) } else { (m - 1, y) });
        let (later, earlier) = (title(later.0, later.1), title(earlier.0, earlier.1));
        let (left_goes, right_goes) = if language == "ar" { (&later, &earlier) } else { (&earlier, &later) };
        h.get_by_label("‹").click();
        h.run_steps(3);
        h.get_by_label(left_goes);
        h.get_by_label("›").click();
        h.run_steps(3);
        h.get_by_label(&title(m, y));
        h.get_by_label("›").click();
        h.run_steps(3);
        h.get_by_label(right_goes);
    }
}

/// The About window open at its `tab` ("Contributors" or "Models").
fn about_tab(language: &'static str, tab: &str) -> Harness<'static, PdfCraftApp> {
    let mut h = harness(language, |app| app.set_option("dialog", "about").unwrap());
    h.run_steps(4);
    let lang = Lang::from_code(language).unwrap();
    h.get_by_label(i18n::tr(lang, tab)).click();
    h.run_steps(4);
    h
}

/// The model table's first column, the company, is at the right when mirrored; English is as it was.
#[test]
fn the_model_table_runs_from_the_start_edge() {
    for language in ["ar", "en"] {
        let h = about_tab(language, "Models");
        let lang = Lang::from_code(language).unwrap();
        let x = |english: &str| h.get_by_role_and_label(Role::Label, i18n::tr(lang, english)).rect().center().x;
        let (company, model, version, lines) = (x("Company"), x("Model"), x("Version"), x("Lines +/−"));
        let in_order = company < model && model < version && version < lines;
        let reversed = company > model && model > version && version > lines;
        assert_eq!((in_order, reversed), (language == "en", language == "ar"), "{language}: {company} {model} {version} {lines}");
        assert_on_screen(&h, &format!("{language} models"));
    }
}

/// The same for the contributor table: its name column is at the right, a click on a header still
/// sorts, and the names are right-aligned in their column.
#[test]
fn the_contributor_table_runs_from_the_start_edge() {
    for language in ["ar", "en"] {
        let mut h = about_tab(language, "Contributors");
        let lang = Lang::from_code(language).unwrap();
        h.get_by_label(i18n::tr(lang, "Table")).click();
        h.run_steps(4);
        let button = |h: &Harness<'static, PdfCraftApp>, english: &str| h.get_by_role_and_label(Role::Button, i18n::tr(lang, english)).rect();
        let (name, prs, commits) = (button(&h, "Name"), button(&h, "PRs"), button(&h, "Commits"));
        let in_order = name.center().x < prs.center().x && prs.center().x < commits.center().x;
        let reversed = name.center().x > prs.center().x && prs.center().x > commits.center().x;
        assert_eq!((in_order, reversed), (language == "en", language == "ar"), "{language}: {name:?} {prs:?} {commits:?}");
        // The first contributor's link is in the name column.
        let link = h.query_all_by_role(Role::Link).next().expect("a contributor link").rect();
        assert_eq!(link.center().x > prs.center().x, language == "ar", "{language}: link {link:?} against the PRs column {prs:?}");
        // Sorting by a header still works from the mirrored header, and flips its arrow.
        h.get_by_role_and_label(Role::Button, i18n::tr(lang, "Name")).click();
        h.run_steps(3);
        h.get_by_role_and_label(Role::Button, &format!("{} ▲", i18n::tr(lang, "Name")));
        assert_on_screen(&h, &format!("{language} contributors"));
    }
}

/// Every vertical scrollbar on screen taller than 100 points: egui's own and the ones the panels
/// and areas draw themselves when the interface is mirrored.
fn tall_bars(h: &Harness<'static, PdfCraftApp>) -> Vec<egui::Rect> {
    h.query_all_by_role(Role::ScrollBar).map(|n| n.rect()).filter(|r| r.width() < 24.0 && r.height() > 100.0).collect()
}

/// A harness on the app `setup` builds, in `language`, `size` big, after a few frames.
fn bare(language: &'static str, size: egui::Vec2, setup: impl FnOnce(&mut PdfCraftApp) + 'static) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(size).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", language).unwrap();
        setup(&mut app);
        app
    });
    h.run_steps(8);
    h
}

/// The scroll area of `region` (the part of the screen it fills) has its one vertical bar against
/// the region's start edge: the left when mirrored (and no bar at its right), the right in English.
fn assert_bar_at_start_edge(language: &str, region: egui::Rect, bars: &[egui::Rect], what: &str) {
    let inside: Vec<egui::Rect> = bars.iter().filter(|b| region.contains_rect(**b)).copied().collect();
    assert_eq!(inside.len(), 1, "{language} {what}: one vertical bar in {region:?} among {bars:?}");
    let bar = inside[0];
    if language == "ar" {
        assert!((bar.left() - region.left()).abs() < 12.0, "ar {what}: the bar {bar:?} is at the left edge of {region:?}");
        assert!(bar.right() < region.center().x, "ar {what}: and is a thin strip: {bar:?}");
    } else {
        assert!((bar.right() - region.right()).abs() < 12.0, "en {what}: the bar {bar:?} is at the right edge of {region:?}");
    }
}

/// The Home screen's scrollbar is at the start edge of the screen: the left in Arabic (egui puts
/// its own at the right, so the screen draws a bar of its own), the right in English.
#[test]
fn the_home_screens_scrollbar_is_at_the_start_edge() {
    /// The width of the tools pane, on the right when mirrored and on the left in English.
    const TOOLS: f32 = 272.0;
    for language in ["ar", "en"] {
        // Short enough that the welcome page outgrows the window.
        let h = bare(language, egui::vec2(WIDTH, 300.0), |_| {});
        let home = if language == "ar" {
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(WIDTH - TOOLS, 300.0))
        } else {
            egui::Rect::from_min_max(egui::pos2(TOOLS, 0.0), egui::pos2(WIDTH, 300.0))
        };
        assert_bar_at_start_edge(language, home, &tall_bars(&h), "home");
    }
}

/// The Combine grid's scrollbar is at the start edge of the grid: the left in Arabic (the grid
/// draws a bar of its own), the right in English.
#[test]
fn the_combine_grids_scrollbar_is_at_the_start_edge() {
    for language in ["ar", "en"] {
        let h = bare(language, egui::vec2(WIDTH, 700.0), |app| {
            // Far more cards than fit: several rows of them.
            let files: Vec<(String, Vec<u8>)> = (1..=60).map(|i| (format!("file-{i}.pdf"), fixture())).collect();
            app.use_files(pdfcraft_ui_egui::FilePurpose::Combine, files);
        });
        // The Combine page is inset 28 points from the sides of the screen; the tools pane is
        // 272 wide, on the right when mirrored and on the left in English.
        let grid = if language == "ar" {
            egui::Rect::from_min_max(egui::pos2(28.0, 0.0), egui::pos2(WIDTH - 272.0, 700.0))
        } else {
            egui::Rect::from_min_max(egui::pos2(272.0, 0.0), egui::pos2(WIDTH - 28.0, 700.0))
        };
        assert_bar_at_start_edge(language, grid, &tall_bars(&h), "combine grid");
    }
}

/// A dialog's scrollbar is at the start side of the dialog: the left of the Keyboard Shortcuts
/// list in Arabic (the list draws a bar of its own, and no egui bar is at its right), the right
/// of it in English.
#[test]
fn a_dialogs_scrollbar_is_at_the_start_edge() {
    for language in ["ar", "en"] {
        let h = harness(language, |app| app.set_option("dialog", "shortcuts").unwrap());
        let card: egui::Rect = h.ctx.data(|d| d.get_temp(egui::Id::new("dialog").with("card"))).expect("the dialog is open");
        let bars: Vec<egui::Rect> = tall_bars(&h).into_iter().filter(|b| card.contains_rect(*b)).collect();
        assert_eq!(bars.len(), 1, "{language}: one vertical bar in the dialog {card:?} among {bars:?}");
        let bar = bars[0];
        if language == "ar" {
            // The card's padding is 20 points.
            assert!(bar.left() - card.left() < 32.0, "ar: the bar {bar:?} is against the left edge of the dialog {card:?}");
            assert!(bar.center().x < card.center().x, "ar: and not at its right");
        } else {
            assert!(bar.center().x > card.center().x, "en: the bar {bar:?} is in the right half of the dialog {card:?}");
        }
        assert_on_screen(&h, &format!("{language} shortcuts"));
    }
}

/// A document of `pages` empty pages, enough for the Organize grid to scroll.
fn many_pages(pages: usize) -> Vec<u8> {
    let kids: String = (0..pages).map(|i| format!("{} 0 R ", i + 3)).collect();
    let objects: String = (0..pages).map(|i| format!("{} 0 obj << /Type /Page /Parent 2 0 R >> endobj\n", i + 3)).collect();
    format!(
        "%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [{kids}] /Count {pages} /MediaBox [0 0 300 400] >> endobj
{objects}trailer << /Root 1 0 R >>
%%EOF"
    )
    .into_bytes()
}

/// The Organize page grid's scrollbar is at the start edge of the grid: the left in Arabic (the
/// grid draws a bar of its own, as the document view does), the right in English.
#[test]
fn the_organize_grids_scrollbar_is_at_the_start_edge() {
    /// The width of the tools pane, on the right when mirrored and on the left in English.
    const TOOLS: f32 = 272.0;
    for language in ["ar", "en"] {
        // Short, with far more pages than fit: several rows of cards.
        let h = bare(language, egui::vec2(WIDTH, 400.0), |app| {
            app.open_bytes("pages.pdf", None, many_pages(40)).expect("opens");
            app.set_option("left", "closed").unwrap();
            app.set_option("tool", "organize").unwrap();
        });
        // The rail stands between the window edge and the grid: at the left in Arabic, at the
        // right in English.
        let rail = h.get_by_role_and_label(Role::Button, i18n::tr(Lang::from_code(language).unwrap(), "Comments")).rect();
        let grid = if language == "ar" {
            egui::Rect::from_min_max(egui::pos2(rail.right(), 0.0), egui::pos2(WIDTH - TOOLS, 400.0))
        } else {
            egui::Rect::from_min_max(egui::pos2(TOOLS, 0.0), egui::pos2(rail.left(), 400.0))
        };
        assert_bar_at_start_edge(language, grid, &tall_bars(&h), "organize grid");
    }
}
