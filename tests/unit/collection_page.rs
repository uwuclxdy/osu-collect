use crate::app::collection::{CollectionPage, FailureReason};
use crate::download::{BeatmapStage, FailedMap};

fn page_with_failures(reasons: &[FailureReason]) -> CollectionPage {
    let mut page = CollectionPage::new(1, "test".to_string(), 2);
    page.set_failed_maps(
        reasons
            .iter()
            .enumerate()
            .map(|(i, &reason)| FailedMap {
                beatmapset_id: i as u32 + 1,
                title: None,
                reason,
            })
            .collect(),
    );
    page
}

#[test]
fn toggle_failed_section_flips_expanded() {
    let mut page = page_with_failures(&[FailureReason::NotFound]);
    assert!(!page.failed_section_expanded);
    page.toggle_failed_section();
    assert!(page.failed_section_expanded);
    page.toggle_failed_section();
    assert!(!page.failed_section_expanded);
}

#[test]
fn toggle_failed_section_noop_when_empty() {
    let mut page = CollectionPage::new(1, "test".to_string(), 2);
    page.toggle_failed_section();
    assert!(!page.failed_section_expanded);
}

#[test]
fn set_failed_maps_stores_reason_and_sorts_by_id() {
    let mut page = CollectionPage::new(1, "test".to_string(), 2);
    page.set_failed_maps(vec![
        FailedMap {
            beatmapset_id: 30,
            title: None,
            reason: FailureReason::NetworkError,
        },
        FailedMap {
            beatmapset_id: 10,
            title: None,
            reason: FailureReason::NotFound,
        },
        FailedMap {
            beatmapset_id: 20,
            title: None,
            reason: FailureReason::ValidationFailed,
        },
    ]);
    let ids: Vec<u32> = page.failed_maps.iter().map(|f| f.beatmapset_id).collect();
    assert_eq!(ids, vec![10, 20, 30]);
    assert_eq!(page.failed_maps[0].reason, FailureReason::NotFound);
    assert_eq!(page.failed_maps[1].reason, FailureReason::ValidationFailed);
    assert_eq!(page.failed_maps[2].reason, FailureReason::NetworkError);
}

#[test]
fn mark_deferred_frees_the_slot_and_keeps_it_queued() {
    let mut page = CollectionPage::new(1, "test".to_string(), 2);
    // Claim an active slot (only a Downloading stage may).
    page.update_active_status(42, BeatmapStage::Downloading, "downloading", false, None);
    assert_eq!(page.active_lines().count(), 1);

    page.mark_deferred(42);
    // Slot freed, not counted terminal — the map is deferred-pending / "queued".
    assert_eq!(page.active_lines().count(), 0);
    assert_eq!(page.deferred_count(), 1);
    assert!(page.rate_limited_or_deferred());
    // No active row left, but a deferred map keeps the run "fully rate limited".
    assert!(page.fully_rate_limited());

    // Re-attempting the map clears its deferred-pending state.
    page.update_active_status(42, BeatmapStage::Downloading, "downloading", false, None);
    assert_eq!(page.deferred_count(), 0);
    assert_eq!(page.active_lines().count(), 1);
    assert!(!page.fully_rate_limited());
}

#[test]
fn clear_active_downloads_drops_deferred_state() {
    let mut page = CollectionPage::new(1, "test".to_string(), 2);
    page.mark_deferred(7);
    assert_eq!(page.deferred_count(), 1);
    page.clear_active_downloads();
    assert_eq!(page.deferred_count(), 0);
    assert!(!page.rate_limited_or_deferred());
}

#[test]
fn failure_reason_labels_are_correct() {
    assert_eq!(FailureReason::NotFound.label(), "not found");
    assert_eq!(FailureReason::RateLimited.label(), "rate-limited");
    assert_eq!(FailureReason::NetworkError.label(), "network error");
    assert_eq!(FailureReason::ValidationFailed.label(), "archive invalid");
    assert_eq!(FailureReason::Unknown.label(), "unknown error");
}
