use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::net::{is_cancelled, Cancelled};
use crate::settings::Settings;
use crate::state::{now_millis, PersistedState};
use crate::{archive, github, installer, net, paths, toolchain};

/// Where pipeline events go. The app wires this to the Tauri event system;
/// tests can use a plain stderr sink.
pub trait Sink: Send + Sync + 'static {
    fn emit_state(&self, state: &PipelineState);
    fn emit_log(&self, line: &str);
    fn emit_status(&self, busy: bool);
    fn notify(&self, title: &str, body: &str);
}

const LOG_TAIL_MAX: usize = 2000;

/// Bump when backend build rules change in a way that invalidates existing
/// configure/build products (e.g. a new required CMake flag).
const BACKEND_RULES_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StageStatus {
    Pending,
    Running,
    Done,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageInfo {
    pub id: String,
    pub name: String,
    pub status: StageStatus,
    pub progress: Option<f64>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildSummary {
    pub version: String,
    pub elapsed_secs: u64,
    pub installed_bytes: u64,
    pub cleaned_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineState {
    pub running: bool,
    pub stages: Vec<StageInfo>,
    pub error: Option<String>,
    pub failed_stage: Option<String>,
    pub summary: Option<BuildSummary>,
}

const STAGES: &[(&str, &str)] = &[
    ("preflight", "Check tools & disk space"),
    ("resolve", "Resolve latest release"),
    ("source", "Download Aseprite source"),
    ("skia", "Download Skia"),
    ("configure", "Configure build"),
    ("compile", "Compile"),
    ("install", "Install"),
    ("register", "Add Aseprite to the app launcher"),
    ("cleanup", "Clean up"),
];

fn fresh_stages() -> Vec<StageInfo> {
    STAGES
        .iter()
        .map(|(id, name)| StageInfo {
            id: (*id).into(),
            name: (*name).into(),
            status: StageStatus::Pending,
            progress: None,
            detail: String::new(),
        })
        .collect()
}

pub fn fmt_bytes(n: u64) -> String {
    const GIB: f64 = (1u64 << 30) as f64;
    const MIB: f64 = (1u64 << 20) as f64;
    let n = n as f64;
    if n >= GIB {
        format!("{:.2} GiB", n / GIB)
    } else if n >= MIB {
        format!("{:.1} MiB", n / MIB)
    } else {
        format!("{:.0} KiB", n / 1024.0)
    }
}

/// Tracks download speed / ETA and throttles UI updates.
struct SpeedMeter {
    started: Instant,
    last_emit: Instant,
    last_bytes: u64,
    last_speed: f64,
}

impl SpeedMeter {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            started: now,
            last_emit: now,
            last_bytes: 0,
            last_speed: 0.0,
        }
    }

    /// Returns Some(detail, progress) when the UI should update.
    fn tick(&mut self, downloaded: u64, total: Option<u64>) -> Option<(String, Option<f64>)> {
        let now = Instant::now();
        if now.duration_since(self.last_emit) < Duration::from_millis(200) {
            return None;
        }
        let dt = now.duration_since(self.last_emit).as_secs_f64();
        let instant_speed = (downloaded.saturating_sub(self.last_bytes)) as f64 / dt.max(0.001);
        // Smooth the speed a little so the number doesn't jump around.
        self.last_speed = if self.last_speed == 0.0 {
            instant_speed
        } else {
            self.last_speed * 0.7 + instant_speed * 0.3
        };
        self.last_emit = now;
        self.last_bytes = downloaded;

        let speed = self.last_speed;
        let mut detail = format!("{} · {}/s", fmt_bytes(downloaded), fmt_bytes(speed as u64));
        let progress = total.map(|t| {
            if speed > 1.0 && t > downloaded {
                let eta = (t - downloaded) as f64 / speed;
                let m = (eta / 60.0) as u64;
                let s = (eta % 60.0) as u64;
                detail = format!(
                    "{} of {} · {}/s · {:02}:{:02} left",
                    fmt_bytes(downloaded),
                    fmt_bytes(t),
                    fmt_bytes(speed as u64),
                    m,
                    s
                );
            }
            (downloaded as f64 / t as f64).min(1.0)
        });
        let _ = self.started;
        Some((detail, progress))
    }
}

struct Ctx {
    settings: Settings,
    release: Option<github::ReleaseInfo>,
    src_root: Option<PathBuf>,
    skia_dir: Option<PathBuf>,
    installed_bytes: u64,
    cleaned_bytes: u64,
    /// Version reported by the staged binary during install validation.
    probed_version: Option<String>,
}

pub struct Engine {
    sink: Box<dyn Sink>,
    state: Mutex<PipelineState>,
    log: Mutex<VecDeque<String>>,
    cancel: AtomicBool,
    running: AtomicBool,
    child_pid: Mutex<Option<u32>>,
}

impl Engine {
    pub fn new(sink: Box<dyn Sink>) -> Arc<Self> {
        Arc::new(Self {
            sink,
            state: Mutex::new(PipelineState {
                running: false,
                stages: fresh_stages(),
                error: None,
                failed_stage: None,
                summary: None,
            }),
            log: Mutex::new(VecDeque::new()),
            cancel: AtomicBool::new(false),
            running: AtomicBool::new(false),
            child_pid: Mutex::new(None),
        })
    }

    pub fn running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn snapshot(&self) -> PipelineState {
        self.state.lock().unwrap().clone()
    }

    pub fn log_tail(&self) -> Vec<String> {
        self.log.lock().unwrap().iter().cloned().collect()
    }

    fn emit_state(&self) {
        let snap = self.snapshot();
        self.sink.emit_state(&snap);
    }

    pub fn log_line(&self, line: impl Into<String>) {
        let line = line.into();
        {
            let mut log = self.log.lock().unwrap();
            log.push_back(line.clone());
            while log.len() > LOG_TAIL_MAX {
                log.pop_front();
            }
        }
        self.sink.emit_log(&line);
    }

    fn set_stage(
        &self,
        id: &str,
        status_: StageStatus,
        progress: Option<f64>,
        detail: impl Into<String>,
    ) {
        {
            let mut st = self.state.lock().unwrap();
            if let Some(s) = st.stages.iter_mut().find(|s| s.id == id) {
                s.status = status_;
                s.progress = progress;
                s.detail = detail.into();
            }
        }
        self.emit_state();
    }

    fn update_stage_progress(&self, id: &str, progress: Option<f64>, detail: impl Into<String>) {
        {
            let mut st = self.state.lock().unwrap();
            if let Some(s) = st.stages.iter_mut().find(|s| s.id == id) {
                s.progress = progress;
                s.detail = detail.into();
            }
        }
        self.emit_state();
    }

    fn check_cancel(&self) -> Result<()> {
        if self.cancel.load(Ordering::SeqCst) {
            bail!(Cancelled);
        }
        Ok(())
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        let pid = *self.child_pid.lock().unwrap();
        if let Some(pid) = pid {
            kill_process_tree(pid);
        }
    }

    /// Start the pipeline on a worker thread. Errors if a run is in progress.
    pub fn start(self: &Arc<Self>, settings: Settings) -> Result<()> {
        if self.running.swap(true, Ordering::SeqCst) {
            bail!("a build is already running");
        }
        self.cancel.store(false, Ordering::SeqCst);
        {
            let mut st = self.state.lock().unwrap();
            st.running = true;
            st.stages = fresh_stages();
            st.error = None;
            st.failed_stage = None;
            st.summary = None;
        }
        self.emit_state();
        self.sink.emit_status(true);

        let engine = Arc::clone(self);
        std::thread::spawn(move || {
            let started = Instant::now();
            let mut ctx = Ctx {
                settings,
                release: None,
                src_root: None,
                skia_dir: None,
                installed_bytes: 0,
                cleaned_bytes: 0,
                probed_version: None,
            };
            let result = engine.run_all(&mut ctx);

            {
                let mut st = engine.state.lock().unwrap();
                st.running = false;
                match &result {
                    Ok(()) => {
                        st.summary = Some(BuildSummary {
                            version: ctx
                                .release
                                .as_ref()
                                .map(|r| r.version.clone())
                                .unwrap_or_default(),
                            elapsed_secs: started.elapsed().as_secs(),
                            installed_bytes: ctx.installed_bytes,
                            cleaned_bytes: ctx.cleaned_bytes,
                        });
                    }
                    Err(e) => {
                        let msg = if is_cancelled(e) {
                            "Cancelled".to_string()
                        } else {
                            format!("{e:#}")
                        };
                        st.failed_stage = st
                            .stages
                            .iter()
                            .find(|s| s.status == StageStatus::Failed)
                            .map(|s| s.name.clone());
                        st.error = Some(msg);
                    }
                }
            }
            engine.running.store(false, Ordering::SeqCst);
            engine.emit_state();
            engine.sink.emit_status(false);

            match &result {
                Ok(()) => {
                    let version = ctx
                        .release
                        .as_ref()
                        .map(|r| r.version.clone())
                        .unwrap_or_default();
                    engine.log_line(format!("Done. Aseprite {version} is installed."));
                    engine.sink.notify(
                        "Aseprite installed",
                        &format!("Aseprite {version} was compiled and installed successfully."),
                    );
                }
                Err(e) if !is_cancelled(e) => {
                    engine.sink.notify("Aseprite build failed", &format!("{e:#}"));
                }
                _ => {}
            }
        });
        Ok(())
    }

    fn run_all(&self, ctx: &mut Ctx) -> Result<()> {
        type StageFn = fn(&Engine, &mut Ctx) -> Result<StageResult>;
        let stages: &[(&str, StageFn)] = &[
            ("preflight", Engine::stage_preflight as StageFn),
            ("resolve", Engine::stage_resolve),
            ("source", Engine::stage_source),
            ("skia", Engine::stage_skia),
            ("configure", Engine::stage_configure),
            ("compile", Engine::stage_compile),
            ("install", Engine::stage_install),
            ("register", Engine::stage_register),
            ("cleanup", Engine::stage_cleanup),
        ];

        for (id, f) in stages {
            self.check_cancel()?;
            self.set_stage(id, StageStatus::Running, None, "");
            match f(self, ctx) {
                Ok(StageResult::Done(detail)) => {
                    self.set_stage(id, StageStatus::Done, Some(1.0), detail);
                }
                Ok(StageResult::Skipped(detail)) => {
                    self.set_stage(id, StageStatus::Skipped, None, detail);
                }
                Err(e) => {
                    let detail = if is_cancelled(&e) { "cancelled" } else { "failed" };
                    self.set_stage(id, StageStatus::Failed, None, detail);
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    // ---- stages ----

    fn stage_preflight(&self, ctx: &mut Ctx) -> Result<StageResult> {
        paths::ensure_dirs()?;

        // Provision missing portable tools automatically (self-contained, in
        // the app's own folder — never system-wide).
        self.update_stage_progress("preflight", None, "checking portable tools");
        toolchain::provision(&self.cancel, |m| self.log_line(m))?;

        #[cfg(target_os = "linux")]
        {
            if toolchain::linux_compiler().is_none() {
                bail!("no C++ compiler found (clang or g++) — see the Tool Health tab");
            }
        }
        #[cfg(target_os = "windows")]
        {
            if toolchain::vcvars64().is_none() {
                bail!("Visual Studio Build Tools with the C++ workload were not found — see the Tool Health tab");
            }
        }

        let report = toolchain::check();
        let missing: Vec<&str> = report
            .tools
            .iter()
            .filter(|t| !t.ok)
            .map(|t| t.name.as_str())
            .collect();
        if !missing.is_empty() {
            bail!(
                "missing system dependencies: {} — see the Tool Health tab for a one-line install command",
                missing.join(", ")
            );
        }

        if let Some(free) = free_space(&paths::data_dir()) {
            const NEED: u64 = 8 * (1 << 30);
            if free < NEED {
                bail!(
                    "not enough free disk space: {} available, about {} needed",
                    fmt_bytes(free),
                    fmt_bytes(NEED)
                );
            }
        }
        // A custom install root can live on a different filesystem than the
        // build workspace — check it separately.
        let root = ctx.settings.install_root();
        std::fs::create_dir_all(&root).ok();
        if let Some(free) = free_space(&root) {
            const NEED_INSTALL: u64 = 500 * (1 << 20);
            if free < NEED_INSTALL {
                bail!(
                    "not enough free disk space at the install location: {} available, \
                     about {} needed",
                    fmt_bytes(free),
                    fmt_bytes(NEED_INSTALL)
                );
            }
        }
        Ok(StageResult::Done("tools ready".into()))
    }

    fn stage_resolve(&self, ctx: &mut Ctx) -> Result<StageResult> {
        let release = github::fetch_latest(ctx.settings.channel)?;
        self.log_line(format!("Latest release: {} ({})", release.name, release.tag));

        PersistedState::update(|st| {
            st.last_check = Some(now_millis());
            st.latest = Some(release.clone());
        });

        let detail = release.tag.clone();
        ctx.release = Some(release);
        Ok(StageResult::Done(detail))
    }

    fn stage_source(&self, ctx: &mut Ctx) -> Result<StageResult> {
        let release = ctx.release.as_ref().ok_or_else(|| anyhow!("no release resolved"))?;
        let base = paths::src_dir(&release.version);
        let marker = base.join(".extract-ok");
        if marker.is_file() {
            ctx.src_root = Some(resolve_src_root(&base)?);
            return Ok(StageResult::Skipped("already downloaded".into()));
        }

        let zip_path = paths::cache_dir().join(format!("Aseprite-{}-Source.zip", release.tag));
        if release.source_zip_sha256.is_none() {
            self.log_line("Note: this release publishes no source checksum; verifying size only.");
        }
        let expected = net::Expected {
            size: Some(release.source_zip_size),
            sha256: release.source_zip_sha256.clone(),
        };
        let mut meter = SpeedMeter::new();
        net::download_verified(
            &release.source_zip_url,
            &zip_path,
            &expected,
            &self.cancel,
            |d, t| {
                if let Some((detail, progress)) =
                    meter.tick(d, t.or(Some(release.source_zip_size)))
                {
                    self.update_stage_progress("source", progress, detail);
                }
            },
        )?;

        self.update_stage_progress("source", None, "extracting");
        std::fs::remove_dir_all(&base).ok();
        archive::extract_zip(&zip_path, &base, &self.cancel, |done, total| {
            self.update_stage_progress(
                "source",
                Some(done as f64 / total.max(1) as f64),
                format!("extracting {done}/{total} files"),
            );
        })?;
        std::fs::write(&marker, b"ok")?;
        ctx.src_root = Some(resolve_src_root(&base)?);
        Ok(StageResult::Done(release.tag.clone()))
    }

    fn stage_skia(&self, ctx: &mut Ctx) -> Result<StageResult> {
        let src_root = ctx.src_root.clone().ok_or_else(|| anyhow!("no source tree"))?;

        // A pinned Skia is resolved exactly or not at all — a pinned
        // dependency must never silently become a different version.
        let asset = match detect_skia_tag(&src_root) {
            Some(pin) => {
                self.log_line(format!("Source pins Skia {pin}"));
                github::skia_asset_exact(&pin)?
            }
            None => {
                self.log_line(
                    "Warning: no Skia pin found in the source tree; falling back to the \
                     latest Skia release."
                        .to_string(),
                );
                github::skia_asset_latest()?
            }
        };

        let tag = asset.tag.clone();
        let dest = paths::cache_dir().join("skia").join(&tag);
        let marker = dest.join(".extract-ok");
        if marker.is_file() {
            ctx.skia_dir = Some(dest);
            return Ok(StageResult::Skipped(format!("{tag} cached")));
        }

        // Tag-qualified filename: a leftover zip from a different pinned tag
        // must never be mistaken for this one.
        let zip_path = paths::cache_dir().join(format!("{tag}-{}", asset.asset_name));
        let expected = net::Expected {
            size: None,
            sha256: asset.sha256.clone(),
        };
        let mut meter = SpeedMeter::new();
        net::download_verified(&asset.url, &zip_path, &expected, &self.cancel, |d, t| {
            if let Some((detail, progress)) = meter.tick(d, t) {
                self.update_stage_progress("skia", progress, detail);
            }
        })?;

        self.update_stage_progress("skia", None, "extracting");
        std::fs::remove_dir_all(&dest).ok();
        archive::extract_zip(&zip_path, &dest, &self.cancel, |done, total| {
            self.update_stage_progress(
                "skia",
                Some(done as f64 / total.max(1) as f64),
                format!("extracting {done}/{total} files"),
            );
        })?;
        std::fs::write(&marker, b"ok")?;
        ctx.skia_dir = Some(dest);
        Ok(StageResult::Done(tag))
    }

    fn stage_configure(&self, ctx: &mut Ctx) -> Result<StageResult> {
        let release = ctx.release.as_ref().ok_or_else(|| anyhow!("no release"))?;
        let src_root = ctx.src_root.clone().ok_or_else(|| anyhow!("no source tree"))?;
        let skia = ctx.skia_dir.clone().ok_or_else(|| anyhow!("no skia"))?;
        let build = paths::build_dir(&release.version);
        let marker = build.join(".configure-ok");

        let cmake = toolchain::require_cmake()?;
        let ninja = toolchain::require_ninja()?;

        let skia_out = skia.join("out").join("Release-x64");
        let skia_lib = skia_out.join(if cfg!(windows) { "skia.lib" } else { "libskia.a" });

        let mut args: Vec<String> = vec![
            "-S".into(),
            src_root.display().to_string(),
            "-B".into(),
            build.display().to_string(),
            "-G".into(),
            "Ninja".into(),
            format!("-DCMAKE_MAKE_PROGRAM={}", ninja.display()),
            "-DCMAKE_BUILD_TYPE=Release".into(),
            "-DLAF_BACKEND=skia".into(),
            format!("-DSKIA_DIR={}", skia.display()),
            format!("-DSKIA_LIBRARY_DIR={}", skia_out.display()),
            format!("-DSKIA_LIBRARY={}", skia_lib.display()),
            "-DENABLE_UPDATER=OFF".into(),
        ];

        #[cfg(target_os = "linux")]
        {
            let (cc, cxx) =
                toolchain::linux_compiler().ok_or_else(|| anyhow!("no C++ compiler found"))?;
            args.push(format!("-DCMAKE_C_COMPILER={}", cc.display()));
            args.push(format!("-DCMAKE_CXX_COMPILER={}", cxx.display()));
            // The prebuilt Skia links against libstdc++. gcc always uses it;
            // for clang say so explicitly instead of relying on the distro's
            // default stdlib configuration.
            if cxx.file_name().is_some_and(|n| n.to_string_lossy().contains("clang")) {
                args.push("-DCMAKE_CXX_FLAGS=-stdlib=libstdc++".into());
                args.push("-DCMAKE_EXE_LINKER_FLAGS=-stdlib=libstdc++".into());
            }
        }

        // Skip only when the previous configuration used identical semantic
        // inputs: tool identities AND versions AND flags AND backend rules.
        // Changing any of them (e.g. a compiler upgrade, or an app update
        // that adds a flag) rebuilds from scratch.
        let compiler_id = {
            #[cfg(target_os = "linux")]
            {
                toolchain::linux_compiler()
                    .map(|(_, cxx)| {
                        format!("{} {}", cxx.display(), toolchain::tool_version(&cxx))
                    })
                    .unwrap_or_default()
            }
            #[cfg(target_os = "windows")]
            {
                toolchain::vcvars64()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            }
            #[cfg(not(any(target_os = "linux", target_os = "windows")))]
            {
                String::new()
            }
        };
        let fingerprint = format!(
            "rules:{BACKEND_RULES_VERSION}\ncmake:{} {}\nninja:{} {}\ncompiler:{}\n{}",
            cmake.display(),
            toolchain::tool_version(&cmake),
            ninja.display(),
            toolchain::tool_version(&ninja),
            compiler_id,
            args.join("\n")
        );
        if build.join("CMakeCache.txt").is_file() {
            if std::fs::read_to_string(&marker).ok().as_deref() == Some(fingerprint.as_str()) {
                return Ok(StageResult::Skipped("already configured".into()));
            }
            self.log_line("Build settings changed — reconfiguring from scratch.");
            std::fs::remove_dir_all(&build).ok();
        }
        std::fs::create_dir_all(&build)?;

        let mut cmd = build_tool_command(&cmake, &args)?;
        cmd.current_dir(&build);
        self.run_cmd(cmd, "configure", |_line, _eng| {})?;
        std::fs::write(&marker, fingerprint)?;
        Ok(StageResult::Done("configured".into()))
    }

    fn stage_compile(&self, ctx: &mut Ctx) -> Result<StageResult> {
        let release = ctx.release.as_ref().ok_or_else(|| anyhow!("no release"))?;
        let build = paths::build_dir(&release.version);
        let ninja = toolchain::require_ninja()?;

        let mut args: Vec<String> = vec!["-C".into(), build.display().to_string()];
        if ctx.settings.parallel_jobs > 0 {
            args.push("-j".into());
            args.push(ctx.settings.parallel_jobs.to_string());
        }
        args.push("aseprite".into());

        let mut cmd = build_tool_command(&ninja, &args)?;
        cmd.current_dir(&build);

        let engine_self = self;
        self.run_cmd(cmd, "compile", move |line, _eng| {
            // Ninja progress lines look like "[123/4567] CXX ..."
            if let Some(rest) = line.strip_prefix('[') {
                if let Some((frac, _)) = rest.split_once(']') {
                    if let Some((n, m)) = frac.split_once('/') {
                        if let (Ok(n), Ok(m)) = (n.trim().parse::<f64>(), m.trim().parse::<f64>()) {
                            if m > 0.0 {
                                engine_self.update_stage_progress(
                                    "compile",
                                    Some(n / m),
                                    format!("[{}/{}]", n as u64, m as u64),
                                );
                            }
                        }
                    }
                }
            }
        })?;
        Ok(StageResult::Done("compiled".into()))
    }

    fn stage_install(&self, ctx: &mut Ctx) -> Result<StageResult> {
        let release = ctx.release.as_ref().ok_or_else(|| anyhow!("no release"))?;
        let build_bin = paths::build_dir(&release.version).join("bin");
        let root = ctx.settings.install_root();

        // Heal any interrupted previous install transaction first so a crash
        // can never cost the last working build.
        installer::recover_install(&root);

        self.update_stage_progress("install", None, "copying files");
        let (bytes, version) = installer::install_build(&build_bin, &root, |staged_bin| {
            // The staged binary must actually run before we activate it.
            toolchain::probe_aseprite_version(staged_bin).ok_or_else(|| {
                anyhow!(
                    "the freshly built Aseprite binary failed to run (--version) — \
                     keeping the previous install"
                )
            })
        })?;
        ctx.installed_bytes = bytes;
        ctx.probed_version = Some(version);

        Ok(StageResult::Done(fmt_bytes(ctx.installed_bytes)))
    }

    fn stage_register(&self, ctx: &mut Ctx) -> Result<StageResult> {
        let release = ctx.release.as_ref().ok_or_else(|| anyhow!("no release"))?;
        let src_root = ctx.src_root.clone().ok_or_else(|| anyhow!("no source tree"))?;
        let root = ctx.settings.install_root();
        installer::register_launcher(&root, &src_root)?;

        // Installed state commits only after the launcher exists — a failed
        // registration leaves a retryable stage, not a half-recorded install.
        let version = ctx
            .probed_version
            .clone()
            .unwrap_or_else(|| release.version.clone());
        PersistedState::update(|st| {
            st.installed_version = Some(version);
            st.install_path = Some(root.display().to_string());
        });
        Ok(StageResult::Done("launcher entry created".into()))
    }

    fn stage_cleanup(&self, ctx: &mut Ctx) -> Result<StageResult> {
        if !ctx.settings.cleanup_after_build {
            return Ok(StageResult::Skipped("disabled in settings".into()));
        }
        let mut cleaned = 0u64;
        let mut partial = false;
        for dir in [paths::work_dir(), paths::cache_dir()] {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let p = entry.path();
                let size = archive::dir_size(&p);
                if p.is_dir() {
                    std::fs::remove_dir_all(&p).ok();
                } else {
                    std::fs::remove_file(&p).ok();
                }
                // Only count bytes that are confirmed gone.
                if p.exists() {
                    partial = true;
                } else {
                    cleaned += size;
                }
            }
        }
        ctx.cleaned_bytes = cleaned;
        Ok(StageResult::Done(if partial {
            format!("{} freed (some files were in use)", fmt_bytes(cleaned))
        } else {
            format!("{} freed", fmt_bytes(cleaned))
        }))
    }

    // ---- process running ----

    fn run_cmd(
        &self,
        mut cmd: Command,
        stage_id: &str,
        mut on_line: impl FnMut(&str, &Engine),
    ) -> Result<()> {
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        cmd.stdin(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        self.check_cancel()?;
        let mut child = cmd.spawn().with_context(|| {
            format!("failed to start {:?}", cmd.get_program())
        })?;
        *self.child_pid.lock().unwrap() = Some(child.id());
        // A cancel that landed between the check above and pid registration
        // found nothing to kill — close that window now that the pid is known.
        if self.cancel.load(Ordering::SeqCst) {
            kill_process_tree(child.id());
        }

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let tx2 = tx.clone();
        let t1 = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let t2 = std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if tx2.send(line).is_err() {
                    break;
                }
            }
        });

        let mut tail: VecDeque<String> = VecDeque::new();
        for line in rx {
            self.log_line(line.clone());
            tail.push_back(line.clone());
            while tail.len() > 30 {
                tail.pop_front();
            }
            on_line(&line, self);
        }
        let _ = t1.join();
        let _ = t2.join();

        let status_ = child.wait()?;
        *self.child_pid.lock().unwrap() = None;
        self.check_cancel()?;

        if !status_.success() {
            let _ = stage_id;
            bail!(
                "command exited with {}\n…\n{}",
                status_,
                tail.iter().cloned().collect::<Vec<_>>().join("\n")
            );
        }
        Ok(())
    }
}

enum StageResult {
    Done(String),
    Skipped(String),
}

/// The source zip may extract with everything at the root or inside a single
/// top-level folder; find the directory that holds CMakeLists.txt.
fn resolve_src_root(base: &Path) -> Result<PathBuf> {
    if base.join("CMakeLists.txt").is_file() {
        return Ok(base.to_path_buf());
    }
    let mut dirs = Vec::new();
    for entry in std::fs::read_dir(base)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            dirs.push(entry.path());
        }
    }
    for d in &dirs {
        if d.join("CMakeLists.txt").is_file() {
            return Ok(d.clone());
        }
    }
    Err(anyhow!(
        "could not find CMakeLists.txt under {}",
        base.display()
    ))
}

/// The pinned Skia release tag for this source tree.
///
/// `laf/misc/skia-tag.txt` is upstream's authoritative pin; the documentation
/// scanner remains only as a compatibility fallback for older source layouts.
fn detect_skia_tag(src_root: &Path) -> Option<String> {
    let pin_file = src_root.join("laf").join("misc").join("skia-tag.txt");
    if let Ok(text) = std::fs::read_to_string(&pin_file) {
        let tag = text.trim();
        // Validate the shape (m###-hex) before trusting it.
        if scan_skia_tag(tag).as_deref() == Some(tag) {
            return Some(tag.to_string());
        }
    }

    let candidates = [
        src_root.join("INSTALL.md"),
        src_root.join("laf").join("misc").join("skia-url.sh"),
        src_root.join("laf").join("INSTALL.md"),
    ];
    for path in candidates {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(tag) = scan_skia_tag(&text) {
            return Some(tag);
        }
    }
    None
}

pub fn scan_skia_tag(text: &str) -> Option<String> {
    // Match tokens shaped like m###-hexhash without pulling in a regex crate.
    for token in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')) {
        let bytes = token.as_bytes();
        if bytes.len() < 8 || bytes[0] != b'm' {
            continue;
        }
        let Some(dash) = token.find('-') else { continue };
        let (ms, hash) = (&token[1..dash], &token[dash + 1..]);
        if ms.len() >= 2
            && ms.chars().all(|c| c.is_ascii_digit())
            && hash.len() >= 6
            && hash.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Some(token.to_string());
        }
    }
    None
}

/// Wrap a tool invocation so it runs inside the MSVC environment on Windows.
fn build_tool_command(tool: &Path, args: &[String]) -> Result<Command> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let vcvars = toolchain::vcvars64()
            .ok_or_else(|| anyhow!("Visual Studio Build Tools not found"))?;
        let mut cmd = Command::new("cmd");
        let quoted: Vec<String> = args.iter().map(|a| format!("\"{a}\"")).collect();
        // raw_arg: std's automatic quoting escapes inner quotes in a way
        // cmd.exe does not understand; with /S the outer quotes are stripped
        // and the inner quoting reaches cmd verbatim.
        cmd.arg("/S").arg("/C").raw_arg(format!(
            "\"call \"{}\" >nul && \"{}\" {}\"",
            vcvars.display(),
            tool.display(),
            quoted.join(" ")
        ));
        return Ok(cmd);
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new(tool);
        cmd.args(args);
        Ok(cmd)
    }
}

fn kill_process_tree(pid: u32) {
    #[cfg(unix)]
    {
        // The child was started in its own process group. Ask nicely first;
        // escalate on a helper thread so the caller (a UI command) never blocks.
        unsafe {
            libc::killpg(pid as i32, libc::SIGTERM);
        }
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1500));
            unsafe {
                libc::killpg(pid as i32, libc::SIGKILL);
            }
        });
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .status();
    }
}

fn free_space(path: &Path) -> Option<u64> {
    fs4::available_space(path).ok()
}
