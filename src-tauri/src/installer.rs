use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use crate::archive;

pub const DESKTOP_FILE: &str = "aseprite-compiler.aseprite.desktop";
pub const ICON_NAME: &str = "aseprite-compiler-aseprite";

pub fn aseprite_bin(install_root: &Path) -> PathBuf {
    install_root
        .join("current")
        .join(if cfg!(windows) { "aseprite.exe" } else { "aseprite" })
}

/// Copy the finished build (aseprite binary + data folder) into the install
/// root atomically: stage first, then swap, restoring the old build on failure.
/// Returns the installed size in bytes.
pub fn install_build(build_bin_dir: &Path, install_root: &Path) -> Result<u64> {
    let bin_name = if cfg!(windows) { "aseprite.exe" } else { "aseprite" };
    if !build_bin_dir.join(bin_name).is_file() {
        bail!(
            "build output not found at {}",
            build_bin_dir.join(bin_name).display()
        );
    }

    fs::create_dir_all(install_root)?;
    let staging = install_root.join(".staging");
    let current = install_root.join("current");
    let old = install_root.join(".previous");

    fs::remove_dir_all(&staging).ok();
    let mut installed_bytes =
        archive::copy_tree(build_bin_dir, &staging).context("staging new build")?;

    // The build's internal code-generator tool isn't part of Aseprite.
    for tool in ["gen", "gen.exe"] {
        let p = staging.join(tool);
        if let Ok(meta) = fs::metadata(&p) {
            installed_bytes = installed_bytes.saturating_sub(meta.len());
            fs::remove_file(&p).ok();
        }
    }

    fs::remove_dir_all(&old).ok();
    let had_current = current.exists();
    if had_current {
        fs::rename(&current, &old).context("moving previous build aside")?;
    }
    if let Err(e) = fs::rename(&staging, &current) {
        // Roll back so the previous install keeps working.
        if had_current {
            let _ = fs::rename(&old, &current);
        }
        return Err(e).context("activating new build");
    }
    fs::remove_dir_all(&old).ok();
    Ok(installed_bytes)
}

/// Register the compiled Aseprite in the OS application launcher.
/// `src_root` is the extracted Aseprite source tree (for the icons).
pub fn register_launcher(install_root: &Path, src_root: &Path, version: &str) -> Result<()> {
    #[cfg(target_os = "linux")]
    return register_linux(install_root, src_root, version);
    #[cfg(target_os = "windows")]
    return register_windows(install_root, version);
    #[allow(unreachable_code)]
    Ok(())
}

#[cfg(target_os = "linux")]
fn register_linux(install_root: &Path, src_root: &Path, version: &str) -> Result<()> {
    let data_home = dirs::data_dir().context("no data dir")?;

    // Icons from the Aseprite source tree into the hicolor theme.
    let icon_src = src_root.join("data").join("icons");
    let mut any_icon = false;
    for size in [16u32, 32, 48, 64, 128, 256] {
        let candidate = icon_src.join(format!("ase{size}.png"));
        if candidate.is_file() {
            let dir = data_home
                .join("icons/hicolor")
                .join(format!("{size}x{size}"))
                .join("apps");
            fs::create_dir_all(&dir)?;
            fs::copy(&candidate, dir.join(format!("{ICON_NAME}.png")))?;
            any_icon = true;
        }
    }

    let bin = aseprite_bin(install_root);
    let apps_dir = data_home.join("applications");
    fs::create_dir_all(&apps_dir)?;
    let desktop = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Aseprite\n\
         Comment=Animated sprite editor & pixel art tool (local build {version})\n\
         Exec=\"{bin}\" %U\n\
         Icon={icon}\n\
         Terminal=false\n\
         Categories=Graphics;2DGraphics;RasterGraphics;\n\
         MimeType=image/x-aseprite;\n\
         StartupWMClass=Aseprite\n",
        bin = bin.display(),
        icon = if any_icon { ICON_NAME } else { "image-x-generic" },
    );
    fs::write(apps_dir.join(DESKTOP_FILE), desktop)?;

    // Best-effort cache refreshes; launchers pick the entry up without them too.
    let _ = std::process::Command::new("update-desktop-database")
        .arg(&apps_dir)
        .status();
    let _ = std::process::Command::new("gtk-update-icon-cache")
        .args(["-q", "-t"])
        .arg(data_home.join("icons/hicolor"))
        .status();
    Ok(())
}

#[cfg(target_os = "windows")]
fn register_windows(install_root: &Path, _version: &str) -> Result<()> {
    let bin = aseprite_bin(install_root);
    let appdata = std::env::var("APPDATA").context("no APPDATA")?;
    let lnk = Path::new(&appdata)
        .join("Microsoft\\Windows\\Start Menu\\Programs\\Aseprite (local build).lnk");
    // Single-quoted PowerShell literals escape ' by doubling it — a path
    // containing an apostrophe (e.g. C:\Users\O'Brien) must not break out.
    let ps = |s: String| s.replace('\'', "''");
    let script = format!(
        "$ws = New-Object -ComObject WScript.Shell; \
         $s = $ws.CreateShortcut('{lnk}'); \
         $s.TargetPath = '{bin}'; \
         $s.WorkingDirectory = '{dir}'; \
         $s.IconLocation = '{bin},0'; \
         $s.Description = 'Aseprite (compiled locally)'; \
         $s.Save()",
        lnk = ps(lnk.display().to_string()),
        bin = ps(bin.display().to_string()),
        dir = ps(bin.parent().unwrap().display().to_string()),
    );
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status()
        .context("running powershell to create Start Menu shortcut")?;
    if !status.success() {
        bail!("could not create the Start Menu shortcut");
    }
    Ok(())
}

/// Remove the installed build and its launcher entry. Aseprite's own
/// preferences and user files are untouched.
pub fn uninstall(install_root: &Path) -> Result<()> {
    fs::remove_dir_all(install_root.join("current")).ok();
    fs::remove_dir_all(install_root.join(".staging")).ok();
    fs::remove_dir_all(install_root.join(".previous")).ok();
    fs::remove_dir(install_root).ok(); // only if now empty

    #[cfg(target_os = "linux")]
    {
        if let Some(data_home) = dirs::data_dir() {
            fs::remove_file(data_home.join("applications").join(DESKTOP_FILE)).ok();
            for size in [16u32, 32, 48, 64, 128, 256] {
                fs::remove_file(
                    data_home
                        .join("icons/hicolor")
                        .join(format!("{size}x{size}"))
                        .join("apps")
                        .join(format!("{ICON_NAME}.png")),
                )
                .ok();
            }
            let _ = std::process::Command::new("update-desktop-database")
                .arg(data_home.join("applications"))
                .status();
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            fs::remove_file(
                Path::new(&appdata)
                    .join("Microsoft\\Windows\\Start Menu\\Programs\\Aseprite (local build).lnk"),
            )
            .ok();
        }
    }

    Ok(())
}
