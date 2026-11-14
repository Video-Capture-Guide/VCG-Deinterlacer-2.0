//! The home-movie script, rendered.
//!
//! The text tests in vcg-core prove the script says `haf.srestore`; this one
//! proves VapourSynth can run what it says, on the tape the feature was built
//! for. A script can be textually right and still fail to make a frame — a
//! plugin call with the wrong argument name, a filter fed the wrong bit
//! depth — and vspipe is the only judge of that.
//!
//! Needs the bundled `_deps` and the sample tape beside the repository, so it
//! passes quietly on a machine without them rather than failing on absence.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use vcg_core::config::{Config, FilmRate, VideoFormat};
use vcg_core::vpy::{generate_vpy, DepsPaths};
use vcg_tools::paths::ToolPaths;

fn beside_repo(rel: &[&str]) -> PathBuf {
    let mut p: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", ".."].iter().collect();
    p.extend(rel);
    p
}

#[test]
fn srestore_turns_120_frames_of_super8_into_72_frames_at_18_fps() {
    let sample = beside_repo(&["..", "Sample Videos", "8mm to VHS to GVUSB2.avi"]);
    let deps = beside_repo(&["..", "_deps"]);
    if !sample.is_file() || !deps.is_dir() {
        eprintln!("skipped: sample tape or _deps not present");
        return;
    }
    let tools = ToolPaths::discover(Some(&deps));
    let (Some(vspipe), Some(vs)) = (&tools.vspipe, &tools.vs_deps_dir) else {
        eprintln!("skipped: vspipe not in _deps");
        return;
    };

    // Four seconds in the middle of the tape, the same stretch the cadence
    // was first measured on: 120 interlaced frames, 240 fields.
    let config = Config {
        input_path: sample.display().to_string(),
        format: VideoFormat::Ntsc,
        home_movie: true,
        home_movie_rate: FilmRate::Fps18,
        trim_ranges: vec![(600, 719)],
        ..Config::default()
    };
    let script = generate_vpy(&config, &DepsPaths::new(vs.clone()));
    let dir = std::env::temp_dir().join("vcg-home-movie-test");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(format!("home-movie-{}.vpy", std::process::id()));
    std::fs::write(&path, script.as_bytes()).expect("write script");

    let mut cmd = vcg_tools::run::command(&vspipe.path);
    cmd.args(["-c", "y4m", &path.display().to_string(), "-"]);
    for key in tools.vspipe_env_removals() {
        cmd.env_remove(key);
    }
    for (key, value) in tools.vspipe_env() {
        cmd.env(key, value);
    }
    cmd.stdin(Stdio::null());
    let out = vcg_tools::run::run(cmd, Duration::from_secs(300)).expect("vspipe starts");
    let _ = std::fs::remove_file(&path);

    assert!(
        out.success(),
        "vspipe failed (status {:?}):\n{}",
        out.status,
        out.stderr
    );
    // The y4m stream header names the rate sRestore set…
    let header = out.stdout.lines().next().unwrap_or_default().to_string();
    assert!(header.contains(" F18:1"), "y4m header: {header}");
    // …and one frame per film frame came out: 240 fields at 18/59.94.
    let frames = out.stdout.matches("FRAME").count();
    assert_eq!(frames, 72, "frames out of 120 in; vspipe said:\n{}", out.stderr);
}
