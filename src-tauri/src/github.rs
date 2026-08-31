use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::net;
use crate::settings::Channel;

pub const ASEPRITE_REPO: &str = "aseprite/aseprite";
pub const SKIA_REPO: &str = "aseprite/skia";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseInfo {
    pub tag: String,
    pub version: String,
    pub name: String,
    pub source_zip_url: String,
    pub source_zip_size: u64,
}

fn find_source_asset(release: &serde_json::Value) -> Option<(String, u64)> {
    release["assets"].as_array()?.iter().find_map(|a| {
        let name = a["name"].as_str()?;
        if name.ends_with("-Source.zip") {
            Some((
                a["browser_download_url"].as_str()?.to_string(),
                a["size"].as_u64().unwrap_or(0),
            ))
        } else {
            None
        }
    })
}

fn to_release_info(release: &serde_json::Value) -> Result<ReleaseInfo> {
    let tag = release["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow!("release has no tag"))?
        .to_string();
    let (source_zip_url, source_zip_size) = find_source_asset(release)
        .ok_or_else(|| anyhow!("release {tag} has no -Source.zip asset"))?;
    Ok(ReleaseInfo {
        version: tag.trim_start_matches('v').to_string(),
        name: release["name"].as_str().unwrap_or(&tag).to_string(),
        tag,
        source_zip_url,
        source_zip_size,
    })
}

/// Latest Aseprite release for the channel. Stable uses the `latest` endpoint
/// (never a pre-release); beta takes the newest release including pre-releases.
pub fn fetch_latest(channel: Channel) -> Result<ReleaseInfo> {
    match channel {
        Channel::Stable => {
            let v = net::http_get_json(&format!(
                "https://api.github.com/repos/{ASEPRITE_REPO}/releases/latest"
            ))
            .context("fetching latest Aseprite release")?;
            to_release_info(&v)
        }
        Channel::Beta => {
            let v = net::http_get_json(&format!(
                "https://api.github.com/repos/{ASEPRITE_REPO}/releases?per_page=10"
            ))
            .context("fetching Aseprite releases")?;
            let releases = v.as_array().ok_or_else(|| anyhow!("unexpected response"))?;
            let newest = releases
                .iter()
                .find(|r| !r["draft"].as_bool().unwrap_or(false))
                .ok_or_else(|| anyhow!("no releases found"))?;
            to_release_info(newest)
        }
    }
}

/// Resolve the Skia package download URL for `tag` (e.g. "m124-08a5439a6b").
/// Returns (tag, asset_name, url).
pub fn skia_asset(tag: Option<&str>) -> Result<(String, String, String)> {
    let release = match tag {
        Some(t) => net::http_get_json(&format!(
            "https://api.github.com/repos/{SKIA_REPO}/releases/tags/{t}"
        ))
        .or_else(|_| {
            // Pinned tag not found — fall back to the latest Skia release.
            net::http_get_json(&format!(
                "https://api.github.com/repos/{SKIA_REPO}/releases/latest"
            ))
        })?,
        None => net::http_get_json(&format!(
            "https://api.github.com/repos/{SKIA_REPO}/releases/latest"
        ))?,
    };

    let resolved_tag = release["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow!("skia release has no tag"))?
        .to_string();

    let assets = release["assets"]
        .as_array()
        .ok_or_else(|| anyhow!("skia release has no assets"))?;

    let candidates: &[&str] = if cfg!(target_os = "windows") {
        &["Skia-Windows-Release-x64.zip"]
    } else {
        // The standard Linux package (built with clang against libc++);
        // the build configures Aseprite with matching flags.
        &["Skia-Linux-Release-x64.zip"]
    };

    for want in candidates {
        if let Some(a) = assets.iter().find(|a| a["name"].as_str() == Some(want)) {
            return Ok((
                resolved_tag,
                want.to_string(),
                a["browser_download_url"]
                    .as_str()
                    .ok_or_else(|| anyhow!("asset has no url"))?
                    .to_string(),
            ));
        }
    }
    Err(anyhow!(
        "no matching Skia package found in release {resolved_tag}"
    ))
}
