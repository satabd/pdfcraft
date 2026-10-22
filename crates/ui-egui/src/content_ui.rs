//! Edit a PDF ▸ Add content: the Text tool (click on the page and type), Image (pick a file;
//! it lands in the middle of the page), and editing what was added: select, move, resize
//! (images keep their proportions), double-click text to retype it, format it from the panel,
//! Delete to remove it. Each change is one undoable engine edit.

use egui::{Color32, CornerRadius, Pos2, Rect, Stroke};
use pdfcraft_engine::{Added, AddedContent, AddedText, Edit, FontFamily, TextAlign};
use pdfcraft_render::DocInfo;

use crate::canvas::{DocView, PageXform};
use crate::theme::Tokens;
use crate::widgets;

/// The colour of the warning under text the standard fonts can't draw.
pub(crate) const UNDRAWABLE: Color32 = Color32::from_rgb(0xD7, 0x37, 0x3F);

/// The first character of a draft that can't be drawn, as the engine decides it (it refuses
/// such text, #125): outside the standard fonts and not shaped with the Arabic face.
pub(crate) fn undrawable(d: &TextDraft, memo: &UndrawableMemo) -> Option<char> {
    // Checked as `finish` will commit it: without trailing whitespace.
    let t = AddedText { text: d.text.trim_end().to_string(), rect: d.rect, ..d.style.clone() };
    if let Some((key, c)) = memo.borrow().as_ref()
        && *key == t
    {
        return *c;
    }
    let c = pdfcraft_engine::first_undrawable(&t);
    *memo.borrow_mut() = Some((t, c));
    c
}

/// The last draft [`undrawable`] checked and its answer: the check lays the text out (and shapes
/// Arabic), too slow to repeat every frame for long text, so it reruns only when the text or
/// anything that changes its layout does.
pub(crate) type UndrawableMemo = std::cell::RefCell<Option<(AddedText, Option<char>)>>;

/// The warning shown for text with `c` in it.
pub(crate) fn undrawable_message(c: char) -> crate::i18n::Logical<'static> {
    crate::i18n::fmt_logical(
        &tl_logical!("The PDF text fonts can't draw “{c}” ({code}). Choose supported text."),
        &[("c", &c.to_string()), ("code", &format!("U+{:04X}", u32::from(c)))],
    )
}

/// The warning after an edit that saved text its font can't draw: PdfCraft drew `?` for it (a form
/// field, which other viewers redraw from its value, or a comment, where the `?` shows everywhere).
pub(crate) fn undrawn_message(n: &pdfcraft_engine::TextNotice) -> crate::i18n::Logical<'static> {
    let template = if n.redrawn_elsewhere {
        tl_logical!(
            "This field's font can't draw “{c}” ({code}), so PdfCraft shows ? in its place. The value is saved, and other PDF viewers redraw the field from it."
        )
    } else {
        tl_logical!("The fonts PdfCraft writes with can't draw “{c}” ({code}), so it shows as ? in every PDF viewer. The text is saved.")
    };
    crate::i18n::fmt_logical(&template, &[("c", &n.ch.to_string()), ("code", &format!("U+{:04X}", u32::from(n.ch)))])
}

const SELECT_BLUE: Color32 = Color32::from_rgb(0x14, 0x73, 0xE6);

/// The text being typed or retyped in place.
#[derive(Clone, Debug, PartialEq)]
pub struct TextDraft {
    pub page: usize,
    /// `None` for new text; the item index for retyping.
    pub index: Option<usize>,
    /// Display space.
    pub rect: [f64; 4],
    pub text: String,
    pub style: AddedText,
    pub focus: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Grab {
    Move,
    /// Which corner: (left, top) in screen terms.
    Corner(bool, bool),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContentView {
    /// The selected added item: (page, index among the page's items).
    pub selected: Option<(usize, usize)>,
    pub draft: Option<TextDraft>,
    /// Select the item an add creates (the page's item count before it).
    pub select_added: Option<(usize, usize)>,
    grab: Option<(usize, Grab, Pos2)>,
    undrawable: UndrawableMemo,
    layout: Option<(AddedText, f32, f64, [f64; 4])>,
}

impl ContentView {
    pub(crate) fn finish_draft(&mut self, added: &[Added]) -> Option<Edit> {
        self.draft.take().and_then(|d| finish(self, d, added))
    }
    /// The draft's first character the standard fonts can't draw. Such a draft is kept open, and
    /// counts as unsaved work, until it is corrected or discarded.
    pub(crate) fn blocked(&self) -> Option<char> {
        self.draft.as_ref().and_then(|d| undrawable(d, &self.undrawable))
    }

    /// Keeps a blocked draft open and focused instead of finishing or replacing it.
    pub(crate) fn hold_blocked(&mut self) -> bool {
        let blocked = self.blocked().is_some();
        if let Some(d) = self.draft.as_mut().filter(|_| blocked) {
            d.focus = true;
        }
        blocked
    }
}

/// While an added item is moved, the page without it, rendered once per gesture: the item's own
/// pixels follow the pointer over it instead of staying put until release (#1488).
#[derive(Default)]
pub(crate) struct MoveBackground {
    key: Option<(u64, usize, usize)>,
    renderer: Option<pdfcraft_render::RenderPool>,
    texture: Option<egui::TextureHandle>,
    tag: Option<u64>,
    /// The item's display-space box when the gesture started.
    from: Option<[f64; 4]>,
    /// The applied move's box. The page raster on screen still has the item where it was until
    /// its replacement arrives, so the preview stays up until then.
    settling: Option<[f64; 4]>,
}

impl MoveBackground {
    pub fn pending(&self) -> bool {
        self.renderer.is_some() && self.texture.is_none()
    }

    pub fn prepare(&mut self, ctx: &egui::Context, doc: &pdfcraft_engine::Document, cv: &ContentView, scale: f32) {
        let key = match (cv.grab, cv.selected) {
            (Some((gp, Grab::Move, _)), Some((sp, si))) if gp == sp => Some((doc.edit_generation(), sp, si)),
            _ => None,
        };
        if !(key.is_none() && self.settling.is_some()) && self.key != key {
            *self = Self { key, ..Default::default() };
            if let Some((_, page, index)) = key {
                self.renderer = doc.added_content_preview(page, index).ok().map(|p| p.renderer());
                self.from = on_page(doc.added(), page).into_iter().find(|(i, _)| *i == index).map(|(_, a)| a.content.rect());
            }
        }
        if self.settling.is_some() {
            return;
        }
        let (Some((_, page, _)), Some(renderer)) = (self.key, self.renderer.as_ref()) else { return };
        let Some(p) = doc.info.pages.get(page) else { return };
        let max_side = ctx.input(|i| i.max_texture_side) as f32;
        let scale = scale.min(crate::canvas::BASE_SIDE.min(max_side) / p.width.max(p.height).max(1.0));
        let tag = (scale * 1000.0) as u64;
        if self.tag != Some(tag) {
            renderer.set_queue(vec![pdfcraft_render::RenderRequest { page, scale, tag, ..Default::default() }]);
            self.tag = Some(tag);
        }
        if let Some(out) = renderer.try_recv() {
            if out.error.is_some() {
                // Without a background the item stays put while dragged, as before.
                self.renderer = None;
                return;
            }
            if out.request.tag == tag {
                self.texture = Some(ctx.load_texture(
                    "added-content-move-background",
                    crate::canvas::texture_image([out.width as usize, out.height as usize], out.rgba),
                    egui::TextureOptions::LINEAR,
                ));
            }
        }
        if self.pending() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    /// The move was applied: keep the preview, now at the new generation, until the page is
    /// rendered again. Any other edit ends it.
    pub fn committed(&mut self, edit: &Edit, generation: u64) {
        match edit {
            Edit::UpdateContent { page, index, content } if self.texture.is_some() && self.key.is_some_and(|(_, p, i)| (p, i) == (*page, *index)) => {
                self.key = Some((generation, *page, *index));
                self.settling = Some(content.rect());
            }
            _ => *self = Self::default(),
        }
    }

    pub fn page_received(&mut self, page: usize) {
        if self.settling.is_some() && self.key.is_some_and(|(_, p, _)| p == page) {
            *self = Self::default();
        }
    }

    /// The page without the moving item, then the item's pixels taken from `page_tex` (the page
    /// as rendered, item included where it was) at the dragged or applied position.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        xf: &PageXform,
        page: usize,
        info: &DocInfo,
        cv: &ContentView,
        page_tex: Option<egui::TextureId>,
    ) {
        let (Some((_, kp, _)), Some(background), Some(page_tex), Some(from)) = (self.key, &self.texture, page_tex, self.from) else { return };
        if kp != page {
            return;
        }
        let r = screen_rect(xf, info, page, from);
        let d = if let Some((_, _, start)) = cv.grab {
            let Some(p) = ui.input(|i| i.pointer.hover_pos()) else { return };
            p - start
        } else if let Some(to) = self.settling {
            // A moved text box keeps its top edge (its height follows its lines).
            screen_rect(xf, info, page, to).left_top() - r.left_top()
        } else {
            return;
        };
        let painter = painter.with_clip_rect(painter.clip_rect().intersect(xf.rect));
        xf.paint_image(&painter, background.id(), 0.0, 0.0, 1.0, 1.0);
        let r = r.expand(1.0);
        let mut mesh = egui::Mesh::with_texture(page_tex);
        for c in [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()] {
            let (u, v) = xf.screen_to_norm(c);
            mesh.vertices.push(egui::epaint::Vertex { pos: c + d, uv: Pos2::new(u, v), color: Color32::WHITE });
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(egui::Shape::mesh(mesh));
    }
}

/// Text style for new text (the panel's Format controls set it).
pub fn default_style() -> AddedText {
    AddedText { size: 12.0, ..AddedText::default() }
}

/// Display-space rect → screen.
fn screen_rect(xf: &PageXform, info: &DocInfo, page: usize, r: [f64; 4]) -> Rect {
    let ph = info.pages[page].height as f64;
    xf.view_rect([r[0] as f32, (ph - r[3]) as f32, r[2] as f32, (ph - r[1]) as f32])
}

/// Screen point → display space.
fn to_display(xf: &PageXform, info: &DocInfo, page: usize, p: Pos2) -> [f64; 2] {
    let (vx, vy) = xf.screen_to_view(p);
    [vx as f64, info.pages[page].height as f64 - vy as f64]
}

fn on_page(added: &[Added], page: usize) -> Vec<(usize, &Added)> {
    added.iter().filter(|a| a.page == page).enumerate().collect()
}

pub(crate) fn hits_added(xf: &PageXform, info: &DocInfo, page: usize, added: &[Added], point: Pos2) -> bool {
    on_page(added, page).iter().any(|(_, a)| screen_rect(xf, info, page, a.content.rect()).expand(2.0).contains(point))
}

fn handles(r: Rect) -> [(Pos2, bool, bool); 4] {
    [(r.left_top(), true, true), (r.right_top(), false, true), (r.left_bottom(), true, false), (r.right_bottom(), false, false)]
}

/// The rect while moving or resizing (images keep their proportions).
fn dragged(r: Rect, grab: Grab, d: egui::Vec2, keep_aspect: bool) -> Rect {
    match grab {
        Grab::Move => r.translate(d),
        Grab::Corner(left, top) => {
            let (mut dx, mut dy) = (if left { -d.x } else { d.x }, if top { -d.y } else { d.y });
            if keep_aspect && r.width() > 0.0 && r.height() > 0.0 {
                let k = ((r.width() + dx) / r.width()).max((r.height() + dy) / r.height()).max(0.05);
                dx = r.width() * k - r.width();
                dy = r.height() * k - r.height();
            }
            let mut out = r;
            if left {
                out.min.x -= dx;
            } else {
                out.max.x += dx;
            }
            if top {
                out.min.y -= dy;
            } else {
                out.max.y += dy;
            }
            Rect::from_two_pos(out.min, out.max)
        }
    }
}

/// Added text and images keep pointer ownership over a form field while editing content.
pub(crate) fn select_grabs(ui: &egui::Ui, xf: &PageXform, page: usize, info: &DocInfo, added: &[Added], view: &DocView) -> bool {
    let cv = &view.content;
    if cv.grab.is_some_and(|(grab_page, _, _)| grab_page == page) {
        return true;
    }
    let Some(p) = ui.input(|i| i.pointer.hover_pos().or(i.pointer.interact_pos())).filter(|p| xf.rect.contains(*p)) else {
        return false;
    };
    on_page(added, page).into_iter().any(|(index, item)| {
        let rect = screen_rect(xf, info, page, item.content.rect());
        rect.expand(2.0).contains(p)
            || cv.selected == Some((page, index)) && handles(rect).into_iter().any(|(corner, _, _)| corner.distance(p) <= 6.0)
    })
}

/// Pointer input on one page in Edit mode (`adding_text` with the Text tool). `true` when it
/// used the input.
#[allow(clippy::too_many_arguments)]
pub(crate) fn page_input(
    ui: &egui::Ui,
    resp: &egui::Response,
    xf: &PageXform,
    page: usize,
    info: &DocInfo,
    added: &[Added],
    adding_text: bool,
    edit_on_click: bool,
    style: &AddedText,
    view: &mut DocView,
) -> bool {
    let mut consumed = false;
    let pointer = ui.input(|i| i.pointer.hover_pos().or(i.pointer.interact_pos()));
    let cv = &mut view.content;
    if let Some((gp, grab, start)) = cv.grab
        && gp == page
    {
        consumed = true;
        if resp.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
            cv.grab = None;
            let end = pointer.unwrap_or(start);
            if let Some((sp, si)) = cv.selected
                && sp == page
                && let Some((_, a)) = on_page(added, page).into_iter().find(|(i, _)| *i == si)
                && (end - start).length() >= 1.0
            {
                let keep = matches!(a.content, AddedContent::Image(_));
                let r = dragged(screen_rect(xf, info, page, a.content.rect()), grab, end - start, keep);
                let (p0, p1) = (to_display(xf, info, page, r.left_bottom()), to_display(xf, info, page, r.right_top()));
                let mut rect = [p0[0], p0[1], p1[0], p1[1]];
                if let AddedContent::Text(t) = &a.content
                    && grab == Grab::Move
                {
                    // Text height follows its lines: keep the top edge's offset only.
                    rect = [rect[0], rect[3] - (t.rect[3] - t.rect[1]), rect[2], rect[3]];
                }
                view.pending_edit = Some(Edit::UpdateContent { page, index: si, content: a.content.with_rect(rect) });
            }
        }
        return consumed;
    }
    let Some(p) = pointer.filter(|p| xf.rect.contains(*p)) else { return consumed };
    let items = on_page(added, page);
    let hit = items.iter().rev().find(|(_, a)| screen_rect(xf, info, page, a.content.rect()).expand(2.0).contains(p)).map(|(i, a)| (*i, *a));
    let selected_rect = cv
        .selected
        .filter(|s| s.0 == page)
        .and_then(|(_, si)| items.iter().find(|(i, _)| *i == si))
        .map(|(_, a)| screen_rect(xf, info, page, a.content.rect()));
    let corner = selected_rect.and_then(|r| handles(r).into_iter().find(|(c, ..)| c.distance(p) <= 6.0));
    if adding_text && hit.is_none() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        if resp.clicked() {
            // Text that can't be drawn stays where it is being typed (#125).
            if cv.hold_blocked() {
                return true;
            }
            // A click elsewhere keeps the text being typed and starts a new box (#74). The click
            // can reach the page before the editor sees it lose focus, so finish it here.
            let typed = cv.draft.take().and_then(|d| finish(cv, d, added));
            // The Format panel styles the new text, not the box just kept.
            (cv.selected, cv.select_added) = (None, None);
            let at = to_display(xf, info, page, p);
            let mut style = style.clone();
            style.text.clear();
            let page_width = info.pages.get(page).map_or(f64::INFINITY, |p| p.width as f64);
            let rect = [at[0], at[1] - style.size * 1.2, (at[0] + 200.0).min(page_width), at[1]];
            style.rect = rect;
            cv.draft = Some(TextDraft { page, index: None, rect, text: String::new(), style, focus: true });
            consumed = true;
            if typed.is_some() {
                view.pending_edit = typed;
            }
        }
        return consumed;
    }
    if let Some((_, left, top)) = corner {
        ui.ctx().set_cursor_icon(if left == top { egui::CursorIcon::ResizeNwSe } else { egui::CursorIcon::ResizeNeSw });
    } else if hit.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
    }
    if resp.drag_started() {
        let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(p);
        if let Some((_, left, top)) = selected_rect.and_then(|r| handles(r).into_iter().find(|(c, ..)| c.distance(origin) <= 6.0)) {
            cv.grab = Some((page, Grab::Corner(left, top), origin));
            consumed = true;
        } else if let Some((i, _)) = items.iter().rev().find(|(_, a)| screen_rect(xf, info, page, a.content.rect()).expand(2.0).contains(origin)) {
            cv.selected = Some((page, *i));
            cv.grab = Some((page, Grab::Move, origin));
            consumed = true;
        }
        return consumed;
    }
    if (resp.double_clicked() || edit_on_click && resp.clicked() && corner.is_none())
        && let Some((i, a)) = hit
        && let AddedContent::Text(t) = &a.content
    {
        if cv.hold_blocked() {
            return true;
        }
        if cv.draft.as_ref().is_some_and(|d| d.page == page && d.index == Some(i)) {
            return true;
        }
        if let Some(edit) = cv.finish_draft(added) {
            view.pending_edit = Some(edit);
        }
        cv.draft = Some(TextDraft { page, index: Some(i), rect: t.rect, text: t.text.clone(), style: t.clone(), focus: true });
        cv.selected = Some((page, i));
        consumed = true;
        return consumed;
    }
    if resp.clicked() {
        match hit {
            Some((i, _)) => {
                cv.selected = Some((page, i));
                consumed = true;
            }
            None if corner.is_none() => cv.selected = None,
            None => consumed = true,
        }
    }
    consumed || hit.is_some() && resp.is_pointer_button_down_on()
}

/// Outlines of added items (hover and selection with handles) and the box being dragged.
pub(crate) fn paint_page(ui: &egui::Ui, painter: &egui::Painter, xf: &PageXform, page: usize, info: &DocInfo, added: &[Added], view: &DocView) {
    let cv = &view.content;
    let pointer = ui.input(|i| i.pointer.hover_pos());
    for (i, a) in on_page(added, page) {
        if cv.draft.as_ref().is_some_and(|d| d.page == page && d.index == Some(i)) {
            continue;
        }
        let mut r = screen_rect(xf, info, page, a.content.rect());
        let selected = cv.selected == Some((page, i));
        if selected
            && let (Some((gp, g, start)), Some(p)) = (cv.grab, pointer)
            && gp == page
        {
            r = dragged(r, g, p - start, matches!(a.content, AddedContent::Image(_)));
        }
        if selected {
            painter.rect_stroke(r.expand(1.0), CornerRadius::ZERO, Stroke::new(1.5, SELECT_BLUE), egui::StrokeKind::Outside);
            for (c, ..) in handles(r.expand(1.0)) {
                painter.circle(c, 3.5, Color32::WHITE, Stroke::new(1.5, SELECT_BLUE));
            }
        } else if pointer.is_some_and(|p| r.expand(2.0).contains(p)) {
            painter.rect_stroke(r.expand(1.0), CornerRadius::ZERO, Stroke::new(1.0, SELECT_BLUE.gamma_multiply(0.7)), egui::StrokeKind::Outside);
        }
    }
}

/// Delete removes the selected item; Escape clears the selection.
pub(crate) fn keys(ctx: &egui::Context, view: &mut DocView) {
    if view.content.selected.is_none() || view.content.draft.is_some() || ctx.egui_wants_keyboard_input() {
        return;
    }
    let (del, esc) = ctx.input(|i| (i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace), i.key_pressed(egui::Key::Escape)));
    if del && let Some((page, index)) = view.content.selected.take() {
        view.pending_edit = Some(Edit::DeleteContent { page, index });
    } else if esc {
        view.content.selected = None;
    }
}

/// After an add: select the new item.
pub(crate) fn after_refresh(view: &mut DocView, added: &[Added]) {
    if let Some((page, before)) = view.content.select_added.take()
        && on_page(added, page).len() > before
    {
        view.content.selected = Some((page, before));
    }
    if let Some((p, i)) = view.content.selected
        && on_page(added, p).len() <= i
    {
        view.content.selected = None;
    }
}

/// The in-place text editor, with Done and Discard beside it. Returns the edit once the text is
/// committed (a click away, or Done), and whether Done finished adding text. Discard or Escape
/// throws the draft away.
pub(crate) fn editor(ctx: &egui::Context, view: &mut DocView, info: &DocInfo, added: &[Added]) -> (Option<Edit>, bool) {
    let Some(page) = view.content.draft.as_ref().map(|d| d.page) else { return (None, false) };
    let Some(xf) = view.page_xform(page) else { return (None, false) };
    let zoom = xf.rect.width() / xf.pw.max(1.0);
    let Some(draft) = view.content.draft.as_mut() else { return (None, false) };
    let font = crate::edit_text_ui::editor_font(&draft.style, (draft.style.size as f32 * zoom).clamp(1.0, 300.0));
    let text = AddedText { text: draft.text.clone(), ..draft.style.clone() };
    let Some(page_width) = info.pages.get(page).map(|p| p.width as f64) else { return (None, false) };
    if let Some((key, z, width, rect)) = &view.content.layout
        && *key == text
        && *z == zoom
        && *width == page_width
    {
        draft.rect = *rect;
    } else {
        let source = draft.style.rect;
        if draft.index.is_some() && draft.text == draft.style.text {
            draft.rect = source;
        } else {
            let natural = ctx.fonts_mut(|f| f.layout_no_wrap(draft.text.clone(), font.clone(), Color32::BLACK).size().x) as f64 / zoom as f64;
            let width = natural.max(pdfcraft_engine::added_text_width(&text)) + 2.0;
            let available = page_width - source[0];
            let width = width.max(source[2] - source[0]).min(available.max(1.0));
            draft.rect = [source[0], source[1], source[0] + width, source[3]];
            draft.rect = pdfcraft_engine::added_text_rect(&AddedText { rect: draft.rect, ..text.clone() });
        }
        view.content.layout = Some((text, zoom, page_width, draft.rect));
    }
    let r = screen_rect(&xf, info, page, draft.rect);
    let draft_rect = draft.rect;
    let source_rect = draft.index.map(|_| screen_rect(&xf, info, page, draft.style.rect));
    let background_ready = draft.index.is_some_and(|index| view.text_background.ready_added(page, index));
    let tokens = Tokens::get(ctx);
    let (mut commit, mut cancel, mut done) = (false, false, false);
    let mut undrawable = None;
    let memo = &view.content.undrawable;
    egui::Area::new(egui::Id::new(("added-text", view.id.0))).order(egui::Order::Foreground).fixed_pos(r.min).constrain(false).show(ctx, |ui| {
        let Some(t) = view.content.draft.as_mut() else { return };
        ui.set_width(r.width() + 64.0);
        if let Some(source_rect) = source_rect.filter(|_| !background_ready) {
            ui.painter().rect_filled(source_rect.union(r).expand(1.0), CornerRadius::ZERO, Color32::WHITE);
        }
        // The box is white in every theme, so the caret must not follow the theme (pale in dark mode) (#655).
        ui.visuals_mut().text_cursor.stroke.color = crate::edit_text_ui::caret_color_on(Color32::WHITE);
        let [cr, cg, cb] = t.style.color.map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            // Typed Arabic and Hebrew read in display order, the rows of the box where the text's
            // alignment puts them, as the saved text will (`added.rs` draws them the same way).
            let align = row_align(t.style.align);
            let style = t.style.clone();
            let resp = ui.add(
                text_field(&mut t.text, &t.style, draft_rect, font)
                    .desired_width(r.width().max(60.0))
                    .background_color(if background_ready { Color32::TRANSPARENT } else { Color32::WHITE })
                    .text_color(Color32::from_rgb(cr, cg, cb))
                    .layout(move |ui, map, ctx| added_galley(ui, map, ctx, &style, zoom, align)),
            );
            if t.focus {
                resp.request_focus();
                t.focus = false;
            }
            let buttons = egui::Frame::NONE
                .fill(tokens.card)
                .stroke(Stroke::new(1.0, tokens.border))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(2)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
                    ui.horizontal(|ui| {
                        done = crate::icons::button(ui, "check", 24.0, false, tl!("Done adding text")).clicked();
                        cancel = crate::icons::button(ui, "x", 24.0, false, tl!("Discard this text (Esc)")).clicked();
                    });
                })
                .response
                .rect;
            // Pressing a button takes the focus from the text: that is not a click away.
            let on_buttons = ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| buttons.contains(p));
            cancel |= ui.input(|i| i.key_pressed(egui::Key::Escape));
            commit = done || resp.lost_focus() && !on_buttons && !view.objects.hold_editor;
        });
        // Page text is drawn with the standard fonts; anything else would be saved as `?` (#125).
        undrawable = self::undrawable(t, memo);
        if let Some(c) = undrawable {
            crate::widgets::small_text(ui, &undrawable_message(c), UNDRAWABLE);
        }
    });
    if cancel {
        view.content.draft = None;
        return (None, false);
    }
    // Text that can't be drawn stays in the editor, with the warning, rather than being lost.
    if !commit || undrawable.is_some() {
        return (None, false);
    }
    let edit = view.content.draft.take().and_then(|d| finish(&mut view.content, d, added));
    (edit, done)
}

/// The Add text box's field: the text breaks into lines where the saved item will, which is at the first word
/// that makes a line wider than the box in the fonts the page writes it in (the standard font for Latin, the
/// shaped Arabic or Hebrew face), and not where the interface font would. `rect` is the box in points.
///
/// The field has no margin: the saved text starts at the box's left edge and wraps at its width, so the
/// field's inner width is the box's. Wrapping is in points (the field is told the box's width and measures
/// lines in points), so it does not depend on the zoom. A word wider than the box stays whole on its line,
/// and only the space ends a word, as the page does it.
fn text_field<'t>(text: &'t mut String, style: &AddedText, rect: [f64; 4], font: egui::FontId) -> widgets::BidiEdit<'t> {
    let meter = pdfcraft_engine::AddedLineMeasure::new(style);
    // The box's width as `added.rs` wraps at it: never narrower than one em.
    let box_width = ((rect[2] - rect[0]).max(style.size)) as f32;
    // Thai is broken between its clusters by the page, which a field cannot follow word by word.
    let thai = text.chars().any(|c| ('\u{0E00}'..='\u{0E7F}').contains(&c));
    widgets::BidiEdit::multiline(text)
        .font(font)
        .margin(egui::Margin::ZERO)
        .desired_rows(1)
        .hint_text(tl!("Type text"))
        .id_salt("added-text-edit")
        .wrap_ltr(true)
        .wrap_key((style.family.label(), style.bold, style.italic, style.size.to_bits()))
        .wrap_at(box_width)
        .measure_in(move |_, candidate| meter.width(candidate.paragraph, candidate.line) as f32)
        .wrap_rules(crate::bidi::WrapRules { whole_words: !thai, spaces_only: true })
}

/// The galley of the Add text box: the rows `map` has, drawn in the interface font but fitted to the width the page
/// writes them at, set at the pitch the page uses, and placed by the text's alignment.
///
/// The interface font is wider than the standard font the page writes Latin in (Inter against Helvetica), so a
/// row with no right-to-left script gets letter spacing that makes it as wide as the page's line: it then ends where
/// the page's line does, and the editor's rows can be compared with the saved ones. The page's Arabic, Hebrew and the
/// words around them are drawn in the same face as the interface's, and are left as they are.
fn added_galley(
    ui: &egui::Ui,
    map: &crate::bidi::ScreenMap,
    ctx: &widgets::LayoutCtx<'_>,
    style: &AddedText,
    zoom: f32,
    align: widgets::RowAlign,
) -> std::sync::Arc<egui::Galley> {
    let mut job = ctx.job(map);
    if let Some(base) = job.sections.first().map(|section| section.format.clone()) {
        let pitch = style.size as f32 * 1.2 * zoom;
        let mut sections = Vec::new();
        let mut start: usize = 0;
        let section = |from: usize, to: usize, format: egui::TextFormat| egui::text::LayoutSection {
            leading_space: 0.0,
            byte_range: egui::text::ByteIndex(from)..egui::text::ByteIndex(to),
            format,
        };
        for row in job.text.split_inclusive('\n') {
            let end = start.saturating_add(row.len());
            let ink = row.trim_end_matches('\n').trim_matches(' ');
            let count = ink.chars().count();
            let mut format = base.clone();
            format.line_height = Some(pitch).filter(|p| p.is_finite() && *p > 0.0).or(format.line_height);
            // The spacing goes between the letters of the ink (epaint puts it before every glyph but the first of the
            // row): what follows the ink (the space a wrap leaves) keeps the font's own advance, so the ink ends where
            // the page's line does.
            let mut split = None;
            if count > 1 && !crate::bidi::has_rtl(row) && pdfcraft_fonts::first_non_win_ansi(ink).is_none() {
                let natural = ui.fonts_mut(|f| f.layout_no_wrap(ink.to_owned(), format.font_id.clone(), egui::Color32::WHITE).size().x);
                let written = style.family.width(ink, style.size, style.bold, style.italic) as f32 * zoom;
                // At most a quarter of an em either way: a font nothing like the page's is not squeezed out of shape.
                let reach = format.font_id.size * 0.25;
                let spacing = (written - natural) / (count - 1) as f32;
                if spacing.is_finite() {
                    format.extra_letter_spacing = spacing.clamp(-reach, reach);
                    split = Some(start.saturating_add(row.trim_end_matches('\n').trim_end_matches(' ').len()));
                }
            }
            match split {
                Some(at) if at < end => {
                    let mut tail = format.clone();
                    tail.extra_letter_spacing = 0.0;
                    sections.push(section(start, at, format));
                    sections.push(section(at, end, tail));
                }
                _ => sections.push(section(start, end, format)),
            }
            start = end;
        }
        if !sections.is_empty() {
            job.sections = sections;
        }
    }
    let galley = ctx.lay_out(ui, map, job);
    ctx.place_aligned(ui, galley, map, align)
}

/// Where the editor puts the rows of the text: the alignment the saved text will have.
fn row_align(align: TextAlign) -> widgets::RowAlign {
    match align {
        TextAlign::Left => widgets::RowAlign::Left,
        TextAlign::Center => widgets::RowAlign::Center,
        TextAlign::Right => widgets::RowAlign::Right,
        TextAlign::Justify => widgets::RowAlign::Justify,
    }
}

/// A finished draft as an edit: new text is added (and selected once it exists), retyped text
/// updated, and text emptied by retyping removed. Empty new text adds nothing.
fn finish(cv: &mut ContentView, d: TextDraft, added: &[Added]) -> Option<Edit> {
    let text = d.text.trim_end().to_string();
    match d.index {
        None if text.trim().is_empty() => None,
        None => {
            cv.select_added = Some((d.page, on_page(added, d.page).len()));
            Some(Edit::AddText { page: d.page, text: AddedText { rect: d.rect, text, ..d.style } })
        }
        Some(i) if text.trim().is_empty() => Some(Edit::DeleteContent { page: d.page, index: i }),
        Some(i) => {
            let (_, a) = on_page(added, d.page).into_iter().find(|(k, _)| *k == i)?;
            let AddedContent::Text(old) = &a.content else { return None };
            let updated = AddedText { text, rect: d.rect, ..d.style };
            (*old != updated).then_some(Edit::UpdateContent { page: d.page, index: i, content: AddedContent::Text(updated) })
        }
    }
}

/// Format text (shown in the Edit panel while a text item is selected, or for new text).
/// Returns the changed style; for a selected item the caller turns it into an update.
pub(crate) fn format_panel(ui: &mut egui::Ui, t: &Tokens, style: &AddedText) -> Option<AddedText> {
    let mut s = style.clone();
    widgets::section_title(ui, tl!("Format text"));
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("font-family").selected_text(s.family.label()).width(110.0).show_ui(ui, |ui| {
            for f in [FontFamily::Helvetica, FontFamily::Times, FontFamily::Courier] {
                ui.selectable_value(&mut s.family, f, f.label());
            }
        });
        // A list rather than a drag value: every change is an undo step.
        egui::ComboBox::from_id_salt("font-size").selected_text(format!("{} pt", s.size)).width(70.0).show_ui(ui, |ui| {
            for size in [8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 24.0, 28.0, 36.0, 48.0, 72.0] {
                ui.selectable_value(&mut s.size, size, format!("{size} pt"));
            }
        });
    });
    ui.horizontal(|ui| {
        if ui.selectable_label(s.bold, egui::RichText::new("B").strong()).on_hover_text(tl!("Bold")).clicked() {
            s.bold = !s.bold;
        }
        if ui.selectable_label(s.italic, egui::RichText::new("I").italics()).on_hover_text(tl!("Italic")).clicked() {
            s.italic = !s.italic;
        }
        ui.separator();
        for (a, icon, tip) in [
            (TextAlign::Left, "align-left", tl!("Align left")),
            (TextAlign::Center, "align-center", tl!("Centre")),
            (TextAlign::Right, "align-right", tl!("Align right")),
            (TextAlign::Justify, "align-justify", tl!("Justify")),
        ] {
            if crate::icons::button(ui, icon, 26.0, s.align == a, tip).clicked() {
                s.align = a;
            }
        }
    });
    let c = s.color;
    if let Some(picked) = crate::comments::swatch_grid(ui, Some(c)) {
        s.color = picked;
    }
    ui.label(egui::RichText::new(tl!("Thai and Arabic use regular fallback fonts when available.")).small().color(t.text_faint));
    (s != *style).then_some(s)
}

/// How to use the tools, under the Add content list.
pub(crate) fn hint(ui: &mut egui::Ui, t: &Tokens) {
    ui.label(
        egui::RichText::new(
            tl!("Click on the page to add text; each click starts a new box, and ✓ finishes. Drag items to move them, drag a corner to resize, double-click text to edit it."),
        )
            .small()
            .color(t.text_faint),
    );
    ui.add_space(4.0);
}

impl crate::PdfCraftApp {
    /// Pick an image file for the active document, read it, and call `then` with its name and
    /// bytes: now when `save_override` answers (tests and automation never see a native dialog),
    /// otherwise on a later frame, and only if that document is still the active one.
    #[cfg(not(target_arch = "wasm32"))]
    fn pick_image(&mut self, title: &str, then: impl FnOnce(&mut Self, String, Vec<u8>) + Send + 'static) {
        let read = |app: &mut Self, path: std::path::PathBuf| match std::fs::read(&path) {
            Ok(bytes) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                then(app, name, bytes);
            }
            Err(e) => app.notify_fmt("Couldn't read {name}: {e}", &[("name", &path.display().to_string()), ("e", &e.to_string())]),
        };
        match self.save_override.clone() {
            Some(p) if p.ends_with(".png") || p.ends_with(".jpg") => read(self, p.into()),
            Some(_) => {}
            None => {
                let dialog = rfd::AsyncFileDialog::new()
                    .add_filter(tl!("Images"), &["png", "jpg", "jpeg", "tif", "tiff", "gif", "bmp", "jp2", "j2k", "jpx"])
                    .set_title(title);
                let target = self.active_ids().map(|(_, id)| id);
                self.ask_one(crate::pickers::Ask::File(dialog), target, read);
            }
        }
    }

    /// Edit a PDF ▸ Add content ▸ Image: pick a file and place it in the middle of the current page.
    pub fn add_image_dialog(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        self.pick_image(tl!("Choose an image"), |app, name, bytes| app.add_image(name, bytes));
        #[cfg(target_arch = "wasm32")]
        self.notify_tr("Adding images arrives on the web with file pickers for images");
    }

    /// Edit text & images ▸ Replace Image: pick a file to draw in a page image's place.
    pub(crate) fn replace_page_image_dialog(&mut self, page: usize, index: usize) {
        #[cfg(not(target_arch = "wasm32"))]
        self.pick_image(tl!("Replace image"), move |app, name, bytes| {
            app.apply_edit(Edit::EditPageImage {
                page,
                index,
                change: pdfcraft_engine::ImageEdit::Replace { name, bytes: std::sync::Arc::new(bytes) },
            });
        });
        #[cfg(target_arch = "wasm32")]
        self.notify_fmt(
            "Replacing images on page {p} arrives on the web with image pickers ({i})",
            &[("p", &(page + 1).to_string()), ("i", &index.to_string())],
        );
    }

    /// Edit text & images ▸ Save Image As.
    pub(crate) fn save_page_image(&mut self, page: usize, index: usize) {
        let Some((_, id)) = self.active_ids() else { return };
        if !self.may_copy(id) {
            return;
        }
        let file = self.session.get(id).map(|d| d.page_image_file(page, index));
        match file {
            Some(Ok((ext, bytes))) => {
                self.write_files(&[(format!("Image page {} #{}.{ext}", page + 1, index + 1), std::sync::Arc::new(bytes))], "Save image");
            }
            Some(Err(e)) => self.notify_fmt("Couldn't save the image: {e}", &[("e", &e.to_string())]),
            None => {}
        }
    }

    /// Copy Image: image `index` on `page`, at its own resolution, to the clipboard.
    pub(crate) fn copy_page_image(&mut self, page: usize, index: usize) -> bool {
        let Some((_, id)) = self.active_ids() else { return false };
        if !self.may_copy(id) {
            return false;
        }
        let Some(pixels) = self.session.get(id).map(|d| d.page_image_rgba(page, index)) else { return false };
        let copied = pixels.and_then(|(w, h, rgba)| {
            #[cfg(not(target_arch = "wasm32"))]
            if self.system_clipboard {
                let img = arboard::ImageData { width: w as usize, height: h as usize, bytes: std::borrow::Cow::Borrowed(&rgba) };
                arboard::Clipboard::new().and_then(|mut c| c.set_image(img)).map_err(|e| e.to_string())?;
            }
            #[cfg(target_arch = "wasm32")]
            if self.system_clipboard {
                return Err("the web version can't put images on the clipboard yet; use Save Image As".to_string());
            }
            self.last_copied_image = Some((w, h, rgba));
            Ok((w, h))
        });
        match copied {
            Ok((w, h)) => {
                self.notify_fmt("The image has been copied ({w} × {h} pixels)", &[("w", &w.to_string()), ("h", &h.to_string())]);
                true
            }
            Err(e) => {
                self.notify_fmt("Couldn't copy the image: {e}", &[("e", &e)]);
                false
            }
        }
    }

    /// The picture's visible box on its page, in user space.
    fn page_image_box(&self, page: usize, index: usize) -> Option<[f64; 4]> {
        let (_, id) = self.active_ids()?;
        let image = self.session.get(id)?.page_images(page).into_iter().nth(index)?;
        Some(image.visible_rect.unwrap_or(image.rect))
    }

    /// Cut: copy the picture, and only when that worked take it off the page (one undo step).
    pub(crate) fn cut_page_image(&mut self, page: usize, index: usize) {
        if self.copy_page_image(page, index) {
            self.apply_edit(Edit::EditPageImage { page, index, change: pdfcraft_engine::ImageEdit::Delete });
        }
    }

    /// Redact image: a redaction mark over the whole picture. It is applied, like any mark, from
    /// Redact a PDF.
    pub(crate) fn redact_page_image(&mut self, page: usize, index: usize) {
        let Some(rect) = self.page_image_box(page, index) else { return };
        let author = self.comment_prefs.author.clone();
        let mark = self.redact_prefs.mark(page, vec![pdfcraft_engine::rect_quad(rect)], &author);
        if self.apply_edit(mark) {
            self.notify_tr("The image is marked for redaction. Apply it with Redact a PDF");
        }
    }

    /// Create link: the link properties for a new link over the picture.
    pub(crate) fn link_page_image(&mut self, page: usize, index: usize) {
        if let Some(rect) = self.page_image_box(page, index) {
            self.open_link_props(page, Some(rect), None);
        }
    }

    /// Recognize text: the Recognize Text dialog, set to this page.
    pub fn recognize_text_on_page(&mut self, page: usize) {
        self.ocr_draft.pages = crate::ocr_ui::OcrPages::Range;
        self.ocr_draft.from = page + 1;
        self.ocr_draft.to = page + 1;
        self.dialog = Some(crate::Dialog::RecognizeText);
    }

    /// Click an image field: pick a picture for it.
    pub fn choose_field_image(&mut self, name: &str) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let name = name.to_string();
            self.pick_image(tl!("Select Icon"), move |app, _, bytes| {
                app.apply_edit(Edit::SetFieldImage { name, image: std::sync::Arc::new(bytes) });
            });
        }
        #[cfg(target_arch = "wasm32")]
        self.notify_fmt("{name}: choosing images arrives on the web with file pickers for images", &[("name", &name)]);
    }

    /// Edit image ▸ Replace: pick a file for the selected image.
    pub fn replace_image_dialog(&mut self, page: usize, index: usize) {
        #[cfg(not(target_arch = "wasm32"))]
        self.pick_image(tl!("Replace image"), move |app, name, bytes| {
            app.apply_edit(Edit::ReplaceImage { page, index, name, bytes: std::sync::Arc::new(bytes) });
        });
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (page, index);
            self.notify_tr("Replacing images arrives on the web with image file pickers");
        }
    }

    /// Place image `bytes` in the middle of the current page and select it.
    pub fn add_image(&mut self, name: String, bytes: Vec<u8>) {
        let Some((i, id)) = self.active_ids() else { return };
        let page = self.views[i].current;
        let before = self.session.get(id).map_or(0, |d| d.added().iter().filter(|a| a.page == page).count());
        if self.apply_edit(Edit::AddImage { page, rect: None, name, bytes: std::sync::Arc::new(bytes) }) {
            self.views[i].content.select_added = Some((page, before));
            self.left = crate::LeftPanel::Tool("edit");
            self.left_open = true;
        }
    }
}

/// What the image tools ask for.
pub(crate) enum ImageAction {
    Update(pdfcraft_engine::AddedContent),
    Replace,
}

// Lucide names a flip icon after the axis it draws, Acrobat names the flip after the direction the
// picture moves: Flip horizontal (left and right swap) shows Lucide's vertical axis.
const FLIP_HORIZONTAL_ICON: &str = "flip-vertical-2";
const FLIP_VERTICAL_ICON: &str = "flip-horizontal-2";

/// Edit image: rotate, flip, crop, replace (shown while an added image is selected).
pub(crate) fn image_panel(ui: &mut egui::Ui, t: &Tokens, img: &pdfcraft_engine::AddedImage) -> Option<ImageAction> {
    let mut out = None;
    widgets::section_title(ui, tl!("Edit image"));
    ui.horizontal(|ui| {
        let mut i = img.clone();
        let r = i.rect;
        let turn = |i: &mut pdfcraft_engine::AddedImage, k: u8| {
            i.rotation = (i.rotation + k) % 4;
            // The box turns with the picture, around its centre.
            let (cx, cy, w, h) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0, r[2] - r[0], r[3] - r[1]);
            i.rect = [cx - h / 2.0, cy - w / 2.0, cx + h / 2.0, cy + w / 2.0];
        };
        if crate::icons::button(ui, "rotate-ccw", 28.0, false, tl!("Rotate counterclockwise")).clicked() {
            turn(&mut i, 1);
            out = Some(ImageAction::Update(AddedContent::Image(i.clone())));
        }
        if crate::icons::button(ui, "rotate-cw", 28.0, false, tl!("Rotate clockwise")).clicked() {
            turn(&mut i, 3);
            out = Some(ImageAction::Update(AddedContent::Image(i.clone())));
        }
        if crate::icons::button(ui, FLIP_HORIZONTAL_ICON, 28.0, false, tl!("Flip horizontal")).clicked() {
            i.flip_h = !i.flip_h;
            out = Some(ImageAction::Update(AddedContent::Image(i.clone())));
        }
        if crate::icons::button(ui, FLIP_VERTICAL_ICON, 28.0, false, tl!("Flip vertical")).clicked() {
            i.flip_v = !i.flip_v;
            out = Some(ImageAction::Update(AddedContent::Image(i.clone())));
        }
        if crate::icons::button(ui, "replace", 28.0, false, tl!("Replace image")).clicked() {
            out = Some(ImageAction::Replace);
        }
    });
    ui.label(egui::RichText::new(tl!("Crop (% trimmed from each side)")).small().color(t.text_muted));
    let mut crop = img.crop.map(|v| (v * 100.0).round());
    let mut changed = false;
    ui.horizontal(|ui| {
        for (k, label) in ["L", "B", "R", "T"].into_iter().enumerate() {
            ui.label(label);
            // Applied when the drag ends, so a drag is one undo step.
            let r = ui.add(egui::DragValue::new(&mut crop[k]).range(0.0..=45.0).speed(0.5).suffix("%"));
            changed |= r.drag_stopped() || (r.changed() && !r.dragged());
        }
    });
    if changed {
        let mut i = img.clone();
        i.crop = crop.map(|v| v / 100.0);
        if i.crop != img.crop {
            out = Some(ImageAction::Update(AddedContent::Image(i)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;

    use super::*;

    /// The image a flip button leaves behind, as the panel returns it.
    fn flipped(tooltip: &str) -> pdfcraft_engine::AddedImage {
        let img = pdfcraft_engine::AddedImage::new([0.0, 0.0, 100.0, 50.0], pdfcraft_cos::ObjRef::new(7, 0));
        let mut fonts = false;
        let mut h = egui_kittest::Harness::new_ui_state(
            move |ui, out: &mut Option<pdfcraft_engine::AddedImage>| {
                // The app's fonts (the semibold section title) take effect from the next frame.
                if !fonts {
                    ui.ctx().set_fonts(crate::theme::font_definitions());
                    fonts = true;
                    return;
                }
                let t = Tokens::get(ui.ctx());
                if let Some(ImageAction::Update(AddedContent::Image(i))) = image_panel(ui, &t, &img) {
                    *out = Some(i);
                }
            },
            None,
        );
        h.run();
        h.get_by_label(tooltip).click();
        h.run();
        h.state().clone().expect("the button updates the image")
    }

    /// The lines the Add text field breaks `item`'s text into at the box `item.rect` has, at `zoom`, in the reading order of
    /// the stored text (what each row holds, whichever order its letters are drawn in).
    fn field_lines(fonts: egui::FontDefinitions, item: &AddedText, zoom: f32) -> Vec<String> {
        let ctx = egui::Context::default();
        ctx.set_os(egui::os::OperatingSystem::Nix);
        ctx.set_fonts(fonts);
        let mut lines = Vec::new();
        // The fonts take effect from the second frame.
        for _ in 0..2 {
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(4000.0, 2000.0))), ..Default::default() };
            let mut text = item.text.clone();
            let frame = ctx.run_ui(raw, |ui| {
                let width = (item.rect[2] - item.rect[0]).max(item.size) as f32 * zoom;
                let font = egui::FontId::proportional(item.size as f32 * zoom);
                let out = text_field(&mut text, item, item.rect, font).desired_width(width).show_mapped(ui);
                let chars: Vec<char> = item.text.chars().collect();
                // The stored characters on each row of the shaper text, from the first to the last.
                let source = out.map.shaper_source();
                let mut rows: Vec<Vec<usize>> = vec![Vec::new()];
                for (c, at) in out.map.shaper.chars().zip(source) {
                    if c == '\n' && at == crate::bidi::VIRTUAL {
                        rows.push(Vec::new());
                    } else if at != crate::bidi::VIRTUAL {
                        rows.last_mut().expect("a row").push(at);
                    }
                }
                lines = rows
                    .iter()
                    .map(|row| match (row.iter().min(), row.iter().max()) {
                        (Some(low), Some(high)) => chars.get(*low..=*high).unwrap_or_default().iter().collect::<String>(),
                        _ => String::new(),
                    })
                    .collect();
            });
            frame.drop_without_applying_deltas();
            assert_eq!(text, item.text, "looking at the field does not change the text");
        }
        lines
    }

    fn assert_field_matches_page(fonts: &egui::FontDefinitions, item: &AddedText, zoom: f32) {
        // A paragraph's own line breaks (`\n`) end a row in the stored text too; the page's lines have none.
        let trim = |lines: Vec<String>| lines.into_iter().map(|l| l.trim().to_owned()).collect::<Vec<_>>();
        let page = trim(pdfcraft_engine::added_text_lines(item));
        let field =
            trim(field_lines(fonts.clone(), item, zoom).into_iter().flat_map(|l| l.split('\n').map(str::to_owned).collect::<Vec<_>>()).collect());
        assert_eq!(field, page, "box {:?}, {:?} {} pt at {zoom}: {:?}", item.rect, item.family, item.size, item.text);
    }

    #[test]
    fn the_add_text_field_breaks_latin_lines_where_the_engine_measures_them() {
        let text = "The quick brown fox jumps over the lazy dog,  and nine wizards quickly box jumpers.\nA second paragraph with WWW iii mmm words.";
        for family in [FontFamily::Helvetica, FontFamily::Times, FontFamily::Courier] {
            for size in [9.0, 12.0, 18.0] {
                for zoom in [1.0, 1.5] {
                    for step in 0..30 {
                        let width = 40.0 + f64::from(step) * 9.7;
                        let item = AddedText { rect: [20.0, 0.0, 20.0 + width, 50.0], text: text.into(), family, size, ..AddedText::default() };
                        assert_field_matches_page(&crate::theme::font_definitions(), &item, zoom);
                    }
                }
            }
        }
    }

    #[test]
    fn the_add_text_field_breaks_arabic_and_mixed_lines_where_the_engine_measures_them() {
        let Some(fonts) = crate::bidi::arabic_test_fonts() else { return };
        for text in [
            "سلام عليكم ورحمة الله وبركاته مع أطيب التحيات من الجميع هنا اليوم",
            "Hello سلام PDF عليكم 2026 ورحمة الله Acrobat وبركاته مع أطيب",
            "English first then عربي في الآخر مع كلمات أخرى كثيرة\nسطر ثان بعد سطر",
        ] {
            for size in [10.0, 14.0, 20.0] {
                for zoom in [1.0, 1.5] {
                    for step in 0..25 {
                        let width = 50.0 + f64::from(step) * 11.3;
                        let item = AddedText { rect: [20.0, 0.0, 20.0 + width, 50.0], text: text.into(), size, ..AddedText::default() };
                        assert_field_matches_page(&fonts, &item, zoom);
                    }
                }
            }
        }
    }

    #[test]
    fn a_word_wider_than_the_box_stays_whole_and_only_spaces_end_a_word() {
        let fonts = crate::theme::font_definitions();
        // The page keeps a long word whole on its own line (it overflows the box); a no-break space is no space.
        let item = AddedText { rect: [0.0, 0.0, 30.0, 50.0], text: "ab Antidisestablishmentarianism cd\u{a0}ef gh".into(), ..AddedText::default() };
        let lines = field_lines(fonts.clone(), &item, 1.0);
        assert!(lines.iter().any(|l| l.trim() == "Antidisestablishmentarianism"), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("cd\u{a0}ef")), "{lines:?}");
        assert_field_matches_page(&fonts, &item, 1.0);
    }

    #[test]
    fn flip_buttons_show_the_mirror_axis_of_the_flip_they_make() {
        // #1834: Acrobat's Flip horizontal swaps left and right across a vertical axis. Lucide names
        // its flip icons after the axis drawn, so that is its flip-vertical-2.
        let svg = |name: &str| {
            let (_, bytes) = crate::icon_data::ICONS.iter().find(|(n, _)| *n == name).expect("bundled icon");
            String::from_utf8_lossy(bytes).into_owned()
        };
        let (h, v) = (svg(FLIP_HORIZONTAL_ICON), svg(FLIP_VERTICAL_ICON));
        assert!(h.contains("M12 2v2") && !h.contains("M4 12H2"), "Flip horizontal draws a vertical axis:\n{h}");
        assert!(v.contains("M4 12H2") && !v.contains("M12 2v2"), "Flip vertical draws a horizontal axis:\n{v}");

        let i = flipped("Flip horizontal");
        assert!(i.flip_h && !i.flip_v, "Flip horizontal mirrors left and right");
        let i = flipped("Flip vertical");
        assert!(i.flip_v && !i.flip_h, "Flip vertical mirrors top and bottom");
    }
}
