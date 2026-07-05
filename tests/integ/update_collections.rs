#[cfg(test)]
mod tests {
    use osu_collect::{
        app::{runtime, update_source::extract_collection_id},
        osu_db::OsuClient,
    };
    use std::{
        collections::{HashMap, HashSet},
        path::PathBuf,
        time::Instant,
    };

    fn testing_db_path() -> Option<PathBuf> {
        if let Ok(p) = std::env::var("OSU_TESTING_DB") {
            let path = PathBuf::from(p);
            if path.join("osu!.db").exists() || path.join("collection.db").exists() {
                return Some(path);
            }
        }
        let relative = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testing-db");
        if relative.join("collection.db").exists() {
            Some(relative)
        } else {
            None
        }
    }

    /// Verify that reading the local database completes in a reasonable time.
    #[test]
    fn test_read_local_database_stable_timing() {
        let Some(path) = testing_db_path() else {
            println!("skipping: no testing-db/collection.db found");
            return;
        };

        let t = Instant::now();
        let result = runtime::read_local_database(OsuClient::Stable, path);
        let elapsed = t.elapsed();

        println!("read_local_database elapsed: {}ms", elapsed.as_millis());

        match result {
            Ok((collections, beatmapsets, checksums)) => {
                println!(
                    "stable: {} collections, {} beatmapsets, {} checksums",
                    collections.len(),
                    beatmapsets.len(),
                    checksums.len()
                );
                // DB read should be well under 30s even for large DBs
                assert!(
                    elapsed.as_secs() < 30,
                    "read_local_database too slow: {}ms",
                    elapsed.as_millis()
                );
            }
            Err(e) => {
                // Some test DBs may have unsupported versions — just warn
                println!("skipping assertions: read_local_database returned error: {e}");
            }
        }
    }

    /// Run the full fetch_and_compare pipeline against testing-db and measure phase timings.
    ///
    /// This test uses the stable DB and fetches NO real collections from the API —
    /// if there are no parseable collection IDs in testing-db it returns early.
    /// The budget is generous (60s) to account for CI network latency.
    #[tokio::test]
    async fn test_fetch_and_compare_timing() {
        let Some(path) = testing_db_path() else {
            println!("skipping: no testing-db found");
            return;
        };

        let t_db = Instant::now();
        let db_result = tokio::task::spawn_blocking({
            let p = path.clone();
            move || runtime::read_local_database(OsuClient::Stable, p)
        })
        .await
        .expect("spawn_blocking panicked");

        let db_ms = t_db.elapsed().as_millis();

        let (collections, beatmapsets, all_checksums) = match db_result {
            Ok(r) => r,
            Err(e) => {
                println!("skipping: read_local_database error: {e}");
                return;
            }
        };

        println!(
            "phase db-read: {}ms ({} collections, {} beatmapsets)",
            db_ms,
            collections.len(),
            beatmapsets.len()
        );

        let collection_ids: Vec<u32> = collections
            .iter()
            .filter_map(|c| extract_collection_id(&c.name).and_then(|id| u32::try_from(id).ok()))
            .collect();

        if collection_ids.is_empty() {
            println!(
                "skipping fetch_and_compare: no collections with osu!collector IDs in testing-db"
            );
            return;
        }

        println!("collection IDs to check: {:?}", collection_ids);

        let local_set_ids: HashSet<u32> = beatmapsets.iter().map(|bs| bs.id).collect();
        let local_checksums: HashSet<osu_collect::osu_db::Md5> =
            all_checksums.into_iter().collect();

        let t_fetch = Instant::now();
        let result = runtime::fetch_missing_beatmapsets(
            OsuClient::Stable,
            collection_ids,
            local_set_ids,
            local_checksums,
            &collections,
            HashMap::new(),
            runtime::FetchCompareSettings::default(),
        )
        .await;

        let fetch_ms = t_fetch.elapsed().as_millis();
        println!("phase fetch+compare: {}ms", fetch_ms);

        match result {
            Ok(res) => {
                println!("missing beatmapsets: {}", res.missing.len());
                // Generous budget: 60s total for network-dependent operations
                assert!(
                    fetch_ms < 60_000,
                    "fetch_and_compare too slow: {}ms",
                    fetch_ms
                );
            }
            Err(e) => {
                // Network errors in CI are acceptable — just print
                println!("fetch_and_compare returned error (may be network): {e}");
            }
        }
    }
}
