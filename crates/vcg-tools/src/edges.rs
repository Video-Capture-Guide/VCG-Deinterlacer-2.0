//! Sampling frames for the auto-crop detector.
//!
//! The measurement itself is in `vcg_core::analysis::edges`; this is only the
//! part that has to spawn ffmpeg. Frames are decoded at **native resolution**:
//! a scaled frame would blur the blanking boundary into the picture, and
//! locating that boundary to the pixel is the entire job.
//!
//! They are decoded as **4:4:4**, not grayscale, because the strongest signal
//! for a head-switching band is how far its chroma departs from the line
//! above — and subsampled chroma would merge the two lines being compared.
//!
//! Sampling is by seek rather than by decoding through the file, so the cost is
//! flat whether the capture is two minutes or two hours. The spread avoids the
//! first and last tenth, where leaders, blank tape and rewind noise live and
//! where the edges are not representative of the recording.

use std::path::Path;
use std::time::Duration;

use vcg_core::analysis::edges::{detect, Detection, EdgeTally, FramePlanes};
use vcg_core::analysis::histogram::LumaHistogram;
use vcg_core::analysis::probe::VideoInfo;

use crate::paths::ToolPaths;
use crate::run::run_tool_binary;

/// How many frames to fold into the measurement.
///
/// Each is one seek-and-decode, so this trades time for stability. Fourteen
/// settles the percentiles well past the point where the recommendation moves.
pub const FRAMES: usize = 14;

/// A single decode gets less than the default: on a damaged capture a seek can
/// stall, and one unusable sample should not hold up the other thirteen.
const FRAME_TIMEOUT: Duration = Duration::from_secs(20);

/// Decode one frame at `at` seconds as planar 4:4:4, at native size.
fn grab(
    tools: &ToolPaths,
    file: &Path,
    info: &VideoInfo,
    at: f64,
) -> Option<Vec<u8>> {
    let ffmpeg = tools.ffmpeg_path()?;
    // Three full-size planes, Y then U then V.
    let expected = info.width as usize * info.height as usize * 3;

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
        // No scale filter: see the module note.
        "-vf".into(),
        "format=yuv444p".into(),
        "-pix_fmt".into(),
        "yuv444p".into(),
        "-f".into(),
        "rawvideo".into(),
        "-".into(),
    ];

    let (_, mut data) = run_tool_binary(ffmpeg, &args, FRAME_TIMEOUT).ok()?;
    if data.len() < expected {
        return None;
    }
    data.truncate(expected);
    Some(data)
}

/// Measure the frame edges of `file` and recommend a crop.
///
/// Returns `Inconclusive` rather than an error for anything that goes wrong:
/// a failed probe is a reason to leave the user's preset alone, not a reason to
/// stop the wizard.
/// Measure the frame edges, and the luma histogram from the same frames.
///
/// Two measurements from one decode pass, for the same reason levels and
/// color already share their samples: fourteen native-resolution frames is
/// the expensive part, and the histogram wants exactly the planes the edge
/// scan has already paid for.
pub fn detect_crop_and_levels(
    tools: &ToolPaths,
    file: &Path,
    info: &VideoInfo,
) -> (Detection, LumaHistogram) {
    let mut histogram = LumaHistogram::new();
    let detection = detect_crop_inner(tools, file, info, Some(&mut histogram));
    (detection, histogram)
}

pub fn detect_crop(tools: &ToolPaths, file: &Path, info: &VideoInfo) -> Detection {
    detect_crop_inner(tools, file, info, None)
}

fn detect_crop_inner(
    tools: &ToolPaths,
    file: &Path,
    info: &VideoInfo,
    mut histogram: Option<&mut LumaHistogram>,
) -> Detection {
    if info.width == 0 || info.height == 0 {
        return Detection::Inconclusive("the frame size could not be read");
    }
    if info.duration <= 0.0 {
        return Detection::Inconclusive("the running time could not be read");
    }

    let mut tally = EdgeTally::new(info.width as usize, info.height as usize);
    for i in 0..FRAMES {
        let fraction = 0.1 + 0.8 * (i as f64 / (FRAMES - 1) as f64);
        let Some(data) = grab(tools, file, info, info.duration * fraction) else {
            continue;
        };
        let n = info.width as usize * info.height as usize;
        if let Some(h) = histogram.as_deref_mut() {
            // Inset, so capture-card blanking does not decide the black
            // point. It is no signal rather than dark picture, it sits at or
            // under the legal floor on every frame, and measured over the
            // whole raster it reports a tape as crushed when the picture
            // inside the frame is nowhere near black.
            h.add_plane_inset(&data[..n], info.width as usize, info.height as usize);
        }
        tally.add_frame(FramePlanes {
            y: &data[..n],
            u: &data[n..2 * n],
            v: &data[2 * n..3 * n],
            width: info.width as usize,
            height: info.height as usize,
        });
    }
    detect(&tally)
}
