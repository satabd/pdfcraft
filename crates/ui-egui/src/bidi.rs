//! Visual order for single-line interface text that may hold right-to-left script (file names,
//! document titles).
//!
//! egui shapes each font-face run with the script's own direction, so the letters of an Arabic
//! word join and run right to left, but it places the runs themselves left to right in logical
//! order: `واحد اثنين.pdf` would show its two words swapped. [`visual`] puts the runs in display
//! order and leaves the letters of each word alone for the shaper.
//!
//! [`display_rtl`] does the same for the labels of a right-to-left interface language (the Arabic
//! catalog), whose base direction is right to left.
//!
//! [`screen_map`] keeps where every character of a [`visual`] line goes, which is what a text field
//! needs to turn the caret and selection egui reports (in the order the glyphs are drawn) into
//! places in the stored, logical text (`widgets::BidiEdit`). A field's line reads in the direction
//! of its first strong character ([`line_rtl`], [`line_screen_map`]), as a paragraph of a
//! multi-line field does ([`wrapped_screen_map`]).

use std::borrow::Cow;
use std::ops::Range;

use unicode_bidi::{BidiInfo, Level, ParagraphInfo};

/// `text` rearranged for an egui call that lays out one line. Text without right-to-left
/// characters comes back borrowed and unchanged.
///
/// The base direction is left to right, as the interface is: a file name keeps its extension
/// at the end, the way the window title shows it. Use it for painting only; accessibility
/// labels and anything stored keep the logical text.
pub fn visual(text: &str) -> Cow<'_, str> {
    if !text.chars().any(is_rtl_script) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    push_visual(&mut out, text, Level::ltr());
    Cow::Owned(out)
}

/// `text` in display order for the paragraph direction `base`, appended to `out`.
fn push_visual(out: &mut String, text: &str, base: Level) {
    let info = BidiInfo::new(text, Some(base));
    for para in &info.paragraphs {
        push_line(out, text, &info, para, para.range.clone());
    }
}

/// The `line` of paragraph `para` (a non-empty byte range of `text`) in display order, as egui
/// needs it, appended to `out`.
fn push_line(out: &mut String, text: &str, info: &BidiInfo<'_>, para: &ParagraphInfo, line: Range<usize>) {
    // First the true display order, every character left to right as it appears on screen.
    let mut shown = String::with_capacity(text.len());
    for_each_shown(text, info, para, line, |c, _, _| shown.push(c));
    push_for_shaper(out, &shown);
}

/// Calls `emit(c, at, rtl)` for each character of the `line` of paragraph `para` in true display
/// order, left to right as it appears on screen: `c` as drawn (brackets mirrored in a right-to-left
/// run), `at` the byte offset of the character in `text`, `rtl` whether its run reads right to
/// left. Direction controls are not drawn and not emitted.
fn for_each_shown(text: &str, info: &BidiInfo<'_>, para: &ParagraphInfo, line: Range<usize>, mut emit: impl FnMut(char, usize, bool)) {
    // unicode-bidi cannot reorder a line whose levels are all the deepest there are (about sixty
    // unclosed embeddings and a letter after them), and panics. Such a line is drawn as it is
    // written, left to right.
    let deepest = Level::max_implicit_depth();
    if info.levels.get(line.clone()).is_none_or(|levels| levels.iter().all(|level| level.number() >= deepest)) {
        for (at, c) in text.get(line.clone()).unwrap_or_default().char_indices().filter(|(_, c)| !is_bidi_control(*c)) {
            emit(c, line.start.saturating_add(at), false);
        }
        return;
    }
    let (levels, runs) = info.visual_runs(para, line);
    for run in runs {
        let rtl = levels.get(run.start).is_some_and(Level::is_rtl);
        // Runs lie on character boundaries; a missing slice is skipped, never a panic.
        let Some(piece) = text.get(run.clone()) else { continue };
        let chars = piece.char_indices().filter(|(_, c)| !is_bidi_control(*c));
        if rtl {
            for (at, c) in chars.rev() {
                emit(mirrored(c), run.start.saturating_add(at), true);
            }
        } else {
            for (at, c) in chars {
                emit(c, run.start.saturating_add(at), false);
            }
        }
    }
}

/// `shown` (in screen order) as egui must get it: egui hands each stretch of right-to-left
/// script to the shaper as one run, which lays it out right to left (and joins its letters), so
/// each such stretch goes back to logical order. What lies between them (spaces, punctuation,
/// Latin) is drawn by the Latin face left to right and stays as it is. The stretches include
/// Persian and Arabic-Indic digits: Arabic script to the shaper, though they read left to right.
fn push_for_shaper(out: &mut String, shown: &str) {
    let mut stretch: Vec<char> = Vec::new();
    for c in shown.chars() {
        if is_shaped_rtl(c) {
            stretch.push(c);
        } else {
            out.extend(stretch.drain(..).rev());
            out.push(c);
        }
    }
    out.extend(stretch.drain(..).rev());
}

/// How [`visual`] puts the characters of one text on screen, character by character: what a text
/// field needs to relate the caret and selection egui reports in the order the galley is drawn
/// (screen order) to the text that is stored (logical order).
///
/// Indices are character indices. `N`, the number of characters drawn, is at most `M`, the number
/// in the text: direction controls are not drawn. A map of wrapped text ([`wrapped_screen_map`])
/// also draws a *line break* between the lines a paragraph is wrapped into, which is no character
/// of the text, so there `N` can exceed `M`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScreenMap {
    /// The text to hand to egui: exactly what [`visual`] returns. Its i-th character is the
    /// i-th one from the left in the galley egui builds from it.
    pub shaper: String,
    /// The logical index of the character drawn at each screen position (`N` entries); [`VIRTUAL`]
    /// for the line break of a wrapped paragraph.
    pub shown: Vec<usize>,
    /// The character as drawn at each screen position (`N` entries): the text's own, brackets
    /// mirrored in a right-to-left run; `'\n'` for a line break, the text's own or wrapped.
    pub drawn: Vec<char>,
    /// Whether the run holding each logical character reads right to left (`M` entries; false
    /// for the characters that are not drawn).
    pub rtl: Vec<bool>,
    /// The screen position of each logical character (`M` entries); `None` for a direction
    /// control, which is not drawn.
    pub to_screen: Vec<Option<usize>>,
    /// Whether each line of a wrapped text (one per `'\n'` of [`Self::shaper`], and the line after
    /// the last) belongs to a paragraph that reads right to left; empty for a single line.
    pub row_rtl: Vec<bool>,
}

/// What [`ScreenMap::shown`] holds for a line break that is no character of the text.
pub const VIRTUAL: usize = usize::MAX;

/// The [`ScreenMap`] of `text`: [`visual`] with the position of every character kept. Text without
/// right-to-left script maps to itself. A field's line is laid out by [`line_screen_map`], which is
/// this for a line that reads left to right.
#[cfg_attr(not(test), expect(dead_code, reason = "the left-to-right map the tests compare a field's line with"))]
pub fn screen_map(text: &str) -> ScreenMap {
    if !has_rtl(text) {
        return ScreenMap::identity(text);
    }
    screen_map_at(text, Level::ltr())
}

/// The direction one line of a field reads in, by the rule [`wrapped_screen_map_with`] gives each
/// paragraph of a multi-line field: `forced_rtl` when the field's owner decides it, else the
/// direction of the line's first strong character (as `dir="auto"` reads it, [`first_strong_rtl`]),
/// else `neutral_rtl` (an empty line, or digits and punctuation only, which do not say where they
/// belong; a field passes the interface's direction, as `widgets::field_align` does).
pub fn line_rtl(text: &str, neutral_rtl: bool, forced_rtl: Option<bool>) -> bool {
    forced_rtl.or_else(|| first_strong_rtl(text)).unwrap_or(neutral_rtl)
}

/// The [`ScreenMap`] of the one line of a single-line field: [`screen_map`], but laid out by UAX #9
/// for the base direction [`line_rtl`] gives the line (paragraph level 1 when it reads right to
/// left), so an Arabic sentence with a Latin word at its end draws that word at the left end. Text
/// without right-to-left script maps to itself.
pub fn line_screen_map(text: &str, neutral_rtl: bool, forced_rtl: Option<bool>) -> ScreenMap {
    if !has_rtl(text) {
        return ScreenMap::identity(text);
    }
    screen_map_at(text, if line_rtl(text, neutral_rtl, forced_rtl) { Level::rtl() } else { Level::ltr() })
}

/// The [`ScreenMap`] of `text`, which holds right-to-left script, with every paragraph laid out for
/// the base direction `base`.
fn screen_map_at(text: &str, base: Level) -> ScreenMap {
    let offsets: Vec<usize> = text.char_indices().map(|(at, _)| at).collect();
    let count = offsets.len();
    let mut map = ScreenMap {
        shaper: String::with_capacity(text.len()),
        shown: Vec::with_capacity(count),
        drawn: Vec::with_capacity(count),
        rtl: vec![false; count],
        to_screen: vec![None; count],
        row_rtl: Vec::new(),
    };
    let info = BidiInfo::new(text, Some(base));
    for para in &info.paragraphs {
        let mut drawn = String::with_capacity(para.range.len());
        for_each_shown(text, &info, para, para.range.clone(), |c, at, rtl| {
            // `at` is the start of a character, so the search finds it; if not, the character is skipped.
            let Ok(logical) = offsets.binary_search(&at) else { return };
            drawn.push(c);
            let screen = map.shown.len();
            map.shown.push(logical);
            map.drawn.push(c);
            if let Some(slot) = map.rtl.get_mut(logical) {
                *slot = rtl;
            }
            if let Some(slot) = map.to_screen.get_mut(logical) {
                *slot = Some(screen);
            }
        });
        push_for_shaper(&mut map.shaper, &drawn);
    }
    map
}

/// The [`ScreenMap`] of multi-line `text` wrapped for a field: every paragraph (the text between
/// line separators) is broken into lines that `fits` accepts, and each line put in display order
/// by the Unicode bidirectional algorithm (UAX #9) for the paragraph's direction, which is that of
/// its first strong character (`neutral_rtl` for one without, such as an empty line or digits
/// only, which would not say where it belongs). `forced_rtl` overrides that for every paragraph
/// (`Some(true)` reads them all right to left, `Some(false)` left to right): a caller whose text
/// has its own rule for the direction (a block that reads as a whole one way) says so.
///
/// The map's [`ScreenMap::shaper`] is the lines joined by `'\n'`, so egui lays one line on each of
/// its rows. A line break the wrapping makes is [`VIRTUAL`]: it maps to no character of the text
/// (`shown` holds [`VIRTUAL`] for it and no `to_screen` entry points at it). The separator at the
/// end of a paragraph is drawn as a `'\n'` that is a character of the text. Everything else is
/// drawn exactly once: unlike [`wrap`] this keeps the spaces at the ends of lines (a space typed
/// at the end of the text must show), so the lines partition the paragraph, bar its direction
/// controls (and the `\n` of a `\r\n`, which shares the break of the `\r`).
///
/// `fits` measures a line in logical order, without the spaces after its last word; a word wider
/// than a line is broken between characters. It is called for the same text many times over; the
/// caller bounds its cost.
pub fn wrapped_screen_map(text: &str, neutral_rtl: bool, forced_rtl: Option<bool>, mut fits: impl FnMut(&str) -> bool) -> ScreenMap {
    wrapped_screen_map_with(text, neutral_rtl, forced_rtl, WrapRules::default(), |candidate| fits(candidate.line))
}

/// How a field decides where its lines break, beyond how wide they may be (see [`wrapped_screen_map_with`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WrapRules {
    /// A word wider than a line stays whole on a line of its own, as a page that wraps at spaces only
    /// writes it; by default it is broken between characters so that it stays inside the field.
    pub whole_words: bool,
    /// Only the space (U+0020) ends a word; by default any whitespace does (a tab, a no-break space, ...).
    pub spaces_only: bool,
}

/// A line [`wrapped_screen_map_with`] asks `fits` about.
#[derive(Clone, Copy, Debug)]
pub struct Candidate<'a> {
    /// The line so far, in logical order: from the start of its paragraph or of the line to the end of
    /// the last word that may go on it, without the spaces after that word.
    pub line: &'a str,
    /// The paragraph the line belongs to (without its separator), in logical order.
    pub paragraph: &'a str,
    /// Where the paragraph begins in the whole text, in bytes.
    pub offset: usize,
    /// Where the line begins in the paragraph, in bytes.
    pub start: usize,
}

/// [`wrapped_screen_map`] for a text whose owner has rules of its own for the lines (see [`WrapRules`]) and
/// measures a line knowing the paragraph it is in (a page measures an Arabic paragraph word by word, and by
/// the face of the paragraph's script, whatever the line holds).
pub fn wrapped_screen_map_with(
    text: &str,
    neutral_rtl: bool,
    forced_rtl: Option<bool>,
    rules: WrapRules,
    mut fits: impl FnMut(Candidate<'_>) -> bool,
) -> ScreenMap {
    let offsets: Vec<usize> = text.char_indices().map(|(at, _)| at).collect();
    let count = offsets.len();
    let mut map = ScreenMap {
        shaper: String::with_capacity(text.len()),
        shown: Vec::with_capacity(count),
        drawn: Vec::with_capacity(count),
        rtl: vec![false; count],
        to_screen: vec![None; count],
        row_rtl: Vec::new(),
    };
    let base = forced_rtl.map(|rtl| if rtl { Level::rtl() } else { Level::ltr() });
    let info = BidiInfo::new(text, base);
    let mut inherited = neutral_rtl;
    for para in &info.paragraphs {
        let whole = text.get(para.range.clone()).unwrap_or_default();
        let content_len = whole.trim_end_matches(is_paragraph_separator).len();
        let content = para.range.start..para.range.start.saturating_add(content_len);
        // unicode-bidi ends a paragraph at the `\r` of `\r\n`; the `\n` is no new paragraph.
        let continues = whole == "\n" && text.get(..para.range.start).is_some_and(|before| before.ends_with('\r'));
        let rtl_para = if let Some(forced) = forced_rtl {
            forced
        } else if para.level.is_rtl() {
            true
        } else {
            match unicode_bidi::get_base_direction(whole) {
                unicode_bidi::Direction::Ltr => false,
                _ => inherited,
            }
        };
        if continues {
            // Drawn nowhere and no line of its own: the `\r` before it is the break.
            continue;
        }
        inherited = rtl_para;
        let paragraph = text.get(content.clone()).unwrap_or_default();
        let rows = if content.is_empty() {
            vec![content.clone()]
        } else {
            // The line is a slice of the paragraph, which is a slice of the text: its place is the difference of their addresses.
            let base = paragraph.as_ptr() as usize;
            break_rows(text, content.clone(), rules, &mut |line: &str| {
                let start = (line.as_ptr() as usize).checked_sub(base).filter(|start| *start <= paragraph.len()).unwrap_or(0);
                fits(Candidate { line, paragraph, offset: content.start, start })
            })
        };
        for (k, row) in rows.into_iter().enumerate() {
            if k > 0 {
                map.shown.push(VIRTUAL);
                map.drawn.push('\n');
                map.shaper.push('\n');
            }
            map.row_rtl.push(rtl_para);
            if row.is_empty() {
                continue;
            }
            let mut drawn = String::with_capacity(row.len());
            for_each_shown(text, &info, para, row, |c, at, rtl| {
                // `at` is the start of a character, so the search finds it; if not, the character is skipped.
                let Ok(logical) = offsets.binary_search(&at) else { return };
                drawn.push(c);
                map.push_shown(logical, c, rtl);
            });
            push_for_shaper(&mut map.shaper, &drawn);
        }
        // The separators that end the paragraph (one, but whatever unicode-bidi calls a paragraph).
        let tail_start = para.range.start.saturating_add(content_len);
        for (at, _) in whole.get(content_len..).unwrap_or_default().char_indices() {
            let Ok(logical) = offsets.binary_search(&tail_start.saturating_add(at)) else { continue };
            map.shaper.push('\n');
            map.push_shown(logical, '\n', false);
        }
    }
    // The empty line egui adds after the text's last separator, which the caret can be on.
    if text.chars().next_back().is_some_and(is_paragraph_separator) {
        map.row_rtl.push(inherited);
    }
    map
}

impl ScreenMap {
    /// The map of `text` drawn exactly as it is stored: every character at its own place, all
    /// reading left to right. What a field that does not reorder (a plain `egui::TextEdit`) shows.
    pub fn identity(text: &str) -> Self {
        let count = text.chars().count();
        ScreenMap {
            shaper: text.to_owned(),
            shown: (0..count).collect(),
            drawn: text.chars().collect(),
            rtl: vec![false; count],
            to_screen: (0..count).map(Some).collect(),
            row_rtl: Vec::new(),
        }
    }

    /// Appends the logical character `logical`, drawn as `c`, at the next screen position.
    fn push_shown(&mut self, logical: usize, c: char, rtl: bool) {
        let screen = self.shown.len();
        self.shown.push(logical);
        self.drawn.push(c);
        if let Some(slot) = self.rtl.get_mut(logical) {
            *slot = rtl;
        }
        if let Some(slot) = self.to_screen.get_mut(logical) {
            *slot = Some(screen);
        }
    }
}

/// The byte ranges of the lines `range` of `text` (one paragraph, not empty) is broken into: as
/// many words as `fits` accepts per line, a word too wide for any line split between characters.
/// The spaces after a word stay on its line, and those before the first word are on the first, so
/// the lines cover `range` exactly. Always at least one line.
fn break_rows(text: &str, range: Range<usize>, rules: WrapRules, fits: &mut impl FnMut(&str) -> bool) -> Vec<Range<usize>> {
    let words = words(text, range.clone(), rules.spaces_only);
    if words.is_empty() {
        return vec![range];
    }
    let mut rows = Vec::new();
    let mut start = range.start;
    let mut row_end = range.start;
    let mut filled = false;
    for (i, word) in words.iter().enumerate() {
        // The word with the spaces after it.
        let chunk_end = words.get(i.saturating_add(1)).map_or(range.end, |next| next.start);
        if filled {
            if fits(text.get(start..word.end).unwrap_or_default()) {
                row_end = chunk_end;
                continue;
            }
            rows.push(start..row_end);
            start = word.start;
        }
        // The word starts a line.
        loop {
            if rules.whole_words || fits(text.get(start..word.end).unwrap_or_default()) {
                row_end = chunk_end;
                filled = true;
                break;
            }
            let end = fitting_prefix(text, start..word.end, fits);
            if end >= word.end {
                row_end = chunk_end;
                filled = true;
                break;
            }
            rows.push(start..end);
            start = end;
        }
    }
    rows.push(start..row_end.max(start));
    // A line of direction controls alone would draw nothing, and a caret there would be no place of
    // the text: it joins the line after it (or before it, at the end).
    let mut joined: Vec<Range<usize>> = Vec::with_capacity(rows.len());
    let mut pending: Option<usize> = None;
    for row in rows {
        let from = pending.take().unwrap_or(row.start);
        if text.get(row.clone()).is_some_and(|piece| piece.chars().all(is_bidi_control)) {
            pending = Some(from);
        } else {
            joined.push(from..row.end);
        }
    }
    match (pending, joined.last_mut()) {
        (Some(_), Some(last)) => last.end = range.end,
        (Some(from), None) => joined.push(from..range.end),
        (None, _) => {}
    }
    joined
}

impl ScreenMap {
    /// `M`: the characters of the text.
    pub fn logical_len(&self) -> usize {
        self.to_screen.len()
    }

    /// `N`: the characters drawn, and the last screen position of a caret.
    pub fn screen_len(&self) -> usize {
        self.shown.len()
    }

    /// The screen position just after the logical character `p` as it is read: at its right edge
    /// in a left-to-right run, at its left edge in a right-to-left one. `None` if it is not drawn.
    fn trail(&self, p: usize) -> Option<usize> {
        let at = (*self.to_screen.get(p)?)?;
        Some(if self.rtl.get(p).copied().unwrap_or(false) { at } else { at.saturating_add(1) })
    }

    /// The screen position just before the logical character `n` as it is read: at its left edge
    /// in a left-to-right run, at its right edge in a right-to-left one. `None` if it is not drawn.
    fn lead(&self, n: usize) -> Option<usize> {
        let at = (*self.to_screen.get(n)?)?;
        Some(if self.rtl.get(n).copied().unwrap_or(false) { at.saturating_add(1) } else { at })
    }

    /// The screen positions (`0..=N`) a caret between logical characters `l - 1` and `l` (`0..=M`)
    /// can be drawn at, the one after the previous character first: typing continues there.
    /// There are two when the neighbours sit apart on screen (at the edge of a run of the other
    /// direction), one otherwise. `l` past the end counts as the end.
    pub fn screen_positions(&self, l: usize) -> Vec<usize> {
        let l = l.min(self.logical_len());
        let before = (0..l).rev().find_map(|p| self.trail(p));
        let after = (l..self.logical_len()).find_map(|n| self.lead(n));
        let mut out = Vec::with_capacity(2);
        for v in [before, after].into_iter().flatten() {
            if !out.contains(&v) {
                out.push(v);
            }
        }
        if out.is_empty() {
            out.push(0);
        }
        out
    }

    /// The logical caret positions (`0..=M`) a caret at screen position `v` (`0..=N`) can mean: the
    /// one at the right edge of the character drawn to its left first, then the one at the left
    /// edge of the character drawn to its right (the same when both read in one direction).
    /// `v` past the end counts as the end.
    pub fn logical_positions(&self, v: usize) -> Vec<usize> {
        let v = v.min(self.screen_len());
        // A line break of wrapped text is no character: next to one only the other neighbour counts.
        let real = |i: usize| self.shown.get(i).copied().filter(|l| *l != VIRTUAL);
        let left = v.checked_sub(1).and_then(real).map(|a| {
            // The right edge of `a` is where it ends in a left-to-right run and begins in the other.
            if self.rtl.get(a).copied().unwrap_or(false) { a } else { a.saturating_add(1) }
        });
        let right = real(v).map(|b| if self.rtl.get(b).copied().unwrap_or(false) { b.saturating_add(1) } else { b });
        let mut out = Vec::with_capacity(2);
        for l in [left, right].into_iter().flatten() {
            if !out.contains(&l) {
                out.push(l);
            }
        }
        if out.is_empty() {
            out.push(self.logical_len());
        }
        out
    }

    /// The logical indices, sorted, of the characters drawn at the screen positions `range`
    /// (past the end ignored; a line break that is no character is none). A selection between two
    /// screen positions covers these.
    pub fn logical_of_screen(&self, range: Range<usize>) -> Vec<usize> {
        let end = range.end.min(self.screen_len());
        let start = range.start.min(end);
        let mut set: Vec<usize> = self.shown.get(start..end).unwrap_or_default().iter().copied().filter(|l| *l != VIRTUAL).collect();
        set.sort_unstable();
        set
    }

    /// Whether line `row` of a wrapped text reads right to left (a single line, and a line past
    /// the last, read left to right).
    pub fn row_is_rtl(&self, row: usize) -> bool {
        self.row_rtl.get(row).copied().unwrap_or(false)
    }

    /// For each row but the last, whether it ends in a line break the wrapping made (rather than in a
    /// line separator of the text): the space a wrapped line keeps after its last word is the one the
    /// break replaced.
    pub fn rows_ended_by_wrap(&self) -> Vec<bool> {
        self.drawn.iter().zip(&self.shown).filter(|(c, _)| **c == '\n').map(|(_, shown)| *shown == VIRTUAL).collect()
    }

    /// The logical index of each character of [`Self::shaper`] (so `shaper.chars().count()` entries):
    /// what a caller that lays the text out piece by piece (a style per character of the stored
    /// text) needs to know about the text egui is given. [`VIRTUAL`] for the line break a wrapped
    /// text makes, and for nothing else.
    ///
    /// The shaper text is the screen text with every stretch of right-to-left script put back in
    /// logical order (see `push_for_shaper`); this does the same to the logical indices, so
    /// character `i` of the shaper text is character `shaper_source()[i]` of the stored text,
    /// except that a bracket drawn mirrored in a right-to-left run is the mirror of its source.
    pub fn shaper_source(&self) -> Vec<usize> {
        // The shaper text is built from the drawn characters and nothing else, so one entry for
        // each of them, permuted the way `push_for_shaper` permutes the characters.
        let mut out = Vec::with_capacity(self.shown.len());
        let mut stretch: Vec<usize> = Vec::new();
        for (c, &logical) in self.drawn.iter().zip(&self.shown) {
            if is_shaped_rtl(*c) {
                stretch.push(logical);
            } else {
                out.extend(stretch.drain(..).rev());
                out.push(logical);
            }
        }
        out.extend(stretch.drain(..).rev());
        out
    }
}

/// One line of wrapped text (see [`wrap`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// The line in display order, for one egui call that lays out one line.
    pub text: String,
    /// The paragraph reads right to left: the line belongs against the right edge.
    pub rtl: bool,
}

/// The direction of the first strong character of `text` (as `dir="auto"` reads it): `Some(true)`
/// for right to left, `Some(false)` for left to right, `None` when there is none yet (empty,
/// digits, punctuation). Only the first 512 characters are looked at.
pub fn first_strong_rtl(text: &str) -> Option<bool> {
    let end = text.char_indices().nth(512).map_or(text.len(), |(at, _)| at);
    match unicode_bidi::get_base_direction(text.get(..end)?) {
        unicode_bidi::Direction::Rtl => Some(true),
        unicode_bidi::Direction::Ltr => Some(false),
        unicode_bidi::Direction::Mixed => None,
    }
}

/// Whether `text` holds any right-to-left script, so [`wrap`] or [`visual_auto`] is needed.
pub fn has_rtl(text: &str) -> bool {
    text.chars().any(is_rtl_script)
}

/// Multi-line text with right-to-left script (a comment in Persian, Arabic, Hebrew), broken
/// into lines that `fits` accepts (it measures a line in logical order) and each put in display
/// order by the Unicode bidirectional algorithm (UAX #9).
///
/// Every paragraph takes its direction from its first strong character, as `dir="auto"` does,
/// so a Persian comment reads right to left with its Latin words and numbers left to right
/// inside it. Lines break between words, in reading order, and a word wider than a line
/// between characters. Use it for painting; copying and accessibility keep the logical text.
pub fn wrap(text: &str, mut fits: impl FnMut(&str) -> bool) -> Vec<Line> {
    let info = BidiInfo::new(text, None);
    let mut lines = Vec::new();
    for para in &info.paragraphs {
        let whole = text.get(para.range.clone()).unwrap_or_default();
        // unicode-bidi ends a paragraph at the `\r` of `\r\n`; the `\n` is no new paragraph.
        if whole == "\n" && text.get(..para.range.start).is_some_and(|before| before.ends_with('\r')) {
            continue;
        }
        let content = para.range.start..para.range.start.saturating_add(whole.trim_end_matches(is_paragraph_separator).len());
        let rtl = para.level.is_rtl();
        for line in break_lines(text, content, &mut fits) {
            let mut out = String::new();
            // unicode-bidi indexes the first level of a line, so an empty one would panic.
            if !line.is_empty() {
                push_line(&mut out, text, &info, para, line);
            }
            lines.push(Line { text: out, rtl });
        }
    }
    lines
}

/// One line of `text` in display order, its direction from its first strong character (an
/// author's name). Text without right-to-left characters comes back borrowed and unchanged.
pub fn visual_auto(text: &str) -> Cow<'_, str> {
    if !has_rtl(text) {
        return Cow::Borrowed(text);
    }
    let lines: Vec<String> = wrap(text, |_| true).into_iter().map(|l| l.text).collect();
    Cow::Owned(lines.join("\n"))
}

/// The byte ranges of the lines `range` of `text` breaks into: as many words as `fits` accepts
/// per line, a word too wide for any line split between characters. Always at least one line.
fn break_lines(text: &str, range: Range<usize>, fits: &mut impl FnMut(&str) -> bool) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    let mut current: Option<Range<usize>> = None;
    for word in words(text, range.clone(), false) {
        if let Some(line) = current.as_mut() {
            if fits(text.get(line.start..word.end).unwrap_or_default()) {
                line.end = word.end;
                continue;
            }
            lines.push(line.clone());
        }
        // The word starts a line.
        let mut start = word.start;
        loop {
            let end = fitting_prefix(text, start..word.end, fits);
            if end >= word.end {
                current = Some(start..word.end);
                break;
            }
            lines.push(start..end);
            start = end;
        }
    }
    lines.push(current.unwrap_or(range.start..range.start));
    lines
}

/// The end of the longest start of `range` that `fits`, at least one character.
fn fitting_prefix(text: &str, range: Range<usize>, fits: &mut impl FnMut(&str) -> bool) -> usize {
    let piece = text.get(range.clone()).unwrap_or_default();
    let mut end = range.end;
    for (i, (at, c)) in piece.char_indices().enumerate() {
        let next = range.start.saturating_add(at).saturating_add(c.len_utf8());
        if i > 0 && !fits(text.get(range.start..next).unwrap_or_default()) {
            end = range.start.saturating_add(at);
            break;
        }
    }
    end
}

/// The rows a text's owner has laid it out in, to break the paragraphs of a field where the owner did (see
/// [`RowPlan::new`] and [`RowPlan::fits`]).
pub struct RowPlan {
    /// Where each paragraph (the text between two `\n`) begins in the text.
    starts: Vec<usize>,
    /// The number of words on each row of each paragraph.
    rows: Vec<Vec<usize>>,
    spaces_only: bool,
    /// The last (paragraph, line start) asked about and the number of words before it.
    seen: std::cell::Cell<(usize, usize, usize)>,
}

impl RowPlan {
    /// The plan for `text` from the number of words on each of its rows in order, if they cover the words of each
    /// paragraph exactly (a paragraph without words has no row of its own, or one with none).
    pub fn new(text: &str, rows: &[usize], spaces_only: bool) -> Option<Self> {
        let mut left = rows.iter().copied().peekable();
        let (mut starts, mut plan) = (Vec::new(), Vec::new());
        let mut offset = 0_usize;
        for paragraph in text.split('\n') {
            let total = word_count(paragraph, spaces_only);
            let mut here = Vec::new();
            if total == 0 {
                if left.peek() == Some(&0) {
                    left.next();
                }
            } else {
                let mut sum = 0_usize;
                while sum < total {
                    let n = left.next().filter(|n| *n > 0)?;
                    sum = sum.checked_add(n)?;
                    here.push(n);
                }
                if sum != total {
                    return None;
                }
            }
            starts.push(offset);
            plan.push(here);
            offset = offset.checked_add(paragraph.len())?.checked_add(1)?;
        }
        left.next().is_none().then_some(Self { starts, rows: plan, spaces_only, seen: std::cell::Cell::new((usize::MAX, 0, 0)) })
    }

    /// Whether the line fits its row: it has no more words than the row that begins where it does. A line the plan
    /// knows nothing about fits.
    pub fn fits(&self, candidate: &Candidate<'_>) -> bool {
        let Ok(paragraph) = self.starts.binary_search(&candidate.offset) else { return true };
        let Some(rows) = self.rows.get(paragraph) else { return true };
        let (seen_paragraph, seen_start, seen_first) = self.seen.get();
        let first = if (seen_paragraph, seen_start) == (paragraph, candidate.start) {
            seen_first
        } else {
            let first = candidate.paragraph.get(..candidate.start).map_or(0, |before| word_count(before, self.spaces_only));
            self.seen.set((paragraph, candidate.start, first));
            first
        };
        let mut at = 0_usize;
        for n in rows {
            if at == first {
                return word_count(candidate.line, self.spaces_only) <= *n;
            }
            at = at.saturating_add(*n);
        }
        true
    }
}

/// How many words `text` has: the stretches between whitespace (only the space if `spaces_only`).
pub fn word_count(text: &str, spaces_only: bool) -> usize {
    words(text, 0..text.len(), spaces_only).len()
}

/// The byte ranges of the words of `range` in `text`, which whitespace (only the space if `spaces_only`) separates.
fn words(text: &str, range: Range<usize>, spaces_only: bool) -> Vec<Range<usize>> {
    let mut words = Vec::new();
    let mut start = None;
    for (at, c) in text.get(range.clone()).unwrap_or_default().char_indices() {
        let at = range.start.saturating_add(at);
        match (if spaces_only { c == ' ' } else { c.is_whitespace() }, start) {
            (true, Some(s)) => {
                words.push(s..at);
                start = None;
            }
            (false, None) => start = Some(at),
            _ => {}
        }
    }
    if let Some(s) = start {
        words.push(s..range.end);
    }
    words
}

/// Characters that end a paragraph (bidi class B).
fn is_paragraph_separator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{1C}'..='\u{1E}' | '\u{85}' | '\u{2029}')
}

/// A label of a right-to-left interface language, in logical order, rearranged for egui: each
/// line reads right to left, with Latin words and numbers inside it left to right.
///
/// - `{name}` placeholders stay whole and unmirrored, so [`crate::i18n::fmt`] still fills them.
///   What it inserts is drawn left to right at that place.
/// - Spaces and separator dots at either end stay at that end: the code appends to a label on
///   its right (`"Issued by: "` + a name), as the interface is still laid out left to right.
/// - A short caption's closing colon stays on the right too, next to the widget it captions.
///
/// Text without right-to-left characters comes back unchanged. Like [`visual`], this is for
/// painting one line; a label egui wraps breaks at the wrong end.
pub fn display_rtl(text: &str) -> String {
    if !text.chars().any(is_rtl_script) {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        push_rtl_line(&mut out, line);
    }
    out
}

/// The most words a caption may have for its closing colon to stay on the right.
const CAPTION_WORDS: usize = 3;
/// First of the private-use characters that stand for placeholders while a line is reordered.
/// They are strong left-to-right, like the names they replace.
const PLACEHOLDER_BASE: u32 = 0xE000;
/// Placeholders protected per line; a line with more keeps the rest as plain text.
const MAX_PLACEHOLDERS: usize = 64;

fn push_rtl_line(out: &mut String, line: &str) {
    let edge = |c: char| c.is_whitespace() || c == '·';
    let core = line.trim_start_matches(edge);
    let lead = line.get(..line.len().saturating_sub(core.len())).unwrap_or_default();
    let trimmed = core.trim_end_matches(edge);
    let tail = core.get(trimmed.len()..).unwrap_or_default();
    let mut core = trimmed;
    let mut colon = "";
    if let Some(caption) = core.strip_suffix(':')
        && caption.split_whitespace().count() <= CAPTION_WORDS
    {
        core = caption;
        colon = ":";
    }
    // A line that already uses the stand-in characters is reordered with its placeholders as text.
    let reserved = core.chars().any(|c| (PLACEHOLDER_BASE..PLACEHOLDER_BASE.saturating_add(MAX_PLACEHOLDERS as u32)).contains(&u32::from(c)));
    let (masked, names) = if reserved { (core.to_owned(), Vec::new()) } else { mask_placeholders(core) };
    let mut visual = String::with_capacity(masked.len());
    push_visual(&mut visual, &masked, Level::rtl());
    out.push_str(lead);
    for c in visual.chars() {
        let index = u32::from(c).checked_sub(PLACEHOLDER_BASE).and_then(|i| usize::try_from(i).ok());
        match index.and_then(|i| names.get(i)) {
            Some(name) => out.push_str(name),
            None => out.push(c),
        }
    }
    out.push_str(colon);
    out.push_str(tail);
}

/// `text` with each `{name}` placeholder and each keyboard shortcut (`⇧⌘+`) replaced by one
/// stand-in character, and what they replaced, in order. A shortcut's symbols have no direction
/// of their own, so left alone they would be drawn in reverse.
fn mask_placeholders(text: &str) -> (String, Vec<&str>) {
    let mut masked = String::with_capacity(text.len());
    let mut names: Vec<&str> = Vec::new();
    let mut rest = text;
    while names.len() < MAX_PLACEHOLDERS
        && let Some((start, end)) = next_atom(rest)
    {
        let (Some(before), Some(atom), Some(after)) = (rest.get(..start), rest.get(start..end), rest.get(end..)) else { break };
        // `names.len()` is below MAX_PLACEHOLDERS, so the sum is a private-use scalar.
        let Some(stand_in) = u32::try_from(names.len()).ok().and_then(|i| char::from_u32(PLACEHOLDER_BASE.saturating_add(i))) else { break };
        masked.push_str(before);
        masked.push(stand_in);
        names.push(atom);
        rest = after;
    }
    masked.push_str(rest);
    (masked, names)
}

/// The byte range of the first placeholder or keyboard shortcut in `text`.
fn next_atom(text: &str) -> Option<(usize, usize)> {
    let placeholder = text.find('{').and_then(|open| {
        let len = text.get(open..)?.find('}')?;
        Some((open, open.saturating_add(len).saturating_add(1)))
    });
    match (placeholder, next_shortcut(text)) {
        (Some(p), Some(s)) => Some(if s.0 < p.0 { s } else { p }),
        (p, s) => p.or(s),
    }
}

/// The byte range of the first keyboard shortcut in `text`: the stretch around a modifier symbol
/// up to a space, a bracket or script text.
fn next_shortcut(text: &str) -> Option<(usize, usize)> {
    let at = text.find(is_modifier_key)?;
    let outside = |c: char| c.is_whitespace() || is_rtl_script(c) || matches!(c, '(' | ')' | '[' | ']' | '{' | '}');
    let start = text
        .get(..at)
        .and_then(|before| before.rfind(outside).and_then(|i| before.get(i..)?.chars().next().map(|c| i.saturating_add(c.len_utf8()))))
        .unwrap_or(0);
    let end = text.get(at..).and_then(|after| after.find(outside)).map_or(text.len(), |i| at.saturating_add(i));
    Some((start, end))
}

/// The most shortcuts [`isolate_shortcuts`] isolates in one text; the rest stay as they are.
const MAX_SHORTCUTS: usize = 64;

/// `text` with each keyboard shortcut (`⇧⌘S`, `⌘P`) wrapped in a left-to-right isolate, so the
/// bidirectional algorithm keeps its symbols in order inside a right-to-left paragraph. The
/// isolate characters are not drawn ([`wrap`] drops them). Text without a shortcut comes back
/// borrowed.
pub fn isolate_shortcuts(text: &str) -> Cow<'_, str> {
    let Some(first) = next_shortcut(text) else { return Cow::Borrowed(text) };
    let mut out = String::with_capacity(text.len().saturating_add(text.len() / 4));
    let mut rest = text;
    let mut next = Some(first);
    for _ in 0..MAX_SHORTCUTS {
        let Some((start, end)) = next else { break };
        let (Some(before), Some(shortcut), Some(after)) = (rest.get(..start), rest.get(start..end), rest.get(end..)) else { break };
        out.push_str(before);
        out.push('\u{2066}');
        out.push_str(shortcut);
        out.push('\u{2069}');
        rest = after;
        next = next_shortcut(rest);
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// The modifier-key symbols of keyboard shortcuts: ⌘ ⇧ ⌥ ⌃.
fn is_modifier_key(c: char) -> bool {
    matches!(c, '\u{2318}' | '\u{21E7}' | '\u{2325}' | '\u{2303}')
}

/// Characters of the right-to-left scripts' blocks: letters, marks and the scripts' own
/// punctuation, all drawn by the script's face.
fn is_rtl_script(c: char) -> bool {
    matches!(u32::from(c), 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFC | 0x1_0800..=0x1_0FFF | 0x1_E800..=0x1_EFFF)
}

/// Whether egui hands `c` to the shaper as part of a right-to-left run (whatever level the
/// bidirectional algorithm gave it): the characters of the right-to-left scripts, and the joiners
/// that belong to the letter before them. egui guesses a run's direction from its script, so such a
/// run is shaped right to left, with the glyph accounting that goes with it.
pub(crate) fn is_shaped_rtl(c: char) -> bool {
    is_rtl_script(c) || is_joiner(c)
}

/// Whether `c` belongs to a script whose letters join their neighbours (Arabic and the scripts written
/// like it), so that a font must shape a stretch of them as one: Hebrew, whose letters stand alone, does
/// not. Includes the joiners inside a word.
pub(crate) fn is_joining_script(c: char) -> bool {
    matches!(u32::from(c), 0x0600..=0x08FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFC) || is_joiner(c)
}

/// The zero-width (non-)joiners inside a word (Persian `می‌روم`): egui shapes them with the
/// letter before them.
fn is_joiner(c: char) -> bool {
    matches!(c, '\u{200C}' | '\u{200D}')
}

/// Direction marks, embeddings, overrides and isolates: they steer the reordering and are not drawn.
pub fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

fn mirrored(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        other => other,
    }
}

/// The interface fonts with an Arabic face, or `None` when this build has none (no craft-fonts)
/// and `PDFCRAFT_TEST_ARABIC_FONT` does not name one either. That variable is for a local run
/// without craft-fonts: any font file with Arabic coverage stands in (DejaVu Sans, say), and
/// the tests that need glyph positions then run instead of skipping.
#[cfg(test)]
pub(crate) fn arabic_test_fonts() -> Option<egui::FontDefinitions> {
    let mut fonts = crate::theme::font_definitions();
    if !pdfcraft_fonts::ui_arabic_fonts().is_empty() {
        return Some(fonts);
    }
    let bytes = std::fs::read(std::env::var_os("PDFCRAFT_TEST_ARABIC_FONT")?).ok()?;
    fonts.font_data.insert("test-arabic".to_owned(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
    for stack in fonts.families.values_mut() {
        stack.push("test-arabic".to_owned());
    }
    Some(fonts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_field_line_reads_in_the_direction_of_its_first_strong_character() {
        let drawn = |text: &str, neutral_rtl: bool, forced_rtl: Option<bool>| -> String {
            line_screen_map(text, neutral_rtl, forced_rtl).drawn.into_iter().collect()
        };
        // "ali abc": right to left, so the Latin word that ends it is at the left end and the first
        // letter of the Arabic word (ain) at the right.
        let ali_abc = "\u{639}\u{644}\u{64A} abc";
        assert!(line_rtl(ali_abc, false, None));
        assert_eq!(drawn(ali_abc, false, None), "abc \u{64A}\u{644}\u{639}");
        let map = line_screen_map(ali_abc, false, None);
        assert_eq!(map.screen_positions(0), [7], "the start of reading is the right edge");
        assert_eq!(map.screen_positions(7), [3], "its end is after the c");
        assert_eq!(map.logical_positions(7), [0]);
        // The interface's direction does not matter once a letter has one.
        assert_eq!(drawn(ali_abc, true, None), "abc \u{64A}\u{644}\u{639}");
        // Latin first: left to right, the same map as before.
        let abc_ali = "abc \u{639}\u{644}\u{64A}";
        assert!(!line_rtl(abc_ali, true, None));
        assert_eq!(line_screen_map(abc_ali, true, None), screen_map(abc_ali));
        assert_eq!(drawn(abc_ali, false, None), "abc \u{64A}\u{644}\u{639}");
        // No strong character (digits, a full stop): the interface's direction.
        let digits = "\u{661}\u{662}.";
        assert_eq!(drawn(digits, false, None), "\u{661}\u{662}.");
        assert_eq!(drawn(digits, true, None), ".\u{661}\u{662}");
        assert!(!line_rtl("2024", false, None) && !line_rtl("", false, None));
        assert!(line_rtl("", true, None), "an empty field in Arabic reads from the right");
        assert_eq!(line_screen_map("", true, None), ScreenMap::identity(""));
        assert_eq!(line_screen_map("2024", true, None), ScreenMap::identity("2024"), "nothing right to left: drawn as stored");
        // The owner's word wins.
        assert_eq!(drawn(ali_abc, false, Some(false)), "\u{64A}\u{644}\u{639} abc");
        assert_eq!(drawn(abc_ali, false, Some(true)), "\u{64A}\u{644}\u{639} abc");
        // Whatever the text, every character is drawn once and maps back to itself.
        for text in [ali_abc, abc_ali, digits, "\u{202B}\u{639}\u{202C} x", "\u{639}\nab", "( \u{639} [b] )"] {
            for neutral in [false, true] {
                let map = line_screen_map(text, neutral, None);
                for (v, l) in map.shown.iter().enumerate() {
                    assert_eq!(map.to_screen.get(*l).copied().flatten(), Some(v), "{text:?}");
                }
            }
        }
    }

    #[test]
    fn text_without_rtl_is_borrowed_unchanged() {
        for s in ["", "report.pdf", "日本語の文字.pdf", "a (b) [c]", "caf\u{00E9} \u{200E}x"] {
            assert!(matches!(visual(s), Cow::Borrowed(v) if v == s), "{s:?}");
        }
    }

    #[test]
    fn arabic_words_swap_and_the_extension_stays_last() {
        // The reported name: "one two.pdf" in Arabic.
        assert_eq!(visual("واحد اثنين.pdf"), "اثنين واحد.pdf");
        assert_eq!(visual("ملف.pdf"), "ملف.pdf");
        assert_eq!(visual("تقرير نهائي جدا.pdf"), "جدا نهائي تقرير.pdf");
    }

    #[test]
    fn latin_and_numbers_keep_their_own_order() {
        assert_eq!(visual("report واحد اثنين final.pdf"), "report اثنين واحد final.pdf");
        // A number inside right-to-left text reads left to right, to the left of what precedes it.
        assert_eq!(visual("ملف 2024.pdf"), "2024 ملف.pdf");
        assert_eq!(visual(r"C:\Users\me\واحد اثنين\a.pdf"), r"C:\Users\me\اثنين واحد\a.pdf");
    }

    #[test]
    fn hebrew_and_brackets() {
        assert_eq!(visual("שלום עולם.pdf"), "עולם שלום.pdf");
        // Brackets between right-to-left words flip with the direction.
        assert_eq!(visual("واحد (اثنين) ثلاثة.pdf"), "ثلاثة (اثنين) واحد.pdf");
    }

    #[test]
    fn direction_controls_are_not_drawn() {
        assert_eq!(visual("\u{202B}واحد اثنين\u{202C}.pdf"), "اثنين واحد.pdf");
        assert_eq!(visual("\u{200F}واحد\u{061C}"), "واحد");
    }

    #[test]
    fn rtl_labels_read_right_to_left() {
        // No right-to-left characters: unchanged.
        for s in ["", "Save as…", "{n} pages", "日本語"] {
            assert_eq!(display_rtl(s), s);
        }
        assert_eq!(display_rtl("حفظ"), "حفظ");
        // "Save as…": the second word and the ellipsis end up on the left.
        assert_eq!(display_rtl("حفظ باسم…"), "…باسم حفظ");
        // Latin words and numbers keep their own order inside the line.
        assert_eq!(display_rtl("إنشاء PDF من ملف"), "ملف من PDF إنشاء");
        assert_eq!(display_rtl("تصدير إلى Microsoft Word الآن"), "الآن Microsoft Word إلى تصدير");
        assert_eq!(display_rtl("صالح لمدة 5 سنوات"), "سنوات 5 لمدة صالح");
        // Keyboard shortcuts are not reversed.
        assert_eq!(display_rtl("طباعة (⌘P)"), "(⌘P) طباعة");
        assert_eq!(display_rtl("تدوير العرض (⇧⌘+)"), "(⇧⌘+) العرض تدوير");
        assert_eq!(display_rtl("التمرير مع ⌘ أيضًا"), "أيضًا ⌘ مع التمرير");
        assert_eq!(display_rtl("نص (عادي)"), "(عادي) نص");
    }

    #[test]
    fn rtl_labels_keep_placeholders_whole() {
        // Two placeholders around a colon stay together, left to right: a file name and its error.
        assert_eq!(display_rtl("تعذّر فتح {name}: {e}"), "{name}: {e} فتح تعذّر");
        assert_eq!(display_rtl("فشل {label}، السبب {e}"), "{e} السبب ،{label} فشل");
        assert_eq!(display_rtl("{n} صفحات"), "صفحات {n}");
        assert_eq!(display_rtl("الصفحة {p} من {n}"), "{n} من {p} الصفحة");
        // Adjacent placeholders keep their order, as numbers around a slash would.
        assert_eq!(display_rtl("جارٍ البحث… {s}/{p}"), "{s}/{p} …البحث جارٍ");
        // Braces that are not a placeholder are ordinary punctuation.
        assert_eq!(display_rtl("حرف } واحد"), "واحد { حرف");
        let many = format!("ا {}", "{a} ".repeat(200));
        assert_eq!(display_rtl(&many).matches("{a}").count(), 200);
    }

    #[test]
    fn rtl_labels_keep_their_edges() {
        // Edge spaces and separator dots stay where the code concatenates.
        assert_eq!(display_rtl(" (صفحة {p})"), " ({p} صفحة)");
        assert_eq!(display_rtl(" · محمي بكلمة مرور"), " · مرور بكلمة محمي");
        assert_eq!(display_rtl("سلسلة المفاتيح  ·  "), "المفاتيح سلسلة  ·  ");
        // A caption's colon stays next to its widget; a sentence's colon ends the sentence.
        assert_eq!(display_rtl("الاسم:"), "الاسم:");
        assert_eq!(display_rtl("صادرة عن: "), "عن صادرة: ");
        assert_eq!(display_rtl("اختر المعرّف الرقمي الذي تريد استخدامه:"), ":استخدامه تريد الذي الرقمي المعرّف اختر");
        // Each line is its own paragraph.
        assert_eq!(display_rtl("تعذّر عرض الصفحة.\n{e}"), ".الصفحة عرض تعذّر\n{e}");
    }

    #[test]
    fn rtl_labels_never_panic_on_odd_input() {
        let long = "ا {n} ".repeat(20_000);
        for s in [
            "ا{",
            "ا}",
            "ا{}",
            "ا{{{",
            "}ا{",
            "ا\u{E000}{n}",
            "⌘",
            "ا ⌘",
            "⌘ا⌘{",
            "ا (⇧⌘",
            "\u{064B}:",
            ":",
            "ا:",
            " : ا : ",
            "ا\n\n\nب",
            "\u{202E}ا{n}",
            long.as_str(),
        ] {
            let v = display_rtl(s);
            assert!(v.chars().count() <= s.chars().count(), "{:?}", s.chars().take(12).collect::<String>());
        }
    }

    /// `wrap` with a width of `n` characters.
    fn wrap_at(text: &str, n: usize) -> Vec<Line> {
        wrap(text, |s| s.chars().count() <= n)
    }

    fn rtl(text: &str) -> Line {
        Line { text: text.to_owned(), rtl: true }
    }

    fn ltr(text: &str) -> Line {
        Line { text: text.to_owned(), rtl: false }
    }

    #[test]
    fn persian_paragraph_reads_right_to_left() {
        // "I go to school": the words come out in reverse for egui's left-to-right placement.
        assert_eq!(wrap_at("من به مدرسه می‌روم", 80), [rtl("می‌روم مدرسه به من")]);
        // The zero-width non-joiner keeps a word whole, so the shaper gets it as one run.
        assert_eq!(wrap_at("کتاب‌ها", 80), [rtl("کتاب‌ها")]);
    }

    #[test]
    fn arabic_lines_break_in_reading_order() {
        // The first words go on the first line, each line reordered on its own.
        assert_eq!(wrap_at("واحد اثنين ثلاثة أربعة", 11), [rtl("اثنين واحد"), rtl("أربعة ثلاثة")]);
        assert_eq!(wrap_at("واحد اثنين ثلاثة أربعة", 80), [rtl("أربعة ثلاثة اثنين واحد")]);
    }

    #[test]
    fn base_direction_comes_from_the_first_strong_character() {
        // Latin first: a left-to-right paragraph with an Arabic phrase inside it.
        assert_eq!(wrap_at("See واحد اثنين here", 80), [ltr("See اثنين واحد here")]);
        // Persian first: right to left, the Latin words and the number left to right inside it.
        assert_eq!(wrap_at("فایل PDF نسخه 2 است", 80), [rtl("است 2 نسخه PDF فایل")]);
        assert_eq!(wrap_at("سلام Hello World", 80), [rtl("Hello World سلام")]);
        // Leading neutrals and numbers don't decide it.
        assert_eq!(wrap_at("12. واحد اثنين", 80), [rtl("اثنين واحد .12")]);
        // Each paragraph finds its own direction.
        assert_eq!(wrap_at("واحد اثنين\nOK then\n\nשלום עולם", 80), [rtl("اثنين واحد"), ltr("OK then"), ltr(""), rtl("עולם שלום")]);
    }

    #[test]
    fn rtl_punctuation_and_brackets() {
        // The sentence's full stop ends up on the left, brackets flip.
        assert_eq!(wrap_at("سلام دنیا.", 80), [rtl(".دنیا سلام")]);
        assert_eq!(wrap_at("واحد (اثنين) ثلاثة", 80), [rtl("ثلاثة (اثنين) واحد")]);
        assert_eq!(wrap_at("آیا آماده است؟", 80), [rtl("است؟ آماده آیا")]);
    }

    #[test]
    fn persian_digits_go_to_the_shaper_reversed() {
        // Persian and Arabic-Indic digits are Arabic script, which egui's shaper lays out right to
        // left; they read left to right, so they are handed over reversed.
        assert_eq!(wrap_at("سال ۱۴۰۲ شمسی", 80), [rtl("شمسی ۲۰۴۱ سال")]);
        assert_eq!(wrap_at("صفحه ١٢", 80), [rtl("٢١ صفحه")]);
        // Glued to a word, number and word swap places within one shaped run.
        assert_eq!(wrap_at("ب۱۲", 80), [rtl("ب۲۱")]);
    }

    #[test]
    fn wrapping_handles_odd_widths_and_input() {
        assert_eq!(wrap_at("", 10), Vec::<Line>::new());
        // A word wider than the line is broken between characters.
        assert_eq!(wrap_at("abcdefghij واحد", 4), [ltr("abcd"), ltr("efgh"), ltr("ij"), ltr("واحد")]);
        // Extra spaces between words don't make empty lines; \r\n is one break.
        assert_eq!(wrap_at("واحد    اثنين\r\nب", 5), [rtl("واحد"), rtl("اثنين"), rtl("ب")]);
        // Nothing fits: one character per line, never an endless loop.
        assert_eq!(wrap("واحد", |_| false).len(), 4);
        let long = "ا ب ۱ (ج) ".repeat(5_000);
        for s in ["\u{202E}ا\u{202C}b", "\u{064B}", "\n\n", "\u{2029}ا", long.as_str()] {
            let lines = wrap_at(s, 7);
            let shown: usize = lines.iter().map(|l| l.text.chars().count()).sum();
            assert!(shown <= s.chars().count(), "{:?}", s.chars().take(12).collect::<String>());
        }
    }

    /// The characters egui puts on screen for each line of `text`, left to right, zero-width
    /// ones left out, with the interface fonts.
    fn on_screen(text: &str) -> Vec<String> {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::font_definitions());
        let mut shown = Vec::new();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            for line in wrap(text, |_| true) {
                let galley = ui.painter().layout_no_wrap(line.text, egui::FontId::proportional(13.0), egui::Color32::WHITE);
                let glyphs = galley.rows.iter().flat_map(|row| row.glyphs.iter());
                shown.push(glyphs.filter(|g| g.advance_width > 0.0).map(|g| g.chr).collect());
            }
        });
        output.drop_without_applying_deltas();
        shown
    }

    fn rev(s: &str) -> String {
        s.chars().rev().collect()
    }

    #[test]
    fn egui_draws_lines_in_reading_order() {
        // Only with an Arabic-script face (the craft-fonts build input, as in CI); without one
        // egui has no shaper run to check.
        if pdfcraft_fonts::ui_arabic_fonts().is_empty() {
            return;
        }
        assert_eq!(on_screen("من به مدرسه می\u{200C}روم"), [rev("من به مدرسه میروم")]);
        assert_eq!(on_screen("فایل PDF نسخه 2 است"), [format!("{} 2 {} PDF {}", rev("است"), rev("نسخه"), rev("فایل"))]);
        assert_eq!(on_screen("سال ۱۴۰۲ شمسی"), [format!("{} ۱۴۰۲ {}", rev("شمسی"), rev("سال"))]);
        assert_eq!(on_screen("ب۱۲"), ["۱۲ب"]);
        assert_eq!(on_screen("See واحد اثنين here"), [format!("See {} {} here", rev("اثنين"), rev("واحد"))]);
        assert_eq!(on_screen("واحد (اثنين)."), [format!(".({}) {}", rev("اثنين"), rev("واحد"))]);
    }

    #[test]
    fn shortcuts_keep_their_order_inside_a_right_to_left_paragraph() {
        let isolated = isolate_shortcuts("احفظ الملف ⇧⌘S");
        assert!(isolated.contains('\u{2066}') && isolated.contains('\u{2069}'));
        let lines = wrap(&isolated, |_| true);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].rtl);
        assert!(lines[0].text.contains("⇧⌘S"), "{:?}", lines[0].text);
        assert!(!lines[0].text.chars().any(is_bidi_control), "{:?}", lines[0].text);
        // The shortcut sits at the left, the end of right-to-left reading.
        assert!(lines[0].text.starts_with("⇧⌘S"), "{:?}", lines[0].text);
        // Without the isolate the symbols are not guaranteed to stay in order; with it, several
        // shortcuts each stay whole.
        let two = isolate_shortcuts("طباعة (⌘P) أو ⇧⌘S الآن");
        let lines = wrap(&two, |_| true);
        assert!(lines[0].text.contains("⌘P") && lines[0].text.contains("⇧⌘S"), "{:?}", lines[0].text);
        assert!(matches!(isolate_shortcuts("plain"), Cow::Borrowed("plain")));
        assert!(matches!(isolate_shortcuts("نص عادي"), Cow::Borrowed(_)));
        assert!(matches!(isolate_shortcuts(""), Cow::Borrowed("")));
    }

    #[test]
    fn isolating_shortcuts_is_bounded_and_total() {
        let many = "⌘A ".repeat(1_000);
        let isolated = isolate_shortcuts(&many);
        assert_eq!(isolated.matches('\u{2066}').count(), MAX_SHORTCUTS);
        assert!(isolated.ends_with("⌘A "));
        for s in ["⌘", "ا⌘", "⌘ا", "(⇧⌘", "⌘⌘⌘", "\u{2066}⌘\u{2069}", "{⌘}"] {
            let v = isolate_shortcuts(s);
            assert!(v.chars().filter(|c| !is_bidi_control(*c)).eq(s.chars().filter(|c| !is_bidi_control(*c))), "{s:?}");
            let _ = wrap(&v, |_| true);
        }
    }

    #[test]
    fn single_line_with_its_own_direction() {
        assert!(matches!(visual_auto("Ada Lovelace"), Cow::Borrowed("Ada Lovelace")));
        assert_eq!(visual_auto("علی رضایی"), "رضایی علی");
        assert_eq!(visual_auto("علی 2"), "2 علی");
        assert_eq!(visual_auto("Team علی رضایی"), "Team رضایی علی");
    }

    #[test]
    fn odd_input_never_panics() {
        let long = "ا ب ".repeat(20_000);
        let odd = [
            "\u{064B}",
            "\u{064B}\u{064C} \u{0301}",
            "\u{202E}\u{202E}\u{202E}ا",
            "\u{2067}ا\u{2066}b",
            "ا\nب\r\nج\u{2029}د",
            "\u{FEFF}ا\u{0000}ب",
            "ا\u{10FFFF}\u{E000}ب",
            "((((ا]]]]",
            long.as_str(),
        ];
        for s in odd {
            let v = visual(s);
            assert!(v.chars().count() <= s.chars().count(), "{:?}", s.chars().take(12).collect::<String>());
        }
    }

    /// Fixed odd strings plus pseudo-random mixes of the characters that steer reordering.
    fn corpus() -> Vec<String> {
        let mut out: Vec<String> = [
            "",
            "abc",
            "ab\u{643}\u{62A}\u{627}\u{628}",
            "\u{648}\u{627}\u{62D}\u{62F} \u{627}\u{62B}\u{646}\u{64A}\u{646}.pdf",
            "report \u{648}\u{627}\u{62D}\u{62F} final",
            "\u{645}\u{644}\u{641} 2024",
            "\u{202B}\u{648}\u{627}\u{62D}\u{62F}\u{202C}.pdf",
            "\u{200F}\u{648}\u{627}\u{62D}\u{62F}\u{61C}",
            "\u{64B}",
            "\u{202E}\u{202E}\u{202E}\u{627}",
            "\u{2067}\u{627}\u{2066}b",
            "\u{627}\nb\r\n\u{62C}\u{2029}\u{62F}",
            "\u{FEFF}\u{627}\u{0}\u{628}",
            "((((\u{627}]]]]",
            "\u{648}\u{627}\u{62D}\u{62F} (\u{627}\u{62B}\u{646}\u{64A}\u{646}) \u{62B}\u{644}\u{627}\u{62B}\u{629}",
            "\u{645}\u{646} \u{628}\u{647} \u{645}\u{62F}\u{631}\u{633}\u{647} \u{645}\u{6CC}\u{200C}\u{631}\u{648}\u{645}",
            "\u{5E9}\u{5DC}\u{5D5}\u{5DD} \u{5E2}\u{5D5}\u{5DC}\u{5DD}",
            "\u{633}\u{627}\u{644} \u{6F1}\u{6F4}\u{6F0}\u{6F2} \u{634}\u{645}\u{633}\u{6CC}",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let alphabet: Vec<char> =
            "ab 1\u{6F1}\u{639}\u{644}\u{64A}\u{628}()[]\u{200C}\u{200E}\u{200F}\u{202B}\u{202C}\u{2066}\u{2069}\u{64B}\n\u{5D0}.,:"
                .chars()
                .collect();
        let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
        for len in (0..40).chain([200]) {
            let mut text = String::new();
            for _ in 0..len {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                if let Some(c) = usize::try_from(seed % alphabet.len() as u64).ok().and_then(|i| alphabet.get(i)) {
                    text.push(*c);
                }
            }
            out.push(text);
        }
        out
    }

    #[test]
    fn the_screen_map_agrees_with_visual() {
        for text in corpus() {
            let map = screen_map(&text);
            let m = text.chars().count();
            let n = map.shaper.chars().count();
            assert_eq!(map.shaper, visual(&text), "{text:?}");
            assert_eq!(map.shown.len(), n, "{text:?}");
            assert_eq!(map.drawn.len(), n, "{text:?}");
            assert!(n <= m && map.logical_len() == m && map.rtl.len() == m, "{text:?}");
            // `shown` and `to_screen` are inverse to each other.
            for (screen, &logical) in map.shown.iter().enumerate() {
                assert!(logical < m, "{text:?}");
                assert_eq!(map.to_screen.get(logical), Some(&Some(screen)), "{text:?}");
            }
            assert_eq!(map.to_screen.iter().flatten().count(), n, "{text:?}");
            // What is not drawn is exactly the direction controls.
            for (logical, c) in text.chars().enumerate() {
                let drawn = map.to_screen.get(logical).copied().flatten().is_some();
                assert!(drawn || is_bidi_control(c), "{text:?} drops {c:?}");
                assert!(!drawn || !is_bidi_control(c) || !has_rtl(&text), "{text:?}");
            }
        }
    }

    #[test]
    fn a_latin_word_then_an_arabic_one_has_two_places_for_the_caret_between_them() {
        // "ab" and the four letters kaf, ta, alef, ba: the word is drawn reversed after "ab".
        let map = screen_map("ab\u{643}\u{62A}\u{627}\u{628}");
        assert_eq!(map.shown, [0, 1, 5, 4, 3, 2]);
        assert_eq!(map.to_screen, [Some(0), Some(1), Some(5), Some(4), Some(3), Some(2)]);
        assert_eq!(map.rtl, [false, false, true, true, true, true]);
        // Logical 2 (between "ab" and the word) is at the right edge of "b" and at the right edge
        // of the word; logical 6 (the end of the text) is at the right edge of "b" too, where the
        // next letter typed appears.
        assert_eq!(map.screen_positions(2), [2, 6]);
        assert_eq!(map.screen_positions(6), [2]);
        assert_eq!(map.screen_positions(0), [0]);
        assert_eq!(map.screen_positions(1), [1]);
        // Inside the word the neighbours are adjacent on screen.
        assert_eq!(map.screen_positions(3), [5]);
        assert_eq!(map.screen_positions(4), [4]);
        assert_eq!(map.screen_positions(5), [3]);
        // And back: boundary 2 is logical 2 (after "b") or 6 (after the word's last letter).
        assert_eq!(map.logical_positions(2), [2, 6]);
        assert_eq!(map.logical_positions(6), [2]);
        assert_eq!(map.logical_positions(0), [0]);
        assert_eq!(map.logical_positions(1), [1]);
        assert_eq!(map.logical_positions(3), [5]);
        // Past the end counts as the end.
        assert_eq!(map.screen_positions(99), [2]);
        assert_eq!(map.logical_positions(99), [2]);
        // The characters a selection between screen positions 1 and 4 covers: "b" and the two
        // letters drawn after it, which are the word's last two.
        assert_eq!(map.logical_of_screen(1..4), [1, 4, 5]);
        assert_eq!(map.logical_of_screen(4..99), [2, 3]);
        let reversed = std::ops::Range { start: 5, end: 2 };
        assert!(map.logical_of_screen(reversed).is_empty());
    }

    #[test]
    fn text_without_right_to_left_script_maps_to_itself() {
        let map = screen_map("a\u{200E}b");
        assert_eq!(map.shaper, "a\u{200E}b");
        assert_eq!(map.shown, [0, 1, 2]);
        assert_eq!(map.screen_positions(2), [2]);
        assert_eq!(map.logical_positions(2), [2]);
        assert_eq!(screen_map("").screen_positions(0), [0]);
        assert_eq!(screen_map("").logical_positions(0), [0]);
    }

    #[test]
    fn caret_positions_round_trip_on_every_text() {
        for text in corpus() {
            let map = screen_map(&text);
            let (m, n) = (map.logical_len(), map.screen_len());
            for v in 0..=n {
                let logical = map.logical_positions(v);
                assert!(!logical.is_empty() && logical.len() <= 2, "{text:?} {v}");
                for l in logical {
                    assert!(l <= m, "{text:?} {v} -> {l}");
                    assert!(map.screen_positions(l).contains(&v), "{text:?}: screen {v} -> logical {l} -> {:?}", map.screen_positions(l));
                }
            }
            for l in 0..=m {
                let screen = map.screen_positions(l);
                assert!(!screen.is_empty() && screen.len() <= 2, "{text:?} {l}");
                assert!(screen.iter().all(|&v| v <= n), "{text:?} {l} -> {screen:?}");
            }
        }
    }

    #[test]
    fn screen_maps_survive_odd_and_long_input() {
        let long = "\u{627} \u{628}1 ".repeat(20_000);
        for text in [long.as_str(), "\u{64B}", "\u{64B}\u{64C} \u{301}", "\u{202E}", "\u{200F}", "\n\n", "\u{10FFFF}\u{627}\u{E000}"] {
            let map = screen_map(text);
            assert!(map.screen_len() <= map.logical_len(), "{:?}", text.chars().take(8).collect::<String>());
            assert_eq!(map.shaper, visual(text));
            let end = map.logical_len();
            assert!(!map.screen_positions(end).is_empty() && !map.logical_positions(map.screen_len()).is_empty());
            assert!(map.logical_of_screen(0..usize::MAX).len() == map.screen_len());
            let _ = (map.screen_positions(usize::MAX), map.logical_positions(usize::MAX));
        }
        // Direction controls are not drawn, so a text with them has fewer characters on screen;
        // without right-to-left script nothing is reordered or dropped.
        let wrapped = screen_map("\u{202B}\u{627}\u{202C}");
        assert_eq!((wrapped.screen_len(), wrapped.logical_len()), (1, 3));
        assert_eq!((wrapped.shown.clone(), wrapped.to_screen.clone()), (vec![1], vec![None, Some(0), None]));
        // Both ends of the text are at the one drawn character's edges: the right edge of an
        // Arabic letter is where it begins.
        assert_eq!(wrapped.screen_positions(0), [1]);
        assert_eq!(wrapped.screen_positions(3), [0]);
        let plain = screen_map("\u{202B}\u{202C}");
        assert_eq!((plain.screen_len(), plain.logical_len()), (2, 2));
    }

    /// A measure that lets `n` characters on a line (without the spaces after the last word).
    fn at_most(n: usize) -> impl FnMut(&str) -> bool {
        move |line: &str| line.chars().count() <= n
    }

    #[test]
    fn a_wrapped_arabic_paragraph_has_two_places_for_the_caret_at_the_break_and_reads_from_the_right() {
        // "abj dhw" (alef ba jim, space, dal heh waw): two words of three letters.
        let text = "\u{627}\u{628}\u{62C} \u{62F}\u{647}\u{648}";
        let map = wrapped_screen_map(text, false, None, at_most(3));
        // Row 0: the space (reading ends there, so it is at the left), then the word's letters
        // from the last to the first; the break; row 1: the second word the same way.
        assert_eq!(map.shown, [3, 2, 1, 0, VIRTUAL, 6, 5, 4]);
        assert_eq!(map.drawn[4], '\n');
        assert_eq!(map.row_rtl, [true, true]);
        assert_eq!(map.to_screen, [Some(3), Some(2), Some(1), Some(0), Some(7), Some(6), Some(5)]);
        assert_eq!((map.logical_len(), map.screen_len()), (7, 8), "the break is drawn and is no character");
        assert_eq!(map.shaper.matches('\n').count(), 1);
        // The place between the rows in reading order is the left end of row 0 and the right end of row 1.
        assert_eq!(map.screen_positions(4), [0, 8]);
        assert_eq!(map.logical_positions(0), [4]);
        assert_eq!(map.logical_positions(8), [4]);
        // The right end of row 0 is the start of the text; the left end of row 1 is the end of it.
        assert_eq!(map.logical_positions(4), [0]);
        assert_eq!(map.logical_positions(5), [7]);
        assert_eq!(map.screen_positions(0), [4]);
        assert_eq!(map.screen_positions(7), [5]);
        // A selection across the break holds the characters on both sides and no more.
        assert_eq!(map.logical_of_screen(2..7), [0, 1, 5, 6]);
    }

    #[test]
    fn a_wrapped_latin_paragraph_keeps_its_spaces_and_has_two_screen_places_at_the_break() {
        let map = wrapped_screen_map("ab cd ef", false, None, at_most(5));
        assert_eq!(map.shaper, "ab cd \nef");
        assert_eq!(map.shown, [0, 1, 2, 3, 4, 5, VIRTUAL, 6, 7]);
        assert_eq!(map.row_rtl, [false, false]);
        // Logical 6 is the end of the first row and the start of the second.
        assert_eq!(map.screen_positions(6), [6, 7]);
        assert_eq!(map.logical_positions(6), [6]);
        assert_eq!(map.logical_positions(7), [6]);
        // A word wider than a line breaks between its characters.
        let long = wrapped_screen_map("abcdefgh", false, None, at_most(3));
        assert_eq!(long.shaper, "abc\ndef\ngh");
        assert_eq!(long.logical_len(), 8);
        assert_eq!(long.logical_of_screen(0..long.screen_len()), (0..8).collect::<Vec<_>>());
        // Spaces at the ends of the text and a space-only paragraph stay.
        let spaces = wrapped_screen_map("  ab   ", false, None, at_most(9));
        assert_eq!(spaces.shaper, "  ab   ");
        assert_eq!(wrapped_screen_map("   ", false, None, at_most(1)).shaper, "   ");
    }

    #[test]
    fn wrap_rules_keep_long_words_whole_and_end_words_at_spaces_only() {
        let rules = WrapRules { whole_words: true, spaces_only: true };
        let with = |text: &str, rules: WrapRules, n: usize| {
            wrapped_screen_map_with(text, false, None, rules, |c| c.line.trim_matches(' ').chars().count() <= n).shaper
        };
        // A word wider than a line stays whole on a line of its own, where it is broken by default.
        assert_eq!(with("ab abcdefgh cd", rules, 3), "ab \nabcdefgh \ncd");
        assert_eq!(with("ab abcdefgh cd", WrapRules::default(), 3), "ab \nabc\ndef\ngh \ncd");
        // Only the space ends a word: a no-break space and a tab do not, by default they do.
        assert_eq!(with("ab c\u{a0}d ef", rules, 4), "ab \nc\u{a0}d \nef");
        assert_eq!(with("ab c\u{a0}d ef", WrapRules::default(), 4), "ab c\u{a0}\nd ef");
        assert_eq!(with("a\tb c", rules, 2), "a\tb \nc");
        // The rules change nothing for text whose words fit.
        for n in [3, 5, 9] {
            assert_eq!(with("ab cd ef gh", rules, n), with("ab cd ef gh", WrapRules::default(), n));
        }
        // Every character is still drawn once.
        let map = wrapped_screen_map_with("ab abcdefgh cd", false, None, rules, at_most_in(3));
        assert_eq!(map.logical_of_screen(0..map.screen_len()), (0..14).collect::<Vec<_>>());
    }

    /// [`at_most`], for a measure that is given the candidate line.
    fn at_most_in(n: usize) -> impl FnMut(Candidate<'_>) -> bool {
        move |c| c.line.trim_matches(' ').chars().count() <= n
    }

    #[test]
    fn a_line_is_measured_knowing_its_paragraph() {
        let text = "one two three\n\nfour five six seven";
        let mut seen: Vec<(String, String)> = Vec::new();
        let map = wrapped_screen_map_with(text, false, None, WrapRules::default(), |c| {
            seen.push((c.line.to_owned(), c.paragraph.to_owned()));
            // Where the paragraph and the line begin: the line is that part of the text.
            assert!(text[c.offset..].starts_with(c.paragraph), "{c:?}");
            assert!(c.paragraph[c.start..].starts_with(c.line), "{c:?}");
            c.line.chars().count() <= 9
        });
        assert_eq!(word_count("  one  two ", false), 2);
        assert_eq!(word_count("a\u{a0}b c", true), 2);
        assert_eq!(word_count("a\u{a0}b c", false), 3);
        assert_eq!(map.shaper, "one two \nthree\n\nfour five \nsix seven");
        assert!(!seen.is_empty());
        for (line, paragraph) in &seen {
            assert!(paragraph == "one two three" || paragraph == "four five six seven", "{paragraph:?}");
            assert!(paragraph.contains(line.as_str()), "{line:?} is part of {paragraph:?}");
        }
        // The paragraph is the one the line is in, without the separator that ends it.
        assert!(seen.iter().any(|(line, paragraph)| line == "six seven" && paragraph == "four five six seven"));
    }

    #[test]
    fn the_rows_a_wrap_ended_are_told_from_those_a_line_break_of_the_text_ended() {
        let map = wrapped_screen_map("ab cd ef\ngh", false, None, at_most(5));
        assert_eq!(map.shaper, "ab cd \nef\ngh");
        assert_eq!(map.rows_ended_by_wrap(), [true, false]);
        assert!(wrapped_screen_map("abc", false, None, |_| true).rows_ended_by_wrap().is_empty());
        assert_eq!(ScreenMap::identity("a\nb").rows_ended_by_wrap(), [false], "an identity map draws the text's own break");
    }

    #[test]
    fn paragraphs_end_at_the_texts_own_line_breaks_and_follow_their_first_strong_letter() {
        // The break is a character of the text and is drawn.
        let map = wrapped_screen_map("ab\n\u{627}\u{628}\n", false, None, |_| true);
        assert_eq!(map.shaper, "ab\n\u{627}\u{628}\n", "the letters of a right-to-left word go to the shaper in typing order");
        assert_eq!(map.shown, [0, 1, 2, 4, 3, 5]);
        // Left to right, right to left, and the empty line after the last break goes with the paragraph above it.
        assert_eq!(map.row_rtl, [false, true, true]);
        assert_eq!(map.screen_len(), 6);
        // A paragraph without a strong letter takes the direction of the one before it, or the interface's.
        assert_eq!(wrapped_screen_map("12\n\n\u{627}\n3", false, None, |_| true).row_rtl, [false, false, true, true]);
        assert_eq!(wrapped_screen_map("12\n3", true, None, |_| true).row_rtl, [true, true]);
        // `\r\n` is one break: the `\n` is drawn nowhere.
        let crlf = wrapped_screen_map("a\r\nb", false, None, |_| true);
        assert_eq!((crlf.shaper.as_str(), crlf.shown.as_slice(), crlf.row_rtl.as_slice()), ("a\nb", [0, 1, 3].as_slice(), [false, false].as_slice()));
        assert_eq!(crlf.to_screen, [Some(0), Some(1), None, Some(2)]);
        assert_eq!(wrapped_screen_map("a\r\n", false, None, |_| true).row_rtl.len(), 2);
        // Nothing at all.
        let empty = wrapped_screen_map("", false, None, |_| true);
        assert_eq!((empty.shaper.as_str(), empty.screen_len(), empty.row_rtl.len()), ("", 0, 0));
        assert_eq!((empty.screen_positions(0), empty.logical_positions(0)), (vec![0], vec![0]));
    }

    /// Texts with every kind of break, bidirectional control and run, as the field is typed into.
    fn wrapped_corpus() -> Vec<String> {
        let mut out = corpus();
        out.extend(
            [
                "a\r\nb\rc\u{2029}d\u{85}e",
                "\u{202B}\n\u{202C}",
                "  \u{627}  ",
                "\u{200F}\u{200E}\n\n",
                "\u{64B}\n\u{64B}",
                "ab\u{627}\u{628} cd\u{62A}\n\u{644}\u{627} x",
                "\n",
                "\r",
                "\u{1C}\u{1D}",
            ]
            .map(str::to_owned),
        );
        out
    }

    #[test]
    fn a_wrapped_map_draws_every_character_once_and_round_trips_the_caret() {
        for text in wrapped_corpus() {
            for width in [1, 2, 3, 5, 8, 1000] {
                let map = wrapped_screen_map(&text, width % 2 == 0, None, at_most(width));
                let (m, n) = (map.logical_len(), map.screen_len());
                assert_eq!(m, text.chars().count());
                assert_eq!((map.shaper.chars().count(), map.drawn.len()), (n, n), "{text:?} at {width}");
                assert_eq!(map.shaper.matches('\n').count(), map.drawn.iter().filter(|c| **c == '\n').count(), "{text:?}");
                // `shown` and `to_screen` are inverse to each other where there is a character.
                for (screen, &logical) in map.shown.iter().enumerate() {
                    if logical == VIRTUAL {
                        assert_eq!(map.drawn[screen], '\n', "{text:?}");
                    } else {
                        assert_eq!(map.to_screen.get(logical), Some(&Some(screen)), "{text:?} at {width}");
                    }
                }
                let real = map.shown.iter().filter(|l| **l != VIRTUAL).count();
                assert_eq!(map.to_screen.iter().flatten().count(), real, "{text:?}");
                // What is not drawn is a direction control or the `\n` of a `\r\n`.
                for (logical, c) in text.chars().enumerate() {
                    let drawn = map.to_screen.get(logical).copied().flatten().is_some();
                    let after_cr = c == '\n' && text.chars().nth(logical.wrapping_sub(1)) == Some('\r');
                    assert!(drawn || is_bidi_control(c) || after_cr, "{text:?} at {width} drops {c:?}");
                }
                // One row of the galley for each line: the breaks plus one, as many as the directions say.
                let rows = if text.is_empty() { 0 } else { map.shaper.matches('\n').count() + 1 };
                assert_eq!(map.row_rtl.len(), rows, "{text:?} at {width}: {:?}", map.row_rtl);
                for v in 0..=n {
                    let logical = map.logical_positions(v);
                    assert!(!logical.is_empty() && logical.len() <= 2, "{text:?} {v}");
                    for l in logical {
                        assert!(l <= m, "{text:?} {v} -> {l}");
                        assert!(
                            map.screen_positions(l).contains(&v),
                            "{text:?} at {width}: screen {v} -> logical {l} -> {:?}",
                            map.screen_positions(l)
                        );
                    }
                }
                for l in 0..=m {
                    let screen = map.screen_positions(l);
                    assert!(!screen.is_empty() && screen.len() <= 2 && screen.iter().all(|&v| v <= n), "{text:?} {l} -> {screen:?}");
                }
                assert_eq!(map.logical_of_screen(0..usize::MAX).len(), real);
            }
        }
    }

    /// The shaper text of `map` rebuilt from the stored `text` through [`ScreenMap::shaper_source`],
    /// asserting along the way that the mapping is sound.
    fn assert_shaper_source(text: &str, map: &ScreenMap, what: &str) {
        let chars: Vec<char> = text.chars().collect();
        let source = map.shaper_source();
        let shaper: Vec<char> = map.shaper.chars().collect();
        assert_eq!(source.len(), shaper.len(), "{what}: one source for each shaper character");
        let mut seen = std::collections::BTreeSet::new();
        for (i, (&c, &logical)) in shaper.iter().zip(&source).enumerate() {
            if logical == VIRTUAL {
                assert_eq!(c, '\n', "{what}: only a line break is virtual (shaper {i})");
                continue;
            }
            let own = chars.get(logical).copied();
            assert!(own.is_some(), "{what}: shaper {i} -> {logical} is no character of the text");
            let own = own.unwrap_or_default();
            // Equal, or a bracket mirrored in a right-to-left run, or a separator the field draws as a line break.
            assert!(
                c == own || c == mirrored(own) || (c == '\n' && is_paragraph_separator(own)),
                "{what}: shaper {i} is {c:?} but the text has {own:?} at {logical}"
            );
            assert!(seen.insert(logical), "{what}: {logical} twice");
        }
        // Every drawn character of the text is there once, in the stretch it was drawn in.
        let drawn: std::collections::BTreeSet<usize> = map.shown.iter().copied().filter(|l| *l != VIRTUAL).collect();
        assert_eq!(seen, drawn, "{what}");
        // A wrapped map's `\n` that is no character of the text is exactly the virtual entries.
        let virtual_breaks = source.iter().filter(|l| **l == VIRTUAL).count();
        assert_eq!(virtual_breaks, map.shown.iter().filter(|l| **l == VIRTUAL).count(), "{what}");
    }

    #[test]
    fn the_shaper_text_is_the_stored_text_through_shaper_source() {
        for text in wrapped_corpus() {
            assert_shaper_source(&text, &screen_map(&text), &format!("{text:?} unwrapped"));
            for width in [1, 2, 3, 5, 8, 1000] {
                for forced in [None, Some(true), Some(false)] {
                    let map = wrapped_screen_map(&text, width % 2 == 0, forced, at_most(width));
                    assert_shaper_source(&text, &map, &format!("{text:?} at {width} forced {forced:?}"));
                }
            }
        }
    }

    #[test]
    fn the_shaper_gets_each_arabic_word_in_logical_order_and_the_words_in_display_order() {
        // "abj dhw": alef ba jim, a space, dal heh waw, in one row.
        let text = "\u{627}\u{628}\u{62C} \u{62F}\u{647}\u{648}";
        let one_row = wrapped_screen_map(text, false, None, |_| true);
        // Displayed from the left: the second word, the space, the first word.
        assert_eq!(one_row.shaper_source(), [4, 5, 6, 3, 0, 1, 2]);
        // Wrapped, the break between the rows is virtual and each row is a stretch of its own.
        let wrapped = wrapped_screen_map(text, false, None, at_most(3));
        assert_eq!(wrapped.shaper_source(), [3, 0, 1, 2, VIRTUAL, 4, 5, 6]);
        // Latin around Arabic stays where it is drawn.
        let mixed = wrapped_screen_map("ab \u{627}\u{628}", false, None, |_| true);
        assert_eq!(mixed.shaper_source(), [0, 1, 2, 3, 4]);
        // A bracket is mirrored where it is drawn in a right-to-left run, and stays out of the stretch.
        let bracket = wrapped_screen_map("\u{627}(\u{628})", false, None, |_| true);
        assert_eq!(bracket.shaper, "(\u{628})\u{627}");
        assert_eq!(bracket.shaper_source(), [3, 2, 1, 0]);
        // Without right-to-left script it is the identity.
        let latin = screen_map("hello world");
        assert_eq!(latin.shaper_source(), (0..11).collect::<Vec<_>>());
        assert!(screen_map("").shaper_source().is_empty());
    }

    #[test]
    fn a_forced_direction_equals_the_automatic_one_when_the_first_strong_character_agrees() {
        let mut compared = 0;
        for text in wrapped_corpus() {
            // One paragraph: the corpus's other texts hold several, which may disagree.
            if text.chars().any(is_paragraph_separator) {
                continue;
            }
            let Some(rtl) = first_strong_rtl(&text) else { continue };
            for width in [2, 5, 1000] {
                let auto = wrapped_screen_map(&text, !rtl, None, at_most(width));
                let forced = wrapped_screen_map(&text, !rtl, Some(rtl), at_most(width));
                assert_eq!(forced, auto, "{text:?} at {width}, forced {rtl}");
                compared += 1;
            }
        }
        assert!(compared > 30, "the corpus must have texts to compare: {compared}");
    }

    #[test]
    fn the_blocks_direction_rule_is_the_engines_and_counts_letters() {
        // The engine writes a block back by the majority of its letters, not by its first strong one:
        // a caller that forces the direction with it can disagree with what `wrapped_screen_map` finds alone.
        let text = "ab \u{5E9}\u{5DC}\u{5D5}\u{5DD} \u{5E2}\u{5D5}\u{5DC}\u{5DD}";
        assert!(pdfcraft_engine::rtl_base(text));
        assert_eq!(first_strong_rtl(text), Some(false));
        let auto = wrapped_screen_map(text, false, None, |_| true);
        let forced = wrapped_screen_map(text, false, Some(pdfcraft_engine::rtl_base(text)), |_| true);
        assert_eq!((auto.row_rtl.as_slice(), forced.row_rtl.as_slice()), ([false].as_slice(), [true].as_slice()));
        assert!(!pdfcraft_engine::rtl_base("abc"));
        assert!(!pdfcraft_engine::rtl_base(""));
    }

    #[test]
    fn a_forced_direction_overrides_the_paragraphs_own() {
        let arabic = "\u{627}\u{628} \u{62C}";
        let latin = "ab cd";
        let ltr = wrapped_screen_map(arabic, true, Some(false), |_| true);
        assert_eq!(ltr.row_rtl, [false], "Arabic text in a block that reads left to right is a left-to-right row");
        let rtl = wrapped_screen_map(arabic, false, Some(true), |_| true);
        assert_eq!(rtl.row_rtl, [true]);
        // Latin then Arabic: the first strong letter says left to right; forced, the Arabic word comes first from the left.
        let mixed = "ab \u{627}\u{628}";
        let auto = wrapped_screen_map(mixed, false, None, |_| true);
        assert_eq!((auto.row_rtl.as_slice(), auto.shown.as_slice()), ([false].as_slice(), [0, 1, 2, 4, 3].as_slice()));
        let forced = wrapped_screen_map(mixed, false, Some(true), |_| true);
        assert_eq!((forced.row_rtl.as_slice(), forced.shown.as_slice()), ([true].as_slice(), [4, 3, 2, 0, 1].as_slice()));
        // A paragraph of Latin in a block that reads right to left is a right-to-left row; its words keep their order.
        let latin_in_rtl = wrapped_screen_map(latin, false, Some(true), |_| true);
        assert_eq!(latin_in_rtl.row_rtl, [true]);
        assert_eq!(latin_in_rtl.shaper, latin);
        // Every paragraph, and a line of digits only, takes it.
        let paragraphs = wrapped_screen_map("ab\n12\n\u{627}", false, Some(true), |_| true);
        assert_eq!(paragraphs.row_rtl, [true, true, true]);
        let paragraphs = wrapped_screen_map("ab\n12\n\u{627}", true, Some(false), |_| true);
        assert_eq!(paragraphs.row_rtl, [false, false, false]);
    }

    #[test]
    fn embeddings_nested_to_the_limit_of_the_algorithm_do_not_panic() {
        // Sixty-three unclosed right-to-left embeddings put the letters after them at the deepest
        // level the algorithm has; a line made of nothing else then has no level to reorder from.
        let deep = format!("{}ab cd ef", "\u{202B}".repeat(70));
        let deeper = format!("{}{}ab cd ef", "\u{202B}\u{202A}".repeat(70), "\u{627}");
        for text in [deep, deeper] {
            let _ = visual(&text);
            let _ = screen_map(&text);
            for width in [1, 2, 3, 100] {
                let map = wrapped_screen_map(&text, false, None, at_most(width));
                assert_eq!(map.logical_len(), text.chars().count());
                assert_eq!(map.logical_of_screen(0..usize::MAX).len(), text.chars().filter(|c| !is_bidi_control(*c)).count());
            }
            let _ = wrap(&text, at_most(2));
        }
    }

    #[test]
    fn wrapping_survives_hostile_measures_and_long_input() {
        // A measure that accepts nothing, then only the first call, then everything.
        for text in wrapped_corpus() {
            let none = wrapped_screen_map(&text, false, None, |_| false);
            assert_eq!(none.logical_len(), text.chars().count());
            let mut first = true;
            let once = wrapped_screen_map(&text, true, None, |_| std::mem::take(&mut first));
            assert_eq!(once.logical_len(), text.chars().count());
        }
        let long = "\u{627}\u{628} ".repeat(5_000);
        let map = wrapped_screen_map(&long, false, None, at_most(40));
        assert!(map.screen_len() >= long.chars().count());
        assert_eq!(map.logical_of_screen(0..usize::MAX).len(), long.chars().count());
        let word = "\u{633}".repeat(3_000);
        assert_eq!(wrapped_screen_map(&word, false, None, at_most(7)).shaper.matches('\n').count(), 3_000 / 7);
    }

    /// With an Arabic face the glyphs of the galley egui builds from the shaper text come in
    /// screen order, left to right: the glyphs that have a width are the characters of `drawn`,
    /// and x never decreases. (egui adds more zero-width glyphs after every right-to-left run, so
    /// their number is not `N`; the text field aligns them, see `widgets::bidi_edit`.)
    #[test]
    fn egui_galleys_are_in_screen_order() {
        let Some(fonts) = arabic_test_fonts() else { return };
        let ctx = egui::Context::default();
        ctx.set_fonts(fonts);
        // Without lam-alef ligatures, which draw two characters as one glyph.
        let texts = [
            "ab\u{643}\u{62A}\u{627}\u{628}",
            "\u{648}\u{627}\u{62D}\u{62F} \u{627}\u{62B}\u{646}\u{64A}\u{646}",
            "x \u{633}\u{645}\u{627}\u{645} 12 y",
            "\u{633}\u{627}\u{644} \u{6F1}\u{6F4}\u{6F0}\u{6F2} \u{634}\u{645}\u{633}\u{6CC}",
        ];
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            for text in texts {
                let map = screen_map(text);
                let galley = ui.painter().layout_no_wrap(map.shaper.clone(), egui::FontId::proportional(14.0), egui::Color32::WHITE);
                assert_eq!(galley.rows.len(), 1, "{text:?}");
                let glyphs = &galley.rows[0].glyphs;
                assert!(glyphs.len() >= map.screen_len(), "{text:?}");
                let wide: Vec<&_> = glyphs.iter().filter(|g| g.advance_width > 0.0).collect();
                let chars: Vec<char> = wide.iter().map(|g| g.chr).collect();
                assert_eq!(chars, map.drawn, "{text:?}: glyphs with a width, left to right");
                for pair in wide.windows(2) {
                    assert!(pair[1].pos.x >= pair[0].pos.x - 0.01, "{text:?}: x runs backwards: {} then {}", pair[0].pos.x, pair[1].pos.x);
                }
            }
        });
        output.drop_without_applying_deltas();
    }
}
