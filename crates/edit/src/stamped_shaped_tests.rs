//! Arabic in header & footer, Bates numbers and watermarks (`stamped`). Most tests run with the
//! stand-in for the craft-fonts Arabic face of `text_shaped_tests`, so they run in any build; the
//! one at the end uses the real face and skips without it.

use pdfcraft_content::Op;
use pdfcraft_cos::{Dict, Document, Object, SaveOptions, write_full};
use pdfcraft_fonts::helvetica_width;

use crate::tests::{cx, fixture, reopen, streams};
use crate::text_shaped_tests::{NO_FACE, Using, fake};
use crate::*;

fn bytes_of(doc: &Document) -> Vec<u8> {
    write_full(doc, &SaveOptions::default()).expect("saves")
}

/// The operators of the last content stream of `page` (the mark, when it was drawn on top).
fn mark_ops(doc: &Document, page: usize) -> Vec<Op> {
    let p = pdfcraft_model::pages(doc).swap_remove(page);
    let contents = doc.resolve(p.dict.get(b"Contents").expect("contents"));
    let last = contents.as_array().and_then(|a| a.last()).expect("a content list").clone();
    pdfcraft_content::parse(&stream_bytes(doc, &last).expect("stream")).ops
}

fn page_fonts(doc: &Document, page: usize) -> Dict {
    let p = pdfcraft_model::pages(doc).swap_remove(page);
    let res = doc.resolve(p.dict.get(b"Resources").expect("resources"));
    let fonts = doc.resolve(res.as_dict().expect("resources dict").get(b"Font").expect("fonts"));
    fonts.as_dict().expect("font dict").clone()
}

fn shaped_font_names(doc: &Document, page: usize) -> Vec<String> {
    page_fonts(doc, page).iter().filter(|(n, _)| n.starts_with(b"PCStAr")).map(|(n, _)| String::from_utf8_lossy(n).into_owned()).collect()
}

fn actual_texts(ops: &[Op]) -> Vec<String> {
    ops.iter()
        .filter(|o| o.is("BDC"))
        .filter_map(|o| match o.operands.get(1) {
            Some(Object::Dict(d)) => match d.get(b"ActualText") {
                Some(Object::String(s)) => {
                    assert!(s.hex && s.bytes.starts_with(&[0xFE, 0xFF]), "UTF-16BE with a byte order mark, as a hex string");
                    Some(s.to_text())
                }
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// One line as drawn: where its text matrix puts it, how far it runs, and what each `Tj` showed.
#[derive(Debug)]
struct Drawn {
    x: f64,
    y: f64,
    extent: f64,
    /// Font resource and bytes of every string.
    shown: Vec<(String, Vec<u8>)>,
}

/// The lines (one per `Tm`) of `ops`, widths read from the fonts' own `/Widths` (and Helvetica's
/// metrics): independent of how the writer measured.
fn drawn_lines(doc: &Document, page: usize, ops: &[Op]) -> Vec<Drawn> {
    let fonts = page_fonts(doc, page);
    let mut lines: Vec<Drawn> = Vec::new();
    let (mut font, mut size) = (String::new(), 0.0);
    for op in ops {
        if op.is("Tm") {
            let m = op.nums::<6>().expect("six numbers");
            lines.push(Drawn { x: m[4], y: m[5], extent: 0.0, shown: Vec::new() });
        } else if op.is("Tf") {
            font = String::from_utf8_lossy(op.name(0).expect("a name")).into_owned();
            size = op.num(1).expect("a size");
        } else if op.is("Tj") {
            let Some(Object::String(s)) = op.operands.first() else { panic!("a string") };
            let line = lines.last_mut().expect("Tm first");
            let width = if font == "PCHelv" {
                helvetica_width(&s.bytes.iter().map(|b| char::from(*b)).collect::<String>(), size)
            } else {
                let f = doc.resolve(fonts.get(font.as_bytes()).unwrap_or_else(|| panic!("font {font} in the resources")));
                let widths = f.as_dict().and_then(|d| d.get(b"Widths")).map(|w| doc.resolve(w)).and_then(|w| w.as_array().cloned()).expect("Widths");
                s.bytes.iter().map(|code| widths.get(usize::from(*code) - 1).and_then(Object::as_f64).expect("a width") / 1000.0 * size).sum()
            };
            line.extent += width;
            line.shown.push((font.clone(), s.bytes.clone()));
        }
    }
    lines
}

fn hf_slot(slot: usize, text: &str) -> HeaderFooter {
    let mut hf = HeaderFooter::default();
    hf.text[slot] = text.into();
    hf
}

#[test]
fn arabic_in_the_right_slot_is_a_shaped_span_that_ends_at_the_margin() {
    let _face = fake();
    let mut doc = fixture();
    add_header_footer(&mut doc, &[0], &hf_slot(2, "مرحبا <<1>>"), false, &cx()).expect("drawn, not refused");
    let doc = reopen(&doc);
    let ops = mark_ops(&doc, 0);
    // One span holding the line in typing order (the page number expanded first).
    assert_eq!(actual_texts(&ops), ["مرحبا 1"]);
    let lines = drawn_lines(&doc, 0, &ops);
    assert_eq!(lines.len(), 1, "{lines:?}");
    let line = &lines[0];
    // Right slot: the line ends at the page width minus the right margin (600 - 72).
    assert!((line.x + line.extent - 528.0).abs() < 0.01, "{line:?}");
    // The Arabic is drawn from generated Type 3 fonts, the digit from Helvetica; nothing is `?`.
    assert!(line.shown.iter().any(|(f, _)| f.starts_with("PCStAr")), "{line:?}");
    assert!(line.shown.iter().filter(|(f, _)| f == "PCHelv").all(|(_, s)| s == b"1"), "{line:?}");
    let names = shaped_font_names(&doc, 0);
    assert!(!names.is_empty());
    for name in &names {
        let font = doc.resolve(page_fonts(&doc, 0).get(name.as_bytes()).expect("font"));
        let font = font.as_dict().expect("dict");
        assert_eq!(font.name(b"Subtype"), Some(b"Type3".as_slice()), "{name}");
        assert!(font.get(b"ToUnicode").is_some(), "{name}");
    }
    // It is a pagination artifact like any header.
    let content = streams(&doc, 0).pop().expect("mark");
    assert!(content.contains("/PCMark /HeaderFooter") && content.contains("/Artifact"), "{content}");
    assert_eq!(marks_present(&doc), [MarkKind::HeaderFooter]);
}

#[test]
fn left_and_centre_slots_and_lines_without_arabic_keep_their_places() {
    let _face = fake();
    let mut doc = fixture();
    let mut hf = HeaderFooter::default();
    hf.text[0] = "سري للغاية".into();
    hf.text[1] = "Page <<1>> مرحبا".into();
    // A footer of two lines: Latin first, Arabic second.
    hf.text[5] = "Confidential\nالسلام عليكم".into();
    add_header_footer(&mut doc, &[0], &hf, false, &cx()).expect("drawn");
    let doc = reopen(&doc);
    let ops = mark_ops(&doc, 0);
    assert_eq!(actual_texts(&ops), ["سري للغاية", "Page 1 مرحبا", "السلام عليكم"]);
    let lines = drawn_lines(&doc, 0, &ops);
    assert_eq!(lines.len(), 4, "{lines:?}");
    // Header: the left slot starts at the left margin, the centre slot is centred on the page.
    let top = 800.0 - 36.0 - 8.0 * 0.8;
    assert!(lines.iter().any(|l| (l.y - top).abs() < 0.01 && (l.x - 72.0).abs() < 0.01), "{lines:?}");
    let centre = lines.iter().find(|l| l.shown.iter().any(|(_, s)| s.starts_with(b"Page"))).expect("the centre line");
    assert!((centre.x + centre.extent / 2.0 - 300.0).abs() < 0.01, "{centre:?}");
    // The Latin footer line, right after an Arabic line in the drawing order, is in Helvetica
    // again (the text state held a Type 3 font when it started).
    let latin = lines.iter().find(|l| l.shown.iter().any(|(_, s)| s == b"Confidential")).expect("the Latin footer line");
    assert!(latin.shown.iter().all(|(f, _)| f == "PCHelv"), "{latin:?}");
    assert!((latin.x + latin.extent - 528.0).abs() < 0.01, "right footer slot: {latin:?}");
}

#[test]
fn a_bates_number_with_an_arabic_prefix_is_shaped_on_every_page_with_shared_fonts() {
    let _face = fake();
    let mut doc = fixture();
    let hf = hf_slot(5, "<<Bates Number#4#7#سري-#>>");
    assert_eq!(hf.first_undrawable(), None);
    add_header_footer(&mut doc, &[0, 1], &hf, false, &cx()).expect("drawn");
    let doc = reopen(&doc);
    assert_eq!(actual_texts(&mark_ops(&doc, 0)), ["سري-0007"]);
    assert_eq!(actual_texts(&mark_ops(&doc, 1)), ["سري-0008"]);
    // One set of fonts for the run; each page's resources name them.
    let (a, b) = (shaped_font_names(&doc, 0), shaped_font_names(&doc, 1));
    assert!(!a.is_empty() && a == b, "{a:?} {b:?}");
    for name in &a {
        assert_eq!(page_fonts(&doc, 0).get(name.as_bytes()), page_fonts(&doc, 1).get(name.as_bytes()), "{name}");
    }
    // The digits are Helvetica, the prefix is not.
    let lines = drawn_lines(&doc, 1, &mark_ops(&doc, 1));
    assert!(lines[0].shown.iter().any(|(f, s)| f == "PCHelv" && s == b"0008"), "{lines:?}");
    // The rotated page's mark is in display space like any other.
    let rotated = streams(&doc, 1).pop().expect("mark");
    assert!(rotated.contains("0 1 -1 0 600 0 cm"), "{rotated}");
}

#[test]
fn an_arabic_watermark_is_shaped_inside_its_rotation() {
    let _face = fake();
    let mut doc = fixture();
    let wm = Watermark { text: "سري".into(), ..Watermark::default() };
    assert_eq!(wm.first_undrawable(), None);
    add_watermark(&mut doc, &[0], &wm, false).expect("drawn");
    let doc = reopen(&doc);
    // Behind nothing: on top, after the original content's wrapper.
    let ops = mark_ops(&doc, 0);
    assert_eq!(actual_texts(&ops), ["سري"]);
    // The rotation's `cm` comes before the span, and the span is inside the marked content.
    let cm = ops.iter().position(|o| o.is("cm") && o.num(0).is_some_and(|c| (c - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-3)).expect("the 45° cm");
    let span =
        ops.iter().position(|o| o.is("BDC") && o.operands.first().is_some_and(|n| matches!(n, Object::Name(n) if n == b"Span"))).expect("span");
    assert!(cm < span);
    let m = ops.get(cm).and_then(|o| o.nums::<6>()).expect("six numbers");
    assert!((m[0] - m[3]).abs() < 1e-3 && (m[1] + m[2]).abs() < 1e-3 && (m[4] - 300.0).abs() < 0.01 && (m[5] - 400.0).abs() < 0.01, "{m:?}");
    // Centred on the rotation's origin.
    let lines = drawn_lines(&doc, 0, &ops);
    assert_eq!(lines.len(), 1);
    assert!((lines[0].x + lines[0].extent / 2.0).abs() < 0.01, "{lines:?}");
    // Sized to about half the page diagonal (a short word is capped at 300 pt), as a Latin one is.
    assert!(lines[0].extent > 400.0 && lines[0].extent < 520.0, "{lines:?}");
    assert!(lines[0].shown.iter().all(|(f, _)| f.starts_with("PCStAr")), "{lines:?}");
    // Several lines are centred each, with Latin text between.
    let mut doc = fixture();
    let wm = Watermark { text: "DRAFT مسودة\nسري".into(), font_size: 20.0, ..Watermark::default() };
    add_watermark(&mut doc, &[0], &wm, false).expect("drawn");
    let ops = mark_ops(&doc, 0);
    assert_eq!(actual_texts(&ops), ["DRAFT مسودة", "سري"]);
    for line in drawn_lines(&doc, 0, &ops) {
        assert!((line.x + line.extent / 2.0).abs() < 0.01, "{line:?}");
    }
}

#[test]
fn updating_and_removing_marks_take_their_fonts_away() {
    let _face = fake();
    let mut doc = fixture();
    let hf = hf_slot(2, "مرحبا");
    add_header_footer(&mut doc, &[0], &hf, false, &cx()).expect("drawn");
    let first = shaped_font_names(&doc, 0);
    assert!(!first.is_empty());
    add_header_footer(&mut doc, &[0], &hf, true, &cx()).expect("replaced");
    let second = shaped_font_names(&doc, 0);
    assert!(!second.is_empty() && second.iter().all(|n| !first.contains(n)), "{first:?} → {second:?}: only the new fonts are left");
    assert_eq!(second.len(), first.len());
    // A watermark on the same page keeps its own fonts when the header goes.
    add_watermark(&mut doc, &[0], &Watermark { text: "سري".into(), ..Watermark::default() }, false).expect("drawn");
    assert_eq!(shaped_font_names(&doc, 0).len(), second.len() * 2);
    assert_eq!(remove_marks(&mut doc, &[0], MarkKind::HeaderFooter).expect("removed"), 1);
    assert_eq!(shaped_font_names(&doc, 0).len(), second.len());
    assert_eq!(remove_marks(&mut doc, &[0], MarkKind::Watermark).expect("removed"), 1);
    assert!(shaped_font_names(&doc, 0).is_empty());
    // The page's own fonts are untouched.
    assert!(page_fonts(&doc, 0).contains(b"F1") && page_fonts(&doc, 0).contains(b"PCHelv"));
}

#[test]
fn arabic_next_to_text_nothing_can_draw_is_refused_and_changes_nothing() {
    let _face = fake();
    let before = bytes_of(&fixture());
    for text in ["مرحبا Ελληνικά", "مرحبا שלום", "مرحبا 日本", "مرحبا \u{1F600}", "مرحبا\rx"] {
        let hf = hf_slot(1, text);
        let wm = Watermark { text: text.into(), ..Watermark::default() };
        let refused = hf.first_undrawable().unwrap_or_else(|| panic!("{text:?} should be refused"));
        assert_eq!(wm.first_undrawable(), Some(refused), "{text:?}");
        let mut doc = fixture();
        assert_eq!(add_header_footer(&mut doc, &[0, 1], &hf, true, &cx()).unwrap_err().to_string(), undrawable_message(refused), "{text:?}");
        assert_eq!(add_watermark(&mut doc, &[0], &wm, true).unwrap_err().to_string(), undrawable_message(refused), "{text:?}");
        assert_eq!(bytes_of(&doc), before, "{text:?}");
    }
}

#[test]
fn shaping_failures_and_hostile_text_leave_the_document_as_it_was() {
    let _face = fake();
    // A document that already has a header and a watermark, which "replace" would remove first.
    let mut marked = fixture();
    add_header_footer(&mut marked, &[0, 1, 2], &hf_slot(1, "Old header"), false, &cx()).expect("drawn");
    add_watermark(&mut marked, &[0, 1, 2], &Watermark { text: "OLD".into(), ..Watermark::default() }, false).expect("drawn");
    let before = bytes_of(&marked);
    let huge = "ا".repeat(1_500_000);
    let long_line = "مرحبا ".repeat(5_000);
    // The stand-in can't shape U+FDFD and can't outline U+06E5 (as the real face may not).
    for text in ["\u{FDFD}", "مرحبا \u{FDFD}", "\u{06E5}", huge.as_str(), long_line.as_str()] {
        let label = text.chars().take(8).collect::<String>();
        let mut doc = Document::open(std::sync::Arc::new(before.clone())).expect("reopens");
        let err = add_header_footer(&mut doc, &[0, 1, 2], &hf_slot(2, text), true, &cx());
        assert!(err.is_err(), "{label:?}");
        let wm = Watermark { text: text.into(), ..Watermark::default() };
        assert!(add_watermark(&mut doc, &[0, 1, 2], &wm, true).is_err(), "{label:?}");
        assert_eq!(bytes_of(&doc), before, "{label:?}");
    }
}

#[test]
fn odd_arabic_text_never_panics_in_marks() {
    let _face = fake();
    let long = "ا".repeat(3_000);
    for s in [
        "\u{202E}ب\u{064B}\u{064B} (]",
        "\u{064B}",
        "ا\u{200F}\u{2067}b\u{2069}",
        "ا\n\nب\tc",
        "١٢٣ 456",
        "(ا)",
        "<<1>>ا<<",
        "ا<<Bates Number#99#x#ب#>>",
        long.as_str(),
    ] {
        for slot in 0..6 {
            let mut doc = fixture();
            let before = bytes_of(&doc);
            let r = add_header_footer(&mut doc, &[0, 1], &hf_slot(slot, s), false, &cx());
            if r.is_err() {
                assert_eq!(bytes_of(&doc), before, "{s:?} in slot {slot}");
            } else {
                reopen(&doc);
            }
        }
        let mut doc = fixture();
        let before = bytes_of(&doc);
        let wm = Watermark { text: s.into(), rotation: 33.0, ..Watermark::default() };
        match add_watermark(&mut doc, &[0], &wm, false) {
            Ok(()) => {
                reopen(&doc);
            }
            Err(_) => assert_eq!(bytes_of(&doc), before, "{s:?}"),
        }
    }
}

#[test]
fn without_the_face_arabic_is_refused_as_before_and_the_bytes_are_unchanged() {
    let _face = Using::shaper(NO_FACE);
    let before = bytes_of(&fixture());
    let hf = hf_slot(5, "<<Bates Number#4#7#سري-#>>");
    assert_eq!(hf.first_undrawable(), Some('س'));
    let wm = Watermark { text: "سري".into(), ..Watermark::default() };
    assert_eq!(wm.first_undrawable(), Some('س'));
    let mut doc = fixture();
    assert_eq!(add_header_footer(&mut doc, &[0], &hf, true, &cx()).unwrap_err().to_string(), undrawable_message('س'));
    assert_eq!(add_watermark(&mut doc, &[0], &wm, true).unwrap_err().to_string(), undrawable_message('س'));
    assert_eq!(bytes_of(&doc), before);
    // Latin text is unaffected by the missing face.
    add_header_footer(&mut doc, &[0], &hf_slot(1, "Page <<1>>"), false, &cx()).expect("Latin still draws");
}

#[test]
fn latin_headers_are_written_exactly_as_before() {
    // The shaping path must not change a byte of what Latin text produced.
    let mut doc = fixture();
    add_header_footer(&mut doc, &[0], &hf_slot(2, "Page <<1>> (draft)"), false, &cx()).expect("drawn");
    let mark = streams(&doc, 0).pop().expect("mark");
    assert!(mark.contains("BT\n/PCHelv 8 Tf\n0 0 0 rg\n1 0 0 1 ") && mark.contains("(Page 1 \\(draft\\)) Tj\nET\n"), "{mark}");
    assert!(!mark.contains("Span") && shaped_font_names(&doc, 0).is_empty());
}

/// With the real craft-fonts Arabic face: the same, drawn from its outlines.
#[test]
fn the_real_face_draws_an_arabic_header_and_watermark() {
    if pdfcraft_fonts::document_arabic_font().is_none() {
        eprintln!("skipping the real-face checks: built without a craft-fonts Arab face (set CRAFT_FONTS_DIR)");
        return;
    }
    let mut doc = fixture();
    let typed = "مرحبا بالعالم";
    let hf = hf_slot(2, &format!("{typed} <<1>>"));
    assert_eq!(hf.first_undrawable(), None);
    add_header_footer(&mut doc, &[0], &hf, false, &cx()).expect("drawn");
    let wm = Watermark { text: "سري".into(), ..Watermark::default() };
    assert_eq!(wm.first_undrawable(), None);
    add_watermark(&mut doc, &[0], &wm, false).expect("drawn");
    let doc = reopen(&doc);
    let header = streams(&doc, 0).into_iter().find(|s| s.contains("/PCMark /HeaderFooter")).expect("the header");
    let ops = pdfcraft_content::parse(header.as_bytes()).ops;
    assert_eq!(actual_texts(&ops), [format!("{typed} 1")]);
    let lines = drawn_lines(&doc, 0, &ops);
    assert!((lines[0].x + lines[0].extent - 528.0).abs() < 0.01, "{lines:?}");
    // A joined line is narrower than isolated letters would be, and has ink.
    assert!(lines[0].extent > 20.0 && lines[0].extent < 200.0, "{lines:?}");
    let watermark = mark_ops(&doc, 0);
    assert_eq!(actual_texts(&watermark), ["سري"]);
    // A letter not in the face is refused (nothing is written).
    let before = bytes_of(&doc);
    let mut again = reopen(&doc);
    let unknown = hf_slot(2, "مرحبا \u{1F600}");
    assert!(unknown.first_undrawable().is_some() && add_header_footer(&mut again, &[0], &unknown, false, &cx()).is_err());
    assert_eq!(bytes_of(&again), before);
}
