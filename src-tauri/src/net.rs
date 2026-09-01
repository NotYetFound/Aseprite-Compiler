use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const USER_AGENT: &str = "aseprite-compiler (https://github.com/NotYetFound/Aseprite-Compiler)";
const MAX_ATTEMPTS: u32 = 3;

/// Shared HTTP agent with sane timeouts. Connect/response timeouts prevent a
/// dead connection from hanging a pipeline stage forever; the body ceiling is
/// generous enough for the largest download on a slow line while still
/// guaranteeing a wedged connection eventually errors (and resumes on retry).
fn agent() -> &'static ureq::Agent {
    use std::sync::OnceLock;
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .timeout_recv_body(Some(Duration::from_secs(30 * 60)))
            .build()
            .new_agent()
    })
}

/// Marker error for user-initiated cancellation; checked with `is_cancelled`.
#[derive(Debug)]
pub struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cancelled")
    }
}

impl std::error::Error for Cancelled {}

pub fn is_cancelled(err: &anyhow::Error) -> bool {
    err.downcast_ref::<Cancelled>().is_some()
}

/// Marker error for integrity failures — never automatically retried.
#[derive(Debug)]
pub struct IntegrityError(pub String);

impl fmt::Display for IntegrityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "integrity check failed: {}", self.0)
    }
}

impl std::error::Error for IntegrityError {}

pub fn http_get_string(url: &str) -> Result<String> {
    let mut resp = agent()
        .get(url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call()
        .with_context(|| format!("GET {url}"))?;
    Ok(resp.body_mut().read_to_string()?)
}

pub fn http_get_json(url: &str) -> Result<serde_json::Value> {
    let text = http_get_string(url)?;
    Ok(serde_json::from_str(&text)?)
}

/// What a download is expected to be. Size and digest are verified when
/// known — a cached or downloaded file that doesn't match is discarded.
#[derive(Debug, Clone, Default)]
pub struct Expected {
    pub size: Option<u64>,
    /// Lowercase hex SHA-256.
    pub sha256: Option<String>,
}

/// Sidecar metadata binding a partial download to the remote object it came
/// from, so a resume can never splice bytes from two different objects.
#[derive(Serialize, Deserialize)]
struct PartMeta {
    url: String,
    etag: Option<String>,
    last_modified: Option<String>,
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Is an existing complete file exactly what we expect?
fn validate_existing(path: &Path, expected: &Expected) -> bool {
    let Ok(meta) = path.metadata() else {
        return false;
    };
    if !meta.is_file() || meta.len() == 0 {
        return false;
    }
    if let Some(size) = expected.size {
        if meta.len() != size {
            return false;
        }
    }
    if let Some(want) = &expected.sha256 {
        match sha256_file(path) {
            Ok(actual) => {
                if !actual.eq_ignore_ascii_case(want) {
                    return false;
                }
            }
            Err(_) => return false,
        }
    }
    true
}

/// Cancellation-aware backoff sleep.
fn backoff(cancel: &AtomicBool, duration: Duration) -> Result<()> {
    let mut remaining = duration;
    while remaining > Duration::ZERO {
        if cancel.load(Ordering::Relaxed) {
            bail!(Cancelled);
        }
        let step = remaining.min(Duration::from_millis(250));
        std::thread::sleep(step);
        remaining = remaining.saturating_sub(step);
    }
    Ok(())
}

fn is_retryable(err: &anyhow::Error) -> bool {
    if is_cancelled(err) || err.downcast_ref::<IntegrityError>().is_some() {
        return false;
    }
    match err.downcast_ref::<ureq::Error>() {
        // Retry transient server-side conditions; never client errors like
        // 404 (a pinned artifact that doesn't exist won't appear on retry).
        Some(ureq::Error::StatusCode(code)) => matches!(code, 408 | 429 | 500 | 502 | 503 | 504),
        // Transport-level failures (reset, timeout, DNS) are worth retrying.
        Some(_) => true,
        None => false,
    }
}

/// Download `url` to `dest`, verified against `expected`. Retries transient
/// failures with backoff, resumes partial downloads safely (ETag-bound via
/// If-Range), and only ever publishes `dest` after verification passes.
pub fn download_verified(
    url: &str,
    dest: &Path,
    expected: &Expected,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<()> {
    // A pre-existing file counts only if it verifies; never trust bare paths.
    if dest.exists() {
        if validate_existing(dest, expected) {
            return Ok(());
        }
        fs::remove_file(dest).ok();
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut attempt = 0u32;
    loop {
        attempt += 1;
        match download_once(url, dest, expected, cancel, &mut on_progress) {
            Ok(()) => return Ok(()),
            Err(e) if attempt < MAX_ATTEMPTS && is_retryable(&e) => {
                backoff(cancel, Duration::from_secs(2 * u64::from(attempt) + 1))?;
            }
            Err(e) => return Err(e),
        }
    }
}

fn part_paths(dest: &Path) -> (PathBuf, PathBuf) {
    let name = dest.file_name().unwrap().to_string_lossy();
    (
        dest.with_file_name(format!("{name}.part")),
        dest.with_file_name(format!("{name}.part.meta")),
    )
}

fn discard_partial(part: &Path, meta: &Path) {
    fs::remove_file(part).ok();
    fs::remove_file(meta).ok();
}

/// "bytes 123-999/1000" → (start, total)
fn parse_content_range(value: &str) -> Option<(u64, Option<u64>)> {
    let rest = value.trim().strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let (start, _) = range.split_once('-')?;
    let start = start.trim().parse::<u64>().ok()?;
    let total = total.trim().parse::<u64>().ok();
    Some((start, total))
}

fn header(resp: &ureq::http::Response<ureq::Body>, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

fn download_once(
    url: &str,
    dest: &Path,
    expected: &Expected,
    cancel: &AtomicBool,
    on_progress: &mut impl FnMut(u64, Option<u64>),
) -> Result<()> {
    let (part, meta_path) = part_paths(dest);

    // Resume only when the partial is bound to a known remote identity.
    let existing_meta: Option<PartMeta> = fs::read_to_string(&meta_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    let existing_len = part.metadata().map(|m| m.len()).unwrap_or(0);
    let resumable = existing_len > 0
        && existing_meta
            .as_ref()
            .map(|m| {
                m.url == url && (m.etag.is_some() || m.last_modified.is_some())
            })
            .unwrap_or(false);
    if existing_len > 0 && !resumable {
        discard_partial(&part, &meta_path);
    }

    let mut req = agent().get(url).header("User-Agent", USER_AGENT);
    if resumable {
        let m = existing_meta.as_ref().unwrap();
        req = req.header("Range", format!("bytes={existing_len}-"));
        if let Some(etag) = &m.etag {
            req = req.header("If-Range", etag.clone());
        } else if let Some(lm) = &m.last_modified {
            req = req.header("If-Range", lm.clone());
        }
    }

    let mut resp = match req.call() {
        Ok(resp) => resp,
        Err(e) => {
            // A stale partial can make the Range unsatisfiable (416); drop it
            // so the retry starts clean.
            if resumable && matches!(e, ureq::Error::StatusCode(416)) {
                discard_partial(&part, &meta_path);
            }
            return Err(anyhow::Error::new(e)).with_context(|| format!("download {url}"));
        }
    };

    let status = resp.status().as_u16();
    let (mut file, mut downloaded) = if status == 206 && resumable {
        // The server honored the range — but only append if it resumes at
        // exactly our partial length for the same object.
        let start = header(&resp, "Content-Range")
            .as_deref()
            .and_then(parse_content_range)
            .map(|(s, _)| s);
        if start != Some(existing_len) {
            discard_partial(&part, &meta_path);
            bail!("server resumed at an unexpected offset (will restart)");
        }
        (OpenOptions::new().append(true).open(&part)?, existing_len)
    } else {
        // Fresh (or restarted) download: record the remote identity so a
        // future resume is bound to this exact object.
        let meta = PartMeta {
            url: url.to_string(),
            etag: header(&resp, "ETag"),
            last_modified: header(&resp, "Last-Modified"),
        };
        fs::write(&meta_path, serde_json::to_string(&meta)?)?;
        (File::create(&part)?, 0u64)
    };

    let total = if status == 206 {
        header(&resp, "Content-Range")
            .as_deref()
            .and_then(parse_content_range)
            .and_then(|(_, t)| t)
    } else {
        header(&resp, "Content-Length").and_then(|v| v.parse::<u64>().ok())
    }
    .or(expected.size);

    let mut reader = resp.body_mut().as_reader();
    let mut buf = [0u8; 65536];
    loop {
        if cancel.load(Ordering::Relaxed) {
            bail!(Cancelled);
        }
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        downloaded += n as u64;
        on_progress(downloaded, total);
    }
    file.flush()?;
    drop(file);

    // Completion requires the exact expected size when one is known.
    if let Some(t) = total {
        if downloaded != t {
            bail!("download ended early: {downloaded} of {t} bytes (will resume on retry)");
        }
    }
    if let Some(size) = expected.size {
        let actual = part.metadata().map(|m| m.len()).unwrap_or(0);
        if actual != size {
            discard_partial(&part, &meta_path);
            bail!(IntegrityError(format!(
                "size mismatch: expected {size} bytes, got {actual}"
            )));
        }
    }
    if let Some(want) = &expected.sha256 {
        let actual = sha256_file(&part)?;
        if !actual.eq_ignore_ascii_case(want) {
            discard_partial(&part, &meta_path);
            bail!(IntegrityError(format!(
                "sha256 mismatch: expected {want}, got {actual}"
            )));
        }
    }

    fs::rename(&part, dest)?;
    fs::remove_file(&meta_path).ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_range_parses() {
        assert_eq!(
            parse_content_range("bytes 100-999/1000"),
            Some((100, Some(1000)))
        );
        assert_eq!(parse_content_range("bytes 0-0/*"), Some((0, None)));
        assert_eq!(parse_content_range("garbage"), None);
    }
}
