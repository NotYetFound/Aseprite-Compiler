# Aseprite Compiler 2 — Project Plan

A desktop app with a clean dashboard that pulls the official Aseprite source from GitHub, compiles it locally, installs it, and registers **Aseprite** in the OS app launcher. Ships as a Linux AppImage and a Windows `.exe` installer, keeps itself updated, and can watch for new Aseprite releases from an optional tray icon.

> Legal note: the tool never distributes Aseprite binaries — it automates the "compile it yourself" path the Aseprite EULA permits. Show a short notice + EULA link on first run.

---

## 1. Tech stack (recommendation)

**Tauri 2 (Rust core + web frontend)** — recommended.

- Clean, modern UI cheaply (HTML/CSS via system webview); frontend in **Svelte + TypeScript** (small, fast, no runtime bloat).
- Rust backend does all the real work: process orchestration, downloads, git, filesystem.
- First-party plugins cover our checklist: `updater` (self-update for both NSIS exe and AppImage), `tray-icon`, `notification`, `single-instance`, `autostart`, `shell/process`.
- `tauri bundle` produces **AppImage** and **NSIS .exe installer** out of the box; `tauri-action` on GitHub Actions builds + signs + publishes both with an updater manifest.
- Small footprint: ~8–15 MB exe installer; AppImage larger (~80 MB, bundles webkit2gtk) but fully self-contained.

Alternative considered: pure-Rust `egui` (no webview, tiny AppImage) — rejected for UI quality/velocity; Electron — rejected for size and update weight.

## 2. Architecture

```
┌─ Frontend (Svelte) ────────────────────────────┐
│ Dashboard · Progress · Tool Health · Settings  │
└──────────────┬─────────────────────────────────┘
               │ Tauri IPC (commands + events)
┌──────────────┴─────────────────────────────────┐
│ Rust core                                      │
│  • release_watcher  (GitHub API, ETag-cached)  │
│  • toolchain        (detect/provision tools)   │
│  • pipeline         (state machine, stages)    │
│  • installer        (install dir, launcher)    │
│  • self_update      (tauri updater)            │
└────────────────────────────────────────────────┘
```

- Pipeline is a resumable **state machine**; each stage emits typed progress events to the UI (percent, speed, ETA where measurable) and can be retried individually without redoing completed stages.
- One writer lock on the workspace so two instances can't collide (`single-instance` plugin + lockfile).

### Workspace layout
- Linux: `~/.local/share/aseprite-compiler/{src, skia, build, tools, logs}`; install to `~/.local/share/aseprite-compiler/install` (or user-chosen).
- Windows: `%LOCALAPPDATA%\AsepriteCompiler\{...}`; install to `%LOCALAPPDATA%\Programs\Aseprite` (or user-chosen).

## 3. Build pipeline (stages)

1. **Preflight** — Tool Health check (section 4), free-disk check (source+Skia+build ≈ 4–6 GB), network check.
2. **Resolve version** — GitHub API: latest stable release tag of `aseprite/aseprite` (channel option: *stable* / *beta* / *main*). Read currently installed version by running `aseprite --version` rather than trusting saved state.
3. **Fetch source** — shallow git clone of the chosen tag with `--recurse-submodules --shallow-submodules`; on update, `git fetch` + checkout instead of recloning (when cleanup is off). Resume/retry-safe.
4. **Fetch Skia** — download the matching prebuilt from `aseprite/skia` releases (the tag Aseprite's INSTALL.md pins, parsed from the source tree, e.g. `laf/misc/skia-url` / INSTALL.md); per-platform archive; resumable download with speed/ETA; cached and reused across builds.
5. **Configure** — CMake + Ninja generator.
   - Linux: clang, `-DLAF_BACKEND=skia`, Skia dirs, flags per Aseprite INSTALL.md.
   - Windows: locate MSVC via `vswhere`, spawn build under `vcvars64` environment; x64.
6. **Compile** — `ninja aseprite`, parallel by default; parse `[N/M]` lines for a real progress bar; optional ccache/sccache if detected (speed win on rebuilds).
7. **Install** — stage new build to a temp dir, atomically swap into the install dir; on failure restore the previous install. Never touch Aseprite user preferences.
8. **Register Aseprite in the app launcher** ⭐
   - Linux: install icons into `~/.local/share/icons/hicolor/*` , write `~/.local/share/applications/aseprite.desktop` pointing at the installed binary, run `update-desktop-database`; optional `~/.local/bin/aseprite` symlink.
   - Windows: Start Menu shortcut (`.lnk`) with icon; optional desktop shortcut; per-user uninstall entry for the Aseprite install ("Repair/Uninstall" from our UI, not Windows' list).
9. **Cleanup (optional)** — delete `src/`, `build/`, Skia archive after success; show reclaimed space. When off, keep everything for fast incremental updates.

Cancel at any point kills the child process tree and leaves the previous install untouched.

## 4. Dependencies: bundle what we can, helper for the rest

**Bundled / self-provisioned (no user action):**
- `ninja` — tiny, bundled in the installer/AppImage.
- `cmake` — Windows: portable build auto-downloaded to `tools/` on first run (keeps installer small); Linux: use system cmake if present, else offer the helper.
- `git` — prefer **libgit2 (git2-rs)** built into the app for clone/fetch/submodules → no system git needed at all; fall back to system git if libgit2 hits an edge case.
- Skia — always fetched prebuilt (never built from source).

**Cannot be bundled → Tool Health + Install Helper:**
- Linux: C++ compiler (clang/g++) and dev headers (X11, Xcursor, Xi, GL, fontconfig…).
- Windows: **VS 2022 Build Tools** with "Desktop development with C++" + Windows SDK.

**Tool Health panel:** live checklist with ✓/✗ per tool (found version + path), and a one-click **Install missing**:
- Linux: detect distro → generate the exact package command (pacman for Arch/CachyOS, apt, dnf) → run via `pkexec` in a visible terminal pane, or copy-to-clipboard fallback.
- Windows: `winget install` commands (VS Build Tools with the C++ workload flags) with UAC elevation; deep-link to the VS installer if winget is unavailable.
- Re-check automatically after the helper finishes; pipeline's Preflight stage reuses the same checks.

## 5. Updates

Two distinct updaters:

**A. Self-update (the compiler app):** Tauri `updater` plugin against a GitHub Releases updater manifest (`latest.json`, signed). Works for the NSIS exe and replaces the AppImage in place. Check on launch + daily; "Update & restart" button; silent-install option.

**B. Aseprite watcher:** background task polls the GitHub releases API (ETag/conditional requests, configurable interval, default 12 h). New release → native notification + tray badge → one click runs the pipeline. Modes: *notify only* / *auto-build on new release*.

**Optional tray (off by default, toggle in Settings):** icon with menu — Check for updates now, Build latest, Open dashboard, Pause watching, Quit. Optional "start minimized to tray at login" via autostart plugin. Closing the window with tray enabled minimizes instead of exiting.

## 6. UI (single-window dashboard)

- **Home:** status card — installed Aseprite version vs latest available, last check time, one primary button (Install / Update / Up to date ✓). During a run: vertical stage list, each with its own progress bar, speed/ETA on downloads, `[N/M]` on compile; collapsible live log console; Cancel + stage-aware Retry. Completion summary (elapsed time, installed size, space cleaned).
- **Tool Health:** the checklist + install helper (section 4).
- **Settings:** channel (stable/beta/main), install path, cleanup toggle, tray + autostart, watcher interval/mode, sccache toggle, app self-update channel.
- **About:** EULA notice, licenses, app version.
- Dark/light theme following the system; keyboard-free flow — everything reachable in ≤2 clicks.

## 7. Packaging & CI

- Repo on GitHub (needed for self-update manifests). `cargo` workspace + `src-tauri` + `ui/`.
- **GitHub Actions** (`tauri-action`): tag `v*` → matrix:
  - `ubuntu-22.04` (older glibc for AppImage compatibility) → `Aseprite-Compiler-x86_64.AppImage`
  - `windows-latest` → `Aseprite-Compiler-Setup-x64.exe` (NSIS, per-user install, no admin needed)
  - plus signed `latest.json` updater manifest attached to the release.
- Updater signing keypair kept in repo secrets.

## 8. Speed checklist

Shallow clones · incremental `git fetch` updates · Skia cache · resumable downloads · full-parallel ninja · optional sccache · skip-if-unchanged stages (source already at tag, Skia already cached) · atomic install swap (no re-copy on failure).

## 9. Milestones

| # | Milestone | Outcome |
|---|-----------|---------|
| 1 | Skeleton | Tauri 2 app boots, dashboard UI with mocked pipeline events |
| 2 | Linux pipeline | End-to-end build+install on this machine (CachyOS) |
| 3 | Launcher + Tool Health | Aseprite appears in app launcher; helper installs deps on Arch/apt/dnf |
| 4 | Windows pipeline | MSVC detection, vcvars build, Start Menu shortcut |
| 5 | Packaging + CI | AppImage + NSIS exe from GitHub Actions on tag push |
| 6 | Updaters + tray | Self-update live; Aseprite watcher + notifications + tray |
| 7 | Polish | Error taxonomy with fix hints, retries, repair/uninstall, completion summaries |

## 10. Decisions to confirm

1. **Stack:** Tauri 2 + Svelte as recommended? (Alternative: pure-Rust egui — smaller AppImage, plainer UI.)
2. **GitHub repo name/visibility** for hosting releases + updater manifest (required for auto-update).
3. **Channels:** stable-only at first, or include beta/main from day one?
