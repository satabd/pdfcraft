//! [`LineMeasure`] is the width [`lines`] wraps by: breaking a paragraph with it where it exceeds the box,
//! the way a text field does, gives the lines the saved item has.

use super::*;

/// The lines `text` breaks into at the first word whose line (from the line's start, the spaces after the
/// last word left out) is wider than `item`'s box, by `LineMeasure::width` alone. The spaces a line ends with
/// are dropped, as are those a break swallows, so these are comparable with [`lines`].
fn broken_by_measure(item: &AddedText) -> Vec<String> {
    let measure = LineMeasure::new(item);
    let width = (item.rect[2] - item.rect[0]).max(item.size);
    let mut out = Vec::new();
    for para in item.text.split('\n') {
        // The words as byte ranges (a field's words: maximal runs of what is not a space).
        let mut words: Vec<std::ops::Range<usize>> = Vec::new();
        let mut start = None;
        for (at, c) in para.char_indices() {
            match (c == ' ', start) {
                (true, Some(s)) => {
                    words.push(s..at);
                    start = None;
                }
                (false, None) => start = Some(at),
                _ => {}
            }
        }
        if let Some(s) = start {
            words.push(s..para.len());
        }
        if words.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut row_start = words[0].start;
        let mut row_end = row_start;
        for (i, word) in words.iter().enumerate() {
            if i == 0 {
                row_end = word.end;
                continue;
            }
            if measure.width(para, &para[row_start..word.end]) > width {
                out.push(para[row_start..row_end].to_owned());
                row_start = word.start;
            }
            row_end = word.end;
        }
        out.push(para[row_start..row_end].to_owned());
    }
    out
}

fn latin_texts() -> Vec<&'static str> {
    vec![
        "The quick brown fox jumps over the lazy dog and keeps running until the end of the line",
        "WWWW iiii MMMM llll mmmm wwww IIII ...... ,,,,,, ------ WiWiWi",
        "one  two   three    four five\nsecond paragraph here with some more words\n\nafter an empty one",
        "Antidisestablishmentarianism is a word wider than a narrow box",
    ]
}

fn arabic_texts() -> Vec<&'static str> {
    vec![
        "مرحبا بكم في هذا النص العربي الطويل الذي سيلتف على عدة أسطر في الصندوق",
        "Hello world مرحبا بالعالم and some more English words after the Arabic ones",
        "English words first then عربي في الآخر مع كلمات أخرى كثيرة جدا لتلتف",
        "שלום עולם זה טקסט ארוך בעברית שיתפרס על כמה שורות\nand a Latin paragraph after it",
        "مرحبا  بكم   في\nسطر ثان  قصير",
    ]
}

fn assert_same_lines(item: &AddedText) {
    let trim = |v: Vec<String>| v.into_iter().map(|l| l.trim_end().to_owned()).collect::<Vec<_>>();
    let saved = trim(lines(item));
    let measured = trim(broken_by_measure(item));
    assert_eq!(measured, saved, "box {:?} size {} {:?}/{}/{} text {:?}", item.rect, item.size, item.family, item.bold, item.italic, item.text);
}

#[test]
fn the_measure_breaks_latin_lines_where_the_saved_item_does() {
    for family in [Family::Helvetica, Family::Times, Family::Courier] {
        for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
            for size in [8.0, 12.0, 17.5, 40.0] {
                for text in latin_texts() {
                    for step in 0..40 {
                        let box_width = 18.0 + step as f64 * 11.3;
                        let item = AddedText {
                            rect: [10.0, 0.0, 10.0 + box_width, 100.0],
                            text: text.into(),
                            family,
                            bold,
                            italic,
                            size,
                            ..AddedText::default()
                        };
                        assert_same_lines(&item);
                    }
                }
            }
        }
    }
}

#[test]
fn the_measure_breaks_arabic_hebrew_and_mixed_lines_where_the_saved_item_does() {
    // With the craft-fonts faces the widths are shaped ones; without them both sides fall back to the
    // standard font's, and still agree.
    for size in [10.0, 14.0, 24.0] {
        for text in arabic_texts() {
            for step in 0..40 {
                let box_width = 30.0 + step as f64 * 13.7;
                let item = AddedText { rect: [10.0, 0.0, 10.0 + box_width, 100.0], text: text.into(), size, ..AddedText::default() };
                assert_same_lines(&item);
            }
        }
    }
}

#[test]
fn a_line_is_as_wide_as_the_item_writes_it() {
    // The width a row is drawn at is the sum of its pieces' widths: a Latin line is its standard-font width.
    let item = AddedText { text: String::new(), size: 13.0, family: Family::Times, italic: true, ..AddedText::default() };
    let measure = LineMeasure::new(&item);
    for line in ["A", "Hello world", "  leading spaces", "trailing  ", ""] {
        let expected = item.family.width(line.trim_start_matches(' '), item.size, item.bold, item.italic);
        assert_eq!(measure.width(line, line), expected, "{line:?}");
    }
    // Words of an Arabic paragraph are measured one by one with the paragraph's space.
    let arabic = "مرحبا بكم";
    let sum = measure.width(arabic, "مرحبا") + shaped_width(&item, " ", true) + measure.width(arabic, "بكم");
    assert!((measure.width(arabic, arabic) - sum).abs() < 1e-9);
    // A Latin line of an Arabic paragraph is measured as the paragraph's, with its space.
    let mixed = "hello world مرحبا";
    let latin_only = measure.width(mixed, "hello world");
    let alone = measure.width("hello world", "hello world");
    let space_here = shaped_width(&item, " ", false);
    let space_latin = item.family.width(" ", item.size, item.bold, item.italic);
    assert!((latin_only - alone - (space_here - space_latin)).abs() < 1e-9, "{latin_only} {alone}");
}

#[test]
fn a_measure_is_total_and_bounded() {
    let item = AddedText { size: 11.0, ..AddedText::default() };
    let measure = LineMeasure::new(&item);
    assert_eq!(measure.width("", ""), 0.0);
    assert_eq!(measure.width("   ", "   "), 0.0);
    // Many distinct words do not grow the memory without bound (and still measure).
    let text: String = (0..20_000).map(|i| format!("مرحبا{i} ")).collect();
    let words: Vec<&str> = text.split(' ').collect();
    let mut total = 0.0;
    for w in &words {
        total += measure.width(&text, w);
    }
    assert!(total.is_finite() && total > 0.0);
    assert!(measure.words.borrow().len() <= LineMeasure::MAX_WORDS);
    // Lines that are no prefix of the paragraph measure all the same.
    assert!(measure.width("abc", "unrelated line").is_finite());
}
