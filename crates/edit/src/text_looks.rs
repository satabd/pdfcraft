//! The looks of a paragraph of several looks that Edit text writes as shaped Arabic ([`crate::text_shaped`]).
//!
//! A paragraph whose runs differ in size, colour or font (a bold word, a larger first line) keeps them when it is
//! retyped: each character of the draft takes the look of the stored character it came from, and an inserted one the
//! look of its neighbour ([`TextBlock::edited_spans`], the rule the styled rewrite and the editor follow too). A look
//! never changes inside a stretch of right-to-left script, which is shaped as a whole: the stretch takes the look of
//! its first letter, as the editor draws it (`shaper_styles` in `pdfcraft-ui-egui`'s `edit_text_ui`).

use super::*;

/// Most looks one paragraph is drawn in; the characters of any look past them take the paragraph's first look.
const MAX_LOOKS: usize = 256;

/// Whether `c` belongs to a stretch of right-to-left script that is shaped as a whole: the letters, marks, digits and
/// punctuation of the right-to-left scripts and the joiners inside a word. The editor's own rule (`is_shaped_rtl` in
/// `pdfcraft-ui-egui`'s `bidi`), so that the page and the editor cut looks at the same places.
fn is_shaped_rtl(c: char) -> bool {
    matches!(u32::from(c), 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFC | 0x1_0800..=0x1_0FFF | 0x1_E800..=0x1_EFFF)
        || matches!(c, '\u{200C}' | '\u{200D}')
}

/// Direction marks, embeddings, overrides and isolates: they are not drawn, so they neither end a stretch nor start one.
fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// The source run (an index into [`text_lines`]) whose look each character of `draft` takes: the stored character it came
/// from ([`TextBlock::edited_spans`]), the first letter's for every character of a stretch of right-to-left script.
pub(super) fn draft_sources(block: &TextBlock, draft: &str) -> Vec<usize> {
    let fallback = block.lines.first().copied().unwrap_or(0);
    let mut sources: Vec<usize> = Vec::with_capacity(draft.len());
    for (range, line) in block.edited_spans(draft) {
        let count = draft.get(range).map_or(0, |piece| piece.chars().count());
        sources.extend(std::iter::repeat_n(line, count));
    }
    // A look never changes inside a stretch of right-to-left script: its letters take their first one's.
    let mut stretch: Option<usize> = None;
    for (c, source) in draft.chars().zip(sources.iter_mut()) {
        if is_bidi_control(c) {
            if let Some(first) = stretch {
                *source = first;
            }
        } else if is_shaped_rtl(c) {
            match stretch {
                Some(first) => *source = first,
                None => stretch = Some(*source),
            }
        } else {
            stretch = None;
        }
    }
    sources.resize(draft.chars().count(), fallback);
    sources
}

/// A draft laid out in several looks: the text as the rewrite lays it out and the look of each of its characters.
pub(super) struct LookedText {
    /// The draft with runs of white space as one space (see [`collapse`]): the text the rewrite lays out.
    pub text: String,
    /// Each character's byte offset in `text` and its look (an index into `sources`).
    pub looks: Vec<(usize, usize)>,
    /// The source run each look is drawn like (an index into [`text_lines`]); look 0 is the paragraph's first run's.
    pub sources: Vec<usize>,
}

impl LookedText {
    /// The looks of the characters of `range`, a byte range of [`Self::text`] on character boundaries.
    pub fn looks_of(&self, range: std::ops::Range<usize>) -> Vec<usize> {
        let start = self.looks.partition_point(|(at, _)| *at < range.start);
        let end = self.looks.partition_point(|(at, _)| *at < range.end);
        self.looks.get(start..end).unwrap_or_default().iter().map(|(_, look)| *look).collect()
    }

    /// The characters drawn in look `look`, a line break as a space.
    pub fn text_of(&self, look: usize) -> String {
        self.text.chars().zip(&self.looks).filter(|(_, (_, l))| *l == look).map(|(c, _)| if c == '\n' { ' ' } else { c }).collect()
    }
}

/// What tells two looks apart: the font, the size and the fill colour a run is drawn in, as `style` leaves them (no fill
/// operator at all is the initial black, `0 g`).
fn look_key(line: &TextLine, style: &BlockStyle) -> (String, u64, String) {
    let size = if style.size.is_some() { 0 } else { line.size.to_bits() };
    let fill = match &line.origin.state.fill {
        _ if style.color.is_some() => String::new(),
        fill if fill.is_empty() => "g [Int(0)];".to_owned(),
        fill => fill.iter().map(|op| format!("{} {:?};", String::from_utf8_lossy(&op.op), op.operands)).collect(),
    };
    (line.font.clone(), size, fill)
}

/// `draft` in the looks of paragraph `block` (see the module): `None` when every character is drawn in the look of
/// the paragraph's first run, which the rewrite then writes as it writes a paragraph of one look.
pub(super) fn looked_text(block: &TextBlock, lines: &[TextLine], draft: &str, style: &BlockStyle) -> Option<LookedText> {
    let first = block.lines.first().and_then(|i| lines.get(*i))?;
    let first_key = look_key(first, style);
    let sources = draft_sources(block, draft);
    // Each source run's look, one per distinct key, the first run's first.
    let mut by_key: HashMap<(String, u64, String), usize> = HashMap::from([(first_key, 0)]);
    let mut by_source: HashMap<usize, usize> = HashMap::new();
    let mut looks_of_sources = vec![block.lines.first().copied().unwrap_or(0)];
    let mut looks = Vec::with_capacity(sources.len());
    for source in sources {
        let look = *by_source.entry(source).or_insert_with(|| {
            let Some(line) = lines.get(source) else { return 0 };
            let key = look_key(line, style);
            if let Some(look) = by_key.get(&key) {
                return *look;
            }
            let look = looks_of_sources.len();
            if look >= MAX_LOOKS {
                return 0;
            }
            by_key.insert(key, look);
            looks_of_sources.push(source);
            look
        });
        looks.push(look);
    }
    if looks.iter().all(|look| *look == 0) {
        return None;
    }
    let (text, char_looks) = collapse(draft, &looks);
    let looks = text.char_indices().map(|(at, _)| at).zip(char_looks).collect();
    Some(LookedText { text, looks, sources: looks_of_sources })
}

/// How a look shows the Latin text among its Arabic: in its run's own font where that font has the characters, in the
/// standard font of its family and weight otherwise.
pub(super) struct LatinLook {
    /// The font's metrics when it is the run's own (`None`: a standard font).
    pub metrics: Option<Metrics>,
    /// The width of a space in the run's own font, in em.
    pub space: f64,
    /// The standard font measured when `metrics` is `None`.
    pub face: pdfcraft_fonts::Std14,
    /// The look's size in text space.
    pub size: f64,
}

impl LatinLook {
    /// The width of Latin text `s` in this look, in text space before the horizontal scale.
    pub fn width(&self, s: &str) -> f64 {
        match &self.metrics {
            Some(m) => source_width(m, s, self.size, self.space, 0.0, 0.0),
            None => self.face.text_width(s, self.size),
        }
    }

    /// The operators that show Latin text `s` in this look's font.
    pub fn show(&self, s: &str) -> Result<Vec<Op>, EditError> {
        match &self.metrics {
            Some(m) => source_show(m, s, self.size, self.space, 0.0, 0.0),
            None => {
                if s.chars().any(|c| !is_win_ansi_char(c) && c != '?') {
                    return Err(text_shaped::unsupported_with_arabic(s));
                }
                Ok(vec![Op::new("Tj", vec![Object::String(PdfString::literal(pdfcraft_fonts::win_ansi(s)))])])
            }
        }
    }

    /// The look of source run `line` for the Latin text `latin` (what it draws besides Arabic), at `size` in text space,
    /// with the font resource it shows the text in: a standard font it needs is added to `fonts_res`. An error when neither
    /// its font nor the standard one has the text.
    pub fn of(
        doc: &Document,
        fonts_res: &mut Dict,
        line: &TextLine,
        latin: &str,
        style: &BlockStyle,
        size: f64,
    ) -> Result<(String, Self), EditError> {
        let (family, bold, italic) = style.family.unwrap_or((source_family(&line.base_font), line.bold, line.italic));
        let bold = style.bold.unwrap_or(bold);
        let face = family.std14(bold, italic);
        let metrics =
            fonts_res.get(line.font.as_bytes()).and_then(|f| doc.resolve(f).as_dict().cloned()).map(|d| Metrics::from_dict(doc, &d)).filter(|m| {
                style.family.is_none() && style.bold.is_none() && source_encodes(m, latin) && source_encodes(m, &text_shaped::with_mirrors(latin))
            });
        if let Some(m) = metrics {
            let space = source_space(doc, fonts_res, &m);
            return Ok((line.font.clone(), LatinLook { metrics: Some(m), space, face, size }));
        }
        if needs_type3(latin) {
            return Err(text_shaped::unsupported_with_arabic(latin));
        }
        let base = family.base_font(bold, italic);
        let font = format!("PCEd{}", base.replace('-', ""));
        let mut f = Dict::new();
        f.set(b"Type".to_vec(), Object::name("Font"));
        f.set(b"Subtype".to_vec(), Object::name("Type1"));
        f.set(b"BaseFont".to_vec(), Object::name(base));
        f.set(b"Encoding".to_vec(), Object::name("WinAnsiEncoding"));
        fonts_res.set(font.clone().into_bytes(), Object::Dict(f));
        Ok((font, LatinLook { metrics: None, space: 0.33, face, size }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A paragraph of two runs, `ab cd` (source run 7) and `ef` (source run 9), as the reader would give it.
    fn block(first: &str, second: &str) -> TextBlock {
        let editor_text = format!("{first}{second}");
        TextBlock {
            text: editor_text.clone(),
            editor_text: editor_text.clone(),
            spans: vec![(0..first.len(), 7), (first.len()..editor_text.len(), 9)],
            rect: [0.0; 4],
            base_font: "F".into(),
            size: 12.0,
            bold: false,
            italic: false,
            color: [0.0; 3],
            rotation: 0.0,
            lines: vec![7, 9],
            rows: vec![vec![7, 9]],
            invisible: false,
        }
    }

    #[test]
    fn each_character_takes_its_stored_characters_look_and_a_word_of_arabic_its_first_letters() {
        // Latin keeps a look change inside a word.
        assert_eq!(draft_sources(&block("ab c", "d"), "ab cd"), [7, 7, 7, 7, 9]);
        // Arabic does not: the word that joins `سم` (7) and `ب` (9) is in its first letter's look, the word after a space
        // in its own; a direction mark inside the word neither ends it nor starts another.
        assert_eq!(draft_sources(&block("مسم س", "ب سم"), "مسم سب سم"), [7, 7, 7, 7, 7, 7, 9, 9, 9]);
        assert_eq!(draft_sources(&block("س", "\u{200F}ب"), "س\u{200F}ب"), [7, 7, 7]);
        // A typed character takes its neighbour's; nothing at all gives nothing.
        assert_eq!(draft_sources(&block("ab ", "cd"), "ab cdx"), [7, 7, 7, 9, 9, 9]);
        assert!(draft_sources(&block("ab", "cd"), "").is_empty());
    }

    #[test]
    fn black_set_out_loud_is_the_initial_black_and_another_colour_another_look() {
        let key = |fill: &str| {
            let doc = crate::tests::text_page(&format!("BT /F1 12 Tf 100 700 Td (ab) Tj {fill} 0 -16 Td (cd) Tj ET"));
            let lines = text_lines(&doc, 0).expect("lines");
            assert_eq!(lines.len(), 2, "{fill}");
            let style = BlockStyle::default();
            (look_key(&lines[0], &style) == look_key(&lines[1], &style), look_key(&lines[1], &BlockStyle { color: Some([0.0; 3]), ..style }))
        };
        assert!(key("0 g").0);
        assert!(!key("1 0 0 rg").0);
        // A colour asked for the whole paragraph makes every run's the same.
        assert_eq!(key("1 0 0 rg").1, key("").1);
    }

    #[test]
    fn collapsing_white_space_keeps_each_characters_look() {
        // As the rewrite always collapsed it: one space between words, none at the ends of a line, ideographic spaces kept.
        let old = |text: &str| -> String {
            text.split('\n')
                .map(|row| row.split(paragraph_separator).filter(|word| !word.is_empty()).collect::<Vec<_>>().join(" "))
                .collect::<Vec<_>>()
                .join("\n")
        };
        for text in ["", " ", "a", "  a  b\t\tc ", "a\n\n b \n", "\n", "日本\u{3000}語  x", "a\r\nb", "\u{A0}x\u{2003}y "] {
            assert_eq!(collapse(text, &[]).0, old(text), "{text:?}");
            assert!(collapse(text, &[]).1.is_empty());
        }
        // A run of spaces takes the look of its first.
        assert_eq!(collapse(" ab  \tc\n d ", &[0, 1, 1, 2, 3, 4, 5, 6, 7, 8, 9]), ("ab c\nd".into(), vec![1, 1, 2, 5, 6, 8]));
        // Fewer looks than characters: the rest are look 0.
        assert_eq!(collapse("ab c", &[3]), ("ab c".into(), vec![3, 0, 0, 0]));
    }

    #[test]
    fn wrapping_by_ranges_breaks_where_the_words_do() {
        let text = "one two three\nfour";
        let width = |range: std::ops::Range<usize>| text.get(range).map_or(0.0, |s| s.chars().count() as f64);
        let rows: Vec<(&str, bool)> = wrap_ranges(text, 7.0, width).into_iter().map(|(r, ends)| (text.get(r).unwrap_or("?"), ends)).collect();
        assert_eq!(rows, [("one two", false), ("three", true), ("four", true)]);
        assert_eq!(wrap_ranges("", 7.0, width), [(0..0, true)]);
        assert_eq!(wrap_ranges("\n", 7.0, |_| 0.0), [(0..0, true), (1..1, true)]);
    }
}
