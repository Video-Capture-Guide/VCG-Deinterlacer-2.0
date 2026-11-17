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
    render_into(name, tweak, "-")
}

/// The same, with the stream sent to `target`: "-" for stdout, which comes
/// back as text and is only good for counting, or a file when the bytes
/// themselves are to be compared.
fn render_into(
    name: &str,
    tweak: impl FnOnce(&mut Config),
    target: &str,
) -> Option<vcg_tools::run::Output> {
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
    cmd.args(["-c", "y4m", &path.display().to_string(), target]);
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

/// The Settings-page preview of a home movie, rendered: one rebuilt film
/// frame with the stage off, and one with it on.
///
/// Through `stage_frame` itself rather than the script alone, because the
/// frame number it asks vspipe for changes with the script — the film
/// preview puts its one frame out as frame 0 — and a mismatch there is a
/// preview of nothing, which only running it shows.
#[test]
fn the_settings_preview_of_a_home_movie_is_a_rebuilt_film_frame() {
    use vcg_core::config::NoiseLevel;
    use vcg_core::vpy::PreviewStage;
    use vcg_tools::{preview, probe};

    let sample = beside_repo(&["..", "Sample Videos", "8mm to VHS to GVUSB2.avi"]);
    let deps_dir = beside_repo(&["..", "_deps"]);
    if !sample.is_file() || !deps_dir.is_dir() {
        eprintln!("skipped: sample tape or _deps not present");
        return;
    }
    let tools = ToolPaths::discover(Some(&deps_dir));
    let Some(vs) = tools.vs_deps_dir.clone() else {
        eprintln!("skipped: vspipe not in _deps");
        return;
    };
    let deps = DepsPaths::new(vs);
    let report = probe::probe_file(&tools, &sample, probe::Options::container_only(), None);
    let info = report.info.expect("the sample has a video stream");

    let config = Config {
        input_path: sample.display().to_string(),
        format: VideoFormat::Ntsc,
        home_movie: true,
        home_movie_rate: FilmRate::Fps18,
        noise_level: NoiseLevel::Heavy,
        ..Config::default()
    };
    let frame = |apply: bool| {
        let started = std::time::Instant::now();
        let image = preview::stage_frame(preview::StageRequest {
            tools: &tools,
            deps: &deps,
            config: &config,
            file: &sample,
            info: &info,
            stage: PreviewStage::Noise,
            apply,
            at: 22.0,
            out: (640, 480),
        })
        .unwrap_or_else(|e| panic!("the film preview (apply={apply}) failed: {e}"));
        eprintln!("film preview, apply={apply}: {:.1}s", started.elapsed().as_secs_f64());
        image
    };
    let (off, on) = (frame(false), frame(true));
    assert_eq!(off.rgb.len(), 640 * 480 * 3);
    assert_eq!(on.rgb.len(), 640 * 480 * 3);

    // A picture, not a blank: the frame number asked for exists.
    let spread = |rgb: &[u8]| rgb.iter().max().unwrap() - rgb.iter().min().unwrap();
    assert!(spread(&off.rgb) > 60, "the untouched half is flat");

    // And the stage did something to it, without replacing it.
    let diff: f64 = off
        .rgb
        .iter()
        .zip(&on.rgb)
        .map(|(a, b)| (*a as i32 - *b as i32).unsigned_abs() as f64)
        .sum::<f64>()
        / off.rgb.len() as f64;
    eprintln!("mean difference with heavy denoising: {diff:.2}");
    assert!(diff > 0.1, "denoising changed nothing ({diff:.3})");
    assert!(diff < 12.0, "the two halves are different pictures ({diff:.1})");
}

/// Dust removal, rendered through the export script.
///
/// The text tests say the script calls the filter; this says VapourSynth can
/// run it — the right argument names, a bit depth every plugin in it
/// accepts — and that what comes out is the same film with a little of it
/// repaired, not a different number of frames or a different picture.
#[test]
fn film_dirt_removal_renders_and_repairs_only_a_little_of_each_frame() {
    use vcg_core::config::FilmDirt;
    // To files: stdout comes back as text, and bytes that are not text do
    // not survive that well enough to be compared one for one.
    let bytes = |name: &str, dirt: FilmDirt| -> Option<Vec<u8>> {
        let file = std::env::temp_dir()
            .join("vcg-home-movie-test")
            .join(format!("{name}-{}.y4m", std::process::id()));
        std::fs::create_dir_all(file.parent().unwrap()).expect("temp dir");
        render_into(name, |c| c.film_dirt = dirt, &file.display().to_string())?;
        let data = std::fs::read(&file).expect("the rendered stream");
        let _ = std::fs::remove_file(&file);
        Some(data)
    };
    let Some(plain) = bytes("dirt-off", FilmDirt::Off) else {
        return;
    };
    let Some(cleaned) = bytes("dirt-on", FilmDirt::Normal) else {
        return;
    };
    let frames = |d: &[u8]| d.windows(6).filter(|w| w == b"FRAME
").count();
    assert!(frames(&plain) >= 72, "{} frames", frames(&plain));

    // Same length of stream, so the bytes line up frame for frame.
    let (a, b) = (&plain[..], &cleaned[..]);
    assert_eq!(a.len(), b.len(), "the two renders are different sizes");
    // By more than two levels. The last step of the script is error-diffusion
    // dither, which carries each pixel's rounding error into its neighbours,
    // so one repaired speck moves pixels a level either way for some distance
    // around it. Those are not repairs and are not counted as repairs.
    let touched = a.iter().zip(b).filter(|(x, y)| x != y).count();
    let changed = a.iter().zip(b).filter(|(x, y)| x.abs_diff(**y) > 2).count();
    let share = changed as f64 / a.len() as f64;
    eprintln!(
        "Normal dust removal: {:.3}% of bytes repaired, {:.3}% touched at all",
        share * 100.0,
        touched as f64 / a.len() as f64 * 100.0
    );
    assert!(changed > 0, "dust removal changed nothing at all");
    // Guarded: a filter that rewrote the picture would change most of it.
    assert!(share < 0.05, "{:.1}% of the stream changed", share * 100.0);
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
