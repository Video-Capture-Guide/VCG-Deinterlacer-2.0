//! Verify the auto-crop detector against real files.
//!
//! Prints the recommendation next to what the fixed presets would have done,
//! which is the comparison that matters: the presets are what a user gets today.

use std::path::PathBuf;

use vcg_core::analysis::edges::Detection;
use vcg_core::config::{Config, Mode};
use vcg_core::vpy::{generate_vpy, DepsPaths};
use vcg_tools::apply::apply_report;
use vcg_tools::paths::ToolPaths;
use vcg_tools::{edges, probe};

fn main() {
    let mut args = std::env::args().skip(1);
    let file = PathBuf::from(args.next().expect("usage: crop_check <file> [expected]"));
    let expected = args.next();

    let tools = ToolPaths::discover(None);
    let report = probe::probe_file(&tools, &file, probe::Options::container_only(), None);
    let info = report.info.expect("a video stream");
    println!(
        "{}\n  {}x{} @ {:.3} fps, {:.1}s",
        file.display(),
        info.width,
        info.height,
        info.fps,
        info.duration
    );

    let started = std::time::Instant::now();
    let (detection, hist) = edges::detect_crop_and_levels(&tools, &file, &info);
    {
        let lo = hist.bins.iter().position(|&n| n > 0).unwrap_or(0);
        let hi = 255 - hist.bins.iter().rev().position(|&n| n > 0).unwrap_or(0);
        let total: u64 = hist.bins.iter().sum();
        let under: u64 = hist.bins[..16].iter().sum();
        println!(
            "  levels: min={lo} max={hi} total={total} pixels under 16 = {under} ({:.2}%)",
            under as f64 / total as f64 * 100.0
        );
        println!("  bins 0..20: {:?}", &hist.bins[..20]);
    }
    println!(
        "  levels: {} frames, p1={} p99={}, below legal {:.2}%, above {:.2}%",
        hist.frames,
        hist.percentile(0.01),
        hist.percentile(0.99),
        hist.below_legal() * 100.0,
        hist.above_legal() * 100.0
    );
    let elapsed = started.elapsed();

    match detection {
        Detection::Inconclusive(why) => println!("  inconclusive: {why}"),
        Detection::Found(c) => {
            println!(
                "  detected   left={} right={} top={} bottom={}  -> {}x{}",
                c.left,
                c.right,
                c.top,
                c.bottom,
                info.width - c.left - c.right,
                info.height - c.top - c.bottom
            );
            if c.noise_rows > 0 {
                println!("  of which {} bottom rows are head-switching noise", c.noise_rows);
            }
            if c.is_lopsided() {
                println!("  sides differ by {} px", c.left.abs_diff(c.right));
            }
            if c.is_nothing() {
                println!("  nothing to crop: the frame is already all picture");
            }
            // What the user would get today.
            println!("  BT.601 preset would crop left=8 right=8 top=0 bottom=0");
        }
    }
    println!("  measured in {:.1}s", elapsed.as_secs_f64());
    if let Some(e) = expected {
        println!("  EXPECTED   {e}");
    }

    // The measurement is only worth anything if it reaches the script that
    // actually renders. Apply it the way the wizard does, then show the crop
    // lines the generator emits.
    // The container read already gave classification and PAR, which is all
    // the crop path needs; no point re-running the measurement.
    let mut full = report;
    full.edges = Some(detection);
    let mut config = Config {
        input_path: file.display().to_string(),
        ..Config::default()
    };
    apply_report(&mut config, &full, Mode::JustDeinterlace);
    println!(
        "  config: preset={:?} left={} right={} top={} bottom={}",
        config.crop_preset, config.crop_left, config.crop_right, config.crop_top,
        config.crop_bottom
    );

    let deps = DepsPaths::new(
        tools.vs_deps_dir.clone().unwrap_or_else(|| PathBuf::from("_deps")),
    );
    let script = generate_vpy(&config, &deps);
    println!("  --- crop and sizing lines from the generated .vpy ---");
    for line in script.lines() {
        let l = line.trim();
        if l.contains("Crop(") || l.contains("Spline36(") || l.contains("_vcg_") {
            println!("    {l}");
        }
    }
}
