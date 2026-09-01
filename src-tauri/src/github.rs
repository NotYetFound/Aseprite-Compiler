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
    /// Lowercase hex SHA-256 of the source zip, when GitHub publishes one.
    #[serde(default)]
    pub source_zip_sha256: Option<String>,
}

/// A resolved Skia package for one platform.
#[derive(Debug, Clone)]
pub struct SkiaAsset {
    pub tag: String,
    pub asset_name: String,
    pub url: String,
    pub sha256: Option<String>,
}

/// GitHub asset "digest" fields look like "sha256:<hex>".
fn asset_sha256(asset: &serde_json::Value) -> Option<String> {
    asset["digest"]
        .as_str()?
        .strip_prefix("sha256:")
        .map(|h| h.to_ascii_lowercase())
}

fn find_source_asset(release: &serde_json::Value) -> Option<(String, u64, Option<String>)> {
    release["assets"].as_array()?.iter().find_map(|a| {
        let name = a["name"].as_str()?;
        if name.ends_with("-Source.zip") {
            Some((
                a["browser_download_url"].as_str()?.to_string(),
                a["size"].as_u64().unwrap_or(0),
                asset_sha256(a),
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
    let (source_zip_url, source_zip_size, source_zip_sha256) = find_source_asset(release)
        .ok_or_else(|| anyhow!("release {tag} has no -Source.zip asset"))?;
    Ok(ReleaseInfo {
        version: tag.trim_start_matches('v').to_string(),
        name: release["name"].as_str().unwrap_or(&tag).to_string(),
        tag,
        source_zip_url,
        source_zip_size,
        source_zip_sha256,
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

fn platform_skia_asset_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "Skia-Windows-Release-x64.zip"
    } else {
        "Skia-Linux-Release-x64.zip"
    }
}

fn skia_from_release(release: &serde_json::Value) -> Result<SkiaAsset> {
    let tag = release["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow!("skia release has no tag"))?
        .to_string();
    let want = platform_skia_asset_name();
    let assets = release["assets"]
        .as_array()
        .ok_or_else(|| anyhow!("skia release {tag} has no assets"))?;
    let asset = assets
        .iter()
        .find(|a| a["name"].as_str() == Some(want))
        .ok_or_else(|| anyhow!("skia release {tag} has no {want} asset"))?;
    Ok(SkiaAsset {
        tag,
        asset_name: want.to_string(),
        url: asset["browser_download_url"]
            .as_str()
            .ok_or_else(|| anyhow!("asset has no url"))?
            .to_string(),
        sha256: asset_sha256(asset),
    })
}

/// Resolve exactly the pinned Skia release. A pinned dependency is never
/// silently substituted: any failure here is a hard error, not a fallback.
pub fn skia_asset_exact(tag: &str) -> Result<SkiaAsset> {
    let release = net::http_get_json(&format!(
        "https://api.github.com/repos/{SKIA_REPO}/releases/tags/{tag}"
    ))
    .with_context(|| {
        format!(
            "the Aseprite source pins Skia {tag}, but that release could not be \
             resolved from {SKIA_REPO} — refusing to substitute a different Skia version"
        )
    })?;
    skia_from_release(&release)
}

/// Latest Skia release — only for sources that carry no pin at all.
pub fn skia_asset_latest() -> Result<SkiaAsset> {
    let release = net::http_get_json(&format!(
        "https://api.github.com/repos/{SKIA_REPO}/releases/latest"
    ))
    .context("fetching latest Skia release")?;
    skia_from_release(&release)
}
