//! Headless integration tests for the build pipeline.
//!
//! `resolve_release` is a quick network test; `full_build` runs the entire
//! real pipeline (download, compile, install, launcher, cleanup) and takes
//! a long time. Both are #[ignore]d — run explicitly:
//!
//!   cargo test --test full_build -- --ignored --nocapture resolve_release
//!   cargo test --test full_build -- --ignored --nocapture full_build

use aseprite_compiler_lib::pipeline::{Engine, PipelineState, Sink, StageStatus};
use aseprite_compiler_lib::settings::{Channel, Settings};
use std::time::Duration;

struct StderrSink;

impl Sink for StderrSink {
    fn emit_state(&self, state: &PipelineState) {
        if let Some(s) = state.stages.iter().find(|s| s.status == StageStatus::Running) {
            eprintln!(
                "[stage] {} {} {}",
                s.name,
                s.progress
                    .map(|p| format!("{:>3.0}%", p * 100.0))
                    .unwrap_or_else(|| "  --".into()),
                s.detail
            );
        }
    }

    fn emit_log(&self, line: &str) {
        eprintln!("[log] {line}");
    }

    fn emit_status(&self, _busy: bool) {}

    fn notify(&self, title: &str, body: &str) {
        eprintln!("[notify] {title}: {body}");
    }
}

#[test]
#[ignore]
fn resolve_release() {
    let stable = aseprite_compiler_lib::github::fetch_latest(Channel::Stable).unwrap();
    eprintln!("stable: {stable:?}");
    assert!(stable.source_zip_url.ends_with("-Source.zip"));
    assert!(!stable.version.is_empty());

    // The digest field is what source verification relies on — assert it's
    // present for the current stable release.
    assert!(
        stable.source_zip_sha256.is_some(),
        "expected GitHub to publish a sha256 digest for the source zip"
    );

    let skia = aseprite_compiler_lib::github::skia_asset_exact("m124-08a5439a6b").unwrap();
    eprintln!("skia exact: {} {} {}", skia.tag, skia.asset_name, skia.url);
    assert_eq!(skia.tag, "m124-08a5439a6b");
    assert!(skia.url.starts_with("https://"));

    // A nonexistent pin must fail — never fall back to a different version.
    assert!(aseprite_compiler_lib::github::skia_asset_exact("m999-doesnotexist").is_err());
}

#[test]
fn skia_tag_scan() {
    let text = "download it from https://github.com/aseprite/skia/releases/tag/m124-08a5439a6b page";
    assert_eq!(
        aseprite_compiler_lib::pipeline::scan_skia_tag(text).as_deref(),
        Some("m124-08a5439a6b")
    );
    assert_eq!(aseprite_compiler_lib::pipeline::scan_skia_tag("no tags here m12"), None);
}

/// Force-provision the portable tools (hiding any system copies via PATH)
/// and verify they run after the CMake distribution is pruned.
#[test]
#[ignore]
fn provision_portable() {
    std::env::set_var("PATH", "");
    let cancel = std::sync::atomic::AtomicBool::new(false);
    aseprite_compiler_lib::toolchain::provision(&cancel, |m| eprintln!("[provision] {m}"))
        .expect("provision");

    let cmake = aseprite_compiler_lib::toolchain::require_cmake().unwrap();
    let ninja = aseprite_compiler_lib::toolchain::require_ninja().unwrap();
    assert!(!aseprite_compiler_lib::toolchain::tool_version(&cmake).is_empty());
    assert!(!aseprite_compiler_lib::toolchain::tool_version(&ninja).is_empty());
    // The pruned distribution must still have its Modules payload.
    let share = cmake.parent().unwrap().parent().unwrap().join("share");
    let modules = std::fs::read_dir(&share)
        .unwrap()
        .flatten()
        .find(|e| e.file_name().to_string_lossy().starts_with("cmake-"))
        .map(|e| e.path().join("Modules"))
        .expect("cmake-X.Y under share");
    assert!(modules.is_dir(), "Modules must survive pruning");
    eprintln!("portable cmake: {}", cmake.display());
}

#[test]
#[ignore]
fn full_build() {
    let engine = Engine::new(Box::new(StderrSink));
    let mut settings = Settings::default(); // cleanup on, stable channel, default install dir
    settings.use_ccache = std::env::var_os("TEST_CCACHE").is_some();
    engine.start(settings).expect("start pipeline");

    while engine.running() {
        std::thread::sleep(Duration::from_secs(2));
    }

    let st = engine.snapshot();
    for s in &st.stages {
        eprintln!("stage {:<10} {:?} {}", s.id, s.status, s.detail);
    }
    if let Some(err) = &st.error {
        panic!("pipeline failed during {:?}: {err}", st.failed_stage);
    }
    let summary = st.summary.expect("summary");
    eprintln!(
        "OK: installed Aseprite {} in {}s ({} bytes installed, {} bytes cleaned)",
        summary.version, summary.elapsed_secs, summary.installed_bytes, summary.cleaned_bytes
    );
    assert!(!summary.version.is_empty());
    assert!(summary.installed_bytes > 10_000_000);
}
