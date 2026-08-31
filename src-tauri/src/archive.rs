use anyhow::{bail, Context, Result};
use std::fs::{self, File};
use std::io::{self, BufReader};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::net::Cancelled;

/// Extract a zip archive into `dest`, preserving unix permissions.
/// Progress reports (entries_done, entries_total).
pub fn extract_zip(
    archive: &Path,
    dest: &Path,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(usize, usize),
) -> Result<()> {
    let file = File::open(archive).with_context(|| format!("opening {}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(BufReader::new(file))?;
    let total = zip.len();
    fs::create_dir_all(dest)?;

    for i in 0..total {
        if cancel.load(Ordering::Relaxed) {
            bail!(Cancelled);
        }
        let mut entry = zip.by_index(i)?;
        let Some(rel) = entry.enclosed_name() else {
            continue; // skip unsafe paths
        };
        let out = dest.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut f = File::create(&out)?;
            io::copy(&mut entry, &mut f)?;
            #[cfg(unix)]
            if let Some(mode) = entry.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&out, fs::Permissions::from_mode(mode));
            }
        }
        if i % 50 == 0 || i + 1 == total {
            on_progress(i + 1, total);
        }
    }
    Ok(())
}

/// Extract a .tar.gz archive into `dest` (used for the portable CMake package).
pub fn extract_tar_gz(archive: &Path, dest: &Path, cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        bail!(Cancelled);
    }
    let file = File::open(archive).with_context(|| format!("opening {}", archive.display()))?;
    let gz = flate2::read::GzDecoder::new(BufReader::new(file));
    let mut tar = tar::Archive::new(gz);
    fs::create_dir_all(dest)?;
    tar.unpack(dest)?;
    Ok(())
}

/// Total size in bytes of a file or directory tree.
pub fn dir_size(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    let mut sum = 0;
    if let Ok(entries) = fs::read_dir(path) {
        for e in entries.flatten() {
            sum += dir_size(&e.path());
        }
    }
    sum
}

/// Copy a directory tree (follows the source's structure exactly).
pub fn copy_tree(from: &Path, to: &Path) -> Result<u64> {
    let mut copied = 0u64;
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest = to.join(entry.file_name());
        if ty.is_dir() {
            copied += copy_tree(&entry.path(), &dest)?;
        } else if ty.is_file() {
            fs::copy(entry.path(), &dest)?;
            copied += entry.metadata().map(|m| m.len()).unwrap_or(0);
        }
        #[cfg(unix)]
        if ty.is_symlink() {
            use std::os::unix::fs::symlink;
            if let Ok(target) = fs::read_link(entry.path()) {
                let _ = symlink(target, &dest);
            }
        }
    }
    Ok(copied)
}
