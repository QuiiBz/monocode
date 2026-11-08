//! Context-window accounting, matching sessions/model/contextUsage.ts.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub used: u64,
    pub window: Option<u64>,
}
impl Usage {
    pub fn ratio(self) -> Option<f32> {
        self.window
            .filter(|w| *w > 0)
            .map(|w| (self.used as f64 / w as f64).min(1.) as f32)
    }
    pub fn merge(&mut self, used: Option<u64>, window: Option<u64>) {
        if let Some(used) = used {
            self.used = used;
        }
        if let Some(window) = window.filter(|w| *w > 0) {
            self.window = Some(window);
        }
    }
}

pub fn claude_used(usage: &Value) -> Option<u64> {
    let used = [
        "input_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
        "output_tokens",
    ]
    .iter()
    .map(|key| usage[key].as_u64().unwrap_or(0))
    .sum();
    (used > 0).then_some(used)
}

pub fn format_tokens(count: u64) -> String {
    let (value, suffix) = if count >= 1_000_000_000 {
        (count as f64 / 1_000_000_000., "B")
    } else if count >= 1_000_000 {
        (count as f64 / 1_000_000., "M")
    } else if count >= 1000 {
        (count as f64 / 1000., "K")
    } else {
        return count.to_string();
    };
    if value < 10. {
        format!(
            "{}{suffix}",
            format!("{value:.1}")
                .trim_end_matches('0')
                .trim_end_matches('.')
        )
    } else {
        format!("{value:.0}{suffix}")
    }
}

#[cfg(test)]
mod tests {
    use super::format_tokens;

    #[test]
    fn token_units_preserve_precision_at_billions() {
        for (count, expected) in [
            (999, "999"),
            (1_000, "1K"),
            (1_250_000, "1.2M"),
            (999_499_999, "999M"),
            (999_999_999, "1000M"),
            (1_000_000_000, "1B"),
            (1_000_000_001, "1B"),
            (1_250_000_000, "1.2B"),
            (9_999_999_999, "10B"),
            (10_000_000_000, "10B"),
            (80_600_000_000, "81B"),
            (u64::MAX, "18446744074B"),
        ] {
            assert_eq!(format_tokens(count), expected, "{count}");
        }
    }
}
