//! Vector icons: embedded Lucide SVGs, recoloured to white, rasterized by egui_extras at the exact
//! on-screen pixel size, and tinted per use (PhotoCraft lesson: never use Unicode glyphs as icons).

use std::sync::OnceLock;

use craft_ui::buttons::{ButtonVisuals, IconButton, IconButtonStyle};
use craft_ui::icons::{SvgIconSet, SvgStyle};

use egui::{Color32, Rect, Response, Stroke, Vec2};

use crate::icon_data::ICONS;
use crate::theme::{self, Tokens};

fn white_icons() -> &'static SvgIconSet {
    static MAP: OnceLock<SvgIconSet> = OnceLock::new();
    MAP.get_or_init(|| SvgIconSet::new(ICONS, SvgStyle { current_color: "#ffffff", stroke_width: Some("1.75") }, "bytes://icons/", "square"))
}

pub fn exists(name: &str) -> bool {
    white_icons().contains(name)
}

fn source(name: &str) -> egui::ImageSource<'static> {
    // PdfCraft caches an unknown name under the resolved fallback name.
    white_icons().source(white_icons().canonical_name(name).unwrap_or("square"))
}

/// The icons that point somewhere or lead a line, and so face the other way in a right-to-left
/// interface (Back is an arrow to the right in Arabic, a list's bullets sit at the right).
/// Everything else is drawn as it is: clocks, check marks, help, rotate arrows, media controls,
/// zoom, file and bookmark glyphs, external-link and the signature have no direction of their
/// own, and `align-left`, `align-right`, `move-right` (the Arrow annotation) and the flip icons
/// name a physical direction, which does not depend on the language.
pub const MIRROR: &[&str] = &[
    "chevron-left",
    "chevron-right",
    "chevrons-left",
    "chevrons-right",
    "undo-2",
    "redo-2",
    "panel-left",
    "panel-right",
    "list",
    "list-ordered",
    "reply",
    "send",
];

/// Whether the icon `name` faces the other way in a right-to-left interface.
fn mirrors(name: &str) -> bool {
    MIRROR.contains(&name)
}

/// The icon as the interface language reads it: a directional icon ([`MIRROR`]) is flipped
/// horizontally in a right-to-left language. Every icon button and painted icon goes through
/// here, so a call site does not decide.
pub fn image(name: &str, size: f32, tint: Color32) -> egui::Image<'static> {
    let image = image_fixed(name, size, tint);
    if crate::rtl::mirrored() && mirrors(name) { image.uv(Rect::from_min_max(egui::pos2(1.0, 0.0), egui::pos2(0.0, 1.0))) } else { image }
}

/// [`image`] for a direction that is physical: the arrows that scroll the tab strip to its left
/// and right, or a control that moves an item to the left of the page grid. Never flipped.
pub fn image_fixed(name: &str, size: f32, tint: Color32) -> egui::Image<'static> {
    egui::Image::new(source(name)).fit_to_exact_size(Vec2::splat(size)).tint(tint)
}

pub fn paint(ui: &egui::Ui, rect: Rect, name: &str, size: f32, tint: Color32) {
    image(name, size, tint).paint_at(ui, Rect::from_center_size(rect.center(), Vec2::splat(size)));
}

/// [`paint`] for a physical direction (see [`image_fixed`]).
pub fn paint_fixed(ui: &egui::Ui, rect: Rect, name: &str, size: f32, tint: Color32) {
    image_fixed(name, size, tint).paint_at(ui, Rect::from_center_size(rect.center(), Vec2::splat(size)));
}

/// Square icon button: transparent until hovered; `selected` gets the accent treatment.
pub fn button(ui: &mut egui::Ui, name: &str, box_size: f32, selected: bool, tooltip: &str) -> Response {
    icon_button(ui, name, box_size, selected, tooltip, paint)
}

/// [`button`] whose icon names a physical direction and is never flipped (see [`image_fixed`]).
pub fn button_fixed(ui: &mut egui::Ui, name: &str, box_size: f32, selected: bool, tooltip: &str) -> Response {
    icon_button(ui, name, box_size, selected, tooltip, paint_fixed)
}

fn icon_button(
    ui: &mut egui::Ui,
    name: &str,
    box_size: f32,
    selected: bool,
    tooltip: &str,
    paint: fn(&egui::Ui, Rect, &str, f32, Color32),
) -> Response {
    let t = Tokens::get(ui.ctx());
    // The shared button paints no fill of its own: the eased press wash below replaces it.
    let clear = |icon| ButtonVisuals { fill: Color32::TRANSPARENT, stroke: Stroke::NONE, icon };
    let style = IconButtonStyle {
        normal: clear(t.icon),
        hovered: clear(t.icon),
        pressed: Some(clear(t.icon)),
        selected: clear(t.accent_text),
        corner_radius: t.radius.into(),
        focus_stroke: Stroke::new(1.0, t.accent),
    };
    let label = if tooltip.is_empty() { name } else { tooltip };
    // The wash eases from the response, so reserve its place under the icon and focus ring.
    let wash = ui.painter().add(egui::Shape::Noop);
    let mut painted = None;
    let resp = IconButton::new(label, Vec2::splat(box_size), &style).selected(selected).show(ui, |_, rect, tint| painted = Some((rect, tint)));
    if let Some((rect, tint)) = painted {
        let press = theme::Press::track(ui, &resp);
        let mut shapes = Vec::with_capacity(2);
        let fill = press.wash_color(&t, selected);
        if fill.a() > 0 {
            shapes.push(egui::Shape::rect_filled(rect, t.radius, fill));
        }
        if selected {
            shapes.push(egui::Shape::rect_stroke(
                rect.shrink(0.5),
                t.radius,
                Stroke::new(1.0, t.accent.gamma_multiply(0.45)),
                egui::StrokeKind::Inside,
            ));
        }
        ui.painter().set(wash, egui::Shape::Vec(shapes));
        // The glyph shrinks a little and drops a pixel while the pointer is down.
        let size = (box_size * 0.5).round() * (1.0 - 0.08 * press.down);
        paint(ui, rect.translate(press.offset()), name, size, tint);
    }
    let resp = theme::hand(resp);
    if tooltip.is_empty() { resp } else { resp.on_hover_text(tooltip) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draws in Arabic for the test's lifetime (the language is per thread), then restores it.
    struct Arabic;
    impl Arabic {
        fn on() -> Self {
            crate::i18n::set_current(crate::i18n::Lang::from_code("ar").expect("ar registered"));
            Self
        }
    }
    impl Drop for Arabic {
        fn drop(&mut self) {
            crate::i18n::set_current(crate::i18n::Lang::EN);
        }
    }

    const FLIPPED: Rect = Rect { min: egui::pos2(1.0, 0.0), max: egui::pos2(0.0, 1.0) };

    #[test]
    fn mirrored_icons_exist_and_the_directionless_ones_are_not_listed() {
        for name in MIRROR {
            assert!(exists(name), "{name} is not an icon");
        }
        for name in [
            "check",
            "circle-check",
            "clock",
            "circle-help",
            "rotate-cw",
            "rotate-ccw",
            "zoom-in",
            "zoom-out",
            "external-link",
            "signature",
            "file-text",
            "bookmark",
            "mic",
            "film",
            "x",
            "plus",
            "search",
            "chevron-up",
            "chevron-down",
            "align-left",
            "align-right",
            "move-right",
            "flip-horizontal-2",
        ] {
            assert!(exists(name), "{name} is not an icon");
            assert!(!MIRROR.contains(&name), "{name} must not be flipped");
        }
        // A listed name is listed once.
        let mut sorted = MIRROR.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), MIRROR.len());
    }

    #[test]
    fn a_directional_icon_flips_only_in_a_right_to_left_language() {
        let default_uv = image_fixed("chevron-left", 16.0, Color32::WHITE).image_options().uv;
        assert_eq!(image("chevron-left", 16.0, Color32::WHITE).image_options().uv, default_uv, "left to right is untouched");
        let _arabic = Arabic::on();
        assert_eq!(image("chevron-left", 16.0, Color32::WHITE).image_options().uv, FLIPPED);
        assert_eq!(image("undo-2", 16.0, Color32::WHITE).image_options().uv, FLIPPED);
        assert_eq!(image("check", 16.0, Color32::WHITE).image_options().uv, default_uv, "no direction of its own");
        assert_eq!(image("move-right", 16.0, Color32::WHITE).image_options().uv, default_uv, "the Arrow annotation is physical");
        assert_eq!(image_fixed("chevron-left", 16.0, Color32::WHITE).image_options().uv, default_uv, "a fixed icon is never flipped");
        assert_eq!(image_fixed("chevron-right", 16.0, Color32::WHITE).image_options().uv, default_uv);
    }

    #[test]
    fn every_catalog_icon_exists() {
        for g in pdfcraft_engine::catalog::TOOL_GROUPS {
            assert!(super::exists(g.icon), "missing icon {}", g.icon);
            for s in g.sections {
                for i in s.items {
                    assert!(super::exists(i.icon), "missing icon {}", i.icon);
                }
            }
        }
    }

    #[test]
    fn every_command_icon_exists() {
        for c in pdfcraft_engine::commands::COMMANDS {
            assert!(super::exists(c.icon), "missing icon {} ({})", c.icon, c.id);
        }
    }

    #[test]
    fn shared_catalog_preserves_existing_icons_and_fallback_uri() {
        assert_eq!(ICONS.len(), 186);
        assert!(exists("minimize-2"));
        for (name, original) in ICONS {
            let expected =
                String::from_utf8_lossy(original).replace("currentColor", "#ffffff").replace("stroke-width=\"2\"", "stroke-width=\"1.75\"");
            let egui::ImageSource::Bytes { uri, bytes } = source(name) else { panic!("expected SVG bytes") };
            assert_eq!(uri.as_ref(), format!("bytes://icons/{name}.svg"));
            assert_eq!(bytes.as_ref(), expected.as_bytes(), "{name}");
        }
        let egui::ImageSource::Bytes { uri, bytes } = source("missing-icon") else { panic!("expected SVG bytes") };
        let egui::ImageSource::Bytes { bytes: fallback, .. } = source("square") else { panic!("expected SVG bytes") };
        assert_eq!(uri.as_ref(), "bytes://icons/square.svg");
        assert_eq!(bytes.as_ref(), fallback.as_ref());
        assert!(!exists("missing-icon"));
    }

    #[test]
    fn shared_button_preserves_pointer_responses() {
        // The shared button must respond like the original allocate-and-paint path in both
        // themes. Its painting differs on purpose (the eased press wash, the selected outline
        // and the sinking glyph); the keyboard-focus outline is covered in craft-ui.
        fn frame(
            ctx: &egui::Context,
            events: Vec<egui::Event>,
            selected: bool,
            enabled: bool,
            legacy: bool,
        ) -> (egui::Response, Vec<egui::epaint::ClippedShape>) {
            let mut response = None;
            let mut output = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                response = Some(
                    ui.add_enabled_ui(enabled, |ui| {
                        if legacy {
                            let t = Tokens::get(ui.ctx());
                            let (rect, response) = ui.allocate_exact_size(Vec2::splat(28.0), egui::Sense::click());
                            if selected {
                                ui.painter().rect_filled(rect, t.radius, t.accent_soft);
                            } else if response.is_pointer_button_down_on() {
                                ui.painter().rect_filled(rect, t.radius, t.pressed);
                            } else if response.hovered() {
                                ui.painter().rect_filled(rect, t.radius, t.hover);
                            }
                            let tint = if selected { t.accent_text } else { t.icon };
                            paint(ui, rect, "square", (28.0_f32 * 0.5).round(), tint);
                            response
                        } else {
                            button(ui, "square", 28.0, selected, "")
                        }
                    })
                    .inner,
                );
            });
            output.textures_delta.clear();
            (response.unwrap(), output.shapes)
        }
        for theme in [crate::theme::ThemeKind::Dark, crate::theme::ThemeKind::Light] {
            for selected in [false, true] {
                for enabled in [false, true] {
                    let old = egui::Context::default();
                    let new = egui::Context::default();
                    for ctx in [&old, &new] {
                        crate::theme::apply(ctx, theme);
                        egui_extras::install_image_loaders(ctx);
                    }
                    let mut center = egui::Pos2::ZERO;
                    for step in 0..7 {
                        let events = match step {
                            2 => vec![egui::Event::PointerMoved(center)],
                            3 | 4 => vec![egui::Event::PointerButton {
                                pos: center,
                                button: egui::PointerButton::Primary,
                                pressed: step == 3,
                                modifiers: egui::Modifiers::NONE,
                            }],
                            5 => vec![egui::Event::PointerMoved(egui::pos2(300.0, 200.0))],
                            _ => vec![],
                        };
                        let (a, _) = frame(&old, events.clone(), selected, enabled, true);
                        let (b, after) = frame(&new, events, selected, enabled, false);
                        center = a.rect.center();
                        assert_eq!(a.rect, b.rect);
                        assert_eq!(
                            (a.clicked(), a.hovered(), a.is_pointer_button_down_on()),
                            (b.clicked(), b.hovered(), b.is_pointer_button_down_on())
                        );
                        assert!(!after.is_empty(), "{theme:?}, selected={selected}, enabled={enabled}, step={step}");
                    }
                }
            }
        }
    }
}
