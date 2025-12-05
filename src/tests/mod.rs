mod download;
mod io;
mod lazer;

#[cfg(test)]
fn minimal_zip_bytes() -> Vec<u8> {
    vec![
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
        0x1E, 0x00, 0x00, 0x00, // Start of central directory
        0x00, 0x00, // Comment length
    ]
}

#[cfg(test)]
fn create_temp_file(content: &[u8]) -> std::path::PathBuf {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir();
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = dir.join(format!(
        "osu_collect_download_test_{}_{}.tmp",
        std::process::id(),
        id
    ));
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(content).unwrap();
    file.sync_all().unwrap();
    path
}
