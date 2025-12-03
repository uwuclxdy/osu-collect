use super::{BeatmapStage, DownloadEvent, DownloadId, integrity::ExpectationIndex};
use std::{collections::HashSet, path::Path, sync::Arc};
use tokio::{fs, sync::mpsc::UnboundedSender};
use tracing::{debug, info};

pub(crate) struct PrecheckReport {
    pub(crate) satisfied: HashSet<u32>,
    pub(crate) skipped: u16,
    pub(crate) unverified: Vec<u32>,
    pub(crate) verified_bytes: u64,
}

pub(crate) async fn verify_existing_beatmapsets(
    id: DownloadId,
    output_dir: &Path,
    expectations: Arc<ExpectationIndex>,
    _parallelism: usize,
    notify_verified: bool,
    tx: &UnboundedSender<DownloadEvent>,
) -> Result<PrecheckReport, String> {
    info!(
        download_id = id,
        directory = %output_dir.display(),
        notify_verified,
        "Starting existing file verification (no zip inspection)"
    );
    let mut satisfied = HashSet::new();
    let mut skipped: u16 = 0;
    let mut verified_bytes: u64 = 0;

    let expectation_snapshot = expectations.snapshot();
    let expected_sets = &expectation_snapshot.by_set;

    let mut dir = fs::read_dir(output_dir)
        .await
        .map_err(|e| format!("Failed to read download directory: {}", e))?;

    while let Some(entry) = dir
        .next_entry()
        .await
        .map_err(|e| format!("Failed to read download directory: {}", e))?
    {
        let path = entry.path();
        if !is_osz_file(&path) {
            continue;
        }

        let Some(beatmapset_id) = extract_beatmapset_id_from_filename(&path) else {
            debug!(file = %path.display(), "Could not extract beatmapset ID from filename");
            continue;
        };

        // Only count files that match expected beatmapsets
        if !expected_sets.contains(&beatmapset_id) {
            continue;
        }

        let file_size = fs::metadata(&path).await.map(|m| m.len()).unwrap_or(0);

        // Skip empty files
        if file_size == 0 {
            debug!(download_id = id, beatmapset_id, "Skipping empty file");
            continue;
        }

        if satisfied.insert(beatmapset_id) {
            skipped = skipped.saturating_add(1);
            verified_bytes += file_size;
            if notify_verified {
                let _ = tx.send(DownloadEvent::BeatmapStatus {
                    id,
                    beatmapset_id,
                    stage: BeatmapStage::Skipped,
                    message: "Already present".to_string(),
                });
            }
        }
    }

    info!(
        download_id = id,
        verified = satisfied.len(),
        skipped,
        "Existing file verification complete"
    );

    Ok(PrecheckReport {
        satisfied,
        skipped,
        unverified: Vec::new(),
        verified_bytes,
    })
}

fn is_osz_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("osz"))
        .unwrap_or(false)
}

fn extract_beatmapset_id_from_filename(path: &Path) -> Option<u32> {
    let filename = path.file_stem()?.to_str()?;

    // Filename format is typically "{id}" or "{id} artist - title"
    let id_part = filename.split_whitespace().next()?;
    id_part.parse().ok()
}
