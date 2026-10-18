//! Run a real export end to end and verify the result.
//!
//! usage: export_check <file> [out.mov]

use std::path::PathBuf;

use vcg_core::config::{Config, Mode};
use vcg_core::vpy::DepsPaths;
use vcg_tools::apply::{apply_report, options_for};
use vcg_tools::encode::{self, Phase};
use vcg_tools::paths::ToolPaths;
use vcg_tools::probe;

fn main() {
    let file = PathBuf::from(std::env::args().nth(1).expect("usage: export_check <file>"));
    let out = PathBuf::from(
        std::env::args()
            .nth(2)
            .unwrap_or_else(|| std::env::temp_dir().join("vcg_export.mov").display().to_string()),
    );

    let tools = ToolPaths::discover(None);
    let deps = DepsPaths::new(tools.vs_deps_dir.clone().expect("portable VapourSynth"));
    println!("vspipe {:?}", tools.vspipe.as_ref().map(|t| &t.path));

    // Analyze first, so the config matches what the app would export with.
    let mode = Mode::JustDeinterlace;
    let report = probe::probe_file(&tools, &file, options_for(mode), None);
    let info = report.info.expect("a video stream");
    let mut config = Config::default();
    apply_report(&mut config, &report, mode);
    // Trimmed to two seconds unless "full" is passed, so the common case can
    // be checked too.
    // "logo" renders with a logo watermark, to prove the overlay and the
    // sound can coexist — they could not, and the failure was silent.
    if let Some(i) = std::env::args().position(|a| a == "logo") {
        let path = std::env::args().nth(i + 1).expect("logo needs a path");
        config.watermark = vcg_core::config::Watermark::Logo {
            logo_path: path,
            logo_size: 0.15,
        };
        config.wm_opacity = 0.8;
    }
    let full = std::env::args().any(|a| a == "full");
    if !full {
        let frames = (info.fps * 2.0).round() as i64;
        config.trim_ranges = vec![(0, frames - 1)];
    }

    println!(
        "source {}x{} {:.3} fps {:.1}s -> exporting {}",
        info.width,
        info.height,
        info.fps,
        info.duration,
        if full { "the whole file".to_string() } else { "2s".to_string() }
    );

    let started = std::time::Instant::now();
    let mut last = -1i64;
    let mut report_progress = |p: Phase| {
        if let Phase::Encoding { out_seconds, total_seconds, fps, speed } = p {
            let pct = (p.fraction() * 100.0) as i64;
            if pct != last {
                last = pct;
                println!(
                    "  {pct:>3}%  {out_seconds:.1}/{total_seconds:.1}s  {fps:.0} fps  {speed:.2}x"
                );
            }
        }
    };

    match encode::run(
        encode::Request {
            tools: &tools,
            deps: &deps,
            config: &config,
            source: &file,
            source_seconds: info.duration,
            source_fps: info.fps,
            output: &out,
            comparison: std::env::args().any(|a| a == "--comparison"),
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        },
        Some(&mut report_progress),
    ) {
        Ok(o) => {
            println!(
                "OK in {:.1}s — {} ({:.2}s, audio: {})",
                started.elapsed().as_secs_f64(),
                o.output.display(),
                o.duration,
                o.had_audio
            );
            if let Some(clip) = &o.comparison {
                println!("  comparison: {}", clip.display());
            }
        }
        Err(why) => println!("FAILED: {why}"),
    }
}
