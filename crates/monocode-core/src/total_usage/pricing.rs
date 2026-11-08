// USD per million tokens, verified 2026-11-08 against the sources below.
pub const OPENAI_SOURCE: &str = "https://developers.openai.com/api/docs/pricing";
pub const CLAUDE_SOURCE: &str = "https://platform.claude.com/docs/en/about-claude/pricing";
pub const VERIFIED: &str = "2026-11-08";

pub(super) fn estimate(provider: &str, model: &str, parts: [u64; 4]) -> [Option<f64>; 4] {
    let model = model.strip_prefix(&format!("{provider}:")).unwrap_or(model);
    let rates = match (provider, model) {
        ("codex", "gpt-6.1-sol") => [2., 0.10, 10.],
        ("codex", "gpt-6.1-terra") => [1., 0.05, 5.],
        ("codex", "gpt-6-astra") => [10., 1., 50.],
        ("codex", "gpt-6-luna") => [0.10, 0.01, 0.50],
        ("claude", "claude-fable-5-1") => [10., 0.25, 50.],
        ("claude", "claude-opus-5-5") => [4., 0.20, 20.],
        ("claude", "claude-sonnet-5-5") => [2., 0.10, 10.],
        _ => return [None; 4],
    };
    [
        Some(parts[0] as f64 * rates[0] / 1_000_000.),
        Some(parts[1] as f64 * rates[1] / 1_000_000.),
        (parts[2] == 0).then_some(0.),
        Some(parts[3] as f64 * rates[2] / 1_000_000.),
    ]
}

pub(super) fn estimate_for_tier(
    provider: &str,
    model: &str,
    parts: [u64; 4],
    tier: Option<&str>,
) -> [Option<f64>; 4] {
    let multiplier = match tier {
        None | Some("default" | "standard") => 1.,
        Some("fast" | "priority")
            if provider == "codex"
                || (provider == "claude"
                    && model.strip_prefix("claude:").unwrap_or(model) == "claude-opus-5-5") =>
        {
            2.
        }
        Some("flex" | "batch") => 0.5,
        Some("ultrafast")
            if provider == "codex"
                && matches!(
                    model.strip_prefix("codex:").unwrap_or(model),
                    "gpt-6.1-sol" | "gpt-6-astra"
                ) =>
        {
            6.
        }
        _ => return [None; 4],
    };
    estimate(provider, model, parts).map(|amount| amount.map(|amount| amount * multiplier))
}
#[derive(Clone, Debug, Default)]
pub struct Cost {
    pub usd: f64,
    pub priced_tokens: u64,
    pub unpriced_tokens: u64,
    pub known: bool,
}
impl Cost {
    pub(super) fn add(&mut self, tokens: u64, amount: Option<f64>) {
        if let Some(amount) = amount {
            self.usd += amount;
            self.priced_tokens += tokens;
            self.known = true;
        } else {
            self.unpriced_tokens += tokens;
        }
    }
    pub fn amount_label(&self) -> String {
        if !self.known {
            return "Unpriced".into();
        }
        let amount = |value: f64| {
            if value > 0. && value < 0.01 {
                "<$0.01".into()
            } else {
                format!("${value:.2}")
            }
        };
        amount(self.usd)
    }
    pub fn label(&self) -> String {
        format!(
            "{}{}",
            self.amount_label(),
            if self.known && self.unpriced_tokens > 0 {
                " · partial"
            } else {
                ""
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_ids_and_million_token_units() {
        let amounts = estimate(
            "codex",
            "codex:gpt-6.1-sol",
            [100_000, 100_000, 0, 1_000_000],
        );
        assert_eq!(amounts, [Some(0.2), Some(0.01), Some(0.), Some(10.)]);
        assert_eq!(
            estimate("codex", "gpt-6.1-terra", [100_000, 100_000, 0, 1_000_000]),
            [Some(0.1), Some(0.005), Some(0.), Some(5.)]
        );
        assert_eq!(
            estimate(
                "claude",
                "claude:claude-opus-5-5",
                [1_000_000, 1_000_000, 0, 1_000_000]
            ),
            [Some(4.), Some(0.2), Some(0.), Some(20.)]
        );
        for (provider, model) in [
            ("codex", "gpt-6.1-sol-fast"),
            ("codex", "GPT-6.1-Sol"),
            ("mono", "gpt-6.1-sol"),
            ("claude", "claude-route1"),
            ("codex", "claude-opus-5-5"),
        ] {
            assert_eq!(estimate(provider, model, [1, 1, 1, 1]), [None; 4]);
        }
    }
    #[test]
    fn unsupported_tiers_and_cache_write_retention() {
        assert!(estimate("codex", "gpt-6.1-sol", [272_000, 0, 0, 1])[0].is_some());
        let parts = [272_000, 1, 0, 1];
        let amounts = estimate("codex", "gpt-6.1-sol", parts);
        assert!(amounts[0].is_some());
        let amounts = estimate("claude", "claude-sonnet-5-5", [100, 100, 500, 100]);
        assert_eq!(amounts[2], None);
        assert!(amounts[0].unwrap() > 0.);
        let mut cost = Cost::default();
        cost.add(10, Some(0.0001));
        cost.add(500, None);
        assert_eq!(cost.label(), "<$0.01 · partial");
        assert_eq!(Cost::default().label(), "Unpriced");
    }
    #[test]
    fn fast_rates_are_explicit_and_unknown_variants_stay_unpriced() {
        let parts = [100_000, 100_000, 0, 1_000];
        for model in ["gpt-6.1-sol", "gpt-6.1-terra", "gpt-6-astra", "gpt-6-luna"] {
            let base = estimate_for_tier("codex", model, parts, Some("default"));
            let fast = estimate_for_tier("codex", model, parts, Some("fast"));
            assert_eq!(fast, base.map(|amount| amount.map(|amount| amount * 2.)));
            assert_eq!(
                fast,
                estimate_for_tier("codex", model, parts, Some("priority"))
            );
        }
        assert_eq!(
            estimate_for_tier("claude", "claude-opus-5-5", parts, Some("fast")),
            [Some(0.8), Some(0.04), Some(0.), Some(0.04)]
        );
        assert_eq!(
            estimate_for_tier("claude", "claude-sonnet-5-5", parts, Some("fast")),
            [None; 4]
        );
        assert_eq!(
            estimate_for_tier("codex", "gpt-6.1-sol-fast", parts, None),
            [None; 4]
        );
        assert_eq!(
            estimate_for_tier("codex", "gpt-6.1-sol", parts, Some("unknown")),
            [None; 4]
        );
    }
    #[test]
    fn cache_reads_use_one_rate_for_large_turn_totals() {
        let parts = [0, 93_000_000, 0, 0];
        let standard = estimate_for_tier("codex", "gpt-6.1-sol", parts, Some("default"));
        let fast = estimate_for_tier("codex", "gpt-6.1-sol", parts, Some("fast"));
        assert!((standard[1].unwrap() - 9.3).abs() < 1e-10);
        assert!((fast[1].unwrap() - 18.6).abs() < 1e-10);
        let mut cost = Cost::default();
        cost.add(parts[1], standard[1]);
        assert_eq!(cost.label(), "$9.30");
    }
}
