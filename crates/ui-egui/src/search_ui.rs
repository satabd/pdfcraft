//! Edit ▸ Advanced Search (⇧⌘F): the search in a side panel that lists every match with its
//! context, page by page; click a result to go to it. It shares the find bar's search.

use egui::text::LayoutJob;
use egui::{Color32, FontId, TextFormat};

use crate::canvas::DocView;
use crate::theme::{self, Tokens};
use crate::widgets;

/// Glyphs of context shown on each side of a match.
const CONTEXT: usize = 36;

/// A result's context split for painting: the text before the match, the match (when it can be
/// told from the rest) and the text after it.
#[derive(Debug, PartialEq, Eq)]
struct Snippet {
    before: String,
    hit: Option<String>,
    after: String,
}

/// The sections of a result line from its flattened `before`, `hit` and `after` text (logical
/// order, whitespace collapsed). Text without right-to-left script is `…before `, the hit and
/// ` after…`. With some, egui would lay the runs out in logical order and swap the words, so the
/// whole line is put in display order ([`crate::bidi::visual_auto`]) and the hit is looked up in
/// it: a stretch of right-to-left script is handed back to the shaper in logical order, so a
/// hit of one script stays a contiguous substring. Found exactly once, the line is split around
/// it and the hit keeps its highlight; otherwise the whole display line is plain.
fn snippet_sections(before: &str, hit: &str, after: &str) -> Snippet {
    let line = format!("…{before} {hit} {after}…");
    if !crate::bidi::has_rtl(&line) {
        return Snippet { before: format!("…{before} "), hit: Some(hit.to_owned()), after: format!(" {after}…") };
    }
    let shown = crate::bidi::visual_auto(&line).into_owned();
    let mut found = shown.match_indices(hit).map(|(at, _)| at);
    if !hit.is_empty()
        && let (Some(at), None) = (found.next(), found.next())
        && let (Some(head), Some(tail)) = (shown.get(..at), shown.get(at.saturating_add(hit.len())..))
    {
        return Snippet { before: head.to_owned(), hit: Some(hit.to_owned()), after: tail.to_owned() };
    }
    Snippet { before: shown, hit: None, after: String::new() }
}

pub(crate) fn panel(ui: &mut egui::Ui, t: &Tokens, view: &mut DocView, pages: usize) {
    crate::widgets::begin_dialog(ui);
    let searched = view.texts.len() + view.text_failed.len();
    let Some(find) = view.find.as_mut() else { return };
    find.in_panel = true;
    let l = ui.label(egui::RichText::new(tl!("What word or phrase would you like to search for?")).color(t.text_muted));
    let r = ui
        .add(crate::widgets::line(&mut find.query).id(egui::Id::new("search-panel-input")).hint_text(tl!("Search")).desired_width(f32::INFINITY))
        .labelled_by(l.id);
    if find.focus {
        r.request_focus();
        find.focus = false;
    }
    ui.horizontal(|ui| {
        // Outlined boxes (the panel and the check-box fill are alike), as in dialogs.
        let w = &mut ui.visuals_mut().widgets;
        w.inactive.bg_stroke = egui::Stroke::new(1.0, t.border);
        w.inactive.bg_fill = t.hover;
        let a = ui.add(widgets::checkbox(&mut find.whole_words, tl!("Whole words only"))).changed();
        let b = ui.add(widgets::checkbox(&mut find.case_sensitive, tl!("Case-sensitive"))).changed();
        if a || b {
            find.case_query.clear();
        }
    });
    if find.query != find.case_query {
        view.rerun_find();
    }
    let Some(find) = view.find.as_ref() else { return };
    ui.add_space(6.0);
    let status = if find.query.trim().is_empty() {
        String::new()
    } else if find.matches.is_empty() && searched < pages {
        crate::i18n::fmt(tl!("Searching… {s} of {p} pages"), &[("s", &searched.to_string()), ("p", &pages.to_string())])
    } else {
        let n = find.matches.len();
        let more = if searched < pages {
            crate::i18n::fmt(tl!(" (searching… {s} of {p} pages)"), &[("s", &searched.to_string()), ("p", &pages.to_string())])
        } else {
            String::new()
        };
        // English keeps its singular; Chinese has no plural.
        let count = if n == 1 { crate::i18n::fmt(tl!("1 instance"), &[]) } else { crate::i18n::fmt(tl!("{n} instances"), &[("n", &n.to_string())]) };
        format!("{count}{more}")
    };
    ui.label(egui::RichText::new(status).font(theme::semibold(12.5)));
    ui.add_space(4.0);
    let mut go = None;
    let mut last_page = None;
    for (i, (p, range)) in find.matches.iter().enumerate().take(2000) {
        if last_page != Some(*p) {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(crate::i18n::fmt(tl!("Page {label}"), &[("label", &(p + 1).to_string())])).small().color(t.text_faint));
            last_page = Some(*p);
        }
        let Some(text) = view.texts.get(p) else { continue };
        let n = text.glyphs.len();
        let before = text.text_of(range.start.saturating_sub(CONTEXT)..range.start.min(n));
        let hit = text.text_of(range.start.min(n)..range.end.min(n));
        let after = text.text_of(range.end.min(n)..(range.end + CONTEXT).min(n));
        let flat = |s: String| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut job = LayoutJob::default();
        let font = FontId::new(12.5, egui::FontFamily::Proportional);
        let plain = TextFormat { font_id: font.clone(), color: t.text_muted, ..Default::default() };
        let snippet = snippet_sections(&flat(before), &flat(hit), &flat(after));
        job.append(&snippet.before, 0.0, plain.clone());
        if let Some(hit) = &snippet.hit {
            job.append(
                hit,
                0.0,
                TextFormat {
                    font_id: theme::semibold(12.5),
                    color: t.text,
                    background: Color32::from_rgba_unmultiplied(255, 214, 0, 90),
                    ..Default::default()
                },
            );
        }
        job.append(&snippet.after, 0.0, plain);
        job.wrap.max_width = ui.available_width();
        let selected = find.current == Some(i);
        let resp = ui.add(egui::Button::selectable(selected, job).wrap()).on_hover_text(tl!("Go to this result"));
        if resp.clicked() {
            go = Some(i);
        }
    }
    if let Some(i) = go {
        view.go_to_match(i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sections(s: &Snippet) -> String {
        format!("{}{}{}", s.before, s.hit.as_deref().unwrap_or_default(), s.after)
    }

    #[test]
    fn latin_context_is_unchanged() {
        let s = snippet_sections("the quick", "brown", "fox jumps");
        assert_eq!(s, Snippet { before: "…the quick ".into(), hit: Some("brown".into()), after: " fox jumps…".into() });
        // Other scripts that read left to right are left alone too.
        let s = snippet_sections("日本", "語", "文字");
        assert_eq!(s.hit.as_deref(), Some("語"));
        assert_eq!(sections(&s), "…日本 語 文字…");
    }

    #[test]
    fn arabic_context_is_in_display_order_with_the_hit_highlighted_once() {
        let (before, hit, after) = ("ذهب الولد إلى", "المدرسة", "في الصباح");
        let s = snippet_sections(before, hit, after);
        assert_eq!(s.hit.as_deref(), Some(hit), "{s:?}");
        // The words of the right-to-left line run from the right: the last word is leftmost.
        // Computed with `visual_auto` for "…ذهب الولد إلى المدرسة في الصباح…": the leading
        // ellipsis ends up at the right end, the trailing one at the left, and the words in reverse.
        assert_eq!(sections(&s), "…الصباح في المدرسة إلى الولد ذهب…");
        assert_eq!(s.before, "…الصباح في ");
        assert_eq!(s.after, " إلى الولد ذهب…");
        // Latin words and numbers inside keep their own order.
        let s = snippet_sections("صفحة 12 من", "Report", "النهائي");
        assert_eq!(s.hit.as_deref(), Some("Report"));
        assert_eq!(sections(&s), crate::bidi::visual_auto("…صفحة 12 من Report النهائي…"));
    }

    #[test]
    fn a_hit_that_cannot_be_located_once_is_not_highlighted() {
        // Twice in the line: ambiguous, so the whole display line is plain.
        let s = snippet_sections("كتاب", "كتاب", "جديد");
        assert_eq!(s.hit, None);
        assert_eq!(s.after, "");
        assert_eq!(s.before, crate::bidi::visual_auto("…كتاب كتاب جديد…"));
        // A mixed hit is reordered inside the line and no longer a substring of it.
        let s = snippet_sections("ملف", "a ب", "جديد");
        assert_eq!(s.hit, None);
        assert_eq!(s.before, crate::bidi::visual_auto("…ملف a ب جديد…"));
        // An empty hit has nothing to highlight.
        assert_eq!(snippet_sections("ملف", "", "جديد").hit, None);
    }
}
