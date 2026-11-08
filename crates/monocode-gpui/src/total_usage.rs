use crate::{
    popover::{Anchor, AnchorSlot, Side, popover},
    theme::*,
};
use gpui::{Context, Window, div, prelude::*, px, relative};
use monocode_core::{
    context::format_tokens,
    total_usage::{Bucket, Cost, Summary},
};

pub(crate) struct TotalUsage {
    pub database_path: std::path::PathBuf,
    anchor: AnchorSlot,
    open: bool,
    loading: bool,
    error: Option<String>,
    summaries: Vec<Summary>,
    timeframe: usize,
    grouping: usize,
    cost: bool,
    last_refresh: Option<std::time::Instant>,
    clock: Option<gpui::Task<()>>,
}
impl TotalUsage {
    pub fn new() -> Self {
        Self {
            database_path: monocode_core::store::monocode_database(),
            anchor: AnchorSlot::default(),
            open: false,
            loading: false,
            error: None,
            summaries: Vec::new(),
            timeframe: 1,
            grouping: 0,
            cost: false,
            last_refresh: None,
            clock: None,
        }
    }
    pub fn set_database(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        if self.database_path == path && self.clock.is_some() {
            return;
        }
        self.database_path = path;
        self.summaries.clear();
        self.last_refresh = None;
        self.refresh(cx);
        self.clock = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(60))
                    .await;
                if !cx.update(|cx| cx.active_window().is_some()) {
                    continue;
                }
                if this.update(cx, |this, cx| this.refresh(cx)).is_err() {
                    break;
                }
            }
        }));
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        self.error = None;
        cx.notify();
        let database_path = self.database_path.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    monocode_core::total_usage::load(
                        &database_path,
                        monocode_core::usage::now_ms().max(0) as u64,
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(summaries) => {
                        this.summaries = summaries;
                        this.last_refresh = Some(std::time::Instant::now());
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn selected_summary(&self) -> Option<&Summary> {
        self.summaries.get(self.timeframe)
    }
    fn footer_description(&self) -> String {
        let range = [
            "today",
            "7 days including today",
            "30 days including today",
            "90 days including today",
        ][self.timeframe];
        format!("Usage for {range}, from local midnight through now. Costs are API estimates.")
    }
    fn footer_label(&self) -> String {
        self.selected_summary()
            .map(|summary| {
                let cost = if summary.cost.known {
                    format!(" · ~{}", summary.cost.amount_label())
                } else {
                    String::new()
                };
                format!("{}{cost}", format_tokens(summary.tokens))
            })
            .unwrap_or_else(|| {
                if self.loading {
                    "Usage · loading…".into()
                } else {
                    "Usage · unavailable".into()
                }
            })
    }
    fn panel(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let mut header = div()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(16.))
            .border_b_1()
            .border_color(content(0.08));
        header = header.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child("Usage overview"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .text_color(content(0.5))
                        .child(
                            div()
                                .id("total-refresh")
                                .cursor_pointer()
                                .child(icon("refresh", 12.))
                                .aria_label(if self.loading {
                                    "Refreshing usage"
                                } else {
                                    "Refresh usage"
                                })
                                .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                        )
                        .child(
                            div()
                                .id("total-close")
                                .cursor_pointer()
                                .child(icon("x", 12.))
                                .aria_label("Close usage")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.open = false;
                                    cx.notify();
                                })),
                        ),
                ),
        );
        let mut periods = div()
            .flex()
            .gap(px(2.))
            .p(px(2.))
            .rounded_md()
            .bg(content(0.04));
        for (index, label) in ["Today", "7d", "30d", "90d"].into_iter().enumerate() {
            periods = periods.child(
                div()
                    .id(("total-time", index))
                    .debug_selector(move || format!("total-time-{index}"))
                    .px(px(9.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .bg(content(if self.timeframe == index { 0.10 } else { 0. }))
                    .text_color(content(if self.timeframe == index { 1. } else { 0.5 }))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.timeframe = index;
                        cx.notify();
                    })),
            );
        }
        header = header.child(periods);
        let mut body = div()
            .id("total-usage-scroll")
            .debug_selector(|| "total-usage-scroll".into())
            .min_h_0()
            .flex_1()
            .overflow_y_scroll();
        let mut sections = div().flex().flex_col().gap(px(16.)).p(px(16.));
        if let Some(error) = &self.error {
            sections = sections.child(
                div()
                    .text_color(content(0.65))
                    .child(format!("Cannot refresh usage: {error}")),
            );
        }
        if let Some(summary) = self.selected_summary() {
            sections = sections.child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        metric(
                            "Processed tokens",
                            format_tokens(summary.tokens),
                            format!("{} measured turns", summary.turns),
                            !self.cost,
                        )
                        .id("total-card-tokens")
                        .debug_selector(|| "total-card-tokens".into())
                        .role(gpui::Role::Button)
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.cost = false;
                            cx.notify();
                        })),
                    )
                    .child(
                        metric(
                            "API estimate",
                            summary.cost.label(),
                            if summary.cost.unpriced_tokens > 0 {
                                format!(
                                    "{} tokens unpriced",
                                    format_tokens(summary.cost.unpriced_tokens)
                                )
                            } else {
                                "USD · base API rates".into()
                            },
                            self.cost,
                        )
                        .id("total-card-cost")
                        .debug_selector(|| "total-card-cost".into())
                        .role(gpui::Role::Button)
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.cost = true;
                            cx.notify();
                        })),
                    ),
            );
            if summary.turns == 0 {
                sections = sections.child("No measured usage in this timeframe.");
            }
            let values: Vec<f64> = if self.cost {
                summary.cost_trend.iter().map(|cost| cost.usd).collect()
            } else {
                summary.trend.iter().map(|tokens| *tokens as f64).collect()
            };
            let maximum = values.iter().copied().fold(0., f64::max);
            let peak = if self.cost {
                Cost {
                    usd: maximum,
                    known: summary.cost.known,
                    ..Default::default()
                }
                .label()
            } else {
                format_tokens(maximum as u64)
            };
            let mut chart_section = div().flex_shrink_0().flex().flex_col().gap(px(8.)).child(
                div()
                    .flex()
                    .justify_between()
                    .child(format!(
                        "Daily {}",
                        if self.cost { "API estimate" } else { "tokens" }
                    ))
                    .child(
                        div()
                            .text_color(content(0.45))
                            .child(format!("Peak {peak}")),
                    ),
            );
            let mut chart = div()
                .id("total-trend")
                .relative()
                .h(px(108.))
                .flex_shrink_0()
                .w_full()
                .rounded_md()
                .bg(content(0.025));
            for offset in [0.25, 0.5, 0.75, 1.] {
                chart = chart.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .top(relative(offset))
                        .h(px(1.))
                        .bg(content(0.06)),
                );
            }
            let mut bars = div()
                .absolute()
                .inset_0()
                .px(px(4.))
                .flex()
                .items_end()
                .gap(px(3.));
            for (index, value) in values.iter().enumerate() {
                let bucket = summary.buckets.get(index).cloned().unwrap_or_default();
                let unpriced = self.cost
                    && summary
                        .cost_trend
                        .get(index)
                        .is_some_and(|cost| cost.usd == 0. && cost.unpriced_tokens > 0);
                let title = interval_label(summary, self.timeframe, index);
                let cost_mode = self.cost;
                let tooltip_bucket = bucket.clone();
                let mut stack = div()
                    .id(("total-bar", index))
                    .debug_selector(move || format!("total-bar-{index}"))
                    .w_full()
                    .flex()
                    .flex_col()
                    .justify_end()
                    .overflow_hidden()
                    .h(px(if unpriced {
                        2.
                    } else if maximum > 0. {
                        (*value / maximum * 102.) as f32
                    } else {
                        0.
                    }))
                    .rounded_t_sm()
                    .when(unpriced, |element| element.bg(content(0.18)));
                for category in (0..4).rev() {
                    let segment = if self.cost {
                        bucket.costs[category].usd
                    } else {
                        bucket.tokens[category] as f64
                    };
                    stack = stack.child(
                        div()
                            .id(("total-segment", index * 4 + category))
                            .debug_selector(move || format!("total-segment-{index}-{category}"))
                            .flex_shrink_0()
                            .w_full()
                            .h(relative(if *value > 0. {
                                (segment / *value) as f32
                            } else {
                                0.
                            }))
                            .bg(token_color(category)),
                    );
                }
                bars = bars.child(
                    div()
                        .id(("total-bucket", index))
                        .debug_selector(move || format!("total-bucket-{index}"))
                        .flex_1()
                        .h_full()
                        .flex()
                        .items_end()
                        .tooltip_show_delay(std::time::Duration::ZERO)
                        .tooltip(move |_, cx| {
                            cx.new(|_| BucketTooltip {
                                title: title.clone(),
                                bucket: tooltip_bucket.clone(),
                                cost: cost_mode,
                            })
                            .into()
                        })
                        .child(stack),
                );
            }
            chart_section = chart_section.child(chart.child(bars)).child(
                div()
                    .flex()
                    .justify_between()
                    .text_size(px(10.))
                    .text_color(content(0.4))
                    .child(summary.bucket_dates.first().cloned().unwrap_or_default())
                    .child("Now · local time"),
            );
            sections = sections.child(chart_section);
            let composition = div().flex_shrink_0().flex().flex_col().gap(px(9.)).child(
                div()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(if self.cost { "Cost mix" } else { "Token mix" }),
            );
            let mut strip = div()
                .id("total-composition")
                .debug_selector(|| "total-composition".into())
                .h(px(6.))
                .min_h(px(6.))
                .flex_shrink_0()
                .w_full()
                .flex()
                .rounded_full()
                .overflow_hidden()
                .bg(content(0.06));
            let mut legend = div().flex().flex_wrap().gap(px(12.));
            for (index, label) in ["Input", "Cache read", "Cache write", "Output"]
                .into_iter()
                .enumerate()
            {
                let cost = summary.cost_groups[2].get(label);
                let value = if self.cost {
                    cost.map_or(0., |cost| cost.usd)
                } else {
                    summary.composition[index] as f64
                };
                let total = if self.cost {
                    summary.cost.usd
                } else {
                    summary.tokens as f64
                };
                let value_label = if self.cost {
                    cost.map(Cost::label).unwrap_or_else(|| "Unpriced".into())
                } else {
                    format_tokens(summary.composition[index])
                };
                let color = token_color(index);
                strip = strip.child(
                    div()
                        .id(("total-mix-segment", index))
                        .debug_selector(move || format!("total-mix-segment-{index}"))
                        .h_full()
                        .w(relative(if total > 0. {
                            (value / total) as f32
                        } else {
                            0.
                        }))
                        .flex_shrink_0()
                        .bg(color),
                );
                legend = legend.child(
                    div()
                        .flex_1()
                        .min_w(px(75.))
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .text_size(px(10.))
                                .text_color(content(0.5))
                                .child(token_badge(index))
                                .child(label),
                        )
                        .child(div().text_size(px(13.)).child(value_label)),
                );
            }
            sections = sections.child(composition.child(strip).child(legend));
            let mut groups = div().flex().flex_wrap().gap(px(3.));
            for (index, label) in ["Agent", "Model", "Type", "Day"].into_iter().enumerate() {
                groups = groups.child(
                    div()
                        .id(("total-group", index))
                        .debug_selector(move || format!("total-group-{index}"))
                        .px(px(7.))
                        .py(px(3.))
                        .rounded(px(4.))
                        .cursor_pointer()
                        .bg(content(if self.grouping == index { 0.08 } else { 0. }))
                        .text_color(content(if self.grouping == index { 1. } else { 0.45 }))
                        .child(label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.grouping = index;
                            cx.notify();
                        })),
                );
            }
            let mut breakdown = div()
                .flex_shrink_0()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .flex_wrap()
                        .justify_between()
                        .gap(px(8.))
                        .child("Breakdown")
                        .child(groups),
                )
                .child(
                    div()
                        .flex()
                        .text_size(px(10.))
                        .text_color(content(0.4))
                        .py(px(4.))
                        .child(div().flex_1().child(
                            ["Agent / harness", "Model", "Token type", "Date (local)"]
                                [self.grouping],
                        ))
                        .child(div().w(px(70.)).text_right().child("Tokens"))
                        .child(div().w(px(105.)).text_right().child("API estimate")),
                );
            let mut rows: Vec<_> = summary.groups[self.grouping].iter().collect();
            rows.sort_by(|(left_label, left_tokens), (right_label, right_tokens)| {
                if self.grouping == 3 {
                    return right_label.cmp(left_label);
                }
                if self.cost {
                    let amount = |label: &String| {
                        summary.cost_groups[self.grouping]
                            .get(label)
                            .map(|cost| cost.usd)
                            .unwrap_or(0.)
                    };
                    amount(right_label).total_cmp(&amount(left_label))
                } else {
                    right_tokens.cmp(left_tokens)
                }
            });
            for (index, (label, tokens)) in rows.into_iter().enumerate() {
                let cost = summary.cost_groups[self.grouping]
                    .get(label)
                    .map(Cost::label)
                    .unwrap_or_else(|| "Unpriced".into());
                breakdown = breakdown.child(
                    div()
                        .id(("total-breakdown", index))
                        .debug_selector(move || format!("total-breakdown-{index}"))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .h(px(32.))
                        .border_t_1()
                        .border_color(content(0.06))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .when(self.grouping == 2, |element| {
                                    element.when_some(
                                        ["Input", "Cache read", "Cache write", "Output"]
                                            .iter()
                                            .position(|token_type| *token_type == label),
                                        |element, category| {
                                            element.child(token_badge(category).debug_selector(
                                                move || format!("total-type-badge-{category}"),
                                            ))
                                        },
                                    )
                                })
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(label.clone()),
                                ),
                        )
                        .child(
                            div()
                                .w(px(70.))
                                .text_right()
                                .text_color(content(if self.cost { 0.55 } else { 0.9 }))
                                .child(format_tokens(*tokens)),
                        )
                        .child(
                            div()
                                .w(px(105.))
                                .text_right()
                                .text_color(content(if self.cost { 0.9 } else { 0.55 }))
                                .child(cost),
                        ),
                );
            }
            sections = sections.child(breakdown);
        } else if self.loading {
            sections = sections.child("Loading usage…");
        }
        body = body.child(sections);
        div()
            .debug_selector(|| "total-panel".into())
            .min_h_0()
            .max_h(px(600.))
            .flex()
            .flex_col()
            .text_size(px(11.))
            .line_height(px(16.))
            .font_weight(gpui::FontWeight::NORMAL)
            .text_color(content(0.9))
            .child(header)
            .child(body)
            .into_any_element()
    }
}
impl Render for TotalUsage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.weak_entity();
        div()
            .id("total-usage")
            .relative()
            .child(self.anchor.probe())
            .child(
                div()
                    .id("total-usage-trigger")
                    .debug_selector(|| "total-usage-trigger".into())
                    .role(gpui::Role::Button)
                    .aria_label(self.footer_description())
                    .flex_shrink_0()
                    .px(px(7.))
                    .py(px(3.))
                    .rounded_md()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .cursor_pointer()
                    .child(icon("response-activity", 12.))
                    .hover(|style| style.bg(content(0.1)))
                    .child(self.footer_label())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open = !this.open;
                        if this.open
                            && this
                                .last_refresh
                                .is_none_or(|updated| updated.elapsed().as_secs() >= 60)
                        {
                            this.refresh(cx);
                        }
                        cx.notify();
                    })),
            )
            .when(self.open, |element| {
                element.child(
                    popover("total-usage-panel", Anchor::Slot(self.anchor.clone()))
                        .side(Side::Top)
                        .gap(7.)
                        .width(540.)
                        .max_height(620.)
                        .padding(0.)
                        .on_dismiss(move |_, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                this.open = false;
                                cx.notify();
                            });
                        })
                        .child(self.panel(cx)),
                )
            })
    }
}

fn interval_label(summary: &Summary, _timeframe: usize, index: usize) -> String {
    summary.bucket_dates.get(index).cloned().unwrap_or_default()
}

struct BucketTooltip {
    title: String,
    bucket: Bucket,
    cost: bool,
}
impl Render for BucketTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut tooltip = div()
            .debug_selector(|| "total-bucket-tooltip".into())
            .w(px(260.))
            .p(px(10.))
            .rounded_md()
            .bg(background_base())
            .border_1()
            .border_color(content(0.15))
            .text_size(px(11.))
            .line_height(px(16.))
            .text_color(content(0.9))
            .flex()
            .flex_col()
            .gap(px(7.))
            .child(div().text_color(content(0.55)).child(self.title.clone()));
        for (index, label) in ["Input", "Cache read", "Cache write", "Output"]
            .into_iter()
            .enumerate()
        {
            let value = if self.cost {
                self.bucket.costs[index].label()
            } else {
                format!("{} tokens", format_tokens(self.bucket.tokens[index]))
            };
            tooltip = tooltip.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(token_badge(index))
                    .child(div().flex_1().child(label))
                    .child(value),
            );
        }
        let total = if self.cost {
            let known = self.bucket.costs.iter().any(|cost| cost.known);
            Cost {
                usd: self.bucket.costs.iter().map(|cost| cost.usd).sum(),
                known,
                unpriced_tokens: self
                    .bucket
                    .costs
                    .iter()
                    .map(|cost| cost.unpriced_tokens)
                    .sum(),
                ..Default::default()
            }
            .label()
        } else {
            format!(
                "{} tokens",
                format_tokens(self.bucket.tokens.iter().sum::<u64>())
            )
        };
        tooltip.child(
            div()
                .flex()
                .justify_between()
                .pt(px(6.))
                .border_t_1()
                .border_color(content(0.1))
                .child(if self.cost {
                    "API estimate · USD"
                } else {
                    "Total"
                })
                .child(total),
        )
    }
}

fn metric(label: &str, value: String, detail: String, selected: bool) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .p(px(12.))
        .rounded_lg()
        .border_1()
        .border_color(content(if selected { 0.15 } else { 0.06 }))
        .bg(content(0.025))
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(
            div()
                .text_size(px(10.))
                .text_color(content(0.5))
                .child(label.to_owned()),
        )
        .child(
            div()
                .text_size(px(24.))
                .line_height(px(28.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(value),
        )
        .child(
            div()
                .text_size(px(10.))
                .text_color(content(0.4))
                .child(detail),
        )
}
fn token_badge(index: usize) -> gpui::Div {
    div()
        .size(px(6.))
        .flex_shrink_0()
        .rounded_full()
        .bg(token_color(index))
}

fn token_color(index: usize) -> gpui::Hsla {
    match index {
        0 => gpui::rgb(0x729de0).into(),
        1 => gpui::rgb(0x70b7aa).into(),
        2 => gpui::rgb(0xb099cf).into(),
        _ => gpui::rgb(0xdbad70).into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Entity, TestAppContext, VisualTestContext, size};
    struct PanelHost(Entity<TotalUsage>);
    impl Render for PanelHost {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(self.0.update(cx, |panel, cx| panel.panel(cx)))
        }
    }
    #[gpui::test]
    fn cost_switch_changes_chart_and_timeframe(cx: &mut TestAppContext) {
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                let panel = cx.new(|_| {
                    let mut panel = TotalUsage::new();
                    let mut summary = Summary::default();
                    summary.turns = 2;
                    summary.tokens = 200;
                    summary.composition = [40, 120, 0, 40];
                    summary.trend = vec![100, 100];
                    summary.buckets = vec![
                        Bucket {
                            tokens: [20, 60, 0, 20],
                            costs: [
                                Cost {
                                    usd: 0.5,
                                    known: true,
                                    ..Default::default()
                                },
                                Cost {
                                    usd: 0.25,
                                    known: true,
                                    ..Default::default()
                                },
                                Cost {
                                    known: true,
                                    ..Default::default()
                                },
                                Cost {
                                    usd: 0.25,
                                    known: true,
                                    ..Default::default()
                                },
                            ],
                        },
                        Bucket {
                            tokens: [20, 60, 0, 20],
                            costs: [
                                Cost {
                                    usd: 1.5,
                                    known: true,
                                    ..Default::default()
                                },
                                Cost {
                                    usd: 0.75,
                                    known: true,
                                    ..Default::default()
                                },
                                Cost {
                                    known: true,
                                    ..Default::default()
                                },
                                Cost {
                                    usd: 0.75,
                                    known: true,
                                    ..Default::default()
                                },
                            ],
                        },
                    ];
                    summary.cost = Cost {
                        usd: 4.,
                        known: true,
                        priced_tokens: 200,
                        ..Default::default()
                    };
                    summary.cost_trend = vec![
                        Cost {
                            usd: 1.,
                            known: true,
                            ..Default::default()
                        },
                        Cost {
                            usd: 3.,
                            known: true,
                            ..Default::default()
                        },
                    ];
                    for group in &mut summary.groups {
                        group.insert("Known".into(), 100);
                        group.insert("Unknown".into(), 100);
                    }
                    for group in &mut summary.cost_groups {
                        group.insert("Known".into(), summary.cost.clone());
                        group.insert("Unknown".into(), Cost::default());
                    }
                    summary.groups[2].clear();
                    summary.cost_groups[2].clear();
                    for (index, label) in ["Input", "Cache read", "Cache write", "Output"]
                        .into_iter()
                        .enumerate()
                    {
                        summary.groups[2].insert(label.into(), summary.composition[index]);
                        summary.cost_groups[2].insert(
                            label.into(),
                            Cost {
                                usd: [2., 1., 0., 1.][index],
                                known: true,
                                ..Default::default()
                            },
                        );
                    }
                    panel.summaries = vec![summary; 4];
                    panel
                });
                cx.new(|_| PanelHost(panel))
            })
            .unwrap()
        });
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.simulate_resize(size(px(640.), px(1100.)));
        visual.run_until_parked();
        visual.refresh().unwrap();
        assert!(
            visual
                .debug_bounds("total-composition")
                .unwrap()
                .size
                .height
                >= px(6.)
        );
        assert!(
            visual
                .debug_bounds("total-usage-scroll")
                .unwrap()
                .size
                .height
                <= px(600.)
        );
        window
            .update(visual, |host, _, cx| {
                assert_eq!(host.0.read(cx).footer_label(), "200 · ~$4.00")
            })
            .unwrap();
        let token_share = visual
            .debug_bounds("total-mix-segment-0")
            .unwrap()
            .size
            .width;
        let tokens_height = visual.debug_bounds("total-bar-0").unwrap().size.height;
        let input = visual.debug_bounds("total-segment-0-0").unwrap();
        let cache = visual.debug_bounds("total-segment-0-1").unwrap();
        let output = visual.debug_bounds("total-segment-0-3").unwrap();
        assert!(
            (input.size.height + cache.size.height + output.size.height - tokens_height).abs()
                < px(0.1)
        );
        assert!(cache.size.height > input.size.height * 2.9);
        assert!(output.bottom() <= cache.top() + px(0.1));
        assert!(cache.bottom() <= input.top() + px(0.1));

        let cost_button = visual.debug_bounds("total-card-cost").unwrap();
        visual.simulate_click(cost_button.center(), Default::default());
        visual.refresh().unwrap();
        let cost_height = visual.debug_bounds("total-bar-0").unwrap().size.height;
        assert!(cost_height < tokens_height / 2.);
        let cost_share = visual
            .debug_bounds("total-mix-segment-0")
            .unwrap()
            .size
            .width;
        assert!(cost_share > token_share * 2.4);
        assert!(visual.debug_bounds("total-unit-1").is_none());
        let interval = visual.debug_bounds("total-bucket-0").unwrap();
        visual.simulate_mouse_move(interval.center(), None, Default::default());
        visual.run_until_parked();
        visual.refresh().unwrap();
        assert!(visual.debug_bounds("total-bucket-tooltip").is_some());
        visual.simulate_mouse_move(cost_button.center(), None, Default::default());
        visual.run_until_parked();
        visual
            .executor()
            .advance_clock(std::time::Duration::from_secs(1));
        visual.run_until_parked();
        visual.refresh().unwrap();
        assert!(visual.debug_bounds("total-bucket-tooltip").is_none());
        visual.simulate_mouse_move(gpui::point(px(0.), px(0.)), None, Default::default());
        visual.run_until_parked();
        visual.refresh().unwrap();

        for index in 0..4 {
            let button = visual
                .debug_bounds(
                    [
                        "total-time-0",
                        "total-time-1",
                        "total-time-2",
                        "total-time-3",
                    ][index],
                )
                .unwrap();
            visual.simulate_click(button.center(), Default::default());
            visual.refresh().unwrap();
            window
                .update(visual, |host, _, cx| {
                    assert_eq!(host.0.read(cx).timeframe, index);
                    assert!(host.0.read(cx).cost);
                })
                .unwrap();
        }
        for index in 0..4 {
            let button = visual
                .debug_bounds(
                    [
                        "total-group-0",
                        "total-group-1",
                        "total-group-2",
                        "total-group-3",
                    ][index],
                )
                .unwrap();
            visual.simulate_click(button.center(), Default::default());
            visual.refresh().unwrap();
            window
                .update(visual, |host, _, cx| {
                    assert_eq!(host.0.read(cx).grouping, index)
                })
                .unwrap();
            assert!(visual.debug_bounds("total-breakdown-0").is_some());
            if index == 2 {
                for selector in [
                    "total-type-badge-0",
                    "total-type-badge-1",
                    "total-type-badge-2",
                    "total-type-badge-3",
                ] {
                    let badge = visual.debug_bounds(selector).unwrap();
                    assert_eq!(badge.size.width, px(6.));
                    assert_eq!(badge.size.height, px(6.));
                }
            } else {
                assert!(visual.debug_bounds("total-type-badge-0").is_none());
            }
        }
        let tokens_button = visual.debug_bounds("total-card-tokens").unwrap();
        visual.simulate_click(tokens_button.center(), Default::default());
        visual.refresh().unwrap();
        assert_eq!(
            visual.debug_bounds("total-bar-0").unwrap().size.height,
            tokens_height
        );
        assert_eq!(
            visual
                .debug_bounds("total-mix-segment-0")
                .unwrap()
                .size
                .width,
            token_share
        );
    }
    #[gpui::test]
    fn footer_shows_cached_stats_and_panel_fits_small_windows(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|_| {
                    let mut panel = TotalUsage::new();
                    let mut summary = Summary::default();
                    summary.tokens = 72_000_000;
                    summary.turns = 92;
                    summary.trend = vec![1; 24];
                    summary.cost_trend = vec![Cost::default(); 24];
                    summary.cost = Cost {
                        usd: 1.2,
                        known: true,
                        unpriced_tokens: 10,
                        ..Default::default()
                    };
                    panel.summaries = vec![summary; 4];
                    panel.last_refresh = Some(std::time::Instant::now());
                    panel
                })
            })
            .unwrap()
        });
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.simulate_resize(size(px(360.), px(440.)));
        visual.run_until_parked();
        visual.refresh().unwrap();
        window
            .update(visual, |panel, _, _| {
                assert_eq!(panel.footer_label(), "72M · ~$1.20")
            })
            .unwrap();
        let trigger = visual.debug_bounds("total-usage-trigger").unwrap();
        visual.simulate_click(trigger.center(), Default::default());
        visual.run_until_parked();
        visual.refresh().unwrap();
        window
            .update(visual, |panel, _, _| {
                assert!(panel.open);
                assert!(
                    !panel.loading,
                    "opening a cached panel must not reload transcripts"
                );
            })
            .unwrap();
        let bounds = visual.debug_bounds("total-panel").unwrap();
        assert!(bounds.size.width <= px(360.), "{bounds:?}");
        assert!(
            bounds.top() >= px(0.) && bounds.bottom() <= px(440.),
            "{bounds:?}"
        );
        let scroll = visual.debug_bounds("total-usage-scroll").unwrap();
        assert!(scroll.size.height > px(80.), "{scroll:?}");
    }
    #[gpui::test]
    fn selected_range_is_shared_through_refresh_and_reopening(cx: &mut TestAppContext) {
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|_| {
                    let mut panel = TotalUsage::new();
                    panel.summaries = (0..4)
                        .map(|index| Summary {
                            tokens: (index + 1) * 1000,
                            trend: vec![0; 24],
                            cost: Cost {
                                usd: (index + 1) as f64,
                                known: true,
                                unpriced_tokens: 10,
                                ..Default::default()
                            },
                            ..Default::default()
                        })
                        .collect();
                    panel.last_refresh = Some(std::time::Instant::now());
                    panel
                })
            })
            .unwrap()
        });
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.simulate_resize(size(px(800.), px(800.)));
        for (index, range) in [
            "today",
            "7 days including today",
            "30 days including today",
            "90 days including today",
        ]
        .into_iter()
        .enumerate()
        {
            visual.refresh().unwrap();
            let trigger = visual.debug_bounds("total-usage-trigger").unwrap();
            visual.simulate_click(trigger.center(), Default::default());
            visual.refresh().unwrap();
            let period = visual
                .debug_bounds(
                    [
                        "total-time-0",
                        "total-time-1",
                        "total-time-2",
                        "total-time-3",
                    ][index],
                )
                .unwrap();
            visual.simulate_click(period.center(), Default::default());
            visual.refresh().unwrap();
            window
                .update(visual, |panel, _, cx| {
                    assert_eq!(panel.timeframe, index);
                    assert_eq!(
                        panel.selected_summary().unwrap().tokens,
                        (index as u64 + 1) * 1000
                    );
                    let expected = format!("{}K · ~${:.2}", index + 1, (index + 1) as f64);
                    assert_eq!(panel.footer_label(), expected);
                    assert!(panel.footer_description().contains(range));
                    assert!(!panel.loading);
                    panel.loading = true;
                    assert_eq!(panel.footer_label(), expected);
                    panel.loading = false;
                    panel.summaries[index].tokens += 1000;
                    panel.summaries[index].cost.usd += 1.;
                    cx.notify();
                })
                .unwrap();
            visual.refresh().unwrap();
            let trigger = visual.debug_bounds("total-usage-trigger").unwrap();
            visual.simulate_click(trigger.center(), Default::default());
            visual.refresh().unwrap();
            visual.simulate_click(trigger.center(), Default::default());
            visual.refresh().unwrap();
            window
                .update(visual, |panel, _, _| {
                    assert!(panel.open);
                    assert_eq!(panel.timeframe, index);
                    assert_eq!(
                        panel.footer_label(),
                        format!("{}K · ~${:.2}", index + 2, (index + 2) as f64)
                    );
                    assert!(!panel.loading);
                })
                .unwrap();
            visual.simulate_click(trigger.center(), Default::default());
        }
        let database_path = std::env::temp_dir().join(format!(
            "monocode-total-usage-{}-{}.db",
            std::process::id(),
            monocode_core::usage::now_ms()
        ));
        let connection = rusqlite::Connection::open(&database_path).unwrap();
        connection
            .execute_batch("CREATE TABLE sessions(id TEXT,harness TEXT,blocks_json TEXT);")
            .unwrap();
        let receipt = serde_json::json!([{
            "role": "user", "id": "refresh", "startedAt": monocode_core::usage::now_ms(),
            "turnModel": {"harness": "codex", "id": "codex:gpt-6.1-sol"},
            "turnMetrics": {"inputTokens": 200000, "cacheReadTokens": 100000,
                "cacheWriteTokens": 0, "outputTokens": 1000}
        }]);
        connection
            .execute(
                "INSERT INTO sessions VALUES('s','codex',?1)",
                [receipt.to_string()],
            )
            .unwrap();
        drop(connection);
        window
            .update(visual, |panel, _, cx| {
                panel.database_path = database_path.clone();
                panel.refresh(cx);
                assert!(panel.loading);
                assert_eq!(panel.timeframe, 3);
            })
            .unwrap();
        visual.run_until_parked();
        window
            .update(visual, |panel, _, _| {
                assert!(!panel.loading);
                assert!(panel.error.is_none(), "{:?}", panel.error);
                assert_eq!(panel.timeframe, 3);
                assert_eq!(panel.selected_summary().unwrap().tokens, 201000);
                assert_eq!(panel.footer_label(), "201K · ~$0.22");
            })
            .unwrap();
        std::fs::remove_file(database_path).unwrap();
    }
    #[test]
    fn bucket_labels_show_only_dates() {
        let summary = Summary {
            bucket_dates: vec!["2026-11-11".into(), "2026-11-12".into()],
            ..Default::default()
        };
        assert_eq!(interval_label(&summary, 1, 0), "2026-11-11");
        assert_eq!(interval_label(&summary, 1, 1), "2026-11-12");
    }
}
