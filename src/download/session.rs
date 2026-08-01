use super::{
    DownloadConfig, DownloadError, DownloadEvent, DownloadId, DownloadStage,
    SelectiveDownloadCollection,
    lock::{ActiveDownloadRegistry, DownloadLockGuard},
    precheck::{PrecheckOptions, PrecheckReport, verify_existing_beatmapsets},
};
use crate::{
    core::collection::{Collection, CollectionService, HttpCollectionService, Uploader},
    utils::{self, prepare_directory},
};
use futures_util::{StreamExt, stream};
use osu_downloader::collection::CollectionClient;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};
use tokio::{fs, sync::watch};
use tracing::{debug, info, warn};

pub(crate) struct OutputPreparation {
    pub(crate) output_dir: PathBuf,
    pub(crate) display: String,
}

pub(crate) enum SessionTarget {
    Collection(Collection),
    Selective {
        collection: Collection,
        collections: Vec<SelectiveDownloadCollection>,
        collection_names: Vec<String>,
    },
    /// A raw-ids run (search / filter): no collection metadata, just a display
    /// label. The ids live on [`DownloadSession::beatmapset_ids`] and there is
    /// no `collection.db` write, so [`collection`](Self::collection) returns
    /// `None` here.
    Ids {
        label: String,
        source: super::IdsRunSource,
    },
}

impl SessionTarget {
    pub(crate) fn announce_ready(
        &self,
        emit: &impl Fn(DownloadEvent),
        id: DownloadId,
        output: &OutputPreparation,
        beatmapset_ids: &[u32],
    ) {
        match self {
            SessionTarget::Collection(collection) => {
                emit(DownloadEvent::CollectionReady {
                    id,
                    collection_name: collection.name.to_string(),
                    uploader: collection.uploader.username.to_string(),
                    // The target list, not `collection.beatmapsets`: the total
                    // the page shows (and its gauge denominator) must count only
                    // what the run enqueues, and the list is already deduped and
                    // stripped of any declined-retry sets.
                    total_maps: beatmapset_ids.len(),
                    output_dir: output.display.clone(),
                });
            }
            SessionTarget::Selective {
                collection,
                collection_names,
                ..
            } => {
                emit(DownloadEvent::CollectionReady {
                    id,
                    collection_name: selective_collection_name(collection_names).to_string(),
                    uploader: collection.uploader.username.to_string(),
                    total_maps: collection.beatmapsets.len(),
                    output_dir: output.display.clone(),
                });
            }
            SessionTarget::Ids { label, source } => {
                emit(DownloadEvent::CollectionReady {
                    id,
                    collection_name: label.clone(),
                    uploader: source.uploader().to_string(),
                    total_maps: beatmapset_ids.len(),
                    output_dir: output.display.clone(),
                });
            }
        }
    }

    /// The resolved collection this run downloads, or `None` for a raw-ids run
    /// (search/filter carry only ids, no collection metadata, and skip `collection.db`).
    pub(crate) fn collection(&self) -> Option<&Collection> {
        match self {
            SessionTarget::Collection(collection) | SessionTarget::Selective { collection, .. } => {
                Some(collection)
            }
            SessionTarget::Ids { .. } => None,
        }
    }

    pub(crate) fn selective_collections(&self) -> Option<&[SelectiveDownloadCollection]> {
        match self {
            SessionTarget::Collection(_) | SessionTarget::Ids { .. } => None,
            SessionTarget::Selective { collections, .. } => Some(collections),
        }
    }
}

pub(crate) struct DownloadSession {
    pub(crate) id: DownloadId,
    pub(crate) target: SessionTarget,
    pub(crate) beatmapset_ids: Vec<u32>,
    pub(crate) pending_ids: Vec<u32>,
    pub(crate) initial_unverified: HashSet<u32>,
    pub(crate) initial_satisfied: HashSet<u32>,
    pub(crate) skipped_existing: u32,
    /// Beatmapsets pre-skipped because they were already in the osu! library
    /// (a subset of `owned_ids` that precheck had not already satisfied).
    pub(crate) skipped_owned: u32,
    /// Sizes already known for `beatmapset_ids` at request time; seeds the
    /// size estimate so a fully-cached selection needs no probe. Empty for
    /// the Collection/Selective targets (no size source).
    pub(crate) known_sizes: HashMap<u32, u64>,
    pub(crate) output: OutputPreparation,
    pub(crate) _lock_guard: DownloadLockGuard,
}

pub(crate) enum PrepareTarget<'a> {
    Collection {
        collection_input: &'a str,
        /// A still-fresh payload from the app's session cache; `None` fetches.
        prefetched: Option<Collection>,
        /// Beatmapsets the user chose not to retry (they failed a previous run
        /// for this collection). Removed from the run's target list, which is
        /// also what the run announces as its total — so no count describes a
        /// map the run never enqueues. The `Collection` payload itself keeps
        /// them, so `collection.db` still records the whole collection.
        skip_previously_failed: &'a HashSet<u32>,
    },
    Selective {
        collection_ids: &'a [u32],
        collections: Vec<SelectiveDownloadCollection>,
        beatmapset_ids: &'a [u32],
        /// Still-fresh payloads from the app's session cache, keyed by collection
        /// id. A hit skips that collection's fetch.
        prefetched: HashMap<u32, Collection>,
    },
    /// Raw beatmapset ids from a search or filter run — no collection fetch.
    /// `label` names the page; `folder_tag` derives the per-run output subdir
    /// (`<source>-<folder_tag>`).
    Ids {
        beatmapset_ids: &'a [u32],
        label: &'a str,
        folder_tag: &'a str,
        source: super::IdsRunSource,
        /// Sizes already known for these ids (search: the osu probe cache;
        /// filter: the free nzbasic `SizeMap`); seeds the size estimate.
        known_sizes: HashMap<u32, u64>,
    },
}

pub(crate) struct PrepareParams<'a> {
    pub(crate) id: DownloadId,
    pub(crate) cancel_rx: watch::Receiver<bool>,
    pub(crate) config: &'a DownloadConfig,
    pub(crate) registry: &'a ActiveDownloadRegistry,
    pub(crate) emit: super::Emit<'a>,
    pub(crate) target: PrepareTarget<'a>,
    /// When set, precheck skips validation so every requested id stays pending
    /// and the library overwrites existing archives (`OnExists::Overwrite`).
    pub(crate) overwrite: bool,
    /// Beatmapsets already in the osu! library: pre-skipped before downloading
    /// but still folded into `collection.db`. Empty for the selective/retry path.
    pub(crate) owned_ids: HashSet<u32>,
}

impl DownloadSession {
    pub(crate) async fn prepare(params: PrepareParams<'_>) -> Result<Option<Self>, DownloadError> {
        let directory = params.config.directory.as_str();
        let (target, output, beatmapset_ids, known_sizes) = match params.target {
            PrepareTarget::Collection {
                collection_input,
                prefetched,
                skip_previously_failed,
            } => {
                let collection = match prefetched {
                    Some(collection) => collection,
                    None => resolve_collection(collection_input).await?,
                };
                if collection.beatmapsets.is_empty() {
                    warn!(
                        collection_id = collection.id,
                        "collection contained no beatmaps"
                    );
                    return Err(DownloadError::EmptyCollection);
                }
                let mut beatmapset_ids: Vec<u32> =
                    collection.beatmapsets.iter().map(|b| b.id).collect();
                beatmapset_ids.sort_unstable();
                beatmapset_ids.dedup();
                // The declined-retry set leaves the run entirely: it is filtered
                // before the target list reaches precheck, the announced total,
                // or the failed-maps reconcile, so a map the user chose not to
                // retry is neither downloaded nor reported as resolved.
                if !skip_previously_failed.is_empty() {
                    let before = beatmapset_ids.len();
                    beatmapset_ids.retain(|id| !skip_previously_failed.contains(id));
                    info!(
                        collection_id = collection.id,
                        skipped = before - beatmapset_ids.len(),
                        "excluding previously-failed beatmapsets from this run"
                    );
                }
                let output = prepare_output_dir(directory, &collection.folder_name()).await?;
                (
                    SessionTarget::Collection(collection),
                    output,
                    beatmapset_ids,
                    HashMap::new(),
                )
            }
            PrepareTarget::Selective {
                collection_ids,
                collections,
                beatmapset_ids,
                prefetched,
            } => {
                let service = HttpCollectionService::new(CollectionClient::new());
                let (collection, collections, collection_names) = resolve_selective_with(
                    &service,
                    collection_ids,
                    collections,
                    beatmapset_ids,
                    &prefetched,
                    params.id,
                    params.emit,
                )
                .await?;
                let output = prepare_selective_output(directory, collection_ids).await?;
                let mut target_ids = beatmapset_ids.to_vec();
                target_ids.sort_unstable();
                target_ids.dedup();
                (
                    SessionTarget::Selective {
                        collection,
                        collections,
                        collection_names,
                    },
                    output,
                    target_ids,
                    HashMap::new(),
                )
            }
            PrepareTarget::Ids {
                beatmapset_ids,
                label,
                folder_tag,
                source,
                known_sizes,
            } => {
                let mut ids = beatmapset_ids.to_vec();
                ids.sort_unstable();
                ids.dedup();
                // Query-derived subdir so different queries land in different
                // dirs (no lock collision). No collection fetch — the ids came
                // straight from the search/filter results.
                let folder = ids_folder_name(source.folder_prefix(), folder_tag);
                let output = prepare_output_dir(directory, &folder).await?;
                (
                    SessionTarget::Ids {
                        label: label.to_string(),
                        source,
                    },
                    output,
                    ids,
                    known_sizes,
                )
            }
        };

        let lock_guard = DownloadLockGuard::acquire(&output.output_dir, params.registry)?;
        target.announce_ready(&params.emit, params.id, &output, &beatmapset_ids);

        Self::finalize(
            params.id,
            params.cancel_rx,
            target,
            beatmapset_ids,
            known_sizes,
            output,
            lock_guard,
            params.config,
            params.overwrite,
            params.owned_ids,
            params.emit,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn finalize(
        id: DownloadId,
        cancel_rx: watch::Receiver<bool>,
        target: SessionTarget,
        beatmapset_ids: Vec<u32>,
        known_sizes: HashMap<u32, u64>,
        output: OutputPreparation,
        lock_guard: DownloadLockGuard,
        config: &DownloadConfig,
        overwrite: bool,
        owned_ids: HashSet<u32>,
        emit: super::Emit<'_>,
    ) -> Result<Option<Self>, DownloadError> {
        // Precheck may only count ids the run actually targets. For a collection
        // that list is its sets minus any the user declined to retry, so a
        // dropped map can never be counted as satisfied against a total that
        // excludes it — nor reach `initial_satisfied` and clear itself out of
        // the failed-maps file without ever being downloaded.
        let expectations: Arc<HashSet<u32>> = Arc::new(beatmapset_ids.iter().copied().collect());
        emit(DownloadEvent::StageChanged {
            id,
            stage: DownloadStage::Rechecking,
        });

        let report = verify_existing_beatmapsets(
            id,
            &output.output_dir,
            expectations,
            config.concurrent.max(1) as usize,
            PrecheckOptions {
                notify_verified: true,
                archive_validation: config.archive_validation,
                overwrite,
            },
            &cancel_rx,
            emit,
        )
        .await?;

        emit(DownloadEvent::StageChanged {
            id,
            stage: DownloadStage::Downloading,
        });

        if report.aborted {
            emit(DownloadEvent::Failed {
                id,
                message: "download aborted by user".into(),
            });
            return Ok(None);
        }

        let PrecheckReport {
            mut satisfied,
            skipped,
            unverified,
            verified_bytes,
            ..
        } = report;

        let mut initial_unverified: HashSet<u32> = unverified.iter().copied().collect();

        if verified_bytes > 0 {
            emit(DownloadEvent::VerifiedMapSizes {
                id,
                total_bytes: verified_bytes,
            });
        }

        let (pending_ids, skipped_owned) = partition_pending(
            &beatmapset_ids,
            &mut satisfied,
            &mut initial_unverified,
            &owned_ids,
        );

        emit(DownloadEvent::DownloadTarget {
            id,
            remaining: pending_ids.len(),
        });

        Ok(Some(Self {
            id,
            target,
            beatmapset_ids,
            pending_ids,
            initial_unverified,
            initial_satisfied: satisfied,
            skipped_existing: skipped,
            skipped_owned,
            known_sizes,
            output,
            _lock_guard: lock_guard,
        }))
    }
}

/// Fold already-owned library ids (scoped to this collection) into `satisfied`
/// and split out the still-pending downloads. An owned id precheck hadn't
/// already satisfied counts as newly skipped; folding it into `satisfied` keeps
/// it out of the download while staying eligible for `collection.db`. A folded
/// id is also dropped from `initial_unverified` so an owned-but-unverified set
/// is not counted as both skipped and unverified. Returns
/// `(pending_ids, skipped_owned)`.
fn partition_pending(
    beatmapset_ids: &[u32],
    satisfied: &mut HashSet<u32>,
    initial_unverified: &mut HashSet<u32>,
    owned_ids: &HashSet<u32>,
) -> (Vec<u32>, u32) {
    let mut skipped_owned = 0u32;
    for &id in beatmapset_ids {
        if owned_ids.contains(&id) && satisfied.insert(id) {
            skipped_owned += 1;
            initial_unverified.remove(&id);
        }
    }
    let pending_ids = beatmapset_ids
        .iter()
        .copied()
        .filter(|id| !satisfied.contains(id))
        .collect();
    (pending_ids, skipped_owned)
}

async fn prepare_output_dir(
    base_path: &str,
    folder_name: &str,
) -> Result<OutputPreparation, DownloadError> {
    let normalized = {
        let trimmed = base_path.trim();
        if trimmed.is_empty() { "." } else { trimmed }
    };

    let base_dir = prepare_directory(normalized).await?;
    debug!(base = %base_dir.display(), "validated base download directory");

    let output_dir = base_dir.join(folder_name);
    fs::create_dir_all(&output_dir).await?;
    let display_str = output_dir.to_string_lossy().to_string();
    info!(output_dir = %display_str, "prepared output directory");

    Ok(OutputPreparation {
        output_dir,
        display: display_str,
    })
}

/// The per-run subdir for a raw-ids download: `<prefix>-<sanitized tag>`, using
/// the same forbidden-character set (`_` replacement) the collection folder
/// sanitizer applies. An empty/blank tag collapses to the bare prefix, so a run
/// always lands in a valid, recognizable directory.
///
/// Search derives the tag from the query TEXT only (not the mode/status/sort
/// filters); filter derives it from the preset name or the query hash. Two runs
/// of the same tag collide the way two runs of one collection do: concurrent →
/// the second fails the per-output-dir lock; sequential → they merge into the
/// one dir.
pub(crate) fn ids_folder_name(prefix: &str, tag: &str) -> String {
    let mut out = String::with_capacity(prefix.len() + tag.len() + 1);
    out.push_str(prefix);
    out.push('-');
    for c in tag.chars() {
        out.push(match c {
            '/' | '\\' | '\0' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        });
    }
    let trimmed = out.trim();
    if trimmed.len() == prefix.len() + 1 {
        prefix.to_string()
    } else {
        trimmed.to_string()
    }
}

/// The per-run subdir for a selective download: `update-<id>` for a single
/// collection, `update-<n>-collections` for a batch. Both the update source and
/// the collection source's browse&pick land here, so a picked subset never
/// writes into the whole collection's folder.
pub(crate) fn selective_folder_name(collection_ids: &[u32]) -> String {
    if let [id] = collection_ids {
        format!("update-{id}")
    } else {
        format!("update-{}-collections", collection_ids.len())
    }
}

async fn prepare_selective_output(
    directory: &str,
    collection_ids: &[u32],
) -> Result<OutputPreparation, DownloadError> {
    prepare_output_dir(directory, &selective_folder_name(collection_ids)).await
}

async fn resolve_collection(collection_input: &str) -> Result<Collection, DownloadError> {
    let collection_id = utils::parse_collection_id(collection_input)?;
    let service = HttpCollectionService::new(CollectionClient::new());
    let collection = service.fetch_collection(collection_id).await?;

    info!(
        collection_id,
        collection_name = %collection.name,
        total_maps = collection.beatmapsets.len(),
        "fetched collection metadata"
    );

    Ok(collection)
}

const RESOLVE_CONCURRENCY: usize = 6;

pub(crate) async fn resolve_selective_with<S>(
    service: &S,
    collection_ids: &[u32],
    requested_collections: Vec<SelectiveDownloadCollection>,
    beatmapset_ids: &[u32],
    prefetched: &HashMap<u32, Collection>,
    id: DownloadId,
    emit: super::Emit<'_>,
) -> Result<(Collection, Vec<SelectiveDownloadCollection>, Vec<String>), DownloadError>
where
    S: CollectionService,
{
    let target_set: HashSet<u32> = beatmapset_ids.iter().copied().collect();
    let total = collection_ids.len() as u32;
    emit(DownloadEvent::ResolveProgress {
        id,
        current: 0,
        total,
    });

    let progress = Arc::new(AtomicU32::new(0));
    let fetch_results: Vec<(u32, Result<_, _>)> = stream::iter(collection_ids.iter().copied())
        .map(|collection_id| {
            let progress = Arc::clone(&progress);
            async move {
                // The id set being downloaded came out of the same scan/resolve that
                // cached this payload, so reusing it is more consistent than pairing
                // a stale id set with freshly fetched checksums.
                let result = match prefetched.get(&collection_id) {
                    Some(collection) => Ok(collection.clone()),
                    None => service.fetch_collection(collection_id).await,
                };
                let done = progress.fetch_add(1, Ordering::Relaxed) + 1;
                emit(DownloadEvent::ResolveProgress {
                    id,
                    current: done,
                    total,
                });
                (collection_id, result)
            }
        })
        .buffered(RESOLVE_CONCURRENCY)
        .collect()
        .await;

    let mut collection_names = Vec::with_capacity(fetch_results.len());
    let mut resolved_collections = Vec::with_capacity(fetch_results.len());
    let mut selected_collection = Collection {
        id: collection_ids.first().copied().unwrap_or_default(),
        name: "updates".to_string(),
        description: None,
        uploader: Uploader {
            id: 0,
            username: "updates".to_string(),
        },
        beatmapsets: Vec::new(),
        favourites: 0,
    };
    let mut seen_beatmapset_ids: HashSet<u32> = HashSet::new();

    for (collection_id, result) in fetch_results {
        match result {
            Ok(collection) => {
                let requested = requested_collections.iter().find(|c| c.id == collection_id);
                let collection_name = requested
                    .and_then(|c| (!c.name.is_empty()).then(|| c.name.clone()))
                    .unwrap_or_else(|| format!("{}-{}", collection.name, collection.id));
                let requested_ids: HashSet<u32> = requested
                    .map(|c| c.beatmapset_ids.iter().copied().collect())
                    .unwrap_or_default();
                let mut resolved = SelectiveDownloadCollection {
                    id: collection_id,
                    name: collection_name.clone(),
                    beatmapset_ids: Vec::new(),
                };

                collection_names.push(collection.name.to_string());

                for beatmapset in collection.beatmapsets {
                    if target_set.contains(&beatmapset.id) {
                        if requested_ids.contains(&beatmapset.id) {
                            resolved.beatmapset_ids.push(beatmapset.id);
                        }
                        if seen_beatmapset_ids.insert(beatmapset.id) {
                            selected_collection.beatmapsets.push(beatmapset);
                        }
                    }
                }

                if !resolved.beatmapset_ids.is_empty() {
                    resolved_collections.push(resolved);
                }
            }
            Err(err) => {
                warn!(
                    collection_id,
                    error = %err,
                    "skipping missing collection in selective download"
                );
            }
        }
    }

    selected_collection.name = selective_collection_name(&collection_names);

    if resolved_collections.is_empty() {
        return Err(DownloadError::EmptyCollection);
    }
    if selected_collection.beatmapsets.is_empty() {
        return Err(DownloadError::NoBeatmapsets);
    }

    Ok((selected_collection, resolved_collections, collection_names))
}

fn selective_collection_name(collection_names: &[String]) -> String {
    if collection_names.len() == 1 {
        format!("update: {}", collection_names[0])
    } else {
        format!("update: {} collections", collection_names.len())
    }
}

#[cfg(test)]
#[path = "../../tests/unit/download_session.rs"]
mod tests;
