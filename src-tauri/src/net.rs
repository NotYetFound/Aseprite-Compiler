use anyhow::{bail, Context, Result};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

const USER_AGENT: &str = "aseprite-compiler (https://github.com)";

/// Shared HTTP agent with sane timeouts. Connect/response timeouts prevent a
/// dead connection from hanging a pipeline stage forever; there is no global
/// timeout so long downloads still work (stalls surface as read errors and
/// resume on retry).
fn agent() -> &'static ureq::Agent {
    use std::sync::OnceLock;
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_connect(Some(std::time::Duration::from_secs(15)))
            .timeout_recv_response(Some(std::time::Duration::from_secs(60)))
            // Hard ceiling so a wedged connection can't pin a stage forever;
            // generous enough for the largest download on a slow line.
            .timeout_recv_body(Some(std::time::Duration::from_secs(30 * 60)))
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

/// Download `url` to `dest`, resuming a partial `.part` file when the server
/// supports ranges. Progress reports (downloaded, total) in bytes.
pub fn download_with_resume(
    url: &str,
    dest: &Path,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<()> {
    if dest.exists() {
        return Ok(());
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    let part = dest.with_file_name(format!(
        "{}.part",
        dest.file_name().unwrap().to_string_lossy()
    ));
    let existing = part.metadata().map(|m| m.len()).unwrap_or(0);

    let mut req = agent().get(url).header("User-Agent", USER_AGENT);
    if existing > 0 {
        req = req.header("Range", format!("bytes={existing}-"));
    }
    let mut resp = match req.call() {
        Ok(resp) => resp,
        Err(e) if existing > 0 => {
            // A stale .part can make the Range unsatisfiable (e.g. it already
            // holds the full file → 416). Drop it and start clean once.
            fs::remove_file(&part).ok();
            let _ = e;
            agent()
                .get(url)
                .header("User-Agent", USER_AGENT)
                .call()
                .with_context(|| format!("download {url}"))?
        }
        Err(e) => return Err(e).with_context(|| format!("download {url}")),
    };

    let status = resp.status().as_u16();
    let (mut file, mut downloaded) = if status == 206 && existing > 0 {
        let f = OpenOptions::new().append(true).open(&part)?;
        (f, existing)
    } else {
        // Server ignored the range (or fresh download): start over.
        let f = File::create(&part)?;
        (f, 0u64)
    };

    let total = if status == 206 {
        // Content-Range: bytes start-end/total
        resp.headers()
            .get("Content-Range")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit('/').next())
            .and_then(|v| v.parse::<u64>().ok())
    } else {
        resp.headers()
            .get("Content-Length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
    };

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

    if let Some(t) = total {
        if downloaded < t {
            bail!("download ended early: {downloaded} of {t} bytes (will resume on retry)");
        }
    }

    fs::rename(&part, dest)?;
    Ok(())
}
