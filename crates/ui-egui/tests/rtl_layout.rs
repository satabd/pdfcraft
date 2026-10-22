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
