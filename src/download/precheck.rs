use super::{
    BeatmapStage, DownloadEvent, DownloadId,
    integrity::{ArchiveOutcome, ExpectationIndex},
};
use futures_util::stream::{FuturesUnordered, StreamExt};
use std::{collections::HashSet, path::Path, sync::Arc};
use tokio::{fs, sync::mpsc::UnboundedSender, task};
use tracing::{debug, info, warn};

pub(crate) struct PrecheckReport {
    pub(crate) satisfied: HashSet<u32>,
    pub(crate) skipped: u16,
    pub(crate) unverified: Vec<u32>,
}

pub(crate) async fn verify_existing_beatmapsets(
    id: DownloadId,
    output_dir: &Path,
    expectations: Arc<ExpectationIndex>,
    parallelism: usize,
    notify_verified: bool,
    tx: &UnboundedSender<DownloadEvent>,
) -> Result<PrecheckReport, String> {
    info!(
        download_id = id,
        directory = %output_dir.display(),
        parallelism,
        notify_verified,
        "Starting existing file verification"
    );
    let mut satisfied = HashSet::new();
    let mut skipped: u16 = 0;
    let mut unverified: HashSet<u32> = HashSet::new();

    let mut dir = fs::read_dir(output_dir)
        .await
        .map_err(|e| format!("Failed to read download directory: {}", e))?;

    let concurrency = parallelism.max(1);
    let mut dir_exhausted = false;
    let mut tasks: FuturesUnordered<_> = FuturesUnordered::new();

    loop {
        while !dir_exhausted && tasks.len() < concurrency {
            match dir
                .next_entry()
                .await
                .map_err(|e| format!("Failed to read download directory: {}", e))?
            {
                Some(entry) => {
                    let path = entry.path();
                    if !is_osz_file(&path) {
                        continue;
                    }

                    let expectation_snapshot = expectations.snapshot();
                    tasks.push(task::spawn_blocking(move || {
                        let outcome =
                            super::integrity::inspect_archive(&path, expectation_snapshot.as_ref());
                        (path, outcome)
                    }));
                }
                None => {
                    dir_exhausted = true;
                }
            }
        }

        if tasks.is_empty() {
            if dir_exhausted {
                break;
            }

            continue;
        }

        match tasks.next().await {
            Some(Ok((path, outcome))) => {
                let checksum_set = match &outcome {
                    ArchiveOutcome::Invalid {
                        beatmapset_id: Some(set_id),
                        ..
                    } if outcome.is_checksum_mismatch(*set_id) => Some(*set_id),
                    _ => None,
                };

                if let Some(set_id) = checksum_set {
                    let filename = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("unknown")
                        .to_string();
                    if satisfied.insert(set_id) {
                        skipped = skipped.saturating_add(1);
                    }
                    if unverified.insert(set_id) && notify_verified {
                        let _ = tx.send(DownloadEvent::BeatmapStatus {
                            id,
                            beatmapset_id: set_id,
                            stage: BeatmapStage::Skipped,
                            message: "Already present (checksum mismatch; marked unverified)"
                                .to_string(),
                        });
                    }
                    let _ = tx.send(DownloadEvent::Log {
                        id,
                        message: format!(
                            "Marked {} as unverified due to checksum mismatch",
                            filename
                        ),
                    });
                    continue;
                }

                match outcome {
                    ArchiveOutcome::Valid { beatmapset_id } => {
                        if satisfied.insert(beatmapset_id) {
                            skipped = skipped.saturating_add(1);
                            if notify_verified {
                                let _ = tx.send(DownloadEvent::BeatmapStatus {
                                    id,
                                    beatmapset_id,
                                    stage: BeatmapStage::Skipped,
                                    message: "Already present (hash verified)".to_string(),
                                });
                            }
                        }
                    }
                    ArchiveOutcome::Invalid {
                        beatmapset_id,
                        reason,
                    } => {
                        let filename = path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("unknown")
                            .to_string();
                        let _ = fs::remove_file(&path).await;
                        debug!(download_id = id, file = %filename, reason = %reason, "Removed invalid archive");
                        let _ = tx.send(DownloadEvent::Log {
                            id,
                            message: format!("Removed {}: {}", filename, reason),
                        });

                        if let Some(set_id) = beatmapset_id {
                            let _ = tx.send(DownloadEvent::BeatmapStatus {
                                id,
                                beatmapset_id: set_id,
                                stage: BeatmapStage::Pending,
                                message: format!(
                                    "Existing file invalid ({}); scheduling re-download",
                                    reason
                                ),
                            });
                        }
                    }
                    ArchiveOutcome::NotPartOfCollection => {}
                }
            }
            Some(Err(err)) => {
                warn!(download_id = id, error = %err, "Archive inspection task failed");
                return Err(format!("Archive inspection task failed: {}", err));
            }
            None => break,
        }
    }

    info!(
        download_id = id,
        verified = satisfied.len(),
        skipped,
        "Existing file verification complete"
    );
    let mut unverified_sorted: Vec<u32> = unverified.into_iter().collect();
    unverified_sorted.sort_unstable();
    Ok(PrecheckReport {
        satisfied,
        skipped,
        unverified: unverified_sorted,
    })
}

fn is_osz_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("osz"))
        .unwrap_or(false)
}
