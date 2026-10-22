//! The vertical scrollbar of a scroll area, at the start edge when the interface is mirrored.
//!
//! egui 0.36 puts a vertical bar at the right edge of every `ScrollArea` and has no setting for the
//! side: its x is `outer_rect.max.x - outer_margin`, and `ScrollArea::scroll_bar_rect` only
//! narrows the bar along its length. So a mirrored area hides egui's bar
//! ([`ScrollBarVisibility::AlwaysHidden`]; the mouse wheel and the keyboard still scroll it) and
//! [`scroll_area`] paints a thin floating bar of its own at the left edge of the viewport,
//! from the geometry the area reports ([`ScrollAreaOutput`]). It follows the style's floating bar
//! (`Spacing::scroll`): thin until the pointer is over it, then `bar_width` wide, the handle and its
//! track fading in with the pointer as egui's do, and dragging the handle (or pressing in the track,
//! which centres the handle there) sets the area's stored offset.
//!
//! English never goes through any of this: [`scroll_area`] is `ScrollArea::show` there. A vertical
//! area gets the start-edge bar only; a horizontal bar stays where egui puts it. An area that
//! scrolls both ways (the document view, [`scroll_area_with`]) cannot keep egui's bottom bar once
//! egui's bars are hidden (the visibility is one setting for both axes), so there the horizontal
//! bar is drawn by the same code, at the bottom edge where egui has it.

use egui::scroll_area::{ScrollAreaOutput, State};
use egui::{Id, Pos2, Rangef, Rect, ScrollArea, Sense, Ui, Vec2, WidgetInfo, WidgetType, lerp, pos2, remap, remap_clamp};

use crate::rtl;

/// Show `area` in `ui`, with its vertical scrollbar at the start edge: the right in English (egui's
/// own bar, untouched), the left when the interface is mirrored.
pub fn scroll_area<R>(ui: &mut Ui, area: ScrollArea, add_contents: impl FnOnce(&mut Ui) -> R) -> ScrollAreaOutput<R> {
    scroll_area_with(ui, area, false, |area, ui| area.show(ui, add_contents))
}

/// [`scroll_area`] for an area that `show` shows in its own way (`show_viewport`, say), and that
/// scrolls sideways too when `sideways` is true. English runs `show(area, ui)` as it is. Mirrored,
/// egui's bars are hidden and the vertical bar is drawn at the left edge; for an area that
/// scrolls sideways the bottom bar is drawn as well, since hiding egui's bars hides both. The
/// area's scrolling (wheel, keys, dragging its content, the offsets set on it) is egui's own.
pub fn scroll_area_with<R>(
    ui: &mut Ui,
    area: ScrollArea,
    sideways: bool,
    show: impl FnOnce(ScrollArea, &mut Ui) -> ScrollAreaOutput<R>,
) -> ScrollAreaOutput<R> {
    if !rtl::mirrored() {
        return show(area, ui);
    }
    let output = show(area.scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden), ui);
    paint_bar(ui, &output, Axis::Y);
    if sideways {
        paint_bar(ui, &output, Axis::X);
    }
    output
}

/// The direction a bar scrolls its area in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    /// A horizontal bar, along the bottom edge of the viewport.
    X,
    /// A vertical bar, along the left edge of the viewport (the start edge when mirrored).
    Y,
}

impl Axis {
    /// The component of `v` along this axis.
    fn of(self, v: Vec2) -> f32 {
        match self {
            Self::X => v.x,
            Self::Y => v.y,
        }
    }

    /// The coordinate of `p` along this axis.
    fn at(self, p: Pos2) -> f32 {
        match self {
            Self::X => p.x,
            Self::Y => p.y,
        }
    }

    /// The extent of `rect` along this axis: where a bar's track runs.
    fn track(self, rect: Rect) -> Rangef {
        match self {
            Self::X => rect.x_range(),
            Self::Y => rect.y_range(),
        }
    }

    /// The extent of `rect` across this axis: how thick a bar in it is.
    fn thickness(self, rect: Rect) -> f32 {
        match self {
            Self::X => rect.height(),
            Self::Y => rect.width(),
        }
    }

    /// Set the component of `v` along this axis.
    fn set(self, v: &mut Vec2, value: f32) {
        match self {
            Self::X => v.x = value,
            Self::Y => v.y = value,
        }
    }

    /// The strip of the viewport `view` that senses the pointer for this axis's bar, `width` thick:
    /// against the left edge or the bottom edge, nudged inwards if an enclosing scroll area has
    /// scrolled that edge out of view (`clip` is the clip rectangle of the enclosing `ui`).
    fn strip(self, view: Rect, clip: Rect, width: f32) -> Rect {
        match self {
            Self::Y => {
                let left = view.left().max(clip.left()).min(view.right());
                Rect::from_min_max(pos2(left, view.top()), pos2((left + width).min(view.right()), view.bottom()))
            }
            Self::X => {
                let bottom = view.bottom().min(clip.bottom()).max(view.top());
                Rect::from_min_max(pos2(view.left(), (bottom - width).max(view.top())), pos2(view.right(), bottom))
            }
        }
    }

    /// The bar drawn in `strip` when it is `thickness` thick: it grows from the edge inwards.
    fn bar(self, strip: Rect, thickness: f32) -> Rect {
        match self {
            Self::Y => Rect::from_min_max(strip.min, pos2(strip.min.x + thickness, strip.max.y)),
            Self::X => Rect::from_min_max(pos2(strip.min.x, strip.max.y - thickness), strip.max),
        }
    }

    /// The handle within `bar`, spanning `span` along the track.
    fn handle(self, bar: Rect, span: Rangef) -> Rect {
        match self {
            Self::Y => Rect::from_min_max(pos2(bar.min.x, span.min), pos2(bar.max.x, span.max)),
            Self::X => Rect::from_min_max(pos2(span.min, bar.min.y), pos2(span.max, bar.max.y)),
        }
    }
}

/// The id of the mirrored bar of the scroll area `area_id` along `axis`.
fn bar_id(area_id: Id, axis: Axis) -> Id {
    match axis {
        Axis::Y => area_id.with("start_scroll_bar"),
        Axis::X => area_id.with("bottom_scroll_bar"),
    }
}

/// Where the start-edge bar of the scroll area `area_id` was last laid out (the strip that senses
/// the pointer, at its full width), or `None` where the area shows egui's own bar or needs none.
#[cfg(test)]
pub(crate) fn start_bar_rect(ctx: &egui::Context, area_id: Id) -> Option<Rect> {
    ctx.read_response(bar_id(area_id, Axis::Y)).map(|r| r.rect)
}

/// The same for the bottom bar of an area that scrolls sideways.
#[cfg(test)]
pub(crate) fn bottom_bar_rect(ctx: &egui::Context, area_id: Id) -> Option<Rect> {
    ctx.read_response(bar_id(area_id, Axis::X)).map(|r| r.rect)
}

/// The vertical extent of the handle of a bar whose track spans `track`, for a viewport `view` tall
/// onto content `content` tall scrolled by `offset`; `None` where nothing scrolls (or a size is not
/// a finite positive number). The handle is at least `min_len` long but never longer than the track.
fn handle_span(track: Rangef, content: f32, view: f32, offset: f32, min_len: f32) -> Option<Rangef> {
    let length = track.max - track.min;
    if ![content, view, offset, min_len, track.min, track.max].iter().all(|v| v.is_finite()) || content <= view || view <= 0.0 || length <= 0.0 {
        return None;
    }
    let max_offset = content - view;
    let len = (length * view / content).max(min_len).min(length);
    let top = remap_clamp(offset, 0.0..=max_offset, track.min..=(track.max - len));
    Some(Rangef::new(top, top + len))
}

/// The scroll offset that puts the top of a handle `len` long at `top` on a track spanning `track`,
/// for content that scrolls by at most `max_offset`.
fn offset_for_handle_top(top: f32, track: Rangef, len: f32, max_offset: f32) -> f32 {
    let travel = track.max - len - track.min;
    if !travel.is_finite() || travel <= 0.0 || !top.is_finite() || !max_offset.is_finite() {
        return 0.0;
    }
    remap(top, track.min..=(track.max - len), 0.0..=max_offset).clamp(0.0, max_offset.max(0.0))
}

/// Paint the bar of `output`'s area along `axis` (at the left or the bottom edge of its viewport),
/// and let the pointer drag it.
fn paint_bar<R>(ui: &mut Ui, output: &ScrollAreaOutput<R>, axis: Axis) {
    let (view, content) = (output.inner_rect, output.content_size);
    let (view_len, content_len) = (axis.of(view.size()), axis.of(content));
    let style = ui.spacing().scroll;
    // The strip egui would sense, at the other edge: its full width along the viewport's edge.
    let strip = axis.strip(view, ui.clip_rect(), style.bar_width);
    let track = axis.track(view);
    let offset_now = axis.of(output.state.offset);
    if handle_span(track, content_len, view_len, offset_now, style.handle_min_length).is_none() || !ui.is_rect_visible(strip) {
        return;
    }

    let id = bar_id(output.id, axis);
    let enabled = ui.is_enabled();
    let response = ui.interact(strip, id, if enabled { Sense::click_and_drag() } else { Sense::hover() });
    response.widget_info(|| WidgetInfo::new(WidgetType::ScrollBar));

    let mut offset = offset_now;
    let max_offset = content_len - view_len;
    let min_len = style.handle_min_length;
    let grab_id = id.with("grab");
    if let Some(pointer) = response.interact_pointer_pos() {
        if let Some(handle) = handle_span(track, content_len, view_len, offset, min_len) {
            let len = handle.max - handle.min;
            let along = axis.at(pointer);
            // Where on the handle the pointer holds it: where it first pressed, or its middle when
            // it pressed in the track (the handle jumps to centre on the pointer).
            let grab = ui.data_mut(|d| {
                *d.get_temp_mut_or_insert_with(grab_id, || {
                    if handle.contains(along) {
                        along - handle.min
                    } else {
                        let top = (along - len / 2.0).clamp(track.min, (track.max - len).max(track.min));
                        along - top
                    }
                })
            });
            offset = offset_for_handle_top(along - grab, track, len, max_offset);
        }
    } else {
        ui.data_mut(|d| d.remove::<f32>(grab_id));
    }
    let offset = if offset.is_finite() { offset.clamp(0.0, max_offset) } else { offset_now };
    if (offset - offset_now).abs() > f32::EPSILON {
        // Stored for the next frame, as egui's own bar does it (a scroll area reads its offset
        // from its stored state when it starts).
        let mut state: State = State::load(ui.ctx(), output.id).unwrap_or(output.state);
        axis.set(&mut state.offset, offset);
        state.store(ui.ctx(), output.id);
        ui.request_repaint();
    }

    // Paint with the new offset: no frame behind the pointer.
    let Some(handle) = handle_span(track, content_len, view_len, offset, min_len) else { return };
    let hovering_bar = response.hovered() || response.dragged();
    let grown = ui.ctx().animate_bool_responsive(id.with("hover"), hovering_bar);
    let thickness = lerp(style.floating_width..=style.bar_width, grown).min(axis.thickness(strip));
    let bar = axis.bar(strip, thickness);
    let handle_rect = axis.handle(bar, handle);

    let over_area = ui.rect_contains_pointer(view) && ui.ctx().dragged_id().is_none();
    let over_handle = response.hovered() && ui.input(|i| i.pointer.latest_pos().is_some_and(|p| handle_rect.contains(p)));
    let visuals = ui.visuals();
    let widget = if !enabled {
        &visuals.widgets.inactive
    } else if response.is_pointer_button_down_on() {
        &visuals.widgets.active
    } else if over_handle {
        &visuals.widgets.hovered
    } else {
        &visuals.widgets.inactive
    };
    let (handle_opacity, background_opacity) = if hovering_bar {
        (style.interact_handle_opacity, style.interact_background_opacity)
    } else if over_area {
        (style.active_handle_opacity, style.active_background_opacity)
    } else {
        (style.dormant_handle_opacity, style.dormant_background_opacity)
    };
    let handle_color = if style.foreground_color { widget.fg_stroke.color } else { widget.bg_fill };
    let (track_fill, radius) = (visuals.extreme_bg_color.gamma_multiply(background_opacity), widget.corner_radius);
    let painter = ui.painter();
    painter.rect_filled(bar, radius, track_fill);
    painter.rect_filled(handle_rect, radius, handle_color.gamma_multiply(handle_opacity));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rtl::Arabic;
    use egui::{Context, Event, PointerButton, Pos2, RawInput, vec2};

    const VIEW: f32 = 200.0;
    const CONTENT: f32 = 1000.0;

    #[test]
    fn the_handle_is_proportional_and_travels_the_track() {
        let track = Rangef::new(10.0, 210.0);
        let top = handle_span(track, CONTENT, VIEW, 0.0, 12.0).expect("scrolls");
        assert_eq!((top.min, top.max), (10.0, 50.0), "a fifth of the track, at the top");
        let bottom = handle_span(track, CONTENT, VIEW, 800.0, 12.0).expect("scrolls");
        assert_eq!((bottom.min, bottom.max), (170.0, 210.0), "and at the bottom at the end");
        let tiny = handle_span(track, 100_000.0, VIEW, 0.0, 12.0).expect("scrolls");
        assert_eq!(tiny.max - tiny.min, 12.0, "never shorter than the minimum");
        let long = handle_span(track, CONTENT, VIEW, 0.0, 500.0).expect("scrolls");
        assert_eq!(long.max - long.min, 200.0, "nor longer than the track");
    }

    #[test]
    fn dragging_the_handle_inverts_the_placement() {
        let track = Rangef::new(10.0, 210.0);
        for offset in [0.0, 1.0, 123.5, 400.0, 799.0, 800.0] {
            let handle = handle_span(track, CONTENT, VIEW, offset, 12.0).expect("scrolls");
            let back = offset_for_handle_top(handle.min, track, handle.max - handle.min, CONTENT - VIEW);
            assert!((back - offset).abs() < 0.01, "offset {offset} came back as {back}");
        }
        // Past either end of the track the offset stops at the end.
        assert_eq!(offset_for_handle_top(-500.0, track, 40.0, 800.0), 0.0);
        assert_eq!(offset_for_handle_top(5000.0, track, 40.0, 800.0), 800.0);
    }

    #[test]
    fn nothing_that_does_not_scroll_or_is_not_a_number_gets_a_handle() {
        let track = Rangef::new(0.0, 100.0);
        assert!(handle_span(track, 100.0, 100.0, 0.0, 12.0).is_none(), "content that fits");
        assert!(handle_span(track, 50.0, 100.0, 0.0, 12.0).is_none(), "content shorter than the view");
        assert!(handle_span(track, 500.0, 0.0, 0.0, 12.0).is_none(), "an empty view");
        assert!(handle_span(Rangef::new(5.0, 5.0), 500.0, 100.0, 0.0, 12.0).is_none(), "an empty track");
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(handle_span(track, bad, 100.0, 0.0, 12.0).is_none());
            assert!(handle_span(track, 500.0, bad, 0.0, 12.0).is_none());
            assert!(handle_span(track, 500.0, 100.0, bad, 12.0).is_none());
            assert!(handle_span(track, 500.0, 100.0, 0.0, bad).is_none());
            assert_eq!(offset_for_handle_top(bad, track, 40.0, 400.0), 0.0);
            assert_eq!(offset_for_handle_top(10.0, track, 40.0, bad), 0.0);
        }
        // A handle as long as the track has no travel.
        assert_eq!(offset_for_handle_top(10.0, track, 100.0, 400.0), 0.0);
    }

    /// One frame of a vertical scroll area (`VIEW` tall, `CONTENT` tall inside) in a window:
    /// the area's id, its viewport, and its stored offset after the frame.
    fn frame(ctx: &Context, time: f64, events: Vec<Event>) -> (Id, Rect, f32) {
        let mut shown = None;
        let input =
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 400.0))), time: Some(time), events, ..RawInput::default() };
        let mut out = ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let area = ScrollArea::vertical().id_salt("probe").max_height(VIEW).auto_shrink([false, false]);
                let out = scroll_area(ui, area, |ui| {
                    ui.allocate_space(vec2(100.0, CONTENT));
                });
                shown = Some((out.id, out.inner_rect));
            });
        });
        out.textures_delta.clear();
        let (id, view) = shown.expect("the area is shown");
        (id, view, State::load(ctx, id).map_or(0.0, |s| s.offset.y))
    }

    /// Run a few frames so that widgets have a layout to hit-test against.
    fn settled() -> (Context, Id, Rect, f64) {
        let ctx = Context::default();
        let mut shown = None;
        let mut time = 0.0;
        for _ in 0..4 {
            time += 0.1;
            shown = Some(frame(&ctx, time, vec![]));
        }
        let (id, view, _) = shown.expect("ran");
        (ctx, id, view, time)
    }

    fn press(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
    }

    #[test]
    fn english_keeps_eguis_bar_at_the_right_edge() {
        let (ctx, id, view, _) = settled();
        assert!(start_bar_rect(&ctx, id).is_none(), "no bar of ours in English");
        let eguis = ctx.read_response(id.with(1)).expect("egui senses its own bar").rect;
        assert!((eguis.right() - view.right()).abs() < 0.5, "egui's bar {eguis:?} is at the right edge of {view:?}");
    }

    #[test]
    fn mirrored_the_bar_is_at_the_left_edge_and_eguis_is_hidden() {
        let _arabic = Arabic::on();
        let (ctx, id, view, _) = settled();
        let bar = start_bar_rect(&ctx, id).expect("a bar for content taller than the view");
        assert!((bar.left() - view.left()).abs() < 0.5, "the bar {bar:?} is at the left edge of {view:?}");
        assert!(bar.right() < view.center().x, "and is a thin strip: {bar:?}");
        assert_eq!((bar.top(), bar.bottom()), (view.top(), view.bottom()), "along the viewport");
        assert!(ctx.read_response(id.with(1)).is_none(), "egui's own bar is not there");
    }

    #[test]
    fn mirrored_content_that_fits_has_no_bar() {
        let _arabic = Arabic::on();
        let ctx = Context::default();
        let mut id = None;
        for i in 0..4 {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 400.0))),
                time: Some(f64::from(i) * 0.1),
                ..RawInput::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let out = scroll_area(ui, ScrollArea::vertical().max_height(VIEW), |ui| {
                        ui.allocate_space(vec2(100.0, 50.0));
                    });
                    id = Some(out.id);
                });
            });
            out.textures_delta.clear();
        }
        assert!(start_bar_rect(&ctx, id.expect("shown")).is_none());
    }

    #[test]
    fn mirrored_dragging_the_handle_scrolls_the_area() {
        let _arabic = Arabic::on();
        let (ctx, id, view, mut time) = settled();
        let bar = start_bar_rect(&ctx, id).expect("a bar");
        let x = bar.center().x;
        let mut offset = 0.0;
        // Press on the handle (it spans the top fifth of the track), then drag 60 down: the handle
        // moves 60 of the 160 points it can travel, so the content moves 60 / 160 of its 800.
        let start = Pos2::new(x, view.top() + 10.0);
        let script = [
            vec![Event::PointerMoved(start)],
            vec![press(start, true)],
            vec![Event::PointerMoved(Pos2::new(x, start.y + 30.0))],
            vec![Event::PointerMoved(Pos2::new(x, start.y + 60.0))],
            vec![press(Pos2::new(x, start.y + 60.0), false)],
            vec![],
        ];
        for events in script {
            time += 0.1;
            offset = frame(&ctx, time, events).2;
        }
        assert!((offset - 300.0).abs() < 1.0, "the area is scrolled to {offset}, not 300");
    }

    #[test]
    fn mirrored_pressing_in_the_track_centres_the_handle_there() {
        let _arabic = Arabic::on();
        let (ctx, id, view, mut time) = settled();
        let x = start_bar_rect(&ctx, id).expect("a bar").center().x;
        // The middle of the track: the 40-point handle is centred on it, at 80 of 160.
        let target = Pos2::new(x, view.top() + 100.0);
        let mut offset = 0.0;
        for events in [vec![Event::PointerMoved(target)], vec![press(target, true)], vec![press(target, false)], vec![]] {
            time += 0.1;
            offset = frame(&ctx, time, events).2;
        }
        assert!((offset - 400.0).abs() < 1.0, "the area is scrolled to {offset}, not 400");
    }

    #[test]
    fn english_dragging_the_left_edge_does_not_scroll() {
        let (ctx, id, view, mut time) = settled();
        let x = view.left() + 2.0;
        let mut offset = 0.0;
        let start = Pos2::new(x, view.top() + 10.0);
        for events in [
            vec![Event::PointerMoved(start)],
            vec![press(start, true)],
            vec![Event::PointerMoved(Pos2::new(x, start.y + 60.0))],
            vec![press(start, false)],
        ] {
            time += 0.1;
            offset = frame(&ctx, time, events).2;
        }
        assert!(start_bar_rect(&ctx, id).is_none());
        assert_eq!(offset, 0.0, "only the bar at the right edge scrolls in English");
    }

    /// One frame of an area that scrolls both ways (`VIEW` square, `CONTENT` square inside), shown with
    /// `show_viewport` as the document view does: the area's id, its viewport and its stored offset.
    fn frame_both(ctx: &Context, time: f64, events: Vec<Event>) -> (Id, Rect, Vec2) {
        let mut shown = None;
        let input =
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 400.0))), time: Some(time), events, ..RawInput::default() };
        let mut out = ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let area = ScrollArea::both().id_salt("both").max_width(VIEW).max_height(VIEW).auto_shrink([false, false]);
                let out = scroll_area_with(ui, area, true, |area, ui| {
                    area.show_viewport(ui, |ui, _viewport| {
                        ui.allocate_space(vec2(CONTENT, CONTENT));
                    })
                });
                shown = Some((out.id, out.inner_rect));
            });
        });
        out.textures_delta.clear();
        let (id, view) = shown.expect("the area is shown");
        (id, view, State::load(ctx, id).map_or(Vec2::ZERO, |s| s.offset))
    }

    fn settled_both() -> (Context, Id, Rect, f64) {
        let ctx = Context::default();
        let mut shown = None;
        let mut time = 0.0;
        for _ in 0..4 {
            time += 0.1;
            shown = Some(frame_both(&ctx, time, vec![]));
        }
        let (id, view, _) = shown.expect("ran");
        (ctx, id, view, time)
    }

    #[test]
    fn english_keeps_both_of_eguis_bars_on_an_area_that_scrolls_both_ways() {
        let (ctx, id, view, _) = settled_both();
        assert!(start_bar_rect(&ctx, id).is_none() && bottom_bar_rect(&ctx, id).is_none(), "no bar of ours in English");
        let right = ctx.read_response(id.with(1)).expect("egui's vertical bar").rect;
        let bottom = ctx.read_response(id.with(0)).expect("egui's horizontal bar").rect;
        assert!((right.right() - view.right()).abs() < 0.5, "{right:?} in {view:?}");
        assert!((bottom.bottom() - view.bottom()).abs() < 0.5, "{bottom:?} in {view:?}");
    }

    #[test]
    fn mirrored_an_area_that_scrolls_both_ways_has_a_left_bar_and_a_bottom_bar() {
        let _arabic = Arabic::on();
        let (ctx, id, view, _) = settled_both();
        let left = start_bar_rect(&ctx, id).expect("a vertical bar");
        let bottom = bottom_bar_rect(&ctx, id).expect("a horizontal bar");
        assert!((left.left() - view.left()).abs() < 0.5 && left.right() < view.center().x, "the vertical bar {left:?} is at the left of {view:?}");
        assert_eq!((left.top(), left.bottom()), (view.top(), view.bottom()));
        assert!(
            (bottom.bottom() - view.bottom()).abs() < 0.5 && bottom.top() > view.center().y,
            "the horizontal bar {bottom:?} is at the bottom of {view:?}"
        );
        assert_eq!((bottom.left(), bottom.right()), (view.left(), view.right()));
        assert!(ctx.read_response(id.with(0)).is_none() && ctx.read_response(id.with(1)).is_none(), "egui's own bars are hidden");
    }

    #[test]
    fn mirrored_dragging_either_handle_scrolls_its_own_axis() {
        let _arabic = Arabic::on();
        let (ctx, id, view, mut time) = settled_both();
        let left = start_bar_rect(&ctx, id).expect("a vertical bar");
        let bottom = bottom_bar_rect(&ctx, id).expect("a horizontal bar");
        // Down the left bar: 60 of the 160 points a 40-point handle travels is 300 of 800.
        let start = Pos2::new(left.center().x, view.top() + 10.0);
        let mut offset = Vec2::ZERO;
        for events in [
            vec![Event::PointerMoved(start)],
            vec![press(start, true)],
            vec![Event::PointerMoved(Pos2::new(start.x, start.y + 60.0))],
            vec![press(Pos2::new(start.x, start.y + 60.0), false)],
            vec![],
        ] {
            time += 0.1;
            offset = frame_both(&ctx, time, events).2;
        }
        assert!((offset.y - 300.0).abs() < 1.0 && offset.x.abs() < 0.5, "scrolled to {offset:?}, not (0, 300)");
        // Along the bottom bar, from its left end.
        let start = Pos2::new(view.left() + 10.0, bottom.center().y);
        for events in [
            vec![Event::PointerMoved(start)],
            vec![press(start, true)],
            vec![Event::PointerMoved(Pos2::new(start.x + 80.0, start.y))],
            vec![press(Pos2::new(start.x + 80.0, start.y), false)],
            vec![],
        ] {
            time += 0.1;
            offset = frame_both(&ctx, time, events).2;
        }
        assert!((offset.x - 400.0).abs() < 1.0 && (offset.y - 300.0).abs() < 1.0, "scrolled to {offset:?}, not (400, 300)");
    }

    #[test]
    fn the_strips_sit_against_their_edges_and_stay_inside_the_viewport() {
        let view = Rect::from_min_max(pos2(10.0, 20.0), pos2(210.0, 220.0));
        let clip = Rect::from_min_max(pos2(0.0, 0.0), pos2(500.0, 500.0));
        assert_eq!(Axis::Y.strip(view, clip, 8.0), Rect::from_min_max(pos2(10.0, 20.0), pos2(18.0, 220.0)));
        assert_eq!(Axis::X.strip(view, clip, 8.0), Rect::from_min_max(pos2(10.0, 212.0), pos2(210.0, 220.0)));
        // An enclosing area scrolled the left edge out of view: the strip moves in with the clip.
        let clipped = Rect::from_min_max(pos2(50.0, 0.0), pos2(500.0, 100.0));
        assert_eq!(Axis::Y.strip(view, clipped, 8.0).left(), 50.0);
        assert_eq!(Axis::X.strip(view, clipped, 8.0).bottom(), 100.0);
        // A clip beyond the viewport cannot push a strip out of it.
        let far = Rect::from_min_max(pos2(900.0, 900.0), pos2(1000.0, 1000.0));
        assert!(
            view.contains_rect(Axis::Y.strip(view, far, 8.0))
                && view.contains_rect(Axis::X.strip(view, Rect::from_min_max(pos2(0.0, -900.0), pos2(1.0, -800.0)), 8.0))
        );
        // A bar grows from its edge inwards, and the handle spans the track.
        let strip = Axis::X.strip(view, clip, 8.0);
        assert_eq!(Axis::X.bar(strip, 3.0), Rect::from_min_max(pos2(10.0, 217.0), pos2(210.0, 220.0)));
        assert_eq!(Axis::Y.bar(Axis::Y.strip(view, clip, 8.0), 3.0), Rect::from_min_max(pos2(10.0, 20.0), pos2(13.0, 220.0)));
        assert_eq!(Axis::X.handle(Axis::X.bar(strip, 3.0), Rangef::new(30.0, 70.0)), Rect::from_min_max(pos2(30.0, 217.0), pos2(70.0, 220.0)));
        assert_eq!(
            Axis::Y.handle(Axis::Y.bar(Axis::Y.strip(view, clip, 8.0), 3.0), Rangef::new(30.0, 70.0)),
            Rect::from_min_max(pos2(10.0, 30.0), pos2(13.0, 70.0))
        );
    }
}
