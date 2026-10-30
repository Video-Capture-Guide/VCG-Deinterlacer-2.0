//! The restoration configuration.
//!
//! In the Python app this is `config_data`: a dict that starts with three keys
//! and grows as the user walks the wizard, where every consumer re-specifies
//! its own fallback via `config.get(key, default)`. The same default is spelled
//! out at several call sites, so they can drift. Here each default is declared
//! exactly once, in `Default`, and a missing field is impossible.

use serde::{Deserialize, Serialize};

/// Video standard of the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoFormat {
    Ntsc,
    Pal,
}

impl VideoFormat {
    pub fn is_pal(self) -> bool {
        self == VideoFormat::Pal
    }
}

/// How the source was digitised. Drives crop defaults and chroma handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptureMethod {
    /// Analog capture card (the common case).
    Sd,
    /// DV / miniDV over FireWire — already digital, no overscan to crop.
    Dv,
}

/// Field dominance, or an explicitly progressive source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldOrder {
    /// Top field first — the usual case for analog NTSC/PAL capture.
    Tff,
    /// Bottom field first — DV NTSC.
    Bff,
    /// No deinterlacing; carry the source frame rate through.
    Progressive,
}

/// Color matrix the source pixels are encoded in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorMatrix {
    Bt601,
    Bt709,
}

/// Output container + codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Prores,
    H264,
    Ffv1,
    Huffyuv,
    Utvideo,
    Lagarith,
}

/// Overscan crop preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CropPreset {
    None,
    /// BT.601 active area — 704 wide.
    Bt601,
    /// Aggressive overscan crop — also 704 wide.
    Overscan,
    /// User-specified crop_left/right/top/bottom.
    Manual,
}

/// Broad class of the source file, from `classify_source()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceClass {
    /// Standard-definition analog capture.
    Sd,
    /// AVCHD camcorder footage — already 1920-wide and BT.709.
    Avchd,
    /// HDV tape — likewise HD and BT.709.
    Hdv,
}

/// Where a watermark sits in the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WatermarkPosition {
    BottomRight,
    BottomLeft,
    TopRight,
    TopLeft,
    Center,
}

/// Which kind of watermark to burn in, if any.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "wm_type", rename_all = "lowercase")]
pub enum Watermark {
    None,
    Text {
        text: String,
        fontsize: u32,
    },
    Logo {
        logo_path: String,
        /// Logo width as a fraction of output frame width.
        logo_size: f64,
    },
}

/// Temporal denoise strength.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoiseLevel {
    None,
    Moderate,
    Heavy,
}

/// Halo-removal strength.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DehaloMode {
    None,
    Light,
    Strong,
}

/// Chroma / saturation correction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorCorrection {
    None,
    /// Shift U/V by the measured cast.
    AutoFix,
    /// Shift U/V by the measured cast, then boost saturation.
    AutoFixBoost,
    BoostSat,
    ReduceSat,
}

/// Luma range handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LevelsAdjustment {
    None,
    /// Compress into the legal range.
    Clamp,
    /// Expand to fill the legal range.
    Stretch,
}

/// Which QTGMC to call.
///
/// Two codebases, the same algorithm. `havsfunc.QTGMC` is the one this app
/// has always emitted and the one the bundled dependency payload carries, so
/// it is the default and the only one parity is measured against.
/// `vsdeinterlace.QTempGaussMC` is the JET rewrite, shipped in vs-jetpack:
/// still maintained, but a separate install, two extra VapourSynth plugins,
/// and a different set of knobs — see `vpy.rs` for which of ours survive the
/// crossing and which are left at the rewrite's own defaults.
///
/// No speed claim is made either way. The one measurement taken here — the
/// generated script, 30 frames of 640×480 through each — had the rewrite at
/// 17 fps against havsfunc's 46, which is the opposite of the folklore and
/// far too small a sample to assert anything from. It is offered because it
/// was asked for, not because it is faster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QtgmcImpl {
    /// `havsfunc.QTGMC`, bundled.
    Legacy,
    /// `vsdeinterlace.QTempGaussMC` from vs-jetpack.
    Modern,
}

impl QtgmcImpl {
    /// What the settings page calls it.
    ///
    /// The library's own name, not "Legacy" and "Modern". Those were a
    /// judgement dressed as a label: they tell somebody choosing between the
    /// two that one is out of date, which is not what the measurements say.
    pub fn label(self) -> &'static str {
        match self {
            Self::Legacy => "havsfunc",
            Self::Modern => "vs-jetpack",
        }
    }
}

/// Dither kernel used on the final bit-depth reduction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DitherMethod {
    /// Floyd-Steinberg (fmtc dmode=3).
    ErrorDiffusion,
    /// Ordered Bayer (fmtc dmode=0).
    Ordered,
}

/// What `classify_source()` worked out about the file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SourceClassification {
    pub source_class: SourceClass,
    /// The source pixel format is 10-bit or otherwise non-standard, so it must
    /// be normalized to 8-bit YUV420 before any filtering (VFM requires
    /// standard 8-bit YUV).
    pub needs_pixfmt_conversion: bool,
    /// The pixel format ffprobe reported, for the generated comment.
    pub pix_fmt: String,
    /// A 1440x1080 HD source with non-square pixels, needing a scale to 1920.
    pub par_needed: bool,
}

impl Default for SourceClassification {
    fn default() -> Self {
        Self {
            source_class: SourceClass::Sd,
            needs_pixfmt_conversion: false,
            pix_fmt: String::new(),
            par_needed: false,
        }
    }
}

/// How much the pipeline is allowed to do.
///
/// Deinterlacing is the tool's job; everything else is optional and must not be
/// applied without being asked for. Someone producing a master to grade in
/// Resolve or Topaz needs the structural work done and nothing else touched,
/// while someone restoring a family tape wants the repairs applied for them.
///
/// The mode is not cosmetic: it decides which probes are worth running and
/// which configuration fields may be set at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Deinterlace and correct. No denoising, no sharpening, no grain.
    JustDeinterlace,
    /// Also apply what the analysis recommends.
    EnhanceAutomatically,
    /// Step through every option.
    EnhanceManually,
}

impl Mode {
    pub const ALL: [Mode; 3] = [
        Mode::JustDeinterlace,
        Mode::EnhanceAutomatically,
        Mode::EnhanceManually,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Mode::JustDeinterlace => "Just deinterlace",
            Mode::EnhanceAutomatically => "Deinterlace and enhance automatically",
            Mode::EnhanceManually => "Deinterlace and enhance manually",
        }
    }

    /// The card title.
    pub fn short_title(self) -> &'static str {
        match self {
            Mode::JustDeinterlace => "Just deinterlace",
            Mode::EnhanceAutomatically => "Deinterlace + auto-fix",
            Mode::EnhanceManually => "Full control",
        }
    }

    /// The lead sentence on the mode card.
    pub fn lead(self) -> &'static str {
        match self {
            Mode::JustDeinterlace => {
                "Remove the interlacing lines. Leave everything else exactly as captured."
            }
            Mode::EnhanceAutomatically => {
                "We check the video and fix common tape problems. You approve each fix."
            }
            Mode::EnhanceManually => {
                "Every setting on its own page. For users who know what they want."
            }
        }
    }

    /// What the mode includes. `true` is a tick, `false` a dash.
    pub fn includes(self) -> &'static [(bool, &'static str)] {
        match self {
            Mode::JustDeinterlace => &[
                (true, "Deinterlace (QTGMC)"),
                (true, "Crop black edges"),
                (false, "No color, brightness or noise changes"),
            ],
            Mode::EnhanceAutomatically => &[
                (true, "Everything in Just deinterlace"),
                (
                    true,
                    "Fix brightness, color tint, noise, halos, color bleeding \u{2014} only if found",
                ),
            ],
            Mode::EnhanceManually => &[
                (true, "Everything in auto-fix"),
                (true, "Trim, upscale, audio, watermark, film grain, dithering"),
            ],
        }
    }

    /// The footer line: how many steps, and what the trade is.
    pub fn footer(self) -> &'static str {
        match self {
            Mode::JustDeinterlace => "4 steps \u{00B7} fastest",
            Mode::EnhanceAutomatically => "4 steps \u{00B7} you approve every fix",
            Mode::EnhanceManually => "Up to 15 steps \u{00B7} skip any you don't need",
        }
    }

    /// Whether the noise, halo and color-bleed probes are worth running.
    ///
    /// They are most of the analysis time and nothing acts on them in
    /// `JustDeinterlace`, so that mode skips them entirely.
    pub fn runs_enhancement_probes(self) -> bool {
        self != Mode::JustDeinterlace
    }

    /// Whether an analysis recommendation may set a filter by itself.
    pub fn applies_recommendations(self) -> bool {
        self == Mode::EnhanceAutomatically
    }

    /// Whether the user walks the per-filter pages.
    pub fn shows_filter_pages(self) -> bool {
        self == Mode::EnhanceManually
    }


}

/// The full restoration configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub input_path: String,

    // ── Source characteristics ──────────────────────────────────────────
    pub format: VideoFormat,
    pub capture_method: CaptureMethod,
    pub field_order: FieldOrder,
    pub color_matrix: ColorMatrix,
    pub source: SourceClassification,
    /// Sample aspect ratio as reported by ffprobe, e.g. "32:27". Empty if unknown.
    pub detected_sar: String,

    // ── Deinterlacing ───────────────────────────────────────────────────
    /// Double-rate ("bob") QTGMC: one output frame per field.
    pub double_rate: bool,
    /// Which QTGMC codebase to call. Expert mode only; everything else gets
    /// the bundled one.
    pub qtgmc_impl: QtgmcImpl,
    /// Inverse telecine — restore native film rate from a 3:2 pulldown source.
    pub ivtc_mode: bool,
    /// Set when the user answered film-versus-video by hand.
    ///
    /// Telecine detection is a measurement with a confidence attached, and on
    /// a borderline transfer the person who knows the tape is better informed
    /// than the measurement. So their answer has to outrank it — and has to
    /// keep outranking it, because the analysis is re-applied on every mode
    /// change. Without this flag a manual choice silently reverted to
    /// whatever idet read the moment the user switched modes.
    pub ivtc_user_override: bool,

    // ── Trim ────────────────────────────────────────────────────────────
    /// Inclusive source-frame ranges to keep. Empty means the whole clip.
    /// Applied before any filtering, so cut material is never processed.
    pub trim_ranges: Vec<(i64, i64)>,

    // ── Restoration filters ─────────────────────────────────────────────
    /// Horizontal chroma shift in pixels, to correct Y/C delay.
    pub yc_delay: i32,
    pub noise_level: NoiseLevel,
    pub dehalo_mode: DehaloMode,
    /// Legacy chroma-shift flag, kept for back-compat with old settings files.
    pub chroma_shift: bool,
    pub dropout_removal: bool,
    pub color_correction: ColorCorrection,
    /// Measured U cast on signalstats' 0-255 scale.
    pub u_correction: f64,
    /// Measured V cast on signalstats' 0-255 scale.
    pub v_correction: f64,
    pub levels_adjustment: LevelsAdjustment,
    /// The picture's own black and white points, as measured, on the 8-bit
    /// stored scale.
    ///
    /// Only `LevelsAdjustment::Stretch` reads them: stretching means taking
    /// where this picture actually starts and ends and expanding that to fill
    /// the legal window, which cannot be done without knowing where it starts
    /// and ends. They default to the legal limits, so a stretch with nothing
    /// measured expands 16-235 onto 16-235 and changes nothing — the safe
    /// direction for a setting whose job is to alter brightness.
    pub levels_black: u8,
    pub levels_white: u8,
    /// AddGrain variance. Zero disables the grain overlay.
    pub grain_strength: f64,

    // ── Geometry ────────────────────────────────────────────────────────
    pub crop_preset: Option<CropPreset>,
    pub crop_left: u32,
    pub crop_right: u32,
    pub crop_top: u32,
    /// Everything coming off the bottom: blanking and the head-switching band
    /// together.
    ///
    /// These were two settings with two controls, and that let the bottom be
    /// cropped twice — once by the band toggle and again by the manual value,
    /// with neither total shown. One number, one control.
    pub crop_bottom: u32,
    /// How many of `crop_bottom` are the head-switching band.
    ///
    /// Informational: it is already inside `crop_bottom` and must never be
    /// added to it. It exists so the screen can say *why* the bottom crop is
    /// as large as it is.
    pub head_switching_rows: u32,
    /// Take the same proportion off the sides when rows are removed.
    ///
    /// PAR correction resizes to a fixed target — 640x480 for NTSC — so any
    /// rows taken off the top or bottom get stretched back to full height.
    /// Removing ten rows of a 480-line frame stretches the picture by 2.1%.
    /// With this on, the sides are trimmed in proportion so the kept region
    /// keeps its shape and the resize is uniform.
    ///
    /// The Python has no equivalent and simply stretches, so the parity
    /// harness turns this off; see `crates/vcg-cli`.
    pub keep_aspect: bool,
    /// Correct non-square pixels to square on output.
    pub par_correction: bool,
    pub upscale_enabled: bool,
    /// Target resolution as "WxH". `None` means use the per-format default.
    pub upscale_resolution: Option<String>,

    // ── Output ──────────────────────────────────────────────────────────
    pub output_format: OutputFormat,
    /// Dither on the final 16-bit -> codec-depth reduction.
    pub dither_enabled: bool,
    pub dither_method: DitherMethod,
    /// Copy one channel to both sides, for a source with sound on only one.
    ///
    /// Lossless and reversible in principle, but it does discard the dead
    /// channel, so it stays a decision rather than something done silently.
    pub fix_one_sided_audio: bool,
    /// Which channel carries the sound, when fixing.
    pub audio_source_channel: u8,
    pub watermark: Watermark,

    // ── What this recording is ──────────────────────────────────────────
    //
    // Optional, and written into the file rather than beside it. A restored
    // tape outlives the folder it was restored in: filenames get changed,
    // sidecars get separated, but a tag travels with the file through
    // copying and re-muxing and reads in any player.
    //
    // Three fields because three is what people reliably know — what is on
    // it, roughly when, and roughly where. Anything more and the form starts
    // asking questions nobody can answer about a tape they just found.
    /// What is on this recording, in the owner's words.
    pub meta_description: String,
    /// When it was recorded, as `YYYY-MM-DD` or a prefix of it.
    pub meta_date: String,
    /// Where it was recorded.
    pub meta_location: String,
    /// Watermark alpha, 0.0–1.0.
    pub wm_opacity: f64,
    pub wm_position: WatermarkPosition,
}

impl Default for Config {
    /// Defaults transcribed from the Python `config.get(key, default)` call
    /// sites, so a `Config::default()` reproduces a dict that was never
    /// written to.
    fn default() -> Self {
        Self {
            input_path: String::new(),

            format: VideoFormat::Ntsc,
            capture_method: CaptureMethod::Sd,
            field_order: FieldOrder::Tff,
            color_matrix: ColorMatrix::Bt601,
            source: SourceClassification::default(),
            detected_sar: String::new(),

            double_rate: true,
            qtgmc_impl: QtgmcImpl::Legacy,
            ivtc_mode: false,
            ivtc_user_override: false,

            trim_ranges: Vec::new(),

            yc_delay: 0,
            noise_level: NoiseLevel::None,
            dehalo_mode: DehaloMode::None,
            chroma_shift: false,
            dropout_removal: false,
            color_correction: ColorCorrection::None,
            u_correction: 0.0,
            v_correction: 0.0,
            levels_adjustment: LevelsAdjustment::None,
            levels_black: crate::analysis::histogram::LEGAL_MIN,
            levels_white: crate::analysis::histogram::LEGAL_MAX,
            grain_strength: 0.0,

            // `None` defers to `crop_preset()`, which mirrors the Python
            // default-expression `'none' if capture_method == 'dv' else 'bt601'`.
            crop_preset: None,
            // The wizard seeds these two non-zero; the others start at 0.
            crop_left: 8,
            crop_right: 8,
            crop_top: 0,
            crop_bottom: 0,
            head_switching_rows: 0,
            keep_aspect: true,
            par_correction: true,
            upscale_enabled: false,
            upscale_resolution: None,

            output_format: OutputFormat::Prores,
            dither_enabled: true,
            dither_method: DitherMethod::ErrorDiffusion,
            fix_one_sided_audio: false,
            audio_source_channel: 0,
            watermark: Watermark::None,
            wm_opacity: 0.6,
            wm_position: WatermarkPosition::BottomRight,
            meta_description: String::new(),
            meta_date: String::new(),
            meta_location: String::new(),
        }
    }
}

impl Config {
    /// Effective crop preset. DV sources are already digital with no overscan,
    /// so they default to no crop; everything else to the BT.601 active area.
    pub fn crop_preset(&self) -> CropPreset {
        self.crop_preset.unwrap_or(match self.capture_method {
            CaptureMethod::Dv => CropPreset::None,
            CaptureMethod::Sd => CropPreset::Bt601,
        })
    }

    /// Target upscale resolution as "WxH", defaulted per video format.
    pub fn upscale_resolution(&self) -> &str {
        self.upscale_resolution.as_deref().unwrap_or(match self.format {
            VideoFormat::Ntsc => "960x720",
            VideoFormat::Pal => "1024x768",
        })
    }

    /// True when the source is already HD-class and BT.709.
    ///
    /// HD sources skip all cropping and all PAR correction.
    pub fn is_hd_source(&self) -> bool {
        matches!(
            self.source.source_class,
            SourceClass::Avchd | SourceClass::Hdv
        )
    }

    /// Turn off everything that is not structurally required.
    ///
    /// Called whenever the mode becomes `JustDeinterlace`, so a filter set in
    /// another mode cannot survive the switch. Deinterlacing, crop, PAR
    /// correction, color tagging and dither are deliberately untouched: they
    /// are what makes the output correct, not what makes it prettier.
    pub fn clear_enhancements(&mut self) {
        let d = Config::default();

        // Defect corrections that alter or discard picture data. Levels
        // clamping in particular throws away out-of-range detail a grader
        // might have wanted.
        self.yc_delay = d.yc_delay;
        self.levels_adjustment = d.levels_adjustment;
        self.dropout_removal = d.dropout_removal;

        // Enhancement proper.
        self.noise_level = d.noise_level;
        self.dehalo_mode = d.dehalo_mode;
        self.chroma_shift = d.chroma_shift;
        self.color_correction = d.color_correction;
        self.u_correction = d.u_correction;
        self.v_correction = d.v_correction;
        self.grain_strength = d.grain_strength;
        self.upscale_enabled = d.upscale_enabled;
        self.upscale_resolution = None;

        // Output decoration.
        self.watermark = d.watermark;
    }

    /// True when no optional filter is set.
    ///
    /// Checked by clearing a copy and seeing whether anything moved, so this
    /// cannot drift out of step with `clear_enhancements`.
    pub fn is_structural_only(&self) -> bool {
        let mut cleared = self.clone();
        cleared.clear_enhancements();
        cleared.yc_delay == self.yc_delay
            && cleared.levels_adjustment == self.levels_adjustment
            && cleared.dropout_removal == self.dropout_removal
            && cleared.noise_level == self.noise_level
            && cleared.dehalo_mode == self.dehalo_mode
            && cleared.chroma_shift == self.chroma_shift
            && cleared.color_correction == self.color_correction
            && cleared.u_correction == self.u_correction
            && cleared.v_correction == self.v_correction
            && cleared.grain_strength == self.grain_strength
            && cleared.upscale_enabled == self.upscale_enabled
            && cleared.watermark == self.watermark
    }

    /// Columns to take off the sides so the kept region has the shape the
    /// output claims.
    ///
    /// Mirrors the expression the generated script evaluates at run time; the
    /// script computes it there because the generator does not know the
    /// source size, and keeping one definition here means the number the Crop
    /// page shows is the number the render uses. `vpy_invariants` pins the two
    /// expressions together.
    ///
    /// `target_height` is the PAR-correction target — 480 for NTSC, 576 for
    /// PAL. `reference_width` is the width that reads as 4:3 at full height
    /// for how this source was sampled: 704 for an analog capture card, 720
    /// for DV. See `reference_width` in `vpy` — how the source was sampled
    /// decides what 4:3 means, not where we chose to crop.
    ///
    /// Returns zero when the region is already too narrow, since the only way
    /// to widen it would be to reinstate blanking.
    ///
    /// Use `Config::reference_width()` for the second argument rather than a
    /// literal, so the caller cannot pick the wrong convention.
    pub fn aspect_trim(
        kept_width: u32,
        kept_height: u32,
        target_height: u32,
        reference_width: u32,
    ) -> u32 {
        if kept_height == 0 || target_height == 0 {
            return 0;
        }
        let needed = reference_width as f64 * kept_height as f64 / target_height as f64;
        let excess = kept_width as f64 - needed;
        if excess <= 0.0 {
            return 0;
        }
        ((excess / 4.0).round() * 4.0) as u32
    }

    /// The width that reads as 4:3 at full height for this source.
    pub fn reference_width(&self) -> u32 {
        match self.capture_method {
            CaptureMethod::Dv => 720,
            CaptureMethod::Sd => 704,
        }
    }

    /// Effective crop preset after the HD override.
    ///
    /// An HD source is forced to no crop regardless of what the user picked,
    /// which the .vpy generator does by reassigning `crop_preset` — so every
    /// later decision that reads the preset must see the override too.
    pub fn effective_crop_preset(&self) -> CropPreset {
        if self.is_hd_source() {
            CropPreset::None
        } else {
            self.crop_preset()
        }
    }
}
