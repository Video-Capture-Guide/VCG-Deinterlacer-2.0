//! Parsing FFmpeg's `signalstats` + `metadata=mode=print` output.
//!
//! Four of the analysis probes share this one ffmpeg invocation and differ only
//! in which metadata keys they read and how they aggregate them, so the parse
//! is factored out here and the thresholds live in `super::classify`.
//!
//! The Python parses these lines inline inside each probe with
//! `float(line.split('=')[-1].strip())` wrapped in a bare `except: pass`, which
//! means a malformed value is silently dropped. That behavior is preserved —
//! it is load-bearing, because ffmpeg emits plenty of other `key=value` log
//! lines that must not be mistaken for measurements.

/// One `ffmpeg … signalstats` run's worth of measurements.
///
/// Each field collects every value seen in output order, across all frames the
/// run decoded — matching the Python, which appends to flat lists rather than
/// grouping per frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SignalStatsRun {
    /// Luma minimum, read as an integer (a fractional value is dropped).
    pub ymin: Vec<i64>,
    /// Luma maximum, read as an integer.
    pub ymax: Vec<i64>,
    pub uavg: Vec<f64>,
    pub vavg: Vec<f64>,
    pub satavg: Vec<f64>,
    /// Inter-frame luma difference — reflects both motion and noise.
    pub ydif: Vec<f64>,
    /// Temporal outliers — the noise-specific indicator.
    pub tout: Vec<f64>,
    pub uhigh: Vec<f64>,
    pub ulow: Vec<f64>,
    pub vhigh: Vec<f64>,
    pub vlow: Vec<f64>,
}

/// Extract the value after the final `=` on a line, as the Python does.
fn value_of(line: &str) -> Option<&str> {
    line.rsplit_once('=').map(|(_, v)| v.trim())
}

impl SignalStatsRun {
    /// Parse one run's stderr.
    pub fn parse(stderr: &str) -> Self {
        let mut r = Self::default();
        for line in stderr.split('\n') {
            // Every key is matched as a substring of the line, because ffmpeg
            // can glue a progress line (`frame= 88 …`) onto the front of a
            // filter line with no intervening newline.
            if !line.contains('=') {
                continue;
            }

            // Order matters only in that each line carries one key; the
            // Python's if/elif chain is reproduced as a single match.
            if line.contains("YMIN") {
                push_int(&mut r.ymin, line);
            } else if line.contains("YMAX") {
                push_int(&mut r.ymax, line);
            } else if line.contains("SATAVG") {
                // Checked before UAVG/VAVG for clarity; the names do not
                // actually overlap as substrings.
                push_float(&mut r.satavg, line);
            } else if line.contains("UAVG") {
                push_float(&mut r.uavg, line);
            } else if line.contains("VAVG") {
                push_float(&mut r.vavg, line);
            } else if line.contains("YDIF") {
                push_float(&mut r.ydif, line);
            } else if line.contains("TOUT") {
                push_float(&mut r.tout, line);
            } else if line.contains("UHIGH") {
                push_float(&mut r.uhigh, line);
            } else if line.contains("VHIGH") {
                push_float(&mut r.vhigh, line);
            } else if line.contains("ULOW") {
                push_float(&mut r.ulow, line);
            } else if line.contains("VLOW") {
                push_float(&mut r.vlow, line);
            }
        }
        r
    }

    /// The last value seen for a channel, or `default` if the run measured none.
    ///
    /// `analyze_color_bleeding` keeps scalars that the last matching line wins,
    /// seeded at 128 (chroma neutral), rather than averaging them.
    pub fn last_or(values: &[f64], default: f64) -> f64 {
        values.last().copied().unwrap_or(default)
    }
}

fn push_int(out: &mut Vec<i64>, line: &str) {
    if let Some(Ok(n)) = value_of(line).map(str::parse::<i64>) {
        out.push(n);
    }
}

fn push_float(out: &mut Vec<f64>, line: &str) {
    if let Some(Ok(n)) = value_of(line).map(str::parse::<f64>) {
        out.push(n);
    }
}

/// Arithmetic mean, or 0.0 for an empty slice — the Python's `sum/len` guarded
/// by an `if values:` check.
pub fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

/// Population standard deviation (divides by n, not n-1), matching the Python.
pub fn std_dev(values: &[f64]) -> f64 {
    if values.len() <= 1 {
        return 0.0;
    }
    let m = mean(values);
    let variance =
        values.iter().map(|x| (x - m).powi(2)).sum::<f64>() / values.len() as f64;
    variance.sqrt()
}

/// Sample timestamps at even intervals across the whole duration.
///
/// `analyze_video_levels` and `analyze_color_data` use
/// `duration * i / (n + 1)` for i in 1..=n, so the samples sit strictly inside
/// the file and never at either end.
pub fn interior_sample_times(duration: f64, samples: u32) -> Vec<f64> {
    (1..=samples)
        .map(|i| duration * i as f64 / (samples + 1) as f64)
        .collect()
}

/// Sample offsets spanning 20%..80% of the file, as fractions.
///
/// `analyze_noise_level`, `analyze_halo_level` and `analyze_color_bleeding`
/// avoid intros and outros this way. With `samples == 1` the Python divides by
/// zero and raises; that is treated as a single 20% sample here instead.
pub fn mid_sample_offsets(samples: u32) -> Vec<f64> {
    if samples <= 1 {
        return vec![0.2];
    }
    (0..samples)
        .map(|i| 0.2 + (0.6 * i as f64 / (samples - 1) as f64))
        .collect()
}
