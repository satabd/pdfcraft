//! Edit a PDF ▸ Edit text & images: boxes around the paragraphs and images already on the page.
//! Click a paragraph to edit it in place (⌘Enter or clicking away applies and rewraps it to the
//! box, Esc cancels); drag it to move it, or drag the handle on its left or right edge to rewrap
//! it to a new width. Click an image to select it: drag to move, drag a corner to resize (keeping
//! its proportions), right-click for rotate, flip, replace, save and delete; Delete removes it.

use std::collections::BTreeMap;
use std::sync::Arc;

use egui::{Color32, CornerRadius, FontFamily, FontId, Pos2, Rect, Stroke};
use pdfcraft_engine::{Edit, ObjectKind, ObjectTarget};
use pdfcraft_render::DocInfo;

use crate::bidi::{self, ScreenMap};
use crate::canvas::{DocView, PageXform};
use crate::widgets::{self, LayoutCtx, RowSlot};

const ACCENT: Color32 = Color32::from_rgb(0x14, 0x73, 0xE6);

#[derive(Clone, Copy, PartialEq)]
enum TextTarget {
    Block(usize),
    Added(usize),
}

/// A cached, read-only page background. The source glyphs are removed before rasterization;
/// colour, pictures and unselected text beneath the editor remain visible.
#[derive(Default)]
pub(crate) struct TextBackground {
    key: Option<(u64, usize, TextTarget)>,
    renderer: Option<pdfcraft_render::RenderPool>,
    texture: Option<egui::TextureHandle>,
    tag: Option<u64>,
    received: Option<u64>,
    draft_key: Option<(String, pdfcraft_engine::BlockStyle)>,
    draft_rect: Option<[f64; 4]>,
}

impl TextBackground {
    pub fn pending(&self) -> bool {
        self.renderer.is_some() && self.received != self.tag
    }

    pub fn prepare(
        &mut self,
        ctx: &egui::Context,
        doc: &pdfcraft_engine::Document,
        editor: Option<&LineEditor>,
        draft: Option<&crate::content_ui::TextDraft>,
        scale: f32,
    ) {
        let key = editor
            .map(|ed| (doc.edit_generation(), ed.page, TextTarget::Block(ed.block)))
            .or_else(|| draft.and_then(|d| d.index.map(|index| (doc.edit_generation(), d.page, TextTarget::Added(index)))));
        if self.key != key {
            *self = Self { key, ..Default::default() };
            if let Some((_, page, target)) = key {
                self.renderer = match target {
                    TextTarget::Block(block) => doc.text_edit_preview(page, block),
                    TextTarget::Added(index) => doc.added_text_preview(page, index),
                }
                .ok()
                .map(|p| p.renderer());
            }
        }
        if let Some(ed) = editor {
            let draft_key = (ed.text.clone(), ed.style());
            if self.draft_key.as_ref() != Some(&draft_key) {
                self.draft_rect = doc.text_edit_layout(ed.page, ed.block, &ed.text, &draft_key.1).ok().and_then(|layout| layout.draft_rect);
                self.draft_key = Some(draft_key);
            }
        }
        let (Some((_, page, _)), Some(renderer)) = (self.key, self.renderer.as_ref()) else { return };
        let Some(p) = doc.info.pages.get(page) else { return };
        // One raster stands in for the page's tiles while a paragraph is open, so everything
        // around the editor is drawn from it: at the tiling threshold it stays sharp on a Retina
        // display up to about 2.5x on a Letter or A4 page, where the backdrop side (2048) went
        // soft past 1.2x.
        let max_side = ctx.input(|i| i.max_texture_side) as f32;
        let scale = scale.min(crate::canvas::TILE_THRESHOLD.min(max_side) / p.width.max(p.height).max(1.0));
        let tag = (scale * 1000.0) as u64;
        if self.tag != Some(tag) {
            renderer.set_queue(vec![pdfcraft_render::RenderRequest { page, scale, tag, ..Default::default() }]);
            self.tag = Some(tag);
        }
        if let Some(out) = renderer.try_recv() {
            if out.error.is_some() {
                self.renderer = None;
                self.texture = None;
                return;
            }
            if out.request.tag == tag {
                self.texture = Some(ctx.load_texture(
                    "text-editor-background",
                    crate::canvas::texture_image([out.width as usize, out.height as usize], out.rgba),
                    egui::TextureOptions::LINEAR,
                ));
                self.received = Some(tag);
            }
        }
        if self.received != Some(tag) {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    pub fn ready(&self, page: usize, block: usize) -> bool {
        self.key.is_some_and(|(_, p, target)| (p, target) == (page, TextTarget::Block(block))) && self.texture.is_some()
    }

    pub fn ready_added(&self, page: usize, index: usize) -> bool {
        self.key.is_some_and(|(_, p, target)| (p, target) == (page, TextTarget::Added(index))) && self.texture.is_some()
    }

    pub fn paint(
        &self,
        painter: &egui::Painter,
        xf: &PageXform,
        page: usize,
        editor: Option<&LineEditor>,
        draft: Option<&crate::content_ui::TextDraft>,
    ) {
        let ready = editor.is_some_and(|ed| ed.page == page && self.ready(page, ed.block))
            || draft.is_some_and(|d| d.page == page && d.index.is_some_and(|index| self.ready_added(page, index)));
        if !ready {
            return;
        }
        let Some(tex) = &self.texture else { return };
        // Font metrics can underestimate the actual glyph ink. Paint the complete textless
        // page so overhanging glyphs cannot remain outside the estimated source rectangle.
        let painter = painter.with_clip_rect(painter.clip_rect().intersect(xf.rect));
        xf.paint_image(&painter, tex.id(), 0.0, 0.0, 1.0, 1.0);
    }
}

/// How many faint Edit-text outlines are drawn per page per frame. CAD plots can draw
/// ~1000 tiny images on one page; stroking all of them every frame (plus a hover query
/// per box) stalls the UI. Past this, only the selected and hovered boxes are outlined:
/// hit-testing still covers every image, so nothing becomes unselectable.
const MAX_OUTLINES: usize = 250;

/// A paragraph being edited.
#[derive(Clone, Debug, PartialEq)]
pub struct LineEditor {
    pub selection: Option<std::ops::Range<usize>>,
    pub font_resource: Option<String>,
    pub page: usize,
    pub block: usize,
    pub text: String,
    original: String,
    rect: Rect,
    /// The original PDF rectangle, in user space. It is converted again each frame so the editor
    /// stays attached while the page is zoomed, scrolled, or rotated.
    source_rect: [f32; 4],
    multiline: bool,
    source: Option<(pdfcraft_engine::TextBlock, Vec<pdfcraft_engine::TextLine>)>,
    pub(crate) fonts: BTreeMap<String, String>,
    /// The draft extends outside the source container; its frame follows the live draft bounds.
    pub overflow: bool,
    /// The initial source container width in screen pixels.
    max_width: f32,
    /// Current screen-space size. Recomputed from the current page transform before painting.
    size: f32,
    focus: bool,
    /// The Format text panel's values, and what the paragraph had (to send only changes).
    pub look: pdfcraft_engine::AddedText,
    look0: pdfcraft_engine::AddedText,
    /// Underline, line spacing (× size; 0 = the paragraph's own), character spacing (pt) and
    /// horizontal scale (%), and what they were.
    pub extras: Extras,
    extras0: Extras,
    /// A read-only copy of the document to ask what rows the page will lay a draft out in (see [`DraftProbe`]).
    probe: Option<DraftProbe>,
}

/// [`pdfcraft_engine::TextDraftProbe`], shared by the copies of the editor the frame makes, with the layouts it was last asked
/// for: the rewrite behind a layout runs once per change of the draft or of the Format panel, not once per use in a frame.
#[derive(Clone)]
struct DraftProbe(std::rc::Rc<ProbeMemo>);

/// A layout the rewrite made of a draft in a style (`None`: it refuses the draft).
type Answer = (String, pdfcraft_engine::BlockStyle, Option<std::rc::Rc<pdfcraft_engine::TextEditLayout>>);

struct ProbeMemo {
    probe: pdfcraft_engine::TextDraftProbe,
    /// The last few answers, the newest last.
    answers: std::cell::RefCell<Vec<Answer>>,
}

/// How many answers a probe keeps (a frame asks about the draft as it was and as it is).
const KEPT_ANSWERS: usize = 4;

impl DraftProbe {
    fn new(probe: pdfcraft_engine::TextDraftProbe) -> Self {
        Self(std::rc::Rc::new(ProbeMemo { probe, answers: std::cell::RefCell::new(Vec::new()) }))
    }

    /// The layout the rewrite that applies `text` in `style` makes, if it accepts the draft.
    fn layout(&self, text: &str, style: &pdfcraft_engine::BlockStyle) -> Option<std::rc::Rc<pdfcraft_engine::TextEditLayout>> {
        if let Ok(answers) = self.0.answers.try_borrow()
            && let Some((_, _, layout)) = answers.iter().find(|(was_text, was_style, _)| was_text == text && was_style == style)
        {
            return layout.clone();
        }
        let layout = self.0.probe.layout(text, style).map(std::rc::Rc::new);
        if let Ok(mut answers) = self.0.answers.try_borrow_mut() {
            if answers.len() >= KEPT_ANSWERS {
                answers.remove(0);
            }
            answers.push((text.to_owned(), style.clone(), layout.clone()));
        }
        layout
    }
}

impl std::fmt::Debug for DraftProbe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DraftProbe")
    }
}

impl PartialEq for DraftProbe {
    fn eq(&self, other: &Self) -> bool {
        std::rc::Rc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extras {
    pub underline: bool,
    pub line_spacing: f64,
    pub char_spacing: f64,
    pub scale: f64,
}

impl Default for Extras {
    fn default() -> Self {
        Extras { underline: false, line_spacing: 0.0, char_spacing: 0.0, scale: 100.0 }
    }
}

/// Underline, line spacing, character spacing and horizontal scale for the paragraph being
/// edited (under Format text). Returns `true` when something changed.
pub(crate) fn extras_panel(ui: &mut egui::Ui, e: &mut Extras) -> bool {
    let before = *e;
    ui.horizontal(|ui| {
        if crate::icons::button(ui, "underline", 26.0, e.underline, tl!("Underline")).clicked() {
            e.underline = !e.underline;
        }
        let zero = tl!("Line spacing").to_string();
        let label = |v: f64| if v == 0.0 { zero.clone() } else { format!("{v:.2}×") };
        egui::ComboBox::from_id_salt("line-spacing").selected_text(label(e.line_spacing)).width(110.0).show_ui(ui, |ui| {
            for v in [1.0, 1.15, 1.5, 2.0] {
                ui.selectable_value(&mut e.line_spacing, v, label(v));
            }
        });
    });
    ui.horizontal(|ui| {
        let l = ui.label(tl!("Character spacing"));
        ui.add(egui::DragValue::new(&mut e.char_spacing).range(-5.0..=50.0).speed(0.1).suffix(" pt")).labelled_by(l.id);
    });
    ui.horizontal(|ui| {
        let l = ui.label(tl!("Horizontal scale"));
        ui.add(egui::DragValue::new(&mut e.scale).range(10.0..=400.0).speed(1.0).suffix(" %")).labelled_by(l.id);
    });
    *e != before
}

impl LineEditor {
    pub(crate) fn has_changes(&self) -> bool {
        self.text != self.original || self.look != self.look0 || self.extras != self.extras0
    }

    /// Extra width permitted by the editor. Source containers now remain fixed.
    pub fn growth(&self) -> Option<f32> {
        (self.max_width > self.rect.width()).then_some(self.max_width)
    }

    /// Whether the edited text differs from the original (unsaved changes).
    pub fn has_unsaved_text(&self) -> bool {
        self.text != self.original
    }

    /// The formatting the panel changed.
    pub fn style(&self) -> pdfcraft_engine::BlockStyle {
        let (l, o) = (&self.look, &self.look0);
        let mut style = pdfcraft_engine::BlockStyle {
            resource: self.font_resource.clone(),
            family: (l.family != o.family || l.bold != o.bold || l.italic != o.italic).then_some((l.family, l.bold, l.italic)),
            size: (l.size != o.size).then_some(l.size),
            color: (l.color != o.color).then_some(l.color),
            align: (l.align != o.align).then_some(l.align),
            underline: (self.extras.underline != self.extras0.underline).then_some(self.extras.underline),
            line_spacing: (self.extras.line_spacing != self.extras0.line_spacing && self.extras.line_spacing > 0.0)
                .then_some(self.extras.line_spacing),
            char_spacing: (self.extras.char_spacing != self.extras0.char_spacing).then_some(self.extras.char_spacing),
            scale: (self.extras.scale != self.extras0.scale).then_some(self.extras.scale),
            ..Default::default()
        };
        if let Some(range) = &self.selection
            && (style.family.is_some()
                || style.resource.is_some()
                || style.size.is_some()
                || style.color.is_some()
                || style.underline.is_some()
                || style.char_spacing.is_some()
                || style.scale.is_some())
        {
            style.ranges.push(pdfcraft_engine::RangeStyle {
                range: range.clone(),
                family: style.family.take(),
                resource: style.resource.take(),
                size: style.size.take(),
                color: style.color.take(),
                underline: style.underline.take(),
                char_spacing: style.char_spacing.take(),
                scale: style.scale.take(),
            });
        }
        style
    }

    /// What the rewrite that applies the draft `text` in the Format panel's style lays the paragraph out as: its rows and
    /// their boxes, and the page fonts it shapes Arabic in. `None` where the editor has no copy of the document to ask,
    /// or the rewrite refuses the draft.
    fn layout_of(&self, text: &str) -> Option<std::rc::Rc<pdfcraft_engine::TextEditLayout>> {
        self.probe.as_ref()?.layout(text, &self.style())
    }

    /// The font family (a `pdf-source-N` the editor installed) that draws the Arabic letters of each source line of the
    /// paragraph for the draft `text`: the page's own face, where Apply shapes the draft in it (see
    /// [`pdfcraft_engine::TextEditLayout::shaping_faces`]). A line the rewrite draws in another face (the generated shaped
    /// glyphs of the interface's Arabic face) is not in the map.
    fn page_faces(&self, text: &str) -> BTreeMap<usize, String> {
        if !text.chars().any(bidi::is_joining_script) {
            return BTreeMap::new();
        }
        let Some(layout) = self.layout_of(text) else { return BTreeMap::new() };
        layout.shaping_faces.iter().filter_map(|(line, resource)| Some((*line, self.fonts.get(resource)?.clone()))).collect()
    }

    /// After applying formatting: the paragraph now has it.
    pub fn applied(&mut self) {
        self.selection = None;
        self.font_resource = None;
        self.look0 = self.look.clone();
        self.extras0 = self.extras;
        self.original = self.text.clone();
        self.focus = true;
    }

    /// Adopt the rewritten paragraph's current geometry before the next overlay frame.
    pub(crate) fn refresh_source(&mut self, block: &pdfcraft_engine::TextBlock, lines: Vec<pdfcraft_engine::TextLine>) {
        self.source_rect = block.rect.map(|v| v as f32);
        self.multiline = block.rows.len() > 1;
        self.source = Some((block.clone(), lines));
        self.text = block.editor_text.clone();
        self.original = self.text.clone();
    }
}

/// The look shown for a paragraph: the family and weight guessed from its PDF font name.
fn source_look(base_font: &str, size: f64, color: [f64; 3], detected_bold: bool, detected_italic: bool) -> pdfcraft_engine::AddedText {
    use pdfcraft_engine::FontFamily as F;
    let name = base_font.to_ascii_lowercase();
    let family = if ["courier", "mono", "consolas", "menlo", "monaco", "lucida console"].iter().any(|s| name.contains(s)) {
        F::Courier
    } else if !name.contains("sans") && ["times", "serif", "roman", "cambria", "georgia", "palatino", "garamond"].iter().any(|s| name.contains(s)) {
        F::Times
    } else {
        F::Helvetica
    };
    pdfcraft_engine::AddedText {
        family,
        bold: detected_bold || ["bold", "black", "heavy", "semibold", "demi"].iter().any(|s| name.contains(s)),
        italic: detected_italic || ["italic", "oblique", "slanted"].iter().any(|s| name.contains(s)),
        size: (size * 10.0).round() / 10.0,
        color,
        ..Default::default()
    }
}

fn look_of(b: &pdfcraft_engine::TextBlock) -> pdfcraft_engine::AddedText {
    source_look(&b.base_font, b.size, b.color, b.bold, b.italic)
}

pub(crate) fn editor_font(look: &pdfcraft_engine::AddedText, size: f32) -> FontId {
    let family = match (look.family, look.bold) {
        (pdfcraft_engine::FontFamily::Courier, _) => FontFamily::Monospace,
        (_, true) => FontFamily::Name("semibold".into()),
        (_, false) => FontFamily::Proportional,
    };
    FontId::new(size, family)
}

fn color32(color: [f64; 3]) -> Color32 {
    Color32::from_rgb(
        (color[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (color[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (color[2].clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

/// The dark box behind text too pale to read on white.
const DARK_EDITOR_FILL: Color32 = Color32::from_gray(0x26);

/// WCAG relative luminance of an sRGB colour.
fn luminance(c: Color32) -> f32 {
    let channel = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
}

/// WCAG contrast ratio between two colours (1 to 21).
fn contrast(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The inline editor's background for text drawn in `text`: white, or a dark box when the text
/// reads better on that (white or pale text, which would vanish on white).
pub(crate) fn editor_fill(text: Color32) -> Color32 {
    if contrast(text, Color32::WHITE) >= contrast(text, DARK_EDITOR_FILL) { Color32::WHITE } else { DARK_EDITOR_FILL }
}

/// A selected page image, and what the pointer is doing to it.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageSelection {
    pub page: usize,
    pub index: usize,
    /// Dragging: the start point and, for a corner, the opposite corner (screen).
    drag: Option<(Pos2, Option<Pos2>)>,
    /// Crop mode (Acrobat's crop handles): the box kept so far.
    pub crop: Option<CropMode>,
}

/// Cropping a page image: the handles trim instead of resizing, the part that will go is
/// dimmed, Enter or Done applies and Esc or Cancel leaves it.
#[derive(Clone, Debug, PartialEq)]
pub struct CropMode {
    /// The box to keep (user space).
    pub keep: [f64; 4],
    /// A drag in progress: the grip (0–3 corners, 4–7 edges, 8 the inside), where it started
    /// and the box then.
    drag: Option<(usize, Pos2, [f64; 4])>,
}

impl CropMode {
    pub fn new(keep: [f64; 4]) -> Self {
        Self { keep, drag: None }
    }
}

/// The screen boxes of the crop's eight handles: corners, then the top, right, bottom and left
/// edge midpoints.
fn crop_handles(k: Rect) -> [Pos2; 8] {
    [k.left_top(), k.right_top(), k.left_bottom(), k.right_bottom(), k.center_top(), k.right_center(), k.center_bottom(), k.left_center()]
}

/// Crop mode for the selected image `i` whose visible frame is `b` (screen). Returns `true`
/// (the pointer and keys are the crop's while it lasts).
#[allow(clippy::too_many_arguments)]
fn crop_input(
    ui: &egui::Ui,
    resp: &egui::Response,
    xf: &PageXform,
    page: usize,
    info: &DocInfo,
    view: &mut DocView,
    i: usize,
    b: Rect,
    allow_interaction: bool,
) -> bool {
    let painter = ui.painter();
    let Some(mode) = view.image_selection.as_mut().and_then(|s| s.crop.as_mut()) else { return false };
    let hover_pos = ui.input(|inp| inp.pointer.hover_pos());
    let keep_screen = |keep: [f64; 4]| xf.user_rect(info, page, keep.map(|v| v as f32)).intersect(b);
    // A drag in progress moves a grip; the box stays inside the picture and at least 6 px.
    if allow_interaction
        && resp.drag_started()
        && let Some(o) = ui.input(|inp| inp.pointer.press_origin())
    {
        let k = keep_screen(mode.keep);
        let grip = crop_handles(k).iter().position(|h| h.distance(o) < 8.0).or_else(|| k.contains(o).then_some(8));
        if let Some(g) = grip {
            mode.drag = Some((g, o, mode.keep));
        }
    }
    if let Some((grip, start, at_start)) = mode.drag
        && let Some(p) = hover_pos
    {
        let d = p - start;
        let k0 = keep_screen(at_start);
        let (mut min, mut max) = (k0.min, k0.max);
        match grip {
            0 => {
                min.x += d.x;
                min.y += d.y;
            }
            1 => {
                max.x += d.x;
                min.y += d.y;
            }
            2 => {
                min.x += d.x;
                max.y += d.y;
            }
            3 => {
                max.x += d.x;
                max.y += d.y;
            }
            4 => min.y += d.y,
            5 => max.x += d.x,
            6 => max.y += d.y,
            7 => min.x += d.x,
            _ => {
                let shift = egui::vec2(d.x.clamp(b.min.x - k0.min.x, b.max.x - k0.max.x), d.y.clamp(b.min.y - k0.min.y, b.max.y - k0.max.y));
                min += shift;
                max += shift;
            }
        }
        let (min_w, min_h) = (6.0_f32.min(b.width()), 6.0_f32.min(b.height()));
        min.x = min.x.clamp(b.min.x, b.max.x - min_w);
        min.y = min.y.clamp(b.min.y, b.max.y - min_h);
        max.x = max.x.clamp(min.x + min_w, b.max.x);
        max.y = max.y.clamp(min.y + min_h, b.max.y);
        mode.keep = user_box(xf, info, page, Rect::from_min_max(min, max));
        if resp.drag_stopped() || !ui.input(|inp| inp.pointer.primary_down()) {
            mode.drag = None;
        }
    }
    let k = keep_screen(mode.keep);
    // What goes is dimmed; what stays keeps its frame and handles.
    let dim = Color32::from_black_alpha(96);
    for r in [
        Rect::from_min_max(b.min, Pos2::new(b.max.x, k.min.y)),
        Rect::from_min_max(Pos2::new(b.min.x, k.max.y), b.max),
        Rect::from_min_max(Pos2::new(b.min.x, k.min.y), Pos2::new(k.min.x, k.max.y)),
        Rect::from_min_max(Pos2::new(k.max.x, k.min.y), Pos2::new(b.max.x, k.max.y)),
    ] {
        if r.is_positive() {
            painter.rect_filled(r.intersect(xf.rect), CornerRadius::ZERO, dim);
        }
    }
    painter.rect_stroke(k, CornerRadius::ZERO, Stroke::new(1.5, ACCENT), egui::StrokeKind::Outside);
    for h in crop_handles(k) {
        painter.rect(
            Rect::from_center_size(h, egui::vec2(8.0, 8.0)),
            CornerRadius::ZERO,
            Color32::WHITE,
            Stroke::new(1.0, ACCENT),
            egui::StrokeKind::Middle,
        );
    }
    // Done and Cancel, painted under the box (above it near the bottom of the view).
    let font = egui::FontId::proportional(12.5);
    let pill = |label: &str, right: f32, y: f32| -> Rect {
        let w = painter.layout_no_wrap(label.to_string(), font.clone(), Color32::WHITE).size().x + 20.0;
        Rect::from_min_size(Pos2::new(right - w, y), egui::vec2(w, 22.0))
    };
    let y = if k.max.y + 30.0 < xf.rect.max.y { k.max.y + 6.0 } else { k.min.y - 28.0 };
    let done = pill(tl!("Done"), k.max.x, y);
    let cancel = pill(tl!("Cancel"), done.min.x - 6.0, y);
    let t = crate::theme::Tokens::get(ui.ctx());
    painter.rect_filled(done, CornerRadius::same(11), ACCENT);
    painter.text(done.center(), egui::Align2::CENTER_CENTER, tl!("Done"), font.clone(), Color32::WHITE);
    painter.rect(cancel, CornerRadius::same(11), t.panel, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    painter.text(cancel.center(), egui::Align2::CENTER_CENTER, tl!("Cancel"), font, t.text);
    if let Some(p) = hover_pos {
        let icon = if let Some(g) = crop_handles(k).iter().position(|h| h.distance(p) < 8.0) {
            match g {
                0 | 3 => egui::CursorIcon::ResizeNwSe,
                1 | 2 => egui::CursorIcon::ResizeNeSw,
                4 | 6 => egui::CursorIcon::ResizeVertical,
                _ => egui::CursorIcon::ResizeHorizontal,
            }
        } else if done.contains(p) || cancel.contains(p) {
            egui::CursorIcon::PointingHand
        } else if k.contains(p) {
            egui::CursorIcon::Move
        } else {
            egui::CursorIcon::Default
        };
        ui.ctx().set_cursor_icon(icon);
    }
    let clicked = |r: Rect| allow_interaction && resp.clicked() && hover_pos.is_some_and(|p| r.contains(p));
    let keys = |key: egui::Key| allow_interaction && ui.input(|inp| inp.key_pressed(key)) && !ui.ctx().egui_wants_keyboard_input();
    if clicked(done) || keys(egui::Key::Enter) {
        let keep = mode.keep;
        if let Some(s) = view.image_selection.as_mut() {
            s.crop = None;
        }
        if keep_screen(keep) != b {
            view.pending_edit = Some(Edit::EditPageImage { page, index: i, change: pdfcraft_engine::ImageEdit::Crop(keep) });
        }
    } else if (clicked(cancel) || keys(egui::Key::Escape))
        && let Some(s) = view.image_selection.as_mut()
    {
        s.crop = None;
    }
    true
}

impl ImageSelection {
    pub(crate) fn dragging(&self) -> bool {
        self.drag.is_some()
    }
}

/// Once a selected corner is grabbed, text under it cannot take over the gesture. The same
/// applies after the pointer leaves the image/page while dragging.
pub(crate) fn image_owns_pointer(ui: &egui::Ui, xf: &PageXform, page: usize, info: &DocInfo, view: &DocView) -> bool {
    let Some(selection) = view.image_selection.as_ref().filter(|s| s.page == page) else { return false };
    if selection.drag.is_some() {
        return true;
    }
    let Some(image) = view.edit_images.get(&page).and_then(|(_, images)| images.get(selection.index)) else { return false };
    let Some(rect) = image.visible_rect else { return false };
    let b = xf.user_rect(info, page, rect.map(|v| v as f32));
    let p = ui.input(|i| if i.pointer.primary_down() { i.pointer.press_origin() } else { i.pointer.hover_pos() });
    p.is_some_and(|p| [b.left_top(), b.right_top(), b.left_bottom(), b.right_bottom()].iter().any(|c| c.distance(p) < 8.0))
}

/// What a drag on a paragraph box takes hold of.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Grip {
    Move,
    /// An edge (screen left or right), dragged to rewrap the paragraph.
    Left,
    Right,
}

/// A paragraph box being dragged: moved, or (from a handle on its left or right edge) resized.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockDrag {
    page: usize,
    block: usize,
    /// A paragraph index belongs to the document generation where the press was resolved.
    generation: u64,
    /// Where the drag started (screen).
    start: Pos2,
    grip: Grip,
}

/// How far an edge grip reaches either side of a box's edge (screen pixels).
const EDGE_REACH: f32 = 6.0;
/// How far outside an open paragraph's frame the pointer still means the frame (screen pixels).
const FRAME_REACH: f32 = 4.0;

/// The rewrap handles drawn on a paragraph box's left and right edges (screen).
fn width_handles(b: Rect) -> [Rect; 2] {
    let h = (b.height() * 0.5).clamp(14.0, 28.0);
    [b.left(), b.right()].map(|x| Rect::from_center_size(Pos2::new(x, b.center().y), egui::vec2(7.0, h)))
}

/// The edge of `b` that `p` grabs: anywhere along its height, within reach of the line. A narrow
/// box keeps its middle for moving.
fn edge_at(b: Rect, p: Pos2) -> Option<Grip> {
    if p.y < b.top() - EDGE_REACH || p.y > b.bottom() + EDGE_REACH {
        return None;
    }
    let inside = EDGE_REACH.min(b.width() / 4.0);
    let (dl, dr) = (p.x - b.left(), b.right() - p.x);
    match (dl >= -EDGE_REACH && dl <= inside, dr >= -EDGE_REACH && dr <= inside) {
        (true, true) if dl <= dr => Some(Grip::Left),
        (_, true) => Some(Grip::Right),
        (true, false) => Some(Grip::Left),
        _ => None,
    }
}

/// The topmost box under `p` and what it grabs there; edges only on an upright page.
fn grab_at(boxes: &[Rect], p: Pos2, upright: bool) -> Option<(usize, Grip)> {
    boxes.iter().enumerate().rev().find_map(|(i, b)| match upright.then(|| edge_at(*b, p)).flatten() {
        Some(g) => Some((i, g)),
        None => b.contains(p).then_some((i, Grip::Move)),
    })
}

/// What a right-click on a selected image asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ImageAction {
    Replace(usize, usize),
    Save(usize, usize),
    Copy(usize, usize),
    CopyFile(usize, usize),
    /// Copy the picture, then remove it from the page.
    Cut(usize, usize),
    /// Mark the picture for redaction (applied later from Redact a PDF).
    Redact(usize, usize),
    /// Create a link over the picture.
    Link(usize, usize),
    /// Open Recognize Text for the picture's page.
    Recognize(usize),
}

fn user_box(xf: &PageXform, info: &DocInfo, page: usize, r: Rect) -> [f64; 4] {
    let p = &info.pages[page];
    let (a, b) = (xf.screen_to_view(r.min), xf.screen_to_view(r.max));
    let (u, v) = (p.view_to_user(a.0, a.1), p.view_to_user(b.0, b.1));
    [u[0].min(v[0]) as f64, u[1].min(v[1]) as f64, u[0].max(v[0]) as f64, u[1].max(v[1]) as f64]
}

/// Images on a page: select, move, resize, right-click. Returns `true` when the pointer was used.
#[allow(clippy::too_many_arguments)]
pub(crate) fn image_input(
    ui: &egui::Ui,
    resp: &egui::Response,
    xf: &PageXform,
    page: usize,
    info: &DocInfo,
    images: &[pdfcraft_engine::PageImage],
    view: &mut DocView,
    action: &mut Option<ImageAction>,
    allow_interaction: bool,
) -> bool {
    let boxes: Vec<Rect> = images.iter().map(|im| im.visible_rect.map_or(Rect::NOTHING, |r| xf.user_rect(info, page, r.map(|v| v as f32)))).collect();
    let painter = ui.painter();
    let hover_pos = ui.input(|i| i.pointer.hover_pos());
    let image_contains = |index: usize, point: Pos2| {
        let (x, y) = xf.screen_to_view(point);
        let Some(p) = info.pages.get(page) else { return false };
        let user = p.view_to_user(x, y);
        images.get(index).is_some_and(|im| im.contains(user.map(f64::from)))
    };
    let selected = view.image_selection.as_ref().filter(|s| s.page == page).map(|s| s.index).filter(|i| *i < boxes.len());
    let dragging = view.image_selection.as_ref().is_some_and(|s| s.drag.is_some());
    // Few images: faint outlines for all of them, as before. Many: only the selected and
    // hovered boxes, so a ~1000-image CAD page stays interactive in Edit text & images.
    // Fully clipped placements (empty visible frames) get no outline.
    let faint_all = boxes.len() <= MAX_OUTLINES;
    if faint_all {
        for b in boxes.iter().filter(|b| b.is_positive()) {
            painter.rect_stroke(*b, CornerRadius::ZERO, Stroke::new(0.75, ACCENT.gamma_multiply(0.35)), egui::StrokeKind::Outside);
        }
    }
    let hovered_box = hover_pos.and_then(|p| boxes.iter().enumerate().rposition(|(i, b)| b.is_positive() && b.contains(p) && image_contains(i, p)));
    if !faint_all {
        for i in selected.into_iter().chain(hovered_box.filter(|h| Some(*h) != selected)) {
            if let Some(b) = boxes.get(i).filter(|b| b.is_positive()) {
                painter.rect_stroke(*b, CornerRadius::ZERO, Stroke::new(0.75, ACCENT.gamma_multiply(0.35)), egui::StrokeKind::Outside);
            }
        }
    }
    for i in selected.into_iter().chain(hovered_box).collect::<Vec<_>>() {
        let Some(b) = boxes.get(i).copied().filter(|b| b.is_positive()) else { continue };
        if selected == Some(i) {
            // The selected frame and handles are drawn below (or the drag preview instead).
            if !dragging {
                painter.rect_filled(b, CornerRadius::ZERO, ACCENT.gamma_multiply(0.04));
            }
        } else {
            painter.rect_stroke(b, CornerRadius::ZERO, Stroke::new(1.5, ACCENT.gamma_multiply(0.7)), egui::StrokeKind::Outside);
        }
    }
    // The selected image: frame, corner handles, dragging.
    if let Some(i) = selected.filter(|i| boxes.get(*i).is_some_and(|b| b.is_positive())) {
        let b = boxes[i];
        if view.image_selection.as_ref().is_some_and(|s| s.crop.is_some()) {
            return crop_input(ui, resp, xf, page, info, view, i, b, allow_interaction);
        }
        let corners = [b.left_top(), b.right_top(), b.left_bottom(), b.right_bottom()];
        let handles = |rect: Rect| {
            for c in [rect.left_top(), rect.right_top(), rect.left_bottom(), rect.right_bottom()] {
                painter.rect(
                    Rect::from_center_size(c, egui::vec2(8.0, 8.0)),
                    CornerRadius::ZERO,
                    Color32::WHITE,
                    Stroke::new(1.0, ACCENT),
                    egui::StrokeKind::Middle,
                );
            }
        };
        if view.image_selection.as_ref().is_some_and(|s| s.drag.is_none()) {
            painter.rect_stroke(b, CornerRadius::ZERO, Stroke::new(1.5, ACCENT), egui::StrokeKind::Outside);
            handles(b);
        }
        if allow_interaction && let Some(corner) = hover_pos.and_then(|p| corners.iter().position(|c| c.distance(p) < 8.0)) {
            ui.ctx().set_cursor_icon(if matches!(corner, 0 | 3) { egui::CursorIcon::ResizeNwSe } else { egui::CursorIcon::ResizeNeSw });
        }
        let origin = ui.input(|i| i.pointer.press_origin());
        if allow_interaction
            && resp.drag_started()
            && let Some(o) = origin
        {
            let corner = corners.iter().position(|c| c.distance(o) < 8.0);
            if corner.is_some() || image_contains(i, o) {
                let opposite = corner.map(|k| corners[3 - k]);
                if let Some(s) = view.image_selection.as_mut() {
                    s.drag = Some((o, opposite));
                }
            }
        }
        if let Some((start, opposite)) = view.image_selection.as_ref().and_then(|s| s.drag)
            && let Some(p) = hover_pos
        {
            let preview = match opposite {
                // Resize from the opposite corner, keeping the aspect ratio.
                Some(fixed) => {
                    let (w0, h0) = (b.width().max(1.0), b.height().max(1.0));
                    // Use the axis with the larger relative pointer change. Taking the maximum
                    // absolute size freezes shrinking when the other axis hasn't moved. Measure
                    // from the press point so grabbing a handle off-centre cannot cause a jump.
                    let horizontal = (p.x - start.x) * (start.x - fixed.x).signum() / w0;
                    let vertical = (p.y - start.y) * (start.y - fixed.y).signum() / h0;
                    let delta = if horizontal.abs() > vertical.abs() { horizontal } else { vertical };
                    let k = (1.0 + delta).max(0.05);
                    let (w, h) = (w0 * k, h0 * k);
                    let x = if start.x < fixed.x { fixed.x - w } else { fixed.x };
                    let y = if start.y < fixed.y { fixed.y - h } else { fixed.y };
                    Rect::from_min_size(Pos2::new(x, y), egui::vec2(w, h))
                }
                None => b.translate(p - start),
            };
            // Move/resize the full placement by the visible frame's change. The engine
            // transforms an isolated crop together with its image, preserving its contents.
            let raw = images.get(i).map_or(b, |im| xf.user_rect(info, page, im.rect.map(|v| v as f32)));
            // A zero-width or zero-height visible frame (or a degenerate ratio) keeps the
            // placement's scale instead of producing inf/NaN.
            let ratio = |new: f32, old: f32| if old.abs() > 1e-3 && (new / old).is_finite() { new / old } else { 1.0 };
            let factor = egui::vec2(ratio(preview.width(), b.width()), ratio(preview.height(), b.height()));
            let target = Rect::from_min_max(preview.min + (raw.min - b.min) * factor, preview.min + (raw.max - b.min) * factor);
            let visible = preview.intersect(xf.rect);
            if visible.is_positive() {
                // The pixels follow the frame: the page raster's picture of the image, stretched
                // into the preview over a blank where it was. Re-rendering every frame of a drag
                // would lag; the edit renders it properly on release.
                if preview != b
                    && let Some(tex) = view.page_raster(page)
                {
                    let clipped = painter.with_clip_rect(painter.clip_rect().intersect(xf.rect));
                    clipped.rect_filled(b, CornerRadius::ZERO, Color32::WHITE);
                    let mut mesh = egui::Mesh::with_texture(tex);
                    for (from, to) in [
                        (b.left_top(), preview.left_top()),
                        (b.right_top(), preview.right_top()),
                        (b.right_bottom(), preview.right_bottom()),
                        (b.left_bottom(), preview.left_bottom()),
                    ] {
                        let (u, v) = xf.screen_to_norm(from);
                        mesh.vertices.push(egui::epaint::Vertex { pos: to, uv: Pos2::new(u, v), color: Color32::WHITE });
                    }
                    mesh.add_triangle(0, 1, 2);
                    mesh.add_triangle(0, 2, 3);
                    clipped.add(egui::Shape::mesh(mesh));
                }
                painter.rect_filled(visible, CornerRadius::ZERO, ACCENT.gamma_multiply(0.04));
                painter.rect_stroke(visible, CornerRadius::ZERO, Stroke::new(1.5, ACCENT), egui::StrokeKind::Outside);
                handles(preview);
            }
            if resp.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
                if let Some(s) = view.image_selection.as_mut() {
                    s.drag = None;
                }
                if preview != b {
                    view.pending_edit =
                        Some(Edit::EditPageImage { page, index: i, change: pdfcraft_engine::ImageEdit::Move(user_box(xf, info, page, target)) });
                }
            }
            ui.ctx().set_cursor_icon(match opposite {
                Some(fixed) if (start.x - fixed.x) * (start.y - fixed.y) < 0.0 => egui::CursorIcon::ResizeNeSw,
                Some(_) => egui::CursorIcon::ResizeNwSe,
                None => egui::CursorIcon::Move,
            });
            return true;
        }
        if allow_interaction
            && ui.input(|inp| inp.key_pressed(egui::Key::Delete) || inp.key_pressed(egui::Key::Backspace))
            && !ui.ctx().egui_wants_keyboard_input()
        {
            view.image_selection = None;
            view.pending_edit = Some(Edit::EditPageImage { page, index: i, change: pdfcraft_engine::ImageEdit::Delete });
            return true;
        }
    }
    if !allow_interaction {
        return false;
    }
    // An open menu stays drawn wherever the pointer is: most of its items lie outside the picture
    // (a logo is smaller than its menu), and a menu that is not drawn for a frame cannot be clicked.
    // Only a menu opened by a right-click on the picture: one opened over text or empty page while
    // a picture stays selected is the page's own menu.
    let menu_key = resp.id.with("image-menu");
    if resp.secondary_clicked() {
        ui.ctx().data_mut(|d| d.insert_temp(menu_key, false));
    }
    let mut menu_drawn = false;
    if let Some(sel) = selected
        && resp.context_menu_opened()
        && ui.ctx().data(|d| d.get_temp::<bool>(menu_key)).unwrap_or(false)
    {
        resp.context_menu(|ui| image_menu(ui, page, sel, images, view, action));
        menu_drawn = true;
    }
    let Some(p) = hover_pos.filter(|p| xf.rect.contains(*p)) else { return false };
    let Some(hit) = boxes.iter().enumerate().rposition(|(i, b)| b.contains(p) && image_contains(i, p)) else { return false };
    if selected != Some(hit) {
        painter.rect_stroke(boxes[hit], CornerRadius::ZERO, Stroke::new(1.5, ACCENT.gamma_multiply(0.7)), egui::StrokeKind::Outside);
    }
    let Some(b) = boxes.get(hit).copied() else { return false };
    let corner = [b.left_top(), b.right_top(), b.left_bottom(), b.right_bottom()].iter().position(|c| c.distance(p) < 8.0);
    ui.ctx().set_cursor_icon(match (selected == Some(hit), corner) {
        (true, Some(0 | 3)) => egui::CursorIcon::ResizeNwSe,
        (true, Some(_)) => egui::CursorIcon::ResizeNeSw,
        (true, None) => egui::CursorIcon::Move,
        _ => egui::CursorIcon::PointingHand,
    });
    if resp.clicked() || resp.secondary_clicked() {
        view.image_selection = Some(ImageSelection { page, index: hit, drag: None, crop: None });
    }
    if resp.secondary_clicked() {
        ui.ctx().data_mut(|d| d.insert_temp(menu_key, true));
    }
    if selected == Some(hit) && !menu_drawn {
        resp.context_menu(|ui| image_menu(ui, page, hit, images, view, action));
    }
    true
}

/// The right-click menu of the picture `hit` (its rotate, flip, replace, copy, cut, redact, link,
/// recognize and delete items).
fn image_menu(
    ui: &mut egui::Ui,
    page: usize,
    hit: usize,
    images: &[pdfcraft_engine::PageImage],
    view: &mut DocView,
    action: &mut Option<ImageAction>,
) {
    use pdfcraft_engine::ImageEdit as E;
    let items: [(&str, Option<E>); 4] = [
        (tl!("Rotate Clockwise"), Some(E::Rotate(1))),
        (tl!("Rotate Counterclockwise"), Some(E::Rotate(3))),
        (tl!("Flip Horizontal"), Some(E::Flip { horizontal: true })),
        (tl!("Flip Vertical"), Some(E::Flip { horizontal: false })),
    ];
    for (label, change) in items {
        if crate::widgets::menu_entry(ui, true, label, None).clicked() {
            view.pending_edit = change.map(|c| Edit::EditPageImage { page, index: hit, change: c });
            ui.close();
        }
    }
    // Crop: the handles trim instead of resizing until Done.
    if crate::widgets::menu_entry(ui, true, tl!("Crop Image"), None).clicked() {
        if let (Some(s), Some(im)) = (view.image_selection.as_mut(), images.get(hit)) {
            s.crop = Some(CropMode::new(im.visible_rect.unwrap_or(im.rect)));
        }
        ui.close();
    }
    let cropped = images.get(hit).is_some_and(|im| im.visible_rect.is_some_and(|v| v.iter().zip(im.rect.iter()).any(|(a, b)| (a - b).abs() > 0.01)));
    if crate::widgets::menu_entry(ui, cropped, tl!("Remove Crop"), None).clicked() {
        if let Some(im) = images.get(hit) {
            view.pending_edit = Some(Edit::EditPageImage { page, index: hit, change: E::Crop(im.rect) });
        }
        ui.close();
    }
    let raster = images.get(hit).is_some_and(|image| !image.is_form);
    if crate::widgets::menu_entry(ui, raster, tl!("Replace Image…"), None).clicked() {
        *action = Some(ImageAction::Replace(page, hit));
        ui.close();
    }
    if crate::widgets::menu_entry(ui, raster, tl!("Save Image As…"), None).clicked() {
        *action = Some(ImageAction::Save(page, hit));
        ui.close();
    }
    // Cutting and copying also find the picture inside grouped Form artwork.
    if crate::widgets::menu_entry(ui, true, tl!("Cut Image"), None).clicked() {
        *action = Some(ImageAction::Cut(page, hit));
        ui.close();
    }
    if crate::widgets::menu_entry(ui, true, tl!("Copy Image"), None).clicked() {
        *action = Some(ImageAction::Copy(page, hit));
        ui.close();
    }
    if crate::widgets::menu_entry(ui, true, tl!("Copy Image in Original Format"), None).clicked() {
        *action = Some(ImageAction::CopyFile(page, hit));
        ui.close();
    }
    ui.separator();
    if crate::widgets::menu_entry(ui, true, tl!("Redact Image"), None).clicked() {
        *action = Some(ImageAction::Redact(page, hit));
        ui.close();
    }
    if crate::widgets::menu_entry(ui, true, tl!("Create Link…"), None).clicked() {
        *action = Some(ImageAction::Link(page, hit));
        ui.close();
    }
    if crate::widgets::menu_entry(ui, true, tl!("Recognize Text…"), None).clicked() {
        *action = Some(ImageAction::Recognize(page));
        ui.close();
    }
    ui.separator();
    if crate::widgets::menu_entry(ui, true, tl!("Delete"), None).clicked() {
        view.image_selection = None;
        view.pending_edit = Some(Edit::EditPageImage { page, index: hit, change: E::Delete });
        ui.close();
    }
}

/// Open `block` on `page` in the inline editor. The box's geometry and source are read from the
/// `lines` handed in, so callers pass the document's current inventory.
#[allow(clippy::too_many_arguments)]
fn open_editor(
    ui: &egui::Ui,
    xf: &PageXform,
    doc: &pdfcraft_engine::Document,
    lines: &[pdfcraft_engine::TextBlock],
    boxes: &[Rect],
    page: usize,
    block: usize,
    view: &mut DocView,
) {
    view.pending_text_open = None;
    let (Some(l), Some(b)) = (lines.get(block), boxes.get(block)) else { return };
    view.image_selection = None;
    let source_lines = doc.text_lines(page);
    let text = l.editor_text.clone();
    // Screen pixels per point, from the box's width.
    let scale = (b.width() - 4.0) / ((l.rect[2] - l.rect[0]).max(1.0) as f32);
    // How far the editor box may grow: a single-line paragraph's rewrite grows to the
    // page's right edge, a multi-line paragraph rewraps to its own width.
    let right = xf.rect.right().min(view.viewport_rect().right()) - 6.0;
    let max_width = if l.rows.len() == 1 { (right - b.left()).max(b.width()) } else { b.width() };
    view.line_editor = Some(LineEditor {
        selection: None,
        font_resource: None,
        page,
        block,
        text: text.clone(),
        original: text,
        rect: *b,
        source_rect: l.rect.map(|v| v as f32),
        multiline: l.rows.len() > 1,
        source: Some((l.clone(), source_lines)),
        fonts: install_source_fonts(ui.ctx(), doc, page),
        overflow: false,
        max_width,
        size: (l.size as f32 * scale).clamp(8.0, 72.0),
        focus: true,
        look: look_of(l),
        look0: look_of(l),
        extras: Extras::default(),
        extras0: Extras::default(),
        probe: doc.text_draft_probe(page, block).map(DraftProbe::new),
    });
}

/// Lines and their screen boxes for a page; hover outlines, click opens the editor. Returns
/// `true` when the pointer was used.
#[allow(clippy::too_many_arguments)]
pub(crate) fn page_input(
    ui: &egui::Ui,
    resp: &egui::Response,
    xf: &PageXform,
    page: usize,
    doc: &pdfcraft_engine::Document,
    lines: &[pdfcraft_engine::TextBlock],
    view: &mut DocView,
    allow_interaction: bool,
) -> bool {
    let info = &doc.info;
    if view.block_drag.is_some_and(|d| d.generation != doc.edit_generation()) {
        view.block_drag = None;
        return true;
    }
    let boxes: Vec<Rect> = lines.iter().map(|l| xf.user_rect(info, page, l.rect.map(|v| v as f32)).expand(2.0)).collect();
    let painter = ui.painter();
    let active = view.line_editor.as_ref().filter(|e| e.page == page).map(|e| e.block);
    for (i, b) in boxes.iter().enumerate() {
        if active != Some(i) {
            painter.rect_stroke(*b, CornerRadius::same(2), Stroke::new(0.75, ACCENT.gamma_multiply(0.35)), egui::StrokeKind::Outside);
        }
    }
    // A box a switching click asked for opens now, once the edit queued with that click has
    // applied and `lines`/`boxes` are the document's new inventory (#2022).
    if allow_interaction
        && active.is_none()
        && let Some((open_page, block)) = view.pending_text_open
        && open_page == page
    {
        open_editor(ui, xf, doc, lines, &boxes, page, block, view);
    }
    if !allow_interaction {
        return false;
    }
    // The width handle needs the page upright (or upside down): on a quarter-turned page the
    // screen's horizontal is the paragraph's vertical.
    let upright = info.pages.get(page).is_some_and(|p| p.rotation % 180 == 0);
    let draw_handles = |b: Rect| {
        for h in width_handles(b) {
            painter.rect(h, CornerRadius::same(2), Color32::WHITE, Stroke::new(1.0, ACCENT), egui::StrokeKind::Middle);
        }
    };
    // A drag in progress: the box follows the pointer (or the grabbed edge does); releasing applies it.
    if let Some(d) = view.block_drag.filter(|d| d.page == page)
        && let (Some(b), Some(l)) = (boxes.get(d.block).copied(), lines.get(d.block))
    {
        let p = ui.input(|i| i.pointer.interact_pos()).unwrap_or(d.start);
        let dx = p.x - d.start.x;
        let preview = match d.grip {
            Grip::Move => b.translate(p - d.start),
            Grip::Left => Rect::from_min_max(Pos2::new((b.left() + dx).min(b.right() - 12.0), b.min.y), b.max),
            Grip::Right => Rect::from_min_max(b.min, Pos2::new((b.right() + dx).max(b.left() + 12.0), b.max.y)),
        };
        painter.rect_stroke(preview, CornerRadius::same(2), Stroke::new(1.5, ACCENT), egui::StrokeKind::Outside);
        let resizing = d.grip != Grip::Move;
        if resizing {
            draw_handles(preview);
        }
        ui.ctx().set_cursor_icon(if resizing { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::Move });
        if resp.drag_stopped() || !ui.input(|i| i.pointer.any_down()) {
            view.block_drag = None;
            let (from, to) = (user_box(xf, info, page, b), user_box(xf, info, page, preview));
            let edit = if resizing {
                // The new width in user space: the box's change, added to the paragraph's own. A
                // moved left side (in user space; on an upside-down page that's the screen's right
                // edge) moves the paragraph with it.
                let width = (l.rect[2] - l.rect[0]) + (to[2] - to[0]) - (from[2] - from[0]);
                let shift = to[0] - from[0];
                let style = pdfcraft_engine::BlockStyle { width: Some(width), offset: (shift != 0.0).then_some([shift, 0.0]), ..Default::default() };
                Edit::EditTextBlock { page, block: d.block, text: l.text.clone(), style }
            } else {
                // Translation must keep the selected source operators. Rewriting unchanged text
                // would also remove unrelated paragraphs that now overlap this one (#594).
                Edit::MoveObjects {
                    page,
                    objects: vec![ObjectTarget { kind: ObjectKind::Text, index: d.block }],
                    offset: [to[0] - from[0], to[1] - from[1]],
                }
            };
            if preview != b {
                view.pending_edit = Some(edit);
            }
        }
        return true;
    }
    // Dragging a box (not while a paragraph is open for typing) moves it; from an edge, resizes it.
    // What it grabs comes from where the button went down, not where the pointer is now: egui
    // only calls it a drag once the pointer has moved a few pixels (or been held a moment), and a
    // quick flick has left the edge by then.
    if resp.drag_started()
        && active.is_none()
        && let Some(o) = ui.input(|i| i.pointer.press_origin())
        && let Some((block, grip)) = grab_at(&boxes, o, upright)
    {
        view.block_drag = Some(BlockDrag { page, block, generation: doc.edit_generation(), start: o, grip });
        return true;
    }
    let Some(p) = ui.input(|i| i.pointer.hover_pos()).filter(|p| xf.rect.contains(*p)) else { return false };
    let Some((hit, grip)) = grab_at(&boxes, p, upright) else { return false };
    if active != Some(hit) {
        painter.rect_stroke(boxes[hit], CornerRadius::same(2), Stroke::new(1.5, ACCENT), egui::StrokeKind::Outside);
    }
    let on_edge = active.is_none() && grip != Grip::Move;
    if upright && active.is_none() {
        draw_handles(boxes[hit]);
    }
    // A closed paragraph moves when dragged (the four-way arrow says so; a click still opens it
    // for typing); an edge rewraps it. The open one takes the text cursor.
    ui.ctx().set_cursor_icon(if on_edge {
        egui::CursorIcon::ResizeHorizontal
    } else if active.is_none() {
        egui::CursorIcon::Move
    } else {
        egui::CursorIcon::Text
    });
    if resp.clicked() {
        view.image_selection = None;
        // A click that moves the editor to another paragraph applies the open one first, so its
        // typed text is not dropped with the switched-away editor (#2022). A click on the open
        // box's own frame, which the editor's overlay does not cover, keeps it open instead.
        if let Some(open) = view.line_editor.as_ref().filter(|e| e.has_changes()) {
            if open.page == page && open.block == hit {
                return true;
            }
            if let Some(old) = view.line_editor.take() {
                let style = old.style();
                if old.text != old.original || style != pdfcraft_engine::BlockStyle::default() {
                    view.pending_edit = Some(Edit::EditTextBlock { page: old.page, block: old.block, text: old.text, style });
                }
            }
            // The clicked box opens next frame, from the document with that edit applied.
            view.pending_text_open = Some((page, hit));
            return true;
        }
        open_editor(ui, xf, doc, lines, &boxes, page, hit, view);
    }
    true
}

/// One TextEdit lays out all source runs; its caret and selection span the whole region.
pub(crate) fn install_source_fonts(ctx: &egui::Context, doc: &pdfcraft_engine::Document, page: usize) -> BTreeMap<String, String> {
    let programs = doc.text_editor_fonts(page);
    let mut definitions = ctx.fonts(|f| f.definitions().clone());
    // Only the current editor's document fonts live in the atlas. Never accumulate the
    // programs of previously opened files or add them to the interface's default families.
    definitions.font_data.retain(|name, _| !name.starts_with("pdf-source-"));
    definitions.families.retain(|family, _| !matches!(family, FontFamily::Name(name) if name.starts_with("pdf-source-")));
    let mut families = BTreeMap::new();
    for (index, program) in programs.iter().enumerate() {
        let name = format!("pdf-source-{index}");
        definitions.font_data.insert(name.clone(), Arc::new(egui::FontData::from_owned(program.bytes.clone())));
        families.insert(program.resource.clone(), name);
    }
    for program in &programs {
        let Some(name) = families.get(&program.resource) else { continue };
        // Word-created PDFs often split one face into several subsets. Use all of the
        // document's matching subsets before falling back for newly inserted characters.
        let mut stack = vec![name.clone()];
        stack.extend(
            programs
                .iter()
                .filter(|p| {
                    pdfcraft_fonts::embedding::face_key(&p.face) == pdfcraft_fonts::embedding::face_key(&program.face)
                        && p.resource != program.resource
                })
                .filter_map(|p| families.get(&p.resource).cloned()),
        );
        if let Some(fallback) = definitions.families.get(&FontFamily::Proportional) {
            stack.extend(fallback.iter().cloned());
        }
        definitions.families.insert(FontFamily::Name(name.clone().into()), stack);
    }
    if !families.is_empty() {
        ctx.set_fonts(definitions);
    }
    families
}
/// The look source line `line` is drawn in, with the Format text panel's changes applied.
fn line_look(ed: &LineEditor, line: &pdfcraft_engine::TextLine) -> pdfcraft_engine::AddedText {
    let mut look = source_look(&line.base_font, line.size, line.color, line.bold, line.italic);
    look.size = line.size;
    if ed.look.family != ed.look0.family {
        look.family = ed.look.family;
    }
    if ed.look.bold != ed.look0.bold {
        look.bold = ed.look.bold;
    }
    if ed.look.italic != ed.look0.italic {
        look.italic = ed.look.italic;
    }
    if ed.look.color != ed.look0.color {
        look.color = ed.look.color;
    }
    if ed.look.size != ed.look0.size {
        look.size = ed.look.size;
    }
    look
}

/// The two ways a source line is drawn in the editor: for ordinary text, and for the letters of a script
/// that joins (Arabic), which the page writes in a face of its own, with no letter spacing.
struct LineFormats {
    plain: egui::TextFormat,
    joined: egui::TextFormat,
}

/// The formats of source line `line`. `rtl_script`: the draft holds right-to-left script, which the page writes
/// in a face of its own and without the letter spacing that stretches the interface face to the width of the
/// line it replaces. `page_face`: the family of the page's face that draws the joined letters of the line (see
/// [`LineEditor::page_faces`]), or the interface's Arabic face where there is none.
fn line_formats(
    ctx: &egui::Context,
    ed: &LineEditor,
    line: &pdfcraft_engine::TextLine,
    scale: f32,
    rtl_script: bool,
    page_face: Option<&String>,
) -> LineFormats {
    let look = line_look(ed, line);
    let mut font = editor_font(&look, (look.size as f32 * scale).clamp(1.0, 300.0));
    // Joined letters are drawn by the interface's Arabic face (the one the page's shaper uses): a source
    // subset may carry the letters' codes without the shaping that joins them. Where Apply shapes the draft in the page's own
    // face (it is a complete face with the letters), they are drawn in that face: the rows are then as wide as the page's.
    let mut joined_font = font.clone();
    if let Some(name) = page_face {
        joined_font.family = FontFamily::Name(name.clone().into());
    }
    let original_face = ed.look.family == ed.look0.family && ed.look.bold == ed.look0.bold && ed.look.italic == ed.look0.italic;
    let embedded = original_face && ed.fonts.contains_key(&line.font);
    if embedded && let Some(name) = ed.fonts.get(&line.font) {
        font.family = FontFamily::Name(name.clone().into());
    }
    let spacing = if rtl_script {
        0.0
    } else {
        let native_width = ctx.fonts_mut(|f| f.layout_no_wrap(line.text.clone(), font.clone(), Color32::BLACK).size().x);
        let target_width = ((line.rect[2] - line.rect[0]) * look.size / line.size.max(0.01)) as f32 * scale;
        let fit = if original_face { (target_width - native_width) / line.text.chars().count().saturating_sub(1).max(1) as f32 } else { 0.0 };
        fit + ed.extras.char_spacing as f32 * scale
    };
    LineFormats {
        plain: egui::TextFormat {
            font_id: font,
            extra_letter_spacing: spacing,
            color: color32(look.color),
            italics: look.italic && !embedded,
            ..Default::default()
        },
        joined: egui::TextFormat { font_id: joined_font, color: color32(look.color), ..Default::default() },
    }
}

/// The source line whose look each character of the shaper text of `map` takes (see [`style_runs`]).
///
/// Every character of the stored `text` has the look of one source line (`spans`, from
/// [`pdfcraft_engine::TextBlock::edited_spans`]); the shaper text has them in another order, so each of its
/// characters takes the look of the stored character it was drawn from (`ScreenMap::shaper_source`). The line
/// break a wrapped text makes belongs to no character and takes the look of the one before it (the first one
/// of the text, of the one after). A look never changes inside a stretch of right-to-left script, which the
/// text engine shapes as a whole: the stretch takes the look of its first letter.
fn shaper_styles(spans: &[(std::ops::Range<usize>, usize)], text: &str, map: &ScreenMap) -> Vec<usize> {
    let fallback = spans.first().map_or(0, |(_, line)| *line);
    let mut stored: Vec<usize> = Vec::with_capacity(map.logical_len());
    for (range, line) in spans {
        let count = text.get(range.clone()).map_or(0, |piece| piece.chars().count());
        stored.extend(std::iter::repeat_n(*line, count));
    }
    let mut styles: Vec<Option<usize>> = map
        .shaper_source()
        .into_iter()
        .map(|at| if at == bidi::VIRTUAL { None } else { Some(stored.get(at).copied().unwrap_or(fallback)) })
        .collect();
    // A look never changes inside a stretch of right-to-left script: its letters take their first one's.
    let mut previous: Option<(char, usize)> = None;
    for (c, style) in map.shaper.chars().zip(styles.iter_mut()) {
        if let (Some(line), Some((before, first))) = (style.as_mut(), previous)
            && bidi::is_shaped_rtl(before)
            && bidi::is_shaped_rtl(c)
        {
            *line = first;
        }
        previous = style.map(|line| (c, line));
    }
    // A break takes the look of the character before it.
    let mut before = None;
    for style in &mut styles {
        match *style {
            Some(line) => before = Some(line),
            None => *style = before,
        }
    }
    // A break at the very start takes that of the character after it.
    let mut after = None;
    for style in styles.iter_mut().rev() {
        match *style {
            Some(line) => after = Some(line),
            None => *style = after,
        }
    }
    styles.into_iter().map(|line| line.unwrap_or(fallback)).collect()
}

/// The shaper text of `map` cut into runs of one look: `(source line, joins, text)`. The letters of a script
/// that joins are a run of their own, as they are drawn in another face than the characters around them.
fn style_runs(spans: &[(std::ops::Range<usize>, usize)], text: &str, map: &ScreenMap) -> Vec<(usize, bool, String)> {
    let mut runs: Vec<(usize, bool, String)> = Vec::new();
    for (c, line) in map.shaper.chars().zip(shaper_styles(spans, text, map)) {
        let joins = bidi::is_joining_script(c);
        match runs.last_mut() {
            Some((last, was, run)) if *last == line && *was == joins => run.push(c),
            _ => runs.push((line, joins, c.to_string())),
        }
    }
    runs
}

/// Whether nothing in the editor differs from the paragraph as the page has it, `text` being the draft.
fn is_pristine(ed: &LineEditor, text: &str) -> bool {
    text == ed.original && ed.look == ed.look0 && ed.extras == ed.extras0
}

/// The job that lays out `map`, the draft `text` as it is put on screen, in the looks of the page's text.
/// `wrap` is where egui breaks a line; a text the field has already broken into lines is given `INFINITY`.
fn editor_job(ctx: &egui::Context, ed: &LineEditor, text: &str, map: &ScreenMap, scale: f32, wrap: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = wrap;
    if let Some((block, lines)) = &ed.source {
        let rtl_script = text.chars().any(bidi::is_shaped_rtl);
        let faces = ed.page_faces(text);
        let mut formats: BTreeMap<usize, LineFormats> = BTreeMap::new();
        for (index, joins, run) in style_runs(&block.edited_spans(text), text, map) {
            let Some(line) = lines.get(index) else {
                job.append(
                    &run,
                    0.0,
                    egui::TextFormat { font_id: editor_font(&ed.look, ed.size), color: color32(ed.look.color), ..Default::default() },
                );
                continue;
            };
            let format = formats.entry(index).or_insert_with(|| line_formats(ctx, ed, line, scale, rtl_script, faces.get(&index)));
            job.append(&run, 0.0, if joins { format.joined.clone() } else { format.plain.clone() });
        }
    } else {
        job.append(
            &map.shaper,
            0.0,
            egui::TextFormat {
                font_id: editor_font(&ed.look, ed.size),
                color: color32(ed.look.color),
                italics: ed.look.italic,
                ..Default::default()
            },
        );
    }
    job
}

/// Whether the rows of the block are centred on its middle (a centred title or paragraph).
fn is_centered(block: &pdfcraft_engine::TextBlock, lines: &[pdfcraft_engine::TextLine]) -> bool {
    let lefts: Vec<_> = block.rows.iter().filter_map(|r| r.first().and_then(|i| lines.get(*i)).map(|l| l.rect[0])).collect();
    let left_span = lefts.iter().copied().fold(f64::NEG_INFINITY, f64::max) - lefts.iter().copied().fold(f64::INFINITY, f64::min);
    block.rows.len() > 1
        && left_span > block.size * 0.5
        && block.rows.iter().all(|row| {
            let rect = row
                .iter()
                .filter_map(|i| lines.get(*i))
                .map(|l| l.rect)
                .reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]);
            rect.is_some_and(|r| ((r[0] + r[2] - block.rect[0] - block.rect[2]) / 2.0).abs() < block.size)
        })
}

/// Where the rows of the paragraph are on the page: each source row's baseline and box, and the leading
/// that places the rows the draft adds below them.
struct RowGeometry {
    sources: Vec<(f64, [f64; 4])>,
    leading: f64,
}

/// What a row of the editor is placed by.
struct RowSource {
    /// The baseline the row sits on.
    baseline: f64,
    /// The box of the source row it stands for (the last one's for a row the draft adds).
    rect: [f64; 4],
    /// The baseline the row's height above and below it is measured from.
    extent: f64,
}

impl RowGeometry {
    /// `None` when the paragraph has no row.
    fn new(ed: &LineEditor, block: &pdfcraft_engine::TextBlock, lines: &[pdfcraft_engine::TextLine], text: &str) -> Option<Self> {
        let sources: Vec<_> = block
            .rows
            .iter()
            .filter_map(|row| {
                let first = row.first().and_then(|i| lines.get(*i))?;
                let rect = row
                    .iter()
                    .filter_map(|i| lines.get(*i))
                    .map(|l| l.rect)
                    .reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])?;
                Some((first.baseline()[1], rect))
            })
            .collect();
        sources.first()?;
        let leading = if ed.extras.line_spacing > 0.0 {
            ed.extras.line_spacing * ed.look.size
        } else {
            let fallback = if block.lines.len() == 1 && !text.contains('\n') {
                block.lines.first().and_then(|i| lines.get(*i)).map_or(block.size * 1.2, |line| line.wrap_leading())
            } else {
                block.size * 1.5
            };
            sources.windows(2).map(|r| (r[0].0 - r[1].0).abs()).rfind(|v| *v > 0.0).unwrap_or(fallback)
        };
        Some(Self { sources, leading })
    }

    fn at(&self, index: usize) -> RowSource {
        let (last_baseline, last_rect) = self.sources.last().copied().unwrap_or_default();
        match self.sources.get(index) {
            Some(&(baseline, rect)) => RowSource { baseline, rect, extent: baseline },
            None => RowSource {
                baseline: last_baseline - index.saturating_add(1).saturating_sub(self.sources.len()) as f64 * self.leading,
                rect: last_rect,
                extent: last_baseline,
            },
        }
    }
}

/// `galley` with the spaces of each row stretched to the width of the row Apply will write: a justified paragraph fills its
/// width with every row but the last, by widening the spaces. The widths are the rewrite's own (`row_rects` of its layout), so
/// a paragraph the rewrite does not justify (the styled route that writes a draft with explicit row breaks sets its rows
/// left-aligned) stays as it is, and so does a draft the rewrite refuses. The caret and clicks follow the letters: the
/// glyphs after a space move on with it (see [`LayoutCtx::stretch_rows`]).
fn justify_like_the_page(ed: &LineEditor, ctx: &LayoutCtx<'_>, galley: Arc<egui::Galley>, map: &ScreenMap, scale: f32) -> Arc<egui::Galley> {
    let Some(layout) = ed.layout_of(ctx.text) else { return galley };
    // The rows that draw anything, one to a row of the page.
    let drawn: Vec<usize> =
        galley.rows.iter().enumerate().filter(|(_, placed)| placed.row.glyphs.iter().any(|g| g.chr != ' ')).map(|(index, _)| index).collect();
    if drawn.len() != layout.row_rects.len() {
        return galley;
    }
    ctx.stretch_rows(galley, map, |slot| {
        let at = drawn.iter().position(|index| *index == slot.index)?;
        let rect = layout.row_rects.get(at)?;
        // (the last row of the paragraph is not stretched by the page)
        (at + 1 < drawn.len()).then(|| ((rect[2] - rect[0]) * f64::from(scale)) as f32)
    })
}

/// The galley of the draft `ctx.text`, shown as `map` says, in the looks of the page's text, with every row
/// on its source row's baseline instead of at egui's interface leading, glyphs, meshes, caret and
/// selection receiving the same transform. A row that reads right to left ends where the page will
/// write it: at the right edge of the source row while nothing has changed, at the right edge of the box
/// (or where the alignment asked for in the Format text panel puts it) after.
fn editor_galley(ui: &egui::Ui, ed: &LineEditor, ctx: &LayoutCtx<'_>, map: &ScreenMap, scale: f32) -> Arc<egui::Galley> {
    let text = ctx.text;
    // The field has broken the lines already; an unchanged PDF row has an authoritative line break of its own
    // and is not wrapped again because font rounding measures a fraction of a pixel wider.
    let wrap = if ctx.prewrapped || is_pristine(ed, text) { f32::INFINITY } else { ctx.wrap_width.unwrap_or(f32::INFINITY) };
    let mut job = editor_job(ui.ctx(), ed, text, map, scale, wrap);
    // The rows of lines the field wrapped keep the spaces at their ends, as the field's own layout does.
    job.keep_trailing_whitespace = ctx.prewrapped;
    let mut galley = ctx.lay_out(ui, map, job);
    if ed.look.align == pdfcraft_engine::TextAlign::Justify && ed.look.align != ed.look0.align {
        galley = justify_like_the_page(ed, ctx, galley, map, scale);
    }
    let Some((block, lines)) = &ed.source else { return ctx.place_rows(ui, galley, map) };
    let Some(geometry) = RowGeometry::new(ed, block, lines, text) else { return ctx.place_rows(ui, galley, map) };
    let width = ctx.wrap_width.unwrap_or_else(|| galley.rect.width());
    let centered = is_centered(block, lines);
    if map.row_rtl.iter().any(|rtl| *rtl) {
        let pristine = is_pristine(ed, text);
        let align = (ed.look.align != ed.look0.align).then_some(ed.look.align);
        galley = ctx.place_rows_with(ui, galley, map, |slot: RowSlot| {
            if !slot.rtl {
                return None;
            }
            let flush = slot.field_width - slot.width;
            Some(match (pristine, align) {
                // As the page has it: against the right edge of the source row.
                (true, _) => (((geometry.at(slot.index).rect[2] - block.rect[0]) as f32 * scale) - slot.width).min(flush),
                (false, Some(pdfcraft_engine::TextAlign::Left)) => 0.0,
                (false, Some(pdfcraft_engine::TextAlign::Center)) => flush / 2.0,
                // Right-to-left text is right-aligned unless an alignment is asked for (and a justified
                // row, which a screen cannot stretch, is flush against the edge it starts from).
                (false, _) => flush,
            })
        });
    }
    let changed_size = ed.look.size != ed.look0.size;
    let ratio = if changed_size { ed.look.size / ed.look0.size.max(0.01) } else { 1.0 };
    let g = Arc::make_mut(&mut galley);
    let mut bounds = Rect::NOTHING;
    let mut meshes = Rect::NOTHING;
    for (index, placed) in g.rows.iter_mut().enumerate() {
        let source = geometry.at(index);
        let ascent = (source.rect[3] - source.extent) as f32 * scale * ratio as f32;
        let descent = (source.extent - source.rect[1]) as f32 * scale * ratio as f32;
        let base_y = (block.rect[3] - source.baseline) as f32 * scale;
        let rtl = map.row_is_rtl(index);
        let row = Arc::make_mut(&mut placed.row);
        placed.pos.y = base_y - ascent;
        if !rtl {
            placed.pos.x = if centered { (width - row.size.x) / 2.0 } else { (source.rect[0] - block.rect[0]) as f32 * scale };
        }
        let mut dy = Vec::with_capacity(row.glyphs.len());
        for glyph in &mut row.glyphs {
            dy.push(egui::vec2(0.0, ascent - glyph.pos.y));
            glyph.pos.y = ascent;
            glyph.font_ascent = ascent;
            glyph.font_height = ascent + descent;
            glyph.line_height = ascent + descent;
        }
        widgets::shift_glyph_vertices(row, &dy);
        row.size.y = (ascent + descent).max(1.0);
        row.visuals.mesh_bounds = row.visuals.mesh.vertices.iter().fold(Rect::NOTHING, |r, v| r.union(Rect::from_min_max(v.pos, v.pos)));
        bounds = bounds.union(Rect::from_min_size(placed.pos, row.size));
        meshes = meshes.union(row.visuals.mesh_bounds.translate(placed.pos.to_vec2()));
    }
    g.rect = Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(bounds.right().max(0.0), bounds.bottom().max(0.0)));
    g.mesh_bounds = meshes;
    galley
}

/// The map of the draft `text` as the editor lays it out in one piece (not wrapped), in the direction the
/// page will write it.
fn unwrapped_map(text: &str) -> ScreenMap {
    if bidi::has_rtl(text) {
        bidi::wrapped_screen_map(text, crate::rtl::mirrored(), Some(pdfcraft_engine::rtl_base(text)), |_| true)
    } else {
        ScreenMap::identity(text)
    }
}

/// How wide the draft would be on one line.
fn natural_width(ctx: &egui::Context, ed: &LineEditor, scale: f32) -> f32 {
    let job = editor_job(ctx, ed, &ed.text, &unwrapped_map(&ed.text), scale, f32::INFINITY);
    ctx.fonts_mut(|f| f.layout_job(job)).size().x
}

/// How the page will wrap a line of the draft: the width of a line in the first source row's font and size,
/// which is how the page writes a block that reads right to left (in the first row's look).
fn line_measure(ed: &LineEditor, scale: f32) -> Option<impl Fn(&egui::Context, &str) -> f32 + 'static> {
    let (block, lines) = ed.source.as_ref()?;
    let first = block.lines.first().and_then(|i| lines.get(*i))?;
    let look = line_look(ed, first);
    let font = editor_font(&look, (look.size as f32 * scale).clamp(1.0, 300.0));
    Some(move |ctx: &egui::Context, line: &str| {
        ctx.fonts_mut(|f| {
            let galley = f.layout_no_wrap(line.to_owned(), font.clone(), Color32::WHITE);
            galley.rows.first().map_or(0.0, |row| row.glyphs.iter().map(|g| g.pos.x + g.advance_width).fold(0.0, f32::max))
        })
    })
}

/// The selection `chars`, a range of the draft's characters (what `BidiOutput::logical_range`
/// says), as the byte range the Format text panel and the engine count in. `None` for a caret (or nothing).
fn selection_bytes(text: &str, chars: std::ops::Range<usize>) -> Option<std::ops::Range<usize>> {
    let byte = |index: usize| text.char_indices().nth(index).map_or(text.len(), |(at, _)| at);
    let range = byte(chars.start)..byte(chars.end);
    (!range.is_empty()).then_some(range)
}

/// Where the editor breaks lines of the draft `text` in a box `width` wide: not at all while the paragraph is
/// as it was (it already has its line breaks), at the box's width after.
fn editor_wrap(ed: &LineEditor, text: &str, width: f32) -> f32 {
    if is_pristine(ed, text) { f32::INFINITY } else { width }
}

/// The field the paragraph is edited in: `text` is the draft and `ed` the editor as it is before the frame's
/// typing, `width` the box's width and `wrap` where its lines break. Right-to-left script is edited in display order (see [`widgets::BidiEdit`]) and the hook lays the
/// text out in the looks of the page's text, in whichever order it is put on screen.
fn text_field<'t>(text: &'t mut String, ed: LineEditor, scale: f32, width: f32, wrap: f32, font: FontId, color: Color32) -> widgets::BidiEdit<'t> {
    // The direction the page writes the block in: the one most of its letters read in.
    let rtl = pdfcraft_engine::rtl_base(text);
    // Lines break where the page will break them (`wrap`), measured in the font it writes a block in.
    let measure = line_measure(&ed, scale);
    // Once the paragraph has changed, the rows the page will lay it out in are asked of the engine (the rewrite Apply
    // runs), for text of either direction: that is where the page breaks it, in whatever fonts it is set in.
    let engine_rows = ed.probe.is_some() && wrap.is_finite() && !is_pristine(&ed, text);
    let rows = ed.probe.clone().map(|probe| {
        let style = ed.style();
        move |_: &egui::Context, text: &str| {
            probe.layout(text, &style).map(|layout| layout.rows.iter().map(|row| row.split_whitespace().count()).collect::<Vec<usize>>())
        }
    });
    let key = format!("{:?}{}", ed.style(), scale.to_bits());
    let field = widgets::BidiEdit::multiline(text)
        .id(egui::Id::new("edit-text-line-input"))
        .font(font)
        .margin(egui::Margin::ZERO)
        .text_color(color)
        .frame(egui::Frame::NONE)
        .desired_width(width)
        .desired_rows(1)
        .wrap_at(wrap)
        .forced_rtl(Some(rtl))
        .wrap_ltr(engine_rows)
        .wrap_key(key)
        .layout(move |ui, map, ctx| editor_galley(ui, &ed, ctx, map, scale));
    let field = match measure {
        Some(measure) => field.measure(measure),
        None => field,
    };
    match rows {
        Some(rows) => field.row_words(rows),
        None => field,
    }
}

/// The inline editor; returns the edit once the text is applied.
pub(crate) fn overlay(ctx: &egui::Context, view: &mut DocView, info: &DocInfo) -> Option<Edit> {
    // Clicks outside the document (the Format text panel) keep the paragraph open.
    let outside = ctx.input(|i| i.pointer.latest_pos()).is_some_and(|p| !view.viewport_rect().contains(p));
    let page = view.line_editor.as_ref()?.page;
    let xf = view.page_xform(page)?;
    let hold = view.objects.hold_editor;
    // An OCR word is invisible text over a scan: the textless background still shows the scanned
    // word, so the editor covers it, as the applied edit will.
    let background_ready = view
        .line_editor
        .as_ref()
        .is_some_and(|ed| view.text_background.ready(ed.page, ed.block) && !ed.source.as_ref().is_some_and(|(b, _)| b.invisible));
    let draft_rect = view.text_background.draft_rect.map(|r| xf.user_rect(info, page, r.map(|v| v as f32)));
    let ed = view.line_editor.as_mut()?;
    if hold {
        ed.focus = true;
    }
    // Reproject the source box every frame. The page may have been zoomed, scrolled or rotated
    // while the format panel was open.
    // A font definition becomes active at the start of the next egui pass. Keep the source
    // raster visible until then instead of flashing an unrelated interface face.
    if !ctx.fonts(|f| ed.fonts.values().all(|name| f.definitions().families.contains_key(&FontFamily::Name(name.clone().into())))) {
        ctx.request_repaint();
        return None;
    }
    ed.rect = xf.user_rect(info, ed.page, ed.source_rect);
    ed.max_width = ed.rect.width();
    let scale = (ed.rect.width() / (ed.source_rect[2] - ed.source_rect[0]).abs().max(0.01)).max(0.01);
    ed.size = (ed.look.size as f32 * scale).clamp(1.0, 300.0);
    // egui draws the caret at least one row of this font tall. Each row already has its source
    // row's height, so the font stays below every row (half the region's smallest size): on a
    // 12 pt row under a 24 pt title the caret is as tall as the row, not the title.
    let smallest =
        ed.source.as_ref().and_then(|(block, lines)| block.lines.iter().filter_map(|i| lines.get(*i)).map(|l| l.size as f32).reduce(f32::min));
    let font = editor_font(&ed.look, smallest.map_or(ed.size, |s| (s * scale * 0.5).clamp(1.0, 300.0).min(ed.size)));
    let text_color = color32(ed.look.color);
    // A line that reads right to left keeps its right edge as it grows, as the page does it.
    let anchored_right = pdfcraft_engine::rtl_base(&ed.text);
    // The editor opens at the source size. Once text changes, show the real draft extent: single rows grow
    // toward the page edge (to the left for a line that reads right to left), paragraphs rewrap at their source
    // width, and the visible frame grows/shrinks vertically with their rows. An OCR phrase (several stretched
    // words on one scanned line) also grows rightwards.
    let single_run = ed.source.as_ref().is_some_and(|(block, _)| block.rows.len() == 1 || block.invisible);
    let (width, wrap) = if !ed.multiline && single_run && !ed.text.contains('\n') && ed.has_changes() {
        let room = if anchored_right { ed.rect.right() - xf.rect.left() } else { xf.rect.right() - ed.rect.left() } - 36.0 * scale;
        let (natural, cap) = (natural_width(ctx, ed, scale), room.max(ed.rect.width()));
        let width = natural.max(ed.size * 0.4).min(cap);
        // The line grows to its natural width: it breaks only where it reaches the page's margin.
        (width, if natural <= cap { f32::INFINITY } else { width })
    } else {
        let width = ed.rect.width().max(1.0);
        (width, editor_wrap(ed, &ed.text, width))
    };
    // (and keeps its right edge where the page's paragraph has it, whatever the width of the text)
    let origin = Pos2::new(if anchored_right { ed.rect.right() - width } else { ed.rect.left() }, ed.rect.top());
    let mut done = None;
    egui::Area::new(egui::Id::new("edit-text-line")).order(egui::Order::Foreground).fixed_pos(origin).show(ctx, |ui| {
        ui.set_width(width);
        // The frame is drawn here, under the cover and the text, once the text has been laid out.
        let frame_slot = ui.painter().add(egui::Shape::Noop);
        // White, unless the text is too pale to read on white (white text from a coloured
        // banner, #913): then a dark box, with a caret that shows on it.
        let fill = if background_ready { Color32::TRANSPARENT } else { editor_fill(text_color) };
        // Until the background worker finishes, cover the entire source box, including
        // lines the draft no longer occupies. Shrinking the text must never expose a copy.
        if !background_ready {
            ui.painter().rect_filled(ed.rect.expand(1.0), CornerRadius::ZERO, fill);
        }
        let mut painted = None;
        let mut hovered = false;
        egui::Frame::NONE.fill(fill).show(ui, |ui| {
            // A textless PDF preview keeps its real page background through the transparent editor.
            // The temporary opaque cover uses the shared caret-contrast rule.
            ui.visuals_mut().text_cursor.stroke.color =
                if background_ready { if luminance(text_color) < 0.5 { Color32::BLACK } else { Color32::WHITE } } else { caret_color_on(fill) };
            let snapshot = ed.clone();
            let edit = text_field(&mut ed.text, snapshot, scale, width, wrap, font.clone(), text_color);
            let output = edit.show_mapped(ui);
            if let Some(selected) = output.logical_range() {
                ed.selection = selection_bytes(&ed.text, selected);
            }
            painted = Some(Arc::clone(&output.out.galley));
            let r = output.out.response;
            hovered = r.hovered();
            if r.changed() {
                // The next pass measures the updated draft with the Apply/Save rewrite.
                ctx.request_repaint();
            }
            if ed.focus {
                r.request_focus();
                ed.focus = false;
            }
            let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
            let apply = ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command);
            if esc {
                done = Some(false);
            } else if apply || (r.lost_focus() && !outside && !hold) {
                done = Some(true);
            }
        });
        // What the draft takes up, from the galley the field drew.
        let measured = painted.map_or(Rect::from_min_size(Pos2::ZERO, egui::vec2(1.0, 1.0)), |galley| galley.rect);
        ed.overflow = ed.has_changes() && (measured.width() > ed.rect.width() + 1.5 || measured.height() > ed.rect.height() + 1.5);
        let visible = Rect::from_min_size(origin, measured.size().max(egui::vec2(1.0, 1.0)));
        // Unsupported source programs use an interface fallback whose glyph advances
        // can differ from the PDF's. Keep every visible row inside the growing frame.
        let live = draft_rect.map_or(visible, |bounds| bounds.union(visible));
        ui.painter()
            .set(frame_slot, egui::Shape::rect_stroke(live.expand(2.0), CornerRadius::ZERO, Stroke::new(1.5, ACCENT), egui::StrokeKind::Outside));
        // On the frame, a press takes hold of the box (a click selects it, a drag moves it):
        // the pointer says so there, and reads as text inside.
        if !hovered
            && let Some(p) = ui.input(|i| i.pointer.hover_pos())
            && live.expand(2.0 + FRAME_REACH).contains(p)
            && !live.expand(2.0).contains(p)
        {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
        }
    });
    match done {
        Some(apply) => {
            let ed = view.line_editor.take()?;
            if apply { finished_edit(ed) } else { None }
        }
        None => None,
    }
}

/// The edit a closed paragraph editor applies, or `None` when nothing in it changed.
pub(crate) fn finished_edit(ed: LineEditor) -> Option<Edit> {
    let style = ed.style();
    (ed.text != ed.original || style != pdfcraft_engine::BlockStyle::default()).then_some(Edit::EditTextBlock {
        page: ed.page,
        block: ed.block,
        text: ed.text,
        style,
    })
}

/// Caret colour that stays visible on a field filled with `fill`: black on light fills, white on dark ones.
pub(crate) fn caret_color_on(fill: Color32) -> Color32 {
    let luma = 299 * u32::from(fill.r()) + 587 * u32::from(fill.g()) + 114 * u32::from(fill.b());
    if luma > 127_500 { Color32::BLACK } else { Color32::WHITE }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An app with one text document open, and an editor on its first paragraph.
    fn preview_app() -> (crate::PdfCraftApp, LineEditor) {
        let mut app = crate::PdfCraftApp::new();
        let bytes = app.session.create_from_text("Preview", "Preview text").unwrap();
        app.open_bytes("preview.pdf", None, bytes.as_ref().clone()).unwrap();
        let doc = app.session.get(app.views[0].id).unwrap();
        let block = doc.text_blocks(0).remove(0);
        let ed = LineEditor {
            selection: None,
            font_resource: None,
            page: 0,
            block: 0,
            text: block.text.clone(),
            original: block.text.clone(),
            rect: Rect::NOTHING,
            source_rect: block.rect.map(|v| v as f32),
            multiline: block.rows.len() > 1,
            source: None,
            fonts: BTreeMap::new(),
            overflow: false,
            max_width: 400.0,
            size: block.size as f32,
            focus: true,
            look: look_of(&block),
            look0: look_of(&block),
            extras: Extras::default(),
            extras0: Extras::default(),
            probe: None,
        };
        (app, ed)
    }

    /// "Hi there everyone" in red over "Yo there everyone" in blue, in Helvetica 12 pt: one paragraph of two rows.
    fn two_style_pdf() -> Vec<u8> {
        let body = "BT /F1 12 Tf 1 0 0 rg 20 150 Td (Hi there everyone) Tj ET\nBT /F1 12 Tf 0 0 1 rg 20 135 Td (Yo there everyone) Tj ET";
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [4 0 R] /Count 1 /MediaBox [0 0 200 300] >>".to_owned(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /Font << /F1 3 0 R >> >> >>".to_owned(),
            format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len()),
        ];
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
        out
    }

    /// An editor on the two-row paragraph of [`two_style_pdf`], holding `draft`, at 3 screen points to the
    /// page's point; `pristine`: the draft is also what the paragraph was.
    fn styled_editor(draft: &str, pristine: bool) -> (crate::PdfCraftApp, LineEditor) {
        let mut app = crate::PdfCraftApp::new();
        app.open_bytes("styled.pdf", None, two_style_pdf()).unwrap();
        let doc = app.session.get(app.views[0].id).unwrap();
        let block = doc.text_blocks(0).remove(0);
        let lines = doc.text_lines(0);
        assert_eq!((block.rows.len(), block.editor_text.as_str()), (2, ORIGINAL), "the fixture is one paragraph of two rows");
        let size = egui::vec2((block.rect[2] - block.rect[0]) as f32 * 3.0, (block.rect[3] - block.rect[1]) as f32 * 3.0);
        let ed = LineEditor {
            selection: None,
            font_resource: None,
            page: 0,
            block: 0,
            text: draft.to_owned(),
            original: if pristine { draft.to_owned() } else { block.editor_text.clone() },
            rect: Rect::from_min_size(Pos2::ZERO, size),
            source_rect: block.rect.map(|v| v as f32),
            multiline: true,
            source: Some((block.clone(), lines)),
            fonts: BTreeMap::new(),
            overflow: false,
            max_width: size.x,
            size: block.size as f32,
            focus: true,
            look: look_of(&block),
            look0: look_of(&block),
            extras: Extras::default(),
            extras0: Extras::default(),
            probe: None,
        };
        (app, ed)
    }

    const SCALE: f32 = 3.0;
    const ORIGINAL: &str = "Hi there everyone\nYo there everyone";

    /// The editor's field in an egui context of its own, shown by hand.
    struct Rig {
        ctx: egui::Context,
        ed: LineEditor,
        time: f64,
        width: f32,
    }

    impl Rig {
        fn new(fonts: egui::FontDefinitions, pixels_per_point: f32, ed: LineEditor) -> Self {
            let ctx = egui::Context::default();
            ctx.set_os(egui::os::OperatingSystem::Nix);
            ctx.set_fonts(fonts);
            ctx.set_pixels_per_point(pixels_per_point);
            let width = ed.rect.width();
            let mut rig = Self { ctx, ed, time: 0.0, width };
            // The pixel ratio and the fonts take effect from the second frame.
            rig.frame(vec![]);
            rig
        }

        /// A rig with the Arabic face, or `None` (the test skips).
        fn arabic(ed: LineEditor) -> Option<Self> {
            Some(Self::new(bidi::arabic_test_fonts()?, 1.0, ed))
        }

        fn frame(&mut self, events: Vec<egui::Event>) -> (egui::text_edit::TextEditOutput, std::sync::Arc<ScreenMap>) {
            self.time += 0.1;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(3000.0, 1000.0))),
                events,
                time: Some(self.time),
                ..Default::default()
            };
            let (width, ed) = (self.width, &mut self.ed);
            let mut out = None;
            let frame = self.ctx.run_ui(raw, |ui| {
                let id = egui::Id::new("edit-text-line-input");
                ui.memory_mut(|m| m.request_focus(id));
                let snapshot = ed.clone();
                let wrap = editor_wrap(&snapshot, &snapshot.text, width);
                let field = text_field(&mut ed.text, snapshot, SCALE, width, wrap, FontId::proportional(8.0), Color32::BLACK);
                let shown = field.show_mapped(ui);
                if let Some(range) = shown.logical_range() {
                    ed.selection = selection_bytes(&ed.text, range);
                }
                out = Some((shown.out, shown.map));
            });
            frame.drop_without_applying_deltas();
            out.expect("the field is shown every frame")
        }
    }

    const SALAM: &str = "\u{633}\u{644}\u{627}\u{645}";
    const ALAYKUM: &str = "\u{639}\u{644}\u{64A}\u{643}\u{645}";

    /// The colour of the section of `galley`'s job that holds character `i` of its text.
    fn section_color(galley: &egui::Galley, i: usize) -> Color32 {
        let byte = galley.job.text.char_indices().nth(i).map(|(at, _)| at).expect("a character of the text");
        galley.job.sections.iter().find(|s| s.byte_range.start.0 <= byte && byte < s.byte_range.end.0).expect("a section holds it").format.color
    }

    /// The colour each character of the draft is drawn in: its source row's.
    fn logical_colors(ed: &LineEditor) -> Vec<Color32> {
        let (block, lines) = ed.source.as_ref().expect("a source");
        let mut out = Vec::new();
        for (range, line) in block.edited_spans(&ed.text) {
            out.extend(std::iter::repeat_n(color32(lines[line].color), ed.text.get(range).map_or(0, |s| s.chars().count())));
        }
        out
    }

    /// The characters the galley draws, row by row (the line breaks are no glyphs).
    fn drawn(galley: &egui::Galley) -> Vec<char> {
        galley.rows.iter().flat_map(|row| row.glyphs.iter().map(|g| g.chr)).collect()
    }

    /// `draft` (more Arabic letters than Latin ones) starts with a word in the first row's red and ends with one
    /// in the second row's blue; every character of the display text is in the colour of the character of the
    /// draft it was drawn from.
    fn check_looks_follow_the_words(draft: &str, rows: usize) {
        let (_app, ed) = styled_editor(draft, false);
        let Some(mut rig) = Rig::arabic(ed) else { return };
        rig.width = 2000.0;
        let (out, map) = rig.frame(vec![]);
        assert_eq!(map.row_rtl, vec![true; rows], "most of the letters read right to left, so every row does");
        assert_ne!(map.shaper, draft, "the words are in display order");
        assert_eq!(out.galley.job.text, map.shaper);
        let (red, blue) = (color32([1.0, 0.0, 0.0]), color32([0.0, 0.0, 1.0]));
        let colors = logical_colors(&rig.ed);
        let word = SALAM.chars().count();
        assert!(colors[..word].iter().all(|c| *c == red), "the first word is in the first row's colour: {colors:?}");
        assert!(colors[colors.len() - ALAYKUM.chars().count()..].iter().all(|c| *c == blue), "the last in the second row's: {colors:?}");
        // Every character of the display text is in the colour of the character of the draft it was drawn from.
        for (i, source) in map.shaper_source().into_iter().enumerate() {
            if source == bidi::VIRTUAL {
                continue;
            }
            assert_eq!(section_color(&out.galley, i), colors[source], "character {i} ({:?}) of {:?}", map.shaper.chars().nth(i), map.shaper);
        }
        // Both Arabic words are there, one in each colour, and every character has a glyph.
        let salam_at = map.shaper.find(SALAM).expect("the first word, joined as the shaper wants it");
        let alaykum_at = map.shaper.find(ALAYKUM).expect("the second word");
        let chars_before = |byte: usize| map.shaper[..byte].chars().count();
        assert_eq!(section_color(&out.galley, chars_before(salam_at)), red);
        assert_eq!(section_color(&out.galley, chars_before(alaykum_at)), blue);
        assert_eq!(drawn(&out.galley), map.drawn.iter().copied().filter(|c| *c != '\n').collect::<Vec<_>>(), "a glyph for every drawn character");
        // The sections are cut at the letters of the script that joins and at the changes of look, nowhere else.
        let cuts: Vec<usize> = out.galley.job.sections.iter().map(|s| s.byte_range.start.0).collect();
        for cut in cuts.into_iter().skip(1) {
            let (before, after) =
                (map.shaper[..cut].chars().next_back().expect("a character"), map.shaper[cut..].chars().next().expect("a character"));
            assert!(!(bidi::is_joining_script(before) && bidi::is_joining_script(after)), "a cut inside a word between {before:?} and {after:?}");
        }
    }

    #[test]
    fn a_styled_arabic_word_keeps_its_own_style_after_the_words_are_put_in_display_order() {
        // "salam" (red, in front of "Hi") and "alaykum" (blue, after "Yo"), in two rows.
        check_looks_follow_the_words(&format!("{SALAM} Hi\nYo {ALAYKUM}"), 2);
    }

    #[test]
    fn a_row_of_two_looks_keeps_each_words_look_when_the_words_swap_places() {
        // One row: red "salam Hi" then blue "Yo alaykum", which is drawn the other way round, blue first from the left.
        check_looks_follow_the_words(&format!("{SALAM} Hi Yo {ALAYKUM}"), 1);
    }

    #[test]
    fn a_line_break_the_wrapping_makes_takes_the_look_of_the_character_before_it() {
        let text = "ab \u{633}\u{644}\u{627}\u{645} cd";
        let spans = [(0..3, 0), (3..text.len(), 1)];
        let map = bidi::wrapped_screen_map(text, false, Some(true), |line: &str| line.chars().count() <= 4);
        assert!(map.shown.contains(&bidi::VIRTUAL), "the paragraph is wrapped: {:?}", map.shaper);
        let styles = shaper_styles(&spans, text, &map);
        let source = map.shaper_source();
        assert_eq!((styles.len(), source.len()), (map.shaper.chars().count(), map.shaper.chars().count()));
        for (i, (&style, &at)) in styles.iter().zip(&source).enumerate() {
            if at == bidi::VIRTUAL {
                assert_eq!(Some(&style), i.checked_sub(1).and_then(|before| styles.get(before)), "the break at {i} follows its neighbour");
            } else {
                assert_eq!(style, usize::from(at >= 3), "character {i} was drawn from character {at}");
            }
        }
        // A break first of all takes the look of what comes after it.
        let mut map = map;
        map.shaper.insert(0, '\n');
        map.shown.insert(0, bidi::VIRTUAL);
        map.drawn.insert(0, '\n');
        assert_eq!(shaper_styles(&spans, text, &map).first(), shaper_styles(&spans, text, &map).get(1));
    }

    #[test]
    fn a_look_never_changes_inside_a_joined_stretch() {
        // The styles of "salam" differ letter by letter, as an edit that inserts into a word could make them.
        let text = format!("x {SALAM} y");
        let mut spans = Vec::new();
        for (at, c) in text.char_indices() {
            spans.push((at..at + c.len_utf8(), if at < 2 { 0 } else { at }));
        }
        let map = bidi::screen_map(&text);
        let styles = shaper_styles(&spans, &text, &map);
        let word: Vec<usize> = map.shaper.chars().zip(&styles).filter(|(c, _)| bidi::is_joining_script(*c)).map(|(_, s)| *s).collect();
        assert_eq!(word.len(), 4);
        assert!(word.iter().all(|s| *s == word[0]), "{word:?}");
        // …and Latin letters keep theirs.
        assert_ne!(styles[0], styles[map.shaper.chars().count() - 1]);
    }

    #[test]
    fn styles_follow_the_stored_characters_whatever_the_text() {
        // Pseudo-random text of both directions, brackets and breaks, with a style for every character.
        let alphabet: Vec<char> = "ab xyz()[]\n\u{633}\u{644}\u{627}\u{645}\u{5D0}\u{5D1}\u{200C}1 ".chars().collect();
        let mut seed = 0x2545_F491_u64;
        let mut next = move |n: usize| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            usize::try_from(seed >> 33).unwrap_or(0) % n
        };
        for case in 0..300 {
            let len = next(24);
            let text: String = (0..len).map(|_| alphabet[next(alphabet.len())]).collect();
            let stored: Vec<usize> = text.chars().map(|_| next(3)).collect();
            let mut spans: Vec<(std::ops::Range<usize>, usize)> = Vec::new();
            for ((at, c), style) in text.char_indices().zip(&stored) {
                match spans.last_mut() {
                    Some((range, last)) if last == style => range.end = at + c.len_utf8(),
                    _ => spans.push((at..at + c.len_utf8(), *style)),
                }
            }
            let forced = [None, Some(false), Some(true)][next(3)];
            let width = 2 + next(8);
            let map = if next(2) == 0 {
                bidi::wrapped_screen_map(&text, false, forced, |line: &str| line.chars().count() <= width)
            } else {
                bidi::screen_map(&text)
            };
            let styles = shaper_styles(&spans, &text, &map);
            let shaper: Vec<char> = map.shaper.chars().collect();
            let source = map.shaper_source();
            assert_eq!(styles.len(), shaper.len(), "case {case}: {text:?}");
            for i in 0..shaper.len() {
                let joined = i > 0 && bidi::is_shaped_rtl(shaper[i]) && bidi::is_shaped_rtl(shaper[i - 1]);
                let expected = if joined {
                    styles[i - 1]
                } else if source[i] == bidi::VIRTUAL {
                    if i > 0 { styles[i - 1] } else { styles.get(1).copied().unwrap_or(0) }
                } else {
                    stored[source[i]]
                };
                assert_eq!(styles[i], expected, "case {case}: {text:?} character {i}");
            }
        }
    }

    #[test]
    fn the_editor_reads_a_block_in_the_direction_the_page_writes_it() {
        // More Arabic letters than Latin ones: right to left, though the first strong letter is Latin.
        let more_arabic = format!("Hi {SALAM} {ALAYKUM}");
        // More Latin letters than Arabic ones: left to right, though the first strong letter is Arabic.
        let more_latin = format!("{SALAM} Hello Yo");
        for (draft, rtl) in [(&more_arabic, true), (&more_latin, false)] {
            let (_app, ed) = styled_editor(draft, false);
            let Some(mut rig) = Rig::arabic(ed) else { return };
            let (_, map) = rig.frame(vec![]);
            assert_eq!(pdfcraft_engine::rtl_base(draft), rtl);
            assert_eq!(map.row_rtl.first(), Some(&rtl), "{draft:?} reads {}", if rtl { "right to left" } else { "left to right" });
            assert_eq!(rig.ed.text, *draft);
        }
    }

    #[test]
    fn a_changed_arabic_block_wraps_at_the_boxs_width_and_each_row_ends_at_its_right_edge() {
        let paragraph = format!("{SALAM} {ALAYKUM} ").repeat(6);
        let (_app, ed) = styled_editor(paragraph.trim_end(), false);
        let Some(mut rig) = Rig::arabic(ed) else { return };
        let (out, map) = rig.frame(vec![]);
        let width = rig.width;
        assert!(out.galley.rows.len() >= 3, "the paragraph wraps in {width}: {} rows", out.galley.rows.len());
        assert!(map.row_rtl.iter().all(|rtl| *rtl));
        for (i, placed) in out.galley.rows.iter().enumerate() {
            let right = placed.pos.x + placed.row.size.x;
            assert!((right - width).abs() <= 1.0, "row {i} ends at {right}, the box at {width}");
            // The line is no wider than the box, once the space it keeps at its end is left out.
            let ink = placed.row.size.x - placed.row.glyphs.iter().take_while(|g| g.chr == ' ').map(|g| g.advance_width).sum::<f32>();
            assert!(ink <= width + 1.0, "row {i} is {ink} wide in {width}");
        }
        assert!((out.galley.rect.width() - width).abs() <= 1.0, "{:?}", out.galley.rect);
        // The rows are on the baselines the paragraph has, the extra ones below at its leading.
        let (block, lines) = rig.ed.source.as_ref().expect("a source");
        for (i, placed) in out.galley.rows.iter().enumerate().take(2) {
            let baseline = placed.pos.y + placed.row.glyphs[0].pos.y;
            let source = &lines[block.rows[i][0]];
            let expected = (block.rect[3] - source.baseline()[1]) as f32 * SCALE;
            assert!((baseline - expected).abs() < 1.5, "row {i} sits on the page's baseline: {baseline} against {expected}");
        }
    }

    #[test]
    fn an_unchanged_arabic_block_is_not_wrapped_and_keeps_the_right_edges_of_the_pages_rows() {
        // A first row that fits the box and a second one that does not.
        let long = format!("{SALAM} {ALAYKUM} ").repeat(6);
        let draft = format!("{SALAM} {ALAYKUM}\n{}", long.trim_end());
        let (_app, ed) = styled_editor(&draft, true);
        let Some(mut rig) = Rig::arabic(ed) else { return };
        let (out, map) = rig.frame(vec![]);
        assert_eq!(out.galley.rows.len(), 2, "a row for each line of the page, wrapped by nothing");
        assert_eq!(map.shaper.matches('\n').count(), 1);
        assert!(out.galley.rows[1].row.size.x > rig.width, "the second row is wider than the box");
        let (block, lines) = rig.ed.source.as_ref().expect("a source");
        let placed = &out.galley.rows[0];
        let expected = (lines[block.rows[0][0]].rect[2] - block.rect[0]) as f32 * SCALE;
        let right = placed.pos.x + placed.row.size.x;
        assert!((right - expected).abs() <= 1.0, "the first row ends at {right}, where the page's row does ({expected})");
        // Once the text changes, the rows are the box's: wrapped at its width and against its right edge.
        rig.ed.original.push('x');
        let (out, _) = rig.frame(vec![]);
        assert!(out.galley.rows.len() > 2, "the long row wraps now: {} rows", out.galley.rows.len());
        for (i, placed) in out.galley.rows.iter().enumerate() {
            let right = placed.pos.x + placed.row.size.x;
            assert!((right - rig.width).abs() <= 1.0, "row {i} ends at {right}, the box at {}", rig.width);
        }
    }

    #[test]
    fn the_alignment_asked_for_places_the_rows_that_read_right_to_left() {
        let draft = format!("{SALAM} {ALAYKUM}\n{SALAM}");
        for (align, left, right) in [
            (pdfcraft_engine::TextAlign::Left, true, false),
            (pdfcraft_engine::TextAlign::Center, false, false),
            (pdfcraft_engine::TextAlign::Right, false, true),
            (pdfcraft_engine::TextAlign::Justify, false, true),
        ] {
            let (_app, mut ed) = styled_editor(&draft, false);
            // (What the paragraph had is another alignment, so this one is a change.)
            ed.look0.align =
                if align == pdfcraft_engine::TextAlign::Left { pdfcraft_engine::TextAlign::Right } else { pdfcraft_engine::TextAlign::Left };
            ed.look.align = align;
            let Some(mut rig) = Rig::arabic(ed) else { return };
            let (out, _) = rig.frame(vec![]);
            let width = rig.width;
            for (i, placed) in out.galley.rows.iter().enumerate() {
                let (x, end) = (placed.pos.x, placed.pos.x + placed.row.size.x);
                let what = format!("{align:?}: row {i} is {x}..{end} in {width}");
                if left {
                    assert!(x.abs() <= 1.0, "{what}");
                } else if right {
                    assert!((end - width).abs() <= 1.0, "{what}");
                } else {
                    assert!((x - (width - end)).abs() <= 1.5, "{what}");
                }
            }
        }
    }

    #[test]
    fn latin_text_is_laid_out_as_the_page_has_it() {
        let (_app, ed) = styled_editor(ORIGINAL, true);
        let mut rig = Rig::new(crate::theme::font_definitions(), 1.0, ed);
        let (out, map) = rig.frame(vec![]);
        assert_eq!(*map, ScreenMap::identity(ORIGINAL));
        let galley = &out.galley;
        assert_eq!(galley.rows.len(), 2);
        let (red, blue) = (color32([1.0, 0.0, 0.0]), color32([0.0, 0.0, 1.0]));
        assert_eq!((section_color(galley, 0), section_color(galley, ORIGINAL.chars().position(|c| c == 'Y').unwrap_or(0))), (red, blue));
        assert_eq!(galley.job.sections.len(), 2, "one section for each source row");
        let (block, lines) = rig.ed.source.as_ref().expect("a source");
        for (i, placed) in galley.rows.iter().enumerate() {
            let source = &lines[block.rows[i][0]];
            let baseline = placed.pos.y + placed.row.glyphs[0].pos.y;
            let expected = (block.rect[3] - source.baseline()[1]) as f32 * SCALE;
            assert!((baseline - expected).abs() < 0.01, "row {i} on the page's baseline: {baseline} against {expected}");
            assert!((placed.pos.x - (source.rect[0] - block.rect[0]) as f32 * SCALE).abs() < 0.01, "row {i} starts where the page's row does");
            // The glyphs' pictures moved with them.
            let top = placed.row.visuals.mesh.vertices.iter().map(|v| v.pos.y).fold(f32::MAX, f32::min);
            assert!(top < placed.row.glyphs[0].pos.y, "row {i}: the picture is above the baseline");
        }
        assert!(galley.job.wrap.max_width.is_infinite(), "an unchanged paragraph is not wrapped");
        // Changed, it wraps at the box's width.
        rig.ed.original.push('x');
        rig.frame(vec![]);
        let (out, _) = rig.frame(vec![]);
        assert!((out.galley.job.wrap.max_width - rig.width).abs() < 0.5, "{}", out.galley.job.wrap.max_width);
    }

    #[test]
    fn typing_arabic_in_the_editor_keeps_the_draft_in_typing_order_and_the_selection_in_stored_indices() {
        let (_app, ed) = styled_editor("Hi\nYo", false);
        let Some(mut rig) = Rig::arabic(ed) else { return };
        let typed = format!("{SALAM} {ALAYKUM}");
        rig.frame(vec![egui::Event::Text(typed.clone())]);
        let (out, map) = rig.frame(vec![]);
        // At the end of the block (or where the caret was), in the order it was typed.
        assert!(rig.ed.text.contains(&typed), "{:?}", rig.ed.text);
        assert_eq!(map.logical_len(), rig.ed.text.chars().count());
        assert_eq!(out.galley.job.text, map.shaper);
        // Select all: the selection counts the draft's own characters, bytes of the draft.
        let select_all = egui::Event::Key {
            key: egui::Key::A,
            physical_key: Some(egui::Key::A),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        rig.frame(vec![select_all]);
        assert_eq!(rig.ed.selection, Some(0..rig.ed.text.len()));
        // Backspace removes the letter typed last.
        let backspace = |key| egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        rig.frame(vec![egui::Event::Text(typed.clone())]);
        rig.frame(vec![backspace(egui::Key::Backspace)]);
        assert_eq!(rig.ed.text, typed.chars().take(typed.chars().count() - 1).collect::<String>());
    }

    #[test]
    fn hebrew_is_edited_in_display_order_too() {
        let hebrew = "\u{5E9}\u{5DC}\u{5D5}\u{5DD} \u{5E2}\u{5D5}\u{5DC}\u{5DD}";
        let (_app, ed) = styled_editor(hebrew, false);
        let Some(mut rig) = Rig::arabic(ed) else { return };
        rig.width = 2000.0;
        let (out, map) = rig.frame(vec![]);
        assert_eq!(map.row_rtl, [true]);
        assert_ne!(map.shaper, hebrew, "the words are in display order");
        assert_eq!(drawn(&out.galley).len(), hebrew.chars().count());
        let placed = &out.galley.rows[0];
        assert!((placed.pos.x + placed.row.size.x - rig.width).abs() <= 1.0);
    }

    #[test]
    fn the_editor_lays_out_at_every_pixel_ratio_and_scale() {
        let draft = format!("{SALAM} {ALAYKUM} Hi\n{ALAYKUM} Yo {SALAM} {ALAYKUM} {SALAM}");
        for ppp in [1.0, 1.5, 2.0] {
            for width_scale in [0.5_f32, 1.0, 2.0] {
                let (_app, ed) = styled_editor(&draft, false);
                let Some(fonts) = bidi::arabic_test_fonts() else { return };
                let mut rig = Rig::new(fonts, ppp, ed);
                rig.width = (rig.ed.rect.width() * width_scale).round();
                let (out, map) = rig.frame(vec![]);
                for (i, placed) in out.galley.rows.iter().enumerate() {
                    if map.row_is_rtl(i) {
                        let right = placed.pos.x + placed.row.size.x;
                        assert!((right - rig.width).abs() <= 1.0 / ppp + 0.5, "row {i} ends at {right} in {} ({ppp} px per point)", rig.width);
                    }
                }
                assert_eq!(drawn(&out.galley).len(), map.drawn.iter().filter(|c| **c != '\n').count());
            }
        }
    }

    #[test]
    fn a_selection_of_characters_is_a_range_of_bytes_of_the_draft() {
        let text = "a\u{633}\u{644}b";
        assert_eq!(selection_bytes(text, 0..0), None);
        assert_eq!(selection_bytes(text, 1..3), Some(1..5));
        assert_eq!(selection_bytes(text, 0..4), Some(0..text.len()));
        assert_eq!(selection_bytes(text, 2..99), Some(3..text.len()), "past the end is the end");
        assert_eq!(selection_bytes("", 0..3), None, "nothing to select");
    }

    /// A letter of Arabic is drawn in the interface's face unless the engine says Apply shapes the draft in the page's own: then in
    /// that face (a `pdf-source-N` family the editor installed), and the other letters of the line stay in the line's font.
    #[test]
    fn the_joined_letters_of_a_line_take_the_pages_face_when_the_engine_names_one() {
        let (_app, mut ed) = styled_editor(ORIGINAL, true);
        let ctx = egui::Context::default();
        let (block, lines) = ed.source.clone().expect("the editor has its source");
        let line = &lines[block.lines[0]];
        ed.fonts.insert(line.font.clone(), "pdf-source-7".into());
        let own = line_formats(&ctx, &ed, line, 2.0, true, None);
        let page = line_formats(&ctx, &ed, line, 2.0, true, Some(&"pdf-source-7".to_owned()));
        assert_ne!(own.joined.font_id.family, FontFamily::Name("pdf-source-7".into()), "the interface's face without a page face");
        assert_eq!(page.joined.font_id.family, FontFamily::Name("pdf-source-7".into()), "the page's face when the engine names one");
        assert_eq!(page.joined.font_id.size, own.joined.font_id.size, "at the same size");
        assert_eq!(page.plain.font_id, own.plain.font_id, "the ordinary letters are drawn as before");
    }

    /// Without a copy of the document to ask (or when the rewrite names no page face for the draft) the editor keeps the interface's face
    /// for Arabic; a face the engine names but the editor did not install is not used either.
    #[test]
    fn no_page_face_is_used_unless_the_engine_names_one_the_editor_has() {
        let arabic = "\u{633}\u{644}\u{627}\u{645}";
        let (app, mut ed) = styled_editor(arabic, false);
        assert!(ed.page_faces(arabic).is_empty(), "nothing to ask");
        let doc = app.session.get(app.views[0].id).unwrap();
        ed.probe = doc.text_draft_probe(0, 0).map(DraftProbe::new);
        assert!(ed.probe.is_some());
        // (Helvetica has no embedded program to shape in: whatever the rewrite does with the draft, it names no page face.)
        assert!(ed.page_faces(arabic).is_empty(), "Helvetica has no face to shape in");
        assert!(ed.page_faces("Hello").is_empty(), "no Arabic in the draft");
    }

    #[test]
    fn a_loaded_font_is_applied_to_the_paragraph_being_edited() {
        // Loading a font used to only register it on the page: the paragraph kept its source
        // face until the font was found again in the document-font menu (#2591).
        let (mut app, ed) = preview_app();
        app.views[0].line_editor = Some(ed);
        let inter = include_bytes!("../../../assets/fonts/Inter-Regular.ttf").to_vec();
        app.use_files(crate::files::FilePurpose::TextFont { page: 0 }, vec![("Inter-Regular.ttf".into(), inter)]);
        let doc = app.session.get(app.views[0].id).unwrap();
        let lines = doc.text_lines(0);
        assert_eq!(lines.iter().map(|l| l.text.as_str()).collect::<String>(), "Preview text");
        assert!(lines.iter().all(|l| l.base_font == "Inter-Regular"), "{:?}", lines.iter().map(|l| &l.base_font).collect::<Vec<_>>());
        assert!(app.views[0].line_editor.is_some(), "the paragraph stays open for editing");
    }

    #[test]
    fn text_background_is_cached_without_editing_and_released_on_cancel() {
        let (app, ed) = preview_app();
        let doc = app.session.get(app.views[0].id).unwrap();
        let source = doc.bytes.clone();
        let generation = doc.edit_generation();
        let ctx = egui::Context::default();
        let mut background = TextBackground::default();
        for _ in 0..100 {
            background.prepare(&ctx, doc, Some(&ed), None, 1.0);
            if background.ready(0, 0) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(background.ready(0, 0));
        assert!(!background.pending());
        let texture = background.texture.as_ref().unwrap().id();
        background.prepare(&ctx, doc, Some(&ed), None, 1.0);
        assert_eq!(background.texture.as_ref().unwrap().id(), texture);
        assert_eq!(doc.bytes, source);
        assert_eq!(doc.edit_generation(), generation);
        assert!(!doc.dirty && doc.can_undo().is_none());
        background.prepare(&ctx, doc, None, None, 1.0);
        assert!(background.texture.is_none() && background.renderer.is_none());
    }

    #[test]
    fn the_editing_background_stays_sharp_on_a_zoomed_page() {
        // While a paragraph is open, the page around it is drawn from this one raster: it is
        // asked for at the tiling threshold, not the 2048-pixel backdrop side, which went soft
        // beyond about 1.2x on a Retina display.
        let (app, ed) = preview_app();
        let doc = app.session.get(app.views[0].id).unwrap();
        let page = &doc.info.pages[0];
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput { max_texture_side: Some(16384), ..Default::default() });
        let mut background = TextBackground::default();
        background.prepare(&ctx, doc, Some(&ed), None, 8.0);
        let expected = 8.0f32.min(crate::canvas::TILE_THRESHOLD / page.width.max(page.height));
        assert!(expected > crate::canvas::BASE_SIDE / page.width.max(page.height), "the test page needs the larger side");
        assert_eq!(background.tag, Some((expected * 1000.0) as u64));
        // A GPU that can't hold such a texture caps it.
        ctx.end_pass().textures_delta.clear();
        ctx.begin_pass(egui::RawInput { max_texture_side: Some(2048), ..Default::default() });
        let mut small = TextBackground::default();
        small.prepare(&ctx, doc, Some(&ed), None, 8.0);
        assert_eq!(small.tag, Some(((2048.0 / page.width.max(page.height)) * 1000.0) as u64));
        ctx.end_pass().textures_delta.clear();
    }

    #[test]
    fn pale_text_is_edited_on_a_dark_box() {
        // #913: white text from a coloured banner was typed onto a white box and vanished.
        for pale in [Color32::WHITE, Color32::from_rgb(0xFF, 0xFF, 0x00), Color32::from_gray(0xE0), Color32::from_rgb(0xFF, 0xA5, 0x00)] {
            assert_eq!(editor_fill(pale), DARK_EDITOR_FILL, "{pale:?}");
            assert!(contrast(pale, editor_fill(pale)) >= 4.5, "{pale:?} is readable");
        }
        // Ordinary dark or saturated text keeps the white box.
        for dark in [Color32::BLACK, Color32::from_rgb(0x1F, 0x2A, 0x6B), Color32::from_rgb(0xC0, 0x00, 0x00), Color32::from_gray(0x60)] {
            assert_eq!(editor_fill(dark), Color32::WHITE, "{dark:?}");
        }
    }

    #[test]
    fn caret_contrasts_with_its_fill() {
        assert_eq!(caret_color_on(Color32::WHITE), Color32::BLACK);
        assert_eq!(caret_color_on(Color32::from_rgb(0xFF, 0xFF, 0xF4)), Color32::BLACK);
        assert_eq!(caret_color_on(Color32::BLACK), Color32::WHITE);
        assert_eq!(caret_color_on(DARK_EDITOR_FILL), Color32::WHITE);
    }
}
