use super::{shared, Store};
use crate::{agent::TurnInfo, total_usage::Bucket};
use std::collections::HashSet;

impl Store {
    pub fn session_usage(
        &self,
        session_id: i64,
        live_turn: Option<&TurnInfo>,
    ) -> Result<Bucket, String> {
        let live_turn = live_turn.filter(|turn| turn.metrics.is_some());
        let mut usage = Bucket::default();
        let mut seen_receipts = HashSet::new();
        if self.remote_session(session_id)?.is_some() {
            return Err("Host transcript is unavailable".into());
        }
        if self.shared.is_none() {
            for (turn, turn_info) in self.agent_settings(session_id)?.unwrap_or_default().turns {
                if live_turn.is_some_and(|live| {
                    live.provider_turn_id.as_ref().is_some_and(|identity| {
                        turn_info.provider_turn_id.as_ref() == Some(identity)
                    }) || live
                        .started_at
                        .is_some_and(|started| turn_info.started_at == Some(started))
                }) {
                    continue;
                }
                if let Some(metrics) = turn_info.metrics {
                    let identity = turn_info
                        .provider_turn_id
                        .unwrap_or_else(|| turn.to_string());
                    if seen_receipts.insert(identity) {
                        usage.add(
                            metrics,
                            &turn_info.provider,
                            turn_info.model_id.as_deref().unwrap_or(""),
                            turn_info.model_id.is_some(),
                            turn_info.service_tier.as_deref(),
                        );
                    }
                }
            }
            usage.add_live_turn(live_turn);
            return Ok(usage);
        }
        let session = self.persisted_session_id(session_id)?;
        let indexed = shared::has_table(&self.conn, "mono_transcripts")?
            && self
                .conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM mono_transcripts WHERE session_id=?1)",
                    [&session],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(|error| error.to_string())?;
        let source = if indexed {
            "SELECT sessions.harness,mono_blocks.block_json AS block FROM sessions JOIN mono_blocks ON mono_blocks.session_id=sessions.id WHERE sessions.id=?1"
        } else {
            "SELECT sessions.harness,json_each.value AS block FROM sessions,json_each(sessions.blocks_json) WHERE sessions.id=?1"
        };
        let query = format!(
            "SELECT harness,json_object('id',json_extract(block,'$.id'),'providerTurnId',json_extract(block,'$.providerTurnId'),'startedAt',json_extract(block,'$.startedAt'),'sentAt',json_extract(block,'$.sentAt'),'turnModel',json_extract(block,'$.turnModel'),'turnMetrics',json_extract(block,'$.turnMetrics'),'role','user') FROM ({source}) WHERE json_extract(block,'$.role')='user' AND json_type(block,'$.turnMetrics')='object'"
        );
        let mut statement = self
            .conn
            .prepare(&query)
            .map_err(|error| error.to_string())?;
        let records = statement
            .query_map([session], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?;
        for record in records {
            let (provider, raw) = record.map_err(|error| error.to_string())?;
            let block = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
            usage.add_receipt(&mut seen_receipts, &block, &provider, live_turn)?;
        }
        usage.add_live_turn(live_turn);
        Ok(usage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn receipt(receipt_id: &str) -> Value {
        json!({
            "role": "user", "id": receipt_id, "providerTurnId": receipt_id,
            "turnModel": {"harness": "codex", "id": "codex:gpt-6.1-sol"},
            "turnMetrics": {"inputTokens": 200000, "cacheReadTokens": 100000,
                "cacheWriteTokens": 0, "outputTokens": 1000}
        })
    }

    fn save_blocks(store: &Store, session_id: i64, blocks: Value) {
        store
            .conn
            .execute(
                "UPDATE sessions SET blocks_json=?2 WHERE id=?1",
                rusqlite::params![
                    store.persisted_session_id(session_id).unwrap(),
                    blocks.to_string()
                ],
            )
            .unwrap();
    }

    #[test]
    fn session_totals_include_all_dates_and_models_without_duplicates() {
        let store = Store::open_monocode(std::path::Path::new(":memory:")).unwrap();
        let session = store.create("codex", "/project").unwrap();
        let other = store.create("codex", "/other").unwrap();
        let mut old = receipt("old");
        old["startedAt"] = json!(1);
        let mut fast = receipt("fast");
        fast["turnModel"]["serviceTier"] = json!("fast");
        let mut duplicate = old.clone();
        duplicate["id"] = json!("steer");
        let mut copied = receipt("copied");
        copied["providerTurnId"] = Value::Null;
        copied["startedAt"] = json!(1);
        copied["sentAt"] = json!(10000);
        save_blocks(&store, session.id, json!([old, duplicate, fast, copied]));
        save_blocks(&store, other.id, json!([receipt("other")]));
        let usage = store.session_usage(session.id, None).unwrap();
        assert_eq!(usage.tokens, [200000, 200000, 0, 2000]);
        assert_eq!(usage.total_tokens(), 402000);
        assert!((usage.total_cost().usd - 0.66).abs() < 1e-10);
        assert_eq!(usage.total_cost().unpriced_tokens, 0);

        let mut claude = receipt("claude");
        claude["turnModel"] = json!({"harness": "claude", "id": "claude:claude-opus-5-5"});
        save_blocks(&store, session.id, json!([claude]));
        let usage = store.session_usage(session.id, None).unwrap();
        assert_eq!(usage.tokens, [200000, 100000, 0, 1000]);
        assert!((usage.total_cost().usd - 0.84).abs() < 1e-10);
    }

    #[test]
    fn indexed_session_totals_include_turns_before_the_history_window() {
        let store = Store::open_monocode(std::path::Path::new(":memory:")).unwrap();
        let session = store.create("codex", "/project").unwrap();
        let original = store.persisted_session_id(session.id).unwrap();
        save_blocks(&store, session.id, json!([receipt("legacy")]));
        store
            .conn
            .execute_batch(
                "CREATE TABLE mono_transcripts(session_id TEXT PRIMARY KEY);
            CREATE TABLE mono_blocks(session_id TEXT,seq INTEGER,block_json TEXT);
            CREATE TABLE mono_turns(session_id TEXT,seq INTEGER);",
            )
            .unwrap();
        store
            .conn
            .execute("INSERT INTO mono_transcripts VALUES(?1)", [&original])
            .unwrap();
        for index in 0..15 {
            store
                .conn
                .execute(
                    "INSERT INTO mono_blocks VALUES(?1,?2,?3)",
                    rusqlite::params![original, index, receipt(&index.to_string()).to_string()],
                )
                .unwrap();
            store
                .conn
                .execute(
                    "INSERT INTO mono_turns VALUES(?1,?2)",
                    rusqlite::params![original, index],
                )
                .unwrap();
        }
        assert_eq!(store.history_before(session.id).unwrap(), Some(5));
        let usage = store.session_usage(session.id, None).unwrap();
        assert_eq!(usage.total_tokens(), 201000 * 15);
        assert!((usage.total_cost().usd - 0.22 * 15.).abs() < 1e-10);
    }

    #[test]
    fn incomplete_and_unknown_receipts_keep_tokens_and_partial_costs() {
        let store = Store::open_monocode(std::path::Path::new(":memory:")).unwrap();
        let session = store.create("codex", "/project").unwrap();
        let mut unknown = receipt("unknown");
        unknown["turnModel"]["id"] = json!("unknown-model");
        let mut incomplete = receipt("incomplete");
        incomplete["turnMetrics"]
            .as_object_mut()
            .unwrap()
            .remove("cacheReadTokens");
        save_blocks(
            &store,
            session.id,
            json!([receipt("known"), unknown, incomplete]),
        );
        let usage = store.session_usage(session.id, None).unwrap();
        let cost = usage.total_cost();
        assert_eq!(usage.total_tokens(), 603000);
        assert!((cost.usd - 0.22).abs() < 1e-10);
        assert_eq!(cost.priced_tokens, 201000);
        assert_eq!(cost.unpriced_tokens, 402000);
        assert_eq!(cost.label(), "$0.22 · partial");
    }

    #[test]
    fn live_metrics_replace_the_saved_turn_instead_of_adding_it_twice() {
        let store = Store::open_monocode(std::path::Path::new(":memory:")).unwrap();
        let session = store.create("codex", "/project").unwrap();
        let mut live_turn = TurnInfo {
            provider: "codex".into(),
            model_id: Some("codex:gpt-6.1-sol".into()),
            provider_turn_id: Some("active".into()),
            started_at: Some(1000),
            metrics: Some(crate::agent::TurnMetrics {
                input_tokens: 400000,
                cache_read_tokens: 300000,
                output_tokens: 2000,
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut active = receipt("active");
        active["startedAt"] = json!(1000);
        save_blocks(&store, session.id, json!([receipt("previous"), active]));
        for provider_turn_id in [Some("active".into()), None] {
            live_turn.provider_turn_id = provider_turn_id;
            let usage = store.session_usage(session.id, Some(&live_turn)).unwrap();
            assert_eq!(usage.total_tokens(), 603000);
            assert!((usage.total_cost().usd - 0.47).abs() < 1e-10);
        }
        live_turn.metrics = None;
        assert_eq!(
            store
                .session_usage(session.id, Some(&live_turn))
                .unwrap()
                .total_tokens(),
            402000
        );
    }
}
