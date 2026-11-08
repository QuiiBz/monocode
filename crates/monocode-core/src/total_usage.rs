mod pricing;
use crate::agent::TurnMetrics;
pub use pricing::{CLAUDE_SOURCE, Cost, OPENAI_SOURCE, VERIFIED};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
};

pub const DAY: u64 = 86_400_000;
#[derive(Clone, Debug, Default)]
pub struct Bucket {
    pub tokens: [u64; 4],
    pub costs: [Cost; 4],
}
#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub end_ms: u64,
    pub buckets: Vec<Bucket>,
    pub tokens: u64,
    pub turns: usize,
    pub skipped: usize,
    pub composition: [u64; 4],
    pub groups: [BTreeMap<String, u64>; 4],
    pub trend: Vec<u64>,
    pub cost: Cost,
    pub cost_groups: [BTreeMap<String, Cost>; 4],
    pub cost_trend: Vec<Cost>,
}

pub fn load(path: &Path, now: u64) -> Result<Vec<Summary>, String> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| error.to_string())?;
    let indexed: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='mono_transcripts')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let sources = if indexed {
        "SELECT sessions.id AS session,sessions.harness,json_each.value AS block FROM sessions,json_each(sessions.blocks_json) WHERE NOT EXISTS(SELECT 1 FROM mono_transcripts WHERE session_id=sessions.id) UNION ALL SELECT sessions.id,sessions.harness,mono_blocks.block_json FROM sessions JOIN mono_transcripts ON mono_transcripts.session_id=sessions.id JOIN mono_blocks ON mono_blocks.session_id=sessions.id"
    } else {
        "SELECT sessions.id AS session,sessions.harness,json_each.value AS block FROM sessions,json_each(sessions.blocks_json)"
    };
    let query = format!(
        "SELECT session,harness,json_object('id',json_extract(block,'$.id'),'providerTurnId',json_extract(block,'$.providerTurnId'),'startedAt',json_extract(block,'$.startedAt'),'sentAt',json_extract(block,'$.sentAt'),'turnModel',json_extract(block,'$.turnModel'),'turnMetrics',json_extract(block,'$.turnMetrics')) FROM ({sources}) WHERE json_extract(block,'$.role')='user' AND json_type(block,'$.turnMetrics')='object'"
    );
    let mut statement = connection
        .prepare(&query)
        .map_err(|error| error.to_string())?;
    let records = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    let mut summaries = vec![Summary::default(); 4];
    for (summary, days) in summaries.iter_mut().zip([1, 7, 30, 90]) {
        summary.end_ms = now;
        summary.trend = vec![0; if days == 1 { 24 } else { days }];
        summary.buckets = vec![Bucket::default(); summary.trend.len()];
        summary.cost_trend = vec![Cost::default(); summary.trend.len()];
    }
    let mut seen = HashSet::new();
    for record in records {
        let (session, harness, raw) = record.map_err(|error| error.to_string())?;
        let block: Value = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
        {
            let identity = block["providerTurnId"]
                .as_str()
                .or_else(|| block["id"].as_str());
            let Some(identity) = identity else {
                continue;
            };
            if !seen.insert((session.clone(), identity.to_owned())) {
                continue;
            }
            let timestamp = block["startedAt"].as_u64().filter(|started| {
                block["sentAt"]
                    .as_u64()
                    .is_none_or(|sent| started.saturating_add(1000) >= sent)
            });
            let metrics: TurnMetrics = serde_json::from_value(block["turnMetrics"].clone())
                .map_err(|error| error.to_string())?;
            let provider = block["turnModel"]["harness"].as_str().unwrap_or(&harness);
            let model = block["turnModel"]["id"]
                .as_str()
                .or_else(|| block["turnModel"]["name"].as_str())
                .unwrap_or("Unknown model");
            for (summary, days) in summaries.iter_mut().zip([1, 7, 30, 90]) {
                let Some(timestamp) = timestamp else {
                    summary.skipped += 1;
                    continue;
                };
                if timestamp > now || timestamp < now.saturating_sub(days * DAY) {
                    continue;
                }
                let complete = block["turnModel"]["id"].as_str().is_some()
                    && [
                        "inputTokens",
                        "outputTokens",
                        "cacheReadTokens",
                        "cacheWriteTokens",
                    ]
                    .iter()
                    .all(|key| block["turnMetrics"][key].as_u64().is_some());
                add(
                    summary,
                    metrics,
                    provider,
                    model,
                    timestamp,
                    now,
                    days,
                    complete,
                    block["turnModel"]["serviceTier"].as_str(),
                );
            }
        }
    }
    Ok(summaries)
}
fn add(
    summary: &mut Summary,
    metrics: TurnMetrics,
    provider: &str,
    model: &str,
    timestamp: u64,
    now: u64,
    days: u64,
    complete: bool,
    tier: Option<&str>,
) {
    let input = if provider == "codex" {
        metrics
            .input_tokens
            .saturating_sub(metrics.cache_read_tokens)
    } else {
        metrics.input_tokens
    };
    let parts = [
        input,
        metrics.cache_read_tokens,
        metrics.cache_write_tokens,
        metrics.output_tokens,
    ];
    let tokens = parts.iter().sum::<u64>();
    summary.tokens += tokens;
    summary.turns += 1;
    for (total, value) in summary.composition.iter_mut().zip(parts) {
        *total += value;
    }
    let day = chrono::DateTime::from_timestamp_millis(timestamp as i64)
        .map(|time| time.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "Unknown date".into());
    let model_label = match tier {
        Some("fast" | "priority") => format!("{model} · Fast"),
        Some("default" | "standard") | None => model.to_owned(),
        Some(tier) => format!("{model} · {tier}"),
    };
    for (index, label) in [
        (0, provider.to_owned()),
        (1, model_label.clone()),
        (3, day.clone()),
    ] {
        *summary.groups[index].entry(label).or_default() += tokens;
    }
    for (label, value) in ["Input", "Cache read", "Cache write", "Output"]
        .into_iter()
        .zip(parts)
    {
        *summary.groups[2].entry(label.into()).or_default() += value;
    }
    let bucket_width = days * DAY / summary.trend.len() as u64;
    let bucket = ((timestamp.saturating_sub(now.saturating_sub(days * DAY))) / bucket_width)
        .min(summary.trend.len() as u64 - 1) as usize;
    summary.trend[bucket] += tokens;
    summary
        .buckets
        .resize_with(summary.trend.len(), Bucket::default);
    summary
        .cost_trend
        .resize(summary.trend.len(), Cost::default());
    let amounts =
        if complete && (provider != "codex" || metrics.cache_read_tokens <= metrics.input_tokens) {
            pricing::estimate_for_tier(provider, model, parts, tier)
        } else {
            [None; 4]
        };
    for (index, (count, amount)) in parts.into_iter().zip(amounts).enumerate() {
        summary.buckets[bucket].tokens[index] += count;
        summary.buckets[bucket].costs[index].add(count, amount);
        summary.cost.add(count, amount);
        summary.cost_trend[bucket].add(count, amount);
        for grouping in [0, 1, 3] {
            let label = match grouping {
                0 => provider.to_owned(),
                1 => model_label.clone(),
                _ => day.clone(),
            };
            summary.cost_groups[grouping]
                .entry(label)
                .or_default()
                .add(count, amount);
        }
        summary.cost_groups[2]
            .entry(["Input", "Cache read", "Cache write", "Output"][index].into())
            .or_default()
            .add(count, amount);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_semantics_and_groups() {
        let metrics = TurnMetrics {
            input_tokens: 100,
            cache_read_tokens: 80,
            output_tokens: 10,
            ..Default::default()
        };
        let mut summary = Summary {
            trend: vec![0; 7],
            ..Default::default()
        };
        add(
            &mut summary,
            metrics,
            "codex",
            "Unknown model",
            DAY,
            DAY * 7,
            7,
            true,
            None,
        );
        assert_eq!(summary.tokens, 110);
        assert_eq!(summary.composition, [20, 80, 0, 10]);
        assert_eq!(summary.groups[0]["codex"], 110);
        assert_eq!(summary.groups[1]["Unknown model"], 110);
        add(
            &mut summary,
            metrics,
            "claude",
            "Unknown model",
            DAY,
            DAY * 7,
            7,
            true,
            None,
        );
        assert_eq!(summary.tokens, 300);
    }
    #[test]
    fn receipts_timeframes_and_duplicates() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let connection = Connection::open(file.path()).unwrap();
        connection
            .execute_batch("CREATE TABLE sessions(id TEXT,harness TEXT,blocks_json TEXT);")
            .unwrap();
        let receipt = serde_json::json!({"role":"user","id":"a","startedAt":DAY*8,"turnMetrics":{"inputTokens":42}});
        let raw = serde_json::json!([receipt,receipt,{"role":"user","id":"b","startedAt":DAY,"turnMetrics":{"inputTokens":10}}]).to_string();
        connection
            .execute("INSERT INTO sessions VALUES('s','mono',?1)", [raw])
            .unwrap();
        let summaries = load(file.path(), DAY * 9).unwrap();
        assert_eq!(
            summaries
                .iter()
                .map(|summary| summary.tokens)
                .collect::<Vec<_>>(),
            vec![42, 42, 52, 52]
        );
        assert_eq!(summaries[0].groups[0]["mono"], 42);
    }
    #[test]
    fn indexed_history_replaces_legacy_and_undated_receipts_are_excluded() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let connection = Connection::open(file.path()).unwrap();
        connection.execute_batch("CREATE TABLE sessions(id TEXT,harness TEXT,blocks_json TEXT); CREATE TABLE mono_transcripts(session_id TEXT); CREATE TABLE mono_blocks(session_id TEXT,seq INTEGER,block_json TEXT);").unwrap();
        connection.execute("INSERT INTO sessions VALUES('s','mono',?1)", [serde_json::json!([{"role":"user","id":"old","startedAt":DAY,"turnMetrics":{"inputTokens":999}}]).to_string()]).unwrap();
        connection
            .execute("INSERT INTO mono_transcripts VALUES('s')", [])
            .unwrap();
        for (index, block) in [serde_json::json!({"role":"user","id":"new","startedAt":DAY,"turnMetrics":{"inputTokens":12}}),serde_json::json!({"role":"user","id":"undated","turnMetrics":{"inputTokens":50}})].into_iter().enumerate() {
            connection.execute("INSERT INTO mono_blocks VALUES('s',?1,?2)", rusqlite::params![index as i64,block.to_string()]).unwrap();
        }
        let summaries = load(file.path(), DAY * 2).unwrap();
        assert_eq!(summaries[0].tokens, 12);
        assert_eq!(summaries[0].skipped, 1);
        assert_eq!(summaries[0].groups[1]["Unknown model"], 12);
        assert!(load(Path::new("/missing/usage.db"), DAY).is_err());
    }
    #[test]
    fn costs_sum_across_timeframes_groups_and_missing_data() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let connection = Connection::open(file.path()).unwrap();
        connection
            .execute_batch("CREATE TABLE sessions(id TEXT,harness TEXT,blocks_json TEXT);")
            .unwrap();
        let receipt = serde_json::json!({"role":"user","id":"a","startedAt":DAY*8,"turnModel":{"harness":"codex","id":"codex:gpt-6.1-sol"},"turnMetrics":{"inputTokens":200000,"cacheReadTokens":100000,"cacheWriteTokens":0,"outputTokens":1000}});
        let mut unknown = receipt.clone();
        unknown["id"] = serde_json::json!("b");
        unknown["turnModel"]["id"] = serde_json::json!("codex:auto-review");
        let mut old = receipt.clone();
        old["id"] = serde_json::json!("c");
        old["startedAt"] = serde_json::json!(DAY);
        let mut missing = receipt.clone();
        missing["id"] = serde_json::json!("d");
        missing["turnMetrics"]
            .as_object_mut()
            .unwrap()
            .remove("cacheReadTokens");
        connection
            .execute(
                "INSERT INTO sessions VALUES('s','codex',?1)",
                [serde_json::json!([receipt, receipt, unknown, old, missing]).to_string()],
            )
            .unwrap();
        let summaries = load(file.path(), DAY * 9).unwrap();
        for (index, summary) in summaries.iter().enumerate() {
            let expected = if index < 2 { 0.22 } else { 0.44 };
            assert!((summary.cost.usd - expected).abs() < 1e-10);
            assert!(
                (summary.cost_trend.iter().map(|cost| cost.usd).sum::<f64>() - expected).abs()
                    < 1e-10
            );
            for group in &summary.cost_groups {
                assert!(
                    (group.values().map(|cost| cost.usd).sum::<f64>() - expected).abs() < 1e-10
                );
            }
            assert_eq!(summary.cost.unpriced_tokens, 402000);
            assert!(summary.cost.label().ends_with("partial"));
            assert_eq!(
                summary.cost_groups[1]["codex:auto-review"].label(),
                "Unpriced"
            );
        }
    }
    #[test]
    fn large_turn_sums_use_token_rates_and_fast_groups() {
        let mut summary = Summary {
            trend: vec![0; 24],
            ..Default::default()
        };
        let metrics = TurnMetrics {
            input_tokens: 77_000_000,
            cache_read_tokens: 76_000_000,
            output_tokens: 10_000,
            ..Default::default()
        };
        add(
            &mut summary,
            metrics,
            "codex",
            "codex:gpt-6.1-sol",
            DAY,
            DAY * 2,
            1,
            true,
            Some("fast"),
        );
        assert_eq!(summary.cost.unpriced_tokens, 0);
        assert_eq!(summary.cost.priced_tokens, 77_010_000);
        assert!((summary.cost.usd - 19.4).abs() < 1e-9);
        assert_eq!(summary.cost.label(), "$19.40");
        for grouping in &summary.cost_groups {
            assert!((grouping.values().map(|cost| cost.usd).sum::<f64>() - 19.4).abs() < 1e-9);
        }
        assert!(summary.cost_groups[1].contains_key("codex:gpt-6.1-sol · Fast"));
        assert!((summary.cost_trend.iter().map(|cost| cost.usd).sum::<f64>() - 19.4).abs() < 1e-9);
    }
    #[test]
    fn interval_stacks_match_totals_and_keep_unpriced_categories() {
        for days in [1, 7, 30, 90] {
            let now = 100 * DAY;
            let mut summary = Summary {
                trend: vec![0; if days == 1 { 24 } else { days as usize }],
                ..Default::default()
            };
            let metrics = TurnMetrics {
                input_tokens: 100,
                cache_read_tokens: 80,
                cache_write_tokens: 5,
                output_tokens: 10,
                ..Default::default()
            };
            add(
                &mut summary,
                metrics,
                "codex",
                "codex:gpt-6.1-sol",
                now - DAY / 2,
                now,
                days,
                true,
                None,
            );
            add(
                &mut summary,
                metrics,
                "claude",
                "Unknown model",
                now - DAY,
                now,
                days,
                true,
                None,
            );
            for (index, bucket) in summary.buckets.iter().enumerate() {
                assert_eq!(bucket.tokens.iter().sum::<u64>(), summary.trend[index]);
                assert!(
                    (bucket.costs.iter().map(|cost| cost.usd).sum::<f64>()
                        - summary.cost_trend[index].usd)
                        .abs()
                        < 1e-12
                );
            }
            for category in 0..4 {
                assert_eq!(
                    summary
                        .buckets
                        .iter()
                        .map(|bucket| bucket.tokens[category])
                        .sum::<u64>(),
                    summary.composition[category]
                );
            }
            let buckets = summary
                .buckets
                .iter()
                .filter(|bucket| bucket.tokens[2] > 0)
                .collect::<Vec<_>>();
            assert!(buckets.iter().all(|bucket| !bucket.costs[2].known));
            assert_eq!(
                summary
                    .buckets
                    .iter()
                    .map(|bucket| bucket.costs[2].unpriced_tokens)
                    .sum::<u64>(),
                10
            );
        }
    }
}
