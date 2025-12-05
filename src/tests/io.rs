#[cfg(test)]
mod tests {
    use crate::tests::{create_temp_file, minimal_zip_bytes};
    use crate::worker::io::ensure_valid_archive;

    fn cleanup_temp_file(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn test_ensure_valid_archive_valid_zip() {
        // Create a minimal valid ZIP file (empty archive)
        // ZIP local file header: PK\x03\x04
        // Followed by end of central directory record
        let zip_bytes = minimal_zip_bytes();

        let path = create_temp_file(&zip_bytes);
        let result = ensure_valid_archive(&path, true).await;
        cleanup_temp_file(&path);

        assert!(result.is_ok(), "Valid ZIP should pass validation");
    }

    #[tokio::test]
    async fn test_ensure_valid_archive_html_error_page() {
        // Simulate actual error page with leading newline (common in real responses)
        let html_content = b"\n<!DOCTYPE html><html><body>Error 404</body></html>";

        let path = create_temp_file(html_content);
        let result = ensure_valid_archive(&path, true).await;
        cleanup_temp_file(&path);

        assert!(result.is_err(), "HTML error page should fail validation");

        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("HTML error page"),
            "Error message should mention HTML: {err_msg}"
        );
    }

    #[tokio::test]
    async fn test_ensure_valid_archive_json_content_not_rejected() {
        // JSON content is no longer explicitly rejected - only checked for ZIP signature
        let json_content = b"{\"error\":\"Beatmapset not found\"}";

        let path = create_temp_file(json_content);
        let result = ensure_valid_archive(&path, true).await;
        cleanup_temp_file(&path);

        assert!(
            result.is_err(),
            "JSON content should fail validation (missing ZIP signature)"
        );

        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("ZIP signature"),
            "Error message should mention ZIP signature: {err_msg}"
        );
    }

    #[tokio::test]
    async fn test_ensure_valid_archive_random_bytes() {
        let random_bytes = b"This is not a ZIP file at all!";

        let path = create_temp_file(random_bytes);
        let result = ensure_valid_archive(&path, true).await;
        cleanup_temp_file(&path);

        assert!(result.is_err(), "Random bytes should fail validation");

        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("ZIP signature"),
            "Error message should mention ZIP signature: {err_msg}"
        );
    }

    #[tokio::test]
    async fn test_ensure_valid_archive_empty_file() {
        let path = create_temp_file(b"");
        let result = ensure_valid_archive(&path, true).await;
        cleanup_temp_file(&path);

        assert!(result.is_err(), "Empty file should fail validation");
    }

    #[tokio::test]
    async fn test_ensure_valid_archive_too_small() {
        let path = create_temp_file(b"PK"); // Only 2 bytes
        let result = ensure_valid_archive(&path, true).await;
        cleanup_temp_file(&path);

        assert!(result.is_err(), "File too small should fail validation");
    }

    #[tokio::test]
    async fn test_central_directory_validation_flag() {
        // Valid header but missing EOCD footer. Strict mode should catch it while lenient mode skips.
        let path = create_temp_file(&[0x50, 0x4B, 0x03, 0x04]);

        let strict = ensure_valid_archive(&path, true).await;
        assert!(strict.is_err(), "Strict mode must fail without EOCD");

        let lenient = ensure_valid_archive(&path, false).await;
        assert!(
            lenient.is_ok(),
            "Lenient mode should allow header-only archives"
        );

        cleanup_temp_file(&path);
    }
}
