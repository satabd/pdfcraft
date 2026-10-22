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
//! English never goes through any of this: [`scroll_area`] is `ScrollArea::show` there. Only for
//! vertical areas; a horizontal bar stays where egui puts it.

use egui::scroll_area::{ScrollAreaOutput, State};
use egui::{Id, Rangef, Rect, ScrollArea, Sense, Ui, WidgetInfo, WidgetType, lerp, pos2, remap, remap_clamp};

use crate::rtl;

/// Show `area` in `ui`, with its vertical scrollbar at the start edge: the right in English (egui's
/// own bar, untouched), the left when the interface is mirrored.
pub fn scroll_area<R>(ui: &mut Ui, area: ScrollArea, add_contents: impl FnOnce(&mut Ui) -> R) -> ScrollAreaOutput<R> {
    if !rtl::mirrored() {
        return area.show(ui, add_contents);
    }
    let output = area.scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, add_contents);
    paint_start_bar(ui, &output);
    output
}

/// The id of the start-edge bar of the scroll area `area_id`.
fn bar_id(area_id: Id) -> Id {
    area_id.with("start_scroll_bar")
}

/// Where the start-edge bar of the scroll area `area_id` was last laid out (the strip that senses
/// the pointer, at its full width), or `None` where the area shows egui's own bar or needs none.
#[cfg(test)]
pub(crate) fn start_bar_rect(ctx: &egui::Context, area_id: Id) -> Option<Rect> {
    ctx.read_response(bar_id(area_id)).map(|r| r.rect)
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

/// Paint the bar of `output`'s area at the left edge of its viewport, and let the pointer drag it.
fn paint_start_bar<R>(ui: &mut Ui, output: &ScrollAreaOutput<R>) {
    let (view, content) = (output.inner_rect, output.content_size);
    let style = ui.spacing().scroll;
    // The strip egui would sense, mirrored: its full width at the left edge of the viewport, nudged
    // right if an enclosing scroll area has scrolled that edge out of view.
    let left = view.left().max(ui.clip_rect().left()).min(view.right());
    let strip = Rect::from_min_max(pos2(left, view.top()), pos2((left + style.bar_width).min(view.right()), view.bottom()));
    let track = Rangef::new(view.top(), view.bottom());
    if handle_span(track, content.y, view.height(), output.state.offset.y, style.handle_min_length).is_none() || !ui.is_rect_visible(strip) {
        return;
    }

    let id = bar_id(output.id);
    let enabled = ui.is_enabled();
    let response = ui.interact(strip, id, if enabled { Sense::click_and_drag() } else { Sense::hover() });
    response.widget_info(|| WidgetInfo::new(WidgetType::ScrollBar));

    let mut offset = output.state.offset.y;
    let max_offset = content.y - view.height();
    let min_len = style.handle_min_length;
    let grab_id = id.with("grab");
    if let Some(pointer) = response.interact_pointer_pos() {
        if let Some(handle) = handle_span(track, content.y, view.height(), offset, min_len) {
            let len = handle.max - handle.min;
            // Where on the handle the pointer holds it: where it first pressed, or its middle when
            // it pressed in the track (the handle jumps to centre on the pointer).
            let grab = ui.data_mut(|d| {
                *d.get_temp_mut_or_insert_with(grab_id, || {
                    if handle.contains(pointer.y) {
                        pointer.y - handle.min
                    } else {
                        let top = (pointer.y - len / 2.0).clamp(track.min, (track.max - len).max(track.min));
                        pointer.y - top
                    }
                })
            });
            offset = offset_for_handle_top(pointer.y - grab, track, len, max_offset);
        }
    } else {
        ui.data_mut(|d| d.remove::<f32>(grab_id));
    }
    let offset = if offset.is_finite() { offset.clamp(0.0, max_offset) } else { output.state.offset.y };
    if (offset - output.state.offset.y).abs() > f32::EPSILON {
        // Stored for the next frame, as egui's own bar does it (a scroll area reads its offset
        // from its stored state when it starts).
        let mut state: State = State::load(ui.ctx(), output.id).unwrap_or(output.state);
        state.offset.y = offset;
        state.store(ui.ctx(), output.id);
        ui.request_repaint();
    }

    // Paint with the new offset: no frame behind the pointer.
    let Some(handle) = handle_span(track, content.y, view.height(), offset, min_len) else { return };
    let hovering_bar = response.hovered() || response.dragged();
    let grown = ui.ctx().animate_bool_responsive(id.with("hover"), hovering_bar);
    let width = lerp(style.floating_width..=style.bar_width, grown).min(strip.width());
    let bar = Rect::from_min_max(strip.min, pos2(strip.min.x + width, strip.max.y));
    let handle_rect = Rect::from_min_max(pos2(bar.min.x, handle.min), pos2(bar.max.x, handle.max));

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
}
