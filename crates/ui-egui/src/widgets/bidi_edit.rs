//! A text field that shows, selects and edits right-to-left script in display order.
//!
//! egui lays the runs of a line out left to right in logical order, so the words of a typed
//! Arabic line come out reversed; [`crate::bidi::visual`] puts the words in display order, and
//! that is what this field hands egui to draw. egui's caret, click placement and selection then
//! all work in the order the glyphs are drawn: *screen order*, the i-th character from the left.
//! The stored text stays in logical order (what is typed, saved and read aloud), so the field
//! keeps `egui::TextEdit` for events, IME, undo and clipboard and gives it a [`TextBuffer`]
//! ([`BidiBuffer`]) whose indices are screen indices and whose `as_str` is the logical text.
//! Where egui edits the buffer (insert, delete, undo) the adapter translates the screen index
//! back to a logical one through a [`ScreenMap`]; a caret can sit at two places on screen where a
//! run of one direction meets a run of the other, so the field also remembers the *logical*
//! caret ([`Caret`]) next to the screen one egui keeps.
//!
//! A multi-line field does the same for paragraphs that wrap. egui would break a paragraph at
//! the word boundaries of the logical text, which for Arabic is the wrong end of every line, so
//! the field wraps it itself ([`bidi::wrapped_screen_map`], at the width egui hands the layouter)
//! and gives egui the lines already in display order, one galley row each. The break between two
//! lines is a newline of the screen text that is no character of the stored text: the map holds
//! it as [`bidi::VIRTUAL`], so a caret, a selection, Backspace or Copy never sees it. The galley's
//! rows are laid out from the left and the rows of right-to-left paragraphs then moved against
//! the right edge ([`place_rows`]), so one field can hold both directions. The width is needed
//! before egui reports it (to put the cursor it kept on the screen), so it is estimated from the
//! field's settings ([`estimate_wrap_width`]); when the field changes width the kept cursor is
//! put back at the same logical place.
//!
//! What this means for the user:
//! - Left and Right move the caret on screen, a character at a time. In a single line, Home and
//!   End go to the start and end of the text when it begins in a right-to-left script (the right
//!   and left edge), to the screen edges otherwise. In a multi-line field they go to the ends of
//!   the row, and Ctrl+Home and Ctrl+End to the start and end of the text in reading order; Up
//!   and Down move between the rows as drawn.
//! - Backspace and Delete work in logical order: Backspace removes the letter typed last.
//! - A selection is the characters between two screen positions, so it is one highlighted
//!   stretch; across a change of direction those characters are not contiguous in the text.
//!   Copy gives them in logical order.
//!
//! Text without right-to-left script, passwords and texts of more than [`MAX_WRAPPED_CHARS`]
//! characters in a multi-line field are plain `egui::TextEdit`s: [`BidiEdit`] is a drop-in for it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use egui::text::{CCursor, CCursorRange, CharIndex, Galley, LayoutJob};
use egui::text_edit::{TextEditOutput, TextEditState};
use egui::text_selection::text_cursor_state::{ccursor_next_word, ccursor_previous_word};
use egui::{
    Align, Atoms, Color32, Event, FontId, FontSelection, Frame, Id, IdSalt, IntoAtoms, Key, Margin, Modifiers, Pos2, Response, TextBuffer, TextEdit,
    Ui, Vec2, Widget,
};

use crate::bidi::{self, ScreenMap};

// ---------------------------------------------------------------------------------------------
// The screen map, shared between the buffer and the layouter.

/// How a multi-line field wraps its text (see [`bidi::wrapped_screen_map`]).
struct Wrap {
    ctx: egui::Context,
    font: FontId,
    /// The width a line may take; what the layouter was last given.
    width: f32,
    /// The direction of a paragraph without a strong character (the interface's).
    neutral_rtl: bool,
}

/// The width of a tab's indentation in spaces, as egui takes it.
const TAB_SIZE: usize = 4;

/// The most characters of lines `Wrap` measures for one map. Wrapping measures a line for every
/// word added to it, so the work grows with the length of a paragraph times that of a line; past
/// this the rest of the text is left unwrapped, rather than a pasted megabyte stalling the frame.
const WRAP_BUDGET: usize = 400_000;

/// Fields with more characters than this are plain `egui::TextEdit`s, in logical order.
const MAX_WRAPPED_CHARS: usize = 50_000;

impl Wrap {
    /// The map of `text` wrapped at the current width.
    fn map(&self, text: &str) -> ScreenMap {
        let width = self.width.round().max(1.0);
        let mut spent = 0_usize;
        bidi::wrapped_screen_map(text, self.neutral_rtl, |line: &str| {
            spent = spent.saturating_add(line.len());
            if !self.width.is_finite() || spent > WRAP_BUDGET {
                return true;
            }
            // The widest extent of its glyphs, as egui measures a row against the wrap width.
            let wide = self.ctx.fonts_mut(|f| {
                let galley = f.layout_no_wrap(line.to_owned(), self.font.clone(), Color32::WHITE);
                galley.rows.first().map_or(0.0, |row| row.glyphs.iter().map(Glyph::max_x).fold(0.0, f32::max))
            });
            wide <= width
        })
    }
}

/// The [`ScreenMap`] of the last text seen, so the buffer, the layouter and the reconciliation
/// compute it once per edit.
#[derive(Default)]
struct MapCache {
    text: String,
    /// The width the map was wrapped at (its bits), for a multi-line field.
    width: Option<u32>,
    map: Option<Arc<ScreenMap>>,
    /// Set for a multi-line field.
    wrap: Option<Wrap>,
}

type SharedCache = Rc<RefCell<MapCache>>;

/// The screen map of `text`, from `cache` when it is the text (and width) the cache saw last.
fn map_of(cache: &SharedCache, text: &str) -> Arc<ScreenMap> {
    // Nothing holds the cache across a call, but a re-entrant borrow must not panic.
    let Ok(mut cache) = cache.try_borrow_mut() else { return Arc::new(bidi::screen_map(text)) };
    let width = cache.wrap.as_ref().map(|w| w.width.to_bits());
    if let Some(map) = &cache.map
        && cache.text == text
        && cache.width == width
    {
        return Arc::clone(map);
    }
    let map = Arc::new(match &cache.wrap {
        Some(wrap) => wrap.map(text),
        None => bidi::screen_map(text),
    });
    text.clone_into(&mut cache.text);
    cache.width = width;
    cache.map = Some(Arc::clone(&map));
    map
}

/// The map of a multi-line field's text as the last frame left it, so a frame in which the text
/// and the width are the same does not wrap the text again.
#[derive(Clone, Default)]
struct KeptMap {
    text: String,
    width: u32,
    neutral_rtl: bool,
    map: Arc<ScreenMap>,
}

/// Puts `kept` in `cache` if it is the map of `text` at the width and direction the cache wraps with.
fn seed_cache(cache: &SharedCache, kept: KeptMap, text: &str) {
    let Ok(mut cache) = cache.try_borrow_mut() else { return };
    let Some(wrap) = &cache.wrap else { return };
    if kept.width == wrap.width.to_bits() && kept.neutral_rtl == wrap.neutral_rtl && kept.text == text {
        cache.width = Some(kept.width);
        cache.text = kept.text;
        cache.map = Some(kept.map);
    }
}

/// What `cache` holds, to keep for the next frame.
fn kept_map(cache: &SharedCache) -> Option<KeptMap> {
    let cache = cache.try_borrow().ok()?;
    let (wrap, map) = (cache.wrap.as_ref()?, cache.map.as_ref()?);
    Some(KeptMap { text: cache.text.clone(), width: wrap.width.to_bits(), neutral_rtl: wrap.neutral_rtl, map: Arc::clone(map) })
}

/// Tells the cache the width the layouter was given.
fn set_wrap_width(cache: &SharedCache, width: f32) {
    if let Ok(mut cache) = cache.try_borrow_mut()
        && let Some(wrap) = cache.wrap.as_mut()
    {
        wrap.width = width;
    }
}

/// Where the caret is, in both orders: `l` between logical characters `l - 1` and `l`, drawn at
/// screen position `v`. egui keeps only `v`; where two logical places are drawn at one position
/// (or one logical place at two) this says which one the user is at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Caret {
    l: usize,
    v: usize,
}

type SharedCaret = Rc<Cell<Option<Caret>>>;

/// An input method's composition (the text it has shown but not committed) as the field holds it:
/// egui replaces it on every event, naming it by the screen range it was told, which for
/// right-to-left text can be empty or hold other characters than the composition; this is where
/// it really is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Preedit {
    /// The screen range egui was given for it, ordered.
    screen: (usize, usize),
    /// The logical characters it occupies.
    logical: (usize, usize),
    /// What it says.
    text: String,
}

type SharedPreedit = Rc<Cell<Option<Preedit>>>;

/// The first of `positions` (never empty for a [`ScreenMap`] answer), 0 if it is.
fn first(positions: &[usize]) -> usize {
    positions.first().copied().unwrap_or(0)
}

/// The caret's logical place for screen position `v`, given the caret remembered `known` (valid
/// only if it is at `v` and the text still draws that place there) or, failing that, the first
/// place `v` can mean.
fn logical_for(map: &ScreenMap, v: usize, known: Option<Caret>) -> usize {
    let v = v.min(map.screen_len());
    match known {
        Some(c) if c.v == v && c.l <= map.logical_len() && map.screen_positions(c.l).contains(&v) => c.l,
        _ => first(&map.logical_positions(v)),
    }
}

/// How the screen position changed without the field editing text, which says which of the two
/// logical places the caret ended up at.
#[derive(Clone, Copy, Debug)]
enum Moved {
    /// Left, Home, Up: the caret passed the character it now stands left of.
    Left,
    /// Right, End, Down, select all: the caret passed the character it now stands right of.
    Right,
    /// A click or drag at `pointer_x`, the screen position being at `boundary_x`.
    Pointer { pointer_x: f32, boundary_x: f32 },
    /// Set from outside (accessibility, another widget): the first place.
    External,
}

/// The logical place for screen position `v` reached by `how`.
fn derive_logical(map: &ScreenMap, v: usize, how: Moved) -> usize {
    let places = map.logical_positions(v);
    let (a, b) = (places.first().copied().unwrap_or(0), places.last().copied().unwrap_or(0));
    match how {
        Moved::Left => b,
        Moved::Right | Moved::External => a,
        // Left of the boundary the pointer is over the character on its left, whose right edge is `a`.
        Moved::Pointer { pointer_x, boundary_x } => {
            if pointer_x < boundary_x {
                a
            } else {
                b
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The galley: one glyph per character, in screen order.

type Glyph = egui::epaint::text::Glyph;

/// The most characters one cluster of glyphs is looked for in: a lam-alef or Allah ligature, a
/// letter with a stack of marks. It keeps the search linear however a text is shaped.
const MAX_CLUSTER: usize = 16;

/// `galley` with exactly one glyph per drawn character, in screen order.
///
/// egui promises `glyphs.len() == chars` (zero-width continuation glyphs make up for a cluster of
/// characters that shapes to fewer glyphs), but its accounting goes wrong for a right-to-left run:
/// the shaper hands the glyphs left to right with clusters counting down, so the continuation
/// glyphs egui adds after the run's last glyph cover the whole run. A word of `k` letters gets up
/// to `2k - 1` glyphs with the extra ones piled up at its end, and the number of glyphs a letter
/// or ligature takes depends on the face: a lam-alef is one glyph in one face and two in another,
/// a letter with dots or a vowel mark is one glyph or a letter plus a zero-width mark.
///
/// This does not assume any of that. It takes the glyphs that are drawn or take room, in their
/// order, groups them into *cells* (one cluster of the shaper: a glyph with the marks that go
/// with it), finds the characters of each cell by the first character of the cluster that
/// every glyph names, and gives every character a glyph (see [`cell_entries`]).
fn align_glyphs(mut galley: Arc<Galley>, map: &ScreenMap, lines: bool) -> Arc<Galley> {
    // Only right-to-left runs break the rule (egui shapes the script's characters that way
    // whatever level the bidirectional algorithm gave them).
    if !map.drawn.iter().any(|c| bidi::is_shaped_rtl(*c)) {
        return galley;
    }
    // Wrapped lines: egui makes a row of the characters between two newlines (the text's own and
    // the wrapping's). A single line is one row, whatever the text holds (egui cuts it short).
    let drawn: Vec<&[char]> = if lines { map.drawn.split(|c| *c == '\n').collect() } else { vec![map.drawn.as_slice()] };
    if galley.rows.len() != drawn.len() {
        return galley;
    }
    let aligned: Vec<Option<Vec<Glyph>>> = galley
        .rows
        .iter()
        .zip(&drawn)
        .map(|(placed, chars)| {
            let glyphs = aligned_glyphs(&placed.row.glyphs, chars, placed.row.visuals.glyph_vertex_range.end);
            (placed.row.glyphs != glyphs).then_some(glyphs)
        })
        .collect();
    if aligned.iter().all(Option::is_none) {
        return galley;
    }
    for (placed, glyphs) in Arc::make_mut(&mut galley).rows.iter_mut().zip(aligned) {
        if let Some(glyphs) = glyphs {
            Arc::make_mut(&mut placed.row).glyphs = glyphs;
        }
    }
    galley
}

/// The glyphs of a row that draws the characters `drawn` (`vertex_end`: where the row's glyph vertices
/// end), as one glyph per drawn character: glyph `i` is character `i` from the left, `x` never goes
/// back and `first_vertex` never decreases. A text that does not match what the glyphs say gets
/// zero-width places for the characters that found no glyph rather than being refused.
fn aligned_glyphs(glyphs: &[Glyph], drawn: &[char], vertex_end: usize) -> Vec<Glyph> {
    let n = drawn.len();
    let end = u32::try_from(vertex_end).unwrap_or(u32::MAX);
    // Zero-width glyphs without a picture are continuations (or the stray ones after a
    // right-to-left run); they are made again below.
    let reals: Vec<(usize, Glyph)> = glyphs.iter().copied().enumerate().filter(|(_, g)| g.advance_width > 0.0 || !g.uv_rect.is_nothing()).collect();
    // Characters without a glyph of their own (a joiner, a zero-width space) get a place of no
    // width in front of the next glyph, which is where the caret passes them; unless a glyph says
    // it is theirs.
    let claimed: Vec<char> = reals.iter().map(|(_, g)| g.chr).filter(|c| is_invisible(*c)).collect();
    let skipped = |c: char| is_invisible(c) && !claimed.contains(&c);
    let rtl_at = |i: usize| drawn.get(i).is_some_and(|c| bidi::is_shaped_rtl(*c));

    let mut slots: Vec<Option<Glyph>> = vec![None; n];
    let mut j = 0_usize;
    let mut a = 0_usize;
    while a < n {
        // A stretch of one direction: the glyphs of a right-to-left run come in screen order,
        // each cluster's marks in front of its letter; those of a left-to-right run, marks after.
        let rtl = rtl_at(a);
        let b = (a..n).find(|i| rtl_at(*i) != rtl).unwrap_or(n);
        let shown: Vec<usize> = (a..b).filter(|i| drawn.get(*i).is_some_and(|c| !skipped(*c))).collect();
        let char_at = |p: usize| shown.get(p).and_then(|at| drawn.get(*at)).copied();
        let mut p = 0_usize;
        while p < shown.len() {
            let Some(&(_, head)) = reals.get(j) else { break };
            // The glyphs of this cluster.
            let mut stop = j.saturating_add(1);
            while let Some(&(_, next)) = reals.get(stop) {
                let marked =
                    if rtl { reals.get(stop.saturating_sub(1)).is_some_and(|(_, g)| g.advance_width == 0.0) } else { next.advance_width == 0.0 };
                if !(marked && next.chr == head.chr) {
                    break;
                }
                stop = stop.saturating_add(1);
            }
            // The characters of this cluster: a glyph names the first one in reading order, which
            // is the last one drawn in a right-to-left run and the first in a left-to-right one.
            let room = shown.len().saturating_sub(p);
            let window = room.min(MAX_CLUSTER);
            let span = if rtl {
                (0..window).find(|m| char_at(p.saturating_add(*m)) == Some(head.chr)).map_or(1, |m| m.saturating_add(1))
            } else {
                match reals.get(stop) {
                    // Up to where the next cluster starts.
                    Some(&(_, next)) => {
                        (1..window).find(|m| char_at(p.saturating_add(*m)) == Some(next.chr)).unwrap_or(if room <= MAX_CLUSTER { room } else { 1 })
                    }
                    None => window,
                }
            }
            .clamp(1, room);
            let cell: Vec<Glyph> = reals.get(j..stop).unwrap_or_default().iter().map(|(_, g)| *g).collect();
            let last_at = reals.get(stop.saturating_sub(1)).map_or(0, |(at, _)| *at);
            let after = glyphs.get(last_at.saturating_add(1)).map_or(end, |g| g.first_vertex);
            let screen: Vec<usize> = (p..p.saturating_add(span)).filter_map(|q| shown.get(q).copied()).collect();
            let chars: Vec<char> = screen.iter().map(|at| drawn.get(*at).copied().unwrap_or(' ')).collect();
            for (at, glyph) in screen.iter().zip(cell_entries(&cell, &chars, after)) {
                if let Some(slot) = slots.get_mut(*at) {
                    *slot = Some(glyph);
                }
            }
            p = p.saturating_add(span);
            j = stop;
        }
        a = b;
    }

    // What found no glyph: in front of the next glyph, or after the last one at the end.
    let mut next: Option<Glyph> = None;
    for (k, slot) in slots.iter_mut().enumerate().rev() {
        match (*slot, next) {
            (Some(glyph), _) => next = Some(glyph),
            (None, Some(after)) => *slot = Some(zero_width(&after, drawn.get(k).copied().unwrap_or(' '))),
            (None, None) => {}
        }
    }
    let mut last: Option<Glyph> = None;
    for (k, slot) in slots.iter_mut().enumerate() {
        match (*slot, last) {
            (Some(glyph), _) => last = Some(glyph),
            (None, template) => {
                let Some(template) = template.or_else(|| glyphs.last().copied()) else { continue };
                let glyph = continuation(&template, drawn.get(k).copied().unwrap_or(' '), template.pos.x + template.advance_width, end);
                *slot = Some(glyph);
                last = Some(glyph);
            }
        }
    }
    let mut out: Vec<Glyph> = slots.into_iter().flatten().collect();
    // Pixel snapping and mark offsets can leave a glyph a fraction right of the next one's start:
    // keep x from going back, for the caret and the hit test.
    for k in (0..out.len().saturating_sub(1)).rev() {
        let next_x = out.get(k.saturating_add(1)).map(|g| g.pos.x);
        if let (Some(glyph), Some(next_x)) = (out.get_mut(k), next_x) {
            glyph.pos.x = glyph.pos.x.min(next_x);
        }
    }
    out
}

/// One glyph for each of `chars`, the characters (in screen order) of one cluster of `cell`, the
/// glyphs the shaper made of it (in screen order); `after` is where the next glyph's vertices begin.
///
/// The glyphs go to the characters in order: a letter and its marks (the marks are zero-width
/// glyphs, drawn on the letter), or a ligature's glyph for its first character. A character left
/// over (the ligature's other letters) gets a zero-width glyph at the cluster's right edge; a
/// glyph left over (a letter the face draws in two pieces) is merged into the cluster's letter,
/// whose vertices then begin at the first piece.
fn cell_entries(cell: &[Glyph], chars: &[char], after: u32) -> Vec<Glyph> {
    let Some(&head) = cell.first() else { return Vec::new() };
    let mut keep = cell.to_vec();
    while keep.len() > chars.len() {
        match keep.iter().position(|g| g.advance_width == 0.0) {
            Some(at) => {
                keep.remove(at);
            }
            None => {
                keep.pop();
            }
        }
    }
    // The cluster's edges are those of the glyphs that take room; a mark's own position is
    // offset from them.
    let left = cell.iter().find(|g| g.advance_width > 0.0).unwrap_or(&head).pos.x;
    let right = cell.iter().filter(|g| g.advance_width > 0.0).map(Glyph::max_x).fold(left, f32::max);
    let first_wide = keep.iter().position(|g| g.advance_width > 0.0);
    let mut out: Vec<Glyph> = Vec::with_capacity(chars.len());
    for (k, chr) in chars.iter().enumerate() {
        let glyph = match keep.get(k) {
            Some(&kept) => {
                let mut glyph = kept;
                glyph.chr = *chr;
                if glyph.advance_width == 0.0 {
                    // A mark in front of its letter stands at the letter's left edge, one behind at its right.
                    glyph.pos.x = if first_wide.is_some_and(|wide| k > wide) { right } else { left };
                }
                if k == 0 {
                    glyph.first_vertex = head.first_vertex;
                }
                glyph
            }
            None => continuation(&out.last().copied().unwrap_or(head), *chr, right, after),
        };
        out.push(glyph);
    }
    out
}

/// Characters that take no room and have no picture, so a font gives them no glyph that is drawn.
fn is_invisible(c: char) -> bool {
    matches!(c, '\u{00AD}' | '\u{034F}' | '\u{200B}'..='\u{200D}' | '\u{2060}'..='\u{2064}' | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}')
}

/// A zero-width glyph for `chr` at the left edge of `next`.
fn zero_width(next: &Glyph, chr: char) -> Glyph {
    let mut glyph = *next;
    glyph.chr = chr;
    glyph.advance_width = 0.0;
    glyph.uv_rect = Default::default();
    glyph
}

/// A zero-width glyph for `chr` at `x`, with the look of `base` and its mesh vertices ending at `first_vertex`.
fn continuation(base: &Glyph, chr: char, x: f32, first_vertex: u32) -> Glyph {
    let mut glyph = *base;
    glyph.chr = chr;
    glyph.pos.x = x;
    glyph.advance_width = 0.0;
    glyph.uv_rect = Default::default();
    glyph.first_vertex = first_vertex;
    glyph
}

// ---------------------------------------------------------------------------------------------
// The buffer.

/// The text of a field as `egui::TextEdit` sees it: indices are screen positions, `as_str` is
/// the logical text.
struct BidiBuffer<'a> {
    text: &'a mut String,
    cache: SharedCache,
    caret: SharedCaret,
    /// The input method's composition in the field, if one is under way.
    preedit: SharedPreedit,
    /// The composition texts the input method reports this frame.
    composing: Vec<String>,
    /// Whether the input method reports anything this frame.
    any_ime: bool,
    /// Whether the field is multi-line: its paragraphs end at the text's own line breaks.
    multiline: bool,
}

/// The byte offset of character `index` of `text` (the end if past it).
fn byte_offset(text: &str, index: usize) -> usize {
    text.char_indices().nth(index).map_or(text.len(), |(at, _)| at)
}

impl BidiBuffer<'_> {
    fn map(&self) -> Arc<ScreenMap> {
        map_of(&self.cache, self.text)
    }

    /// Removes the logical characters `from..to`.
    fn remove_range(&mut self, from: usize, to: usize) {
        if from >= to {
            return;
        }
        let (start, end) = (byte_offset(self.text, from), byte_offset(self.text, to));
        // Both are character starts of the text, so the range is on boundaries.
        if start <= end {
            self.text.replace_range(start..end, "");
        }
    }

    /// Removes the logical characters in `set`, sorted.
    fn remove_set(&mut self, set: &[usize]) {
        if set.is_empty() {
            return;
        }
        let kept: String = self.text.chars().enumerate().filter(|(i, _)| set.binary_search(i).is_err()).map(|(_, c)| c).collect();
        *self.text = kept;
    }

    /// The caret at logical place `l` of the (edited) text, at the screen position of its places
    /// nearest to `near`; returns that position as egui's cursor.
    fn settle(&self, l: usize, near: usize) -> CCursor {
        let map = self.map();
        let l = l.min(map.logical_len());
        let places = map.screen_positions(l);
        let v = places.iter().copied().min_by_key(|p| p.abs_diff(near)).unwrap_or(0).min(map.screen_len());
        self.caret.set(Some(Caret { l, v }));
        CCursor { index: CharIndex(v), prefer_next_row: true }
    }

    /// The caret at logical place `l`, at the position the next typed character appears.
    fn settle_after(&self, l: usize) -> CCursor {
        let map = self.map();
        let l = l.min(map.logical_len());
        let v = first(&map.screen_positions(l)).min(map.screen_len());
        self.caret.set(Some(Caret { l, v }));
        CCursor { index: CharIndex(v), prefer_next_row: true }
    }

    /// The start of the paragraph that logical place `l` is in: just after the last line break
    /// before it.
    fn paragraph_start(&self, l: usize) -> usize {
        let before: Vec<char> = self.text.chars().take(l).collect();
        before.iter().rposition(|c| *c == '\n').map_or(0, |at| at.saturating_add(1))
    }

    /// The end of the paragraph that logical place `l` is in: the next line break at or after it
    /// (the text's end if none).
    fn paragraph_end(&self, l: usize) -> usize {
        self.text.chars().enumerate().skip(l).find(|(_, c)| *c == '\n').map_or_else(|| self.text.chars().count(), |(at, _)| at)
    }

    /// The logical place of the caret at screen index `v`.
    fn logical(&self, map: &ScreenMap, v: usize) -> usize {
        logical_for(map, v, self.caret.get())
    }
}

impl TextBuffer for BidiBuffer<'_> {
    fn is_mutable(&self) -> bool {
        true
    }

    fn as_str(&self) -> &str {
        self.text.as_str()
    }

    fn insert_text(&mut self, text: &str, char_index: CharIndex) -> usize {
        let map = self.map();
        let l = self.logical(&map, char_index.0);
        self.text.insert_str(byte_offset(self.text, l), text);
        let inserted = text.chars().count();
        self.settle_after(l.saturating_add(inserted));
        inserted
    }

    fn delete_char_range(&mut self, char_range: std::ops::Range<CharIndex>) {
        let map = self.map();
        let set = map.logical_of_screen(char_range.start.0..char_range.end.0);
        let Some(&l0) = set.first() else { return };
        self.remove_set(&set);
        self.settle(l0, char_range.start.0);
    }

    fn clear(&mut self) {
        self.text.clear();
        self.caret.set(None);
        self.preedit.set(None);
    }

    fn replace_with(&mut self, text: &str) {
        text.clone_into(self.text);
        self.caret.set(None);
        self.preedit.set(None);
    }

    fn insert_text_at(&mut self, ccursor: &mut CCursor, text_to_insert: &str, char_limit: usize) {
        let room = char_limit.saturating_sub(self.text.chars().count());
        let limited: String = if char_limit < usize::MAX { text_to_insert.chars().take(room).collect() } else { text_to_insert.to_owned() };
        let map = self.map();
        let v = ccursor.index.0.min(map.screen_len());
        if limited.is_empty() {
            ccursor.index = CharIndex(v);
            return;
        }
        let l = self.logical(&map, v);
        self.text.insert_str(byte_offset(self.text, l), &limited);
        let end = l.saturating_add(limited.chars().count());
        let after = self.settle_after(end);
        if self.any_ime && self.composing.iter().any(|t| t == text_to_insert) {
            // egui will name this composition by the screen range from here to the new caret.
            let (a, b) = (v, after.index.0);
            self.preedit.set(Some(Preedit { screen: (a.min(b), a.max(b)), logical: (l, end), text: limited }));
        }
        *ccursor = CCursor { prefer_next_row: ccursor.prefer_next_row, ..after };
    }

    fn decrease_indentation(&mut self, ccursor: &mut CCursor) {
        if !self.multiline {
            // There is no indentation to take back.
            return;
        }
        let map = self.map();
        let v = ccursor.index.0.min(map.screen_len());
        let l = self.logical(&map, v);
        let start = self.paragraph_start(l);
        // A tab, or up to a tab's worth of spaces, at the start of the paragraph in reading order.
        let mut after = self.text.chars().skip(start);
        let len = match after.next() {
            Some('\t') => 1,
            Some(' ') => 1_usize.saturating_add(after.take(TAB_SIZE.saturating_sub(1)).take_while(|c| *c == ' ').count()),
            _ => 0,
        };
        if len == 0 {
            return;
        }
        self.remove_range(start, start.saturating_add(len));
        let at = if l > start { l.saturating_sub(len).max(start) } else { l };
        *ccursor = CCursor { prefer_next_row: ccursor.prefer_next_row, ..self.settle(at, v) };
    }

    fn delete_selected_ccursor_range(&mut self, [min, max]: [CCursor; 2]) -> CCursor {
        let map = self.map();
        let n = map.screen_len();
        let (a, b) = (min.index.0.min(n), max.index.0.min(n));
        if self.any_ime
            && let Some(span) = self.preedit.take()
        {
            // The composition egui replaces: delete it where it really is, if it still says what it did.
            let (from, to) = span.logical;
            let still_there = (a.min(b), a.max(b)) == span.screen && self.text.chars().skip(from).take(to.saturating_sub(from)).eq(span.text.chars());
            if still_there {
                self.remove_range(from, to);
                return self.settle(from, a);
            }
        }
        if a >= b {
            return CCursor { index: CharIndex(a), prefer_next_row: true };
        }
        let set = map.logical_of_screen(a..b);
        // The characters in front of the first one removed keep their places.
        let Some(&l0) = set.first() else { return CCursor { index: CharIndex(a), prefer_next_row: true } };
        self.remove_set(&set);
        self.settle(l0, a)
    }

    fn delete_previous_char(&mut self, ccursor: CCursor) -> CCursor {
        let map = self.map();
        let v = ccursor.index.0.min(map.screen_len());
        let l = self.logical(&map, v);
        // The nearest character before the caret that is drawn; direction controls on the way go too.
        let Some(p) = (0..l).rev().find(|p| map.to_screen.get(*p).is_some_and(Option::is_some)) else {
            return CCursor { index: CharIndex(v), ..ccursor };
        };
        self.remove_range(p, l);
        self.settle(p, v)
    }

    fn delete_next_char(&mut self, ccursor: CCursor) -> CCursor {
        let map = self.map();
        let v = ccursor.index.0.min(map.screen_len());
        let l = self.logical(&map, v);
        let Some(q) = (l..map.logical_len()).find(|q| map.to_screen.get(*q).is_some_and(Option::is_some)) else {
            return CCursor { index: CharIndex(v), ..ccursor };
        };
        self.remove_range(l, q.saturating_add(1));
        self.settle(l, v)
    }

    fn delete_previous_word(&mut self, max_ccursor: CCursor) -> CCursor {
        let map = self.map();
        let v = max_ccursor.index.0.min(map.screen_len());
        let l = self.logical(&map, v);
        let start = ccursor_previous_word(self.text, CCursor::new(l)).index.0.min(l);
        self.remove_range(start, l);
        self.settle(start, v)
    }

    fn delete_next_word(&mut self, min_ccursor: CCursor) -> CCursor {
        let map = self.map();
        let v = min_ccursor.index.0.min(map.screen_len());
        let l = self.logical(&map, v);
        let end = ccursor_next_word(self.text, CCursor::new(l)).index.0.min(map.logical_len());
        self.remove_range(l, end);
        self.settle(l, v)
    }

    fn delete_surrounding_chars(&mut self, cursor_range: CCursorRange, before_chars: usize, after_chars: usize) -> CCursorRange {
        let map = self.map();
        let n = map.screen_len();
        let [min, max] = cursor_range.sorted_cursors();
        let (a, b) = (min.index.0.min(n), max.index.0.min(n));
        let (lo, hi) = if a >= b {
            let l = self.logical(&map, a);
            (l, l)
        } else {
            let set = map.logical_of_screen(a..b);
            (set.first().copied().unwrap_or(0), set.last().map_or(0, |l| l.saturating_add(1)))
        };
        self.remove_range(hi, hi.saturating_add(after_chars));
        let start = lo.saturating_sub(before_chars);
        self.remove_range(start, lo);
        CCursorRange::one(self.settle(start, a))
    }

    fn delete_paragraph_before_cursor(&mut self, galley: &Galley, cursor_range: &CCursorRange) -> CCursor {
        let [min, max] = cursor_range.sorted_cursors();
        if min == max {
            // From the caret back to the start of its paragraph (of the text, in a single line), in
            // reading order; at the start already, the character before.
            let map = self.map();
            let v = max.index.0.min(map.screen_len());
            let l = self.logical(&map, v);
            let start = if self.multiline { self.paragraph_start(l) } else { 0 };
            if start >= l {
                return self.delete_previous_char(max);
            }
            self.remove_range(start, l);
            return self.settle(start, v);
        }
        // A selection goes, with what lies before it in the line; in a multi-line field just it,
        // as egui's idea of a paragraph there is a screen line.
        self.delete_selected_ccursor_range([if self.multiline { min } else { galley.begin() }, max])
    }

    fn delete_paragraph_after_cursor(&mut self, galley: &Galley, cursor_range: &CCursorRange) -> CCursor {
        let [min, max] = cursor_range.sorted_cursors();
        if min == max {
            let map = self.map();
            let v = min.index.0.min(map.screen_len());
            let l = self.logical(&map, v);
            let end = if self.multiline { self.paragraph_end(l) } else { map.logical_len() };
            if end <= l {
                return self.delete_next_char(min);
            }
            self.remove_range(l, end);
            return self.settle(l, v);
        }
        self.delete_selected_ccursor_range([min, if self.multiline { max } else { galley.end() }])
    }

    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<BidiBuffer<'static>>()
    }
}

// ---------------------------------------------------------------------------------------------
// The widget.

/// The builder settings of [`BidiEdit`]; the same names and meaning as on `egui::TextEdit`.
#[derive(Default)]
struct Settings {
    id: Option<Id>,
    id_salt: Option<IdSalt>,
    hint_text: Option<Atoms<'static>>,
    font: FontSelection,
    text_color: Option<Color32>,
    password: bool,
    frame: Option<Frame>,
    margin: Option<Margin>,
    desired_width: Option<f32>,
    desired_rows: Option<usize>,
    min_size: Option<Vec2>,
    halign: Option<Align>,
    background_color: Option<Color32>,
    char_limit: Option<usize>,
    return_key: Option<Option<egui::KeyboardShortcut>>,
}

impl Settings {
    /// The id the field will have, as `egui::TextEdit` would pick it.
    fn resolve_id(&self, ui: &mut Ui) -> Id {
        if let Some(id) = self.id {
            return id;
        }
        if let Some(salt) = self.id_salt {
            return ui.make_persistent_id(salt);
        }
        let id = ui.next_auto_id();
        ui.skip_ahead_auto_ids(1);
        id
    }

    /// `edit` with these settings.
    fn apply<'t>(&mut self, mut edit: TextEdit<'t>, id: Id) -> TextEdit<'t> {
        edit = edit.id(id).font(self.font.clone()).password(self.password);
        if let Some(hint) = self.hint_text.take() {
            edit = edit.hint_text(hint);
        }
        if let Some(color) = self.text_color {
            edit = edit.text_color(color);
        }
        if let Some(frame) = self.frame {
            edit = edit.frame(frame);
        }
        if let Some(margin) = self.margin {
            edit = edit.margin(margin);
        }
        if let Some(width) = self.desired_width {
            edit = edit.desired_width(width);
        }
        if let Some(rows) = self.desired_rows {
            edit = edit.desired_rows(rows);
        }
        if let Some(size) = self.min_size {
            edit = edit.min_size(size);
        }
        if let Some(align) = self.halign {
            edit = edit.horizontal_align(align);
        }
        if let Some(color) = self.background_color {
            edit = edit.background_color(color);
        }
        if let Some(limit) = self.char_limit {
            edit = edit.char_limit(limit);
        }
        if let Some(key) = self.return_key {
            edit = edit.return_key(key);
        }
        edit
    }
}

/// A text field with `egui::TextEdit`'s builder, that edits right-to-left script in display
/// order (see the [module](self)). Text without such script, passwords and multi-line fields are
/// a plain `TextEdit`.
#[must_use = "You should put this widget in a ui with `ui.add(widget);`"]
pub struct BidiEdit<'t> {
    text: &'t mut String,
    multiline: bool,
    s: Settings,
}

impl<'t> BidiEdit<'t> {
    /// A single-line field.
    pub fn singleline(text: &'t mut String) -> Self {
        Self { text, multiline: false, s: Settings::default() }
    }

    /// A multi-line field. Where it holds right-to-left script the field wraps each paragraph
    /// itself, at the width egui gives it, and shows the lines in display order, a right-to-left
    /// paragraph against the right edge (see the [module](self)).
    pub fn multiline(text: &'t mut String) -> Self {
        Self { text, multiline: true, s: Settings::default() }
    }

    /// Use this [`Id`] for the field.
    pub fn id(mut self, id: Id) -> Self {
        self.s.id = Some(id);
        self
    }

    /// Derive the [`Id`] from `salt`.
    pub fn id_salt(mut self, salt: impl egui::AsIdSalt) -> Self {
        self.s.id_salt = Some(IdSalt::new(salt));
        self
    }

    /// Show `hint` while the field is empty.
    pub fn hint_text(mut self, hint: impl IntoAtoms<'static>) -> Self {
        self.s.hint_text = Some(hint.into_atoms());
        self
    }

    /// Hide the letters and keep them from being copied. A password is never reordered.
    pub fn password(mut self, password: bool) -> Self {
        self.s.password = password;
        self
    }

    /// Pick the font.
    pub fn font(mut self, font: impl Into<FontSelection>) -> Self {
        self.s.font = font.into();
        self
    }

    /// The color of the text.
    pub fn text_color(mut self, color: Color32) -> Self {
        self.s.text_color = Some(color);
        self
    }

    /// Replace the frame around the field.
    pub fn frame(mut self, frame: Frame) -> Self {
        self.s.frame = Some(frame);
        self
    }

    /// The margin around the text.
    pub fn margin(mut self, margin: impl Into<Margin>) -> Self {
        self.s.margin = Some(margin.into());
        self
    }

    /// The width asked for; `f32::INFINITY` takes all there is.
    pub fn desired_width(mut self, width: f32) -> Self {
        self.s.desired_width = Some(width);
        self
    }

    /// The number of rows shown.
    pub fn desired_rows(mut self, rows: usize) -> Self {
        self.s.desired_rows = Some(rows);
        self
    }

    /// The smallest size of the field.
    pub fn min_size(mut self, size: Vec2) -> Self {
        self.s.min_size = Some(size);
        self
    }

    /// Where the text sits in the field.
    pub fn horizontal_align(mut self, align: Align) -> Self {
        self.s.halign = Some(align);
        self
    }

    /// The fill behind the text.
    pub fn background_color(mut self, color: Color32) -> Self {
        self.s.background_color = Some(color);
        self
    }

    /// The most characters the field takes (counted in the stored text).
    pub fn char_limit(mut self, limit: usize) -> Self {
        self.s.char_limit = Some(limit);
        self
    }

    /// The key that ends the line in a single-line field and starts a new one in a multi-line one
    /// (`None`: none does).
    pub fn return_key(mut self, key: impl Into<Option<egui::KeyboardShortcut>>) -> Self {
        self.s.return_key = Some(key.into());
        self
    }

    /// Show the field and return what `egui::TextEdit::show` does. The cursor in the output
    /// counts characters in screen order when the field holds right-to-left script.
    pub fn show(self, ui: &mut Ui) -> TextEditOutput {
        let Self { text, multiline, mut s } = self;
        let id = s.resolve_id(ui);
        restore_deferred(ui, id);
        let too_long = multiline && text.chars().nth(MAX_WRAPPED_CHARS).is_some();
        if s.password || too_long || !wants_reordering(ui, id, text) {
            forget_caret(ui.ctx(), id);
            let edit = if multiline { TextEdit::multiline(text) } else { TextEdit::singleline(text) };
            return s.apply(edit, id).show(ui);
        }
        show_reordered(ui, id, text, s, multiline)
    }
}

impl Widget for BidiEdit<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui).response.response
    }
}

/// Whether the field needs the screen-order treatment this frame: it holds right-to-left script,
/// or such script is about to be typed or pasted into it, or restored by an undo or redo (which
/// can bring back text the field does not hold now).
fn wants_reordering(ui: &Ui, id: Id, text: &str) -> bool {
    if bidi::has_rtl(text) {
        return true;
    }
    ui.memory(|m| m.has_focus(id))
        && ui.input(|i| {
            i.events.iter().any(|e| match e {
                Event::Text(t) | Event::Paste(t) | Event::Ime(egui::ImeEvent::Commit(t) | egui::ImeEvent::Preedit { text: t, .. }) => {
                    bidi::has_rtl(t)
                }
                Event::Key { key: Key::Z | Key::Y, pressed: true, modifiers, .. } => modifiers.command,
                _ => false,
            })
        })
}

/// Forgets the caret record of a field that is drawn the plain way this frame.
fn forget_caret(ctx: &egui::Context, id: Id) {
    ctx.data_mut(|d| {
        d.remove_temp::<Caret>(id);
        d.remove_temp::<Preedit>(id);
        d.remove_temp::<WrapShift>(id);
        d.remove_temp::<WrapUsed>(id);
        d.remove_temp::<KeptMap>(id);
        #[cfg(test)]
        d.insert_temp(Id::new(("bidi-test-map", id)), None::<ScreenMap>);
    });
}

/// The cursor egui should start the frame with, and the caret it stands for.
///
/// `stored` is what egui kept; `known` is the record of the last frame this field was drawn
/// reordered: without it the cursor was kept by a plain field, in logical indices.
fn initial_cursor(stored: Option<CCursorRange>, known: Option<Caret>, map: &ScreenMap, at_end: bool) -> (CCursorRange, Caret) {
    let (m, n) = (map.logical_len(), map.screen_len());
    let cursor_at = |v: usize, prefer_next_row: bool| CCursor { index: CharIndex(v), prefer_next_row };
    match stored {
        None => {
            let l = if at_end { m } else { 0 };
            let v = first(&map.screen_positions(l)).min(n);
            (CCursorRange::one(cursor_at(v, true)), Caret { l, v })
        }
        Some(range) if known.is_none() => {
            let to_screen = |c: CCursor| {
                let l = c.index.0.min(m);
                (l, first(&map.screen_positions(l)).min(n))
            };
            let ((l, v), (_, secondary)) = (to_screen(range.primary), to_screen(range.secondary));
            (
                CCursorRange {
                    primary: cursor_at(v, range.primary.prefer_next_row),
                    secondary: cursor_at(secondary, range.secondary.prefer_next_row),
                    h_pos: None,
                },
                Caret { l, v },
            )
        }
        Some(mut range) => {
            range.primary.index = CharIndex(range.primary.index.0.min(n));
            range.secondary.index = CharIndex(range.secondary.index.0.min(n));
            let v = range.primary.index.0;
            (range, Caret { l: logical_for(map, v, known), v })
        }
    }
}

/// The text the screen range of `cursor` holds, in logical order.
fn selected_text(text: &str, map: &ScreenMap, cursor: &CCursorRange) -> String {
    let [min, max] = cursor.sorted_cursors();
    let set = map.logical_of_screen(min.index.0..max.index.0);
    text.chars().enumerate().filter(|(i, _)| set.binary_search(i).is_ok()).map(|(_, c)| c).collect()
}

/// How the keys of this frame move the caret (the last one decides), if any.
fn key_motion(events: &[Event]) -> Option<Moved> {
    events.iter().rev().find_map(|e| match e {
        Event::Key { key: Key::ArrowLeft | Key::Home | Key::ArrowUp, pressed: true, .. } => Some(Moved::Left),
        Event::Key { key: Key::ArrowRight | Key::End | Key::ArrowDown, pressed: true, .. } => Some(Moved::Right),
        Event::Key { key: Key::A, pressed: true, modifiers, .. } if modifiers.command => Some(Moved::Right),
        _ => None,
    })
}

/// Which Home and End keys the field takes over to go to the ends of the text in reading order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HomeEnd {
    /// None: egui moves along the screen line.
    Off,
    /// All of them, in a single line of text that starts right to left (the line's ends are the
    /// text's, and Home is where reading starts).
    Line,
    /// Those with Ctrl, in a multi-line field (the plain ones stay on the screen line).
    Text,
}

/// Whether `event` is a Home or End `mode` takes over: whether it is Home, and whether Shift is down.
fn home_end(event: &Event, mode: HomeEnd) -> Option<(bool, bool)> {
    match event {
        Event::Key { key: key @ (Key::Home | Key::End), pressed: true, modifiers, .. }
            if !modifiers.alt
                && !modifiers.mac_cmd
                && match mode {
                    HomeEnd::Off => false,
                    HomeEnd::Line => true,
                    HomeEnd::Text => modifiers.ctrl,
                } =>
        {
            Some((*key == Key::Home, modifiers.shift))
        }
        _ => None,
    }
}

/// What [`take_over_events`] did.
#[derive(Default)]
struct Takeover {
    /// A Copy or Cut was taken: the selection goes to the clipboard in logical order.
    copied: bool,
    /// A Home or End was taken: the caret goes to this logical place; the flag is Shift.
    motion: Option<(usize, bool)>,
}

/// Takes from `events` the ones egui would apply wrongly to a field in screen order: a Copy and
/// a Cut of a non-empty selection (a Cut stays, as a Backspace over the selection) and the Home
/// and End `home_end` names, which go to the start and end of the text in reading order.
fn take_over_events(events: &mut Vec<Event>, has_selection: bool, home_end_mode: HomeEnd, logical_len: usize) -> Takeover {
    let mut taken = Takeover::default();
    let mut kept = Vec::with_capacity(events.len());
    for event in std::mem::take(events) {
        match event {
            Event::Copy if has_selection => taken.copied = true,
            Event::Cut if has_selection => {
                taken.copied = true;
                kept.push(Event::Key {
                    key: Key::Backspace,
                    physical_key: Some(Key::Backspace),
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                });
            }
            other => match home_end(&other, home_end_mode) {
                Some((home, shift)) => taken.motion = Some((if home { 0 } else { logical_len }, shift)),
                None => kept.push(other),
            },
        }
    }
    *events = kept;
    taken
}

/// How a field lays its text out: what egui's default layouter would use, for the text in
/// screen order.
#[derive(Clone)]
struct Look {
    font: FontId,
    color: Color32,
    /// Where a single line sits in the field.
    halign: Align,
    line_height: f32,
}

impl Look {
    /// The galley of `map` for `egui::TextEdit`: one glyph per drawn character, in screen order.
    /// A single line (`wrap_width` `None`) is laid out as a `TextEdit::singleline` does; with a
    /// width, the lines the map is wrapped into (see [`bidi::wrapped_screen_map`]) one to a row,
    /// each against the edge its paragraph reads from.
    fn layout(&self, ui: &Ui, map: &ScreenMap, wrap_width: Option<f32>) -> Arc<Galley> {
        let mut job = match wrap_width {
            // The lines are already broken: egui must not break them again.
            Some(_) => LayoutJob::simple(map.shaper.clone(), self.font.clone(), self.color, f32::INFINITY),
            None => LayoutJob::simple_singleline(map.shaper.clone(), self.font.clone(), self.color),
        };
        job.halign = if wrap_width.is_some() { Align::Min } else { self.halign };
        // Typing a space must show at once, as in a plain field.
        job.keep_trailing_whitespace = true;
        for section in &mut job.sections {
            section.format.line_height = Some(self.line_height);
        }
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        let galley = align_glyphs(galley, map, wrap_width.is_some());
        match wrap_width {
            Some(width) => place_rows(galley, map, width, ui.ctx().pixels_per_point()),
            None => galley,
        }
    }
}

/// `galley` (its rows laid out from the left) with the rows of right-to-left paragraphs moved
/// against the right edge of a galley `width` wide. The galley's rect then spans the whole
/// width, so egui puts it at the field's left edge whatever its alignment, and left-to-right and
/// right-to-left paragraphs sit at their own sides.
fn place_rows(mut galley: Arc<Galley>, map: &ScreenMap, width: f32, pixels_per_point: f32) -> Arc<Galley> {
    if !map.row_rtl.iter().any(|rtl| *rtl) {
        return galley;
    }
    let inner = Arc::make_mut(&mut galley);
    let width = if width.is_finite() { width.floor().max(inner.rect.width()) } else { inner.rect.width() };
    let snap = |x: f32| if pixels_per_point > 0.0 && pixels_per_point.is_finite() { (x * pixels_per_point).round() / pixels_per_point } else { x };
    let mut bounds = egui::Rect::NOTHING;
    for (i, placed) in inner.rows.iter_mut().enumerate() {
        if map.row_is_rtl(i) {
            placed.pos.x = snap((inner.rect.min.x + width - placed.row.size.x).max(inner.rect.min.x));
        }
        bounds |= placed.row.visuals.mesh_bounds.translate(placed.pos.to_vec2());
    }
    inner.rect.max.x = inner.rect.min.x + width;
    inner.mesh_bounds = bounds;
    galley
}

/// The width egui will lay the field's text out at, from the field's settings and the room it has.
/// It is needed before egui says what it is, to put the cursor egui kept on the screen.
fn estimate_wrap_width(ui: &Ui, s: &Settings) -> f32 {
    let min = s.min_size.map_or(0.0, |size| size.x);
    let available = ui.available_width().max(24.0);
    let wanted = s.desired_width.unwrap_or(ui.spacing().text_edit_width).max(min);
    let allocated = wanted.min(available).max(min);
    let margin = match s.frame {
        Some(frame) => frame.total_margin().sum().x,
        None => s.margin.unwrap_or_else(|| Margin::symmetric(4, 2)).sum().x,
    };
    allocated - margin
}

/// How far the width egui laid a multi-line field's text out at was from [`estimate_wrap_width`]
/// the last frame (nothing when they agree, which they do for a field in an ordinary layout), so
/// a layout the estimate does not know is corrected from the second frame on.
#[derive(Clone, Copy, Default)]
struct WrapShift(f32);

/// The width (its bits) a multi-line field's text was last laid out at, which the cursor egui
/// kept is in terms of.
#[derive(Clone, Copy, Default)]
struct WrapUsed(u32);

/// Events held back from the frame they came in, to run in the next one.
#[derive(Clone, Default)]
struct Deferred {
    events: Vec<Event>,
    /// When they were held back.
    time: f64,
}

/// How long a held-back event stays good: longer than that, the user has gone elsewhere.
const DEFERRED_SECS: f64 = 1.0;

/// Whether `event` is an Undo or Redo.
fn is_undo_redo(event: &Event) -> bool {
    matches!(event, Event::Key { key: Key::Z | Key::Y, pressed: true, modifiers, .. } if modifiers.command)
}

/// Holds back the events that follow an Undo or Redo in this frame, for the next.
///
/// An undo gives back the cursor it took with the text, and in a wrapped field that cursor counts
/// the lines of the width the field had then. If the field has been resized since, it can lie past
/// the end of what is drawn now, and an event that moves by words in the same frame would read
/// the text from there. After the frame the cursor is kept inside the text (see `show_reordered`).
fn defer_after_undo(ui: &Ui, id: Id) {
    let later = ui.input_mut(|i| {
        let at = i.events.iter().position(is_undo_redo)?;
        let later = i.events.split_off(at.saturating_add(1));
        (!later.is_empty()).then_some(later)
    });
    if let Some(events) = later {
        let time = ui.input(|i| i.time);
        ui.ctx().data_mut(|d| d.insert_temp(id, Deferred { events, time }));
        ui.ctx().request_repaint();
    }
}

/// Puts back at the front of this frame's events the ones [`defer_after_undo`] held back, if the
/// field still has the focus.
fn restore_deferred(ui: &Ui, id: Id) {
    let Some(held) = ui.ctx().data_mut(|d| d.remove_temp::<Deferred>(id)) else { return };
    let now = ui.input(|i| i.time);
    if ui.memory(|m| m.has_focus(id)) && now - held.time <= DEFERRED_SECS {
        ui.input_mut(|i| {
            i.events.splice(0..0, held.events);
        });
    }
}

fn show_reordered(ui: &mut Ui, id: Id, text: &mut String, mut s: Settings, multiline: bool) -> TextEditOutput {
    let ctx = ui.ctx().clone();
    let cache: SharedCache = Rc::default();
    let caret: SharedCaret = Rc::default();

    // The layouter: the default one of a `TextEdit`, but for the text in display order.
    let font = s.font.clone().resolve(ui.style());
    let row_height = ui.fonts_mut(|f| f.row_height(&font));
    let line_height = row_height + ui.spacing().extra_text_line_spacing;
    let color = s.text_color.or(ui.visuals().override_text_color).unwrap_or_else(|| ui.visuals().widgets.inactive.text_color());
    let look = Look { font: font.clone(), color, halign: s.halign.unwrap_or(Align::Min), line_height };

    // A multi-line field wraps its paragraphs itself, at the width egui hands the layouter; the
    // width is wanted before that (to put the cursor egui kept on the screen).
    let estimate = multiline.then(|| estimate_wrap_width(ui, &s));
    let wrap_width: Option<f32> = estimate.map(|width| width + ctx.data(|d| d.get_temp::<WrapShift>(id)).unwrap_or_default().0);
    if let (Some(width), Ok(mut shared)) = (wrap_width, cache.try_borrow_mut()) {
        shared.wrap = Some(Wrap { ctx: ctx.clone(), font, width, neutral_rtl: crate::rtl::mirrored() });
    }
    if let Some(kept) = ctx.data(|d| d.get_temp::<KeptMap>(id)) {
        seed_cache(&cache, kept, text);
    }
    let map = map_of(&cache, text);

    // The cursor egui starts from, in screen order, and the caret it stands for.
    let mut known: Option<Caret> = ctx.data(|d| d.get_temp(id));
    let mut state = TextEditState::load(&ctx, id).unwrap_or_default();
    let kept = state.cursor.char_range();
    let mut stored = kept;
    let used: Option<WrapUsed> = ctx.data(|d| d.get_temp(id));
    if let (Some(width), Some(WrapUsed(old)), Some(was), Some(range)) = (wrap_width, used, known, stored)
        && old != width.to_bits()
    {
        // The field was wrapped at another width: the cursor egui kept is a screen position of
        // that wrapping, so it is put where the same logical places are drawn now.
        let (before, after) = (f32::from_bits(old), width);
        set_wrap_width(&cache, before);
        let old_map = map_of(&cache, text);
        set_wrap_width(&cache, after);
        let to_logical = |index: usize, caret: Option<Caret>| logical_for(&old_map, index.min(old_map.screen_len()), caret);
        let (primary, secondary) = (to_logical(range.primary.index.0, Some(was)), to_logical(range.secondary.index.0, None));
        let screen = |l: usize| first(&map.screen_positions(l)).min(map.screen_len());
        let at = |l: usize, prefer_next_row: bool| CCursor { index: CharIndex(screen(l)), prefer_next_row };
        let moved = CCursorRange { primary: at(primary, true), secondary: at(secondary, true), h_pos: None };
        known = Some(Caret { l: primary.min(map.logical_len()), v: screen(primary) });
        stored = Some(moved);
    }
    let (mut cursor, mut hint) = initial_cursor(stored, known, &map, true);
    let focused = ui.memory(|m| m.has_focus(id));

    // Events egui would apply to the wrong characters: Copy and Cut slice the logical text with
    // screen indices, Home and End of text that starts right to left go to the screen edges, and
    // Left and Right would stop at the zero-width places inside a ligature.
    let rtl_start = bidi::first_strong_rtl(text) == Some(true);
    if focused && multiline {
        defer_after_undo(ui, id);
    }
    if focused {
        let selection = selected_text(text, &map, &cursor);
        let logical_len = map.logical_len();
        let (plain_arrows, only_motion) = ui.input(|i| (i.events.iter().any(is_plain_arrow), i.events.iter().all(|e| !edits_text(e))));
        let stops: Vec<Pos2> = if plain_arrows && only_motion {
            let galley = look.layout(ui, &map, wrap_width);
            (0..=map.screen_len()).map(|v| galley.pos_from_cursor(CCursor::new(v)).min).collect()
        } else {
            Vec::new()
        };
        let home_end = match (multiline, only_motion) {
            (true, true) => HomeEnd::Text,
            (false, true) if rtl_start => HomeEnd::Line,
            _ => HomeEnd::Off,
        };
        let taken = ui.input_mut(|i| take_over_events(&mut i.events, !selection.is_empty(), home_end, logical_len));
        if taken.copied {
            ctx.copy_text(selection);
        }
        if let Some((l, shift)) = taken.motion {
            let v = first(&map.screen_positions(l)).min(map.screen_len());
            cursor.primary = CCursor { index: CharIndex(v), prefer_next_row: true };
            if !shift {
                cursor.secondary = cursor.primary;
            }
            hint = Caret { l, v };
        }
        if !stops.is_empty() {
            let moved = ui.input_mut(|i| take_arrows(&mut i.events, &mut cursor, &stops));
            if let Some(how) = moved {
                let v = cursor.primary.index.0;
                hint = Caret { l: derive_logical(&map, v, how), v };
            }
        }
    }
    if kept != Some(cursor) {
        state.cursor.set_char_range(Some(cursor));
        state.store(&ctx, id);
    }
    caret.set(Some(hint));
    let keys = ui.input(|i| key_motion(&i.events));

    // An input method's composition is a stretch of the text the field keeps replacing.
    let (composing, any_ime) = ui.input(|i| {
        let texts: Vec<String> = i
            .events
            .iter()
            .filter_map(|e| if let Event::Ime(egui::ImeEvent::Preedit { text, .. }) = e { Some(text.clone()) } else { None })
            .collect();
        (texts, i.events.iter().any(|e| matches!(e, Event::Ime(_))))
    });
    let preedit: SharedPreedit = Rc::new(Cell::new(ctx.data(|d| d.get_temp(id))));

    let layout_cache = Rc::clone(&cache);
    let layout_look = look.clone();
    let mut layouter = move |ui: &Ui, buffer: &dyn TextBuffer, width: f32| -> Arc<Galley> {
        if multiline {
            set_wrap_width(&layout_cache, width);
        }
        let map = map_of(&layout_cache, buffer.as_str());
        layout_look.layout(ui, &map, multiline.then_some(width))
    };

    let mut out = {
        let mut buffer = BidiBuffer {
            text: &mut *text,
            cache: Rc::clone(&cache),
            caret: Rc::clone(&caret),
            preedit: Rc::clone(&preedit),
            composing,
            any_ime,
            multiline,
        };
        let edit = if multiline { TextEdit::multiline(&mut buffer) } else { TextEdit::singleline(&mut buffer) };
        s.apply(edit, id).layouter(&mut layouter).show(ui)
    };
    let span = preedit.take();
    ctx.data_mut(|d| match span {
        Some(span) => {
            d.insert_temp(id, span);
        }
        None => {
            d.remove_temp::<Preedit>(id);
        }
    });
    if let (Some(estimate), Some(laid_out)) = (estimate, cache.try_borrow().ok().and_then(|c| c.wrap.as_ref().map(|w| w.width))) {
        let shift = laid_out - estimate;
        ctx.data_mut(|d| {
            d.insert_temp(id, WrapUsed(laid_out.to_bits()));
            if shift.is_finite() && shift.abs() > 0.01 {
                d.insert_temp(id, WrapShift(shift));
            } else {
                d.remove_temp::<WrapShift>(id);
            }
        });
    }

    // Which logical place the cursor egui left means: where the pointer is if it is on the field
    // (two places can share a screen position, and the click may land on either), else what the
    // adapter set while editing, else the movement that brought the cursor there.
    let map = map_of(&cache, text);
    if multiline && let Some(kept) = kept_map(&cache) {
        ctx.data_mut(|d| d.insert_temp(id, kept));
    }
    #[cfg(test)]
    ctx.data_mut(|d| d.insert_temp(Id::new(("bidi-test-map", id)), Some((*map).clone())));
    // An undo gives back the cursor it took with the text, which for a wrapped text can be a
    // position of another wrapping (the field was resized since): never past the end of this one.
    if let Some(range) = out.state.cursor.char_range() {
        let end = out.galley.end().index;
        if range.primary.index > end || range.secondary.index > end {
            let clamp = |c: CCursor| CCursor { index: c.index.min(end), prefer_next_row: c.prefer_next_row };
            out.state.cursor.set_char_range(Some(CCursorRange { primary: clamp(range.primary), secondary: clamp(range.secondary), h_pos: None }));
            out.state.clone().store(&ctx, id);
        }
    }
    if let Some(range) = out.state.cursor.char_range() {
        let v = range.primary.index.0.min(map.screen_len());
        let known = caret.get().filter(|c| c.v == v && map.screen_positions(c.l).contains(&v));
        let how = match (out.response.response.interact_pointer_pos(), known) {
            (Some(pointer), _) => {
                let boundary = out.galley.pos_from_cursor(CCursor::new(v)).min.x + out.galley_pos.x - out.galley.rect.left();
                Some(Moved::Pointer { pointer_x: pointer.x, boundary_x: boundary })
            }
            (None, Some(_)) => None,
            (None, None) => Some(keys.unwrap_or(Moved::External)),
        };
        let l = how.map_or_else(|| logical_for(&map, v, known), |how| derive_logical(&map, v, how));
        if multiline && !bidi::has_rtl(text) {
            // Nothing right to left is left in the field, so the next frame draws it as a plain
            // `TextEdit` whose cursor counts the text's characters, not the screen's (a line
            // break the wrapping made counts as one of them).
            hand_back_cursor(&ctx, id, &map, &range, l);
            ctx.data_mut(|d| d.remove_temp::<Caret>(id));
        } else {
            ctx.data_mut(|d| d.insert_temp(id, Caret { l, v }));
        }
    }
    out
}

/// Stores the cursor `range` (screen indices of `map`, its primary at logical place `l`) of the
/// field `id` as logical indices, for the plain `TextEdit` it becomes next frame to carry on with.
/// (The output of this frame stays as the galley of this frame says.)
fn hand_back_cursor(ctx: &egui::Context, id: Id, map: &ScreenMap, range: &CCursorRange, l: usize) {
    let Some(mut state) = TextEditState::load(ctx, id) else { return };
    let secondary = first(&map.logical_positions(range.secondary.index.0));
    let at = |index: usize, prefer_next_row: bool| CCursor { index: CharIndex(index), prefer_next_row };
    state.cursor.set_char_range(Some(CCursorRange {
        primary: at(l, range.primary.prefer_next_row),
        secondary: at(secondary, range.secondary.prefer_next_row),
        h_pos: None,
    }));
    state.store(ctx, id);
}

/// A Left or Right arrow that moves the caret one place (Shift extends the selection); with
/// Ctrl, Alt or Cmd they jump by words or lines, which egui does right.
fn is_plain_arrow(event: &Event) -> bool {
    matches!(event, Event::Key { key: Key::ArrowLeft | Key::ArrowRight, pressed: true, modifiers, .. } if !modifiers.ctrl && !modifiers.alt && !modifiers.mac_cmd && !modifiers.command)
}

/// Whether `event` changes the text (or an input method's composition).
fn edits_text(event: &Event) -> bool {
    match event {
        Event::Text(_) | Event::Paste(_) | Event::Cut | Event::Ime(_) => true,
        Event::Key { key, pressed: true, .. } => {
            matches!(key, Key::Backspace | Key::Delete | Key::Enter | Key::Tab | Key::Z | Key::Y | Key::H | Key::K | Key::U | Key::W)
        }
        _ => false,
    }
}

/// Takes the plain Left and Right arrows from `events` and moves `cursor` the way egui does,
/// except that a step goes on to the next place that is drawn somewhere else: `stops` holds where
/// every screen position is drawn, and the places inside a ligature share one. Returns the
/// direction of the last arrow.
fn take_arrows(events: &mut Vec<Event>, cursor: &mut CCursorRange, stops: &[Pos2]) -> Option<Moved> {
    let last = stops.len().saturating_sub(1);
    let mut moved = None;
    events.retain(|event| {
        let Event::Key { key: key @ (Key::ArrowLeft | Key::ArrowRight), pressed: true, modifiers, .. } = event else { return true };
        if !is_plain_arrow(event) {
            return true;
        }
        let right = *key == Key::ArrowRight;
        let [min, max] = cursor.sorted_cursors();
        let target = if modifiers.shift {
            step(stops, cursor.primary.index.0.min(last), right)
        } else if !cursor.is_empty() {
            // A selection collapses to its near end, as in egui.
            if right { max.index.0.min(last) } else { min.index.0.min(last) }
        } else {
            step(stops, cursor.primary.index.0.min(last), right)
        };
        cursor.primary = CCursor { index: CharIndex(target), prefer_next_row: true };
        if !modifiers.shift {
            cursor.secondary = cursor.primary;
        }
        cursor.h_pos = None;
        moved = Some(if right { Moved::Right } else { Moved::Left });
        false
    });
    moved
}

/// The screen position one step left or right of `from` that is drawn somewhere else (another x
/// or another line), or the end.
fn step(stops: &[Pos2], from: usize, right: bool) -> usize {
    let last = stops.len().saturating_sub(1);
    let here = stops.get(from).copied().unwrap_or(Pos2::ZERO);
    let mut at = from;
    loop {
        at = if right { at.saturating_add(1).min(last) } else { at.saturating_sub(1) };
        let edge = if right { at >= last } else { at == 0 };
        if edge || stops.get(at).is_none_or(|p| (p.x - here.x).abs() > 0.01 || (p.y - here.y).abs() > 0.5) {
            return at;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{PointerButton, Pos2, RawInput, Rect, vec2};

    const AIN: &str = "\u{639}";
    const LAM: &str = "\u{644}";
    const YA: &str = "\u{64A}";
    /// "ab" and the three letters of "ali" (ain, lam, ya), which draw as "ab" then ya, lam, ain.
    const AB_ALI: &str = "ab\u{639}\u{644}\u{64A}";

    fn key(key: Key) -> Event {
        Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: Modifiers::NONE }
    }

    /// A focused field in its own egui context; frames are driven by hand.
    struct Field {
        ctx: egui::Context,
        text: String,
        id: Id,
        time: f64,
        /// What the field put on the clipboard, in order.
        copied: Vec<String>,
        char_limit: Option<usize>,
        /// A multi-line field, `width` wide.
        multiline: bool,
        width: f32,
        /// A plain `egui::TextEdit` instead, to compare with.
        plain: bool,
    }

    impl Field {
        fn new(fonts: egui::FontDefinitions) -> Self {
            let ctx = egui::Context::default();
            ctx.set_os(egui::os::OperatingSystem::Nix);
            ctx.set_fonts(fonts);
            Self {
                ctx,
                text: String::new(),
                id: Id::new("bidi-field"),
                time: 0.0,
                copied: Vec::new(),
                char_limit: None,
                multiline: false,
                width: 300.0,
                plain: false,
            }
        }

        /// A field in a build with an Arabic face (craft-fonts, or `PDFCRAFT_TEST_ARABIC_FONT`).
        fn arabic() -> Option<Self> {
            Some(Self::new(bidi::arabic_test_fonts()?))
        }

        /// [`Self::arabic`] holding `text`, laid out once.
        fn arabic_with(text: &str) -> Option<(Self, TextEditOutput)> {
            let mut field = Self::arabic()?;
            field.text = text.to_owned();
            let out = field.frame(vec![]);
            Some((field, out))
        }

        fn frame(&mut self, events: Vec<Event>) -> TextEditOutput {
            self.time += 0.1;
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 100.0))),
                events,
                time: Some(self.time),
                ..Default::default()
            };
            let (id, text, limit, width) = (self.id, &mut self.text, self.char_limit, self.width);
            let (multiline, plain) = (self.multiline, self.plain);
            let mut out = None;
            let frame = self.ctx.run_ui(raw, |ui| {
                ui.memory_mut(|m| m.request_focus(id));
                if plain {
                    out = Some(TextEdit::multiline(text).id(id).desired_width(width).show(ui));
                    return;
                }
                let mut edit = if multiline { BidiEdit::multiline(text) } else { BidiEdit::singleline(text) }.id(id).desired_width(width);
                if let Some(limit) = limit {
                    edit = edit.char_limit(limit);
                }
                out = Some(edit.show(ui));
            });
            for command in &frame.platform_output.commands {
                if let egui::OutputCommand::CopyText(copied) = command {
                    self.copied.push(copied.clone());
                }
            }
            frame.drop_without_applying_deltas();
            out.expect("the field is shown every frame")
        }

        fn type_text(&mut self, text: &str) -> TextEditOutput {
            self.frame(vec![Event::Text(text.to_owned())])
        }

        fn press(&mut self, key: Key) -> TextEditOutput {
            self.frame(vec![self::key(key)])
        }

        fn press_with(&mut self, key: Key, modifiers: Modifiers) -> TextEditOutput {
            self.frame(vec![Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }])
        }

        /// A quiet moment: long enough for egui's undo to take what was typed as a step.
        fn idle(&mut self) -> TextEditOutput {
            self.time += 2.0;
            self.frame(vec![])
        }

        /// Selects screen positions `from..to` with the keyboard: Home, then Shift+Right.
        fn select(&mut self, from: usize, to: usize) {
            self.press(Key::Home);
            for _ in 0..from {
                self.press(Key::ArrowRight);
            }
            for _ in from..to {
                self.press_with(Key::ArrowRight, Modifiers::SHIFT);
            }
        }

        fn caret(&self) -> Option<Caret> {
            self.ctx.data(|d| d.get_temp(self.id))
        }

        /// The screen map the field used the last frame (it was shown reordered).
        fn map(&self) -> Option<ScreenMap> {
            self.ctx.data(|d| d.get_temp::<Option<ScreenMap>>(Id::new(("bidi-test-map", self.id)))).flatten()
        }

        /// A focused multi-line field `width` wide holding `text`, laid out once.
        fn multiline_with(width: f32, text: &str) -> Option<(Self, TextEditOutput)> {
            let mut field = Self::arabic()?;
            field.multiline = true;
            field.width = width;
            field.text = text.to_owned();
            let out = field.frame(vec![]);
            Some((field, out))
        }
    }

    /// The screen position of egui's cursor in `out`, and where it is drawn (screen x).
    fn cursor_of(out: &TextEditOutput) -> (usize, f32) {
        let v = out.state.cursor.char_range().map_or(0, |r| r.primary.index.0);
        (v, out.galley_pos.x - out.galley.rect.left() + out.galley.pos_from_cursor(CCursor::new(v)).min.x)
    }

    /// Where glyph `i` starts on screen, and where it ends.
    fn glyph_span(out: &TextEditOutput, i: usize) -> (f32, f32) {
        let glyph = out.galley.rows[0].glyphs[i];
        let origin = out.galley_pos.x - out.galley.rect.left();
        (origin + glyph.pos.x, origin + glyph.pos.x + glyph.advance_width)
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1.5
    }

    #[test]
    fn typing_latin_then_arabic_stores_logical_text_and_keeps_the_caret_at_the_screen_edge() {
        let Some(mut field) = Field::arabic() else { return };
        field.frame(vec![]);
        field.type_text("a");
        let out = field.type_text("b");
        assert_eq!(field.text, "ab");
        assert_eq!(field.caret(), None, "Latin text is a plain field");
        let b_end = glyph_span(&out, 1).1;
        assert!(near(cursor_of(&out).1, b_end), "{} vs {b_end}", cursor_of(&out).1);

        for (typed, letters) in [(AIN, 3), (LAM, 4), (YA, 5)] {
            let out = field.type_text(typed);
            // Stored in logical order; drawn with each new letter to the left of the last.
            assert_eq!(field.text.chars().count(), letters);
            assert!(field.text.starts_with("ab"));
            let glyphs: String = out.galley.rows[0].glyphs.iter().map(|g| g.chr).collect();
            assert_eq!(glyphs.chars().count(), letters, "one glyph per character: {glyphs:?}");
            assert_eq!(field.caret(), Some(Caret { l: letters, v: 2 }), "the caret stays where the next letter appears");
            let (v, x) = cursor_of(&out);
            assert_eq!(v, 2);
            assert!(near(x, b_end), "caret x {x} is the right edge of b {b_end}");
            assert!(near(x, glyph_span(&out, 2).0), "and the left edge of the newest letter");
        }
        assert_eq!(field.text, AB_ALI);
        let out = field.frame(vec![]);
        let shown: String = out.galley.rows[0].glyphs.iter().map(|g| g.chr).collect();
        assert_eq!(shown, "ab\u{64A}\u{644}\u{639}", "ya, lam, ain left to right");
        assert!(out.galley.rows[0].glyphs.windows(2).all(|w| w[1].pos.x >= w[0].pos.x - 0.01));
    }

    #[test]
    fn backspace_removes_the_letter_typed_last() {
        let Some((mut field, _)) = Field::arabic_with(AB_ALI) else { return };
        assert_eq!(field.caret(), Some(Caret { l: 5, v: 2 }), "a field opens with its caret at the end of the reading order, not of the screen");
        let out = field.press(Key::Backspace);
        assert_eq!(field.text, "ab\u{639}\u{644}");
        assert_eq!(field.caret(), Some(Caret { l: 4, v: 2 }));
        // The caret stays at the left edge of what is now the newest letter.
        assert!(near(cursor_of(&out).1, glyph_span(&out, 2).0));
        field.press(Key::Backspace);
        field.press(Key::Backspace);
        assert_eq!(field.text, "ab");
        let out = field.press(Key::Backspace);
        assert_eq!(field.text, "a");
        assert!(near(cursor_of(&out).1, glyph_span(&out, 0).1));
    }

    #[test]
    fn a_click_picks_the_logical_place_next_to_the_character_it_is_over() {
        let Some((mut field, out)) = Field::arabic_with(AB_ALI) else { return };
        // Boundary 2 lies between the "b" and the newest letter: the end of "ab" in reading order
        // on its left, and the end of the whole text on its right.
        let (b_start, b_end) = glyph_span(&out, 1);
        let y = out.response.response.rect.center().y;
        let click = |field: &mut Field, x: f32| {
            // Not a double click.
            field.time += 2.0;
            let at = Pos2::new(x, y);
            field.frame(vec![Event::PointerMoved(at)]);
            field.frame(vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
            field.frame(vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
            field.caret()
        };
        assert_eq!(click(&mut field, b_end - 2.0), Some(Caret { l: 2, v: 2 }), "over the b");
        assert_eq!(click(&mut field, b_end + 2.0), Some(Caret { l: 5, v: 2 }), "over the newest letter");
        assert_eq!(click(&mut field, b_start + 1.0), Some(Caret { l: 1, v: 1 }), "inside the b, near its left edge");
        // Typing goes where the caret is.
        click(&mut field, b_end + 2.0);
        field.type_text("X");
        assert_eq!(field.text, "ab\u{639}\u{644}\u{64A}X");
        click(&mut field, b_end - 2.0);
        field.type_text("Y");
        assert_eq!(field.text, "abY\u{639}\u{644}\u{64A}X");
    }

    #[test]
    fn dragging_across_arabic_selects_one_stretch() {
        let Some((mut field, out)) = Field::arabic_with(AB_ALI) else { return };
        let y = out.response.response.rect.center().y;
        let from = Pos2::new(glyph_span(&out, 2).0 + 1.0, y);
        let to = Pos2::new(glyph_span(&out, 4).0 + 1.0, y);
        field.frame(vec![Event::PointerMoved(from)]);
        field.frame(vec![Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
        let out = field.frame(vec![Event::PointerMoved(Pos2::new((from.x + to.x) / 2.0, y))]);
        let out2 = field.frame(vec![Event::PointerMoved(to)]);
        drop(out);
        let range = out2.state.cursor.char_range().expect("a selection");
        assert_eq!(range.as_sorted_char_range(), CharIndex(2)..CharIndex(4), "the two letters between the pointer positions");
        // One highlight rectangle, as wide as the two glyphs.
        let fill = egui::Visuals::default().selection.bg_fill;
        let xs: Vec<f32> = out2.galley.rows[0].visuals.mesh.vertices.iter().filter(|v| v.color == fill).map(|v| v.pos.x).collect();
        assert_eq!(xs.len(), 4, "one rectangle: {xs:?}");
        let (min, max) = xs.iter().fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(*x), hi.max(*x)));
        let wanted = glyph_span(&out2, 2).1 - glyph_span(&out2, 2).0 + glyph_span(&out2, 3).1 - glyph_span(&out2, 3).0;
        assert!(near(max - min, wanted), "{} vs {wanted}", max - min);
        // The letters under the highlight, and only they, change colour: the glyphs' vertices
        // are the ones the galley's glyph list says they are, whatever the face drew them with.
        let selected = egui::Visuals::default().selection.stroke.color;
        let recoloured: Vec<f32> = out2.galley.rows[0].visuals.mesh.vertices.iter().filter(|v| v.color == selected).map(|v| v.pos.x).collect();
        assert!(!recoloured.is_empty(), "the selected letters change colour");
        assert!(recoloured.iter().all(|x| *x >= min - 3.0 && *x <= max + 3.0), "{recoloured:?} within {min}..{max}");
        let outside = out2.galley.rows[0]
            .visuals
            .mesh
            .vertices
            .iter()
            .filter(|v| v.color != selected && v.color != fill && v.pos.x > min + 3.0 && v.pos.x < max - 3.0)
            .count();
        assert_eq!(outside, 0, "no letter under the highlight keeps its colour");
        // Typing replaces exactly the highlighted letters (ya and lam), whatever order they are
        // stored in; the new letter follows the ain, which is what is left of the word.
        field.frame(vec![Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
        field.type_text("X");
        assert_eq!(field.text, "ab\u{639}X");
    }

    /// What the field needs of the galley of `text`: glyph `i` is the `i`-th character from the
    /// left, `x` never goes back and the glyphs' vertices follow each other (the selection paints
    /// the text it covers by them).
    fn assert_aligned(out: &TextEditOutput, text: &str) {
        let map = bidi::screen_map(text);
        let n = map.screen_len();
        let Some(row) = out.galley.rows.first() else {
            assert_eq!(n, 0, "{text:?}: a galley without rows");
            return;
        };
        let glyphs = &row.glyphs;
        assert_eq!(glyphs.len(), n, "{text:?}: one glyph per character");
        assert_eq!(out.galley.end().index.0, n, "{text:?}");
        if !map.drawn.iter().any(|c| bidi::is_shaped_rtl(*c)) {
            return;
        }
        for (i, (glyph, drawn)) in glyphs.iter().zip(&map.drawn).enumerate() {
            assert_eq!(glyph.chr, *drawn, "{text:?}: glyph {i} is the character drawn there");
        }
        assert!(
            glyphs.windows(2).all(|w| w[1].pos.x >= w[0].pos.x),
            "{text:?}: x never goes back: {:?}",
            glyphs.iter().map(|g| g.pos.x).collect::<Vec<_>>()
        );
        assert!(glyphs.windows(2).all(|w| w[1].first_vertex >= w[0].first_vertex), "{text:?}: vertices follow each other");
        let vertices = &row.visuals.glyph_vertex_range;
        assert!(glyphs.iter().all(|g| (g.first_vertex as usize) <= vertices.end), "{text:?}: vertices stay in the row");
    }

    #[test]
    fn galleys_have_one_glyph_per_drawn_character_whatever_the_face_makes_of_them() {
        let Some(mut field) = Field::arabic() else { return };
        field.frame(vec![]);
        // Words (a face draws a letter as one glyph or as a letter with dots), lam-alef (one glyph
        // in some faces, two in others), letters with marks (zero-width glyphs, before the letter
        // or after it), a joiner, Latin and digits around them, and marks with no letter.
        for text in [
            "\u{627}\u{644}\u{627}\u{633}\u{645}",
            "\u{627}\u{644}\u{644}\u{647}",
            "\u{644}\u{627}",
            "x \u{644}\u{627} y",
            "\u{644}\u{64E}\u{627}",
            "\u{633}\u{64E}\u{644}\u{627}\u{645}",
            "\u{639}\u{639}\u{64E}\u{639}",
            "\u{633}\u{64E}\u{64F}\u{650}",
            "\u{64E}",
            "\u{64E}\u{64E}\u{633}",
            "\u{633}\u{64E}\u{64E}",
            "\u{645}\u{6CC}\u{200C}\u{631}\u{648}\u{645} abc \u{633}\u{644}\u{627}\u{645}",
            "\u{200C}\u{633}\u{200C}",
            "ab \u{663}\u{662} \u{6A9}\u{62A}\u{627}\u{628} (cd) \u{627}\u{644}\u{639}\u{631}\u{628}\u{64A}\u{629}.pdf",
            "\u{644}\u{627}\u{644}\u{627}\u{644}\u{644}\u{627}\u{627}",
            AB_ALI,
            "\u{639}\u{644}\u{64A}\u{633}",
        ] {
            field.text = text.to_owned();
            let out = field.frame(vec![]);
            assert_aligned(&out, text);
        }
    }

    #[test]
    fn a_joiner_takes_no_room_and_the_arrows_pass_it_in_one_step() {
        let Some(mut field) = Field::arabic() else { return };
        field.frame(vec![]);
        // "mi-ravam" with the zero-width non-joiner between the verb's two parts.
        let text = "\u{645}\u{6CC}\u{200C}\u{631}\u{648}\u{645}";
        field.text = text.to_owned();
        let out = field.frame(vec![]);
        let map = bidi::screen_map(text);
        let at = map.drawn.iter().position(|c| *c == '\u{200C}').expect("the joiner is drawn");
        let glyphs = &out.galley.rows[0].glyphs;
        assert_eq!(glyphs.len(), map.screen_len());
        assert_eq!(glyphs[at].advance_width, 0.0);
        assert!((glyphs[at].pos.x - glyphs[at + 1].pos.x).abs() < 0.01, "in front of the next glyph, not at the edge of the one before");
        // Moving left across the whole word, from its start at the right, stops once at each edge of
        // a letter: the joiner's two places are one stop.
        let mut stops = std::collections::BTreeSet::new();
        stops.insert((cursor_of(&field.press(Key::Home)).1 * 10.0).round() as i64);
        let mut presses = 0;
        loop {
            let (v, x) = cursor_of(&field.press(Key::ArrowLeft));
            stops.insert((x * 10.0).round() as i64);
            presses += 1;
            if v == 0 || presses > 20 {
                break;
            }
        }
        assert_eq!(stops.len(), map.screen_len(), "{stops:?}");
        assert_eq!(presses, map.screen_len() - 1, "one press less than there are places, as the joiner's two are one");
    }

    /// The same events on a [`BidiEdit`] and a plain `TextEdit` give the same text, cursor and look.
    #[test]
    fn text_without_right_to_left_script_is_a_plain_text_edit() {
        let run = |reordering: bool| {
            let ctx = egui::Context::default();
            ctx.set_os(egui::os::OperatingSystem::Nix);
            ctx.set_fonts(crate::theme::font_definitions());
            let id = Id::new("plain");
            let mut text = String::from("report 2024");
            let mut seen = Vec::new();
            for events in [
                vec![],
                vec![Event::Text(" final".into())],
                vec![key(Key::ArrowLeft), key(Key::ArrowLeft)],
                vec![key(Key::Backspace)],
                vec![Event::Text("é".into())],
                vec![key(Key::Home)],
            ] {
                let raw = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 100.0))), events, ..Default::default() };
                let frame = ctx.run_ui(raw, |ui| {
                    ui.memory_mut(|m| m.request_focus(id));
                    let out = if reordering {
                        BidiEdit::singleline(&mut text).id(id).desired_width(200.0).show(ui)
                    } else {
                        TextEdit::singleline(&mut text).id(id).desired_width(200.0).show(ui)
                    };
                    seen.push((
                        out.state.cursor.char_range().map(|r| (r.primary.index.0, r.secondary.index.0)),
                        out.galley.job.text.clone(),
                        out.response.response.rect,
                    ));
                });
                frame.drop_without_applying_deltas();
            }
            (text, seen)
        };
        let (plain, via_plain) = run(false);
        let (ours, via_ours) = run(true);
        assert_eq!(ours, plain);
        assert_eq!(via_ours, via_plain);
        assert_eq!(plain, "report 2024 fi\u{e9}al");
    }

    // The buffer, without drawing anything.

    fn buffer_over(text: &mut String) -> BidiBuffer<'_> {
        BidiBuffer {
            text,
            cache: Rc::default(),
            caret: Rc::default(),
            preedit: Rc::default(),
            composing: Vec::new(),
            any_ime: false,
            multiline: false,
        }
    }

    fn typed(buffer: &mut BidiBuffer<'_>, cursor: &mut CCursor, chunks: &[&str]) {
        for chunk in chunks {
            buffer.insert_text_at(cursor, chunk, usize::MAX);
        }
    }

    #[test]
    fn the_buffer_inserts_in_logical_order_and_reports_screen_positions() {
        let mut stored = String::new();
        let mut buffer = buffer_over(&mut stored);
        let mut cursor = CCursor::new(0);
        typed(&mut buffer, &mut cursor, &["a", "b"]);
        assert_eq!((cursor.index.0, buffer.caret.get()), (2, Some(Caret { l: 2, v: 2 })));
        typed(&mut buffer, &mut cursor, &[AIN, LAM, YA]);
        assert_eq!(buffer.as_str(), AB_ALI);
        assert_eq!((cursor.index.0, buffer.caret.get()), (2, Some(Caret { l: 5, v: 2 })));
        // A Latin letter typed after the word lands right of it.
        typed(&mut buffer, &mut cursor, &["X"]);
        assert_eq!((buffer.as_str(), cursor.index.0), ("ab\u{639}\u{644}\u{64A}X", 6));
        // Back to the middle of the word: logical place 4, between lam and ya.
        let map = buffer.map();
        assert_eq!(map.shown, [0, 1, 4, 3, 2, 5]);
        buffer.caret.set(None);
        let mut cursor = CCursor::new(3);
        typed(&mut buffer, &mut cursor, &[AIN]);
        assert_eq!(buffer.as_str(), "ab\u{639}\u{644}\u{639}\u{64A}X", "screen position 3 is between ya and lam: logical place 4");
    }

    #[test]
    fn the_buffer_deletes_the_logical_neighbour_and_selections_by_screen_range() {
        let mut stored = AB_ALI.to_owned();
        let mut buffer = buffer_over(&mut stored);
        // Backspace at logical 5 (screen position 2): the ya, which is drawn right after the "b".
        buffer.caret.set(Some(Caret { l: 5, v: 2 }));
        let after = buffer.delete_previous_char(CCursor::new(2));
        assert_eq!((buffer.as_str(), after.index.0), ("ab\u{639}\u{644}", 2));
        // Delete at logical 2 (screen position 2, at the end of "ab"): the ain.
        buffer.caret.set(Some(Caret { l: 2, v: 2 }));
        let after = buffer.delete_next_char(CCursor::new(2));
        assert_eq!(buffer.as_str(), "ab\u{644}");
        assert_eq!(after.index.0, 2);
        // Nothing before the first character.
        buffer.caret.set(None);
        assert_eq!(buffer.delete_previous_char(CCursor::new(0)).index.0, 0);
        assert_eq!(buffer.as_str(), "ab\u{644}");
        // The right end of the screen is the start of the Arabic run in reading order: Delete
        // there takes the lam, the first character after the caret.
        let after = buffer.delete_next_char(CCursor::new(3));
        assert_eq!((buffer.as_str(), after.index.0), ("ab", 2));
        assert_eq!(buffer.delete_next_char(CCursor::new(2)).index.0, 2, "nothing after the last");
        assert_eq!(buffer.as_str(), "ab");

        // A screen range covers the characters drawn there: "b", ya and lam, not the ain.
        let mut stored = AB_ALI.to_owned();
        let mut buffer = buffer_over(&mut stored);
        let after = buffer.delete_selected_ccursor_range([CCursor::new(1), CCursor::new(4)]);
        assert_eq!((buffer.as_str(), after.index.0), ("a\u{639}", 1));
        // Out-of-range and inverted ranges delete nothing and panic nowhere.
        let _ = buffer.delete_selected_ccursor_range([CCursor::new(9), CCursor::new(99)]);
        let _ = buffer.delete_selected_ccursor_range([CCursor::new(2), CCursor::new(1)]);
        buffer.delete_char_range(CharIndex(5)..CharIndex(1));
        assert_eq!(buffer.as_str(), "a\u{639}");
    }

    #[test]
    fn the_buffer_deletes_words_and_paragraphs_in_reading_order() {
        let mut stored = "abc \u{639}\u{644}\u{64A}".to_owned();
        let mut buffer = buffer_over(&mut stored);
        let end = buffer.map().screen_positions(7)[0];
        buffer.caret.set(Some(Caret { l: 7, v: end }));
        let after = buffer.delete_previous_word(CCursor::new(end));
        assert_eq!(buffer.as_str(), "abc ");
        assert_eq!(after.index.0, 4);

        let mut stored = "abc \u{639}\u{644}\u{64A} def".to_owned();
        let mut buffer = buffer_over(&mut stored);
        buffer.caret.set(None);
        let galley_less = egui::Context::default();
        let _ = galley_less;
        buffer.caret.set(Some(Caret { l: 4, v: 4 }));
        let _ = buffer.delete_next_word(CCursor::new(4));
        assert!(buffer.as_str().starts_with("abc "), "{:?}", buffer.as_str());
        assert!(buffer.as_str().ends_with("def"), "{:?}", buffer.as_str());
        assert!(!buffer.as_str().contains('\u{639}'), "{:?}", buffer.as_str());
    }

    #[test]
    fn the_buffer_honours_the_character_limit_and_odd_input() {
        let mut stored = String::new();
        let mut buffer = buffer_over(&mut stored);
        let mut cursor = CCursor::new(0);
        for chunk in ["ab", AIN, LAM, YA] {
            buffer.insert_text_at(&mut cursor, chunk, 4);
        }
        assert_eq!(buffer.as_str(), "ab\u{639}\u{644}", "four characters at most, counted in the stored text");
        buffer.insert_text_at(&mut cursor, "", usize::MAX);
        buffer.insert_text_at(&mut cursor, "x", 0);
        assert_eq!(buffer.as_str(), "ab\u{639}\u{644}");
        // Indices past the end are clamped.
        let mut far = CCursor::new(999);
        buffer.insert_text_at(&mut far, "z", usize::MAX);
        assert!(far.index.0 <= buffer.map().screen_len());
        assert!(buffer.as_str().contains('z'));
        buffer.replace_with("\u{202B}\u{64B}\n");
        assert_eq!(buffer.caret.get(), None);
        let _ = buffer.delete_previous_char(CCursor::new(99));
        let _ = buffer.delete_previous_word(CCursor::new(99));
        let _ = buffer.delete_next_word(CCursor::new(99));
        let _ = buffer.delete_surrounding_chars(CCursorRange::one(CCursor::new(99)), 5, 5);
        buffer.clear();
        assert_eq!(buffer.as_str(), "");
        let mut cursor = CCursor::new(0);
        buffer.insert_text_at(&mut cursor, "\u{639}\u{644}", usize::MAX);
        assert_eq!(buffer.take(), "\u{639}\u{644}");
    }

    #[test]
    fn the_starting_cursor_follows_where_it_was_kept() {
        let map = bidi::screen_map(AB_ALI);
        let at = |v: usize| CCursor::new(v);
        // Nothing kept: the end (or the start) of the reading order.
        let (cursor, caret) = initial_cursor(None, None, &map, true);
        assert_eq!((cursor.primary.index.0, caret), (2, Caret { l: 5, v: 2 }));
        let (cursor, caret) = initial_cursor(None, None, &map, false);
        assert_eq!((cursor.primary.index.0, caret), (0, Caret { l: 0, v: 0 }));
        // Kept by a plain field: logical indices become screen positions.
        let (cursor, caret) = initial_cursor(Some(CCursorRange::two(at(0), at(5))), None, &map, true);
        assert_eq!((cursor.secondary.index.0, cursor.primary.index.0, caret), (0, 2, Caret { l: 5, v: 2 }));
        // Kept by this field: screen positions, past the end clamped (a select-all from outside).
        let known = Some(Caret { l: 5, v: 2 });
        let (cursor, caret) = initial_cursor(Some(CCursorRange::two(at(0), at(99))), known, &map, true);
        assert_eq!((cursor.primary.index.0, caret), (5, Caret { l: logical_for(&map, 5, known), v: 5 }));
        // The remembered place is only used where it is still drawn.
        let stale = Some(Caret { l: 9, v: 2 });
        assert_eq!(logical_for(&map, 2, stale), first(&map.logical_positions(2)));
        assert_eq!(logical_for(&map, 2, Some(Caret { l: 5, v: 2 })), 5);
        assert_eq!(logical_for(&map, 2, Some(Caret { l: 2, v: 2 })), 2);
    }

    #[test]
    fn movements_pick_the_logical_place_on_their_side() {
        let map = bidi::screen_map(AB_ALI);
        // Screen position 2: the end of "b" on its left, the end of the text on its right.
        assert_eq!(derive_logical(&map, 2, Moved::Left), 5, "arrived from the right, over the newest letter");
        assert_eq!(derive_logical(&map, 2, Moved::Right), 2, "arrived from the left, over the b");
        assert_eq!(derive_logical(&map, 2, Moved::External), 2);
        assert_eq!(derive_logical(&map, 2, Moved::Pointer { pointer_x: 10.0, boundary_x: 12.0 }), 2);
        assert_eq!(derive_logical(&map, 2, Moved::Pointer { pointer_x: 14.0, boundary_x: 12.0 }), 5);
        // At the ends there is one place.
        for how in [Moved::Left, Moved::Right, Moved::External] {
            assert_eq!(derive_logical(&map, 0, how), 0);
            assert_eq!(derive_logical(&map, 5, how), 2);
        }
        assert_eq!(derive_logical(&map, 99, Moved::External), 2);
    }

    #[test]
    fn copy_cut_and_home_end_are_taken_over() {
        let map = bidi::screen_map(AB_ALI);
        // Screen positions 1..4 hold "b", ya, lam: copied in reading order.
        let cursor = CCursorRange::two(CCursor::new(4), CCursor::new(1));
        assert_eq!(selected_text(AB_ALI, &map, &cursor), "b\u{644}\u{64A}");
        assert_eq!(selected_text(AB_ALI, &map, &CCursorRange::one(CCursor::new(3))), "");

        let mut events = vec![Event::Copy, Event::Text("x".into()), Event::Cut, key(Key::Home)];
        let taken = take_over_events(&mut events, true, HomeEnd::Off, 5);
        assert!(taken.copied && taken.motion.is_none());
        assert_eq!(events.len(), 3, "the Copy went, the Cut became a Backspace, the Home stays with egui: {events:?}");
        assert!(matches!(events.get(1), Some(Event::Key { key: Key::Backspace, pressed: true, .. })));

        // Without a selection they are egui's.
        let mut events = vec![Event::Copy, Event::Cut];
        assert!(!take_over_events(&mut events, false, HomeEnd::Off, 5).copied);
        assert_eq!(events.len(), 2);

        // Text that starts right to left: Home is the start in reading order, End the end.
        let mut events = vec![
            key(Key::End),
            Event::Key { key: Key::Home, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::SHIFT },
            key(Key::ArrowLeft),
        ];
        let taken = take_over_events(&mut events, false, HomeEnd::Line, 5);
        assert_eq!(taken.motion, Some((0, true)), "the last one decides");
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn keys_say_which_way_the_caret_moved() {
        assert!(matches!(key_motion(&[key(Key::ArrowLeft)]), Some(Moved::Left)));
        assert!(matches!(key_motion(&[key(Key::ArrowLeft), key(Key::End)]), Some(Moved::Right)));
        assert!(key_motion(&[Event::Text("a".into())]).is_none());
        assert!(matches!(
            key_motion(&[Event::Key { key: Key::A, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND }]),
            Some(Moved::Right)
        ));
    }

    #[test]
    fn copy_gives_the_selected_characters_in_logical_order_and_cut_removes_them() {
        let Some((mut field, _)) = Field::arabic_with(AB_ALI) else { return };
        // Screen positions 0..4 hold a, b, ya and lam; the ain is left out.
        field.select(0, 4);
        field.frame(vec![Event::Copy]);
        assert_eq!(field.copied, ["ab\u{644}\u{64A}"], "in reading order, not as drawn");
        assert_eq!(field.text, AB_ALI);
        field.frame(vec![Event::Cut]);
        assert_eq!(field.copied.len(), 2);
        assert_eq!(field.copied[1], "ab\u{644}\u{64A}");
        assert_eq!(field.text, "\u{639}", "only the ain is left");
        // Nothing selected: no copy.
        field.frame(vec![Event::Copy]);
        assert_eq!(field.copied.len(), 2);
    }

    #[test]
    fn paste_inserts_logical_text_at_the_caret_and_flattens_lines() {
        let Some((mut field, _)) = Field::arabic_with("ab") else { return };
        field.frame(vec![Event::Paste("\u{639}\u{644}\r\n\u{64A}".to_owned())]);
        assert_eq!(field.text, "ab\u{639}\u{644}  \u{64A}", "each line break becomes a space");
        assert_eq!(field.caret().map(|c| c.l), Some(7));
        // Select the whole text and paste over it.
        field.press_with(Key::A, Modifiers::COMMAND);
        field.frame(vec![Event::Paste("\u{628}".to_owned())]);
        assert_eq!(field.text, "\u{628}");
    }

    #[test]
    fn undo_and_redo_restore_the_text_and_keep_the_caret_usable() {
        let Some((mut field, _)) = Field::arabic_with("ab") else { return };
        field.idle();
        for letter in [AIN, LAM, YA] {
            field.type_text(letter);
            field.idle();
        }
        assert_eq!(field.text, AB_ALI);
        field.press_with(Key::Z, Modifiers::COMMAND);
        assert_eq!(field.text, "ab\u{639}\u{644}");
        field.press_with(Key::Z, Modifiers::COMMAND);
        assert_eq!(field.text, "ab\u{639}");
        field.press_with(Key::Y, Modifiers::COMMAND);
        assert_eq!(field.text, "ab\u{639}\u{644}");
        field.press_with(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
        assert_eq!(field.text, AB_ALI, "Ctrl+Shift+Z is redo too");
        // The caret is where the undone text had it.
        let out = field.press_with(Key::Z, Modifiers::COMMAND);
        assert_eq!(field.text, "ab\u{639}\u{644}");
        assert_eq!(cursor_of(&out).0, 2);
        field.type_text(LAM);
        assert_eq!(field.text.chars().count(), 5, "typing goes on after an undo");
    }

    #[test]
    fn ctrl_backspace_and_delete_remove_words_in_reading_order() {
        let Some((mut field, _)) = Field::arabic_with("abc \u{639}\u{644}\u{64A} def") else { return };
        // Back over "def" and the space, then over the Arabic word.
        field.press_with(Key::Backspace, Modifiers::CTRL);
        assert_eq!(field.text, "abc \u{639}\u{644}\u{64A} ");
        field.press_with(Key::Backspace, Modifiers::CTRL);
        assert_eq!(field.text, "abc ");
        // Delete forward from the start.
        field.press(Key::Home);
        field.press_with(Key::Delete, Modifiers::CTRL);
        assert_eq!(field.text, " ", "a word at a time: its space is left");
        let Some((mut field, _)) = Field::arabic_with("\u{639}\u{644}\u{64A} \u{628}\u{628}") else { return };
        field.press(Key::Home);
        field.press_with(Key::Delete, Modifiers::CTRL);
        assert_eq!(field.text, " \u{628}\u{628}", "the first word in reading order goes first");
    }

    #[test]
    fn the_character_limit_counts_stored_characters() {
        let Some(mut field) = Field::arabic() else { return };
        field.char_limit = Some(4);
        field.frame(vec![]);
        field.type_text("ab");
        field.type_text(AIN);
        field.type_text(LAM);
        field.type_text(YA);
        assert_eq!(field.text, "ab\u{639}\u{644}");
        field.frame(vec![Event::Paste("\u{628}\u{628}\u{628}".to_owned())]);
        assert_eq!(field.text, "ab\u{639}\u{644}");
        // A selection frees room.
        field.press_with(Key::A, Modifiers::COMMAND);
        field.frame(vec![Event::Paste("\u{628}\u{628}\u{628}\u{628}\u{628}\u{628}".to_owned())]);
        assert_eq!(field.text.chars().count(), 4);
    }

    #[test]
    fn an_input_method_composition_is_replaced_not_repeated() {
        let Some((mut field, _)) = Field::arabic_with("ab") else { return };
        let preedit = |text: &str| Event::Ime(egui::ImeEvent::Preedit { text: text.to_owned(), active_range_chars: None });
        field.frame(vec![preedit("\u{639}\u{644}")]);
        assert_eq!(field.text, "ab\u{639}\u{644}");
        field.frame(vec![preedit("\u{639}\u{644}\u{64A}")]);
        assert_eq!(field.text, AB_ALI, "the first composition is replaced");
        field.frame(vec![]);
        field.frame(vec![preedit("\u{639}")]);
        assert_eq!(field.text, "ab\u{639}");
        field.frame(vec![preedit("")]);
        assert_eq!(field.text, "ab", "dismissed");
        field.frame(vec![preedit("\u{628}\u{628}")]);
        field.frame(vec![Event::Ime(egui::ImeEvent::Commit("\u{628}\u{628}\u{628}".to_owned()))]);
        assert_eq!(field.text, "ab\u{628}\u{628}\u{628}");
        // Latin before the Arabic, typed after a commit.
        field.type_text("z");
        assert!(field.text.ends_with('z') || field.text.contains('z'));
    }

    #[test]
    fn home_and_end_go_to_the_ends_of_reading_for_text_that_starts_right_to_left() {
        // Starts left to right: the screen edges, as in any field.
        let Some((mut field, _)) = Field::arabic_with(AB_ALI) else { return };
        let out = field.press(Key::Home);
        assert_eq!(cursor_of(&out).0, 0);
        let out = field.press(Key::End);
        assert_eq!(cursor_of(&out).0, 5);

        // Starts right to left: Home is the start of the text, at the right; End its end, at the left.
        let word = "\u{639}\u{644}\u{64A} ab";
        let Some((mut field, _)) = Field::arabic_with(word) else { return };
        let out = field.press(Key::Home);
        let map = bidi::screen_map(word);
        assert_eq!(field.caret().map(|c| c.l), Some(0));
        assert_eq!(cursor_of(&out).0, first(&map.screen_positions(0)));
        assert_eq!(cursor_of(&out).0, 3, "the right edge of the Arabic word, which is on the left here");
        let out = field.press(Key::End);
        assert_eq!(field.caret().map(|c| c.l), Some(6));
        assert_eq!(cursor_of(&out).0, first(&map.screen_positions(6)));
        // Shift keeps the other end of the selection.
        field.press(Key::Home);
        let out = field.press_with(Key::End, Modifiers::SHIFT);
        let range = out.state.cursor.char_range().expect("a selection");
        assert_ne!(range.primary, range.secondary);
    }

    #[test]
    fn arrows_never_stop_twice_on_one_spot() {
        let Some(mut field) = Field::arabic() else { return };
        // Lam-alef is one glyph for two letters in most Arabic faces; the arrows must not need two presses.
        for text in ["\u{627}\u{644}\u{627}\u{633}\u{645}", "x \u{644}\u{627} y", "\u{633}\u{644}\u{627}\u{645} \u{628}\u{628}"] {
            field.text = text.to_owned();
            field.press(Key::Home);
            field.text = text.to_owned();
            let out = field.press(Key::Home);
            let n = bidi::screen_map(text).screen_len();
            let mut xs = vec![cursor_of(&out).1];
            for _ in 0..n {
                let out = field.press(Key::ArrowRight);
                xs.push(cursor_of(&out).1);
            }
            assert!(xs.windows(2).all(|w| w[1] > w[0] - 0.01), "{text:?}: {xs:?}");
            let stationary = xs.windows(2).filter(|w| (w[1] - w[0]).abs() < 0.01).count();
            // Only at the end (pressing Right there) may the caret stay.
            let at_end = xs.iter().rev().take_while(|x| (**x - xs[xs.len() - 1]).abs() < 0.01).count().saturating_sub(1);
            assert_eq!(stationary, at_end, "{text:?}: {xs:?}");
        }
    }

    #[test]
    fn a_selection_set_from_outside_is_clamped_to_what_is_drawn() {
        let Some(mut field) = Field::arabic() else { return };
        // Direction controls are not drawn: the text has five characters and three on screen.
        field.text = "\u{202B}\u{639}\u{644}\u{64A}\u{202C}".to_owned();
        field.frame(vec![]);
        let mut state = TextEditState::load(&field.ctx, field.id).unwrap_or_default();
        state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(field.text.chars().count()))));
        state.store(&field.ctx, field.id);
        let out = field.frame(vec![]);
        let range = out.state.cursor.char_range().expect("still selected");
        assert_eq!(range.as_sorted_char_range(), CharIndex(0)..CharIndex(3), "the whole line");
        field.frame(vec![Event::Copy]);
        assert_eq!(field.copied, ["\u{639}\u{644}\u{64A}"]);
        // A caret set past the end is the end, and typing works.
        let mut state = TextEditState::load(&field.ctx, field.id).unwrap_or_default();
        state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(99))));
        state.store(&field.ctx, field.id);
        field.type_text("\u{628}");
        assert!(field.text.contains('\u{628}'));
    }

    #[test]
    fn a_field_that_is_not_focused_keeps_its_text_and_draws_it_in_order() {
        let ctx = egui::Context::default();
        let Some(fonts) = bidi::arabic_test_fonts() else { return };
        ctx.set_fonts(fonts);
        let mut text = AB_ALI.to_owned();
        let mut shown = String::new();
        let frame = ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 100.0))), ..Default::default() }, |ui| {
            let out = BidiEdit::singleline(&mut text).desired_width(200.0).hint_text("hint").show(ui);
            shown = out.galley.job.text.clone();
        });
        frame.drop_without_applying_deltas();
        assert_eq!(text, AB_ALI);
        assert_eq!(shown, bidi::visual(AB_ALI));
        // An empty field shows its hint and not the drawing of the text.
        let mut empty = String::new();
        let frame = ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 100.0))), ..Default::default() }, |ui| {
            BidiEdit::singleline(&mut empty).desired_width(200.0).hint_text("hint").show(ui);
        });
        frame.drop_without_applying_deltas();
        assert!(empty.is_empty());
    }

    /// Stops at the given x on one line.
    fn on_one_line(xs: &[f32]) -> Vec<Pos2> {
        xs.iter().map(|x| Pos2::new(*x, 0.0)).collect()
    }

    #[test]
    fn stepping_skips_places_drawn_at_the_same_x() {
        let stops = on_one_line(&[0.0, 5.0, 10.0, 10.0, 10.0, 16.0]);
        assert_eq!(step(&stops, 1, true), 2);
        assert_eq!(step(&stops, 2, true), 5, "the two places inside the ligature are skipped");
        assert_eq!(step(&stops, 5, false), 4);
        assert_eq!(step(&stops, 4, false), 1);
        assert_eq!(step(&stops, 5, true), 5);
        assert_eq!(step(&stops, 0, false), 0);
        assert_eq!(step(&[], 0, true), 0);
        assert_eq!(step(&on_one_line(&[0.0]), 7, true), 0);
        // Trailing zero-width places: the end.
        assert_eq!(step(&on_one_line(&[0.0, 4.0, 4.0, 4.0]), 1, true), 3);
    }

    #[test]
    fn plain_arrows_are_taken_and_collapse_selections_like_egui() {
        let stops = on_one_line(&[0.0, 5.0, 10.0, 10.0, 16.0]);
        let mut events = vec![key(Key::ArrowRight), Event::Text("x".into()), key(Key::ArrowRight)];
        let mut cursor = CCursorRange::one(CCursor::new(0));
        let moved = take_arrows(&mut events, &mut cursor, &stops);
        assert!(matches!(moved, Some(Moved::Right)));
        assert_eq!((cursor.primary.index.0, events.len()), (2, 1), "two steps (the second jumps the ligature), the text stays");
        // A selection collapses to the end the arrow points to.
        let mut events = vec![key(Key::ArrowLeft)];
        let mut cursor = CCursorRange::two(CCursor::new(4), CCursor::new(1));
        take_arrows(&mut events, &mut cursor, &stops);
        assert_eq!((cursor.primary.index.0, cursor.secondary.index.0), (1, 1));
        // Shift extends and keeps the anchor; word jumps are left to egui.
        let mut events = vec![
            Event::Key { key: Key::ArrowRight, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::SHIFT },
            Event::Key { key: Key::ArrowRight, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::CTRL },
        ];
        let mut cursor = CCursorRange::one(CCursor::new(1));
        take_arrows(&mut events, &mut cursor, &stops);
        assert_eq!((cursor.primary.index.0, cursor.secondary.index.0, events.len()), (2, 1, 1));
        // Nothing to step on: the caret stays.
        let mut cursor = CCursorRange::one(CCursor::new(9));
        take_arrows(&mut vec![key(Key::ArrowRight)], &mut cursor, &stops);
        assert!(cursor.primary.index.0 <= 4);
    }

    /// A small deterministic generator for the tests below.
    struct Dice(u64);

    impl Dice {
        fn next(&mut self, below: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            usize::try_from(self.0 % below.max(1) as u64).unwrap_or(0)
        }

        fn text(&mut self, longest: usize) -> String {
            const ALPHABET: [&str; 14] =
                ["a", "b", " ", "1", "\u{639}", "\u{644}", "\u{64A}", "\u{627}", "\u{64B}", "\u{200C}", "\u{202B}", "(", "\u{5D0}", "\n"];
            (0..self.next(longest + 1)).map(|_| ALPHABET[self.next(ALPHABET.len())]).collect()
        }
    }

    #[test]
    fn random_buffer_operations_keep_the_text_and_the_caret_valid() {
        let mut dice = Dice(0x2545_F491_4F6C_DD1D);
        for round in 0..300 {
            let mut stored = dice.text(12);
            let mut buffer = buffer_over(&mut stored);
            for step in 0..12 {
                let map = buffer.map();
                let n = map.screen_len();
                let at = CCursor::new(dice.next(n + 3));
                let other = CCursor::new(dice.next(n + 3));
                let returned = match dice.next(10) {
                    0 => {
                        let mut cursor = at;
                        buffer.insert_text_at(&mut cursor, &dice.text(4), if dice.next(3) == 0 { dice.next(10) } else { usize::MAX });
                        cursor
                    }
                    1 => buffer.delete_previous_char(at),
                    2 => buffer.delete_next_char(at),
                    3 => buffer.delete_selected_ccursor_range([at, other]),
                    4 => buffer.delete_previous_word(at),
                    5 => buffer.delete_next_word(at),
                    6 => buffer.delete_surrounding_chars(CCursorRange::two(at, other), dice.next(4), dice.next(4)).primary,
                    7 => {
                        buffer.replace_with(&dice.text(8));
                        CCursor::new(0)
                    }
                    8 => {
                        buffer.delete_char_range(CharIndex(at.index.0)..CharIndex(other.index.0));
                        CCursor::new(0)
                    }
                    _ => {
                        buffer.insert_text(&dice.text(3), CharIndex(at.index.0));
                        CCursor::new(0)
                    }
                };
                let map = buffer.map();
                assert!(
                    returned.index.0 <= map.screen_len() || buffer.caret.get().is_none(),
                    "round {round} step {step}: {:?} past {} in {:?}",
                    returned,
                    map.screen_len(),
                    buffer.as_str()
                );
                if let Some(caret) = buffer.caret.get() {
                    assert!(
                        caret.l <= map.logical_len() && caret.v <= map.screen_len(),
                        "round {round} step {step}: {caret:?} in {:?}",
                        buffer.as_str()
                    );
                    assert!(
                        map.screen_positions(caret.l).contains(&caret.v),
                        "round {round} step {step}: {caret:?} is not a place of its own in {:?}",
                        buffer.as_str()
                    );
                }
                assert_eq!(map.shaper, bidi::visual(buffer.as_str()));
            }
        }
    }

    #[test]
    fn random_keystrokes_never_panic_and_leave_one_glyph_per_character() {
        for seed in
            [0x9E37_79B9_7F4A_7C15, 0x1234_5678_9ABC_DEF1, 0xDEAD_BEEF_0BAD_F00D, 0x0F0F_1234_5555_AAAA, 0x7FFF_FFFF_FFFF_FFF1, 0x0123_4567_89AB_CDEF]
        {
            let Some(mut field) = Field::arabic() else { return };
            if seed % 3 == 0 {
                field.char_limit = Some(12);
            }
            field.frame(vec![]);
            let mut dice = Dice(seed);
            let keys = [Key::Backspace, Key::Delete, Key::ArrowLeft, Key::ArrowRight, Key::Home, Key::End, Key::ArrowLeft, Key::ArrowRight];
            let modifiers = [Modifiers::NONE, Modifiers::NONE, Modifiers::SHIFT, Modifiers::CTRL, Modifiers::COMMAND | Modifiers::SHIFT];
            for step in 0..1200 {
                let mut events = Vec::new();
                for _ in 0..=dice.next(2) {
                    events.push(match dice.next(12) {
                        0..=3 => Event::Text(dice.text(3)),
                        4..=7 => {
                            let (key, modifiers) = (keys[dice.next(keys.len())], modifiers[dice.next(modifiers.len())]);
                            Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }
                        }
                        8 => Event::Copy,
                        9 => Event::Cut,
                        10 => Event::Paste(dice.text(5)),
                        _ => {
                            let key = [Key::A, Key::Z, Key::Y][dice.next(3)];
                            Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: Modifiers::COMMAND }
                        }
                    });
                }
                if dice.next(25) == 0 {
                    field.time += 2.0;
                }
                let out = field.frame(events);
                let map = bidi::screen_map(&field.text);
                let glyphs = out.galley.rows.first().map_or(0, |row| row.glyphs.len());
                assert_eq!(glyphs, map.screen_len(), "step {step}: one glyph per character in {:?}", field.text);
                assert_aligned(&out, &field.text);
                if let Some(range) = out.state.cursor.char_range() {
                    assert!(range.primary.index.0 <= glyphs && range.secondary.index.0 <= glyphs, "step {step}: {range:?} past {glyphs}");
                }
                if let Some(caret) = field.caret() {
                    assert!(caret.l <= map.logical_len() && caret.v <= map.screen_len(), "step {step}: {caret:?}");
                    assert!(map.screen_positions(caret.l).contains(&caret.v), "step {step}: {caret:?} in {:?}", field.text);
                }
            }
        }
    }

    // Multi-line fields.

    /// An Arabic paragraph that wraps in a narrow field.
    const PARAGRAPH: &str = "هذا نص عربي طويل يكتب في حقل ضيق ليلتف على عدة أسطر";
    /// A Latin paragraph that wraps in a narrow field into two rows.
    const LATIN: &str = "alpha beta gamma delta epsilon zeta";

    fn row_text(out: &TextEditOutput, row: usize) -> String {
        out.galley.rows[row].glyphs.iter().map(|g| g.chr).collect()
    }

    /// Where row `row` of the galley is drawn: its left and its right edge, in screen x.
    fn row_span(out: &TextEditOutput, row: usize) -> (f32, f32) {
        let placed = &out.galley.rows[row];
        let left = out.galley_pos.x - out.galley.rect.left() + placed.pos.x;
        (left, left + placed.row.size.x)
    }

    /// The galley index of the first character of row `row`.
    fn row_start(out: &TextEditOutput, row: usize) -> usize {
        out.galley.rows[..row].iter().map(|r| r.char_count_including_newline().0).sum()
    }

    /// What the field needs of the galley of wrapped `map`: the text egui laid out is the map's,
    /// every row has one glyph per character drawn on it, and in each x never goes back and the
    /// glyphs' vertices follow each other.
    fn assert_rows_aligned(out: &TextEditOutput, map: &ScreenMap, text: &str) {
        assert_eq!(out.galley.job.text, map.shaper, "{text:?}: the galley is laid out from the map's text");
        let drawn: Vec<&[char]> = map.drawn.split(|c| *c == '\n').collect();
        assert_eq!(out.galley.rows.len(), drawn.len(), "{text:?}: a row to every line");
        assert_eq!(out.galley.end().index.0, map.screen_len(), "{text:?}: the galley and the map count alike");
        for (i, (placed, chars)) in out.galley.rows.iter().zip(&drawn).enumerate() {
            let glyphs = &placed.row.glyphs;
            assert_eq!(glyphs.len(), chars.len(), "{text:?}: row {i} has one glyph per character");
            if chars.iter().any(|c| bidi::is_shaped_rtl(*c)) {
                for (k, (glyph, c)) in glyphs.iter().zip(*chars).enumerate() {
                    assert_eq!(glyph.chr, *c, "{text:?}: row {i} glyph {k}");
                }
            }
            assert!(glyphs.windows(2).all(|w| w[1].pos.x >= w[0].pos.x), "{text:?}: row {i}: x never goes back");
            assert!(glyphs.windows(2).all(|w| w[1].first_vertex >= w[0].first_vertex), "{text:?}: row {i}: vertices follow each other");
            let vertices = &placed.row.visuals.glyph_vertex_range;
            assert!(glyphs.iter().all(|g| (g.first_vertex as usize) <= vertices.end), "{text:?}: row {i}: vertices stay in the row");
        }
    }

    #[test]
    fn a_long_arabic_paragraph_wraps_into_rows_that_read_from_the_right() {
        let Some((field, out)) = Field::multiline_with(150.0, PARAGRAPH) else { return };
        let rows = out.galley.rows.len();
        assert!(rows >= 3, "{rows} rows");
        // The first row holds the first words: reading starts at its right end, with the first
        // letter of the first word (heh).
        assert_eq!(out.galley.rows[0].glyphs.last().map(|g| g.chr), Some('\u{647}'), "row 0 is {:?}", row_text(&out, 0));
        // Every row is against the right edge of the text area, the paragraph being right to left.
        let area = out.text_clip_rect.right();
        for row in 0..rows {
            let (_, right) = row_span(&out, row);
            assert!(near(right, area), "row {row} ends at {right}, the text area at {area}");
        }
        // The caret starts at the end of the reading order, which is the left end of the last row.
        let last = rows - 1;
        let (v, x) = cursor_of(&out);
        assert_eq!(v, row_start(&out, last), "at the start of the last row's screen order");
        assert!(near(x, row_span(&out, last).0), "caret at {x}, the last row begins at {}", row_span(&out, last).0);
        assert_rows_aligned(&out, &field.map().expect("reordered"), PARAGRAPH);
        // The text is as it was: wrapping adds nothing to it.
        assert_eq!(field.text, PARAGRAPH);
    }

    #[test]
    fn a_latin_paragraph_wraps_as_it_does_in_a_plain_text_edit_and_stays_at_the_left() {
        let both = format!("{PARAGRAPH}\n{LATIN}");
        let Some((field, ours)) = Field::multiline_with(150.0, &both) else { return };
        let mut plain = Field::arabic().expect("the fonts are there");
        (plain.multiline, plain.plain, plain.width, plain.text) = (true, true, 150.0, LATIN.to_owned());
        let wanted = plain.frame(vec![]);
        assert!(wanted.galley.rows.len() >= 2);
        let rows = ours.galley.rows.len();
        let latin = wanted.galley.rows.len();
        for k in 0..latin {
            assert_eq!(row_text(&ours, rows - latin + k), row_text(&wanted, k), "row {k} of the Latin paragraph");
            assert_eq!(ours.galley.rows[rows - latin + k].pos.x, 0.0, "left to right rows begin at the left edge");
        }
        // The Arabic paragraph above it is against the other edge: also its last row, which is short.
        let last_arabic = rows - latin - 1;
        assert!(ours.galley.rows[last_arabic].ends_with_newline);
        assert!(row_span(&ours, last_arabic).0 > ours.text_clip_rect.left() + 20.0, "a short row: {:?}", row_span(&ours, last_arabic));
        assert!(near(row_span(&ours, last_arabic).1, ours.text_clip_rect.right()), "{:?}", row_span(&ours, last_arabic));
        assert_eq!(ours.galley.rows[0].glyphs.last().map(|g| g.chr), Some('\u{647}'), "the first row begins with the first word's first letter");
        assert_rows_aligned(&ours, &field.map().expect("reordered"), &both);
    }

    /// The logical index just after the last character of the first row of the field's map.
    fn first_break(field: &Field) -> usize {
        let map = field.map().expect("reordered");
        map.shown.iter().position(|l| *l == bidi::VIRTUAL).expect("the text wraps")
    }

    #[test]
    fn typing_at_a_wrap_boundary_keeps_the_stored_text_logical() {
        let Some(mut field) = Field::arabic() else { return };
        (field.multiline, field.width) = (true, 150.0);
        field.frame(vec![]);
        // Typed word by word, the paragraph wraps as it grows; the stored text is what was typed.
        let mut typed = String::new();
        for word in PARAGRAPH.split_inclusive(' ') {
            field.type_text(word);
            typed.push_str(word);
            assert_eq!(field.text, typed);
        }
        let out = field.frame(vec![]);
        assert!(out.galley.rows.len() >= 3);
        assert!(!field.text.contains('\n'), "a soft wrap is not a newline of the text");
        let map = field.map().expect("reordered");
        assert_eq!(
            field.caret(),
            Some(Caret { l: typed.chars().count(), v: map.screen_len() - out.galley.rows.last().map_or(0, |r| r.glyphs.len()) })
        );
        // The caret is at the end of the reading order: the left end of the last row.
        let last = out.galley.rows.len() - 1;
        assert!(near(cursor_of(&out).1, row_span(&out, last).0));

        // Home goes to the left end of the row the caret is on: the end of that row in reading
        // order. Down from the start of the text lands on the second row.
        field.press_with(Key::Home, Modifiers::CTRL);
        assert_eq!(field.caret().map(|c| c.l), Some(0), "Ctrl+Home is the start of the text in reading order");
        let out = field.press(Key::ArrowDown);
        assert_eq!(out.state.cursor.char_range().map(|r| r.primary.index.0 >= row_start(&out, 1)), Some(true), "on the second row");
        let out = field.press(Key::Home);
        assert_eq!(out.state.cursor.char_range().map(|r| r.primary.index.0), Some(row_start(&out, 1)));
        let map = field.map().expect("reordered");
        assert_eq!(field.caret().map(|c| c.l), map.logical_positions(row_start(&out, 1)).last().copied());
        // Row 0's left end is where row 0 ends, and a letter typed there is stored between the
        // first row's text and the second's.
        let b0 = first_break(&field);
        field.press_with(Key::Home, Modifiers::CTRL);
        field.press(Key::Home);
        assert_eq!(field.caret().map(|c| c.l), Some(b0));
        field.type_text("\u{635}");
        let wanted: String = typed.chars().take(b0).chain(std::iter::once('\u{635}')).chain(typed.chars().skip(b0)).collect();
        assert_eq!(field.text, wanted);
    }

    #[test]
    fn backspace_and_delete_across_a_wrap_remove_the_right_logical_character() {
        // Arabic: the left end of row 0 is the wrap in reading order.
        let Some((mut field, _)) = Field::multiline_with(150.0, PARAGRAPH) else { return };
        let b0 = first_break(&field);
        let chars: Vec<char> = PARAGRAPH.chars().collect();
        assert_eq!(chars[b0 - 1], ' ', "rows end with the space after their last word");
        field.press_with(Key::Home, Modifiers::CTRL);
        field.press(Key::Home);
        assert_eq!(field.caret().map(|c| c.l), Some(b0));
        field.press(Key::Backspace);
        let without_space: String = chars.iter().take(b0 - 1).chain(chars.iter().skip(b0)).collect();
        assert_eq!(field.text, without_space, "Backspace removed the space before the break, nothing else");
        // Delete at the same place (now between the words, which have rejoined) removes the first
        // letter of the next word; the caret is still between the two words.
        assert_eq!(field.caret().map(|c| c.l), Some(b0 - 1));
        field.press(Key::Delete);
        let without_letter: String = without_space.chars().take(b0 - 1).chain(without_space.chars().skip(b0)).collect();
        assert_eq!(field.text, without_letter);

        // Latin: the start of the second row and the end of the first are the same place.
        let both = format!("{PARAGRAPH}\n{LATIN}");
        let wrapped = |both: &str| Field::multiline_with(150.0, both).map(|(field, _)| field);
        let Some(mut field) = wrapped(&both) else { return };
        // The wrap of the Latin paragraph is the last virtual line break; it is after a space.
        let map = field.map().expect("reordered");
        let at = map.shown.iter().rposition(|l| *l == bidi::VIRTUAL).expect("the Latin paragraph wraps");
        let b = map.shown[at + 1];
        assert_eq!(map.shown[at - 1] + 1, b, "the break is between two neighbours of the text");
        let chars: Vec<char> = both.chars().collect();
        assert_eq!(chars[b - 1], ' ');
        // The left end of the last row: where it starts.
        field.press_with(Key::End, Modifiers::CTRL);
        field.press(Key::Home);
        assert_eq!(field.caret().map(|c| c.l), Some(b));
        field.press(Key::Backspace);
        let mut expected = chars.clone();
        expected.remove(b - 1);
        assert_eq!(field.text, expected.iter().collect::<String>(), "the space that ended the row above went");

        // The end of the first Latin row, after its space, is the same place; Delete takes the
        // letter that starts the next row.
        let Some(mut field) = wrapped(&both) else { return };
        field.press_with(Key::End, Modifiers::CTRL);
        field.press(Key::ArrowUp);
        field.press(Key::End);
        assert_eq!(field.caret().map(|c| c.l), Some(b));
        field.press(Key::Delete);
        let mut expected = chars;
        expected.remove(b);
        assert_eq!(field.text, expected.iter().collect::<String>());
    }

    #[test]
    fn copy_and_cut_across_wrapped_rows_use_the_logical_text() {
        let Some((mut field, _)) = Field::multiline_with(150.0, PARAGRAPH) else { return };
        field.press_with(Key::A, Modifiers::COMMAND);
        field.frame(vec![Event::Copy]);
        assert_eq!(field.copied.last().map(String::as_str), Some(PARAGRAPH), "no line break of the wrapping is copied");
        // A selection from the second row's start to the end of the paragraph.
        field.press_with(Key::Home, Modifiers::CTRL);
        for _ in 0..8 {
            field.press_with(Key::ArrowLeft, Modifiers::SHIFT);
        }
        field.frame(vec![Event::Cut]);
        let cut = field.copied.last().cloned().expect("copied");
        assert_eq!(cut.chars().count(), 8);
        assert_eq!(format!("{cut}{}", field.text), PARAGRAPH, "what was cut and what is left are the paragraph, in order");
        // Pasted back at the start of the text, it is stored in order as well.
        field.press_with(Key::Home, Modifiers::CTRL);
        field.frame(vec![Event::Paste(cut.clone())]);
        assert_eq!(field.text, PARAGRAPH);
        field.frame(vec![Event::Paste("a\nb\r\nc".to_owned())]);
        assert!(field.text.contains("a\nb\nc") || field.text.contains("a\nb\r\nc"), "{:?}", field.text);
    }

    #[test]
    fn a_new_paragraph_follows_the_direction_of_what_is_typed_in_it() {
        let Some((mut field, _)) = Field::multiline_with(150.0, PARAGRAPH) else { return };
        let out = field.press(Key::Enter);
        assert_eq!(field.text, format!("{PARAGRAPH}\n"));
        // The empty line after the break takes the paragraph's direction: its caret is at the right.
        let last = out.galley.rows.len() - 1;
        assert!(out.galley.rows[last].glyphs.is_empty());
        assert!(near(cursor_of(&out).1, out.text_clip_rect.right()), "caret at {}", cursor_of(&out).1);
        // Typing Latin turns the paragraph left to right: its row goes to the left edge.
        let out = field.type_text("abc");
        assert_eq!(row_span(&out, last).0, out.text_clip_rect.left());
        assert_eq!(field.text, format!("{PARAGRAPH}\nabc"));
        // Arabic after it does not turn that paragraph (it begins with a Latin letter).
        let out = field.type_text("\u{639}\u{644}\u{64A}");
        assert_eq!(row_span(&out, last).0, out.text_clip_rect.left());
        // A second break, then Arabic: right to left again.
        field.press(Key::Enter);
        let out = field.type_text("\u{639}\u{644}\u{64A}");
        assert!(near(row_span(&out, last + 1).1, out.text_clip_rect.right()));
        assert_rows_aligned(&out, &field.map().expect("reordered"), &field.text);
    }

    #[test]
    fn a_field_that_gets_narrower_keeps_its_caret_at_the_same_place_in_the_text() {
        let Some((mut field, _)) = Field::multiline_with(300.0, PARAGRAPH) else { return };
        field.press_with(Key::Home, Modifiers::CTRL);
        for _ in 0..9 {
            field.press(Key::ArrowRight);
        }
        let at = field.caret().expect("a caret");
        for width in [300.0, 200.0, 120.0, 90.0, 150.0, 400.0] {
            field.width = width;
            let out = field.frame(vec![]);
            assert_eq!(field.text, PARAGRAPH);
            let now = field.caret().expect("a caret");
            assert_eq!(now.l, at.l, "at width {width}");
            let v = out.state.cursor.char_range().map(|r| r.primary.index.0);
            assert_eq!(v, Some(now.v), "egui's cursor is the caret's screen position at width {width}");
            assert_rows_aligned(&out, &field.map().expect("reordered"), PARAGRAPH);
        }
        // The same with a selection: both ends move.
        field.press_with(Key::ArrowRight, Modifiers::SHIFT);
        field.press_with(Key::ArrowRight, Modifiers::SHIFT);
        field.width = 100.0;
        let out = field.frame(vec![]);
        let range = out.state.cursor.char_range().expect("a selection");
        let map = field.map().expect("reordered");
        assert_eq!(map.logical_of_screen(range.as_sorted_char_range().start.0..range.as_sorted_char_range().end.0).len(), 2);
    }

    #[test]
    fn a_text_that_loses_its_arabic_hands_the_cursor_back_in_logical_indices() {
        // A Latin paragraph that wraps, with one Arabic letter in its second row.
        let text = format!("{LATIN} \u{639} omega");
        let Some((mut field, out)) = Field::multiline_with(150.0, &text) else { return };
        assert!(out.galley.rows.len() >= 2 && field.map().is_some_and(|m| m.shown.contains(&bidi::VIRTUAL)));
        // The caret goes to just before the letter. In the screen text a line break precedes it.
        let target = LATIN.chars().count() + 1;
        field.press_with(Key::Home, Modifiers::CTRL);
        for _ in 0..100 {
            if field.caret().map(|c| c.l) == Some(target) {
                break;
            }
            field.press(Key::ArrowRight);
        }
        let caret = field.caret().expect("a caret");
        assert_eq!(caret.l, target);
        assert!(caret.v > caret.l, "the break before it makes the screen position {} later than the text position {}", caret.v, caret.l);
        // Deleting the letter leaves nothing right to left: the field is egui's from the next frame on,
        // and the caret must stay where it was in the text, not where it was on the screen.
        field.press(Key::Delete);
        let wanted = format!("{LATIN}  omega");
        assert_eq!(field.text, wanted);
        let kept = TextEditState::load(&field.ctx, field.id).and_then(|s| s.cursor.char_range());
        assert_eq!(kept.map(|r| r.primary.index.0), Some(target), "the cursor counts the text's characters again");
        field.type_text("!");
        let expected: String = wanted.chars().take(target).chain(std::iter::once('!')).chain(wanted.chars().skip(target)).collect();
        assert_eq!(field.text, expected);
    }

    #[test]
    fn random_keystrokes_in_a_multi_line_field_never_panic_and_leave_one_glyph_per_character() {
        for seed in [0x9E37_79B9_7F4A_7C15, 0x1234_5678_9ABC_DEF1, 0xDEAD_BEEF_0BAD_F00D, 0x0F0F_1234_5555_AAAA, 0x7FFF_FFFF_FFFF_FFF1] {
            let Some(mut field) = Field::arabic() else { return };
            field.multiline = true;
            field.width = 90.0;
            if seed % 3 == 0 {
                field.char_limit = Some(40);
            }
            field.frame(vec![]);
            let mut dice = Dice(seed);
            let keys = [
                Key::Backspace,
                Key::Delete,
                Key::ArrowLeft,
                Key::ArrowRight,
                Key::ArrowUp,
                Key::ArrowDown,
                Key::Home,
                Key::End,
                Key::Enter,
                Key::Tab,
                Key::ArrowLeft,
                Key::ArrowRight,
            ];
            let modifiers = [Modifiers::NONE, Modifiers::NONE, Modifiers::SHIFT, Modifiers::CTRL, Modifiers::COMMAND | Modifiers::SHIFT];
            for step in 0..900 {
                let mut events = Vec::new();
                for _ in 0..=dice.next(2) {
                    events.push(match dice.next(12) {
                        0..=3 => Event::Text(dice.text(6)),
                        4..=7 => {
                            let (key, modifiers) = (keys[dice.next(keys.len())], modifiers[dice.next(modifiers.len())]);
                            Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }
                        }
                        8 => Event::Copy,
                        9 => Event::Cut,
                        10 => Event::Paste(dice.text(12)),
                        _ => {
                            let key = [Key::A, Key::Z, Key::Y][dice.next(3)];
                            Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: Modifiers::COMMAND }
                        }
                    });
                }
                if dice.next(25) == 0 {
                    field.time += 2.0;
                }
                if dice.next(30) == 0 {
                    // The field is resized: narrow, wide, hardly there.
                    field.width = [24.0, 40.0, 90.0, 150.0, 500.0][dice.next(5)];
                }
                let described = format!("{events:?}");
                let before = (field.text.clone(), field.width, field.caret());
                let out = field.frame(events);
                let end = out.galley.end().index.0;
                if let Some(range) = out.state.cursor.char_range() {
                    assert!(
                        range.primary.index.0 <= end && range.secondary.index.0 <= end,
                        "step {step}: {range:?} past {end} after {described} on {before:?} -> {:?} {:?} map {:?}",
                        field.text,
                        field.caret(),
                        field.map().map(|m| (m.screen_len(), m.shaper))
                    );
                }
                // A text with no right-to-left script is a plain field, which is egui's to keep right.
                let Some(map) = field.map() else { continue };
                assert_rows_aligned(&out, &map, &field.text);
                assert_eq!(end, map.screen_len(), "step {step}");
                if let Some(caret) = field.caret() {
                    assert!(caret.l <= map.logical_len() && caret.v <= map.screen_len(), "step {step}: {caret:?}");
                    assert!(map.screen_positions(caret.l).contains(&caret.v), "step {step}: {caret:?} in {:?}", field.text);
                }
            }
        }
    }

    #[test]
    fn hostile_text_in_a_multi_line_field_never_panics() {
        let many_words = "\u{627}\u{628}\u{62A} ".repeat(3000);
        let one_word = "\u{633}".repeat(6000);
        let texts = [
            String::new(),
            "\n".to_owned(),
            "\r\n".to_owned(),
            "\r".to_owned(),
            "\n\n\n\u{639}\n\n".to_owned(),
            "\u{639}\r\n\u{644}\r\n".to_owned(),
            "   ".to_owned(),
            "\u{639}  \u{644}   ".to_owned(),
            "  \u{639}".to_owned(),
            "\u{202B}".to_owned(),
            "\u{202B}\u{639}\u{202C} \u{202E}ab\u{202C}".to_owned(),
            "\u{200F}\u{200E}\u{2029}\u{85}\u{1C}x".to_owned(),
            "\u{64B}\u{64C}\u{64B}".to_owned(),
            "\u{200C}\u{200D}\u{200B}".to_owned(),
            "\u{10FFFF}\u{639}\u{E000}".to_owned(),
            "ab \u{663}\u{662} (\u{644}\u{627}) [\u{5D0}] \u{644}\u{627}\u{644}\u{627}".repeat(40),
            many_words,
            one_word,
        ];
        for text in &texts {
            for width in [24.0, 60.0, 200.0, 5000.0] {
                let Some((mut field, out)) = Field::multiline_with(width, text) else { return };
                if let Some(map) = field.map() {
                    assert_rows_aligned(&out, &map, text);
                }
                // A little editing at both ends and in the middle.
                for events in [
                    vec![Event::Key { key: Key::End, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::CTRL }],
                    vec![Event::Text("\u{639}".into())],
                    vec![Event::Key { key: Key::Backspace, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
                    vec![Event::Key { key: Key::ArrowUp, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
                    vec![Event::Key { key: Key::Delete, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
                    vec![Event::Key { key: Key::A, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND }, Event::Cut],
                ] {
                    let out = field.frame(events);
                    if let Some(map) = field.map() {
                        assert_rows_aligned(&out, &map, &field.text);
                    }
                }
            }
        }
    }

    #[test]
    fn a_field_with_a_huge_text_is_left_to_egui() {
        let Some(mut field) = Field::arabic() else { return };
        field.multiline = true;
        field.text = "\u{639}\u{644}\u{64A} ".repeat(MAX_WRAPPED_CHARS / 4 + 10);
        let out = field.frame(vec![]);
        assert!(field.map().is_none(), "not reordered");
        assert!(out.galley.rows.len() > 1, "but wrapped by egui");
    }

    #[test]
    fn up_and_down_move_between_the_rows_as_drawn_and_home_and_end_go_along_a_row() {
        let Some((mut field, _)) = Field::multiline_with(150.0, PARAGRAPH) else { return };
        let on_row = |out: &TextEditOutput| {
            let at = out.state.cursor.char_range().expect("a cursor").primary;
            out.galley.layout_from_cursor(at)
        };
        let out = field.press_with(Key::End, Modifiers::CTRL);
        let last = out.galley.rows.len() - 1;
        assert_eq!(on_row(&out).row, last);
        assert_eq!(field.caret().map(|c| c.l), Some(PARAGRAPH.chars().count()), "Ctrl+End is the end of the text in reading order");
        let out = field.press(Key::ArrowUp);
        assert_eq!(on_row(&out).row, last - 1);
        // Home is the left end of the row, End the right end, whichever way the row reads.
        let out = field.press(Key::Home);
        assert_eq!((on_row(&out).row, on_row(&out).column.0), (last - 1, 0));
        let out = field.press(Key::End);
        assert_eq!((on_row(&out).row, on_row(&out).column.0), (last - 1, out.galley.rows[last - 1].glyphs.len()));
        let out = field.press(Key::ArrowDown);
        assert_eq!(on_row(&out).row, last);
        // Whatever the keys did, the text is the paragraph.
        assert_eq!(field.text, PARAGRAPH);
    }

    /// The same events on a multi-line [`BidiEdit`] and a plain multi-line `TextEdit` give the same
    /// text, cursor and look when there is nothing right to left.
    #[test]
    fn multi_line_text_without_right_to_left_script_is_a_plain_text_edit() {
        let run = |plain: bool| {
            let mut field = Field::arabic().expect("the fonts are there");
            (field.multiline, field.plain, field.width) = (true, plain, 120.0);
            field.text = "report 2024 final version of the document".to_owned();
            let mut seen = Vec::new();
            for events in [
                vec![],
                vec![Event::Text(" and more words".into())],
                vec![key(Key::ArrowLeft), key(Key::ArrowUp)],
                vec![key(Key::Enter), Event::Text("next line".into())],
                vec![key(Key::Backspace), key(Key::ArrowDown), key(Key::Home)],
                vec![Event::Key { key: Key::A, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND }],
            ] {
                let out = field.frame(events);
                seen.push((
                    out.state.cursor.char_range().map(|r| (r.primary.index.0, r.secondary.index.0)),
                    out.galley.job.text.clone(),
                    out.galley.rows.iter().map(|r| (r.pos, r.text())).collect::<Vec<_>>(),
                    out.response.response.rect,
                ));
            }
            (field.text, seen)
        };
        let (plain_text, plain) = run(true);
        let (ours_text, ours) = run(false);
        assert_eq!(ours_text, plain_text);
        assert_eq!(ours, plain);
    }

    #[test]
    fn events_after_an_undo_wait_for_the_next_frame() {
        let Some(mut field) = Field::arabic() else { return };
        (field.multiline, field.width) = (true, 60.0);
        field.frame(vec![]);
        // Typed word by word in a narrow field, with a pause after each so that each is an undo step.
        for word in PARAGRAPH.split_inclusive(' ') {
            field.type_text(word);
            field.idle();
        }
        let typed = field.text.clone();
        // The field grows: the cursor the undo gives back counts the lines of the narrow field.
        field.width = 500.0;
        let with = |key, modifiers| Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers };
        let out = field.frame(vec![with(Key::Z, Modifiers::COMMAND), with(Key::ArrowLeft, Modifiers::CTRL)]);
        assert!(typed.starts_with(&field.text), "the undo gave back an earlier text: {:?}", field.text);
        let end = out.galley.end().index.0;
        assert!(out.state.cursor.char_range().is_some_and(|r| r.primary.index.0 <= end && r.secondary.index.0 <= end));
        // The word move was held back, and runs now, from where the undo left the cursor.
        let before = field.caret();
        let out = field.frame(vec![]);
        assert!(out.state.cursor.char_range().is_some_and(|r| r.primary.index.0 <= out.galley.end().index.0));
        assert_ne!(field.caret(), before, "the held-back Ctrl+Left moved the caret");
        // Nothing is left over for a later frame.
        let again = field.caret();
        field.frame(vec![]);
        assert_eq!(field.caret(), again);
    }
}
