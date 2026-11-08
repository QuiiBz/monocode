use super::*;
use gpui::{Entity, ScrollDelta, ScrollWheelEvent, VisualTestContext, point, size};

struct UsageHost {
    footer: Entity<UsageFooter>,
    bottom_inset: f32,
}

impl Render for UsageHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .justify_end()
            .pb(px(self.bottom_inset))
            .child(self.footer.clone())
    }
}

fn snapshot(reset_count: usize) -> Snapshot {
    Snapshot {
        windows: [300, 10_080]
            .into_iter()
            .map(|minutes| usage::Window {
                used_percent: 32.,
                minutes,
                resets_at: Some(usage::now_ms() + 3_600_000),
            })
            .collect(),
        updated_at: usage::now_ms(),
        reset_credits: Some(usage::ResetCredits {
            available_count: reset_count as u64,
            credits: (0..reset_count)
                .map(|i| usage::ResetCredit {
                    id: i.to_string(),
                    status: "available".into(),
                    reset_type: "codexRateLimits".into(),
                    granted_at: None,
                    expires_at: Some(usage::now_ms() + 3_600_000),
                    title: Some("Full reset (Weekly + 5 hr)".into()),
                    description: Some(
                        "Thanks for using Codex! You've been granted one free rate limit reset."
                            .into(),
                    ),
                })
                .collect(),
        }),
        ..Default::default()
    }
}

#[gpui::test]
fn two_reset_cards_scroll_fully_into_view_at_small_window_heights(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            let footer = cx.new(|cx| {
                let mut footer = UsageFooter::new(Kind::Codex, cx);
                footer.cache[0].complete(Ok(snapshot(2)));
                footer.open = true;
                footer
            });
            cx.new(|_| UsageHost {
                footer,
                bottom_inset: 0.,
            })
        })
        .unwrap()
    });
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    for height in [900., 520., 260.] {
        visual.simulate_resize(size(px(820.), px(height)));
        for bottom_inset in [0., height / 2.] {
            window
                .update(visual, |host, _, cx| {
                    host.bottom_inset = bottom_inset;
                    cx.notify();
                })
                .unwrap();
            visual.run_until_parked();
            visual.refresh().unwrap();
            let scroll = visual.debug_bounds("usage-details-scroll").unwrap();
            assert!(scroll.top() >= px(19.), "scroll={scroll:?}");
            assert!(scroll.bottom() <= px(height - 19.), "scroll={scroll:?}");
            visual.simulate_event(ScrollWheelEvent {
                position: scroll.center(),
                delta: ScrollDelta::Pixels(point(px(0.), px(-2000.))),
                ..Default::default()
            });
            visual.refresh().unwrap();
            let last_card = visual.debug_bounds("banked-reset-credit:1").unwrap();
            let button = visual
                .debug_bounds("banked-reset-Use-reset-credit:1")
                .unwrap();
            assert!(last_card.top() >= scroll.top(), "card={last_card:?}");
            assert!(last_card.bottom() <= scroll.bottom(), "card={last_card:?}");
            assert!(scroll.contains(&button.center()));
            assert!(button.bottom() <= scroll.bottom());
            window
                .update(visual, |host, _, cx| {
                    assert!(host.footer.read(cx).confirm_reset.is_none());
                })
                .unwrap();
        }
    }
}

#[gpui::test]
fn short_usage_content_keeps_its_natural_height(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            let footer = cx.new(|cx| {
                let mut footer = UsageFooter::new(Kind::Codex, cx);
                footer.cache[0].complete(Ok(snapshot(0)));
                footer.open = true;
                footer
            });
            cx.new(|_| UsageHost {
                footer,
                bottom_inset: 0.,
            })
        })
        .unwrap()
    });
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let mut natural_height = None;
    for height in [900., 520.] {
        visual.simulate_resize(size(px(820.), px(height)));
        visual.run_until_parked();
        visual.refresh().unwrap();
        let scroll = visual.debug_bounds("usage-details-scroll").unwrap();
        let body = visual.debug_bounds("usage-details-body").unwrap();
        assert_eq!(scroll.size.height, body.size.height);
        assert!(scroll.size.height < px(300.));
        assert_eq!(
            *natural_height.get_or_insert(scroll.size.height),
            scroll.size.height
        );
        visual.simulate_event(ScrollWheelEvent {
            position: scroll.center(),
            delta: ScrollDelta::Pixels(point(px(0.), px(-2000.))),
            ..Default::default()
        });
        visual.refresh().unwrap();
        assert_eq!(visual.debug_bounds("usage-details-body").unwrap(), body);
    }
}
