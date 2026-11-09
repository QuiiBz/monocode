use super::*;
use monocode_core::{
    context::format_tokens,
    total_usage::{Bucket, Cost},
};

#[derive(Default)]
pub(super) struct State {
    usage: Option<Bucket>,
    error: Option<String>,
    refresh_task: Option<gpui::Task<()>>,
}

impl Conversation {
    pub(super) fn hover_context(&mut self, hovered: bool, cx: &mut Context<Self>) {
        if self.context_hovered == hovered {
            return;
        }
        self.context_hovered = hovered;
        self.session_usage.refresh_task = None;
        if hovered {
            let store = self.store.clone();
            let session_id = self.summary.id;
            self.session_usage.refresh_task = Some(cx.spawn(async move |this, cx| {
                loop {
                    let store = store.clone();
                    let Ok((snapshot, provider, live_turn)) = this.update(cx, |this, cx| {
                        let view = this.transcript.read(cx);
                        let live_turn = this
                            .busy
                            .then(|| {
                                view.active_turn()
                                    .and_then(|turn| view.turns.get(&turn))
                                    .cloned()
                            })
                            .flatten();
                        (
                            this.remote_state.snapshot.clone(),
                            this.summary.provider.clone(),
                            live_turn,
                        )
                    }) else {
                        break;
                    };
                    let result = cx
                        .background_executor()
                        .spawn(async move {
                            if let Some(snapshot) = snapshot {
                                let blocks = snapshot["session"]["blocks"]
                                    .as_array()
                                    .ok_or("Host transcript is unavailable")?;
                                Bucket::from_blocks(blocks, &provider, live_turn.as_ref())
                            } else {
                                locked(&store)?.session_usage(session_id, live_turn.as_ref())
                            }
                        })
                        .await;
                    if this
                        .update(cx, |this, cx| {
                            match result {
                                Ok(usage) => {
                                    this.session_usage.usage = Some(usage);
                                    this.session_usage.error = None;
                                }
                                Err(error) => this.session_usage.error = Some(error),
                            }
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(1))
                        .await;
                }
            }));
        }
        cx.notify();
    }
}

pub(super) fn panel(context: Option<monocode_core::context::Usage>, state: &State) -> gpui::Div {
    let ratio = context.and_then(|usage| usage.ratio());
    let headline = ratio
        .map(|ratio| format!("{:.0}% context used", ratio * 100.))
        .unwrap_or_else(|| "No context used yet".into());
    let mut panel = div()
        .debug_selector(|| "context-usage-panel".into())
        .flex()
        .flex_col()
        .gap(px(3.))
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(content(1.))
        .child(div().font_weight(gpui::FontWeight::MEDIUM).child(headline));
    if let Some(context) = context {
        let detail = context.window.map_or_else(
            || format!("{} tokens", format_tokens(context.used)),
            |window| {
                format!(
                    "{} / {} tokens",
                    format_tokens(context.used),
                    format_tokens(window)
                )
            },
        );
        panel = panel.child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(11.))
                        .text_color(content(0.6))
                        .child(detail),
                )
                .when_some(ratio, |row, ratio| {
                    row.child(
                        div()
                            .flex_1()
                            .min_w(px(48.))
                            .h(px(6.))
                            .rounded_full()
                            .overflow_hidden()
                            .bg(content(0.1))
                            .child(div().h_full().w(gpui::relative(ratio)).rounded_full().bg(
                                if ratio >= 0.9 {
                                    gpui::rgb(0xf87171).into()
                                } else if ratio >= 0.75 {
                                    gpui::rgb(0xfbbf24).into()
                                } else {
                                    accent_color()
                                },
                            )),
                    )
                }),
        );
    }
    let mut session = div()
        .mt(px(7.))
        .pt(px(10.))
        .border_t_1()
        .border_color(content(0.08))
        .flex()
        .flex_col()
        .gap(px(9.))
        .child(
            div()
                .font_weight(gpui::FontWeight::MEDIUM)
                .child("Session usage"),
        );
    if let Some(error) = &state.error {
        return panel.child(
            session.child(
                div()
                    .text_size(px(11.))
                    .text_color(content(0.5))
                    .child(format!("Cannot load usage: {error}")),
            ),
        );
    }
    let Some(usage) = &state.usage else {
        return panel.child(
            session.child(
                div()
                    .text_size(px(11.))
                    .text_color(content(0.5))
                    .child("Loading usage…"),
            ),
        );
    };
    let total_tokens = usage.total_tokens();
    let total_cost = usage.total_cost();
    let cost_label = |cost: &Cost| {
        if !cost.known && cost.unpriced_tokens == 0 {
            "$0.00".into()
        } else {
            cost.label()
        }
    };
    session = session.child(
        div()
            .flex()
            .gap(px(12.))
            .child(
                metric("Processed tokens", format_tokens(total_tokens))
                    .debug_selector(|| "session-total-tokens".into()),
            )
            .child(
                metric("API estimate", cost_label(&total_cost))
                    .debug_selector(|| "session-total-cost".into()),
            ),
    );
    let mut strip = div()
        .h(px(6.))
        .flex_shrink_0()
        .flex()
        .rounded_full()
        .overflow_hidden()
        .bg(content(0.06));
    for (index, tokens) in usage.tokens.iter().enumerate() {
        strip = strip.child(
            div()
                .h_full()
                .flex_shrink_0()
                .w(gpui::relative(if total_tokens > 0 {
                    *tokens as f32 / total_tokens as f32
                } else {
                    0.
                }))
                .bg(crate::total_usage::token_color(index)),
        );
    }
    session = session.child(strip);
    let columns = |label: gpui::AnyElement, tokens: String, cost: String| {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().flex_1().child(label))
            .child(div().w(px(60.)).text_right().child(tokens))
            .child(div().w(px(96.)).text_right().child(cost))
    };
    let mut breakdown = div()
        .flex()
        .flex_col()
        .gap(px(7.))
        .text_size(px(11.))
        .child(
            columns(
                div().child("Type").into_any_element(),
                "Tokens".into(),
                "API estimate".into(),
            )
            .text_color(content(0.45)),
        );
    for (index, label) in ["Input", "Cache read", "Cache write", "Output"]
        .into_iter()
        .enumerate()
    {
        breakdown = breakdown.child(
            columns(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(crate::total_usage::token_badge(index))
                    .child(label)
                    .into_any_element(),
                format_tokens(usage.tokens[index]),
                cost_label(&usage.costs[index]),
            )
            .debug_selector(move || format!("session-usage-type-{index}")),
        );
    }
    panel.child(
        session
            .child(breakdown)
            .when(total_cost.unpriced_tokens > 0, |session| {
                session.child(
                    div()
                        .text_size(px(10.))
                        .text_color(content(0.45))
                        .child(format!(
                            "{} tokens unpriced",
                            format_tokens(total_cost.unpriced_tokens)
                        )),
                )
            }),
    )
}

fn metric(label: &'static str, value: String) -> gpui::Div {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(
            div()
                .text_size(px(10.))
                .text_color(content(0.5))
                .child(label),
        )
        .child(
            div()
                .text_size(px(16.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(value),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, VisualTestContext, size};
    use monocode_core::{
        agent::{Settings, TurnInfo, TurnMetrics},
        store::Store,
    };
    use std::sync::Mutex;

    struct PanelHost(State);
    impl Render for PanelHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().w(px(340.)).p(px(12.)).child(panel(
                Some(monocode_core::context::Usage {
                    used: 126000,
                    window: Some(258000),
                }),
                &self.0,
            ))
        }
    }

    #[gpui::test]
    fn session_breakdown_fits_the_context_popup(cx: &mut TestAppContext) {
        let window =
            cx.update(|cx| {
                cx.open_window(Default::default(), |_, cx| {
                    cx.new(|_| {
                        PanelHost(State {
                usage: Some(Bucket::from_blocks(&[serde_json::json!({
                    "role":"user", "id":"turn",
                    "turnModel":{"harness":"codex","id":"codex:gpt-6.1-sol"},
                    "turnMetrics":{"inputTokens":200000,"cacheReadTokens":100000,
                        "cacheWriteTokens":50000,"outputTokens":1000}
                })], "codex", None).unwrap()),
                ..Default::default()
            })
                    })
                })
                .unwrap()
            });
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.simulate_resize(size(px(400.), px(500.)));
        visual.run_until_parked();
        visual.refresh().unwrap();
        let panel = visual.debug_bounds("context-usage-panel").unwrap();
        let tokens = visual.debug_bounds("session-total-tokens").unwrap();
        let cost = visual.debug_bounds("session-total-cost").unwrap();
        assert!(tokens.right() <= cost.left());
        assert!(cost.right() <= panel.right());
        for selector in [
            "session-usage-type-0",
            "session-usage-type-1",
            "session-usage-type-2",
            "session-usage-type-3",
        ] {
            let row = visual.debug_bounds(selector).unwrap();
            assert!(row.top() > tokens.bottom());
            assert!(row.bottom() <= panel.bottom());
            assert!(row.right() <= panel.right());
        }
    }

    #[gpui::test]
    fn hover_refreshes_session_receipts_and_stops_when_closed(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let session = store.create("codex", "/project").unwrap();
        let session_id = session.id;
        let store = Arc::new(Mutex::new(store));
        let mut settings = Settings::default();
        settings.turns.insert(
            1,
            TurnInfo {
                provider: "codex".into(),
                model_id: Some("codex:gpt-6.1-sol".into()),
                metrics: Some(TurnMetrics {
                    input_tokens: 100,
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        locked(&store)
            .unwrap()
            .save_agent_settings(session_id, &settings)
            .unwrap();
        let chat = cx.new(|cx| Conversation::new(session, store.clone(), cx));
        chat.update(cx, |chat, cx| chat.hover_context(true, cx));
        cx.run_until_parked();
        chat.read_with(cx, |chat, _| {
            assert_eq!(
                chat.session_usage.usage.as_ref().unwrap().total_tokens(),
                100
            )
        });
        settings
            .turns
            .get_mut(&1)
            .unwrap()
            .metrics
            .as_mut()
            .unwrap()
            .input_tokens = 250;
        locked(&store)
            .unwrap()
            .save_agent_settings(session_id, &settings)
            .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        chat.read_with(cx, |chat, _| {
            assert_eq!(
                chat.session_usage.usage.as_ref().unwrap().total_tokens(),
                250
            )
        });
        chat.update(cx, |chat, cx| chat.hover_context(false, cx));
        chat.read_with(cx, |chat, _| {
            assert!(chat.session_usage.refresh_task.is_none())
        });
    }
}
