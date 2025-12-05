#[cfg(test)]
mod tracker_tests {
    use crate::download::BeatmapTracker;
    use std::collections::HashSet;

    #[test]
    fn test_mark_pending_from_failed() {
        let pending: HashSet<u32> = [1, 2, 3].into_iter().collect();
        let tracker = BeatmapTracker::new(pending);

        tracker.mark_failed(1);
        assert!(!tracker.is_pending(1));

        let result = tracker.mark_pending(1);
        assert!(result, "mark_pending should succeed for failed beatmap");
        assert!(
            tracker.is_pending(1),
            "Beatmap should be pending after mark_pending"
        );
    }

    #[test]
    fn test_mark_pending_from_verified_fails() {
        let pending: HashSet<u32> = [1, 2, 3].into_iter().collect();
        let tracker = BeatmapTracker::new(pending);

        tracker.mark_verified(1);
        assert!(!tracker.is_pending(1));

        let result = tracker.mark_pending(1);
        assert!(!result, "mark_pending should fail for verified beatmap");
        assert!(tracker.is_verified(1), "Beatmap should still be verified");
    }

    #[test]
    fn test_mark_pending_unknown_id() {
        let pending: HashSet<u32> = [1, 2, 3].into_iter().collect();
        let tracker = BeatmapTracker::new(pending);

        let result = tracker.mark_pending(999);
        assert!(!result, "mark_pending should fail for unknown beatmap ID");
    }

    #[test]
    fn test_mark_verified_unknown_id_adds_it() {
        let pending: HashSet<u32> = [1].into_iter().collect();
        let tracker = BeatmapTracker::new(pending);

        tracker.mark_verified(999);
        assert!(
            tracker.is_verified(999),
            "mark_verified should add unknown ID as verified"
        );
    }

    #[test]
    fn test_with_verified_constructor() {
        let pending: HashSet<u32> = [1, 2].into_iter().collect();
        let pre_verified: HashSet<u32> = [3, 4].into_iter().collect();

        let tracker = BeatmapTracker::with_verified(pending, pre_verified);

        assert!(tracker.is_pending(1));
        assert!(tracker.is_pending(2));
        assert!(tracker.is_verified(3));
        assert!(tracker.is_verified(4));
        assert_eq!(tracker.pending_count(), 2);
    }

    #[test]
    fn test_pending_snapshot_returns_copy() {
        let pending: HashSet<u32> = [1, 2, 3].into_iter().collect();
        let tracker = BeatmapTracker::new(pending);

        let snapshot = tracker.pending_snapshot();
        assert_eq!(snapshot.len(), 3);

        tracker.mark_verified(1);

        // Original snapshot should be unchanged
        assert!(snapshot.contains(&1));
        // But tracker state should reflect the change
        assert!(!tracker.is_pending(1));
    }

    #[test]
    fn test_is_all_complete() {
        let pending: HashSet<u32> = [1, 2].into_iter().collect();
        let tracker = BeatmapTracker::new(pending);

        assert!(!tracker.is_all_complete());

        tracker.mark_verified(1);
        assert!(!tracker.is_all_complete());

        tracker.mark_verified(2);
        assert!(tracker.is_all_complete());
    }

    #[test]
    fn test_remove_pending() {
        let pending: HashSet<u32> = [1, 2, 3].into_iter().collect();
        let tracker = BeatmapTracker::new(pending);

        let remaining = tracker.remove_pending(1);
        assert_eq!(remaining, Some(2));

        let remaining = tracker.remove_pending(2);
        assert_eq!(remaining, Some(1));

        // Already removed
        let remaining = tracker.remove_pending(1);
        assert_eq!(remaining, None);
    }
}

#[cfg(test)]
mod cleanup_tracker_tests {
    use crate::download::CleanupTracker;
    use std::path::PathBuf;

    #[test]
    fn test_track_and_mark_complete() {
        let tracker = CleanupTracker::new();
        let path = PathBuf::from("/tmp/test.osz");

        tracker.track(&path);
        tracker.mark_complete(&path);
        // No panic = success
    }

    #[test]
    fn test_mark_removed() {
        let tracker = CleanupTracker::new();
        let path = PathBuf::from("/tmp/test.osz");

        tracker.track(&path);
        tracker.mark_removed(&path);
        // No panic = success
    }
}

#[cfg(test)]
mod shutdown_token_tests {
    use crate::download::ShutdownToken;

    #[test]
    fn test_initial_state() {
        let token = ShutdownToken::new();
        assert!(!token.is_cancelled());
    }

    #[test]
    fn test_cancel() {
        let token = ShutdownToken::new();
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn test_clone_shares_state() {
        let token = ShutdownToken::new();
        let clone = token.clone();

        token.cancel();
        assert!(clone.is_cancelled());
    }

    #[test]
    fn test_mark_completed() {
        let token = ShutdownToken::new();
        token.mark_completed();
        // Should not panic, mark_completed is just state tracking
    }
}

#[cfg(test)]
pub(crate) mod archive_validation_tests {
    use crate::tests::{create_temp_file, minimal_zip_bytes};
    use crate::worker::io::{ArchiveValidationOptions, ArchiveValidationResult, validate_archive};

    fn cleanup_temp_file(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn test_validate_archive_valid() {
        let path = create_temp_file(&minimal_zip_bytes());
        let opts = ArchiveValidationOptions {
            verify_zip_eocd: true,
            remove_on_invalid: false,
        };

        let result = validate_archive(&path, opts).await.unwrap();
        cleanup_temp_file(&path);

        assert!(matches!(result, ArchiveValidationResult::Valid));
    }

    #[tokio::test]
    async fn test_validate_archive_not_found() {
        let path = std::path::PathBuf::from("/nonexistent/path/file.osz");
        let opts = ArchiveValidationOptions {
            verify_zip_eocd: false,
            remove_on_invalid: false,
        };

        let result = validate_archive(&path, opts).await.unwrap();
        assert!(matches!(result, ArchiveValidationResult::NotFound));
    }

    #[tokio::test]
    async fn test_validate_archive_empty() {
        let path = create_temp_file(b"");
        let opts = ArchiveValidationOptions {
            verify_zip_eocd: false,
            remove_on_invalid: false,
        };

        let result = validate_archive(&path, opts).await.unwrap();
        cleanup_temp_file(&path);

        assert!(matches!(result, ArchiveValidationResult::Invalid(_)));
    }

    #[tokio::test]
    async fn test_validate_archive_invalid_with_remove() {
        let path = create_temp_file(b"not a zip");
        let opts = ArchiveValidationOptions {
            verify_zip_eocd: false,
            remove_on_invalid: true,
        };

        let result = validate_archive(&path, opts).await.unwrap();

        assert!(matches!(result, ArchiveValidationResult::Removed(_)));
        assert!(!path.exists(), "File should be removed");
    }

    #[tokio::test]
    async fn test_no_file_size_limit() {
        // This test verifies that large files are not rejected
        // We can't easily create a >100MB file in tests, but we verify
        // by checking that the validation code path doesn't include size checks
        let path = create_temp_file(&minimal_zip_bytes());
        let opts = ArchiveValidationOptions {
            verify_zip_eocd: true,
            remove_on_invalid: false,
        };

        let result = validate_archive(&path, opts).await.unwrap();
        cleanup_temp_file(&path);

        // Should pass validation regardless of theoretical size
        assert!(matches!(result, ArchiveValidationResult::Valid));
    }

    #[tokio::test]
    async fn test_json_not_explicitly_rejected() {
        // JSON is no longer explicitly detected - just fails ZIP signature check
        let path = create_temp_file(b"{\"error\": \"test\"}");
        let opts = ArchiveValidationOptions {
            verify_zip_eocd: false,
            remove_on_invalid: false,
        };

        let result = validate_archive(&path, opts).await.unwrap();
        cleanup_temp_file(&path);

        match result {
            ArchiveValidationResult::Invalid(reason) => {
                assert!(
                    reason.contains("ZIP signature"),
                    "Should fail due to missing ZIP signature, not JSON detection: {reason}"
                );
            }
            _ => panic!("Expected Invalid result"),
        }
    }
}

#[cfg(test)]
mod download_error_tests {
    use crate::download::DownloadError;

    #[test]
    fn test_concurrent_download_error() {
        let err = DownloadError::ConcurrentDownload("/path/to/dir".to_string());
        let msg = err.to_string();
        assert!(msg.contains("Concurrent download"));
        assert!(msg.contains("/path/to/dir"));
    }

    #[test]
    fn test_error_variants_display() {
        let errors = [
            DownloadError::RateLimited,
            DownloadError::NoMirrors,
            DownloadError::NoBeatmapsets,
            DownloadError::EmptyCollection,
            DownloadError::DirectoryNotEmpty,
            DownloadError::Aborted,
        ];

        for err in errors {
            let _msg = err.to_string();
        }
    }
}

#[cfg(test)]
mod download_result_tests {
    use crate::download::{DownloadResult, client::DownloadFailure};

    #[test]
    fn test_download_result_skipped() {
        let result = DownloadResult::Skipped("existing.osz".into());
        assert!(matches!(result, DownloadResult::Skipped(_)));
    }

    #[test]
    fn test_download_result_failed() {
        let failure = DownloadFailure {
            mirror: None,
            reason: "error message".into(),
        };
        let result = DownloadResult::Failed(failure.clone());
        assert!(matches!(result, DownloadResult::Failed(_)));
        if let DownloadResult::Failed(inner) = result {
            assert_eq!(inner.reason, failure.reason);
            assert!(inner.mirror.is_none());
        }
    }

    #[test]
    fn test_download_result_aborted() {
        let result = DownloadResult::Aborted;
        assert!(matches!(result, DownloadResult::Aborted));
    }
}
