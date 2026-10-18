//! Verify the before/after preview end to end against a real file.
//!
//! Writes both frames as PPM so they can be opened and compared by eye.

use std::path::PathBuf;

use vcg_core::config::Config;
use vcg_core::vpy::DepsPaths;
use vcg_tools::paths::ToolPaths;
use vcg_tools::{preview, probe};

fn write_ppm(path: &PathBuf, img: &preview::Image) -> std::io::Result<()> {
    let mut out = format!("P6\n{} {}\n255\n", img.width, img.height).into_bytes();
    out.extend_from_slice(&img.rgb);
    std::fs::write(path, out)
}

fn main() {
    let file = PathBuf::from(std::env::args().nth(1).expect("usage: preview_check <file>"));
    let deps_arg = std::env::args().nth(2);

    let tools = ToolPaths::discover(deps_arg.as_deref().map(std::path::Path::new));
    let deps = DepsPaths::new(
        tools
            .vs_deps_dir
            .clone()
            .expect("a portable VapourSynth directory is required"),
    );

    println!("ffmpeg  {:?}", tools.ffmpeg.as_ref().map(|t| &t.path));
    println!("vspipe  {:?}", tools.vspipe.as_ref().map(|t| &t.path));

    let report = probe::probe_file(&tools, &file, probe::Options::container_only(), None);
    let info = report.info.expect("a video stream");
    println!("{}x{} @ {:.3} fps, {:.1}s", info.width, info.height, info.fps, info.duration);

    let (rw, rh) = (360usize, 240usize);
    let started = std::time::Instant::now();
    match preview::find_spot(&tools, &file, &info, rw, rh, 0.0) {
        Some(s) => println!(
            "most combed: {:.2}s at ({},{}) {}x{} — {:.2}% of pixels",
            s.at_seconds, s.x, s.y, s.width, s.height, s.score * 100.0
        ),
        None => println!("nothing combed enough to show"),
    }
    println!("scoring took {:.1}s", started.elapsed().as_secs_f64());

    let config = Config::default();
    let started = std::time::Instant::now();
    match preview::comparison(&tools, &deps, &config, &file, &info, rw, rh) {
        Ok(c) => {
            let dir = std::env::temp_dir();
            let b = dir.join("vcg_preview_before.ppm");
            let a = dir.join("vcg_preview_after.ppm");
            write_ppm(&b, &c.before).expect("write before");
            write_ppm(&a, &c.after).expect("write after");
            println!(
                "rendered {}x{} in {:.1}s",
                c.before.width,
                c.before.height,
                started.elapsed().as_secs_f64()
            );
            println!("  before: {}", b.display());
            println!("  after:  {}", a.display());
        }
        Err(why) => println!("comparison unavailable: {why}"),
    }
}
