use gpui::{
    AnyElement, App, Bounds, Context, Element, ElementId, Entity, GlobalElementId,
    InspectorElementId, LayoutId, Pixels, Render, StyleRefinement, Window, div, prelude::*, px,
};
use monocode_core::project_appearance::ProjectAppearance;

#[derive(Clone, PartialEq)]
pub(crate) enum Appearance {
    Project(ProjectAppearance, String),
    Mono(String, u32),
}
#[derive(Clone, PartialEq)]
pub(crate) struct WorkingIcon {
    appearance: Appearance,
    working: bool,
    side: f32,
}
impl WorkingIcon {
    pub(crate) fn project(
        appearance: ProjectAppearance,
        project: String,
        working: bool,
        side: f32,
    ) -> Self {
        Self {
            appearance: Appearance::Project(appearance, project),
            working,
            side,
        }
    }
    pub(crate) fn mono(name: &str, color: u32, working: bool, side: f32) -> Self {
        Self {
            appearance: Appearance::Mono(name.into(), color),
            working,
            side,
        }
    }
}
impl Render for WorkingIcon {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let side = self.side;
        let working = self.working;
        let appearance = self.appearance.clone();
        div()
            .size(px(side))
            .overflow_hidden()
            .child(match appearance {
                Appearance::Project(appearance, project) => {
                    crate::project_settings::working_project_icon(
                        &appearance,
                        &project,
                        side,
                        working,
                    )
                }
                Appearance::Mono(name, color) => crate::monos::avatar::avatar(
                    &name,
                    color,
                    if working { "working" } else { "idle" },
                    side,
                    true,
                ),
            })
    }
}
impl IntoElement for WorkingIcon {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for WorkingIcon {
    type RequestLayoutState = AnyElement;
    type PrepaintState = bool;
    fn id(&self) -> Option<ElementId> {
        Some("working-project-icon".into())
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, AnyElement) {
        let view =
            window.with_element_state(id.unwrap(), |state: Option<(Self, Entity<Self>)>, _| {
                let (previous, view) =
                    state.unwrap_or_else(|| (self.clone(), cx.new(|_| self.clone())));
                if previous != *self {
                    let appearance = self.clone();
                    let updated_view = view.clone();
                    cx.defer(move |cx| {
                        updated_view.update(cx, |current, cx| {
                            *current = appearance;
                            cx.notify();
                        })
                    });
                }
                (view.clone(), (self.clone(), view))
            });
        let mut element = view
            .cached(
                StyleRefinement::default()
                    .size(px(self.side))
                    .flex_shrink_0(),
            )
            .into_any_element();
        (element.request_layout(window, cx), element)
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        element: &mut AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let visible = bounds.intersects(&window.content_mask().bounds);
        if visible {
            element.prepaint(window, cx);
        }
        visible
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        element: &mut AnyElement,
        visible: &mut bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        if *visible {
            element.paint(window, cx);
        }
    }
}

pub(crate) fn talk_frame(elapsed: f32) -> bool {
    (elapsed * 1000. / 180.).floor() as u64 % 2 == 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use std::time::Duration;

    #[test]
    fn prompt_frames_change_every_180_ms() {
        assert!(!talk_frame(0.));
        assert!(!talk_frame(0.179));
        assert!(talk_frame(0.181));
        assert!(!talk_frame(0.361));
    }

    #[gpui::test]
    fn regular_and_mono_work_stops_for_all_terminal_states(cx: &mut TestAppContext) {
        for mono in [false, true] {
            cx.update(|cx| cx.set_reduce_motion(false));
            let (view, visual) = cx.add_window_view(|_, _| {
                if mono {
                    WorkingIcon::mono("cat", 0x459cf7, true, 14.)
                } else {
                    WorkingIcon::project(
                        ProjectAppearance::default(),
                        "/work/project".into(),
                        true,
                        14.,
                    )
                }
            });
            visual.run_until_parked();
            visual.update(|_, cx| assert!(crate::pulse::running(cx)));
            for _outcome in ["finished", "cancelled", "error"] {
                view.update(visual, |view, cx| {
                    view.working = false;
                    cx.notify();
                });
                visual.run_until_parked();
                visual.executor().advance_clock(Duration::from_secs(3));
                visual.run_until_parked();
                visual.update(|_, cx| assert!(!crate::pulse::running(cx)));
                view.update(visual, |view, cx| {
                    view.working = true;
                    cx.notify();
                });
                visual.run_until_parked();
            }
            view.update(visual, |view, cx| {
                view.working = false;
                cx.notify();
            });
            visual.run_until_parked();
            visual.executor().advance_clock(Duration::from_secs(3));
            visual.run_until_parked();
        }
    }

    struct IconHost {
        renders: usize,
        working: bool,
    }
    impl Render for IconHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.renders += 1;
            div().child(WorkingIcon::project(
                ProjectAppearance::default(),
                "/work/project".into(),
                self.working,
                14.,
            ))
        }
    }
    struct IconRoot(Entity<IconHost>, usize);
    impl Render for IconRoot {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            if window.view_layout_dirty(cx.entity_id()) {
                self.1 += 1;
            }
            self.0
                .clone()
                .cached(StyleRefinement::default().size_full())
        }
    }
    #[gpui::test]
    fn icon_ticks_do_not_invalidate_parent_layout(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(false));
        let (root, visual) = cx.add_window_view(|_, cx| {
            IconRoot(
                cx.new(|_| IconHost {
                    renders: 0,
                    working: true,
                }),
                0,
            )
        });
        let host = root.read_with(visual, |root, _| root.0.clone());
        visual.run_until_parked();
        let layout_changes = root.read_with(visual, |root, _| root.1);
        visual.executor().advance_clock(Duration::from_millis(500));
        visual.run_until_parked();
        assert_eq!(layout_changes, root.read_with(visual, |root, _| root.1));
        host.update(visual, |host, cx| {
            host.working = false;
            cx.notify();
        });
        visual.run_until_parked();
        visual.executor().advance_clock(Duration::from_secs(3));
        visual.run_until_parked();
        visual.update(|_, cx| assert!(!crate::pulse::running(cx)));
    }

    struct ClippedIcon {
        visible: bool,
        project: String,
    }
    impl Render for ClippedIcon {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size(px(32.)).overflow_hidden().child(
                div()
                    .relative()
                    .top(px(if self.visible { 0. } else { 100. }))
                    .child(WorkingIcon::project(
                        ProjectAppearance::default(),
                        self.project.clone(),
                        true,
                        14.,
                    )),
            )
        }
    }
    #[gpui::test]
    fn offscreen_project_and_logo_icons_release_clock(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(false));
        for project in ["/work/project", "remote://machine/project"] {
            let (view, visual) = cx.add_window_view(|_, _| ClippedIcon {
                visible: false,
                project: project.into(),
            });
            visual.run_until_parked();
            visual.update(|_, cx| assert!(!crate::pulse::running(cx)));
            view.update(visual, |view, cx| {
                view.visible = true;
                cx.notify();
            });
            visual.run_until_parked();
            visual.update(|_, cx| assert!(crate::pulse::running(cx)));
            view.update(visual, |view, cx| {
                view.visible = false;
                cx.notify();
            });
            visual.run_until_parked();
            visual.executor().advance_clock(Duration::from_secs(3));
            visual.run_until_parked();
            visual.update(|_, cx| assert!(!crate::pulse::running(cx)));
        }
    }

    #[gpui::test]
    fn reduced_motion_does_not_lease_clock(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (_, visual) = cx.add_window_view(|_, _| {
            WorkingIcon::project(
                ProjectAppearance::default(),
                "/work/project".into(),
                true,
                12.,
            )
        });
        visual.run_until_parked();
        visual.update(|_, cx| assert!(!crate::pulse::running(cx)));
    }
}
