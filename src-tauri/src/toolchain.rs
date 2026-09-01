use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

use crate::{archive, net, paths};

const NINJA_VERSION: &str = "1.12.1";
const CMAKE_VERSION: &str = "3.31.6";

// Pinned SHA-256 checksums for the managed portable tools, taken from the
// vendors' official release checksum files (Kitware's cmake-*-SHA-256.txt;
// ninja computed from the official GitHub release assets).
#[cfg(target_os = "windows")]
const NINJA_SHA256: &str = "f550fec705b6d6ff58f2db3c374c2277a37691678d6aba463adcbb129108467a";
#[cfg(not(target_os = "windows"))]
const NINJA_SHA256: &str = "6f98805688d19672bd699fbbfa2c2cf0fc054ac3df1f0e6a47664d963d530255";
#[cfg(target_os = "windows")]
const CMAKE_SHA256: &str = "d163cd3ab4959b0a53fa8988f2ddbd2e6c501658201e6a154386bad9dbe4f836";
#[cfg(not(target_os = "windows"))]
const CMAKE_SHA256: &str = "5a1133ff103c71eb5120e2cc3de922733e7d8a26a98ae716397e8676adb367bf";

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
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.lines().next().unwrap_or("").trim().to_string())
        .unwrap_or_default()
}

/// A tool's reported version line — used in build fingerprints, so a tool
/// upgrade at the same path invalidates stale configure/build products.
pub fn tool_version(bin: &Path) -> String {
    version_of(bin, "--version")
}

/// Version stamp of a managed portable tool ("ninja"/"cmake" dir name).
fn portable_stamp(dir_name: &str) -> Option<String> {
    std::fs::read_to_string(paths::tools_dir().join(dir_name).join(".version"))
        .ok()
        .map(|s| s.trim().to_string())
}

fn write_portable_stamp(dir_name: &str, version: &str) {
    let _ = std::fs::write(
        paths::tools_dir().join(dir_name).join(".version"),
        version,
    );
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

    // Portable/managed tools: "found" is not enough — the tool must actually
    // run and report a version to count as healthy.
    for (id, name, found, portable_path, prefix) in [
        (
            "cmake",
            "CMake",
            cmake_path(),
            portable_cmake(),
            "",
        ),
        (
            "ninja",
            "Ninja",
            ninja_path(),
            portable_ninja(),
            "ninja ",
        ),
    ] {
        match found {
            Some(p) => {
                let version = version_of(&p, "--version");
                let portable = p == portable_path;
                if version.is_empty() {
                    tools.push(ToolStatus {
                        id: id.into(),
                        name: name.into(),
                        ok: false,
                        detail: format!(
                            "found at {} but it can't be run — the app can reinstall a portable copy",
                            p.display()
                        ),
                        provisionable: true,
                    });
                } else {
                    tools.push(ToolStatus {
                        id: id.into(),
                        name: name.into(),
                        ok: true,
                        detail: format!(
                            "{prefix}{version}{}",
                            if portable { " · portable" } else { "" }
                        ),
                        provisionable: true,
                    });
                }
            }
            None => tools.push(ToolStatus {
                id: id.into(),
                name: name.into(),
                ok: false,
                detail: "Not found — the app can install a portable copy".into(),
                provisionable: true,
            }),
        }
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
            ("libwebp", "WebP headers"),
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
                "sudo pacman -S --needed clang pkgconf libx11 libxcursor libxi libxrandr mesa fontconfig libwebp",
                "Arch-based system detected. Run this in a terminal:",
            ),
            "debian" | "ubuntu" | "linuxmint" | "pop" => (
                "sudo apt install clang pkg-config libx11-dev libxcursor-dev libxi-dev libxrandr-dev libgl1-mesa-dev libfontconfig1-dev libwebp-dev",
                "Debian/Ubuntu-based system detected. Run this in a terminal:",
            ),
            "fedora" | "nobara" => (
                "sudo dnf install clang pkgconf-pkg-config libX11-devel libXcursor-devel libXi-devel libXrandr-devel mesa-libGL-devel fontconfig-devel libwebp-devel",
                "Fedora-based system detected. Run this in a terminal:",
            ),
            "opensuse-tumbleweed" | "opensuse-leap" => (
                "sudo zypper install clang pkgconf-pkg-config libX11-devel libXcursor-devel libXi-devel libXrandr-devel Mesa-libGL-devel fontconfig-devel libwebp-devel",
                "openSUSE detected. Run this in a terminal:",
            ),
            _ => (
                "clang (or g++), pkg-config, and development headers for: X11, Xcursor, Xi, Xrandr, OpenGL, Fontconfig, WebP",
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
/// Downloads are verified against pinned SHA-256 checksums, and a portable
/// tool whose version stamp no longer matches the pinned version is replaced
/// (only after the replacement downloads and verifies).
pub fn provision(cancel: &AtomicBool, mut log: impl FnMut(String)) -> Result<()> {
    paths::ensure_dirs()?;

    let ninja_stale = portable_ninja().is_file()
        && portable_stamp("ninja").as_deref() != Some(NINJA_VERSION);
    if ninja_path().is_none() || ninja_stale {
        log(format!(
            "{} portable Ninja {NINJA_VERSION}…",
            if ninja_stale { "Updating" } else { "Downloading" }
        ));
        let archive_path = paths::cache_dir().join(format!("ninja-{NINJA_VERSION}.zip"));
        let expected = net::Expected {
            size: None,
            sha256: Some(NINJA_SHA256.into()),
        };
        net::download_verified(&ninja_url(), &archive_path, &expected, cancel, |_, _| {})?;
        let dest = paths::tools_dir().join("ninja");
        std::fs::remove_dir_all(&dest).ok();
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
        if portable_ninja().is_file() && !tool_version(&portable_ninja()).is_empty() {
            write_portable_stamp("ninja", NINJA_VERSION);
        } else {
            bail!("ninja extraction failed");
        }
        log("Portable Ninja installed.".into());
    }

    let cmake_stale = portable_cmake().is_file()
        && portable_stamp("cmake").as_deref() != Some(CMAKE_VERSION);
    if cmake_path().is_none() || cmake_stale {
        let (url, root_name) = cmake_url();
        log(format!(
            "{} portable CMake {CMAKE_VERSION}…",
            if cmake_stale { "Updating" } else { "Downloading" }
        ));
        let file_name = url.rsplit('/').next().unwrap().to_string();
        let archive_path = paths::cache_dir().join(&file_name);
        let expected = net::Expected {
            size: None,
            sha256: Some(CMAKE_SHA256.into()),
        };
        net::download_verified(&url, &archive_path, &expected, cancel, |_, _| {})?;

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
        if portable_cmake().is_file() && !tool_version(&portable_cmake()).is_empty() {
            write_portable_stamp("cmake", CMAKE_VERSION);
        } else {
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
    // Source-compiled Aseprite reports e.g. "1.3.18.3-dev" — the "-dev"
    // marks an unofficial build, it is not part of the release version.
    let version = version.strip_suffix("-dev").unwrap_or(version);
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
