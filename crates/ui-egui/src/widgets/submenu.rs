//! The submenu of a menu, opening on the side the interface reads towards.
//!
//! egui's `SubMenu` always asks for `RectAlign::RIGHT_START` and only falls back to the left at the
//! edge of the window, with no way to choose the side. A mirrored (Arabic) menu has its entries
//! against the right edge and its arrow pointing left, so its submenu belongs on the left. This is
//! `egui::containers::menu::SubMenu::show` (egui 0.36, MIT or Apache-2.0) with the one change that
//! the preferred side is [`rtl::submenu_align`], and without its `unwrap`: it is built from the public
//! pieces of egui (`Popup`, `MenuState`, `MenuConfig`), so the open and close behaviour is egui's:
//! the submenu opens when its entry is hovered (or clicked, or activated from the keyboard), stays
//! open while the pointer is in the entry, the submenu or on its way to it, and closes when another
//! entry of the parent is hovered or the parent closes. The popup keeps egui's alternatives, so it
//! still flips to the other side where it would leave the window.

use egui::containers::menu::{MenuConfig, MenuState, SubMenu};
use egui::{Align, Button, Frame, InnerResponse, Layout, PointerButton, Popup, PopupCloseBehavior, Response, Ui, UiKind, UiStackInfo, Vec2};

use crate::rtl;

/// The glyph of the arrow of an entry that opens a submenu on the right (English).
const ARROW_RIGHT: &str = "\u{23F5}";
/// The glyph of the arrow of an entry that opens a submenu on the left (mirrored).
const ARROW_LEFT: &str = "\u{23F4}";

/// A submenu entry named `label` in a menu: the label at the start edge, the arrow at the end, and
/// `add_contents` in a popup on the [`rtl::submenu_align`] side of the entry.
pub fn sub_menu_button<R>(ui: &mut Ui, label: &str, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<Option<R>> {
    let id = ui.next_auto_id();
    let open = MenuState::from_ui(ui, |state, _| state.open_item == Some(SubMenu::id_from_widget_id(id)));
    let inactive = ui.style().visuals.widgets.inactive;
    if open {
        ui.style_mut().visuals.widgets.inactive = ui.style().visuals.widgets.open;
    }
    let button = if rtl::mirrored() {
        let mut atoms = egui::Atoms::new(label);
        atoms.push_left(egui::Atom::grow());
        atoms.push_left(ARROW_LEFT);
        Button::new(atoms)
    } else {
        Button::new(label).right_text(ARROW_RIGHT)
    };
    let response = ui.add(button);
    ui.style_mut().visuals.widgets.inactive = inactive;
    // The accessible name stays "label arrow" in either direction, as in English.
    let name = format!("{label} {ARROW_RIGHT}");
    let enabled = ui.is_enabled();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &name));

    let inner = show(ui, &response, add_contents);
    InnerResponse::new(inner.map(|i| i.inner), response)
}

/// The popup of the submenu whose entry is `button_response`, if it is open.
fn show<R>(ui: &Ui, button_response: &Response, content: impl FnOnce(&mut Ui) -> R) -> Option<InnerResponse<R>> {
    let frame = Frame::menu(ui.style());
    let id = SubMenu::id_from_widget_id(button_response.id);

    // The state of the parent menu.
    let (open_item, menu_id) = MenuState::from_ui(ui, |state, stack| (state.open_item, stack.id));
    let menu_config = MenuConfig::find(ui);
    // The parent's response of the last frame; it is missing only on the frame that opens the menu.
    let menu_root_response = ui.ctx().read_response(menu_id)?;

    let hover_pos = ui.ctx().pointer_hover_pos();

    // The border of the menu does not count as hovering it.
    let menu_rect = menu_root_response.rect - frame.total_margin();
    let is_hovering_menu = hover_pos.is_some_and(|pos| ui.ctx().layer_id_at(pos) == Some(menu_root_response.layer_id) && menu_rect.contains(pos));

    let is_any_open = open_item.is_some();
    let mut is_open = open_item == Some(id);
    let was_open = is_open;
    let mut set_open = None;

    // The entry is grown by half the spacing so that no gap between two entries closes the submenu.
    let button_rect = button_response.rect.expand2(ui.style().spacing.item_spacing / 2.0);
    let is_hovered = hover_pos.is_some_and(|pos| button_rect.contains(pos));

    // `clicked` includes keyboard and accessibility activation. Enter or Space toggles an open
    // submenu; a pointer click keeps it open (touch).
    let clicked = button_response.clicked();
    let clicked_by_pointer = button_response.clicked_by(PointerButton::Primary);
    let clicked_by_keyboard_or_access = clicked && !clicked_by_pointer;

    if ui.is_enabled() && is_open && clicked_by_keyboard_or_access {
        set_open = Some(false);
        is_open = false;
    }

    let should_open = ui.is_enabled() && ((!was_open && clicked) || (is_hovered && !is_any_open));
    if should_open {
        set_open = Some(true);
        is_open = true;
        // Opening one submenu closes its siblings.
        MenuState::from_id(ui.ctx(), menu_id, |state| state.open_item = None);
    }

    let gap = frame.total_margin().sum().x / 2.0 + 2.0;

    let mut response = button_response.clone();
    // The entry and the first item of the submenu are level.
    response.interact_rect = response.interact_rect.expand2(Vec2::new(0.0, frame.total_margin().sum().y / 2.0));

    let popup_response = Popup::from_response(&response)
        .id(id)
        .open(is_open)
        .align(rtl::submenu_align())
        .layout(Layout::top_down_justified(Align::Min))
        .gap(gap)
        .style(menu_config.style.clone())
        .frame(frame)
        // The menu closes on a click itself (below).
        .close_behavior(PopupCloseBehavior::IgnoreClicks)
        .info(UiStackInfo::new(UiKind::Menu).with_tag_value(MenuConfig::MENU_CONFIG_TAG, menu_config.clone()))
        .show(|ui| {
            // The submenu stays on top when its entry is pressed.
            if button_response.clicked() || button_response.is_pointer_button_down_on() {
                ui.ctx().move_to_top(ui.layer_id());
            }
            content(ui)
        });

    if let Some(popup_response) = &popup_response {
        // No open child submenu: this one is the deepest.
        let is_deepest_submenu = MenuState::is_deepest_open_sub_menu(ui.ctx(), id);

        // A click outside both this submenu and the parent menu closes everything. (The parent
        // would not have this submenu open if the pointer were in another entry of it.)
        let clicked_outside = is_deepest_submenu && popup_response.response.clicked_elsewhere() && menu_root_response.clicked_elsewhere();

        // Clicking a submenu entry never closes by itself, so that menus work by touch.
        let submenu_button_clicked = button_response.clicked();
        let clicked_inside = is_deepest_submenu
            && !submenu_button_clicked
            && response.ctx.input(|i| i.pointer.any_click())
            && hover_pos.is_some_and(|pos| popup_response.response.interact_rect.contains(pos));

        let click_close = match menu_config.close_behavior {
            PopupCloseBehavior::CloseOnClick => clicked_outside || clicked_inside,
            PopupCloseBehavior::CloseOnClickOutside => clicked_outside,
            PopupCloseBehavior::IgnoreClicks => false,
        };
        if click_close {
            set_open = Some(false);
            ui.close();
        }

        let is_moving_towards_rect = ui.input(|i| i.pointer.is_moving_towards_rect(&popup_response.response.rect));
        if is_moving_towards_rect {
            // Repaint until the pointer stops heading for the submenu.
            ui.request_repaint();
        }
        let hovering_other_menu_entry =
            is_open && !is_hovered && !popup_response.response.contains_pointer() && !is_moving_towards_rect && is_hovering_menu;

        // The submenu asked to close: close the parent too (an entry that was chosen).
        if popup_response.response.should_close() {
            ui.close();
        }
        if hovering_other_menu_entry {
            set_open = Some(false);
        }
        if ui.will_parent_close() {
            ui.data_mut(|data| data.remove_by_type::<MenuState>());
        }
    }

    if let Some(set_open) = set_open {
        MenuState::from_id(ui.ctx(), menu_id, |state| state.open_item = set_open.then_some(id));
    }

    popup_response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rtl::Arabic;
    use egui::{Context, Event, Pos2, RawInput, Rect, vec2};

    /// A menu opened in the middle of a wide window, with one submenu entry that the pointer hovers:
    /// the entry's rectangle and the content rectangle of the submenu it opened (`None` if it did
    /// not open).
    fn hover_a_submenu() -> (Rect, Option<Rect>) {
        let ctx = Context::default();
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 600.0));
        let (mut entry, mut sub) = (Rect::NOTHING, None);
        for frame in 0..14 {
            let mut input = RawInput { screen_rect: Some(screen), time: Some(f64::from(frame) * 0.1), ..RawInput::default() };
            if frame >= 4 && entry.is_positive() {
                input.events.push(Event::PointerMoved(entry.center()));
            }
            let mut out = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let anchor = ui.put(Rect::from_center_size(Pos2::new(500.0, 300.0), vec2(60.0, 20.0)), egui::Button::new("anchor"));
                    egui::Popup::menu(&anchor).open(true).show(|ui| {
                        ui.set_min_width(120.0);
                        let shown = sub_menu_button(ui, "More", |ui| {
                            ui.label("child entry");
                            ui.min_rect()
                        });
                        entry = shown.response.rect;
                        sub = shown.inner;
                    });
                });
            });
            out.textures_delta.clear();
        }
        (entry, sub)
    }

    #[test]
    fn the_submenu_opens_to_the_right_of_its_entry_in_english() {
        let (entry, sub) = hover_a_submenu();
        let sub = sub.expect("hovering the entry opens its submenu");
        assert!(sub.left() >= entry.right(), "the submenu {sub:?} is right of its entry {entry:?}");
    }

    #[test]
    fn the_submenu_opens_to_the_left_of_its_entry_when_mirrored() {
        let _arabic = Arabic::on();
        let (entry, sub) = hover_a_submenu();
        let sub = sub.expect("hovering the entry opens its submenu");
        assert!(sub.right() <= entry.left(), "the submenu {sub:?} is left of its entry {entry:?}");
    }

    /// The window has no room on the left: the popup is flipped to the right instead of leaving the
    /// window (egui's alternatives are kept).
    #[test]
    fn a_mirrored_submenu_flips_to_the_right_at_the_window_edge() {
        let _arabic = Arabic::on();
        let ctx = Context::default();
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
        let (mut entry, mut sub) = (Rect::NOTHING, None);
        for frame in 0..14 {
            let mut input = RawInput { screen_rect: Some(screen), time: Some(f64::from(frame) * 0.1), ..RawInput::default() };
            if frame >= 4 && entry.is_positive() {
                input.events.push(Event::PointerMoved(entry.center()));
            }
            let mut out = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    // The menu hugs the left edge of the window.
                    let anchor = ui.put(Rect::from_min_size(Pos2::new(4.0, 100.0), vec2(60.0, 20.0)), egui::Button::new("anchor"));
                    egui::Popup::menu(&anchor).open(true).align(egui::RectAlign::BOTTOM_START).show(|ui| {
                        let shown = sub_menu_button(ui, "More", |ui| {
                            ui.label("child entry");
                            ui.min_rect()
                        });
                        entry = shown.response.rect;
                        sub = shown.inner;
                    });
                });
            });
            out.textures_delta.clear();
        }
        let (sub, bounds) = (sub.expect("opens"), screen);
        assert!(bounds.contains_rect(sub), "the submenu {sub:?} stays in the window");
        assert!(sub.left() >= entry.right(), "and it is on the right of {entry:?}");
    }
}
