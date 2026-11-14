//! Native counterpart of HarnessUpdateNotice.tsx. The parent positions this
//! entity in the shared top-right notification stack.
use crate::theme::*;
use gpui::{Context, EventEmitter, Render, Role, Window, div, prelude::*, px};
use monocode_core::{
    harness_updates::{self, Update},
    provider::Kind,
};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub(crate) enum HarnessUpdatesEvent {
    Changed,
    Updated(Kind),
}
#[derive(Clone)]
enum RowState {
    Updating,
    Updated(String),
    Failed(String),
}
pub(crate) struct HarnessUpdates {
    updates: Vec<Update>,
    rows: HashMap<Kind, RowState>,
    checking: bool,
    generation: u64,
    automatic_enabled: bool,
    automatic_cancel: Option<Arc<AtomicBool>>,
    automatic_timer: Option<gpui::Task<()>>,
    last_check: Option<Instant>,
    pending_automatic: Option<Vec<String>>,
}
impl EventEmitter<HarnessUpdatesEvent> for HarnessUpdates {}
impl HarnessUpdates {
    pub(crate) fn new(
        hidden: Vec<String>,
        automatic_enabled: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            updates: Vec::new(),
            rows: HashMap::new(),
            checking: false,
            generation: 0,
            automatic_enabled,
            automatic_cancel: None,
            automatic_timer: None,
            last_check: None,
            pending_automatic: None,
        };
        if automatic_enabled && harness_updates::claim_launch_check() {
            this.check(hidden, true, cx);
        }
        this
    }
    pub(crate) fn set_automatic(
        &mut self,
        enabled: bool,
        hidden: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if self.automatic_enabled == enabled {
            return;
        }
        self.automatic_enabled = enabled;
        self.automatic_timer = None;
        self.pending_automatic = None;
        if !enabled {
            if let Some(cancel) = self.automatic_cancel.take() {
                cancel.store(true, Ordering::SeqCst);
                self.generation += 1;
                self.checking = false;
            }
            return;
        }
        self.schedule_automatic(hidden, cx);
    }
    fn schedule_automatic(&mut self, hidden: Vec<String>, cx: &mut Context<Self>) {
        let delay = self
            .last_check
            .map(|last| Duration::from_secs(60).saturating_sub(last.elapsed()))
            .unwrap_or_default();
        self.automatic_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |this, cx| {
                this.automatic_timer = None;
                if this.automatic_enabled {
                    if this.busy() {
                        this.pending_automatic = Some(hidden);
                    } else {
                        this.check(hidden, true, cx);
                    }
                }
            });
        }));
    }
    pub(crate) fn is_visible(&self) -> bool {
        !self.updates.is_empty()
    }
    fn busy(&self) -> bool {
        self.rows.values().any(|s| matches!(s, RowState::Updating))
    }
    /// Explicit refresh is permitted after the launch claim or a dismissal.
    pub(crate) fn check_now(&mut self, hidden: Vec<String>, cx: &mut Context<Self>) {
        self.check(hidden, false, cx);
    }
    fn check(&mut self, hidden: Vec<String>, automatic: bool, cx: &mut Context<Self>) {
        if (automatic && !self.automatic_enabled) || self.checking || self.busy() {
            return;
        }
        self.automatic_timer = None;
        self.pending_automatic = None;
        self.last_check = Some(Instant::now());
        let cancel = Arc::new(AtomicBool::new(false));
        self.automatic_cancel = automatic.then(|| cancel.clone());
        self.checking = true;
        self.generation += 1;
        let generation = self.generation;
        let work = cx.background_executor().spawn(async move {
            harness_updates::find_updates_while(&hidden, || !cancel.load(Ordering::SeqCst))
        });
        cx.spawn(async move |this, cx| {
            let updates = work.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.checking = false;
                this.automatic_cancel = None;
                this.updates = updates;
                this.rows.clear();
                cx.emit(HarnessUpdatesEvent::Changed);
                cx.notify();
            });
        })
        .detach();
    }
    pub(crate) fn dismiss(&mut self, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        if let Some(cancel) = self.automatic_cancel.take() {
            cancel.store(true, Ordering::SeqCst);
        }
        self.generation += 1;
        self.checking = false;
        self.updates.clear();
        self.rows.clear();
        cx.emit(HarnessUpdatesEvent::Changed);
        cx.notify();
    }
    fn start(&mut self, targets: Vec<Update>, cx: &mut Context<Self>) {
        for update in targets {
            let kind = update.harness;
            if matches!(
                self.rows.get(&kind),
                Some(RowState::Updating | RowState::Updated(_))
            ) {
                continue;
            }
            self.rows.insert(kind, RowState::Updating);
            let work = cx
                .background_executor()
                .spawn(async move { harness_updates::run_update(&update) });
            cx.spawn(async move |this, cx| {
                let result = work.await;
                let _ = this.update(cx, |this, cx| {
                    this.finish_update(kind, result, cx);
                });
            })
            .detach();
        }
        cx.notify();
    }
    fn finish_update(
        &mut self,
        kind: Kind,
        result: Result<String, String>,
        cx: &mut Context<Self>,
    ) {
        let state = match result {
            Ok(version) => {
                cx.emit(HarnessUpdatesEvent::Updated(kind));
                RowState::Updated(version)
            }
            Err(error) => RowState::Failed(error),
        };
        self.rows.insert(kind, state);
        if !self.busy()
            && self.automatic_enabled
            && let Some(hidden) = self.pending_automatic.take()
        {
            self.schedule_automatic(hidden, cx);
        }
        cx.emit(HarnessUpdatesEvent::Changed);
        cx.notify();
    }
}
impl Render for HarnessUpdates {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.is_visible() {
            return div().into_any_element();
        }
        let busy = self.busy();
        let pending = self
            .updates
            .iter()
            .filter(|u| {
                !matches!(
                    self.rows.get(&u.harness),
                    Some(RowState::Updating | RowState::Updated(_))
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        let updated = self
            .rows
            .values()
            .any(|s| matches!(s, RowState::Updated(_)));
        let mut header = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .py(px(8.))
            .border_b_1()
            .border_color(content(0.1))
            .child(
                div()
                    .flex_1()
                    .text_size(px(12.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(if self.updates.len() == 1 {
                        "Harness update available"
                    } else {
                        "Harness updates available"
                    }),
            );
        if pending.len() > 1 {
            header = header.child(
                div()
                    .id("update-all-harnesses")
                    .role(Role::Button)
                    .aria_label("Update all")
                    .cursor_pointer()
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(6.))
                    .text_size(px(11.))
                    .text_color(content(0.7))
                    .hover(|s| s.bg(content(0.1)))
                    .on_click(cx.listener(move |this, _, _, cx| this.start(pending.clone(), cx)))
                    .child("Update all"),
            );
        }
        header = header.child(
            div()
                .id("dismiss-harness-updates")
                .role(Role::Button)
                .aria_label("Dismiss harness updates")
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .text_color(content(if busy { 0.16 } else { 0.4 }))
                .when(!busy, |el| {
                    el.cursor_pointer()
                        .hover(|s| s.bg(content(0.1)))
                        .on_click(cx.listener(|this, _, _, cx| this.dismiss(cx)))
                })
                .child(icon("x", 12.)),
        );
        let mut panel = div().relative().child(header);
        for update in &self.updates {
            let state = self.rows.get(&update.harness);
            let mut line = div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(provider_icon(update.harness.id(), 16.))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(13.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(update.harness.label()),
                );
            if let Some(RowState::Updated(version)) = state {
                line = line.child(
                    div()
                        .text_size(px(11.))
                        .text_color(gpui::rgb(0x34d399))
                        .child(format!("✓ Updated to {version}")),
                );
            } else {
                let updating = matches!(state, Some(RowState::Updating));
                let target = update.clone();
                line = line
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_family("monospace")
                            .text_color(content(0.5))
                            .child(format!("{} → {}", update.installed, update.latest)),
                    )
                    .child(
                        div()
                            .id(gpui::SharedString::from(format!(
                                "update-harness-{}",
                                update.harness.id()
                            )))
                            .px(px(8.))
                            .py(px(4.))
                            .rounded(px(6.))
                            .bg(content(0.1))
                            .text_size(px(11.))
                            .opacity(if updating { 0.6 } else { 1. })
                            .when(!updating, |el| {
                                el.cursor_pointer().hover(|s| s.bg(content(0.15))).on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        this.start(vec![target.clone()], cx)
                                    }),
                                )
                            })
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .when(updating, |el| {
                                el.child(crate::pulse::looping(
                                    "harness-update-spinner",
                                    std::time::Duration::from_secs(1),
                                    crate::pulse::Rate::Smooth,
                                    move |delta| {
                                        icon("loader", 12.).with_transformation(
                                            gpui::Transformation::rotate(gpui::percentage(delta)),
                                        )
                                    },
                                ))
                            })
                            .child(if updating {
                                "Updating"
                            } else if matches!(state, Some(RowState::Failed(_))) {
                                "Retry"
                            } else {
                                "Update"
                            }),
                    );
            }
            let mut row = div()
                .px(px(12.))
                .py(px(10.))
                .border_b_1()
                .border_color(content(0.1))
                .child(line);
            if let Some(RowState::Failed(error)) = state {
                row = row.child(
                    div()
                        .mt(px(6.))
                        .text_size(px(11.))
                        .text_color(gpui::rgb(0xfca5a5))
                        .child(error.clone()),
                );
            }
            panel = panel.child(row);
        }
        super::surface::glass_frame()
            .id("harness-update-notice")
            .role(Role::Status)
            .aria_label("Harness updates")
            .child(
                panel.child(
                    div()
                        .px(px(12.))
                        .py(px(8.))
                        .text_size(px(11.))
                        .text_color(content(0.5))
                        .child(if updated {
                            "Model picker refreshed with the new version’s models."
                        } else {
                            "New models often need the latest version."
                        }),
                ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hidden_providers() -> Vec<String> {
        Kind::ALL.into_iter().map(|kind| kind.id().into()).collect()
    }

    #[gpui::test]
    fn disabled_automatic_checks_allow_manual_checks(cx: &mut gpui::TestAppContext) {
        let notice = cx.new(|cx| HarnessUpdates::new(hidden_providers(), false, cx));
        notice.update(cx, |notice, cx| {
            notice.check(hidden_providers(), true, cx);
            assert!(!notice.checking);
            notice.check_now(hidden_providers(), cx);
            assert!(notice.checking);
            assert!(notice.automatic_cancel.is_none());
            notice.set_automatic(true, hidden_providers(), cx);
            notice.set_automatic(false, hidden_providers(), cx);
            assert!(notice.checking, "manual checks must remain active");
            assert!(notice.automatic_timer.is_none());
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn disabling_cancels_automatic_work_and_pending_schedule(cx: &mut gpui::TestAppContext) {
        let notice = cx.new(|cx| HarnessUpdates::new(hidden_providers(), false, cx));
        notice.update(cx, |notice, cx| {
            notice.updates.push(Update {
                harness: Kind::Codex,
                installed: "1.0.0".into(),
                latest: "2.0.0".into(),
            });
            notice.automatic_enabled = true;
            notice.check(hidden_providers(), true, cx);
            let cancel = notice.automatic_cancel.clone().unwrap();
            let generation = notice.generation;
            notice.set_automatic(false, hidden_providers(), cx);
            assert!(cancel.load(Ordering::SeqCst));
            assert_ne!(notice.generation, generation);
            assert!(!notice.checking);
            notice.set_automatic(true, hidden_providers(), cx);
            assert!(notice.automatic_timer.is_some());
            notice.set_automatic(true, hidden_providers(), cx);
            notice.set_automatic(false, hidden_providers(), cx);
            assert!(notice.automatic_timer.is_none());
        });
        cx.executor().advance_clock(Duration::from_secs(61));
        cx.run_until_parked();
        notice.update(cx, |notice, _| {
            assert!(!notice.checking);
            assert_eq!(
                notice.updates.len(),
                1,
                "cancelled results must not replace notices"
            );
        });
    }
    #[gpui::test]
    fn automatic_check_waits_for_all_manual_updates(cx: &mut gpui::TestAppContext) {
        let notice = cx.new(|cx| HarnessUpdates::new(hidden_providers(), false, cx));
        notice.update(cx, |notice, cx| {
            notice.rows.insert(Kind::Codex, RowState::Updating);
            notice.rows.insert(Kind::Claude, RowState::Updating);
            notice.set_automatic(true, hidden_providers(), cx);
        });
        cx.run_until_parked();
        notice.update(cx, |notice, cx| {
            assert!(notice.pending_automatic.is_some());
            assert!(notice.automatic_timer.is_none());
            assert_eq!(notice.generation, 0);
            notice.finish_update(Kind::Codex, Ok("2.0.0".into()), cx);
            assert!(notice.pending_automatic.is_some());
            assert!(notice.automatic_timer.is_none());
            notice.last_check = Some(Instant::now());
            notice.finish_update(Kind::Claude, Err("update failed".into()), cx);
            assert!(notice.pending_automatic.is_none());
            assert!(notice.automatic_timer.is_some());
        });
        cx.run_until_parked();
        notice.update(cx, |notice, _| assert_eq!(notice.generation, 0));
        cx.executor().advance_clock(Duration::from_secs(61));
        cx.run_until_parked();
        notice.update(cx, |notice, _| {
            assert_eq!(notice.generation, 1);
            assert!(notice.automatic_timer.is_none());
        });
        cx.executor().advance_clock(Duration::from_secs(120));
        cx.run_until_parked();
        notice.update(cx, |notice, _| assert_eq!(notice.generation, 1));
    }

    #[gpui::test]
    fn disabling_pending_check_prevents_check_after_manual_update(cx: &mut gpui::TestAppContext) {
        let notice = cx.new(|cx| HarnessUpdates::new(hidden_providers(), false, cx));
        notice.update(cx, |notice, cx| {
            notice.rows.insert(Kind::Codex, RowState::Updating);
            notice.set_automatic(true, hidden_providers(), cx);
        });
        cx.run_until_parked();
        notice.update(cx, |notice, cx| {
            assert!(notice.pending_automatic.is_some());
            notice.set_automatic(false, hidden_providers(), cx);
            assert!(notice.pending_automatic.is_none());
            notice.finish_update(Kind::Codex, Ok("2.0.0".into()), cx);
            assert!(matches!(
                notice.rows.get(&Kind::Codex),
                Some(RowState::Updated(_))
            ));
            assert!(notice.automatic_timer.is_none());
        });
        cx.executor().advance_clock(Duration::from_secs(120));
        cx.run_until_parked();
        notice.update(cx, |notice, _| assert_eq!(notice.generation, 0));
    }
}
