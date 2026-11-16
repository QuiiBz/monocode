use super::{blocks, HistoryMetadata, HistoryRevision, Transcript, Value};

const PARALLEL_BLOCKS: usize = 512;

pub(super) fn project(
    source: &[Value],
    mut metadata: Option<&mut HistoryMetadata>,
) -> Result<(Transcript, HistoryRevision), String> {
    if source.len() >= PARALLEL_BLOCKS {
        let workers = std::thread::available_parallelism()
            .map_or(1, usize::from)
            .saturating_sub(1)
            .min(3);
        if workers > 0 {
            if let Some(metadata) = metadata.as_deref_mut() {
                return project_parallel(source, metadata, workers);
            }
        }
    }
    let mut rows = Vec::new();
    for block in source {
        let projected = if metadata.is_some() {
            blocks::project_for_view(block)?
        } else {
            blocks::project(std::slice::from_ref(block))?
        };
        if let Some(metadata) = metadata.as_mut() {
            metadata.push(block, &projected, rows.len());
        }
        rows.extend(projected);
    }
    Ok((Transcript { rows }, HistoryRevision::of(source)))
}

fn project_parallel(
    source: &[Value],
    metadata: &mut HistoryMetadata,
    workers: usize,
) -> Result<(Transcript, HistoryRevision), String> {
    let chunk_size = source.len().div_ceil(workers);
    std::thread::scope(|scope| {
        let jobs: Vec<_> = source
            .chunks(chunk_size)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(blocks::project_for_view)
                        .collect::<Result<Vec<_>, _>>()
                })
            })
            .collect();
        let revision = HistoryRevision::of(source);
        let mut rows = Vec::new();
        for (chunk, job) in source.chunks(chunk_size).zip(jobs) {
            let projected = job
                .join()
                .map_err(|_| "History preparation worker failed".to_string())??;
            for (block, projected) in chunk.iter().zip(projected) {
                metadata.push(block, &projected, rows.len());
                rows.extend(projected);
            }
        }
        Ok((Transcript { rows }, revision))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parallel_projection_preserves_rows_metadata_and_revision() {
        let source = vec![
            json!({"id":"root","role":"user","text":"/mono Inspect","startedAt":10,"providerTurnId":"turn","turnMetrics":{"cacheHitPercent":95.12525746447635}}),
            json!({"id":"read","role":"tool","tool":{"kind":"read","status":"completed","rawInput":{"file_path":"src/lib.rs"},"detail":"Saved output 🙂".repeat(1024)},"future":{"keep":true}}),
            json!({"id":"reply","role":"assistant","text":"Done\n\nMore detail"}),
            json!({"id":"steer","role":"user","text":"Also check tests","sentAt":12,"attachments":[{"kind":"file","name":"test.rs","path":"test.rs"}]}),
            json!({"id":"agent","role":"tool","tool":{"kind":"agent","title":"Review","status":"failed","detail":"Saved report"}}),
            json!({"id":"internal","role":"user","text":"Internal","internal":true}),
            json!({"id":"draft","role":"user","text":"Unsent","draft":true}),
        ];
        let expected_rows = blocks::project(&source).unwrap();
        let expected_metadata = HistoryMetadata::from_blocks(&source).unwrap();
        let mut metadata = HistoryMetadata::default();
        let (history, revision) = project_parallel(&source, &mut metadata, 3).unwrap();
        assert_eq!(revision, HistoryRevision::of(&source));
        assert_eq!(history.rows, expected_rows);
        assert_eq!(metadata.root_turn, expected_metadata.root_turn);
        assert_eq!(
            metadata.operator_enabled,
            expected_metadata.operator_enabled
        );
        assert_eq!(metadata.draft, expected_metadata.draft);
        assert_eq!(
            serde_json::to_value(&metadata.turns).unwrap(),
            serde_json::to_value(&expected_metadata.turns).unwrap()
        );
    }
}
