//! Rendering the before/after deinterlacing comparison.
//!
//! A newcomer has no idea what interlacing looks like, so the screen shows it:
//! one region of one frame from their own tape as captured, next to the same
//! region after QTGMC.
//!
//! **The comparison must be a crop, never a resize.** Combing is a
//! one-pixel-period vertical pattern, so it only survives at native
//! resolution — scaling 480 lines down to 214 aliases the comb into noise and
//! the two pictures then look equally busy. Measured on a real capture, a
//! downscaled pair differed by 14%; cropped at native resolution the
//! difference is unmistakable. So the frame is scored at full resolution, the
//! most combed region is located, and that region is cropped from both.
//!
//! Every step can fail harmlessly: anything that goes wrong returns an error
//! and the caller omits the picture rather than showing a placeholder. The
//! error has to reach the screen, though — it was being recorded on the job
//! and dropped, so a failure after a two-minute render looked exactly like a
//! render still running. See `check::preview_failed`.

use std::path::Path;
use std::time::Duration;

use vcg_core::analysis::probe::VideoInfo;
use vcg_core::config::{Config, VideoFormat};
use vcg_core::vpy::{
    generate_preview_vpy, generate_stage_preview_vpy, DepsPaths, PreviewStage,
};

use crate::paths::ToolPaths;
use crate::run::{run_piped, run_tool_binary, DEFAULT_TIMEOUT};

/// QTGMC on one frame is slow — it decodes and motion-analyzes a window of
/// neighbors — so it gets considerably longer than a probe.
const RENDER_TIMEOUT: Duration = Duration::from_secs(120);

/// How many frames to score when looking for motion.
const CANDIDATES: usize = 9;

/// A pixel counts as combed when it differs from *both* vertical neighbors by
/// this much. A plain horizontal edge differs from only one, which is why this
/// is not simply a gradient threshold.
const COMB_MARGIN: i32 = 14;

/// The matrix ffmpeg should assume when converting YUV to RGB.
///
/// Analog captures are almost never tagged, so leaving this to `auto` makes the
/// two halves disagree on color: the source frame gets guessed from its
/// dimensions while the y4m from vspipe carries no tag at all.
fn in_matrix(config: &Config) -> &'static str {
    match config.format {
        // NTSC SMPTE170M and PAL BT.470BG are both the BT.601 coefficients.
        VideoFormat::Ntsc | VideoFormat::Pal => "bt601",
    }
}

/// One frame region, as RGB, ready to upload as a texture.
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
}

/// The moment and region worth showing.
#[derive(Debug, Clone, Copy)]
pub struct Spot {
    pub at_seconds: f64,
    /// Top-left of the crop, in source pixels.
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    /// Fraction of pixels in the region that are combed.
    pub score: f64,
}

/// The finished comparison.
pub struct Comparison {
    pub before: Image,
    pub after: Image,
    pub spot: Spot,
}

/// Measure how combed a whole grayscale frame is.
pub fn comb_score(luma: &[u8], width: usize, height: usize) -> f64 {
    if width == 0 || height < 3 {
        return 0.0;
    }
    let mut combed = 0u64;
    let mut considered = 0u64;
    for y in 1..height - 1 {
        for x in 0..width {
            considered += 1;
            if is_combed(luma, width, x, y) {
                combed += 1;
            }
        }
    }
    if considered == 0 {
        0.0
    } else {
        combed as f64 / considered as f64
    }
}

fn is_combed(luma: &[u8], width: usize, x: usize, y: usize) -> bool {
    let up = luma[(y - 1) * width + x] as i32;
    let mid = luma[y * width + x] as i32;
    let down = luma[(y + 1) * width + x] as i32;
    let a = mid - up;
    let b = mid - down;
    (a > COMB_MARGIN && b > COMB_MARGIN) || (a < -COMB_MARGIN && b < -COMB_MARGIN)
}

/// Find the most combed region of a frame.
///
/// Builds a comb map, then an integral image over it, so scoring every
/// candidate window costs four lookups instead of a rescan.
fn best_region(
    luma: &[u8],
    width: usize,
    height: usize,
    region_w: usize,
    region_h: usize,
) -> (usize, usize, f64) {
    if width < region_w || height < region_h || height < 3 {
        return (0, 0, comb_score(luma, width, height));
    }

    let mut map = vec![0u32; width * height];
    for y in 1..height - 1 {
        for x in 0..width {
            if is_combed(luma, width, x, y) {
                map[y * width + x] = 1;
            }
        }
    }

    let stride = width + 1;
    let mut sum = vec![0u32; stride * (height + 1)];
    for y in 0..height {
        let mut row = 0u32;
        for x in 0..width {
            row += map[y * width + x];
            sum[(y + 1) * stride + x + 1] = sum[y * stride + x + 1] + row;
        }
    }

    let step_x = (region_w / 3).max(1);
    let step_y = (region_h / 3).max(1);
    let mut best = (0usize, 0usize, 0u32);
    let mut y = 0;
    while y + region_h <= height {
        let mut x = 0;
        while x + region_w <= width {
            let total = sum[(y + region_h) * stride + x + region_w] + sum[y * stride + x]
                - sum[y * stride + x + region_w]
                - sum[(y + region_h) * stride + x];
            if total > best.2 {
                best = (x, y, total);
            }
            x += step_x;
        }
        y += step_y;
    }

    let area = (region_w * region_h) as f64;
    (best.0, best.1, best.2 as f64 / area)
}

/// Grab one whole frame as grayscale at `width`.
fn grab_luma(
    tools: &ToolPaths,
    file: &Path,
    info: &VideoInfo,
    at: f64,
    width: usize,
) -> Option<(Vec<u8>, usize, usize)> {
    let ffmpeg = tools.ffmpeg_path()?;
    if info.width == 0 || info.height == 0 {
        return None;
    }
    let height =
        ((width as f64 * info.height as f64 / info.width as f64).round() as usize).max(2);
    let height = height + (height % 2);
    let expected = width * height;

    let args: Vec<String> = vec![
        "-hide_banner".into(),
        "-fflags".into(),
        "+discardcorrupt".into(),
        "-ss".into(),
        format!("{at}"),
        "-i".into(),
        file.display().to_string(),
        "-vframes".into(),
        "1".into(),
        "-vf".into(),
        format!("scale={width}:{height}:flags=neighbor,format=gray"),
        "-pix_fmt".into(),
        "gray".into(),
        "-f".into(),
        "rawvideo".into(),
        "-".into(),
    ];

    let (_, data) = run_tool_binary(ffmpeg, &args, DEFAULT_TIMEOUT).ok()?;
    if data.len() < expected {
        return None;
    }
    Some((data[..expected].to_vec(), width, height))
}

/// Below this a region is not worth showing: a static shot, or material that
/// is already progressive.
///
/// Only consulted when nothing better is known. See `find_spot`.
const MIN_COMB_SCORE: f64 = 0.01;

/// Find the most combed moment and region, scoring at native resolution.
///
/// `min_score` is the fraction of combed pixels the best region must carry
/// before it is worth showing. Pass 0.0 when the file is already known to be
/// interlaced: the scan-type probe looked at far more of the tape than these
/// nine frames did, and refusing a preview on this metric's say-so is
/// second-guessing it. Reported as "takes a long time and then doesn't load
/// the preview" — this threshold was one of the two ways that happened, and
/// the one that happens on a tape with gentle motion in it.
pub fn find_spot(
    tools: &ToolPaths,
    file: &Path,
    info: &VideoInfo,
    region_w: usize,
    region_h: usize,
    min_score: f64,
) -> Option<Spot> {
    if info.duration <= 0.0 || info.width == 0 {
        return None;
    }
    let mut best: Option<Spot> = None;

    for i in 0..CANDIDATES {
        // Across the middle of the running time: leaders and black frames live
        // at the ends and show nothing.
        let fraction = 0.15 + 0.7 * (i as f64 / (CANDIDATES - 1) as f64);
        let at = info.duration * fraction;
        let Some((luma, w, h)) = grab_luma(tools, file, info, at, info.width as usize) else {
            continue;
        };
        let rw = region_w.min(w);
        let rh = region_h.min(h);
        let (x, y, score) = best_region(&luma, w, h, rw, rh);
        if best.is_none_or(|b| score > b.score) {
            best = Some(Spot {
                at_seconds: at,
                x,
                y,
                width: rw,
                height: rh,
                score,
            });
        }
    }

    best.filter(|s| s.score > min_score)
}

/// The filter chain that crops the region and converts to RGB.
///
/// A crop, never a scale — see the module note.
fn crop_filter(spot: &Spot, config: &Config) -> String {
    format!(
        "crop={}:{}:{}:{},scale=in_color_matrix={}",
        spot.width,
        spot.height,
        spot.x,
        spot.y,
        in_matrix(config)
    )
}

/// Crop the region from the file as captured — already woven, which is the
/// point.
fn grab_before(
    tools: &ToolPaths,
    config: &Config,
    file: &Path,
    spot: &Spot,
) -> Option<Image> {
    let ffmpeg = tools.ffmpeg_path()?;
    let expected = spot.width * spot.height * 3;

    let args: Vec<String> = vec![
        "-hide_banner".into(),
        "-fflags".into(),
        "+discardcorrupt".into(),
        "-ss".into(),
        format!("{}", spot.at_seconds),
        "-i".into(),
        file.display().to_string(),
        "-vframes".into(),
        "1".into(),
        "-vf".into(),
        crop_filter(spot, config),
        "-pix_fmt".into(),
        "rgb24".into(),
        "-f".into(),
        "rawvideo".into(),
        "-".into(),
    ];

    let (_, data) = run_tool_binary(ffmpeg, &args, DEFAULT_TIMEOUT).ok()?;
    if data.len() < expected {
        return None;
    }
    Some(Image {
        width: spot.width,
        height: spot.height,
        rgb: data[..expected].to_vec(),
    })
}

/// Render the same region through QTGMC.
///
/// `vspipe` is asked for a single frame by number, so VapourSynth evaluates
/// only that frame and the neighbors QTGMC needs rather than the whole clip.
fn render_after(
    tools: &ToolPaths,
    deps: &DepsPaths,
    config: &Config,
    spot: &Spot,
    frame: u64,
) -> Result<Image, String> {
    let vspipe = tools
        .vspipe
        .as_ref()
        .ok_or("VapourSynth was not found")?
        .path
        .clone();
    let ffmpeg = tools
        .ffmpeg_path()
        .ok_or("ffmpeg was not found")?
        .to_path_buf();

    let script = generate_preview_vpy(config, deps);
    // A temp file, because vspipe takes a script path rather than stdin.
    let dir = std::env::temp_dir().join("vcg-preview");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let script_path = dir.join(format!("preview-{}.vpy", std::process::id()));
    std::fs::write(&script_path, script.as_bytes()).map_err(|e| e.to_string())?;

    let expected = spot.width * spot.height * 3;

    let mut producer = crate::run::command(&vspipe);
    producer.args([
        "--start".to_string(),
        frame.to_string(),
        "--end".to_string(),
        frame.to_string(),
        // y4m carries only YUV or Gray, which is why the script outputs
        // YUV444P8 and the RGB conversion happens here in ffmpeg.
        "--container".to_string(),
        "y4m".to_string(),
        script_path.display().to_string(),
        "-".to_string(),
    ]);
    for key in tools.vspipe_env_removals() {
        producer.env_remove(key);
    }
    for (key, value) in tools.vspipe_env() {
        producer.env(key, value);
    }

    let mut consumer = crate::run::command(&ffmpeg);
    consumer.args([
        "-hide_banner".to_string(),
        "-i".to_string(),
        "-".to_string(),
        "-vframes".to_string(),
        "1".to_string(),
        "-vf".to_string(),
        crop_filter(spot, config),
        "-pix_fmt".to_string(),
        "rgb24".to_string(),
        "-f".to_string(),
        "rawvideo".to_string(),
        "-".to_string(),
    ]);

    let result = run_piped(producer, consumer, RENDER_TIMEOUT);

    let (data, stderr) = match result {
        Ok(v) => v,
        Err(e) => {
            let _ = std::fs::remove_file(&script_path);
            return Err(format!("could not start vspipe: {e}"));
        }
    };
    if data.len() < expected {
        // The script is kept on failure so it can be inspected.
        return Err(format!(
            "vspipe produced {} of {expected} bytes. Script: {}\n{}",
            data.len(),
            script_path.display(),
            stderr.trim()
        ));
    }
    let _ = std::fs::remove_file(&script_path);

    Ok(Image {
        width: spot.width,
        height: spot.height,
        rgb: data[..expected].to_vec(),
    })
}

/// Build the before/after comparison for a region of `region_w` x `region_h`.
pub fn comparison(
    tools: &ToolPaths,
    deps: &DepsPaths,
    config: &Config,
    file: &Path,
    info: &VideoInfo,
    region_w: usize,
    region_h: usize,
) -> Result<Comparison, String> {
    // The file's own scan type decides whether the comb threshold applies.
    // An interlaced file will be deinterlaced whatever these nine frames
    // measure, so the preview shows the most combed moment it found rather
    // than refusing outright — the alternative is a page that silently has no
    // before-and-after on exactly the tapes it was built for.
    let min_score = if config.field_order == vcg_core::config::FieldOrder::Progressive {
        MIN_COMB_SCORE
    } else {
        0.0
    };
    let spot = find_spot(tools, file, info, region_w, region_h, min_score)
        .ok_or("no frame could be read to compare")?;

    // The generated script reads its source path from the config, so it is set
    // from the file being previewed rather than trusted to already match. An
    // empty input_path produces a script that raises "All source loaders
    // failed" with nothing after the colon.
    let mut config = config.clone();
    config.input_path = file.display().to_string();

    let before = grab_before(tools, &config, file, &spot)
        .ok_or("could not grab the source frame")?;

    // Timestamp to frame index: the script numbers from the start of the file.
    let frame = (spot.at_seconds * info.fps).round().max(0.0) as u64;
    let after = render_after(tools, deps, &config, &spot, frame)?;

    Ok(Comparison {
        before,
        after,
        spot,
    })
}

/// Grab one whole frame as RGB at native resolution, for the Crop page.
///
/// Native resolution matters here for a different reason than the comb
/// comparison: the crop overlay is drawn in source pixels, so a scaled frame
/// would put the guide lines in the wrong place. The texture is scaled for
/// display instead, where a blurred edge costs nothing.
///
/// The moment is taken from the middle of the running time, away from the
/// leader and the tail where a black frame would show no edges at all.
pub fn whole_frame(
    tools: &ToolPaths,
    config: &Config,
    file: &Path,
    info: &VideoInfo,
) -> Result<Image, String> {
    // A moment in the middle, away from the leader and the tail where a black
    // frame would show no edges at all.
    let at = if info.duration > 0.0 { info.duration * 0.45 } else { 0.0 };
    frame_at(tools, config, file, info, at, info.width as usize)
}

/// Grab the frame at `at` seconds, scaled to `width`.
///
/// Scaling is fine here and not on the Crop page: scrubbing a timeline is
/// about recognising *where you are*, so a smaller, faster frame is better
/// than an exact one. The crop overlay needs native pixels; this does not.
pub fn frame_at(
    tools: &ToolPaths,
    config: &Config,
    file: &Path,
    info: &VideoInfo,
    at: f64,
    width: usize,
) -> Result<Image, String> {
    let ffmpeg = tools.ffmpeg_path().ok_or("ffmpeg was not found")?;
    if info.width == 0 || info.height == 0 {
        return Err("the frame size could not be read".into());
    }
    let w = width.max(16);
    let h = ((w as f64 * info.height as f64 / info.width as f64).round() as usize).max(2);
    // Even dimensions: an odd height is illegal for some pixel formats on the
    // way through the scaler.
    let h = h + (h % 2);
    let expected = w * h * 3;

    let args: Vec<String> = vec![
        "-hide_banner".into(),
        "-fflags".into(),
        "+discardcorrupt".into(),
        "-ss".into(),
        format!("{at}"),
        "-i".into(),
        file.display().to_string(),
        "-vframes".into(),
        "1".into(),
        // The matrix must be stated for the same reason as the comparison:
        // analog captures are almost never tagged.
        "-vf".into(),
        format!("scale={w}:{h}:in_color_matrix={}", in_matrix(config)),
        "-pix_fmt".into(),
        "rgb24".into(),
        "-f".into(),
        "rawvideo".into(),
        "-".into(),
    ];

    let (_, data) = run_tool_binary(ffmpeg, &args, DEFAULT_TIMEOUT)
        .map_err(|e| format!("ffmpeg could not be started: {e}"))?;
    if data.len() < expected {
        return Err(format!(
            "ffmpeg returned {} bytes, expected {expected}",
            data.len()
        ));
    }
    Ok(Image {
        width: w,
        height: h,
        rgb: data[..expected].to_vec(),
    })
}

/// Grab one frame and run it through an extra filter chain, for the setting
/// screens that need to show what a change looks like.
///
/// The frame is rendered at the size the finished file will be, so a
/// watermark lands where it will actually land and at the point size it will
/// actually be. A preview drawn at some other size would misplace a corner
/// logo and misjudge text.
///
/// `extra` is inserted after the scale and before the RGB conversion. An
/// empty chain gives the untouched frame, which is what the "before" half of
/// a comparison wants.
pub fn filtered_frame(
    tools: &ToolPaths,
    config: &Config,
    file: &Path,
    at: f64,
    out_w: usize,
    out_h: usize,
    extra: &str,
) -> Result<Image, String> {
    let ffmpeg = tools.ffmpeg_path().ok_or("ffmpeg was not found")?;
    if out_w == 0 || out_h == 0 {
        return Err("the output size is not known".into());
    }
    let expected = out_w * out_h * 3;

    let mut chain = format!(
        "scale={out_w}:{out_h}:in_color_matrix={}",
        in_matrix(config)
    );
    if !extra.is_empty() {
        chain.push(',');
        chain.push_str(extra);
    }

    let args: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-fflags".into(),
        "+discardcorrupt".into(),
        "-ss".into(),
        format!("{at}"),
        "-i".into(),
        file.display().to_string(),
        "-vframes".into(),
        "1".into(),
        "-vf".into(),
        chain,
        "-pix_fmt".into(),
        "rgb24".into(),
        "-f".into(),
        "rawvideo".into(),
        "-".into(),
    ];

    let (_, data) = run_tool_binary(ffmpeg, &args, DEFAULT_TIMEOUT)
        .map_err(|e| format!("ffmpeg could not be started: {e}"))?;
    if data.len() < expected {
        return Err(format!(
            "ffmpeg returned {} bytes, expected {expected}",
            data.len()
        ));
    }
    Ok(Image {
        width: out_w,
        height: out_h,
        rgb: data[..expected].to_vec(),
    })
}

/// The filter that previews a Y/C delay correction.
///
/// **The sign is inverted, and it has to be.** The render shuffles the planes
/// apart and resamples the chroma with VapourSynth's `src_left`, which moves
/// the source window right and so the picture *left*. FFmpeg's `chromashift`
/// moves the chroma *right* for the same positive number. Measured on a frame
/// with a hard chroma edge at x=160: src_left=+8 put it at 152, chromashift=+8
/// put it at 168.
///
/// Using the same sign in both would have made every preview on the Color
/// bleeding page show the exact opposite of what the export produces — which
/// is worse than showing nothing, because it looks authoritative.
pub fn chroma_shift_filter(yc_delay: i32) -> String {
    if yc_delay == 0 {
        return String::new();
    }
    let n = -yc_delay;
    format!("chromashift=cbh={n}:crh={n}")
}


/// Render one frame with the configured watermark burnt in.
///
/// Rendered at the finished file's size, because that is what decides where a
/// corner logo lands and how large the text is. A preview at some other scale
/// would misplace both.
///
/// A logo needs `-filter_complex` and a second input, so this builds the
/// command rather than going through `filtered_frame`.
pub fn watermark_frame(
    tools: &ToolPaths,
    config: &Config,
    file: &Path,
    at: f64,
    out_w: usize,
    out_h: usize,
) -> Result<Image, String> {
    let ffmpeg = tools.ffmpeg_path().ok_or("ffmpeg was not found")?;
    if out_w == 0 || out_h == 0 {
        return Err("the output size is not known".into());
    }
    let expected = out_w * out_h * 3;
    let wm = crate::watermark::watermark_args(config, crate::watermark_font().as_deref());

    let scale = format!(
        "scale={out_w}:{out_h}:in_color_matrix={}",
        in_matrix(config)
    );

    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-fflags".into(),
        "+discardcorrupt".into(),
        "-ss".into(),
        format!("{at}"),
        "-i".into(),
        file.display().to_string(),
    ];
    args.extend(wm.extra_inputs.iter().cloned());

    match &wm.filter {
        // A logo: the overlay graph expects the picture as [0:v] and the logo
        // as [1:v], which is exactly how the inputs are ordered here. The
        // scale has to come first so the logo is sized against the output.
        Some(f) if f.flag == "-filter_complex" => {
            args.push("-filter_complex".into());
            args.push(format!("[0:v]{scale}[base];{}", f.graph.replace("[0:v]", "[base]")));
        }
        Some(f) => {
            args.push("-vf".into());
            args.push(format!("{scale},{}", f.graph));
        }
        None => {
            args.push("-vf".into());
            args.push(scale);
        }
    }

    args.extend([
        "-vframes".into(),
        "1".into(),
        "-pix_fmt".into(),
        "rgb24".into(),
        "-f".into(),
        "rawvideo".into(),
        "-".into(),
    ]);

    let (_, data) = run_tool_binary(ffmpeg, &args, DEFAULT_TIMEOUT)
        .map_err(|e| format!("ffmpeg could not be started: {e}"))?;
    if data.len() < expected {
        return Err(format!(
            "ffmpeg returned {} bytes, expected {expected}",
            data.len()
        ));
    }
    Ok(Image {
        width: out_w,
        height: out_h,
        rgb: data[..expected].to_vec(),
    })
}


/// Render one frame with a single filter stage applied, or with none.
///
/// Used for the before/after pairs on the Settings pages. Both halves run the
/// identical script except for `apply`, so what changes on screen is the
/// filter and nothing else.
///
/// Slower than the ffmpeg-only previews — this is a real VapourSynth render —
/// and considerably slower again for the two stages that need deinterlacing
/// first. The caller shows a skeleton meanwhile and caches the untouched
/// half, which does not change when a strength does.
pub struct StageRequest<'a> {
    pub tools: &'a ToolPaths,
    pub deps: &'a DepsPaths,
    pub config: &'a Config,
    pub file: &'a Path,
    pub info: &'a VideoInfo,
    pub stage: PreviewStage,
    /// Whether the stage is applied. The two halves of a comparison differ
    /// only in this.
    pub apply: bool,
    pub at: f64,
    pub out: (usize, usize),
}

pub fn stage_frame(request: StageRequest<'_>) -> Result<Image, String> {
    let StageRequest {
        tools,
        deps,
        config,
        file,
        info,
        stage,
        apply,
        at,
        out,
    } = request;
    let vspipe = tools
        .vspipe
        .as_ref()
        .ok_or("VapourSynth was not found")?
        .path
        .clone();
    let ffmpeg = tools
        .ffmpeg_path()
        .ok_or("ffmpeg was not found")?
        .to_path_buf();
    let (out_w, out_h) = out;
    if out_w == 0 || out_h == 0 {
        return Err("the output size is not known".into());
    }

    // The script reads its source from the config, so it is set from the file
    // being previewed rather than trusted to already match.
    let mut config = config.clone();
    config.input_path = file.display().to_string();

    let script = generate_stage_preview_vpy(&config, deps, stage, apply);
    let dir = std::env::temp_dir().join("vcg-preview");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let script_path = dir.join(format!(
        "stage-{}-{}-{}.vpy",
        stage.id(),
        if apply { "on" } else { "off" },
        std::process::id()
    ));
    std::fs::write(&script_path, script.as_bytes()).map_err(|e| e.to_string())?;

    let frame = (at * info.fps).round().max(0.0) as u64;
    let expected = out_w * out_h * 3;

    let mut producer = crate::run::command(&vspipe);
    producer.args([
        "--start".to_string(),
        frame.to_string(),
        "--end".to_string(),
        frame.to_string(),
        "-c".to_string(),
        "y4m".to_string(),
        script_path.display().to_string(),
        "-".to_string(),
    ]);
    for key in tools.vspipe_env_removals() {
        producer.env_remove(key);
    }
    for (key, value) in tools.vspipe_env() {
        producer.env(key, value);
    }

    let mut consumer = crate::run::command(&ffmpeg);
    consumer.args([
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-f".to_string(),
        "yuv4mpegpipe".to_string(),
        "-i".to_string(),
        "pipe:0".to_string(),
        "-vf".to_string(),
        format!(
            "scale={out_w}:{out_h}:in_color_matrix={}",
            in_matrix(&config)
        ),
        "-pix_fmt".to_string(),
        "rgb24".to_string(),
        "-f".to_string(),
        "rawvideo".to_string(),
        "-".to_string(),
    ]);

    let (data, stderr) = run_piped(producer, consumer, RENDER_TIMEOUT)
        .map_err(|e| format!("the preview render could not start: {e}"))?;
    let _ = std::fs::remove_file(&script_path);

    if data.len() < expected {
        return Err(format!(
            "the preview produced {} of {expected} bytes. {}",
            data.len(),
            stderr.trim()
        ));
    }
    Ok(Image {
        width: out_w,
        height: out_h,
        rgb: data[..expected].to_vec(),
    })
}

#[cfg(test)]
mod chroma_tests {
    use super::chroma_shift_filter;

    #[test]
    fn no_delay_means_no_filter() {
        assert!(chroma_shift_filter(0).is_empty());
    }

    /// Pinning the inversion, with the measurement that established it in
    /// the doc comment above. If this ever reads the same sign again, the
    /// preview is lying.
    #[test]
    fn the_preview_shift_is_the_opposite_sign_to_the_render() {
        assert_eq!(chroma_shift_filter(4), "chromashift=cbh=-4:crh=-4");
        assert_eq!(chroma_shift_filter(-2), "chromashift=cbh=2:crh=2");
    }
}
