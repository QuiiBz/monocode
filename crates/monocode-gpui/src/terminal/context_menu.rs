use super::*;
use crate::popover::{Anchor, Menu, MenuAction, MenuEvent, menu_popover};

pub(super) struct EditingMenu {
    menu: Entity<Menu>,
    position: gpui::Point<gpui::Pixels>,
    _events: gpui::Subscription,
}

impl Terminal {
    pub(super) fn open_context_menu(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(target_os = "macos")]
        crate::traffic_lights::focus_content(window);
        self.stop_selection();
        self.pressed_link = None;
        window.focus(&self.focus, cx);
        let selected = self
            .term
            .lock()
            .selection_to_string()
            .is_some_and(|s| !s.is_empty());
        let clipboard = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .is_some_and(|s| !s.is_empty());
        let modifier = if cfg!(target_os = "macos") {
            "⌘"
        } else {
            "Ctrl+Shift+"
        };
        let items = vec![
            MenuAction::new("copy", "Copy")
                .shortcut(format!("{modifier}C"))
                .disabled(!selected)
                .into(),
            MenuAction::new("paste", "Paste")
                .shortcut(format!("{modifier}V"))
                .disabled(!clipboard || self.exited)
                .into(),
            crate::popover::MenuItem::Separator,
            MenuAction::new("select-all", "Select All")
                .shortcut(format!("{modifier}A"))
                .into(),
        ];
        let menu = cx.new(|cx| Menu::new("Terminal editing", items, window, cx));
        let events = cx.subscribe_in(&menu, window, |this, _, event, window, cx| {
            this.context_menu = None;
            if let MenuEvent::Picked(id, _) = event {
                match id.as_ref() {
                    "copy" => this.copy(cx),
                    "paste" => this.paste(cx),
                    "select-all" => this.select_all(cx),
                    _ => {}
                }
            }
            window.focus(&this.focus, cx);
            cx.notify();
        });
        self.context_menu = Some(EditingMenu {
            menu,
            position: event.position,
            _events: events,
        });
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn editing_menu(&self, cx: &App) -> Option<impl IntoElement> {
        self.context_menu.as_ref().map(|state| {
            menu_popover(&state.menu, Anchor::Point(state.position), cx).without_animation()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn terminal_mouse_tracking_routes_buttons_to_program_and_shift_to_local_actions(
        cx: &mut gpui::TestAppContext,
    ) {
        let window = cx.update(|cx| {
            crate::popover::bind(cx);
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| Terminal::new("/tmp".into(), cx))
            })
            .unwrap()
        });
        window
            .update(cx, |terminal, _, _| {
                write_note(&terminal.term, "\x1b[?1000h\x1b[?1006h")
            })
            .unwrap();
        cx.refresh().unwrap();
        for button in [MouseButton::Left, MouseButton::Middle, MouseButton::Right] {
            cx.update_window(window.into(), |_, window, cx| {
                window.dispatch_event(
                    gpui::PlatformInput::MouseDown(MouseDownEvent {
                        position: point(px(20.), px(20.)),
                        button,
                        click_count: 1,
                        ..Default::default()
                    }),
                    cx,
                );
            })
            .unwrap();
            window
                .update(cx, |terminal, _, _| {
                    assert_eq!(terminal.reported_button, Some(button));
                    assert!(!terminal.selecting);
                    assert!(terminal.context_menu.is_none());
                })
                .unwrap();
            cx.update_window(window.into(), |_, window, cx| {
                window.dispatch_event(
                    gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
                        position: point(px(20.), px(20.)),
                        button,
                        ..Default::default()
                    }),
                    cx,
                );
            })
            .unwrap();
            window
                .update(cx, |terminal, _, _| {
                    assert_eq!(terminal.reported_button, None)
                })
                .unwrap();
        }
        for button in [MouseButton::Left, MouseButton::Right] {
            cx.update_window(window.into(), |_, window, cx| {
                window.dispatch_event(
                    gpui::PlatformInput::MouseDown(MouseDownEvent {
                        position: point(px(20.), px(20.)),
                        button,
                        click_count: 1,
                        modifiers: gpui::Modifiers {
                            shift: true,
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    cx,
                );
            })
            .unwrap();
            window
                .update(cx, |terminal, _, _| {
                    assert_eq!(terminal.reported_button, None);
                    assert_eq!(terminal.selecting, button == MouseButton::Left);
                    assert_eq!(
                        terminal.context_menu.is_some(),
                        button == MouseButton::Right
                    );
                })
                .unwrap();
        }
    }

    #[gpui::test]
    fn terminal_context_menu_copies_selection_and_restores_focus(cx: &mut gpui::TestAppContext) {
        let window = cx.update(|cx| {
            super::super::bind(cx);
            crate::popover::bind(cx);
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| Terminal::new("/tmp".into(), cx))
            })
            .unwrap()
        });
        window
            .update(cx, |terminal, _, cx| {
                write_note(&terminal.term, "terminal selection");
                terminal.select_all(cx);
            })
            .unwrap();
        cx.refresh().unwrap();
        let position = point(px(20.), px(20.));
        cx.update_window(window.into(), |_, w, cx| {
            w.dispatch_event(
                gpui::PlatformInput::MouseDown(MouseDownEvent {
                    position,
                    button: MouseButton::Right,
                    ..Default::default()
                }),
                cx,
            );
        })
        .unwrap();
        cx.refresh().unwrap();
        window
            .update(cx, |terminal, _, _| {
                assert!(terminal.context_menu.is_some());
                assert!(!terminal.selecting);
                assert!(
                    terminal
                        .term
                        .lock()
                        .selection_to_string()
                        .unwrap()
                        .contains("terminal selection")
                );
            })
            .unwrap();
        cx.simulate_keystrokes(window.into(), "enter");
        cx.run_until_parked();
        window
            .update(cx, |terminal, w, cx| {
                assert!(terminal.context_menu.is_none());
                assert!(terminal.focus.is_focused(w));
                assert!(
                    cx.read_from_clipboard()
                        .unwrap()
                        .text()
                        .unwrap()
                        .contains("terminal selection")
                );
            })
            .unwrap();
        window
            .update(cx, |terminal, w, cx| {
                terminal.open_context_menu(
                    &MouseDownEvent {
                        position,
                        button: MouseButton::Right,
                        ..Default::default()
                    },
                    w,
                    cx,
                );
            })
            .unwrap();
        cx.refresh().unwrap();
        cx.simulate_keystrokes(window.into(), "escape");
        cx.run_until_parked();
        window
            .update(cx, |terminal, w, _| {
                assert!(terminal.context_menu.is_none());
                assert!(terminal.focus.is_focused(w));
            })
            .unwrap();
    }
}
