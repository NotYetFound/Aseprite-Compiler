//! "Export diagnostics": bundle everything needed to debug a failed build
//! into one zip — settings, state, tool health, and the persisted build logs.

use anyhow::{Context, Result};
use std::io::Write;
use std::path::PathBuf;

use crate::state::{now_millis, utc_stamp};
use crate::{paths, toolchain};

pub fn export(app_version: &str) -> Result<PathBuf> {
    let stamp = utc_stamp(now_millis());
    let dir = dirs::download_dir().unwrap_or_else(paths::data_dir);
    std::fs::create_dir_all(&dir).ok();
    let dest = dir.join(format!("aseprite-compiler-diagnostics-{stamp}.zip"));

    let file = std::fs::File::create(&dest)
        .with_context(|| format!("creating {}", dest.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let info = format!(
        "Aseprite Compiler {app_version}\nexported: {stamp} UTC\nos: {} {}\n",
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    zip.start_file("info.txt", opts)?;
    zip.write_all(info.as_bytes())?;

    // Tool health as the app sees it right now.
    zip.start_file("tools.json", opts)?;
    zip.write_all(serde_json::to_string_pretty(&toolchain::check())?.as_bytes())?;

    for (name, path) in [
        ("settings.json", paths::settings_file()),
        ("state.json", paths::state_file()),
    ] {
        if let Ok(text) = std::fs::read_to_string(&path) {
            zip.start_file(name, opts)?;
            zip.write_all(text.as_bytes())?;
        }
    }

    // All rotated build logs (at most 10).
    if let Ok(entries) = std::fs::read_dir(paths::logs_dir()) {
        for entry in entries.flatten() {
            let p = entry.path();
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with(".log") {
                continue;
            }
            if let Ok(bytes) = std::fs::read(&p) {
                zip.start_file(format!("logs/{name}"), opts)?;
                zip.write_all(&bytes)?;
            }
        }
    }

    zip.finish()?;
    Ok(dest)
}
