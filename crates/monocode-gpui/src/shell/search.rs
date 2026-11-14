use super::*;
use crate::search::{Event, SearchView, Sources};
use monocode_core::search::Target;

pub(super) struct Open {
    pub(super) view: Entity<SearchView>,
    /// Floating command panel rather than a page with its own tab.
    pub(super) panel: bool,
    restore: Option<FocusHandle>,
    _events: Subscription,
}

impl Shell {
    /// Search is open as a page that replaces the workspace.
    pub(super) fn search_page(&self) -> bool {
        self.app_search.as_ref().is_some_and(|s| !s.panel)
    }
    /// The panel floats over whatever is open, like Quick Open, so nothing
    /// underneath is left or rearranged while searching.
    pub(super) fn search_modal(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let open = self.app_search.as_ref().filter(|s| s.panel)?;
        let viewport = window.viewport_size();
        let width = 680f32.min(viewport.width.as_f32() - 32.);
        let anchor = Anchor::Point(gpui::point(
            px((viewport.width.as_f32() - width) / 2.),
            px((viewport.height.as_f32() * 0.14).max(32.)),
        ));
        let weak = cx.weak_entity();
        Some(
            div()
                .absolute()
                .size_full()
                .child(crate::modal::scrim("app-search-scrim"))
                .child(
                    popover("app-search-modal", anchor)
                        .modal_surface(self.workspace.boxy_layout())
                        .width(width)
                        .gap(0.)
                        .priority(80)
                        .on_dismiss(move |window, cx| {
                            let _ = weak.update(cx, |this, cx| this.close_app_search(window, cx));
                        })
                        .child(open.view.clone()),
                )
                .into_any_element(),
        )
    }
    pub(super) fn leave_app_search(&mut self, cx: &mut Context<Self>) {
        if let Some(search) = self.app_search.take() {
            self.saved_search = Some(search.view.read(cx).draft(cx));
        }
    }
    pub(super) fn close_search_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_title_tabs(&[TitleTab::Search], None, window, cx);
    }
    pub(super) fn search_title_tab(
        &self,
        width: f32,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let active = self.app_search.is_some();
        let close = active || width >= TAB_CLOSE_MIN_WIDTH;
        div()
            .id("search-title-tab")
            .debug_selector(|| "search-title-tab".into())
            .group("search-title-tab")
            .role(Role::Tab)
            .aria_label("Search")
            .aria_selected(active)
            .cursor_pointer()
            .w(px(width))
            .h(px(TITLE_BAR_HEIGHT))
            .flex_shrink_0()
            .flex()
            .items_center()
            .relative()
            .child(
                title_tab_plate(active, false)
                    .text_color(content(if active { 1. } else { 0.5 }))
                    .w_full()
                    .min_w_0()
                    .when(close && width >= TAB_CLOSE_MIN_WIDTH, |d| d.pr_7())
                    .when(width < NARROW_TAB, |d| {
                        d.px(px(((width - TAB_TILE.w) / 2.).max(0.)))
                    })
                    .gap(px(TAB_TILE_GAP))
                    .group_hover("search-title-tab", |s| {
                        s.text_color(content(1.)).bg(title_tab_fill(active, true))
                    })
                    .child(mark_tile(
                        &TAB_TILE,
                        Face::Neutral,
                        icon("search", 14.).text_color(crate::tab_tile::tile_ink()),
                    ))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_size(px(TITLE_TAB_TEXT_SIZE))
                            .child("Search"),
                    ),
            )
            .when(close, |d| {
                d.child(
                    div()
                        .id("close-search-tab")
                        .debug_selector(|| "close-search-tab".into())
                        .role(Role::Button)
                        .aria_label("Close Search")
                        .absolute()
                        .right_1()
                        .top(px(10.))
                        .size(px(20.))
                        .rounded_sm()
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity(0.)
                        .group_hover("search-title-tab", |s| s.opacity(1.))
                        .hover(|s| s.bg(content(0.1)))
                        .child(icon("close", 12.))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            this.close_search_tab(window, cx);
                        })),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event, _, cx| this.start_tab_drag(TitleTab::Search, event, cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.open_menu(
                        MenuTarget::SearchTab,
                        Anchor::Point(event.position),
                        window,
                        cx,
                    )
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.close_search_tab(window, cx);
                }),
            )
            .on_click(cx.listener(|this, _, window, cx| {
                if std::mem::take(&mut this.skip_tab_click) {
                    return;
                }
                this.open_app_search(window, cx);
            }))
    }
    pub(super) fn close_app_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(search) = self.app_search.take() {
            self.saved_search = Some(search.view.read(cx).draft(cx));
            self.update_chat_visibility(cx);
            if let Some(restore) = search.restore {
                window.focus(&restore, cx);
            } else {
                self.focus_active(window, cx);
            }
            if let Some(tab) = self.active_title_tab(cx) {
                self.remember_title_tab(tab);
            }
            self.persist(cx);
            self.invalidate_chrome(cx);
            cx.notify();
        }
    }
    pub(super) fn open_app_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(search) = &self.app_search {
            window.focus(&search.view.focus_handle(cx), cx);
            return;
        }
        let panel = self.workspace.preferences.search_panel;
        self.close_menu(cx);
        if panel {
            if self.quick_open.is_some() {
                self.close_quick_open(window, cx);
            }
        } else {
            self.close_preview(cx);
            self.leave_app_views(cx);
            if let Some(monos) = &self.monos {
                monos.update(cx, |m, cx| m.close(cx));
            }
            if self.settings.is_some() {
                self.close_settings(window, cx);
            }
        }
        self.navigation_revision += 1;
        let restore = window.focused(cx);
        let (mono_roots, root_summaries) = self
            .monos
            .as_ref()
            .and_then(|monos| monos.read(cx).search_sources().ok())
            .unwrap_or_default();
        let mut summaries: HashMap<_, _> = locked(&self.store)
            .and_then(|store| store.search_summaries())
            .unwrap_or_else(|_| {
                self.order
                    .iter()
                    .map(|&id| self.session_metadata(id, cx).summary.clone())
                    .collect()
            })
            .into_iter()
            .map(|summary| (summary.id, summary))
            .collect();
        summaries.extend(
            root_summaries
                .into_iter()
                .map(|summary| (summary.id, summary)),
        );
        let conversations = self
            .sessions
            .values()
            .cloned()
            .chain(
                self.monos
                    .as_ref()
                    .map(|monos| monos.read(cx).search_conversations())
                    .unwrap_or_default(),
            )
            .filter(|conversation| summaries.contains_key(&conversation.read(cx).summary.id))
            .collect::<Vec<_>>();
        for conversation in &conversations {
            let summary = conversation.read(cx).summary.clone();
            summaries.insert(summary.id, summary);
        }
        let summaries = summaries.into_values().collect();
        let live = conversations
            .iter()
            // A paged Mono's saved search covers turns not loaded yet.
            .filter(|s| {
                s.read(cx).history_loaded() && s.read(cx).transcript.read(cx).history_complete()
            })
            .map(|s| {
                (
                    s.read(cx).summary.id,
                    Arc::new(s.read(cx).transcript.read(cx).history_rows().to_vec()),
                )
            })
            .collect();
        let cwd = self.working_copy().to_owned();
        let store = self.store.clone();
        let projects = self.workspace.projects.clone();
        let recent = self
            .workspace
            .recent_files
            .get(&cwd)
            .cloned()
            .unwrap_or_default();
        let view = cx.new(|cx| {
            SearchView::new(
                cwd,
                store,
                Sources {
                    summaries,
                    conversations,
                    mono_roots,
                    live,
                    projects,
                    recent,
                    classic: self.workspace.boxy_layout(),
                    panel,
                    project_appearance: self.workspace.project_appearance.clone(),
                },
                cx,
            )
        });
        if let Some(draft) = self.saved_search.clone() {
            view.update(cx, |v, cx| v.restore_draft(draft, cx));
        }
        let events = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &Event, window, cx| match event {
                Event::Close => this.close_app_search(window, cx),
                Event::Open(target, query) => {
                    let session = match target {
                        Target::Conversation(session) | Target::Message { session, .. } => {
                            Some(*session)
                        }
                        _ => None,
                    };
                    if let Some(session) = session
                        && let Some(monos) = &this.monos
                        && let Err(error) = monos.read(cx).search_session_access(session)
                    {
                        if let Some(search) = &this.app_search {
                            search
                                .view
                                .update(cx, |search, cx| search.set_navigation_error(error, cx));
                        }
                        return;
                    }
                    let panel = this.app_search.as_ref().is_some_and(|search| search.panel);
                    this.close_app_search(window, cx);
                    match target {
                        Target::Conversation(id) => {
                            this.open_search_session(*id, None, panel, window, cx)
                        }
                        Target::Message { session, row } => this.open_search_session(
                            *session,
                            Some((*row, query.clone())),
                            panel,
                            window,
                            cx,
                        ),
                        Target::File { path, location } => {
                            let cwd = this.working_copy().to_owned();
                            this.open_review(
                                crate::review::ReviewTab::File {
                                    cwd,
                                    path: path.clone(),
                                },
                                window,
                                cx,
                            );
                            if let Some((line, column)) = location
                                && let Some(pane) = this.active_review(cx)
                            {
                                pane.update(cx, |p, cx| {
                                    p.navigate_file(*line as usize, *column as usize, cx)
                                });
                            }
                            if panel && this.active_review(cx).is_some() {
                                this.clear_saved_search_query();
                            }
                        }
                        Target::Project(path) => {
                            this.select_project(path.clone(), window, cx);
                            if panel {
                                this.clear_saved_search_query();
                            }
                        }
                    }
                }
            },
        );
        window.focus(&view.focus_handle(cx), cx);
        self.app_search = Some(Open {
            view,
            panel,
            restore,
            _events: events,
        });
        if panel {
            cx.notify();
            return;
        }
        self.remember_title_tab(TitleTab::Search);
        self.reveal_active_tab(cx);
        self.update_chat_visibility(cx);
        self.persist(cx);
        self.invalidate_chrome(cx);
        cx.notify();
    }
    fn clear_saved_search_query(&mut self) {
        if let Some((query, _)) = &mut self.saved_search {
            query.clear();
        }
    }
    fn open_search_session(
        &mut self,
        id: i64,
        jump: Option<(usize, String)>,
        panel: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(monos) = &self.monos {
            match monos.update(cx, |monos, cx| {
                monos.open_search_session(id, jump.clone(), window, cx)
            }) {
                Ok(true) => {
                    if panel {
                        self.clear_saved_search_query();
                    }
                    return;
                }
                Err(error) => {
                    self.error = Some(error);
                    cx.notify();
                    return;
                }
                Ok(false) => {}
            }
        }
        if let Some(view) = self.ensure_session(id, cx) {
            self.open(id, window, cx);
            if let Some((row, query)) = jump {
                view.update(cx, |v, cx| v.reveal_search(row, query, cx));
            }
            if panel && self.workspace.active == Some(id) {
                self.clear_saved_search_query();
            }
            return;
        }
        let store = self.store.clone();
        let revision = self.navigation_revision;
        let work = cx.background_executor().spawn(async move {
            locked(&store)?
                .sessions()?
                .into_iter()
                .find(|s| s.id == id)
                .ok_or_else(|| "Conversation no longer exists".to_owned())
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.navigation_revision != revision {
                    return;
                }
                match result {
                    Ok(summary) => {
                        this.add(summary, cx);
                        this.open(id, window, cx);
                        if let Some((row, query)) = jump {
                            this.sessions[&id].update(cx, |v, cx| v.reveal_search(row, query, cx));
                        }
                        if panel && this.workspace.active == Some(id) {
                            this.clear_saved_search_query();
                        }
                    }
                    Err(error) => {
                        this.error = Some(error);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
    pub(super) fn sync_mono_search(&mut self, cx: &mut Context<Self>) {
        let Some(search) = &self.app_search else {
            return;
        };
        if let Some(monos) = &self.monos
            && let Ok((roots, summaries)) = monos.read(cx).search_sources()
        {
            search
                .view
                .update(cx, |search, cx| search.set_mono_roots(roots, summaries, cx));
        }
        let views = self
            .monos
            .as_ref()
            .map(|m| m.read(cx).search_conversations())
            .unwrap_or_default();
        for view in views {
            if !search
                .view
                .read(cx)
                .contains_session(view.read(cx).summary.id)
            {
                continue;
            }
            search
                .view
                .update(cx, |search, cx| search.watch_conversation(&view, cx));
            let transcript = view.read(cx).transcript.read(cx);
            if !transcript.history_complete() {
                continue;
            }
            let rows = Arc::new(transcript.history_rows().to_vec());
            let summary = view.read(cx).summary.clone();
            search
                .view
                .update(cx, |v, cx| v.update_session(summary, rows, cx));
        }
    }
    pub(super) fn sync_search_session(&mut self, id: i64, cx: &mut Context<Self>) {
        let Some(search) = &self.app_search else {
            return;
        };
        if !search.view.read(cx).contains_session(id)
            && !locked(&self.store)
                .and_then(|store| store.search_summaries())
                .is_ok_and(|summaries| summaries.iter().any(|summary| summary.id == id))
        {
            return;
        }
        let Some(session) = self.sessions.get(&id) else {
            return;
        };
        search
            .view
            .update(cx, |search, cx| search.watch_conversation(session, cx));
        let session = session.read(cx);
        let transcript = session.transcript.read(cx);
        if !transcript.history_complete() {
            return;
        }
        let summary = session.summary.clone();
        let rows = Arc::new(transcript.history_rows().to_vec());
        search
            .view
            .update(cx, |v, cx| v.update_session(summary, rows, cx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use monocode_core::search::Scope;
    #[gpui::test]
    fn search_preserves_disabled_mono_results_and_opens_enabled_roots(cx: &mut TestAppContext) {
        use monocode_core::{
            mono::{Mono, Roster},
            store::Store,
            transcript::Row,
        };
        use std::{sync::Mutex, time::Duration};
        cx.update(|cx| {
            input::bind(cx);
            super::super::bind_keys(cx);
            cx.set_reduce_motion(true);
        });
        for (query, panel) in [
            ("Chief", true),
            ("needle", true),
            ("Chief", false),
            ("needle", false),
        ] {
            let mut store = Store::open(std::path::Path::new(":memory:")).unwrap();
            let root = store.create("codex", "/Users/tom").unwrap();
            store.title(root.id, "MonoInvader").unwrap();
            store
                .write_rows(
                    root.id,
                    0,
                    &[Row {
                        role: "user".into(),
                        text: "needle saved history".into(),
                        first: true,
                        sent_at: None,
                    }],
                )
                .unwrap();
            let mut roster = Roster::default();
            roster.monos.push(Mono {
                id: "search-root".into(),
                name: "Chief".into(),
                session_id: Some(root.id),
                projects: vec!["/assigned".into()],
                ..Default::default()
            });
            roster.hidden_sessions.push(root.id);
            roster.save(&store).unwrap();
            let summaries = store.sessions().unwrap();
            let database = Arc::new(Mutex::new(store));
            let window = cx.update(|cx| {
                cx.open_window(Default::default(), |_, cx| {
                    cx.new(|cx| Shell::new(database, summaries, "/assigned".into(), None, cx))
                })
                .unwrap()
            });
            cx.run_until_parked();
            window
                .update(cx, |shell, window, cx| {
                    assert!(
                        shell
                            .mono_manager()
                            .read(cx)
                            .open_chat("search-root")
                            .is_none()
                    );
                    assert!(!shell.sessions.contains_key(&root.id));
                    shell.workspace.preferences.search_panel = panel;
                    shell.open_app_search(window, cx);
                    shell
                        .app_search
                        .as_ref()
                        .unwrap()
                        .view
                        .update(cx, |search, cx| {
                            search.restore_draft((query.into(), Scope::Conversations), cx)
                        });
                })
                .unwrap();
            cx.run_until_parked();
            cx.executor().advance_clock(Duration::from_millis(200));
            cx.run_until_parked();
            let search_view = window
                .update(cx, |shell, window, cx| {
                    shell.workspace.preferences.monos = false;
                    let preferences = shell.workspace.preferences.clone();
                    let manager = shell.mono_manager();
                    manager.update(cx, |manager, cx| {
                        manager.set_preferences(preferences, cx);
                        assert!(
                            manager
                                .open_search_session(root.id, None, window, cx)
                                .is_err()
                        );
                    });
                    shell.app_search.as_ref().unwrap().view.clone()
                })
                .unwrap();
            cx.run_until_parked();
            cx.simulate_keystrokes(window.into(), "enter");
            cx.run_until_parked();
            window
                .update(cx, |shell, window, cx| {
                    let search = shell.app_search.as_ref().unwrap();
                    assert_eq!(search.view.entity_id(), search_view.entity_id());
                    assert_eq!(
                        search.view.read(cx).draft(cx),
                        (query.into(), Scope::Conversations)
                    );
                    assert!(search.view.focus_handle(cx).is_focused(window));
                    assert!(shell.saved_search.is_none());
                    assert!(!shell.workspace.preferences.monos);
                    let manager = shell.mono_manager();
                    assert!(manager.read(cx).current().is_none());
                    assert!(manager.read(cx).open_chat("search-root").is_none());
                    assert!(!shell.sessions.contains_key(&root.id));
                })
                .unwrap();
            let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
            visual.run_until_parked();
            assert!(visual.debug_bounds("app-search-navigation-error").is_some());
            window
                .update(visual, |shell, _, cx| {
                    shell.workspace.preferences.monos = true;
                    let preferences = shell.workspace.preferences.clone();
                    shell
                        .mono_manager()
                        .update(cx, |manager, cx| manager.set_preferences(preferences, cx));
                })
                .unwrap();
            cx.simulate_keystrokes(window.into(), "enter");
            cx.run_until_parked();
            window
                .update(cx, |shell, _, cx| {
                    assert!(shell.app_search.is_none());
                    assert!(!shell.sessions.contains_key(&root.id));
                    let manager = shell.mono_manager();
                    let manager = manager.read(cx);
                    assert_eq!(manager.current().unwrap().id, "search-root");
                    let chat = manager.open_chat("search-root").unwrap();
                    assert_eq!(chat.read(cx).summary.id, root.id);
                    assert!(chat.read(cx).history_loaded());
                    assert_eq!(
                        shell.saved_search.as_ref().unwrap().0,
                        if panel { "" } else { query }
                    );
                    assert!(shell.error.is_none());
                })
                .unwrap();
        }
    }
    #[gpui::test]
    fn app_search_restores_draft_and_opens_cross_project_result(cx: &mut TestAppContext) {
        cx.update(|cx| {
            input::bind(cx);
            super::super::bind_keys(cx);
            cx.set_reduce_motion(true);
        });
        let (window, a, b) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, w, cx| {
                shell.open(b, w, cx);
                let input = shell.sessions[&b].read(cx).input.clone();
                input.update(cx, |i, cx| i.set_text("preserve this draft".into(), cx));
                w.focus(&input.focus_handle(cx), cx);
                shell.sessions[&a]
                    .update(cx, |v, _| v.summary.title = "Needle conversation".into());
                shell.focus_search(w, cx);
            })
            .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        window
            .update(cx, |shell, w, cx| {
                let view = &shell.app_search.as_ref().unwrap().view;
                assert!(view.focus_handle(cx).is_focused(w));
            })
            .unwrap();
        cx.simulate_keystrokes(window.into(), "escape");
        cx.run_until_parked();
        window
            .update(cx, |shell, w, cx| {
                assert!(shell.app_search.is_none());
                let input = &shell.sessions[&b].read(cx).input;
                assert_eq!(input.read(cx).content, "preserve this draft");
                assert!(input.focus_handle(cx).is_focused(w));
                shell.focus_search(w, cx);
            })
            .unwrap();
        cx.run_until_parked();
        cx.simulate_input(window.into(), "Needle");
        cx.run_until_parked();
        cx.simulate_keystrokes(window.into(), "enter");
        cx.run_until_parked();
        window
            .update(cx, |shell, window, cx| {
                assert!(shell.app_search.is_none());
                assert_eq!(shell.workspace.active, Some(a));
                assert_eq!(shell.workspace.project, "/work/another-project");
                assert_eq!(
                    shell.sessions[&b].read(cx).input.read(cx).content,
                    "preserve this draft"
                );
                shell.open_app_search(window, cx);
                assert_eq!(
                    shell.app_search.as_ref().unwrap().view.read(cx).draft(cx).0,
                    "Needle"
                );
            })
            .unwrap();
    }
    #[gpui::test]
    fn search_panel_clears_query_after_navigation(cx: &mut TestAppContext) {
        let (window, conversation, _) = crate::shell::tests::window(cx);
        let saved_conversation = window
            .update(cx, |shell, _, _| {
                shell.workspace.preferences.search_panel = true;
                locked(&shell.store)
                    .unwrap()
                    .create("codex", "/work/another-project")
                    .unwrap()
                    .id
            })
            .unwrap();
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml").to_owned();
        for (target, scope, navigates) in [
            (
                Target::Conversation(conversation),
                Scope::Conversations,
                true,
            ),
            (
                Target::Message {
                    session: conversation,
                    row: 0,
                },
                Scope::Conversations,
                true,
            ),
            (
                Target::File {
                    path: file.clone(),
                    location: None,
                },
                Scope::Files,
                true,
            ),
            (
                Target::File {
                    path: file,
                    location: Some((1, 1)),
                },
                Scope::Files,
                true,
            ),
            (
                Target::Project("/work/monocode".into()),
                Scope::Projects,
                true,
            ),
            (
                Target::Conversation(saved_conversation),
                Scope::Conversations,
                true,
            ),
            (Target::Conversation(i64::MAX), Scope::Conversations, false),
        ] {
            window
                .update(cx, |shell, window, cx| {
                    shell.open_app_search(window, cx);
                    let search = shell.app_search.as_ref().unwrap().view.clone();
                    search.update(cx, |search, cx| {
                        search.restore_draft(("query".into(), scope), cx);
                        cx.emit(Event::Open(target.clone(), "query".into()));
                    });
                })
                .unwrap();
            cx.run_until_parked();
            window
                .update(cx, |shell, window, cx| {
                    assert!(shell.app_search.is_none());
                    if navigates {
                        match &target {
                            Target::Conversation(id) | Target::Message { session: id, .. } => {
                                assert_eq!(shell.workspace.active, Some(*id));
                            }
                            Target::File { path, .. } => {
                                assert!(shell.active_review(cx).unwrap().read(cx).state.tabs.iter().any(
                                    |tab| matches!(tab, crate::review::ReviewTab::File { path: opened, .. } if opened == path)
                                ));
                            }
                            Target::Project(path) => assert_eq!(&shell.workspace.project, path),
                        }
                    } else {
                        assert_eq!(shell.error.as_deref(), Some("Conversation no longer exists"));
                    }
                    shell.open_app_search(window, cx);
                    assert_eq!(
                        shell.app_search.as_ref().unwrap().view.read(cx).draft(cx),
                        (if navigates { "" } else { "query" }.into(), scope)
                    );
                    shell.close_app_search(window, cx);
                })
                .unwrap();
            cx.run_until_parked();
        }
    }
    #[gpui::test]
    fn app_search_shares_tab_sizing_order_cycle_and_close(cx: &mut TestAppContext) {
        cx.update(|cx| {
            input::bind(cx);
            super::super::bind_keys(cx);
            cx.set_reduce_motion(true);
        });
        let (window, _, _) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, w, cx| {
                for _ in 0..10 {
                    let summary = locked(&shell.store)
                        .unwrap()
                        .create("codex", &shell.workspace.project)
                        .unwrap();
                    let id = summary.id;
                    shell.add(summary, cx);
                    shell.open(id, w, cx);
                }
                shell.open_app_search(w, cx);
                assert_eq!(shell.visible_tabs(cx).last(), Some(&TitleTab::Search));
                assert_eq!(shell.active_title_tab(cx), Some(TitleTab::Search));
            })
            .unwrap();
        cx.run_until_parked();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let strip = visual.debug_bounds("title-tab-strip").unwrap();
        let search = visual.debug_bounds("search-title-tab").unwrap();
        let new = visual.debug_bounds("new-tab").unwrap();
        assert!(
            new.right() <= strip.right(),
            "Search and every session must fit before the new-tab button"
        );
        assert!(search.right() <= new.left());
        assert!(
            new.left() - search.right() <= px(4.5),
            "Search uses the same 2px gap and button margin"
        );
        assert_eq!(search.size.width, px(SELECTED_TAB_MIN_WIDTH));
        window
            .update(visual, |shell, w, cx| {
                assert_eq!(shell.tab_scroll.max_offset().x, px(0.));
                shell.cycle(-1, w, cx);
                assert!(shell.app_search.is_none());
                assert!(matches!(
                    shell.active_title_tab(cx),
                    Some(TitleTab::Session(_))
                ));
                shell.cycle(1, w, cx);
                assert!(shell.app_search.is_some());
                shell.reorder_title_tab(&TitleTab::Search, 0, cx);
                assert_eq!(shell.visible_tabs(cx).first(), Some(&TitleTab::Search));
                shell.run_title_tab_menu(TitleTab::Search, "right", w, cx);
                assert_eq!(shell.visible_tabs(cx), vec![TitleTab::Search]);
                assert!(shell.app_search.is_some());
                shell.close_search_tab(w, cx);
                assert!(!shell.workspace.title_order.contains(&TitleTab::Search));
                assert!(shell.app_search.is_none() && shell.saved_search.is_none());
            })
            .unwrap();
        visual.run_until_parked();
    }
    #[gpui::test]
    fn search_panel_preference_floats_over_the_workspace_without_a_tab(cx: &mut TestAppContext) {
        cx.update(|cx| {
            input::bind(cx);
            super::super::bind_keys(cx);
            cx.set_reduce_motion(true);
        });
        let (window, _, active) = crate::shell::tests::window(cx);
        let tabs = window
            .update(cx, |shell, w, cx| {
                assert!(
                    !shell.workspace.preferences.search_panel,
                    "search opens as a page by default"
                );
                shell.workspace.preferences.search_panel = true;
                let tabs = shell.visible_tabs(cx);
                shell.open_app_search(w, cx);
                tabs
            })
            .unwrap();
        cx.run_until_parked();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        assert!(visual.debug_bounds("app-search").is_some());
        assert!(visual.debug_bounds("title-tab-strip").is_some());
        window
            .update(visual, |shell, _, cx| {
                assert!(!shell.search_page());
                assert_eq!(shell.visible_tabs(cx), tabs);
                assert!(!shell.workspace.title_order.contains(&TitleTab::Search));
                assert_eq!(shell.active_title_tab(cx), Some(TitleTab::Session(active)));
            })
            .unwrap();
        visual.simulate_input("scope query");
        for (keystroke, expected_scope) in [
            ("tab", Scope::Conversations),
            ("tab", Scope::Files),
            ("tab", Scope::Projects),
            ("tab", Scope::All),
            ("shift-tab", Scope::Projects),
            ("shift-tab", Scope::Files),
            ("shift-tab", Scope::Conversations),
            ("shift-tab", Scope::All),
        ] {
            visual.simulate_keystrokes(keystroke);
            window
                .update(visual, |shell, window, cx| {
                    let search = &shell.app_search.as_ref().unwrap().view;
                    assert_eq!(
                        search.read(cx).draft(cx),
                        ("scope query".into(), expected_scope)
                    );
                    assert!(search.focus_handle(cx).is_focused(window));
                })
                .unwrap();
        }
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        window
            .update(visual, |shell, w, cx| {
                assert!(shell.app_search.is_none());
                assert_eq!(shell.saved_search, Some(("scope query".into(), Scope::All)));
                shell.open_app_search(w, cx);
                assert_eq!(
                    shell.app_search.as_ref().unwrap().view.read(cx).draft(cx),
                    ("scope query".into(), Scope::All)
                );
                shell.close_app_search(w, cx);
                // Switching back to the page style takes effect on the next open.
                shell.workspace.preferences.search_panel = false;
                shell.open_app_search(w, cx);
                assert!(shell.search_page());
                assert_eq!(shell.active_title_tab(cx), Some(TitleTab::Search));
            })
            .unwrap();
    }
}
