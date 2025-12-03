#[cfg(test)]
mod tests {
    use crate::worker::io::ensure_valid_archive;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn create_temp_file(content: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir();
        let id = COUNTER.fetch_add(1, AtomicOrdering::SeqCst);
        let path = dir.join(format!(
            "osu_collect_test_{}_{}.tmp",
            std::process::id(),
            id
        ));
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(content).unwrap();
        file.sync_all().unwrap();
        path
    }

    fn cleanup_temp_file(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn test_ensure_valid_archive_valid_zip() {
        // Create a minimal valid ZIP file (empty archive)
        // ZIP local file header: PK\x03\x04
        // Followed by end of central directory record
        let zip_bytes: &[u8] = &[
            0x50, 0x4B, 0x03, 0x04, // Local file header signature
            0x14, 0x00, // Version needed to extract
            0x00, 0x00, // General purpose bit flag
            0x00, 0x00, // Compression method
            0x00, 0x00, // Last mod file time
            0x00, 0x00, // Last mod file date
            0x00, 0x00, 0x00, 0x00, // CRC-32
            0x00, 0x00, 0x00, 0x00, // Compressed size
            0x00, 0x00, 0x00, 0x00, // Uncompressed size
            0x00, 0x00, // File name length
            0x00, 0x00, // Extra field length
            // End of central directory record
            0x50, 0x4B, 0x05, 0x06, // End of central directory signature
            0x00, 0x00, // Number of this disk
            0x00, 0x00, // Disk where central directory starts
            0x00, 0x00, // Number of central directory records on this disk
            0x00, 0x00, // Total number of central directory records
            0x00, 0x00, 0x00, 0x00, // Size of central directory
            0x1E, 0x00, 0x00, 0x00, // Offset of start of central directory
            0x00, 0x00, // Comment length
        ];

        let path = create_temp_file(zip_bytes);
        let result = ensure_valid_archive(&path).await;
        cleanup_temp_file(&path);

        assert!(result.is_ok(), "Valid ZIP should pass validation");
    }

    #[tokio::test]
    async fn test_ensure_valid_archive_html_error_page() {
        // Simulate actual error page with leading newline (common in real responses)
        let html_content = b"\n<!DOCTYPE html><html><body>Error 404</body></html>";

        let path = create_temp_file(html_content);
        let result = ensure_valid_archive(&path).await;
        cleanup_temp_file(&path);

        assert!(result.is_err(), "HTML error page should fail validation");

        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("HTML error page"),
            "Error message should mention HTML: {err_msg}"
        );
    }

    #[tokio::test]
    async fn test_ensure_valid_archive_json_error() {
        let json_content = b"{\"error\":\"Beatmapset not found\"}";

        let path = create_temp_file(json_content);
        let result = ensure_valid_archive(&path).await;
        cleanup_temp_file(&path);

        assert!(
            result.is_err(),
            "JSON error response should fail validation"
        );

        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("JSON error"),
            "Error message should mention JSON: {err_msg}"
        );
    }

    #[tokio::test]
    async fn test_ensure_valid_archive_random_bytes() {
        let random_bytes = b"This is not a ZIP file at all!";

        let path = create_temp_file(random_bytes);
        let result = ensure_valid_archive(&path).await;
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
        let result = ensure_valid_archive(&path).await;
        cleanup_temp_file(&path);

        assert!(result.is_err(), "Empty file should fail validation");
    }

    #[tokio::test]
    async fn test_ensure_valid_archive_too_small() {
        let path = create_temp_file(b"PK"); // Only 2 bytes
        let result = ensure_valid_archive(&path).await;
        cleanup_temp_file(&path);

        assert!(result.is_err(), "File too small should fail validation");
    }
}
