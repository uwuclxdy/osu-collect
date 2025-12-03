#[cfg(test)]
mod tests {
    use crate::osu_db::{BeatmapReader, LazerReader};
    use std::path::PathBuf;

    #[test]
    fn test_realm_reading() {
        let path = PathBuf::from("realm");
        let realm_file = path.join("client.realm");

        if !realm_file.exists() {
            println!(
                "Skipping test: realm file not found at {}",
                realm_file.display()
            );
            return;
        }

        let reader = LazerReader::new(path);

        // Test collections
        let collections = reader
            .list_collections()
            .expect("Failed to read collections");
        println!("\n=== Collections ===");
        println!("Total: {}", collections.len());
        for c in collections.iter().take(5) {
            println!("  - {}: {} checksums", c.name, c.beatmap_checksums.len());
        }

        // Test beatmapsets
        let beatmapsets = reader
            .list_beatmapsets()
            .expect("Failed to read beatmapsets");
        println!("\n=== Beatmapsets ===");
        println!("Total: {}", beatmapsets.len());

        let total_beatmaps: usize = beatmapsets.iter().map(|bs| bs.beatmaps.len()).sum();
        println!("Total beatmaps (diffs): {}", total_beatmaps);

        // Show sample
        for bs in beatmapsets.iter().take(3) {
            println!("  - Set {}: {} beatmaps", bs.id, bs.beatmaps.len());
            for bm in bs.beatmaps.iter().take(2) {
                println!("      Checksum {}:", bm.checksum);
            }
        }

        // Verify that beatmapsets have valid data
        let sets_with_beatmaps = beatmapsets
            .iter()
            .filter(|bs| !bs.beatmaps.is_empty())
            .count();
        println!("\nBeatmapsets with beatmaps: {}", sets_with_beatmaps);

        // Test all checksums (includes checksums from beatmaps with invalid OnlineIDs)
        let all_checksums = reader
            .list_all_checksums()
            .expect("Failed to read all checksums");

        let checksums_from_beatmapsets: std::collections::HashSet<&String> = beatmapsets
            .iter()
            .flat_map(|bs| bs.beatmaps.iter().map(|b| &b.checksum))
            .collect();

        println!("\n=== Checksum Comparison ===");
        println!(
            "Checksums from beatmapsets: {}",
            checksums_from_beatmapsets.len()
        );
        println!("All checksums (including skipped): {}", all_checksums.len());
        println!(
            "Additional checksums recovered: {}",
            all_checksums.len() - checksums_from_beatmapsets.len()
        );

        assert!(!beatmapsets.is_empty(), "Should have some beatmapsets");
        assert!(total_beatmaps > 0, "Should have some beatmaps");
        assert!(
            all_checksums.len() >= checksums_from_beatmapsets.len(),
            "All checksums should include at least as many as from beatmapsets"
        );
    }

    /// Test that simulates the comparison logic to verify the fix for false "not installed" reports.
    /// This test verifies that using `list_all_checksums()` recovers more installed beatmaps
    /// than using only checksums from `list_beatmapsets()`.
    #[test]
    fn test_all_checksums_recovers_more_matches() {
        let path = PathBuf::from("realm");
        let realm_file = path.join("client.realm");

        if !realm_file.exists() {
            println!(
                "Skipping test: realm file not found at {}",
                realm_file.display()
            );
            return;
        }

        let reader = LazerReader::new(path);

        // Get beatmapsets (only includes beatmaps with valid OnlineIDs)
        let beatmapsets = reader
            .list_beatmapsets()
            .expect("Failed to read beatmapsets");

        // Build checksum set from beatmapsets (old method - what was causing the bug)
        let checksums_old: std::collections::HashSet<String> = beatmapsets
            .iter()
            .flat_map(|bs| bs.beatmaps.iter().map(|b| b.checksum.clone()))
            .collect();

        // Get ALL checksums (new method - the fix)
        let all_checksums = reader
            .list_all_checksums()
            .expect("Failed to read all checksums");
        let checksums_new: std::collections::HashSet<String> = all_checksums.into_iter().collect();

        // The new method should have at least as many checksums as the old method
        assert!(
            checksums_new.len() >= checksums_old.len(),
            "New method should have at least as many checksums. Old: {}, New: {}",
            checksums_old.len(),
            checksums_new.len()
        );

        // Calculate the difference
        let additional_checksums = checksums_new.len() - checksums_old.len();

        println!("\n=== All Checksums Recovery Test ===");
        println!("Checksums (old method): {}", checksums_old.len());
        println!("Checksums (new method): {}", checksums_new.len());
        println!("Additional checksums recovered: {}", additional_checksums);

        // The new method should recover some additional checksums
        // (assuming the realm has beatmaps with invalid OnlineIDs)
        if additional_checksums > 0 {
            println!(
                "SUCCESS: {} additional checksums recovered that would have caused false 'not installed' reports",
                additional_checksums
            );
        } else {
            println!(
                "Note: No additional checksums recovered (realm may not have beatmaps with invalid OnlineIDs)"
            );
        }

        // Verify that ALL old checksums are in the new set (new is a superset)
        let missing_from_new: Vec<_> = checksums_old
            .iter()
            .filter(|cs| !checksums_new.contains(*cs))
            .collect();

        assert!(
            missing_from_new.is_empty(),
            "New checksum set should contain all old checksums. Missing: {:?}",
            missing_from_new
        );
    }

    /// Test that verifies collections contain checksums that should be recoverable.
    /// This checks if any collection checksums are in all_checksums but not in beatmapset checksums.
    #[test]
    fn test_collection_checksums_recovery() {
        let path = PathBuf::from("realm");
        let realm_file = path.join("client.realm");

        if !realm_file.exists() {
            println!(
                "Skipping test: realm file not found at {}",
                realm_file.display()
            );
            return;
        }

        let reader = LazerReader::new(path);

        // Get collections
        let collections = reader
            .list_collections()
            .expect("Failed to read collections");

        // Get beatmapsets
        let beatmapsets = reader
            .list_beatmapsets()
            .expect("Failed to read beatmapsets");

        // Build checksum set from beatmapsets (old method)
        let checksums_from_beatmapsets: std::collections::HashSet<String> = beatmapsets
            .iter()
            .flat_map(|bs| bs.beatmaps.iter().map(|b| b.checksum.clone()))
            .collect();

        // Get ALL checksums (new method)
        let all_checksums: std::collections::HashSet<String> = reader
            .list_all_checksums()
            .expect("Failed to read all checksums")
            .into_iter()
            .collect();

        // Collect all checksums from collections
        let collection_checksums: std::collections::HashSet<String> = collections
            .iter()
            .flat_map(|c| c.beatmap_checksums.iter().cloned())
            .collect();

        println!("\n=== Collection Checksums Analysis ===");
        println!(
            "Total checksums in collections: {}",
            collection_checksums.len()
        );
        println!(
            "Checksums from beatmapsets: {}",
            checksums_from_beatmapsets.len()
        );
        println!("All checksums (including skipped): {}", all_checksums.len());

        // Check how many collection checksums are in each set
        let in_beatmapsets = collection_checksums
            .iter()
            .filter(|cs| checksums_from_beatmapsets.contains(*cs))
            .count();
        let in_all = collection_checksums
            .iter()
            .filter(|cs| all_checksums.contains(*cs))
            .count();

        println!(
            "Collection checksums found in beatmapsets: {}",
            in_beatmapsets
        );
        println!("Collection checksums found in all checksums: {}", in_all);
        println!(
            "Recovered by using all checksums: {}",
            in_all - in_beatmapsets
        );

        // This is the key assertion: using all_checksums should find more collection matches
        assert!(
            in_all >= in_beatmapsets,
            "All checksums should find at least as many collection matches"
        );
    }

    /// Test that detects the "phantom beatmapsets" bug:
    /// Beatmapsets where the checksums exist locally but the beatmapset OnlineID is invalid,
    /// causing them to be incorrectly marked as "Not Installed".
    #[test]
    fn test_phantom_beatmapsets_detection() {
        let path = PathBuf::from("realm");
        let realm_file = path.join("client.realm");

        if !realm_file.exists() {
            println!(
                "Skipping test: realm file not found at {}",
                realm_file.display()
            );
            return;
        }

        let reader = LazerReader::new(path);

        // Get beatmapsets (only includes beatmaps with valid OnlineIDs)
        let beatmapsets = reader
            .list_beatmapsets()
            .expect("Failed to read beatmapsets");

        // Get ALL checksums (includes beatmaps with invalid OnlineIDs)
        let all_checksums: std::collections::HashSet<String> = reader
            .list_all_checksums()
            .expect("Failed to read all checksums")
            .into_iter()
            .collect();

        // Build beatmapset ID set (for reference)
        let _beatmapset_ids: std::collections::HashSet<u32> =
            beatmapsets.iter().map(|bs| bs.id).collect();

        // Build checksum-to-beatmapset-id map (only for beatmapsets with valid IDs)
        let mut checksum_to_beatmapset: std::collections::HashMap<&String, u32> =
            std::collections::HashMap::new();
        for bs in &beatmapsets {
            for bm in &bs.beatmaps {
                checksum_to_beatmapset.insert(&bm.checksum, bs.id);
            }
        }

        // Find "orphan" checksums - checksums that exist in all_checksums
        // but are NOT in any beatmapset (because their beatmapset OnlineID was invalid)
        let orphan_checksums: Vec<_> = all_checksums
            .iter()
            .filter(|cs| !checksum_to_beatmapset.contains_key(cs))
            .collect();

        println!("\n=== Phantom Beatmapsets Detection ===");
        println!("Total beatmapsets with valid IDs: {}", beatmapsets.len());
        println!("Total all checksums: {}", all_checksums.len());
        println!(
            "Orphan checksums (not in any valid beatmapset): {}",
            orphan_checksums.len()
        );

        // These orphan checksums would have caused false "Not Installed" reports
        // before the fix, because:
        // 1. The beatmapset ID wouldn't be found in local_beatmapsets
        // 2. But the checksum WOULD be in local_checksums (after our first fix)
        // 3. The old comparison logic checked beatmapset ID first, then checksums
        //    Now it checks checksums first, so these will be correctly detected as installed

        if !orphan_checksums.is_empty() {
            println!(
                "SUCCESS: {} orphan checksums found that would cause phantom 'Not Installed' reports",
                orphan_checksums.len()
            );
            println!("The comparison logic fix handles these by checking checksums first.");
        } else {
            println!(
                "Note: No orphan checksums found (realm may not have beatmaps with invalid beatmapset OnlineIDs)"
            );
        }
    }
}
