//! Worker module for streaming downloads and I/O

use crate::{Error, Result};
use bytes::Bytes;
use futures_util::StreamExt;
use md5::{Digest, Md5};
use std::{
    future::{Future, pending},
    path::{Path, PathBuf},
    sync::{
        Arc, LazyLock,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tokio::{
    fs::{self, OpenOptions},
    io::AsyncWriteExt,
    task,
    time::timeout,
};

const MIN_PROGRESS_DELTA: u64 = 131_072;
const MIN_PROGRESS_INTERVAL: Duration = Duration::from_millis(200);

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);
static PID_STRING: LazyLock<String> = LazyLock::new(|| std::process::id().to_string());

/// Streamed download output.
pub(crate) struct DownloadStreamResult {
    /// Whether the download was cancelled before completion.
    pub aborted: bool,
    /// MD5 digest of the downloaded bytes.
    pub hash: Option<Box<str>>,
    /// Number of bytes written to the temp file.
    pub bytes_written: u64,
    /// Temporary file path holding the completed download.
    pub temp_path: PathBuf,
}

struct HashWorker {
    sender: Option<mpsc::Sender<Bytes>>,
    handle: task::JoinHandle<Box<str>>,
}

impl HashWorker {
    fn new() -> Self {
        let (sender, receiver) = mpsc::channel::<Bytes>();
        let handle = task::spawn_blocking(move || {
            let mut hasher = Md5::new();
            while let Ok(chunk) = receiver.recv() {
                hasher.update(&chunk);
            }
            finalize_md5(hasher)
        });

        Self {
            sender: Some(sender),
            handle,
        }
    }

    /// Send a chunk for hashing. `Bytes` is reference-counted — no allocation.
    fn update(&self, data: Bytes) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(data);
        }
    }

    async fn finalize(mut self) -> Result<Box<str>> {
        self.sender.take();
        self.handle
            .await
            .map_err(|err| Error::network(format!("hash worker failed: {err}")))
    }

    fn abort(mut self) {
        self.sender.take();
        self.handle.abort();
    }
}

pub(crate) fn finalize_md5(hasher: Md5) -> Box<str> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = hasher.finalize();
    let mut buf = [0u8; 32];
    for (i, &b) in digest.iter().enumerate() {
        buf[i * 2] = HEX[(b >> 4) as usize];
        buf[i * 2 + 1] = HEX[(b & 0xf) as usize];
    }
    // buf contains only ASCII hex digits — valid UTF-8 by construction
    std::str::from_utf8(&buf)
        .expect("hex digits are valid utf-8")
        .into()
}

struct TempFileGuard {
    path: PathBuf,
    armed: bool,
}

impl TempFileGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Stream a response body into a temporary download file.
pub(crate) async fn stream_download(
    response: reqwest::Response,
    output_path: &Path,
    content_length: Option<u64>,
    progress_callback: Option<Arc<dyn Fn(u64, u64) + Send + Sync>>,
    progress_timeout: Duration,
    mut cancel_rx: tokio::sync::watch::Receiver<bool>,
) -> Result<DownloadStreamResult> {
    let temp_path = temp_path_for(output_path);
    let Some(raw_file) = run_cancelable(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path),
        &cancel_rx,
    )
    .await
    else {
        return Ok(aborted_stream(temp_path, 0));
    };

    let mut file = raw_file?;
    let mut guard = TempFileGuard::new(temp_path.clone());
    let mut stream = response.bytes_stream();
    let mut downloaded = 0u64;
    let total = content_length.unwrap_or(0);
    let mut hash_worker = Some(HashWorker::new());
    let mut last_progress_bytes = 0u64;
    let mut last_progress_emitted = Instant::now();
    let mut last_progress_at = Instant::now();

    loop {
        if *cancel_rx.borrow_and_update() {
            abort_download(&mut hash_worker, &mut file, &temp_path).await;
            return Ok(aborted_stream(temp_path, downloaded));
        }

        let Some(maybe_chunk) =
            run_cancelable(timeout(progress_timeout, stream.next()), &cancel_rx).await
        else {
            abort_download(&mut hash_worker, &mut file, &temp_path).await;
            return Ok(aborted_stream(temp_path, downloaded));
        };

        let chunk = match maybe_chunk {
            Ok(Some(Ok(bytes))) => bytes,
            Ok(Some(Err(err))) => {
                abort_download(&mut hash_worker, &mut file, &temp_path).await;
                return Err(Error::network(err.to_string()));
            }
            Ok(None) => break,
            Err(_) => {
                abort_download(&mut hash_worker, &mut file, &temp_path).await;
                let stalled_for = last_progress_at.elapsed().as_secs();
                return Err(Error::network(format!(
                    "download stalled with no progress for {} seconds",
                    stalled_for.max(progress_timeout.as_secs())
                )));
            }
        };

        downloaded += chunk.len() as u64;

        let Some(write_result) = run_cancelable(file.write_all(&chunk), &cancel_rx).await else {
            abort_download(&mut hash_worker, &mut file, &temp_path).await;
            return Ok(aborted_stream(temp_path, downloaded));
        };
        if let Err(err) = write_result {
            abort_download(&mut hash_worker, &mut file, &temp_path).await;
            return Err(Error::io(err.to_string()));
        }

        if let Some(worker) = hash_worker.as_ref() {
            worker.update(chunk);
        }

        let delta = downloaded.saturating_sub(last_progress_bytes);
        if delta >= MIN_PROGRESS_DELTA {
            last_progress_at = Instant::now();
        }

        if let Some(ref callback) = progress_callback {
            let is_complete = total != 0 && downloaded >= total;
            if !is_complete
                && (delta >= MIN_PROGRESS_DELTA
                    || last_progress_emitted.elapsed() >= MIN_PROGRESS_INTERVAL)
            {
                callback(downloaded, total);
                last_progress_bytes = downloaded;
                last_progress_emitted = Instant::now();
            }
        }
    }

    if *cancel_rx.borrow() {
        abort_download(&mut hash_worker, &mut file, &temp_path).await;
        return Ok(aborted_stream(temp_path, downloaded));
    }

    flush_download(&mut file, &mut hash_worker, &temp_path, &cancel_rx).await?;

    let hash = match hash_worker.take() {
        Some(worker) => Some(worker.finalize().await?),
        None => None,
    };

    guard.disarm();

    Ok(DownloadStreamResult {
        aborted: false,
        hash,
        bytes_written: downloaded,
        temp_path,
    })
}

fn temp_path_for(output_path: &Path) -> PathBuf {
    let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = output_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download");
    let counter_s = counter.to_string();
    let mut tmp = String::with_capacity(name.len() + PID_STRING.len() + counter_s.len() + 14);
    tmp.push_str(name);
    tmp.push_str(".download-");
    tmp.push_str(&PID_STRING);
    tmp.push('-');
    tmp.push_str(&counter_s);
    tmp.push_str(".tmp");
    output_path.with_file_name(tmp)
}

fn aborted_stream(temp_path: PathBuf, bytes_written: u64) -> DownloadStreamResult {
    DownloadStreamResult {
        aborted: true,
        hash: None,
        bytes_written,
        temp_path,
    }
}

async fn flush_download(
    file: &mut tokio::fs::File,
    hash_worker: &mut Option<HashWorker>,
    temp_path: &Path,
    cancel_rx: &tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let Some(flush_result) = run_cancelable(file.flush(), cancel_rx).await else {
        abort_download(hash_worker, file, temp_path).await;
        return Err(Error::Cancelled);
    };
    if let Err(err) = flush_result {
        abort_download(hash_worker, file, temp_path).await;
        return Err(Error::io(err.to_string()));
    }

    let Some(sync_result) = run_cancelable(file.sync_all(), cancel_rx).await else {
        abort_download(hash_worker, file, temp_path).await;
        return Err(Error::Cancelled);
    };
    if let Err(err) = sync_result {
        abort_download(hash_worker, file, temp_path).await;
        return Err(Error::io(err.to_string()));
    }

    let Some(shutdown_result) = run_cancelable(file.shutdown(), cancel_rx).await else {
        abort_download(hash_worker, file, temp_path).await;
        return Err(Error::Cancelled);
    };
    if let Err(err) = shutdown_result {
        abort_download(hash_worker, file, temp_path).await;
        return Err(Error::io(err.to_string()));
    }

    Ok(())
}

async fn abort_download(
    hash_worker: &mut Option<HashWorker>,
    file: &mut tokio::fs::File,
    temp_path: &Path,
) {
    if let Some(worker) = hash_worker.take() {
        worker.abort();
    }
    let _ = file.shutdown().await;
    let _ = fs::remove_file(temp_path).await;
}

async fn run_cancelable<T>(
    future: impl Future<Output = T>,
    cancel_rx: &tokio::sync::watch::Receiver<bool>,
) -> Option<T> {
    let mut cancel_rx = cancel_rx.clone();
    tokio::select! {
        biased;
        _ = wait_until_cancelled(&mut cancel_rx) => None,
        result = future => Some(result),
    }
}

async fn wait_until_cancelled(cancel_rx: &mut tokio::sync::watch::Receiver<bool>) {
    loop {
        if *cancel_rx.borrow_and_update() {
            return;
        }
        if cancel_rx.changed().await.is_err() {
            pending::<()>().await;
        }
    }
}

#[cfg(test)]
#[path = "../tests/worker.rs"]
mod tests;
