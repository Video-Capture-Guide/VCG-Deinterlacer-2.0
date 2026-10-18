//! Verify the setting-screen previews render, and write them out to look at.
//!
//! usage: preview_filters <file> [logo.png]

use std::path::PathBuf;

use vcg_core::config::{Config, Watermark, WatermarkPosition};
use vcg_tools::paths::ToolPaths;
use vcg_tools::{preview, probe};

fn save(name: &str, img: &preview::Image) {
    let mut out = format!("P6\n{} {}\n255\n", img.width, img.height).into_bytes();
    out.extend_from_slice(&img.rgb);
    let path = std::env::temp_dir().join(name);
    std::fs::write(&path, out).expect("write");
    println!(
        "  {} -> {}x{}  {}",
        name,
        img.width,
        img.height,
        path.display()
    );
}

fn main() {
    let mut args = std::env::args().skip(1);
    let file = PathBuf::from(args.next().expect("usage: preview_filters <file> [logo]"));
    let logo = args.next();

    let tools = ToolPaths::discover(None);
    let report = probe::probe_file(&tools, &file, probe::Options::container_only(), None);
    let info = report.info.expect("a video stream");
    let at = info.duration * 0.45;
    let (w, h) = (640usize, 480usize);

    let mut config = Config {
        input_path: file.display().to_string(),
        ..Config::default()
    };

    println!("chroma shift:");
    for delay in [0i32, 4] {
        let filter = preview::chroma_shift_filter(delay);
        println!("  delay {delay:+} -> filter {filter:?}");
        match preview::filtered_frame(&tools, &config, &file, at, w, h, &filter) {
            Ok(img) => save(&format!("vcg_yc_{delay}.ppm"), &img),
            Err(e) => println!("  FAILED: {e}"),
        }
    }

    println!("watermark (text):");
    config.watermark = Watermark::Text {
        text: "Smith family, 1998".into(),
        fontsize: 36,
    };
    config.wm_position = WatermarkPosition::BottomRight;
    config.wm_opacity = 0.8;
    match preview::watermark_frame(&tools, &config, &file, at, w, h) {
        Ok(img) => save("vcg_wm_text.ppm", &img),
        Err(e) => println!("  FAILED: {e}"),
    }

    // The real filter stages, before and after, through VapourSynth.
    println!("filter stages:");
    let deps = vcg_core::vpy::DepsPaths::new(
        tools.vs_deps_dir.clone().unwrap_or_else(|| PathBuf::from("_deps")),
    );
    let mut staged = Config {
        input_path: file.display().to_string(),
        levels_adjustment: vcg_core::config::LevelsAdjustment::Stretch,
        noise_level: vcg_core::config::NoiseLevel::Heavy,
        dehalo_mode: vcg_core::config::DehaloMode::Strong,
        grain_strength: 3.0,
        ..Config::default()
    };
    staged.color_correction = vcg_core::config::ColorCorrection::BoostSat;
    for stage in [
        vcg_core::vpy::PreviewStage::Levels,
        vcg_core::vpy::PreviewStage::ColorTint,
        vcg_core::vpy::PreviewStage::FilmGrain,
        vcg_core::vpy::PreviewStage::Halos,
        vcg_core::vpy::PreviewStage::Noise,
    ] {
        let t = std::time::Instant::now();
        let mk = |apply| preview::StageRequest {
            tools: &tools, deps: &deps, config: &staged, file: &file, info: &info,
            stage, apply, at, out: (w, h),
        };
        let before = preview::stage_frame(mk(false));
        let after = preview::stage_frame(mk(true));
        match (before, after) {
            (Ok(b), Ok(a)) => {
                let diff: f64 = b
                    .rgb
                    .iter()
                    .zip(a.rgb.iter())
                    .map(|(x, y)| (*x as i32 - *y as i32).unsigned_abs() as f64)
                    .sum::<f64>()
                    / b.rgb.len() as f64;
                println!(
                    "  {:<7} {:.1}s  mean difference {diff:.2}",
                    stage.id(),
                    t.elapsed().as_secs_f64()
                );
                save(&format!("vcg_stage_{}_off.ppm", stage.id()), &b);
                save(&format!("vcg_stage_{}_on.ppm", stage.id()), &a);
            }
            (b, a) => println!(
                "  {:<7} FAILED  before={:?} after={:?}",
                stage.id(),
                b.err(),
                a.err()
            ),
        }
    }

    if let Some(logo) = logo {
        println!("watermark (logo):");
        config.watermark = Watermark::Logo {
            logo_path: logo,
            logo_size: 0.2,
        };
        match preview::watermark_frame(&tools, &config, &file, at, w, h) {
            Ok(img) => save("vcg_wm_logo.ppm", &img),
            Err(e) => println!("  FAILED: {e}"),
        }
    }
}
