//! Text measurement and line breaking with the standard 14 fonts that stand in for the
//! template's typefaces (no font programs are embedded in dynamic XFA forms, and none are
//! bundled here).

use pdfcraft_fonts::shaped_text::{self, Measure};

use crate::model::{Font, HAlign, Para, Rect, RichText, Run};

/// Most characters of one piece of text measured by shaping it; a longer piece is estimated at
/// half an em a character, which keeps a hostile text from costing more than it is worth.
const MAX_SHAPED_CHARS: usize = 4000;

/// The standard-14 family a typeface maps to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    Helvetica,
    Courier,
    Times,
}

impl Family {
    pub fn of(typeface: &str) -> Family {
        let t = typeface.to_ascii_lowercase();
        if t.contains("courier") || t.contains("mono") {
            Family::Courier
        } else if t.contains("times")
            || t.contains("georgia")
            || t.contains("garamond")
            || t.contains("minion")
            || t.contains("serif") && !t.contains("sans")
        {
            Family::Times
        } else {
            Family::Helvetica
        }
    }

    /// The PDF base font name for a face.
    pub fn base_font(self, bold: bool, italic: bool) -> &'static str {
        match (self, bold, italic) {
            (Family::Helvetica, false, false) => "Helvetica",
            (Family::Helvetica, true, false) => "Helvetica-Bold",
            (Family::Helvetica, false, true) => "Helvetica-Oblique",
            (Family::Helvetica, true, true) => "Helvetica-BoldOblique",
            (Family::Courier, false, false) => "Courier",
            (Family::Courier, true, false) => "Courier-Bold",
            (Family::Courier, false, true) => "Courier-Oblique",
            (Family::Courier, true, true) => "Courier-BoldOblique",
            (Family::Times, false, false) => "Times-Roman",
            (Family::Times, true, false) => "Times-Bold",
            (Family::Times, false, true) => "Times-Italic",
            (Family::Times, true, true) => "Times-BoldItalic",
        }
    }

    /// The resource name used in content streams and `/DA` strings (Acrobat's conventions).
    pub fn resource_name(self, bold: bool, italic: bool) -> &'static str {
        match (self, bold, italic) {
            (Family::Helvetica, false, false) => "Helv",
            (Family::Helvetica, true, false) => "HeBo",
            (Family::Helvetica, false, true) => "HeOb",
            (Family::Helvetica, true, true) => "HeBO",
            (Family::Courier, false, false) => "Cour",
            (Family::Courier, true, false) => "CoBo",
            (Family::Courier, false, true) => "CoOb",
            (Family::Courier, true, true) => "CoBO",
            (Family::Times, false, false) => "TiRo",
            (Family::Times, true, false) => "TiBo",
            (Family::Times, false, true) => "TiIt",
            (Family::Times, true, true) => "TiBI",
        }
    }
}

/// A resolved face: family, weight, posture, size and colour.
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub family: Family,
    pub bold: bool,
    pub italic: bool,
    pub size: f64,
    pub color: [f64; 3],
    pub underline: bool,
}

impl Face {
    pub fn of(font: &Font) -> Face {
        Face {
            family: Family::of(&font.typeface),
            bold: font.bold,
            italic: font.italic,
            size: font.size.clamp(1.0, 300.0),
            color: font.color.0,
            underline: font.underline,
        }
    }

    pub fn with_run(&self, run: &Run) -> Face {
        Face {
            family: self.family,
            bold: run.bold.unwrap_or(self.bold),
            italic: run.italic.unwrap_or(self.italic),
            size: run.size.unwrap_or(self.size).clamp(1.0, 300.0),
            color: self.color,
            underline: run.underline.unwrap_or(self.underline),
        }
    }

    pub fn resource_name(&self) -> &'static str {
        self.family.resource_name(self.bold, self.italic)
    }

    /// Advance of `s` at this face's size: Arabic is measured as it is drawn (shaped, in the
    /// direction its first strong letter gives), everything else in the standard font.
    pub fn width(&self, s: &str) -> f64 {
        self.width_in(s, shaped_text::has_arabic(s) && shaped_text::first_strong_is_rtl(s))
    }

    /// [`Self::width`] for text in a paragraph that runs `rtl` or not.
    pub fn width_in(&self, s: &str, rtl: bool) -> f64 {
        if !shaped_text::has_arabic(s) {
            return self.latin_width(s);
        }
        if s.chars().take(MAX_SHAPED_CHARS + 1).count() > MAX_SHAPED_CHARS {
            return s.chars().count() as f64 * 0.5 * self.size;
        }
        let latin = |t: &str| self.latin_width(t);
        let w = Measure::new(rtl, self.size, &latin).line(s);
        if w.is_finite() { w } else { 0.0 }
    }

    /// Advance of `s` in the standard font that stands in for the face.
    fn latin_width(&self, s: &str) -> f64 {
        match self.family {
            Family::Courier => s.chars().count() as f64 * 0.6 * self.size,
            Family::Helvetica => pdfcraft_fonts::helvetica_width(s, self.size) * if self.bold { 1.04 } else { 1.0 },
            Family::Times => pdfcraft_fonts::helvetica_width(s, self.size) * if self.bold { 0.96 } else { 0.92 },
        }
    }
}

/// Default line height: XFA's font leading for the standard faces.
pub fn line_height(size: f64) -> f64 {
    size * 1.15
}

/// One positioned piece of text, ready to emit.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    /// The left end of the line as drawn.
    pub x: f64,
    /// Baseline, top-left based (y grows downwards).
    pub baseline: f64,
    /// In typing order.
    pub text: String,
    pub face: Face,
    /// Whether its paragraph runs right to left (the order Arabic and Latin pieces are drawn in).
    pub rtl: bool,
}

/// A laid-out block of text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Block {
    pub spans: Vec<Span>,
    /// Height used, from the top of the box.
    pub height: f64,
}

#[derive(Clone, Debug)]
struct Word {
    text: String,
    face: Face,
    width: f64,
}

/// The text of each run of a paragraph, as it is set: embedded fields resolved, spaces and
/// invisible characters normalised.
fn run_texts(runs: &[Run], base: &Face, embed: &dyn Fn(&str) -> String) -> Vec<(Face, String)> {
    runs.iter()
        .map(|run| {
            let face = base.with_run(run);
            let text: String = match &run.embed {
                Some(id) => embed(id),
                None => run
                    .text
                    .chars()
                    .filter_map(|c| match c {
                        '\r' | '\n' | '\t' | '\u{2028}' | '\u{2029}' | '\u{2009}' | '\u{202F}' | '\u{3000}' => Some(' '),
                        '\u{00AD}' | '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{FEFF}' => None,
                        '\u{2011}' | '\u{2010}' | '\u{2012}' | '\u{2013}' => Some('-'),
                        '\u{2018}' | '\u{2019}' => Some('\''),
                        _ => Some(c),
                    })
                    .collect(),
            };
            (face, text)
        })
        .collect()
}

/// Break the runs of a paragraph into words (spaces stay attached to the word before them), set
/// in a paragraph that runs `rtl` or not.
fn words(parts: &[(Face, String)], rtl: bool) -> Vec<Word> {
    let mut out = Vec::new();
    for (face, text) in parts {
        let mut cur = String::new();
        for ch in text.chars() {
            cur.push(ch);
            if ch == ' ' {
                let width = face.width_in(&cur, rtl);
                out.push(Word { text: std::mem::take(&mut cur), face: face.clone(), width });
            }
        }
        if !cur.is_empty() {
            let width = face.width_in(&cur, rtl);
            out.push(Word { text: cur, face: face.clone(), width });
        }
    }
    out
}

/// Lay `rich` out inside `rect` with `para` and `base` face. `embed` resolves embedded fields to
/// their text. Returns the spans (absolute coordinates) and the height the text takes.
pub fn layout(rich: &RichText, rect: Rect, para: &Para, base: &Face, embed: &dyn Fn(&str) -> String) -> Block {
    struct Line {
        words: Vec<Word>,
        width: f64,
        size: f64,
        first: bool,
        /// Whether its paragraph runs right to left.
        rtl: bool,
        /// Whether the paragraph states its alignment (`hAlign`, or `text-align` in rich text).
        stated: bool,
    }
    let avail = (rect.w - para.margin_left - para.margin_right).max(1.0);
    let mut lines: Vec<(Line, Option<HAlign>)> = Vec::new();
    let mut total = 0.0;
    for (pi, p) in rich.paragraphs.iter().enumerate() {
        let parts = run_texts(&p.runs, base, embed);
        // The paragraph's direction: its first strong character (UAX #9).
        let rtl = parts.iter().any(|(_, t)| shaped_text::has_arabic(t))
            && shaped_text::first_strong_is_rtl(&parts.iter().map(|(_, t)| t.as_str()).collect::<String>());
        let stated = p.h_align.is_some() || para.h_explicit;
        let fresh = |first: bool| Line { words: Vec::new(), width: 0.0, size: base.size, first, rtl, stated };
        let ws = words(&parts, rtl);
        let mut line = fresh(true);
        let mut first_in_para = true;
        let indent = |first: bool| if first { para.text_indent.max(0.0) } else { 0.0 };
        for w in ws {
            let fits = line.width + w.width <= avail - indent(line.first) || line.words.is_empty() && w.width <= avail - indent(line.first);
            if !fits && !line.words.is_empty() {
                let done = std::mem::replace(&mut line, fresh(false));
                lines.push((done, p.h_align));
            }
            if w.width > avail - indent(line.first) && line.words.is_empty() {
                // A word longer than a line: break it by character.
                let shaped = shaped_text::has_arabic(&w.text);
                let mut piece = String::new();
                let mut piece_width = 0.0;
                for ch in w.text.chars() {
                    piece.push(ch);
                    // Shaped text is measured a character at a time (a long word would otherwise
                    // be shaped again for every character); the standard fonts' widths add up.
                    let fits_width = if shaped {
                        piece_width += w.face.width_in(&ch.to_string(), rtl);
                        piece_width
                    } else {
                        w.face.width(&piece)
                    };
                    if fits_width > avail - indent(line.first) && piece.chars().count() > 1 {
                        piece.pop();
                        let width = w.face.width_in(&piece, rtl);
                        line.words.push(Word { text: std::mem::take(&mut piece), face: w.face.clone(), width });
                        line.width += width;
                        line.size = line.size.max(w.face.size);
                        let done = std::mem::replace(&mut line, fresh(false));
                        lines.push((done, p.h_align));
                        piece.push(ch);
                        piece_width = if shaped { w.face.width_in(&ch.to_string(), rtl) } else { 0.0 };
                    }
                }
                if !piece.is_empty() {
                    let width = w.face.width_in(&piece, rtl);
                    line.size = line.size.max(w.face.size);
                    line.width += width;
                    line.words.push(Word { text: piece, face: w.face.clone(), width });
                }
                first_in_para = false;
                continue;
            }
            line.size = line.size.max(w.face.size);
            line.width += w.width;
            line.words.push(w);
            first_in_para = false;
        }
        if !line.words.is_empty() || first_in_para && pi + 1 < rich.paragraphs.len() {
            lines.push((line, p.h_align));
        }
        total += para.space_above + para.space_below;
    }
    let heights: Vec<f64> = lines.iter().map(|(l, _)| para.line_height.unwrap_or_else(|| line_height(l.size))).collect();
    total += heights.iter().sum::<f64>();
    let start_y = match para.v_align {
        crate::model::VAlign::Top => rect.y,
        crate::model::VAlign::Middle => rect.y + ((rect.h - total) / 2.0).max(0.0),
        crate::model::VAlign::Bottom => rect.y + (rect.h - total).max(0.0),
    };
    let mut spans = Vec::new();
    let mut y = start_y + para.space_above;
    for ((line, align), lh) in lines.into_iter().zip(heights) {
        let indent = if line.first { para.text_indent.max(0.0) } else { 0.0 };
        // Trailing spaces don't count for alignment.
        let trailing = line.words.last().map_or(0.0, |w| if w.text.ends_with(' ') { w.face.width_in(" ", line.rtl) } else { 0.0 });
        let used = (line.width - trailing).max(0.0);
        // A paragraph that runs right to left starts at the right: it is right-aligned unless the
        // template says otherwise, and "justify" starts at the right too.
        let align = match align.unwrap_or(para.h_align) {
            _ if line.rtl && !line.stated => HAlign::Right,
            HAlign::Justify if line.rtl => HAlign::Right,
            a => a,
        };
        // The indent is on the side the paragraph starts from.
        let mut x = rect.x + para.margin_left + if line.rtl { 0.0 } else { indent };
        x += match align {
            HAlign::Left | HAlign::Justify => 0.0,
            HAlign::Center => ((avail - indent - used) / 2.0).max(0.0),
            HAlign::Right => (avail - indent - used).max(0.0),
        };
        // Baseline: ascent sits about 0.8 of the size below the line top for these faces.
        let baseline = y + lh - (lh - line.size * 0.8).max(0.0) / 2.0 - line.size * 0.08;
        // Runs of one face are one span. In a paragraph that runs right to left the first run is
        // at the right, so the runs are placed from the line's right end.
        let mut runs: Vec<Span> = Vec::new();
        for w in line.words {
            match runs.last_mut() {
                Some(s) if s.face == w.face => s.text.push_str(&w.text),
                _ => runs.push(Span { x, baseline, text: w.text, face: w.face, rtl: line.rtl }),
            }
        }
        if line.rtl {
            // The ink ends at the alignment edge; a trailing space hangs out past the left end.
            let right = x + used;
            let mut consumed = 0.0;
            for s in &mut runs {
                consumed += s.face.width_in(&s.text, true);
                s.x = right - consumed;
            }
        } else {
            let mut pen = x;
            for s in &mut runs {
                s.x = pen;
                pen += s.face.width_in(&s.text, false);
            }
        }
        spans.extend(runs);
        y += lh;
    }
    Block { spans, height: total }
}

/// Height `rich` needs at `width` (for boxes without an explicit height).
pub fn measure_height(rich: &RichText, width: f64, para: &Para, base: &Face, embed: &dyn Fn(&str) -> String) -> f64 {
    let p = Para { v_align: crate::model::VAlign::Top, ..para.clone() };
    layout(rich, Rect::new(0.0, 0.0, width.max(1.0), 0.0), &p, base, embed).height
}

/// Plain text as rich text (paragraphs split on line breaks; a carriage return and line feed
/// together are one break, as form data often holds them).
pub fn plain(text: &str) -> RichText {
    RichText {
        paragraphs: text
            .replace("\r\n", "\n")
            .split(['\n', '\r', '\u{2028}', '\u{2029}'])
            .map(|l| crate::model::Paragraph { runs: vec![Run { text: l.to_string(), ..Run::default() }], h_align: None })
            .collect(),
    }
}
