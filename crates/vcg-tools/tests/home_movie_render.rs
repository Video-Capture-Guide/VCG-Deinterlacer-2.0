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

/// Render four seconds of the sample through the generated script.
///
/// `None` when the sample tape or the bundled tools are not on this machine.
/// `name` keeps the scripts of tests running side by side apart.
fn render(name: &str, tweak: impl FnOnce(&mut Config)) -> Option<vcg_tools::run::Output> {
    let sample = beside_repo(&["..", "Sample Videos", "8mm to VHS to GVUSB2.avi"]);
    let deps = beside_repo(&["..", "_deps"]);
    if !sample.is_file() || !deps.is_dir() {
        eprintln!("skipped: sample tape or _deps not present");
        return None;
    }
    let tools = ToolPaths::discover(Some(&deps));
    let (Some(vspipe), Some(vs)) = (&tools.vspipe, &tools.vs_deps_dir) else {
        eprintln!("skipped: vspipe not in _deps");
        return None;
    };

    // Four seconds in the middle of the tape, the same stretch the cadence
    // was first measured on: 120 interlaced frames, 240 fields.
    let mut config = Config {
        input_path: sample.display().to_string(),
        format: VideoFormat::Ntsc,
        home_movie: true,
        home_movie_rate: FilmRate::Fps18,
        trim_ranges: vec![(600, 719)],
        ..Config::default()
    };
    tweak(&mut config);
    let mut script = generate_vpy(&config, &DepsPaths::new(vs.clone()));
    // The fields chosen, said out loud so that they can be checked: a frame
    // count alone is satisfied by plain decimation, which is exactly what
    // this path once was without anyone noticing.
    script.push_str(
        "\nimport sys as _t_sys\n\
         print(\"VCGPICKS \" + \" \".join(str(p) for p in _vcg_picks), file=_t_sys.stderr)\n\
         print(\"VCGPLAIN \" + \" \".join(str(min(int(m / _vcg_factor), _vcg_last)) \
         for m in range(len(_vcg_picks))), file=_t_sys.stderr)\n",
    );
    let dir = std::env::temp_dir().join("vcg-home-movie-test");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(format!("home-movie-{name}-{}.vpy", std::process::id()));
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
    Some(out)
}

/// The y4m header's rate token and the number of frames that followed it.
fn rate_and_frames(out: &vcg_tools::run::Output) -> (String, usize) {
    let header = out.stdout.lines().next().unwrap_or_default();
    let rate = header
        .split(' ')
        .find(|t| t.starts_with('F'))
        .unwrap_or_default()
        .to_string();
    (rate, out.stdout.matches("FRAME").count())
}

/// The other two clocks, rendered: the same four seconds must come out as
/// four seconds of video-rate frames, or as the measured number of film
/// frames on an 18 fps clock.
#[test]
fn the_video_and_conformed_clocks_render() {
    use vcg_core::config::FilmOutput;
    let Some(video) = render("video", |c| {
        c.home_movie_fps = Some(18.5);
        c.home_movie_output = FilmOutput::Video;
    }) else {
        return;
    };
    // 240 fields is 4.004 s, which is 120 frames at 29.97.
    assert_eq!(rate_and_frames(&video), ("F30000:1001".to_string(), 120));

    let Some(conform) = render("conform", |c| {
        c.home_movie_fps = Some(18.5);
        c.home_movie_output = FilmOutput::Conform;
    }) else {
        return;
    };
    // 4.004 s of film at 18.5 fps is 74 frames; on an 18 fps clock they
    // run 4.11 s, which is the stretch.
    assert_eq!(rate_and_frames(&conform), ("F18:1".to_string(), 74));
}

#[test]
fn srestore_turns_120_frames_of_super8_into_72_frames_at_18_fps() {
    let Some(out) = render("measured", |_| {}) else {
        return;
    };
    // The y4m stream header names the rate sRestore set…
    let header = out.stdout.lines().next().unwrap_or_default().to_string();
    assert!(header.contains(" F18:1"), "y4m header: {header}");
    // …and one frame per film frame came out: 240 fields at 18/59.94.
    let frames = out.stdout.matches("FRAME").count();
    assert_eq!(frames, 72, "frames out of 120 in; vspipe said:\n{}", out.stderr);

    // …and they are sRestore's choices, not arithmetic.
    let list = |tag: &str| -> Vec<i64> {
        out.stderr
            .lines()
            .find_map(|l| l.strip_prefix(tag))
            .unwrap_or_else(|| panic!("no {tag} line in:\n{}", out.stderr))
            .split_whitespace()
            .map(|n| n.parse().expect("a field number"))
            .collect()
    };
    let (picks, plain) = (list("VCGPICKS "), list("VCGPLAIN "));
    assert_eq!(picks.len(), 72);
    let moved = picks.iter().zip(&plain).filter(|(a, b)| a != b).count();
    eprintln!("picks  {picks:?}\nplain  {plain:?}\nmoved {moved} of 72");
    assert!(
        moved >= 8,
        "only {moved} of 72 picks differ from plain decimation — sRestore is not deciding anything"
    );
    // One field per film frame: never backwards, and no field shown twice.
    assert!(picks.windows(2).all(|w| w[1] >= w[0]), "{picks:?}");
    let repeats = picks.windows(2).filter(|w| w[1] == w[0]).count();
    assert!(repeats <= 2, "{repeats} repeated fields in {picks:?}");
}
