//! VapourSynth script generation.
//!
//! Each pipeline stage is its own function taking `&Config` and the script
//! buffer, so a stage can be read and tested on its own.
//!
//! Stage order is load-bearing and must not be rearranged — notably the
//! vertical crop is deferred until after deinterlacing (see `crop`), and the
//! 601->709 matrix conversion must follow the upscale that triggers it.

use std::path::{Path, PathBuf};

use crate::colorspace::needs_601_to_709;
use crate::config::{
    CaptureMethod, ColorCorrection, ColorMatrix, Config, CropPreset, DehaloMode, DitherMethod,
    FieldOrder, LevelsAdjustment, NoiseLevel, OutputFormat, QtgmcImpl, SourceClass, VideoFormat,
};

/// The width that reads as 4:3 at full height, by how the source was sampled.
///
/// Not a constant, and assuming it was broke DV. An analog capture card
/// digitises a 13.5 MHz line into 720 samples of which 704 are active, so 704
/// is 4:3 and the pixel aspect is 10/11 — which is exactly what the BT.601
/// preset crops to and why resizing 704 to a 640-wide target is correct. DV
/// uses the whole 720 as active, giving a pixel aspect of 8/9, which is why
/// DV defaults to no crop and resizes 720 to the same 640.
///
/// Every judgement about whether a cropped region is correctly shaped is made
/// against this.
fn reference_width(config: &Config) -> u32 {
    match config.capture_method {
        CaptureMethod::Dv => 720,
        CaptureMethod::Sd => 704,
    }
}

/// Stamped into the generated script's header.
use crate::{AUTHOR_HANDLE, VERSION_STRING};

/// QTGMC arguments, in emission order.
///
/// Values are kept as pre-rendered text rather than numbers so the emitted
/// script says exactly what is meant — `0.1` must not become `0`, and the
/// preset names carry their own quotes.
///
/// # A least-destructive profile
///
/// QTGMC's only job here is to deinterlace. Everything that is not
/// deinterlacing is either off or turned down as far as it goes, because
/// this app does those jobs separately and better: denoising is the Noise
/// step (BM3D, KNLMeansCL or SMDegrain), run after the deinterlace on
/// progressive frames where it can be judged and switched off.
///
/// What is passed, and why each one:
///
/// * `Preset="Slower"` — the forum-standard quality baseline. The presets
///   above it buy nothing visible.
/// * `TR2=1` — light final temporal smoothing. The default of 2 is mild
///   denoising by another name, which is exactly what this profile is for
///   getting rid of.
/// * `SourceMatch=3` — corrects the output back toward the original fields.
///   The highest-fidelity mode there is.
/// * `Lossless=2` — puts the exact source field lines back into the output.
/// * `Sharpness=0.1` — near zero, against a typical 0.2 to 1.0. Preserves
///   the analog look and leaves no halos for the halo step to undo.
/// * `Sbb=0` — no extra back-blending, so nothing is softened.
/// * `MatchPreset="slow"` — quality of the SourceMatch refinement pass.
/// * `NoiseProcess=0` — no denoising inside QTGMC at all.
///
/// Everything else is left to the Slower preset's own defaults. That is
/// deliberate, and several of the values this app used to pass were worse
/// than the defaults they overrode:
///
/// * The whole noise group (`NoiseProcess=2` with `NoiseRestore=0.4`,
///   `GrainRestore`, `NoisePreset`, `NoiseTR`, `NoiseDeint`,
///   `StabilizeNoise`) was denoising inside the deinterlacer — removing
///   roughly 60% of the noise it identified — and then putting a measured
///   fraction back. That duplicated the Noise step, and it happened whether
///   or not the user had asked for any denoising at all.
/// * `ThSCD1=300` / `ThSCD2=110` were *looser* than the defaults of 180/98,
///   so fewer scene changes were detected and motion vectors were carried
///   across cuts. That risks ghosting at every hard cut.
/// * `Rep0=1` is weaker than the default of 4, letting more temporal
///   artifacts through.
/// * `DCT=5` costs speed in block matching and buys no fidelity.
/// * `TR0`, `TR1`, `Rep1` and `Rep2` were each set to the value the Slower
///   preset already uses, so passing them said nothing.
///
/// The trade: slightly more residual shimmer and aliasing than an aggressive
/// preset would leave, in exchange for staying faithful to the captured
/// fields. That is the right way round for an archival tool, where the
/// source cannot be recaptured and a destroyed detail is gone for good.
const QTGMC_SETTINGS: &[(&str, &str)] = &[
    ("Preset", "\"Slower\""),
    ("TR2", "1"),
    ("SourceMatch", "3"),
    ("Lossless", "2"),
    ("Sharpness", "0.1"),
    ("Sbb", "0"),
    ("MatchPreset", "\"slow\""),
    ("NoiseProcess", "0"),
];

/// Why each setting in `QTGMC_SETTINGS` is what it is, for the About page.
///
/// Paired with the table rather than written beside it on the page, so the
/// two cannot drift: the Python kept its About text as a separate block and
/// it ended up describing settings the app had stopped passing months
/// earlier. `the_profile_and_its_descriptions_line_up` fails the build if a
/// setting is added here or there and not in both.
const QTGMC_WHY: &[(&str, &str)] = &[
    (
        "Preset",
        "Forum-standard quality baseline; higher presets give negligible visible gain",
    ),
    (
        "TR2",
        "Light final temporal smoothing (default 2 acts as mild denoising)",
    ),
    (
        "SourceMatch",
        "Corrects output back toward the original fields — highest fidelity mode",
    ),
    (
        "Lossless",
        "Reinserts the exact source field lines into the output",
    ),
    (
        "Sharpness",
        "Near-zero sharpening; preserves the analog look and avoids halos",
    ),
    ("Sbb", "No extra back-blending (no added softening)"),
    (
        "MatchPreset",
        "Quality setting for the SourceMatch refinement pass",
    ),
    (
        "NoiseProcess",
        "No denoising inside QTGMC — use the Noise step if wanted",
    ),
];

/// The QTGMC profile as `(name, value, why)`, in emission order.
///
/// The About page reads this rather than restating it, so what the screen
/// says and what the script passes are the same list.
pub fn qtgmc_profile() -> Vec<(&'static str, &'static str, &'static str)> {
    QTGMC_SETTINGS
        .iter()
        .map(|(key, value)| {
            let why = QTGMC_WHY
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, w)| *w)
                .unwrap_or("");
            // Quotes are for Python, not for a person reading a panel.
            (*key, value.trim_matches('"'), why)
        })
        .collect()
}

/// Where the portable VapourSynth payload lives.
///
/// Taken as a parameter rather than read from a global so the generated script
/// is reproducible — the Python builds these paths from a module-level
/// `VS_DEPS_DIR`, which makes its output depend on the machine it ran on.
#[derive(Debug, Clone)]
pub struct DepsPaths {
    pub vs_deps_dir: PathBuf,
}

impl DepsPaths {
    pub fn new(vs_deps_dir: impl Into<PathBuf>) -> Self {
        Self {
            vs_deps_dir: vs_deps_dir.into(),
        }
    }

    /// The plugin directory, preferring `plugins64` over `plugins`.
    ///
    /// VapourSynth R73 portable autoloading is unreliable, so the script loads
    /// every DLL explicitly; `None` means there is nothing to load from.
    fn plugin_dir(&self) -> Option<PathBuf> {
        ["plugins64", "plugins"]
            .iter()
            .map(|d| self.vs_deps_dir.join(d))
            .find(|p| p.is_dir())
    }

    fn site_packages(&self) -> String {
        fwd(&self.vs_deps_dir.join("site-packages"))
    }

    /// Note this hardcodes `plugins64`, unlike `plugin_dir()`.
    fn nnedi3_dll(&self) -> String {
        fwd(&self.vs_deps_dir.join("plugins64").join("libnnedi3.dll"))
    }
}

/// Forward slashes — safe on Windows and avoids escaping inside the script.
fn fwd(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Python's `{:g}`: six significant digits with trailing zeros and any
/// trailing point removed.
///
/// Only the fixed-notation range is implemented; grain strength is a small
/// positive slider value, so the exponent forms cannot arise.
fn py_g(v: f64) -> String {
    if v == 0.0 {
        return "0".to_string();
    }
    let digits = v.abs().log10().floor() as i32 + 1;
    let precision = (6 - digits).max(0) as usize;
    let s = format!("{v:.precision$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

/// The script under construction.
struct Script {
    lines: Vec<String>,
}

impl Script {
    fn new() -> Self {
        Self { lines: Vec::new() }
    }

    fn push(&mut self, line: impl Into<String>) {
        self.lines.push(line.into());
    }

    /// A blank separator line.
    fn gap(&mut self) {
        self.lines.push(String::new());
    }

    fn extend<I, S>(&mut self, lines: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.lines.extend(lines.into_iter().map(Into::into));
    }

    fn finish(self) -> String {
        self.lines.join("\n")
    }
}

/// Exactly what one stage contributes to the script, for this config.
///
/// The Details panel on each settings page quotes this rather than describing
/// the filter in prose. Prose drifts: a panel saying "AddGrain at variance
/// 1.0" keeps saying it after the strength changes, and nobody notices for a
/// year. Generated from the same functions the render uses, so the page
/// cannot claim a setting the export does not apply.
///
/// An empty string means the stage emits nothing — which is itself worth
/// showing, because "this option runs no filter at all" is a real answer.
pub fn stage_snippet(config: &Config, stage: PreviewStage) -> String {
    let mut s = Script::new();
    match stage {
        PreviewStage::Levels => levels(&mut s, config),
        PreviewStage::ColorTint => color_correction(&mut s, config),
        PreviewStage::Noise => denoise(&mut s, config),
        PreviewStage::Halos => dehalo(&mut s, config),
        PreviewStage::FilmGrain => grain(&mut s, config),
        PreviewStage::Dither => dither(&mut s, config),
    }
    s.finish().trim_end().to_string()
}

/// Generate the complete VapourSynth script.
pub fn generate_vpy(config: &Config, deps: &DepsPaths) -> String {
    let mut s = Script::new();

    header(&mut s);
    plugin_loading(&mut s, config, deps);
    source_loading(&mut s, config);
    normalize_pixel_format(&mut s, config);
    trim(&mut s, config);
    convert_to_yuv422(&mut s, config);
    lift_to_16bit(&mut s);

    // The crop stage returns the vertical half, which cannot be applied until
    // the clip is progressive.
    let crop = crop(&mut s, config);
    snap_height(&mut s);
    deinterlace(&mut s, config);

    // Deferred vertical crop: the clip is progressive from here on, so any
    // even top/bottom crop is legal for 4:2:0 chroma.
    // Must follow the vertical crop: the compensation reads clip.height to
    // work out the proportion, and that is only correct once the rows are
    // actually gone.
    let overscan_sar = crop.overscan_sar;
    s.extend(crop.deferred_vertical);
    keep_aspect(&mut s, config, overscan_sar);

    hd_par_correction(&mut s, config);
    yc_delay(&mut s, config);
    denoise(&mut s, config);
    dehalo(&mut s, config);
    legacy_chroma_shift(&mut s, config);
    dropout_removal(&mut s, config);
    color_correction(&mut s, config);
    levels(&mut s, config);
    par_and_output_sizing(&mut s, config, overscan_sar);
    upscale(&mut s, config, deps);
    matrix_601_to_709(&mut s, config);
    grain(&mut s, config);
    tag_colorspace(&mut s, config);
    dither(&mut s, config);

    s.push("clip.set_output()");
    s.finish()
}

// ── Header and environment ──────────────────────────────────────────────────

fn header(s: &mut Script) {
    s.push("# -*- coding: utf-8 -*-");
    s.push("# ============================================================");
    s.push("# VapourSynth Restoration Script");
    s.push(format!("# VCG Deinterlacer {VERSION_STRING}"));
    s.push(format!("# {AUTHOR_HANDLE}"));
    s.push("# ============================================================");
    s.gap();
    s.push("import vapoursynth as vs");
    s.push("from vapoursynth import core");
    s.gap();
}

fn plugin_loading(s: &mut Script, config: &Config, deps: &DepsPaths) {
    // VapourSynth R73 portable autoloading is unreliable — the plugins dir is
    // sometimes not found via the portable.vs mechanism. Load every DLL
    // explicitly so we never depend on autoloading.
    //
    // Most load failures really are benign and must stay quiet: a plugin the
    // host autoloaded already, and the support libraries in the same folder
    // that are not VapourSynth plugins at all (libfftw3-3.dll and friends
    // have no entry point).
    //
    // But they were *all* quiet, and that turned the worst failure this
    // payload can have into a mystery. VapourSynth R80 removed API 3, and
    // almost every plugin here is API 3: pointed at an R80 or newer core,
    // twenty-one of the thirty-one are refused, nothing is said, and the run
    // dies much later inside havsfunc with "no attribute or namespace named
    // mv". Reported exactly that way. The failure is not that mvtools is
    // missing — it is on disk — it is that this VapourSynth cannot load it,
    // which is a different problem with a different fix.
    if let Some(dir) = deps.plugin_dir() {
        s.push("# Explicitly load all plugins from portable _deps (bypass autoloading)");
        s.push("import os as _vcg_os");
        s.push(format!("_vcg_p = \"{}\"", fwd(&dir)));
        s.push("_vcg_api3 = []");
        s.push("for _vcg_dll in sorted(_vcg_os.listdir(_vcg_p)):");
        s.push("    if _vcg_dll.lower().endswith(\".dll\"):");
        s.push("        try:");
        s.push("            core.std.LoadPlugin(_vcg_os.path.join(_vcg_p, _vcg_dll))");
        s.push("        except Exception as _vcg_e:");
        s.push("            if \"API 3\" in str(_vcg_e):");
        s.push("                _vcg_api3.append(_vcg_dll)");
        s.gap();
        // Stated once, up front, instead of as a missing namespace later.
        s.push("# A core too new for the bundled plugins is fatal, and is worth");
        s.push("# saying here rather than letting it surface as a missing filter.");
        s.push("if _vcg_api3:");
        s.push("    raise RuntimeError(");
        s.push(
            "        \"This VapourSynth is too new for the bundled plugins: \"",
        );
        s.push(
            "        + str(len(_vcg_api3)) + \" of them use API 3, which VapourSynth \"",
        );
        s.push(
            "        \"R80 removed — including \" + \", \".join(_vcg_api3[:3]) + \". \"",
        );
        s.push(
            "        \"Use VapourSynth R79 or earlier: either point the VCG_VSPIPE \"",
        );
        s.push(
            "        \"environment variable at one, or run \" + chr(39) + \"pip install \"",
        );
        s.push(
            "        \"vapoursynth==79\" + chr(39) + \" for the Python this is using.\"",
        );
        s.push("    )");
        s.gap();
    }

    // Inject the bundled site-packages path so vspipe's embedded Python can
    // find havsfunc.py, vsutil.py etc. even if the _pth file is incomplete.
    s.push("# Ensure bundled site-packages is on sys.path for havsfunc / vsutil");
    s.push("import sys as _vcg_sys");
    s.push(format!("_vcg_sp = \"{}\"", deps.site_packages()));
    s.push("if _vcg_sp not in _vcg_sys.path:");
    s.push("    _vcg_sys.path.insert(0, _vcg_sp)");
    s.gap();
    match config.qtgmc_impl {
        QtgmcImpl::Legacy => s.push("import havsfunc as haf"),
        QtgmcImpl::Modern => {
            // vs-jetpack is not in the bundled payload: it has to be installed
            // into whichever Python runs this script. Caught here rather than
            // left to fail at the call, because a bare ImportError arrives as
            // a traceback several frames deep in vspipe's stderr and says
            // nothing about which setting caused it or how to undo it.
            s.push("# Deinterlacing with vs-jetpack's QTempGaussMC");
            s.push("try:");
            s.push("    from vsdeinterlace import QTempGaussMC as _vcg_qtgmc");
            // vsaa ships in the same vsjetpack distribution, so one install
            // covers both and a separate error message for it would be a
            // message nobody can ever hit. It is imported here rather than
            // beside the call so the whole dependency is checked in one
            // place.
            s.push("    from vsaa import NNEDI3 as _vcg_nnedi3");
            s.push("except ImportError as _vcg_import_error:");
            s.push("    raise RuntimeError(");
            s.push(
                "        \"The vs-jetpack deinterlacer was chosen, but vsdeinterlace \"",
            );
            s.push(
                "        \"is not in this payload. It ships from payload v11 \"",
            );
            s.push(
                "        \"onward, so updating the dependencies on the Welcome page \"",
            );
            s.push(
                "        \"is the fix. Failing that, pip install vsjetpack into the \"",
            );
            s.push(
                "        \"Python that runs this, or choose havsfunc \"",
            );
            s.push("        \"on the Check video page.\"");
            s.push("    ) from _vcg_import_error");
            s.gap();

            // The plugins, checked separately and by name.
            //
            // Importing vs-jetpack proves nothing about these: they are
            // VapourSynth plugins, not Python packages, and they are resolved
            // lazily at the call. Without this the failure is twenty lines of
            // traceback ending in "No attribute with the name resize2
            // exists", several frames inside vskernels, with nothing naming
            // the setting that caused it — and then, because vspipe wrote no
            // frames, ffmpeg's own "Invalid data found when processing input"
            // on top. That is what was reported.
            //
            // Both were found by running the chain, not by reading: the
            // bundled payload carries mvtools, nnedi3, dfttest and akarin
            // already, so these two are the whole gap. `resize2` is wanted by
            // the hardcoded `Catrom().bob()` in the rewrite's prefilter stage
            // and `zsmooth` by its lossless stage.
            s.push("# …and the plugins it calls, which are not Python packages and");
            s.push("# so are not covered by the import above.");
            s.push("_vcg_need = [n for n in (\"resize2\", \"zsmooth\") if not hasattr(core, n)]");
            s.push("if _vcg_need:");
            s.push("    raise RuntimeError(");
            s.push(
                "        \"The vs-jetpack deinterlacer needs VapourSynth plugins that are \"",
            );
            s.push(
                "        \"not in this payload: \" + \", \".join(_vcg_need) + \". They ship \"",
            );
            s.push(
                "        \"from payload v11 onward, so updating the dependencies on the \"",
            );
            s.push(
                "        \"Welcome page is the fix. Failing that, drop their .dll files \"",
            );
            s.push(
                "        \"into the app's _deps/vs/plugins64 folder. Or choose the \"",
            );
            s.push("        \"havsfunc deinterlacer on the Check video page.\"");
            s.push("    )");
        }
    }
    s.gap();
}

// ── Source ──────────────────────────────────────────────────────────────────

/// Frame rate forced at the decoder, as the Python does, for proper sync.
fn fps(format: VideoFormat) -> (u32, u32) {
    match format {
        VideoFormat::Ntsc => (30000, 1001),
        VideoFormat::Pal => (25, 1),
    }
}

fn source_loading(s: &mut Script, config: &Config) {
    let (fpsnum, fpsden) = fps(config.format);

    // Backslashes are doubled before being embedded, even though every use
    // site is a raw string — so the emitted path contains doubled separators.
    // Windows tolerates them, and reproducing it keeps the diff clean.
    let path = config.input_path.replace('\\', "\\\\");

    // AVCHD/HDV: lsmas relies on container metadata for num_frames, and files
    // moved out of their BDMV directory have corrupt duration metadata (e.g.
    // 26 hours), so lsmas thinks there are millions of frames. It processes
    // the real frames fine then fails reading past the end. ffms2 scans the
    // file physically so it always reports the true count.
    let ffms2_first = matches!(
        config.source.source_class,
        SourceClass::Avchd | SourceClass::Hdv
    );

    s.push("# Load source — tries multiple loaders in order, testing frame 0 each time");
    s.push("# to catch lazy failures (e.g. lsmas succeeds on open but fails mid-output).");
    s.push("# AVCHD/HDV files moved out of their BDMV directory have corrupt duration");
    s.push("# metadata; ffms2 scans the file physically and always gets the right frame count.");
    s.push("_vcg_src_errors = []");
    s.push("def _try_src(name, fn):");
    s.push("    try:");
    s.push("        c = fn()");
    s.push("        c.get_frame(0)");
    s.push("        return c");
    s.push("    except Exception as e:");
    s.push("        _vcg_src_errors.append(\"  %s: %s\" % (name, e))");
    s.push("        return None");
    s.gap();

    let lsmas = format!(
        "_try_src(\"lsmas\", lambda: core.lsmas.LWLibavSource(r\"{path}\", stream_index=0, cache=0, fpsnum={fpsnum}, fpsden={fpsden}))"
    );
    let ffms2 = format!(
        "_try_src(\"ffms2\", lambda: core.std.AssumeFPS(core.ffms2.Source(r\"{path}\"), fpsnum={fpsnum}, fpsden={fpsden}))"
    );
    let bs = format!(
        "_try_src(\"bestsource\", lambda: core.std.AssumeFPS(core.bs.VideoSource(r\"{path}\"), fpsnum={fpsnum}, fpsden={fpsden}))"
    );

    // SD puts lsmas first for better performance on normal files.
    let order = if ffms2_first {
        [ffms2, lsmas, bs]
    } else {
        [lsmas, ffms2, bs]
    };

    s.push("clip = (");
    s.push(format!("    {}", order[0]));
    s.push(format!("    or {}", order[1]));
    s.push(format!("    or {}", order[2]));
    s.push(")");
    s.push("if clip is None:");
    // Every reason, not just the fact of failure. A missing plugin reads as
    // an AttributeError naming the namespace, which is the difference between
    // "this payload is broken" and "this file is not readable".
    s.push(format!(
        "    raise RuntimeError(\"Could not open: {path}\\n\" + \"\\n\".join(_vcg_src_errors))"
    ));
    s.gap();
}

fn normalize_pixel_format(s: &mut Script, config: &Config) {
    // classify_source() flagged a pix_fmt VFM cannot accept (yuv420p10le and
    // friends). Convert to 8-bit YUV420 before any other filter so the rest
    // of the pipeline sees a standard clip.
    if !config.source.needs_pixfmt_conversion {
        return;
    }
    let pix_fmt = if config.source.pix_fmt.is_empty() {
        "unknown"
    } else {
        &config.source.pix_fmt
    };
    s.push(format!(
        "# Source pix_fmt \"{pix_fmt}\" is 10-bit / non-standard — normalize to"
    ));
    s.push("# 8-bit YUV420 before any filtering (VFM requires standard 8-bit YUV)");
    s.push("clip = core.resize.Bicubic(clip, format=vs.YUV420P8)");
    s.gap();
}

fn trim(s: &mut Script, config: &Config) {
    // Applied on SOURCE frames, before any processing, so cut material is
    // never deinterlaced or filtered, and audio — trimmed separately with
    // matching timestamps — stays in sync.
    if config.trim_ranges.is_empty() {
        return;
    }
    s.push("# Trim to selected segments (frame-accurate, source frames)");
    s.push("# Frame numbers are clamped to the actual clip length in case the");
    s.push("# container duration over-reported the frame count.");
    s.push("def _vcg_trim(c, a, b):");
    s.push("    b = min(b, c.num_frames - 1)");
    s.push("    a = max(0, min(a, b))");
    s.push("    return core.std.Trim(c, first=a, last=b)");
    s.push("_vcg_src = clip");
    let splice = config
        .trim_ranges
        .iter()
        .map(|(a, b)| format!("_vcg_trim(_vcg_src, {a}, {b})"))
        .collect::<Vec<_>>()
        .join(" + ");
    s.push(format!("clip = {splice}"));
    s.gap();
}

fn convert_to_yuv422(s: &mut Script, config: &Config) {
    // matrix_s is the OUTPUT matrix, required when converting TO YUV from RGB.
    // "170m" = SMPTE 170M / BT.601 for NTSC; "470bg" = BT.470BG for PAL.
    let rgb_matrix = match config.format {
        VideoFormat::Pal => "\"470bg\"",
        VideoFormat::Ntsc => "\"170m\"",
    };

    s.push("# Convert to YUV422 if needed (DV 4:1:1 and RGB not supported by QTGMC directly)");
    s.push("if clip.format.subsampling_w == 2 and clip.format.subsampling_h == 0:");
    s.push("    # YUV411 (DV) detected, convert to YUV422");
    s.push("    clip = core.resize.Spline36(clip, format=vs.YUV422P8)");
    s.push("elif clip.format.color_family == vs.RGB:");
    s.push("    # RGB source (e.g. Lagarith RGB, HuffYUV RGB) — must specify matrix for RGB->YUV");
    s.push(format!(
        "    clip = core.resize.Spline36(clip, format=vs.YUV422P8, matrix_s={rgb_matrix})"
    ));
    s.push("elif clip.format.id not in [vs.YUV420P8, vs.YUV422P8, vs.YUV444P8, vs.YUV420P10, vs.YUV422P10, vs.YUV444P10, vs.GRAY8, vs.GRAY16]:");
    s.push("    # Convert other unsupported formats to YUV422");
    s.push("    clip = core.resize.Spline36(clip, format=vs.YUV422P8)");
    s.gap();
}

fn lift_to_16bit(s: &mut Script) {
    // The whole pipeline (QTGMC, BM3D, color ops, levels) runs at 16-bit;
    // fmtc.bitdepth dithers back to the codec depth at the very end.
    s.push("# Lift to 16-bit so QTGMC/BM3D/colour ops have full precision");
    s.push("clip = core.fmtc.bitdepth(clip, bits=16)");
    s.gap();
}

// ── Crop ────────────────────────────────────────────────────────────────────

/// What the crop stage produced for later stages to use.
struct Crop {
    /// Vertical crop lines to emit after deinterlacing.
    deferred_vertical: Vec<String>,
    /// The overscan preset sets SAR instead of resizing for PAR.
    overscan_sar: bool,
}

fn crop(s: &mut Script, config: &Config) -> Crop {
    // Vertical crop is deferred until after deinterlacing: SeparateFields
    // needs each field to have whole chroma rows, so a 4:2:0 source must keep
    // its luma height mod 4 — the overscan crop (576-6=570, 480-6=474)
    // violates that and crashed on DVD rips. After deinterlacing the clip is
    // progressive, any mod-2 height is legal, and cropping there no longer
    // throws away field lines. Horizontal crop stays here so the whole
    // pipeline works on the narrower frame.
    let mut out = Crop {
        deferred_vertical: Vec::new(),
        overscan_sar: false,
    };

    match config.effective_crop_preset() {
        CropPreset::Bt601 => {
            s.push("# Crop to BT.601 Active Picture area (SD capture — removes blanking edges)");
            s.push("clip = core.std.Crop(clip, left=8, right=8, top=0, bottom=0)");
            s.gap();
        }
        CropPreset::Overscan => {
            // NTSC 704x474 -> SAR 79:88; PAL 704x570 -> SAR 95:88, both 4:3 DAR.
            match config.format {
                VideoFormat::Ntsc => {
                    s.push("# Full Overscan Clean crop, horizontal part (NTSC → 704×474 after");
                    s.push("# the post-deinterlace vertical crop; SAR 79:88 for 4:3 DAR)");
                }
                VideoFormat::Pal => {
                    s.push("# Full Overscan Clean crop, horizontal part (PAL → 704×570 after");
                    s.push("# the post-deinterlace vertical crop; SAR 95:88 for 4:3 DAR)");
                }
            }
            s.push("clip = core.std.Crop(clip, left=8, right=8)");
            s.gap();
            out.deferred_vertical = vec![
                "# Full Overscan Clean crop, vertical part (after deinterlacing so".into(),
                "# 4:2:0 field chroma stays intact — luma height 570/474 is not mod 4)".into(),
                "clip = core.std.Crop(clip, top=2, bottom=4)".into(),
                String::new(),
            ];
            out.overscan_sar = true;
        }
        CropPreset::Manual => {
            let (l, r) = (config.crop_left, config.crop_right);
            let (t, b) = (config.crop_top, config.crop_bottom);
            s.push(format!(
                "# Manual crop, horizontal part (left={l}, right={r}) — top/bottom"
            ));
            s.push("# applied after deinterlacing so 4:2:0 field chroma stays intact");
            s.push(format!("_vcg_l, _vcg_r = {l}, {r}"));
            s.push("if _vcg_l % 2 != 0: _vcg_l += 1  # YUV422: left offset must be even");
            s.push("if (clip.width  - _vcg_l - _vcg_r) % 2 != 0: _vcg_r += 1");
            s.push("clip = core.std.Crop(clip, left=_vcg_l, right=_vcg_r)");
            s.gap();
            out.deferred_vertical = vec![
                format!("# Manual crop, vertical part (top={t}, bottom={b}, deferred from"),
                "# before deinterlacing)".into(),
                format!("_vcg_t, _vcg_b = {t}, {b}"),
                "if _vcg_t % 2 != 0: _vcg_t += 1  # YUV420: top offset must be even".into(),
                "if (clip.height - _vcg_t - _vcg_b) % 2 != 0: _vcg_b += 1".into(),
                "clip = core.std.Crop(clip, top=_vcg_t, bottom=_vcg_b)".into(),
                String::new(),
            ];
        }
        CropPreset::None => {}
    }

    out
}

/// Trim the sides so the kept region still has the shape the output claims.
///
/// PAR correction maps whatever is left onto a fixed target — 640x480 for
/// NTSC — and that only tells the truth for one particular region shape.
/// **The raster decides what 4:3 means, not the crop**: the BT.601 preset
/// leaves 704 columns and the code resizes those to 640, so the pixel aspect
/// is 10/11 and 704 is the width that reads as 4:3 at full height. A region
/// of any other shape comes out distorted.
///
/// So the region must satisfy `width / height == REFERENCE_WIDTH / target
/// height`, and the sides are trimmed until it does.
///
/// Getting this wrong once is instructive. The first attempt assumed the
/// cropped width *was* 4:3 by definition and trimmed in proportion to the rows
/// removed — 16 columns for 10 rows on the measured sample. In fact an
/// over-trimmed side crop already leans the other way: 14 columns a side
/// instead of the nominal 8 makes the picture 1.70% too wide, the 10-row band
/// crop makes it 2.13% too tall, and those nearly cancel at +0.39%. The
/// "compensation" destroyed the cancellation and took it to -1.93%. The
/// correct trim there is under three columns.
///
/// Columns come off in multiples of four so both sides stay even, which the
/// 4:2:2 clip at this point in the pipeline requires.
///
/// Only ever trims. When the region is already too narrow — sides over-cropped
/// with no rows removed — the fix would be to add width back, which would mean
/// reinstating blanking, so the residual error is left alone.
///
/// Skipped where nothing would be distorted: no PAR resize, an HD source, or
/// the overscan preset, which sets SAR for its own exact geometry instead.
fn keep_aspect(s: &mut Script, config: &Config, overscan_sar: bool) {
    if !config.keep_aspect || !config.par_correction || overscan_sar || config.is_hd_source() {
        return;
    }
    let (_, target_h, _) = par_target(config);
    if target_h == 0 {
        return;
    }

    // Only when rows were actually removed. Nothing else may touch the sides:
    // "do not crop anything" has to mean it, and a source whose horizontal
    // crop is already whatever the user asked for is not ours to second-guess.
    let rows = match config.effective_crop_preset() {
        // The overscan preset crops 2 and 4 vertically of its own accord.
        CropPreset::Overscan => 6,
        CropPreset::Manual => config.crop_top + config.crop_bottom,
        CropPreset::Bt601 | CropPreset::None => 0,
    };
    if rows == 0 {
        return;
    }
    let reference = reference_width(config);

    s.push("# Keep the shape: the resize below maps whatever is left onto a fixed");
    s.push(format!(
        "# target, which is only honest when the region is {reference}x{target_h}-shaped."
    ));
    s.push("# Trim the sides until it is. Multiples of four keep both sides even.");
    s.push(format!("_vcg_need = {reference} * clip.height / {target_h}"));
    s.push("_vcg_ax = int(round((clip.width - _vcg_need) / 4.0)) * 4");
    s.push("if _vcg_ax > 0:");
    s.push("    clip = core.std.Crop(clip, left=_vcg_ax // 2, right=_vcg_ax - _vcg_ax // 2)");
    s.gap();
}

fn snap_height(s: &mut Script) {
    // 4:2:0 needs luma height mod 4 (each field must have whole chroma rows);
    // everything else needs mod 2. Guards odd / nonstandard source heights.
    s.push("# Snap height for SeparateFields: 4:2:0 needs mod 4, others mod 2");
    s.push("_vcg_hmod = 4 if clip.format.subsampling_h else 2");
    s.push("if clip.height % _vcg_hmod:");
    s.push("    clip = core.std.Crop(clip, bottom=clip.height % _vcg_hmod)");
    s.gap();
}

// ── Deinterlacing ───────────────────────────────────────────────────────────

fn deinterlace(s: &mut Script, config: &Config) {
    if config.field_order == FieldOrder::Progressive {
        s.push("# Progressive source — deinterlacing skipped");
        s.gap();
        return;
    }

    if config.ivtc_mode {
        // Applies to any capture method: an analog-to-DV converter happily
        // writes 3:2-pulldown film into a DV-codec file, so the codec cannot
        // veto IVTC — only the content analysis and the user's choice decide.
        let order = if config.field_order == FieldOrder::Tff { 1 } else { 0 };
        s.push("# Inverse Telecine (vivtc) — removing 3:2 / 2:2 pulldown");
        s.push("# VFM only accepts 8-bit YUV and the clip was lifted to 16-bit above, so");
        s.push("# field-match on an 8-bit copy and pull the output frames from the 16-bit");
        s.push("# clip via clip2 (documented VFM usage — no precision loss).");
        s.push("_vfm8 = core.fmtc.bitdepth(clip, bits=8)");
        s.push(format!("clip = core.vivtc.VFM(_vfm8, order={order}, clip2=clip)"));
        s.gap();

        // Before decimation, not after: VDecimate drops one frame in five by
        // comparing neighbours, and a combed frame left in the cycle is both
        // a bad comparison and a candidate for being the one kept.
        let tff = if config.field_order == FieldOrder::Tff {
            "True"
        } else {
            "False"
        };
        ivtc_postprocess(s, config, tff);
        s.gap();

        if config.format == VideoFormat::Pal {
            // PAL 2:2 has no duplicate frames to remove — VDecimate (cycle=5)
            // would discard every 5th real frame, producing 20fps judder.
            // Field matching alone restores the 25 progressive frames.
            s.push("# PAL 2:2 — no decimation: there are no duplicate frames to drop");
        } else {
            s.push("clip = core.vivtc.VDecimate(clip)");
        }
        s.gap();
        return;
    }

    // Double-rate "bob" (FPSDivisor=1) produces one frame per field
    // (59.94/50 fps) for the smoothest motion; single-rate (FPSDivisor=2) one
    // frame per interlaced frame (29.97/25 fps) for compatibility and smaller
    // files. Double-rate is the default, matching historical behavior.
    let tff = if config.field_order == FieldOrder::Tff {
        "True"
    } else {
        "False"
    };
    let fps_divisor = if config.double_rate { 1 } else { 2 };
    let mode = if config.double_rate {
        "double-rate (bob)"
    } else {
        "single-rate"
    };

    if config.qtgmc_impl == QtgmcImpl::Modern {
        s.push("# QTGMC Deinterlacing — vs-jetpack QTempGaussMC");
        s.push(format!(
            "# Frame-rate mode: {mode} — fps_divisor={fps_divisor}"
        ));
        modern_qtgmc(s, "clip", "clip", tff, fps_divisor);
        s.gap();
        return;
    }

    s.push("# QTGMC Deinterlacing");
    s.push(format!(
        "# Frame-rate mode: {mode} — FPSDivisor={fps_divisor}"
    ));
    legacy_qtgmc(s, "clip", "clip", tff, fps_divisor);
    s.gap();
}

/// Emit `target = haf.QTGMC(source, ...)` with the standard profile.
///
/// Named rather than hardcoded to `clip` because the inverse-telecine path
/// needs a second, separately named deinterlace to fall back on — see
/// `ivtc_postprocess`.
fn legacy_qtgmc(s: &mut Script, target: &str, source: &str, tff: &str, fps_divisor: u32) {
    s.push(format!("{target} = haf.QTGMC("));
    s.push(format!("    {source},"));
    s.push(format!("    TFF={tff},"));
    s.push(format!("    FPSDivisor={fps_divisor},"));
    // The Python appends a trailing comma to every setting then strips it from
    // the last line; emitting it correctly is equivalent and clearer.
    let last = QTGMC_SETTINGS.len() - 1;
    for (i, (key, value)) in QTGMC_SETTINGS.iter().enumerate() {
        let comma = if i == last { "" } else { "," };
        s.push(format!("    {key}={value}{comma}"));
    }
    s.push(")");
}

/// Repair the frames the field match could not.
///
/// A film transfer is rarely film all the way through. Titles, credits,
/// concert inserts and effects shots are routinely finished at the video rate
/// and cut into the same tape, and a scene change can land mid-cadence. VFM
/// flags every frame it could not assemble from one film frame's two fields,
/// and until this existed those frames went out still combed — which is
/// exactly what a viewer sees as stepping on an otherwise clean film
/// transfer, and it is the documented way to use VFM rather than a
/// refinement. Measured at 15% of frames on a telecined music video, spread
/// evenly over the whole running time; raising VFM's match mode moved it by
/// less than one point, because those fields have no match to find.
///
/// Only the flagged frames are deinterlaced. `FrameEval` asks the deinterlaced
/// clip for a frame only when the matched one is combed, so the cost is in
/// proportion to how much of the file is not film, and a true film transfer
/// pays almost nothing.
///
/// Single-rate (`FPSDivisor=2`) is not a preference here: `FrameEval` needs
/// both clips to have the same length, and the output of this stage is one
/// frame per matched frame either way.
fn ivtc_postprocess(s: &mut Script, config: &Config, tff: &str) {
    s.push("# Frames the field match could not repair — deinterlace just those.");
    s.push("#");
    s.push("# A film transfer is rarely film throughout: titles, credits and");
    s.push("# effects shots are often finished at the video rate, and a scene");
    s.push("# cut can land mid-cadence. VFM flags what it could not match, and");
    s.push("# those frames would otherwise go out still combed.");
    s.push("_ivtc_matched = clip");
    if config.qtgmc_impl == QtgmcImpl::Modern {
        modern_qtgmc(s, "_ivtc_deint", "_ivtc_matched", tff, 2);
    } else {
        legacy_qtgmc(s, "_ivtc_deint", "_ivtc_matched", tff, 2);
    }
    s.push("def _ivtc_pp(n, f):");
    s.push("    return _ivtc_deint if f.props.get(\"_Combed\", 0) else _ivtc_matched");
    s.push("clip = core.std.FrameEval(_ivtc_matched, _ivtc_pp, prop_src=_ivtc_matched)");
}

/// The same least-destructive deinterlace, spelled in the JET rewrite's
/// vocabulary.
///
/// `QTempGaussMC` is a complete rewrite, not a rename: the stages are
/// configured one builder call at a time, and the mapping from
/// `QTGMC_SETTINGS` is by *meaning*, never by number. That distinction is
/// not pedantic — see the two traps below, where the same integer means
/// opposite things in the two libraries.
///
/// Every value here was checked against the installed vsjetpack source
/// rather than against its documentation, because the parameter names and
/// the enum members both move between releases.
///
/// # Where the numbers lie
///
/// * **`Lossless`.** havsfunc documents `1=after final temporal smooth,
///   2=before resharpening`. The rewrite's enum is `PRESHARPEN = 1,
///   POSTSMOOTH = 2`. The two are *swapped*: havsfunc's 2 is the rewrite's
///   `PRESHARPEN`, whose value is 1. Mapping by number puts the source
///   fields back at the wrong stage, which is what this used to do.
/// * **`NoiseProcess`.** havsfunc documents `1=denoise source, 2=identify
///   noise only`. The rewrite's enum is `IDENTIFY = 0, DENOISE = 1`. Also
///   swapped. Moot now that noise processing is off, but it is the same
///   trap and worth leaving written down.
///
/// # What `Preset="Slower"` means here
///
/// The rewrite has no preset concept at all, so the preset is spelled out
/// as the individual values havsfunc's Slower row picks:
///
/// * `NNEDI3(nsize=1, nns=1, qual=1)` for the bobber — havsfunc Slower uses
///   `NNSize=1, NNeurons=1, EdiQual=1`. The rewrite's own defaults are
///   `nns=4` (256 neurons against 32) and `qual=2`, which is eight times the
///   neurons and twice the network evaluations for no gain this profile
///   wants, and the bobber runs several times per frame under source match.
/// * `analyze(refine=0)` — havsfunc's `RefineMotion` defaults to `False` and
///   runs no `Recalculate` pass. The rewrite defaults to `refine=1`, which
///   adds one at blksize 8 / overlap 4 with SATD block matching.
///
/// # Settings with no counterpart
///
/// * `MatchPreset="slow"` has no separate match preset in the rewrite.
///   `source_match_bobber` falls back to the basic bobber, so setting the
///   basic bobber above gives the refinement pass the same quality. That is
///   the closest the rewrite can express.
/// * `Sbb=0` has no `NONE` in `BackBlendMode`. `_apply_back_blend` guards on
///   `if self.backblend_sigma`, so `sigma=0` is the off switch.
/// * Scene-change thresholds and repair strengths are left at the rewrite's
///   defaults, deliberately. The old `ThSCD1=300` / `ThSCD2=110` were looser
///   than any default and risked carrying motion across cuts.
///
/// # The denoise stage does not run
///
/// `NoiseProcess=0` has no enum member — the rewrite always has a denoise
/// mode. The equivalent is structural: `_apply_denoise` opens with
///
/// ```text
/// no_restore = self.basic_noise_restore == self.final_noise_restore == 0
/// if self.denoise_mode == self.NoiseProcessMode.IDENTIFY and no_restore:
///     return
/// ```
///
/// so `IDENTIFY` with both restores at zero returns before building
/// anything. Both restores are left at their default of zero, so the stage
/// is skipped outright rather than merely having its output discarded.
///
/// # One mapping that is not one-to-one
///
/// **`FPSDivisor=2` must not pull in motion blur.** The rewrite decides
/// whether to blur by comparing the two shutter angles, and with the
/// default pair of (180, 180) a divisor of 2 reads as a deliberate request
/// for a 50% flow blur. havsfunc's `FPSDivisor` only ever dropped frames.
/// Saying the source angle is 360 makes `angle_out * divisor == angle_in`,
/// which is the branch that blurs nothing and decimates.
fn modern_qtgmc(s: &mut Script, target: &str, source: &str, tff: &str, fps_divisor: u32) {
    s.push("# Least-destructive profile, the same one the havsfunc path uses:");
    s.push("# deinterlace only, no denoising, near-zero sharpening, and source");
    s.push("# matching with lossless field reinsertion for maximum fidelity.");
    s.push("# Preset=\"Slower\" is spelled out as the bobber and refine values");
    s.push("# it implies; the rewrite has no preset of its own.");
    s.push(format!("{target} = ("));
    s.push("    _vcg_qtgmc()");
    // RefineMotion=False: no Recalculate pass.
    s.push("    .analyze(refine=0)");
    // The Slower preset's bobber. Left to its own defaults this is nns=4,
    // qual=2 — eight times the neurons and twice the evaluations.
    s.push("    .basic(bobber=_vcg_nnedi3(nsize=1, nns=1, qual=1))");
    // NoiseProcess=0. IDENTIFY plus both noise restores at their default of
    // zero makes the denoise stage return before it builds anything.
    s.push("    .denoise(mode=_vcg_qtgmc.NoiseProcessMode.IDENTIFY)");
    // SourceMatch=3.
    s.push("    .source_match(mode=_vcg_qtgmc.SourceMatchMode.TWICE_REFINED)");
    // Lossless=2 — "before resharpening", which is PRESHARPEN, not the
    // identically numbered POSTSMOOTH. See the note above.
    s.push("    .lossless(mode=_vcg_qtgmc.LosslessMode.PRESHARPEN)");
    // Sharpness=0.1, in the rewrite's default UNSHARP_MINMAX mode, which is
    // what havsfunc's Slower row selects with SMode=2.
    s.push("    .sharpen(strength=0.1)");
    // Sbb=0.
    s.push("    .back_blend(sigma=0)");
    // TR2=1, with no noise restored on the way out.
    s.push("    .final(tr=1)");
    if fps_divisor > 1 {
        s.push(format!(
            "    .motion_blur(shutter_angle=(360, 180), fps_divisor={fps_divisor})"
        ));
    } else {
        s.push("    .motion_blur(fps_divisor=1)");
    }
    s.push(format!("    .deinterlace({source}, tff={tff})"));
    s.push(")");
}

fn hd_par_correction(s: &mut Script, config: &Config) {
    // HDV (MPEG-2) and some AVCHD camcorders (Sony, Panasonic) record
    // 1440x1080 with non-square pixels (4:3 PAR) and must be scaled to
    // 1920x1080 for correct 16:9 display. Canon and most AVCHD cameras record
    // 1920x1080 square pixels and do not need this.
    if config.is_hd_source() && config.source.par_needed {
        s.push("# PAR correction: scale 1440x1080 non-square pixels to 1920x1080");
        s.push("clip = core.resize.Spline36(clip, width=1920, height=1080)");
        s.gap();
    }
}

// ── Restoration filters ─────────────────────────────────────────────────────

fn yc_delay(s: &mut Script, config: &Config) {
    // Per-file horizontal chroma shift: split planes, shift U+V, merge back.
    // Luma is untouched.
    let d = config.yc_delay;
    if d == 0 {
        return;
    }
    s.push(format!(
        "# Y/C delay correction — chroma horizontal shift: {d:+} px"
    ));
    s.push("_yp = core.std.ShufflePlanes(clip, planes=0, colorfamily=vs.GRAY)");
    s.push("_up = core.std.ShufflePlanes(clip, planes=1, colorfamily=vs.GRAY)");
    s.push("_vp = core.std.ShufflePlanes(clip, planes=2, colorfamily=vs.GRAY)");
    s.push(format!("_us = core.resize.Point(_up, src_left={d})"));
    s.push(format!("_vs = core.resize.Point(_vp, src_left={d})"));
    s.push("clip = core.std.ShufflePlanes([_yp, _us, _vs], planes=[0, 0, 0], colorfamily=vs.YUV)");
    s.gap();
}

/// The denoise chain, fastest working BM3D first.
///
/// Three things were wrong here and two of them were hiding each other.
///
/// **`core.bm3d.BM3D` does not exist.** The bundled plugin
/// (VapourSynth-BM3D) exposes `Basic`, `Final`, `VBasic`, `VFinal` and
/// `VAggregate` — there is no `BM3D`. Every call raised `AttributeError`,
/// the `except` caught it, and every denoised export this app has ever
/// produced was silently made by KNLMeansCL while the interface, the help
/// text and the log all said BM3D.
///
/// **The sigmas were scaled by 256.** `sigma` is on the 0-255 scale for
/// every BM3D variant whatever the clip's bit depth, so `768` did not mean
/// "3 in 16-bit" — it meant 768. Measured on a real capture, the bundled
/// plugin at `sigma=768` returns a picture with a luma detail of **0.0000**:
/// not denoised, erased. That never reached anybody only because of the
/// first bug.
///
/// **The fast builds were not used.** The bundled `bm3d` is the original
/// implementation. `bm3dcpu` (AVX2) and `bm3dcuda` from the BM3DCUDA project
/// are the same algorithm, measured here at 4.0x and 7.9x its speed on the
/// same clip, and agreeing with it to 52 dB PSNR. With QTGMC in front, the
/// end-to-end export goes 1.49x (AVX2) or 1.58x (CUDA) faster than the
/// KNLMeansCL path that actually ran.
///
/// Order: CUDA, then AVX2, then the bundled original, then KNLMeansCL, then
/// SMDegrain. The first two need 32-bit float, so the clip is converted to
/// the float version of its own format and back — preserving subsampling
/// rather than assuming 4:2:2.
///
/// The CUDA attempt catches `Exception`, not `AttributeError`: the DLL loads
/// on a machine with no NVIDIA driver and then fails when the filter is
/// created, which is not an `AttributeError` and would otherwise take the
/// export down with no fallback.
fn denoise(s: &mut Script, config: &Config) {
    let (sigma, radius, knlm_d, knlm_h, sm_tr, sm_thsad, label) = match config.noise_level {
        NoiseLevel::None => return,
        NoiseLevel::Moderate => (3.0_f64, 1, 1, 1.2_f64, 1, 300, "moderate"),
        NoiseLevel::Heavy => (6.0_f64, 2, 2, 2.0_f64, 2, 400, "heavy"),
    };
    // Chroma takes half again the luma sigma. Tape chroma is soft, which is
    // the reason to filter it harder, not more gently: softness means there is
    // little detail there to lose, while the colour-under subcarrier the noise
    // rides on is as noisy as ever.
    let chroma_sigma = sigma * 1.5;
    let knlm_chroma_h = knlm_h * 1.5;

    s.push(format!(
        "# Temporal denoising ({label}) — BM3D, fastest build available."
    ));
    s.push("# sigma is on the 0-255 scale for every BM3D variant, whatever the");
    s.push("# clip's bit depth.");
    s.push("#");
    s.push("# Luma and chroma are denoised in two separate passes because BM3D");
    s.push("# takes one `radius` for the whole call, and chroma wants radius=0:");
    s.push("# block matching across frames on soft, noisy chroma is unreliable");
    s.push("# and smears colour. Two passes are also *cheaper* than one — +15%");
    s.push("# over luma-only here, against +57% for one call at radius>0.");
    s.push("_vcg_fmt = clip.format");
    s.push("try:");
    s.push(
        "    _vcg_f = core.resize.Point(clip, format=_vcg_fmt.replace(bits_per_sample=32, sample_type=vs.FLOAT).id)",
    );
    s.push("    try:");
    s.push(format!(
        "        _vcg_y = core.bm3dcuda.BM3Dv2(_vcg_f, sigma=[{sigma:.1}, 0, 0], radius={radius})"
    ));
    s.push(format!(
        "        _vcg_c = core.bm3dcuda.BM3Dv2(_vcg_f, sigma=[0, {chroma_sigma:.1}, {chroma_sigma:.1}], radius=0)"
    ));
    // A missing driver surfaces when the filter is created, not as an
    // AttributeError, so this arm has to be broad.
    s.push("    except Exception:");
    s.push(format!(
        "        _vcg_y = core.bm3dcpu.BM3Dv2(_vcg_f, sigma=[{sigma:.1}, 0, 0], radius={radius})"
    ));
    s.push(format!(
        "        _vcg_c = core.bm3dcpu.BM3Dv2(_vcg_f, sigma=[0, {chroma_sigma:.1}, {chroma_sigma:.1}], radius=0)"
    ));
    // Subsampled chroma needs no 4:4:4 detour here: with the default
    // chroma=0 these plugins denoise each plane independently. (chroma=1 is
    // the joint CBM3D mode, and *that* is what would demand YUV444.)
    s.push("    _vcg_dn = core.std.ShufflePlanes([_vcg_y, _vcg_c, _vcg_c], planes=[0, 1, 2], colorfamily=vs.YUV)");
    s.push("    clip = core.resize.Point(_vcg_dn, format=_vcg_fmt.id)");
    s.push("except AttributeError:");
    s.push("    try:");
    // The bundled original refuses outright: "sub-sampled format is not
    // supported when chroma is processed, convert it to YUV444 or RGB first".
    // So this arm stays luma-only and hands chroma to KNLMeansCL below.
    s.push("        # The bundled BM3D cannot process chroma on a subsampled");
    s.push("        # clip at all, so this arm denoises luma and leaves the");
    s.push("        # colour planes to the KNLMeansCL pass that follows.");
    s.push(format!(
        "        _vcg_ref = core.bm3d.Basic(clip, sigma=[{sigma:.1}, 0, 0])"
    ));
    s.push(format!(
        "        clip = core.bm3d.Final(clip, ref=_vcg_ref, sigma=[{sigma:.1}, 0, 0])"
    ));
    s.push("        try:");
    s.push(format!(
        "            clip = core.knlm.KNLMeansCL(clip, d=0, a=2, s=4, h={knlm_chroma_h:.1}, channels=\"UV\")"
    ));
    s.push("        except AttributeError:");
    s.push("            pass  # no KNLMeansCL — luma-only denoise");
    s.push("    except AttributeError:");
    s.push("        try:");
    // channels="auto" is NOT the answer here: on a 4:2:0 clip it resolves to
    // "Y" and never touches chroma, and channels="YUV" is a hard crash on
    // anything but 4:4:4. A second "UV" pass is the only working form.
    s.push(format!(
        "            clip = core.knlm.KNLMeansCL(clip, d={knlm_d}, a=2, s=4, h={knlm_h}, channels=\"Y\")"
    ));
    s.push(format!(
        "            clip = core.knlm.KNLMeansCL(clip, d=0, a=2, s=4, h={knlm_chroma_h:.1}, channels=\"UV\")"
    ));
    s.push("        except AttributeError:");
    s.push(format!(
        "            clip = haf.SMDegrain(clip, tr={sm_tr}, thSAD={sm_thsad}, plane=4)  # last resort; plane=4 (all planes) is the default"
    ));
    s.gap();
}

fn dehalo(s: &mut Script, config: &Config) {
    // The bundled havsfunc removed FineDehalo/DeHalo_alpha (they raise vs.Error
    // pointing at vs-dehalo, which is not bundled), so a faithful
    // self-contained port is emitted into the script instead. It needs only
    // core std/resize ops, rgvs.Repair (RemoveGrainVS.dll — bundled) and the
    // havsfunc mask utilities that do still exist. Luma-only, like the original.
    let params = match config.dehalo_mode {
        DehaloMode::None => return,
        DehaloMode::Light => "rx=2.0, darkstr=0.0, brightstr=0.8",
        DehaloMode::Strong => "rx=2.4, darkstr=0.3, brightstr=1.0",
    };
    let mode = match config.dehalo_mode {
        DehaloMode::Light => "light",
        DehaloMode::Strong => "strong",
        DehaloMode::None => unreachable!(),
    };

    s.push(format!(
        "# Dehalo ({mode}) — FineDehalo port (edge-masked DeHalo_alpha)"
    ));
    s.push("import math as _vcg_math");
    s.push("def _vcg_fine_dehalo(src, rx=2.0, darkstr=0.0, brightstr=1.0):");
    s.push("    _m4 = lambda v: max(16, int(_vcg_math.floor(v / 4 + 0.5)) * 4)");
    s.push("    y = core.std.ShufflePlanes(src, planes=0, colorfamily=vs.GRAY)");
    s.push("    ox, oy = y.width, y.height");
    s.push("    # --- DeHalo_alpha: blur-and-repair, masked to halo-prone zones ---");
    s.push("    halos = core.resize.Bicubic(y, _m4(ox / rx), _m4(oy / rx))");
    s.push("    halos = core.resize.Bicubic(halos, ox, oy, filter_param_a=1, filter_param_b=0)");
    s.push("    are  = core.std.Expr([core.std.Maximum(y), core.std.Minimum(y)], 'x y -')");
    s.push("    ugly = core.std.Expr([core.std.Maximum(halos), core.std.Minimum(halos)], 'x y -')");
    // Expr constants scaled for 16-bit (x256): 255->65535, 50->12800,
    // 256->65536, 512->131072, 80->20480, 48->12288. Ratios are scale-invariant.
    s.push("    so   = core.std.Expr([ugly, are], 'y x - y 0.001 + / 65535 * 12800 - y 65536 + 131072 / 0.5 + *')");
    s.push("    lets = core.std.MaskedMerge(halos, y, so)");
    s.push("    remove = core.rgvs.Repair(y, lets, 1)");
    s.push("    them = core.std.Expr([y, remove], f'x y < x x y - {darkstr} * - x x y - {brightstr} * - ?')");
    s.push("    # --- FineDehalo edge mask: act on strong edges, protect fine detail ---");
    s.push("    edges  = haf.AvsPrewitt(y)");
    s.push("    strong = core.std.Expr(edges, 'x 20480 - 12288 / 65535 *')");
    s.push("    large  = haf.mt_expand_multi(strong, sw=2, sh=2)");
    s.push("    light  = core.std.Expr(edges, 'x 12800 - 12800 / 65535 *')");
    s.push("    shrink = haf.mt_expand_multi(light, mode='ellipse', sw=2, sh=2)");
    s.push("    shrink = core.std.Expr(shrink, 'x 4 *')");
    s.push("    shrink = haf.mt_inpand_multi(shrink, mode='ellipse', sw=2, sh=2)");
    s.push("    shrink = core.std.Convolution(shrink, matrix=[1] * 9)");
    s.push("    shrink = core.std.Convolution(shrink, matrix=[1] * 9)");
    s.push("    outside = core.std.Expr([large, shrink], 'x y max')");
    s.push("    outside = core.std.Expr(core.std.Convolution(outside, matrix=[1] * 9), 'x 2 *')");
    s.push("    y_out = core.std.MaskedMerge(y, them, outside)");
    s.push("    if src.format.color_family == vs.GRAY:");
    s.push("        return y_out");
    s.push("    return core.std.ShufflePlanes([y_out, src, src], planes=[0, 1, 2], colorfamily=vs.YUV)");
    s.gap();
    s.push("try:");
    s.push(format!("    clip = _vcg_fine_dehalo(clip, {params})"));
    s.push("except Exception:");
    s.push("    pass  # dehalo prerequisites unavailable — skipping");
    s.gap();
}

fn legacy_chroma_shift(s: &mut Script, config: &Config) {
    if config.chroma_shift {
        s.push("# Chroma shift correction (legacy color-bleeding fix)");
        s.push("clip = core.resize.Point(clip, src_left=2, src_top=2)");
        s.gap();
    }
}

fn dropout_removal(s: &mut Script, config: &Config) {
    if config.dropout_removal {
        s.push("# Dropout removal");
        s.push("clip = core.rgvs.Clense(clip)");
        s.gap();
    }
}

fn color_correction(s: &mut Script, config: &Config) {
    // Chroma neutral in 16-bit YUV is 32768 (vs 128 in 8-bit). The u/v
    // corrections arrive from signalstats on a 0-255 scale, so scale x256.
    let (u, v) = (config.u_correction, config.v_correction);
    let (u16, v16) = (u * 256.0, v * 256.0);

    /// `x N +` or `x N -` depending on the sign, as the Python builds it.
    fn shift(v16: f64) -> String {
        if v16 >= 0.0 {
            format!("x {v16:.1} +")
        } else {
            format!("x {:.1} -", v16.abs())
        }
    }

    /// The same shift, followed by a 1.2x saturation boost around neutral.
    fn shift_boost(v16: f64) -> String {
        format!("{} 32768 - 1.2 * 32768 +", shift(v16))
    }

    match config.color_correction {
        ColorCorrection::None => {}
        ColorCorrection::AutoFix => {
            s.push("# Auto color cast correction (16-bit: chroma neutral = 32768)");
            s.push(format!(
                "# Shifting U by {u:.1} → {u16:.0} and V by {v:.1} → {v16:.0} (×256 for 16-bit)"
            ));
            s.push(format!(
                "clip = core.std.Expr([clip], [\"\", \"{}\", \"{}\"])",
                shift(u16),
                shift(v16)
            ));
            s.gap();
        }
        ColorCorrection::AutoFixBoost => {
            s.push("# Auto color cast correction + saturation boost (16-bit)");
            s.push(format!(
                "# Shifting U by {u16:.0} and V by {v16:.0}, then boosting saturation"
            ));
            s.push(format!(
                "clip = core.std.Expr([clip], [\"\", \"{}\", \"{}\"])",
                shift_boost(u16),
                shift_boost(v16)
            ));
            s.gap();
        }
        ColorCorrection::BoostSat => {
            s.push("# Boost saturation (16-bit chroma neutral = 32768)");
            s.push("clip = core.std.Expr([clip], [\"\", \"x 32768 - 1.2 * 32768 +\", \"x 32768 - 1.2 * 32768 +\"])");
            s.gap();
        }
        ColorCorrection::ReduceSat => {
            s.push("# Reduce saturation (16-bit chroma neutral = 32768)");
            s.push("clip = core.std.Expr([clip], [\"\", \"x 32768 - 0.8 * 32768 +\", \"x 32768 - 0.8 * 32768 +\"])");
            s.gap();
        }
    }
}

/// Clamp or stretch the luma, as the two options actually mean.
///
/// Both of these diverge from the Python deliberately, because the Python
/// does neither of the things it offers. It emits the same call for both —
/// `gamma=1.0` is `std.Levels`' default, so the two differ by a comment — and
/// that call is `min_in=0, max_in=65535, min_out=4096, max_out=60160`, which
/// maps the *whole* code range onto the legal window. Measured against
/// VapourSynth, a picture already inside the range comes out with its black
/// at 29.7 and its white at 216: it darkens the blacks and dulls the whites
/// of footage that needed nothing done to it, under a label saying "clamp".
///
/// 16-bit limited luma: 16x256=4096 (black), 235x256=60160 (white).
fn levels(s: &mut Script, config: &Config) {
    match config.levels_adjustment {
        LevelsAdjustment::None => {}
        LevelsAdjustment::Clamp => {
            // Identity inside the window, cut outside it. `std.Levels` clamps
            // anything beyond [min_in, max_in] to the output limits, so
            // mapping the legal window onto itself is exactly a clamp:
            // measured, 16 stays 16 and 235 stays 235, while 0 and 255 are
            // pulled to the limits.
            s.push("# Clamp luma to the legal range: 16-235 passes through untouched,");
            s.push("# anything outside is cut to the limit (4096-60160 in 16-bit)");
            s.push("clip = core.std.Levels(clip, min_in=4096, max_in=60160, min_out=4096, max_out=60160, planes=[0])");
            s.gap();
        }
        LevelsAdjustment::Stretch => {
            // Where this picture actually starts and ends, expanded to fill
            // the window. Stretching against a fixed 0-255 input, as the
            // Python does, is not a stretch at all — the input range has to
            // be the measured one or there is nothing being stretched.
            let black = config.levels_black as u32 * 256;
            let white = config.levels_white as u32 * 256;
            if white <= black {
                // Nothing measured, or measured nonsense. Leave the picture
                // alone rather than invent a transform for it.
                s.push("# Stretch luma: no usable black and white point measured, so");
                s.push("# the picture is left as it is");
                s.gap();
                return;
            }
            s.push(format!(
                "# Stretch luma: this picture's own {}-{} expanded to fill the legal",
                config.levels_black, config.levels_white
            ));
            s.push("# range 16-235 (4096-60160 in 16-bit). Anything beyond is clipped.");
            s.push(format!(
                "clip = core.std.Levels(clip, min_in={black}, max_in={white}, min_out=4096, max_out=60160, planes=[0])"
            ));
            s.gap();
        }
    }
}

// ── Sizing ──────────────────────────────────────────────────────────────────

/// Square-pixel target size and whether the source is anamorphic widescreen.
fn par_target(config: &Config) -> (u32, u32, bool) {
    match config.format {
        VideoFormat::Ntsc if config.detected_sar == "32:27" => (854, 480, true),
        VideoFormat::Ntsc => (640, 480, false),
        VideoFormat::Pal if config.detected_sar == "64:45" => (1024, 576, true),
        VideoFormat::Pal => (768, 576, false),
    }
}

fn par_and_output_sizing(s: &mut Script, config: &Config, overscan_sar: bool) {
    // Overscan preset: preserve the exact cropped dimensions and set SAR so
    // the player renders at the correct 4:3 display aspect ratio.
    if overscan_sar {
        match config.format {
            VideoFormat::Ntsc => {
                // 704x474, SAR 79:88 -> display width 704x79/88 ~= 632; 632/474 ~= 4:3
                s.push("# Set pixel aspect ratio for 4:3 DAR (NTSC overscan clean 704×474)");
                s.push("clip = core.std.SetFrameProps(clip, _SARNum=79, _SARDen=88)");
            }
            VideoFormat::Pal => {
                // 704x570, SAR 95:88 -> display width 704x95/88 ~= 760; 760/570 ~= 4:3
                s.push("# Set pixel aspect ratio for 4:3 DAR (PAL overscan clean 704×570)");
                s.push("clip = core.std.SetFrameProps(clip, _SARNum=95, _SARDen=88)");
            }
        }
        s.gap();
        return;
    }

    if !config.par_correction {
        return;
    }

    let preset = config.effective_crop_preset();
    let (w, h, wide) = par_target(config);

    if preset != CropPreset::None {
        let note = if wide {
            " (16:9 widescreen)"
        } else {
            ""
        };
        s.push(format!(
            "# PAR correction → {w}×{h} square pixels{note}"
        ));
        s.push(format!(
            "clip = core.resize.Spline36(clip, width={w}, height={h})"
        ));
        s.gap();
    } else if !config.is_hd_source() {
        // No crop (DV or explicit none): still correct PAR.
        let note = if wide {
            " (16:9 widescreen, no crop path)"
        } else {
            " (no crop path)"
        };
        s.push(format!(
            "# PAR correction → {w}×{h} square pixels{note}"
        ));
        s.push(format!(
            "clip = core.resize.Spline36(clip, width={w}, height={h})"
        ));
        s.gap();
    }
}

fn upscale(s: &mut Script, config: &Config, deps: &DepsPaths) {
    if !config.upscale_enabled {
        return;
    }
    // A malformed resolution degrades to the per-format default.
    let fallback = match config.format {
        VideoFormat::Ntsc => (960, 720),
        VideoFormat::Pal => (1024, 768),
    };
    let res = config.upscale_resolution();
    let (w, h) = parse_wxh(res).unwrap_or(fallback);

    s.push(format!("# nnedi3 upscale → {w}×{h} (nsize=0, nns=3, rfactor=2)"));
    s.push("if not hasattr(core, \"nnedi3\"):");
    s.push(format!(
        "    core.std.LoadPlugin(r\"{}\")",
        deps.nnedi3_dll()
    ));
    s.push("# First pass: double height");
    s.push("clip = core.nnedi3.nnedi3(clip, field=1, dh=True, nsize=0, nns=3)");
    s.push("# Rotate 90°, double height again, rotate back");
    s.push("clip = core.std.Transpose(clip)");
    s.push("clip = core.nnedi3.nnedi3(clip, field=1, dh=True, nsize=0, nns=3)");
    s.push("clip = core.std.Transpose(clip)");
    s.push(format!("# Resize to target {w}×{h}"));
    s.push(format!(
        "clip = core.resize.Spline36(clip, width={w}, height={h})"
    ));
    s.gap();
}

/// Parse a "WxH" resolution. Both halves must be present and numeric, matching
/// the Python's `(int(x) for x in res_str.split('x'))` under a ValueError guard.
fn parse_wxh(res: &str) -> Option<(u32, u32)> {
    let (w, h) = res.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

fn matrix_601_to_709(s: &mut Script, config: &Config) {
    // When the upscale makes the frame HD-sized (height >= 720) the file
    // becomes HD-class and platforms like YouTube read it as BT.709, so the
    // pixels — not just the tag — must be converted or an upscaled SD capture
    // displays dark and oversaturated. Done after the upscale, at 16-bit.
    // fmtconv's matrix needs 4:4:4, so chroma is upsampled around the matrix
    // op and restored afterwards; limited (TV) range is kept on both sides so
    // the conversion never flips to full range. Falls back to zimg, which does
    // the matrix and chroma resampling in one call ("170m"/"470bg" are the
    // same BT.601 coefficients for NTSC SMPTE170M and PAL BT.470BG).
    if !needs_601_to_709(config) {
        return;
    }
    s.push("# Convert BT.601 → BT.709 (HD upscale) — real pixel conversion at 16-bit");
    s.push("_vcg_src_fmt = clip.format");
    s.push("try:");
    s.push("    _vcg_c = core.fmtc.resample(clip, css=\"444\")");
    s.push("    _vcg_c = core.fmtc.matrix(_vcg_c, mats=\"601\", matd=\"709\", fulls=False, fulld=False)");
    s.push("    _vcg_ss = (_vcg_src_fmt.subsampling_w, _vcg_src_fmt.subsampling_h)");
    s.push("    _vcg_css = {(0, 0): \"444\", (1, 0): \"422\", (1, 1): \"420\", (2, 0): \"411\"}.get(_vcg_ss, \"444\")");
    s.push("    clip = core.fmtc.resample(_vcg_c, css=_vcg_css)");
    s.push("except Exception:");
    s.push("    clip = core.resize.Spline36(clip, format=_vcg_src_fmt.id,");
    s.push("                                matrix_in_s=\"170m\", matrix_s=\"709\",");
    s.push("                                range_in_s=\"limited\", range_s=\"limited\")");
    s.gap();
}

fn grain(s: &mut Script, config: &Config) {
    // Added at final resolution (after any upscale) so the grain stays
    // pixel-fine, and before dithering while the pipeline is still 16-bit.
    // AddGrain normalizes `var` to the clip's bit depth internally, so the
    // slider value is passed through unscaled. Skipped silently if the
    // AddGrain plugin is absent from this _deps set.
    let g = config.grain_strength;
    if g <= 0.0 {
        return;
    }
    s.push(format!(
        "# Film grain overlay (AddGrain, strength/var {})",
        py_g(g)
    ));
    s.push("try:");
    s.push(format!("    clip = core.grain.Add(clip, var={g:.1}, uvar=0)"));
    s.push("except AttributeError:");
    s.push("    pass  # grain plugin not available — skipping");
    s.gap();
}

fn tag_colorspace(s: &mut Script, config: &Config) {
    // VS _Matrix: 1=BT.709, 5=BT.470BG (PAL BT.601), 6=SMPTE170M (NTSC BT.601).
    // _Range: 0=full, 1=limited. Spelled _Range and not _ColorRange: the
    // old name still works but VapourSynth warns on every script that uses
    // it, and a deprecation warning in the export log looks like a fault
    // in the render to anyone reading it.
    // Uses the EFFECTIVE output matrix: after an SD->HD upscale the pixels were
    // converted 601->709 above, so the tags must read 709 to match them.
    let matrix = crate::colorspace::effective_output_matrix(config);
    let (n, label) = match (matrix, config.format) {
        (ColorMatrix::Bt709, _) => (1, "BT709"),
        (ColorMatrix::Bt601, VideoFormat::Pal) => (5, "BT601"),
        (ColorMatrix::Bt601, VideoFormat::Ntsc) => (6, "BT601"),
    };
    s.push(format!(
        "# Tag colorspace — {label} (_Matrix={n}, limited range)"
    ));
    s.push(format!(
        "clip = core.std.SetFrameProps(clip, _Matrix={n}, _Primaries={n}, _Transfer={n}, _Range=1)"
    ));
    s.gap();
}

fn dither(s: &mut Script, config: &Config) {
    // Dither from 16-bit down to the codec's native depth: ProRes takes 10-bit
    // (fully populating the container, no free LSBs), everything else 8-bit.
    // fmtc dmode=3 is error diffusion (Floyd-Steinberg), dmode=0 ordered Bayer.
    let out_bits = if config.output_format == OutputFormat::Prores {
        10
    } else {
        8
    };
    let vs_fmt = if out_bits == 10 {
        "vs.YUV422P10"
    } else {
        "vs.YUV422P8"
    };
    let (dmode, dmode_label) = match config.dither_method {
        DitherMethod::Ordered => (0, "ordered Bayer"),
        DitherMethod::ErrorDiffusion => (3, "error diffusion"),
    };

    if config.dither_enabled {
        s.push(format!(
            "# Dither 16-bit → {out_bits}-bit ({dmode_label}, fmtc dmode={dmode})"
        ));
        s.push("try:");
        s.push(format!(
            "    clip = core.fmtc.bitdepth(clip, bits={out_bits}, dmode={dmode})"
        ));
        s.push("except Exception:");
        s.push(format!(
            "    clip = core.resize.Spline36(clip, format={vs_fmt})  # fallback: no dither"
        ));
    } else {
        s.push(format!(
            "# Output at {out_bits}-bit (dithering disabled by user)"
        ));
        s.push(format!(
            "clip = core.resize.Spline36(clip, format={vs_fmt})"
        ));
    }
    s.gap();
}


/// One filter stage, for the before/after previews on the Settings pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewStage {
    Levels,
    ColorTint,
    Noise,
    Halos,
    FilmGrain,
    /// The bit-depth reduction, where banding comes from.
    Dither,
}

impl PreviewStage {

    pub fn id(self) -> &'static str {
        match self {
            PreviewStage::Levels => "levels",
            PreviewStage::ColorTint => "tint",
            PreviewStage::Noise => "noise",
            PreviewStage::Halos => "halos",
            PreviewStage::FilmGrain => "grain",
            PreviewStage::Dither => "dither",
        }
    }
}

/// The dither stage, as the preview needs it.
///
/// The real stage targets 10 bits for ProRes and 8 for everything else. A
/// preview at 10 bits would show nothing: the banding this setting exists to
/// prevent only appears at 8, and the pipe carries 8-bit frames anyway. So
/// the preview always reduces to 8 — which is what the comparison is of, and
/// what the untouched half gets from the plain conversion below.
fn preview_dither(s: &mut Script, config: &Config) {
    if !config.dither_enabled {
        return;
    }
    let (dmode, label) = match config.dither_method {
        DitherMethod::Ordered => (0, "ordered Bayer"),
        DitherMethod::ErrorDiffusion => (3, "error diffusion"),
    };
    s.push(format!("# Preview: dither 16-bit to 8-bit ({label}, fmtc dmode={dmode})"));
    s.push("try:");
    s.push(format!(
        "    clip = core.fmtc.bitdepth(clip, bits=8, dmode={dmode})"
    ));
    s.push("except Exception:");
    s.push("    pass");
    s.gap();
}

/// A script that renders one frame with exactly one filter stage applied, or
/// with none.
///
/// Both halves of a before/after go through the identical pipeline and differ
/// only in whether `apply` is set, so the difference on screen is the filter
/// and nothing else. That matters more than matching the full export exactly:
/// the question the page asks is "what does this setting do", and every other
/// stage in the chain would only add noise to the answer.
pub fn generate_stage_preview_vpy(
    config: &Config,
    deps: &DepsPaths,
    stage: PreviewStage,
    apply: bool,
) -> String {
    let mut s = Script::new();

    header(&mut s);
    plugin_loading(&mut s, config, deps);
    source_loading(&mut s, config);
    normalize_pixel_format(&mut s, config);
    convert_to_yuv422(&mut s, config);
    lift_to_16bit(&mut s);

    // Always deinterlaced, whatever the stage.
    //
    // Denoising and dehaloing need it to be judged at all — on a woven frame
    // they spend their effort on the comb lines. The per-pixel stages do not
    // need it and it costs a couple of seconds a frame, which is why only
    // those two paid for it at first. But a preview whose whole job is to
    // show what the finished video will look like cannot show a combed frame
    // and be doing that job, and nobody reading the Levels page wants to be
    // told the comb is not really there.
    //
    // It also pays for itself: with no stage applied the script no longer
    // depends on which stage was asked for, so the untouched half is one
    // image shared by every page instead of one per page.
    snap_height(&mut s);
    deinterlace(&mut s, config);

    if apply {
        match stage {
            PreviewStage::Levels => levels(&mut s, config),
            PreviewStage::ColorTint => color_correction(&mut s, config),
            PreviewStage::Noise => denoise(&mut s, config),
            PreviewStage::Halos => dehalo(&mut s, config),
            PreviewStage::FilmGrain => grain(&mut s, config),
            PreviewStage::Dither => preview_dither(&mut s, config),
        }
    }

    // y4m carries only YUV or Gray, so the RGB conversion happens downstream
    // in ffmpeg with the matrix stated explicitly.
    //
    // For the dither preview the depth reduction has already happened above,
    // and this conversion only changes the layout.
    s.push("# 8-bit YUV 4:4:4 for the y4m pipe");
    s.push("clip = core.resize.Spline36(clip, format=vs.YUV444P8)");
    s.gap();
    s.push("clip.set_output()");
    s.finish()
}

/// Generate a minimal script that deinterlaces and outputs RGB.
///
/// Deliberately not the full pipeline: a before/after preview has to show the
/// effect of *deinterlacing alone*, so crop, PAR correction, denoising and
/// dither are all left out. Including them would make the right-hand image
/// differ from the left for reasons that have nothing to do with combing.
///
/// Single-rate (`FPSDivisor=2`) so output frame N corresponds to source frame
/// N and the two images are the same moment.
pub fn generate_preview_vpy(config: &Config, deps: &DepsPaths) -> String {
    let mut s = Script::new();

    header(&mut s);
    plugin_loading(&mut s, config, deps);
    source_loading(&mut s, config);
    normalize_pixel_format(&mut s, config);

    // QTGMC needs YUV, and the same 16-bit lift the real pipeline uses so the
    // result matches what the export will produce.
    //
    // The *same* conversion the export runs, not a reduced copy of it. This
    // handled RGB alone, which left DV untouched: a MiniDV capture is 4:1:1,
    // `normalize_pixel_format` lets it through because VFM accepts 411, and
    // `mv.Super` then refuses the clip outright —
    //
    //     Super: input clip must be GRAY, 420, 422, 440, or 444, up to
    //     16 bits, with constant dimensions
    //
    // — so every DV source showed that traceback where the before/after
    // comparison should have been, while the export of the same file worked.
    // The settings preview beside this one already shared the function;
    // only this path had its own copy, and only this path was wrong.
    s.push("# Preview: deinterlace only, so the comparison isolates combing");
    convert_to_yuv422(&mut s, config);
    s.push("clip = core.fmtc.bitdepth(clip, bits=16)");
    s.gap();

    s.push("# Snap height for SeparateFields: 4:2:0 needs mod 4, others mod 2");
    s.push("_vcg_hmod = 4 if clip.format.subsampling_h else 2");
    s.push("if clip.height % _vcg_hmod:");
    s.push("    clip = core.std.Crop(clip, bottom=clip.height % _vcg_hmod)");
    s.gap();

    if config.field_order == FieldOrder::Progressive {
        s.push("# Progressive source — nothing to deinterlace");
        s.gap();
    } else {
        let tff = if config.field_order == FieldOrder::Tff {
            "True"
        } else {
            "False"
        };
        // Single-rate keeps output frame numbers aligned with the source.
        if config.qtgmc_impl == QtgmcImpl::Modern {
            modern_qtgmc(&mut s, "clip", "clip", tff, 2);
        } else {
            legacy_qtgmc(&mut s, "clip", "clip", tff, 2);
        }
        s.gap();
    }

    // 8-bit YUV 4:4:4, because the frame leaves through y4m and y4m carries
    // only YUV or Gray — an RGB clip makes vspipe refuse outright with "can
    // only apply y4m headers to YUV and Gray format clips". The conversion to
    // RGB happens downstream in ffmpeg, which is told the matrix explicitly.
    s.push("# 8-bit YUV 4:4:4 for the y4m pipe; RGB conversion happens downstream");
    s.push("clip = core.resize.Spline36(clip, format=vs.YUV444P8)");
    s.gap();
    s.push("clip.set_output()");
    s.finish()
}
