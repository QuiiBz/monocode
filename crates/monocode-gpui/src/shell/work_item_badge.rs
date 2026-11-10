use super::*;
use monocode_core::{
    inbox::{Checks, Item},
    session_groups::WorkItem,
};
use std::time::{Duration, Instant};

const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

pub(super) struct Badge {
    pub(super) item: WorkItem,
    session: i64,
    cwd: String,
    compact: bool,
    metadata: Option<Item>,
    comments: Option<u64>,
    checks: Option<Checks>,
    checked: Option<Instant>,
    unavailable: bool,
    loading: bool,
    refresh: Option<gpui::Task<()>>,
}

impl Badge {
    pub(super) fn new(session: i64, item: WorkItem, cwd: String, compact: bool) -> Self {
        Self {
            item,
            session,
            cwd,
            compact,
            metadata: None,
            comments: None,
            checks: None,
            checked: None,
            unavailable: false,
            loading: false,
            refresh: None,
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.loading
            || self
                .checked
                .is_some_and(|checked| checked.elapsed() < REFRESH_INTERVAL)
        {
            return;
        }
        self.checked = Some(Instant::now());
        self.loading = true;
        let item = self.item.clone();
        let cwd = self.cwd.clone();
        self.refresh = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let number = i64::try_from(item.number)
                        .map_err(|_| "Invalid GitHub number".to_string())?;
                    monocode_core::inbox::github_work_item_overview(
                        std::path::Path::new(&cwd),
                        &item.repo,
                        &item.kind,
                        number,
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                this.unavailable = result.is_err();
                if let Ok(metadata) = result {
                    this.comments = Some(metadata.comments);
                    this.checks = metadata.checks;
                    this.metadata = Some(metadata.item);
                }
                cx.notify();
            });
            cx.background_executor().timer(REFRESH_INTERVAL).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        }));
    }
}

fn presentation(pr: bool, metadata: Option<&Item>) -> (&'static str, &'static str, gpui::Hsla) {
    let open_icon = if pr { "git-pull-request" } else { "circle-dot" };
    match metadata {
        Some(item) if item.state.eq_ignore_ascii_case("merged") => {
            ("git-merge", "Merged", rgb(0xa78bfa).into())
        }
        Some(item) if item.state.eq_ignore_ascii_case("closed") => (
            if pr {
                "git-pull-request-closed"
            } else {
                "circle-x"
            },
            "Closed",
            rgb(0xfb7185).into(),
        ),
        Some(item) if item.draft => ("git-pull-request-draft", "Draft", content(0.5)),
        Some(item) if item.state.eq_ignore_ascii_case("open") => {
            (open_icon, "Open", rgb(0x34d399).into())
        }
        _ => (open_icon, "Status unavailable", content(0.5)),
    }
}

fn check_status(checks: &Checks) -> Option<(&'static str, u32)> {
    let states: Vec<_> = checks
        .checks
        .iter()
        .map(|check| check.state.as_str())
        .collect();
    if states
        .iter()
        .any(|state| matches!(*state, "fail" | "cancel"))
    {
        Some(("Checks failed", 0xfb7185))
    } else if states.contains(&"pending") {
        Some(("Checks pending", 0xfbbf24))
    } else if states.contains(&"pass")
        && states
            .iter()
            .all(|state| matches!(*state, "pass" | "skipping"))
    {
        Some(("Checks passed", 0x34d399))
    } else {
        None
    }
}

fn review_status(checks: Option<&Checks>) -> Option<(&'static str, &'static str, u32)> {
    match checks.map(|checks| checks.review_decision.as_str()) {
        Some("CHANGES_REQUESTED") => Some(("file-diff", "Changes requested", 0xfb7185)),
        Some("APPROVED") => Some(("check", "Approved", 0x34d399)),
        Some("REVIEW_REQUIRED" | "") => Some(("", "Not approved", 0xfbbf24)),
        _ => None,
    }
}

fn status_pill(checks: Option<&Checks>) -> Option<AnyElement> {
    let review = review_status(checks);
    let check = checks.and_then(check_status).map(|(label, color)| {
        (
            match label {
                "Checks passed" => "check-circle",
                "Checks pending" => "clock",
                _ => "alert-circle",
            },
            label,
            color,
        )
    });
    if review.is_none() && check.is_none() {
        return None;
    }
    Some(
        div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .rounded_full()
            .bg(content(0.07))
            .px_2()
            .py(px(2.))
            .text_size(px(10.))
            .font_weight(FontWeight::MEDIUM)
            .whitespace_nowrap()
            .when_some(review, |d, (glyph, label, color)| {
                d.when(!glyph.is_empty(), |d| {
                    d.child(icon(glyph, 11.).text_color(rgb(color)))
                })
                .child(div().text_color(rgb(color)).child(label))
            })
            .when(review.is_some() && check.is_some(), |d| {
                d.child(div().text_color(content(0.35)).child("·"))
            })
            .when_some(check, |d, (glyph, label, color)| {
                d.child(icon(glyph, 11.).text_color(rgb(color)))
                    .child(div().text_color(rgb(color)).child(label))
            })
            .into_any_element(),
    )
}

fn check_icon(
    glyph: &'static str,
    size: f32,
    color: gpui::Hsla,
    checks: Option<&Checks>,
) -> impl IntoElement {
    div()
        .relative()
        .size(px(size))
        .flex_shrink_0()
        .child(icon(glyph, size).text_color(color))
        .when_some(checks.and_then(check_status), |d, (label, color)| {
            d.child(
                div()
                    .id("github-check-status")
                    .role(Role::Image)
                    .aria_label(label)
                    .absolute()
                    .bottom(px(-2.))
                    .right(px(-2.))
                    .size(px(7.))
                    .rounded_full()
                    .border_1()
                    .border_color(pane_fill())
                    .bg(rgb(color)),
            )
        })
        .when_some(
            review_status(checks).filter(|(glyph, _, _)| !glyph.is_empty()),
            |d, (glyph, label, color)| {
                d.child(
                    div()
                        .id("github-review-status")
                        .role(Role::Image)
                        .aria_label(label)
                        .absolute()
                        .top(px(-3.))
                        .right(px(-4.))
                        .size(px(10.))
                        .rounded_full()
                        .bg(pane_fill())
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon(glyph, 9.).text_color(rgb(color))),
                )
            },
        )
}

impl Render for Badge {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh(cx);
        let pr = self.item.kind == "pr";
        let (glyph, status, color) = presentation(pr, self.metadata.as_ref());
        let noun = if pr { "PR" } else { "Issue" };
        let badge = cx.entity();
        let url = self.item.url.clone();
        div()
            .id(("work-item", self.session as usize))
            .role(Role::Link)
            .aria_label(format!("Open {noun} #{} · {status}", self.item.number))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(2.))
            .px(px(2.))
            .rounded_sm()
            .text_size(px(11.))
            .text_color(color)
            .child(check_icon(
                glyph,
                18.,
                color,
                self.checks.as_ref().filter(|_| !self.unavailable),
            ))
            .when(!self.compact, |d| d.child(format!("#{}", self.item.number)))
            .cursor_pointer()
            .when(self.compact, |d| {
                d.border_b_1()
                    .border_color(gpui::transparent_black())
                    .hover(move |s| s.border_color(color))
            })
            .when(!self.compact, |d| d.hover(|s| s.underline()))
            .tooltip_show_delay(Duration::ZERO)
            .tooltip(move |_, cx| {
                cx.new(|cx| {
                    let observation = cx.observe(&badge, |_, _, cx| cx.notify());
                    BadgeTooltip {
                        badge: badge.clone(),
                        _observation: observation,
                    }
                })
                .into()
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
            .on_mouse_up(MouseButton::Middle, {
                let url = url.clone();
                move |_, _, cx| {
                    cx.stop_propagation();
                    cx.open_url(&url);
                }
            })
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                cx.open_url(&url);
            })
    }
}

struct BadgeTooltip {
    badge: Entity<Badge>,
    _observation: Subscription,
}

impl Render for BadgeTooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let badge = self.badge.read(cx);
        let (glyph, _, color) = presentation(badge.item.kind == "pr", badge.metadata.as_ref());
        let title = badge
            .metadata
            .as_ref()
            .map(|item| item.title.clone())
            .unwrap_or_else(|| {
                if badge.unavailable {
                    "Details are unavailable"
                } else {
                    "Loading details…"
                }
                .into()
            });
        let updated = badge
            .metadata
            .as_ref()
            .map(|item| crate::inbox::relative_time(&item.updated_at))
            .unwrap_or_default();
        div()
            .id("sidebar-github-work-item-tooltip")
            .role(Role::Tooltip)
            .relative()
            .w(px(360.))
            .child(
                canvas(
                    |_, _, _| {},
                    |bounds, _, window, _| crate::popover::paint_surface(bounds, window),
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .p(px(14.))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(11.))
                            .child(icon(glyph, 18.).text_color(color))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(content(0.65))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(badge.item.repo.clone()),
                            )
                            .child(
                                div()
                                    .font_family(crate::fonts::mono())
                                    .text_color(content(0.5))
                                    .child(format!("#{}", badge.item.number)),
                            )
                            .when(!badge.unavailable, |d| {
                                d.children(status_pill(badge.checks.as_ref()))
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .line_height(px(13. * 1.35))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(content(1.))
                            .line_clamp(2)
                            .text_ellipsis()
                            .child(title),
                    )
                    .when(!updated.is_empty(), |d| {
                        d.child(
                            div()
                                .text_size(px(11.))
                                .text_color(content(0.4))
                                .whitespace_nowrap()
                                .child(match badge.comments {
                                    Some(count) => format!(
                                        "Updated {updated} · {count} {}",
                                        if count == 1 { "comment" } else { "comments" }
                                    ),
                                    None => format!("Updated {updated}"),
                                }),
                        )
                    })
                    .when(badge.unavailable && badge.metadata.is_some(), |d| {
                        d.child(
                            div()
                                .text_size(px(11.))
                                .text_color(content(0.5))
                                .child("Could not refresh GitHub status."),
                        )
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_status_labels_and_colors_follow_approval() {
        for (decision, expected) in [
            (
                "CHANGES_REQUESTED",
                Some(("file-diff", "Changes requested", 0xfb7185)),
            ),
            ("APPROVED", Some(("check", "Approved", 0x34d399))),
            ("REVIEW_REQUIRED", Some(("", "Not approved", 0xfbbf24))),
            ("", Some(("", "Not approved", 0xfbbf24))),
            ("UNKNOWN", None),
        ] {
            let checks = Checks {
                review_decision: decision.into(),
                ..Default::default()
            };
            assert_eq!(review_status(Some(&checks)), expected);
        }
        assert!(review_status(None).is_none());
    }

    #[test]
    fn check_indicator_handles_mixed_and_missing_results() {
        for (states, expected) in [
            (vec![], None),
            (vec!["pass", "pass"], Some("Checks passed")),
            (vec!["pass", "skipping"], Some("Checks passed")),
            (vec!["skipping"], None),
            (vec!["pass", "pending"], Some("Checks pending")),
            (vec!["pending", "fail"], Some("Checks failed")),
            (vec!["pass", "cancel"], Some("Checks failed")),
            (vec!["pass", "unknown"], None),
        ] {
            let checks = Checks {
                checks: states
                    .into_iter()
                    .map(|state| monocode_core::inbox::Check {
                        state: state.into(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            };
            assert_eq!(check_status(&checks).map(|(label, _)| label), expected);
        }
    }

    #[test]
    fn pr_status_controls_icon_and_color() {
        for (state, draft, glyph, label, color) in [
            (
                "open",
                false,
                "git-pull-request",
                "Open",
                rgb(0x34d399).into(),
            ),
            ("merged", true, "git-merge", "Merged", rgb(0xa78bfa).into()),
            (
                "closed",
                true,
                "git-pull-request-closed",
                "Closed",
                rgb(0xfb7185).into(),
            ),
            (
                "OPEN",
                true,
                "git-pull-request-draft",
                "Draft",
                content(0.5),
            ),
            (
                "unknown",
                false,
                "git-pull-request",
                "Status unavailable",
                content(0.5),
            ),
        ] {
            let metadata = Item {
                state: state.into(),
                draft,
                ..Default::default()
            };
            assert_eq!(presentation(true, Some(&metadata)), (glyph, label, color));
        }
        assert_eq!(presentation(true, None).1, "Status unavailable");
    }
}
