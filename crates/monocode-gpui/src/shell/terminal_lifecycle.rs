use super::*;

impl Shell {
    pub(super) fn terminal_changed(&self, terminal: Entity<Terminal>, cx: &mut Context<Self>) {
        if terminal.read(cx).exited {
            let terminal_id = terminal.entity_id();
            let shell = cx.weak_entity();
            cx.defer(move |cx| {
                cx.with_window(shell.entity_id(), |window, cx| {
                    let _ = shell.update(cx, |shell, cx| {
                        shell.close_exited_terminal(terminal_id, window, cx);
                    });
                });
            });
        }
        cx.notify();
    }

    pub(super) fn defer_exited_terminals(&self, window: &Window, cx: &mut Context<Self>) {
        // Read retained exit state so exits before tab registration are also handled.
        let exited = self
            .terminals
            .values()
            .flatten()
            .chain(self.terminal_pane_views.values())
            .filter(|terminal| terminal.read(cx).exited)
            .map(Entity::entity_id)
            .collect::<Vec<_>>();
        if exited.is_empty() {
            return;
        }
        cx.defer_in(window, move |shell, window, cx| {
            for terminal_id in exited {
                shell.close_exited_terminal(terminal_id, window, cx);
            }
        });
    }

    fn close_exited_terminal(
        &mut self,
        terminal_id: gpui::EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dock_terminal = self.terminals.iter().find_map(|(project, terminals)| {
            terminals
                .iter()
                .position(|terminal| terminal.entity_id() == terminal_id)
                .map(|index| (project.clone(), index))
        });
        if let Some((project, index)) = dock_terminal {
            self.remove_terminal(&project, index, window, cx);
            return;
        }
        let pane = self
            .terminal_pane_views
            .iter()
            .find_map(|(pane, terminal)| (terminal.entity_id() == terminal_id).then_some(*pane));
        if let Some(pane) = pane {
            if self
                .workspace
                .tab_of(pane)
                .is_some_and(|tab| self.workspace.layout(tab).is_split())
            {
                self.close_pane(pane, window, cx);
            } else {
                self.close_tabs(&[pane], None, window, cx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::Event;
    use gpui::{TestAppContext, VisualTestContext};
    use std::process::ExitStatus;

    fn status(code: i32) -> ExitStatus {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            ExitStatus::from_raw(code << 8)
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::ExitStatusExt;
            ExitStatus::from_raw(code as u32)
        }
    }

    fn exit(terminal: &Entity<Terminal>, code: i32, cx: &mut Context<Shell>) {
        terminal.update(cx, |terminal, cx| {
            terminal.on_event(Event::ChildExit(status(code)), cx);
            terminal.on_event(Event::Exit, cx);
        });
    }

    #[gpui::test]
    fn normal_shell_exit_closes_active_dock_tabs_and_restores_focus(cx: &mut TestAppContext) {
        let (window, _, session) = super::super::tests::window(cx);
        let (project, first, second) = window
            .update(cx, |shell, window, cx| {
                cx.set_reduce_motion(true);
                shell.open(session, window, cx);
                shell.new_terminal(window, cx);
                shell.new_terminal(window, cx);
                let project = shell.workspace.project.clone();
                let first = shell.terminals[&project][0].downgrade();
                let second = shell.terminals[&project][1].downgrade();
                exit(&second.upgrade().unwrap(), 0, cx);
                (project, first, second)
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        window
            .update(&mut visual, |shell, window, cx| {
                assert_eq!(shell.workspace.docks[&project].active, 0);
                assert_eq!(shell.terminals[&project].len(), 1);
                assert_eq!(shell.dock_focused(window, cx), Some(0));
                assert!(second.upgrade().is_none());
                exit(&first.upgrade().unwrap(), 0, cx);
            })
            .unwrap();
        visual.run_until_parked();
        window
            .update(&mut visual, |shell, window, cx| {
                assert!(shell.terminals[&project].is_empty());
                assert!(shell.workspace.docks[&project].terminals.is_empty());
                assert!(!shell.workspace.docks[&project].open);
                assert!(first.upgrade().is_none());
                assert!(
                    shell.sessions[&session]
                        .read(cx)
                        .input
                        .focus_handle(cx)
                        .is_focused(window)
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn inactive_dock_exit_keeps_the_active_terminal_and_focus(cx: &mut TestAppContext) {
        let (window, _, session) = super::super::tests::window(cx);
        let (project, active) = window
            .update(cx, |shell, window, cx| {
                cx.set_reduce_motion(true);
                shell.open(session, window, cx);
                for _ in 0..3 {
                    shell.new_terminal(window, cx);
                }
                let project = shell.workspace.project.clone();
                let active = shell.terminals[&project][2].downgrade();
                exit(&shell.terminals[&project][0].clone(), 0, cx);
                exit(&shell.terminals[&project][1].clone(), 0, cx);
                (project, active)
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        window
            .update(&mut visual, |shell, window, cx| {
                assert_eq!(shell.terminals[&project], [active.upgrade().unwrap()]);
                assert_eq!(shell.workspace.docks[&project].active, 0);
                assert_eq!(shell.dock_focused(window, cx), Some(0));
            })
            .unwrap();
    }

    #[gpui::test]
    fn background_project_exit_does_not_change_the_current_dock(cx: &mut TestAppContext) {
        let (window, background_session, session) = super::super::tests::window(cx);
        let (background_project, project, active) = window
            .update(cx, |shell, window, cx| {
                cx.set_reduce_motion(true);
                shell.open(background_session, window, cx);
                shell.new_terminal(window, cx);
                let background_project = shell.workspace.project.clone();
                let background = shell.terminals[&background_project][0].clone();
                shell.open(session, window, cx);
                shell.new_terminal(window, cx);
                let project = shell.workspace.project.clone();
                let active = shell.terminals[&project][0].downgrade();
                exit(&background, 0, cx);
                (background_project, project, active)
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        window
            .update(&mut visual, |shell, window, cx| {
                assert!(shell.terminals[&background_project].is_empty());
                assert!(!shell.workspace.docks[&background_project].open);
                assert_eq!(shell.workspace.project, project);
                assert!(shell.workspace.docks[&project].open);
                assert_eq!(shell.terminals[&project], [active.upgrade().unwrap()]);
                assert_eq!(shell.dock_focused(window, cx), Some(0));
                assert!(shell.dock_slide.is_none());
            })
            .unwrap();
    }

    #[gpui::test]
    fn an_exit_before_tab_registration_is_retained(cx: &mut TestAppContext) {
        let (window, _, session) = super::super::tests::window(cx);
        let terminal = window
            .update(cx, |shell, window, cx| {
                cx.set_reduce_motion(true);
                shell.open(session, window, cx);
                let project = shell.workspace.project.clone();
                let terminal = shell.spawn_terminal(project.clone(), cx);
                exit(&terminal, 0, cx);
                shell
                    .workspace
                    .docks
                    .entry(project.clone())
                    .or_default()
                    .add(project.clone());
                let weak = terminal.downgrade();
                shell.terminals.entry(project).or_default().push(terminal);
                cx.notify();
                weak
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        window
            .update(&mut visual, |shell, _, _| {
                assert!(shell.terminals[&shell.workspace.project].is_empty());
                assert!(terminal.upgrade().is_none());
            })
            .unwrap();
    }

    #[gpui::test]
    fn shell_exits_close_tabs_for_all_exit_codes(cx: &mut TestAppContext) {
        let (window, _, session) = super::super::tests::window(cx);
        let terminals = window
            .update(cx, |shell, window, cx| {
                cx.set_reduce_motion(true);
                shell.open(session, window, cx);
                let mut terminals = Vec::new();
                for code in [0, 1, 7, 130, 255] {
                    shell.new_terminal(window, cx);
                    let terminal = shell.terminals[&shell.workspace.project]
                        .last()
                        .unwrap()
                        .clone();
                    exit(&terminal, code, cx);
                    terminals.push(terminal.downgrade());
                }
                shell.new_terminal(window, cx);
                let terminal = shell.terminals[&shell.workspace.project].last().unwrap();
                terminal.update(cx, |terminal, cx| terminal.on_event(Event::Exit, cx));
                terminals.push(terminal.downgrade());
                terminals
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        window
            .update(&mut visual, |shell, _, _| {
                assert!(shell.terminals[&shell.workspace.project].is_empty());
                assert!(
                    shell.workspace.docks[&shell.workspace.project]
                        .terminals
                        .is_empty()
                );
                assert!(
                    terminals
                        .iter()
                        .all(|terminal| terminal.upgrade().is_none())
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn failed_starts_and_running_shells_keep_their_tabs(cx: &mut TestAppContext) {
        let (window, _, session) = super::super::tests::window(cx);
        window
            .update(cx, |shell, window, cx| {
                cx.set_reduce_motion(true);
                shell.open(session, window, cx);
                for _ in 0..2 {
                    shell.new_terminal(window, cx);
                }
                let project = shell.workspace.project.clone();
                shell.terminals[&project][0].update(cx, |terminal, cx| {
                    terminal.run("false", cx);
                    terminal.on_event(Event::Wakeup, cx);
                    assert!(!terminal.exited);
                });
                assert!(!shell.terminals[&project][1].read(cx).exited);
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        window
            .update(&mut visual, |shell, _, _| {
                assert_eq!(shell.terminals[&shell.workspace.project].len(), 2);
                assert_eq!(
                    shell.workspace.docks[&shell.workspace.project]
                        .terminals
                        .len(),
                    2
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn title_and_split_terminal_exits_use_the_pane_lifecycle(cx: &mut TestAppContext) {
        let (window, _, session) = super::super::tests::window(cx);
        let terminal = window
            .update(cx, |shell, window, cx| {
                cx.set_reduce_motion(true);
                shell.open(session, window, cx);
                shell.new_title_terminal(window, cx);
                let pane = shell.workspace.active.unwrap();
                let terminal = shell.terminal_pane_views[&pane].clone();
                exit(&terminal, 0, cx);
                terminal.downgrade()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        window
            .update(&mut visual, |shell, window, cx| {
                assert!(shell.workspace.terminal_panes.is_empty());
                assert!(shell.terminal_pane_views.is_empty());
                assert!(terminal.upgrade().is_none());
                assert_eq!(shell.workspace.active, Some(session));
                shell.split_title_terminal(window, cx);
                let pane = shell.workspace.active.unwrap();
                assert_eq!(shell.workspace.tab_of(pane), Some(session));
                exit(&shell.terminal_pane_views[&pane].clone(), 0, cx);
            })
            .unwrap();
        visual.run_until_parked();
        window
            .update(&mut visual, |shell, window, cx| {
                assert!(shell.workspace.terminal_panes.is_empty());
                assert!(shell.terminal_pane_views.is_empty());
                assert_eq!(shell.workspace.active, Some(session));
                assert_eq!(shell.workspace.layout(session).leaf_ids(), [session]);
                assert!(
                    shell.sessions[&session]
                        .read(cx)
                        .input
                        .focus_handle(cx)
                        .is_focused(window)
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn inactive_title_terminal_exit_keeps_chat_focus(cx: &mut TestAppContext) {
        let (window, _, session) = super::super::tests::window(cx);
        window
            .update(cx, |shell, window, cx| {
                cx.set_reduce_motion(true);
                shell.open(session, window, cx);
                shell.new_title_terminal(window, cx);
                let pane = shell.workspace.active.unwrap();
                let terminal = shell.terminal_pane_views[&pane].clone();
                shell.open(session, window, cx);
                exit(&terminal, 0, cx);
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        window
            .update(&mut visual, |shell, window, cx| {
                assert!(shell.workspace.terminal_panes.is_empty());
                assert_eq!(shell.workspace.active, Some(session));
                assert!(
                    shell.sessions[&session]
                        .read(cx)
                        .input
                        .focus_handle(cx)
                        .is_focused(window)
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn last_panel_shell_exit_hides_the_panel_without_starting_a_shell(cx: &mut TestAppContext) {
        let (window, _, session) = super::super::tests::window(cx);
        window
            .update(cx, |shell, window, cx| {
                cx.set_reduce_motion(true);
                shell.open(session, window, cx);
                shell.workspace.preferences.set_terminal_placement("panel");
                shell.show_panel_terminal(window, cx);
                let terminal = shell.terminals[&shell.workspace.project][0].clone();
                exit(&terminal, 0, cx);
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        window
            .update(&mut visual, |shell, window, cx| {
                assert!(!shell.workspace.right_panel_open);
                assert!(shell.terminals[&shell.workspace.project].is_empty());
                let _ = shell.panel_terminal(window, cx);
                assert!(shell.terminals[&shell.workspace.project].is_empty());
                assert!(
                    shell.workspace.docks[&shell.workspace.project]
                        .terminals
                        .is_empty()
                );
            })
            .unwrap();
    }
}
