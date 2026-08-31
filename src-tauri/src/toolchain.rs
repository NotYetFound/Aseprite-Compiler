use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

use crate::{archive, net, paths};

const NINJA_VERSION: &str = "1.12.1";
const CMAKE_VERSION: &str = "3.31.6";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    pub id: String,
    pub name: String,
    pub ok: bool,
    pub detail: String,
    pub provisionable: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolReport {
    pub tools: Vec<ToolStatus>,
    pub all_ok: bool,
    pub helper_command: Option<String>,
    pub helper_label: Option<String>,
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Search PATH for an executable (we avoid shelling out to `which`).
pub fn find_in_path(name: &str) -> Option<PathBuf> {
    let name = exe(name);
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let full = dir.join(&name);
            full.is_file().then_some(full)
        })
    })
}

/// Portable ninja managed by this app.
pub fn portable_ninja() -> PathBuf {
    paths::tools_dir().join("ninja").join(exe("ninja"))
}

/// Portable cmake managed by this app.
pub fn portable_cmake() -> PathBuf {
    paths::tools_dir().join("cmake").join("bin").join(exe("cmake"))
}

/// Resolve the ninja to use: portable first, then system.
pub fn ninja_path() -> Option<PathBuf> {
    let p = portable_ninja();
    if p.is_file() {
        return Some(p);
    }
    find_in_path("ninja")
}

pub fn cmake_path() -> Option<PathBuf> {
    let p = portable_cmake();
    if p.is_file() {
        return Some(p);
    }
    find_in_path("cmake")
}

fn version_of(bin: &Path, arg: &str) -> String {
    Command::new(bin)
        .arg(arg)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.lines().next().unwrap_or("").trim().to_string())
        .unwrap_or_default()
}

#[cfg(target_os = "linux")]
fn pkg_config_ok(pkg: &str) -> Option<bool> {
    let pc = find_in_path("pkg-config").or_else(|| find_in_path("pkgconf"))?;
    Some(
        Command::new(pc)
            .args(["--exists", pkg])
            .status()
            .map(|s| s.success())
            .unwrap_or(false),
    )
}

#[cfg(target_os = "windows")]
pub fn find_msvc() -> Option<PathBuf> {
    let vswhere = PathBuf::from(std::env::var_os("ProgramFiles(x86)")?)
        .join("Microsoft Visual Studio")
        .join("Installer")
        .join("vswhere.exe");
    if !vswhere.is_file() {
        return None;
    }
    let out = Command::new(vswhere)
        .args([
            "-latest",
            "-products",
            "*",
            "-requires",
            "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
            "-property",
            "installationPath",
        ])
        .output()
        .ok()?;
    let path = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

#[cfg(target_os = "windows")]
pub fn vcvars64() -> Option<PathBuf> {
    let vs = find_msvc()?;
    let bat = vs.join("VC").join("Auxiliary").join("Build").join("vcvars64.bat");
    bat.is_file().then_some(bat)
}

/// Compiler pair to use on Linux: (cc, cxx). Aseprite's prebuilt Skia links
/// against the default libstdc++, so either clang or gcc works; clang is
/// preferred per the Aseprite docs.
#[cfg(target_os = "linux")]
pub fn linux_compiler() -> Option<(PathBuf, PathBuf)> {
    if let (Some(cc), Some(cxx)) = (find_in_path("clang"), find_in_path("clang++")) {
        return Some((cc, cxx));
    }
    if let (Some(cc), Some(cxx)) = (find_in_path("gcc"), find_in_path("g++")) {
        return Some((cc, cxx));
    }
    None
}

fn linux_distro_id() -> String {
    std::fs::read_to_string("/etc/os-release")
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix("ID=").map(|v| v.trim_matches('"').to_string()))
        .unwrap_or_default()
}

pub fn check() -> ToolReport {
    let mut tools = Vec::new();
    let mut missing_system = false;

    // Portable tools
    match cmake_path() {
        Some(p) => {
            let portable = p == portable_cmake();
            tools.push(ToolStatus {
                id: "cmake".into(),
                name: "CMake".into(),
                ok: true,
                detail: format!(
                    "{}{}",
                    version_of(&p, "--version"),
                    if portable { " · portable" } else { "" }
                ),
                provisionable: true,
            });
        }
        None => tools.push(ToolStatus {
            id: "cmake".into(),
            name: "CMake".into(),
            ok: false,
            detail: "Not found — the app can install a portable copy".into(),
            provisionable: true,
        }),
    }

    match ninja_path() {
        Some(p) => {
            let portable = p == portable_ninja();
            tools.push(ToolStatus {
                id: "ninja".into(),
                name: "Ninja".into(),
                ok: true,
                detail: format!(
                    "ninja {}{}",
                    version_of(&p, "--version"),
                    if portable { " · portable" } else { "" }
                ),
                provisionable: true,
            });
        }
        None => tools.push(ToolStatus {
            id: "ninja".into(),
            name: "Ninja".into(),
            ok: false,
            detail: "Not found — the app can install a portable copy".into(),
            provisionable: true,
        }),
    }

    #[cfg(target_os = "linux")]
    {
        match linux_compiler() {
            Some((_, cxx)) => tools.push(ToolStatus {
                id: "compiler".into(),
                name: "C++ compiler".into(),
                ok: true,
                detail: version_of(&cxx, "--version"),
                provisionable: false,
            }),
            None => {
                missing_system = true;
                tools.push(ToolStatus {
                    id: "compiler".into(),
                    name: "C++ compiler".into(),
                    ok: false,
                    detail: "clang (preferred) or g++ is required".into(),
                    provisionable: false,
                });
            }
        }

        let have_pkg_config =
            find_in_path("pkg-config").is_some() || find_in_path("pkgconf").is_some();
        if !have_pkg_config {
            missing_system = true;
            tools.push(ToolStatus {
                id: "pkg-config".into(),
                name: "pkg-config".into(),
                ok: false,
                detail: "needed to locate system libraries".into(),
                provisionable: false,
            });
        }

        for (pkg, label) in [
            ("x11", "X11 headers"),
            ("xcursor", "Xcursor headers"),
            ("xi", "Xi headers"),
            ("xrandr", "Xrandr headers"),
            ("gl", "OpenGL headers"),
            ("fontconfig", "Fontconfig headers"),
        ] {
            let ok = pkg_config_ok(pkg).unwrap_or(false);
            if !ok {
                missing_system = true;
            }
            tools.push(ToolStatus {
                id: format!("hdr-{pkg}"),
                name: label.into(),
                ok,
                detail: if ok {
                    format!("{pkg}.pc found")
                } else if have_pkg_config {
                    format!("development package for “{pkg}” not found")
                } else {
                    "cannot check without pkg-config".into()
                },
                provisionable: false,
            });
        }
    }

    #[cfg(target_os = "windows")]
    {
        match find_msvc() {
            Some(p) => tools.push(ToolStatus {
                id: "compiler".into(),
                name: "Visual Studio Build Tools".into(),
                ok: true,
                detail: p.display().to_string(),
                provisionable: false,
            }),
            None => {
                missing_system = true;
                tools.push(ToolStatus {
                    id: "compiler".into(),
                    name: "Visual Studio Build Tools".into(),
                    ok: false,
                    detail: "VS 2022 with “Desktop development with C++” is required".into(),
                    provisionable: false,
                });
            }
        }
    }

    let (helper_command, helper_label) = if missing_system {
        helper_command()
    } else {
        (None, None)
    };

    ToolReport {
        all_ok: tools.iter().all(|t| t.ok),
        tools,
        helper_command,
        helper_label,
    }
}

fn helper_command() -> (Option<String>, Option<String>) {
    #[cfg(target_os = "linux")]
    {
        let id = linux_distro_id();
        let (cmd, label) = match id.as_str() {
            "arch" | "cachyos" | "endeavouros" | "manjaro" | "garuda" => (
                "sudo pacman -S --needed clang pkgconf libx11 libxcursor libxi libxrandr mesa fontconfig",
                "Arch-based system detected. Run this in a terminal:",
            ),
            "debian" | "ubuntu" | "linuxmint" | "pop" => (
                "sudo apt install clang pkg-config libx11-dev libxcursor-dev libxi-dev libxrandr-dev libgl1-mesa-dev libfontconfig1-dev",
                "Debian/Ubuntu-based system detected. Run this in a terminal:",
            ),
            "fedora" | "nobara" => (
                "sudo dnf install clang pkgconf-pkg-config libX11-devel libXcursor-devel libXi-devel libXrandr-devel mesa-libGL-devel fontconfig-devel",
                "Fedora-based system detected. Run this in a terminal:",
            ),
            "opensuse-tumbleweed" | "opensuse-leap" => (
                "sudo zypper install clang pkgconf-pkg-config libX11-devel libXcursor-devel libXi-devel libXrandr-devel Mesa-libGL-devel fontconfig-devel",
                "openSUSE detected. Run this in a terminal:",
            ),
            _ => (
                "clang (or g++), pkg-config, and development headers for: X11, Xcursor, Xi, Xrandr, OpenGL, Fontconfig",
                "Install these with your distribution's package manager:",
            ),
        };
        (Some(cmd.to_string()), Some(label.to_string()))
    }
    #[cfg(target_os = "windows")]
    {
        (
            Some(
                "winget install --id Microsoft.VisualStudio.2022.BuildTools --override \"--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended --passive\""
                    .to_string(),
            ),
            Some("Run this in a terminal (or install Visual Studio 2022 with the C++ workload):".to_string()),
        )
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        (None, None)
    }
}

fn ninja_url() -> String {
    let file = if cfg!(windows) { "ninja-win.zip" } else { "ninja-linux.zip" };
    format!("https://github.com/ninja-build/ninja/releases/download/v{NINJA_VERSION}/{file}")
}

fn cmake_url() -> (String, String) {
    if cfg!(windows) {
        let name = format!("cmake-{CMAKE_VERSION}-windows-x86_64");
        (
            format!("https://github.com/Kitware/CMake/releases/download/v{CMAKE_VERSION}/{name}.zip"),
            name,
        )
    } else {
        let name = format!("cmake-{CMAKE_VERSION}-linux-x86_64");
        (
            format!("https://github.com/Kitware/CMake/releases/download/v{CMAKE_VERSION}/{name}.tar.gz"),
            name,
        )
    }
}

/// Download portable copies of any missing tools into the app's own tools dir.
/// Everything stays inside the app's data folder; nothing touches the system.
pub fn provision(cancel: &AtomicBool, mut log: impl FnMut(String)) -> Result<()> {
    paths::ensure_dirs()?;
    let no_cancel = AtomicBool::new(false);
    let _ = &no_cancel;

    if ninja_path().is_none() {
        log(format!("Downloading portable Ninja {NINJA_VERSION}…"));
        let archive_path = paths::cache_dir().join("ninja.zip");
        net::download_with_resume(&ninja_url(), &archive_path, cancel, |_, _| {})?;
        let dest = paths::tools_dir().join("ninja");
        archive::extract_zip(&archive_path, &dest, cancel, |_, _| {})?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(
                portable_ninja(),
                std::fs::Permissions::from_mode(0o755),
            );
        }
        std::fs::remove_file(&archive_path).ok();
        if !portable_ninja().is_file() {
            bail!("ninja extraction failed");
        }
        log("Portable Ninja installed.".into());
    }

    if cmake_path().is_none() {
        let (url, root_name) = cmake_url();
        log(format!("Downloading portable CMake {CMAKE_VERSION}…"));
        let file_name = url.rsplit('/').next().unwrap().to_string();
        let archive_path = paths::cache_dir().join(&file_name);
        net::download_with_resume(&url, &archive_path, cancel, |_, _| {})?;

        let extract_root = paths::tools_dir().join("cmake-extract");
        std::fs::remove_dir_all(&extract_root).ok();
        if file_name.ends_with(".zip") {
            archive::extract_zip(&archive_path, &extract_root, cancel, |_, _| {})?;
        } else {
            archive::extract_tar_gz(&archive_path, &extract_root, cancel)?;
        }

        let final_dir = paths::tools_dir().join("cmake");
        std::fs::remove_dir_all(&final_dir).ok();
        std::fs::rename(extract_root.join(&root_name), &final_dir)
            .context("moving extracted cmake into place")?;
        std::fs::remove_dir_all(&extract_root).ok();
        std::fs::remove_file(&archive_path).ok();
        if !portable_cmake().is_file() {
            bail!("cmake extraction failed");
        }
        log("Portable CMake installed.".into());
    }

    Ok(())
}

/// Best-effort read of the installed Aseprite's own version.
pub fn probe_aseprite_version(bin: &Path) -> Option<String> {
    if !bin.is_file() {
        return None;
    }
    let out = Command::new(bin).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // Output looks like "Aseprite 1.3.15.3-x64" or "Aseprite 1.3-beta21-x64".
    // Strip only the architecture suffix — a prerelease suffix is part of the
    // version and must survive (it's compared against release tags).
    let first = text.lines().next()?.trim();
    let version = first.strip_prefix("Aseprite ").unwrap_or(first);
    let version = version.split_whitespace().next().unwrap_or(version);
    let version = version
        .strip_suffix("-x64")
        .or_else(|| version.strip_suffix("-x86"))
        .or_else(|| version.strip_suffix("-arm64"))
        .unwrap_or(version);
    if version.is_empty() {
        None
    } else {
        Some(version.to_string())
    }
}

pub fn require_cmake() -> Result<PathBuf> {
    cmake_path().ok_or_else(|| anyhow!("CMake not available — check Tool Health"))
}

pub fn require_ninja() -> Result<PathBuf> {
    ninja_path().ok_or_else(|| anyhow!("Ninja not available — check Tool Health"))
}
