//! Multipart-envelope unwrap for mirrors that don't serve a bare archive.
//!
//! nzbasic's CDN (`direct.nzbasic.com`) returns a cached `.osz` wrapped in a
//! `multipart/form-data` envelope: the body's first line is the boundary, the
//! part headers carry the filename, the real ZIP follows the blank line, and a
//! closing `--boundary--` trails the EOCD. No response header declares the
//! boundary, so it is sniffed from the body. A missing set can come back as a
//! `200` with a JSON or HTML error body instead, which must read as a miss.
//!
//! The unwrap runs on the fully streamed temp file (the incomplete-length
//! check upstream compares raw bytes against the raw `Content-Length`, so
//! stripping mid-stream would break it) and rewrites the payload down to
//! offset 0 in place — both `Magic` and `Eocd` validation assume the ZIP
//! starts at file offset 0.

use md5::{Digest, Md5};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// ZIP local-file-header signature (`PK\x03\x04`).
const ZIP_MAGIC: &[u8; 4] = b"PK\x03\x04";
/// Sniff window for the boundary line + part headers (real envelopes are
/// ~170 bytes; anything without a blank line inside this window is malformed).
const HEAD_WINDOW: usize = 4096;
const COPY_BUF: usize = 256 * 1024;

/// Outcome of inspecting an envelope-flagged mirror's downloaded body.
#[derive(Debug)]
pub(crate) enum Unwrap {
    /// The body was already a bare archive; the file is untouched.
    Bare,
    /// The envelope was stripped in place; the file now holds the bare payload.
    Stripped {
        /// MD5 of the payload now on disk.
        md5: Box<str>,
        /// Payload length the file was truncated to.
        len: u64,
    },
    /// The body is the mirror's "set does not exist" answer (JSON/HTML).
    Miss,
}

/// Inspect a fully streamed download from an envelope-flagged mirror and
/// strip the multipart envelope in place if present.
pub(crate) async fn unwrap_envelope(path: &Path) -> Result<Unwrap, String> {
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || unwrap_blocking(&path))
        .await
        .map_err(|err| format!("envelope unwrap task failed: {err}"))?
}

fn unwrap_blocking(path: &Path) -> Result<Unwrap, String> {
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|err| format!("failed to reopen download: {err}"))?;
    let file_len = file
        .metadata()
        .map_err(|err| format!("failed to stat download: {err}"))?
        .len();

    let mut head = vec![0u8; HEAD_WINDOW.min(file_len as usize)];
    file.read_exact(&mut head)
        .map_err(|err| format!("failed to read download head: {err}"))?;

    if head.starts_with(ZIP_MAGIC) {
        return Ok(Unwrap::Bare);
    }
    if !head.starts_with(b"--") {
        return Ok(Unwrap::Miss);
    }

    let boundary_len =
        find(&head, b"\r\n").ok_or("malformed multipart envelope: no boundary line")?;
    let headers_end = find(&head, b"\r\n\r\n")
        .ok_or("malformed multipart envelope: unterminated part headers")?;
    let payload_start = headers_end as u64 + 4;

    // The closing delimiter is CRLF + the dash-boundary line + trailing dashes.
    let trailer = [b"\r\n", &head[..boundary_len], b"--".as_slice()].concat();
    let tail_len = (trailer.len() + 16).min(file_len as usize);
    let mut tail = vec![0u8; tail_len];
    file.seek(SeekFrom::End(-(tail_len as i64)))
        .map_err(|err| format!("failed to seek download tail: {err}"))?;
    file.read_exact(&mut tail)
        .map_err(|err| format!("failed to read download tail: {err}"))?;
    let trailer_at =
        rfind(&tail, &trailer).ok_or("malformed multipart envelope: missing closing boundary")?;
    let payload_end = file_len - tail_len as u64 + trailer_at as u64;

    if payload_end <= payload_start {
        return Err("malformed multipart envelope: empty payload".into());
    }

    let mut hasher = Md5::new();
    let mut buf = vec![0u8; COPY_BUF];
    let mut src = payload_start;
    let mut dst = 0u64;
    while src < payload_end {
        let want = ((payload_end - src) as usize).min(COPY_BUF);
        file.seek(SeekFrom::Start(src))
            .map_err(|err| format!("failed to seek payload: {err}"))?;
        file.read_exact(&mut buf[..want])
            .map_err(|err| format!("failed to read payload: {err}"))?;
        hasher.update(&buf[..want]);
        file.seek(SeekFrom::Start(dst))
            .map_err(|err| format!("failed to seek payload target: {err}"))?;
        file.write_all(&buf[..want])
            .map_err(|err| format!("failed to rewrite payload: {err}"))?;
        src += want as u64;
        dst += want as u64;
    }

    let len = payload_end - payload_start;
    file.set_len(len)
        .map_err(|err| format!("failed to truncate envelope trailer: {err}"))?;

    Ok(Unwrap::Stripped {
        md5: crate::worker::finalize_md5(hasher),
        len,
    })
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).rposition(|w| w == needle)
}

#[cfg(test)]
#[path = "../tests/envelope.rs"]
mod tests;
