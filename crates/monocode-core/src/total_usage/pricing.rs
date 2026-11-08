// USD per million tokens; cache write rates come from the Gateway catalog.
pub const OPENAI_SOURCE: &str = "https://developers.openai.com/api/docs/pricing";
pub const CLAUDE_SOURCE: &str = "https://platform.claude.com/docs/en/about-claude/pricing";
pub const GATEWAY_SOURCE: &str = "https://ai-gateway.vercel.sh/v1/models";
pub const VERIFIED: &str = "2026-11-09";

#[cfg(test)]
pub(super) fn estimate(provider: &str, model: &str, parts: [u64; 4]) -> [Option<f64>; 4] {
    estimate_for_tier(provider, model, parts, None)
}

pub(super) fn estimate_for_tier(
    provider: &str,
    model: &str,
    parts: [u64; 4],
    tier: Option<&str>,
) -> [Option<f64>; 4] {
    let model = model.strip_prefix(&format!("{provider}:")).unwrap_or(model);
    let (family, model) = match (provider, model) {
        ("codex", model) if model.starts_with("openai/") => ("codex", &model[7..]),
        ("codex" | "claude", model) if model.starts_with("anthropic/") => ("claude", &model[10..]),
        _ => (provider, model),
    };
    let (model, fast_alias) = model
        .strip_suffix("-fast")
        .map_or((model, false), |model| (model, true));
    let rates = match (family, model) {
        ("codex", "gpt-6.1-sol") => [2., 0.10, 2.50, 10.],
        ("codex", "gpt-6.1-terra") => [1., 0.05, 1.25, 5.],
        ("codex", "gpt-6-astra") => [10., 1., 12.50, 50.],
        ("codex", "gpt-6-luna") => [0.10, 0.01, 0.125, 0.50],
        ("claude", "claude-fable-5-1" | "claude-fable-5.1") => [10., 0.25, 12.50, 50.],
        ("claude", "claude-opus-5-5" | "claude-opus-5.5") => [4., 0.20, 5., 20.],
        ("claude", "claude-sonnet-5-5" | "claude-sonnet-5.5") => [2., 0.10, 2.50, 10.],
        _ => return [None; 4],
    };
    let supports_fast = family == "codex" || matches!(model, "claude-opus-5-5" | "claude-opus-5.5");
    if fast_alias && !supports_fast {
        return [None; 4];
    }
    let multiplier = match tier {
        None | Some("default" | "standard") => {
            if fast_alias {
                2.
            } else {
                1.
            }
        }
        Some("fast" | "priority") if supports_fast => 2.,
        Some("flex" | "batch") if !fast_alias => 0.5,
        Some("ultrafast")
            if !fast_alias
                && family == "codex"
                && matches!(model, "gpt-6.1-sol" | "gpt-6-astra") =>
        {
            6.
        }
        _ => return [None; 4],
    };
    std::array::from_fn(|index| {
        if index == 2 && provider == "claude" && parts[index] > 0 {
            return None;
        }
        Some(parts[index] as f64 * rates[index] * multiplier / 1_000_000.)
    })
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
            ("codex", "openai/gpt-6.1-sol-unknown"),
            ("codex", "GPT-6.1-Sol"),
            ("mono", "gpt-6.1-sol"),
            ("claude", "claude-route1"),
            ("codex", "claude-opus-5-5"),
            ("codex", "other/gpt-6.1-sol"),
        ] {
            assert_eq!(estimate(provider, model, [1, 1, 1, 1]), [None; 4]);
        }
    }
    #[test]
    fn published_cache_write_rates_and_partial_costs() {
        assert!(estimate("codex", "gpt-6.1-sol", [272_000, 0, 0, 1])[0].is_some());
        let parts = [272_000, 1, 0, 1];
        let amounts = estimate("codex", "gpt-6.1-sol", parts);
        assert!(amounts[0].is_some());
        let amounts = estimate("codex", "openai/gpt-6.1-sol", [100, 100, 500, 100]);
        assert_eq!(amounts[2], Some(0.00125));
        assert!(amounts[0].unwrap() > 0.);
        assert_eq!(
            estimate("claude", "claude-sonnet-5-5", [100, 100, 500, 100])[2],
            None
        );
        let mut cost = Cost::default();
        cost.add(10, Some(0.0001));
        cost.add(500, None);
        assert_eq!(cost.label(), "<$0.01 · partial");
        assert_eq!(Cost::default().label(), "Unpriced");
    }
    #[test]
    fn fast_rates_are_explicit_and_unknown_tiers_stay_unpriced() {
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
            estimate_for_tier("codex", "gpt-6.1-sol", parts, Some("priority"))
        );
        assert_eq!(
            estimate_for_tier("codex", "gpt-6.1-sol", parts, Some("unknown")),
            [None; 4]
        );
    }
    #[test]
    fn gateway_ids_and_fast_aliases_use_one_tier_multiplier() {
        let parts = [1_000_000; 4];
        for model in ["openai/gpt-6.1-sol-fast", "codex:openai/gpt-6.1-sol-fast"] {
            for tier in [None, Some("default"), Some("fast"), Some("priority")] {
                assert_eq!(
                    estimate_for_tier("codex", model, parts, tier),
                    [Some(4.), Some(0.2), Some(5.), Some(20.)]
                );
            }
            assert_eq!(
                estimate_for_tier("codex", model, parts, Some("flex")),
                [None; 4]
            );
        }
        assert_eq!(
            estimate("codex", "codex:openai/gpt-6-astra-fast", parts),
            [Some(20.), Some(2.), Some(25.), Some(100.)]
        );
        assert_eq!(
            estimate("codex", "codex:anthropic/claude-opus-5.5", parts),
            [Some(4.), Some(0.2), Some(5.), Some(20.)]
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
