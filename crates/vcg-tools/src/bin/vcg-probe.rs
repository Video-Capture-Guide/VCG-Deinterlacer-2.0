//! Analyze a video file and print what the ported probes recommend.
//!
//! The first thing in this port that can be run against real footage. It
//! exercises the whole analysis layer — the same code the wizard will call —
//! so its output can be compared against what the Python app reports for the
//! same file.

use std::path::PathBuf;

use vcg_core::analysis::classify::{BleedLevel, ColorCast, NoiseVerdict, SaturationIssue};
use vcg_core::analysis::halo::HaloLevel;
use vcg_core::analysis::idet::{Confidence, Pattern};
use vcg_core::analysis::probe::{DetectedCaptureMethod, DetectedFieldOrder};
use vcg_core::analysis::Recommendation;
use vcg_core::config::{SourceClass, VideoFormat};
use vcg_tools::paths::ToolPaths;
use vcg_tools::probe::{probe_file, Options, Report};

const USAGE: &str = "\
vcg-probe — analyze a video file and report what the restoration probes find

USAGE:
    vcg-probe [OPTIONS] <FILE>

OPTIONS:
    --quick            Skip the slow probes (telecine and halos)
    --deps <DIR>       Use this _deps directory instead of searching for one
    --no-telecine      Skip telecine detection (the slowest probe)
    --no-halo          Skip halo detection
    --quiet            Only print the report, no progress
    -h, --help         Print this help
";

struct Args {
    file: PathBuf,
    deps: Option<PathBuf>,
    options: Options,
    quiet: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut file = None;
    let mut deps = None;
    let mut options = Options::default();
    let mut quiet = false;

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "--quick" => options = Options::quick(),
            "--no-telecine" => options.telecine = false,
            "--no-halo" => options.halo = false,
            "--quiet" => quiet = true,
            "--deps" => {
                deps = Some(PathBuf::from(
                    it.next().ok_or("--deps needs a directory")?,
                ))
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown option {other:?}"))
            }
            other => {
                if file.is_some() {
                    return Err("only one file at a time".to_string());
                }
                file = Some(PathBuf::from(other));
            }
        }
    }

    Ok(Args {
        file: file.ok_or("no file given")?,
        deps,
        options,
        quiet,
    })
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("vcg-probe: {e}\n");
            eprint!("{USAGE}");
            std::process::exit(2);
        }
    };

    if !args.file.is_file() {
        eprintln!("vcg-probe: no such file: {}", args.file.display());
        std::process::exit(1);
    }

    let tools = ToolPaths::discover(args.deps.as_deref());
    print_tools(&tools);

    if !tools.can_probe() {
        eprintln!(
            "\nffmpeg and ffprobe are both required. Point --deps at a folder \
             containing ffmpeg\\ffmpeg.exe, or put them on PATH."
        );
        std::process::exit(1);
    }

    println!("\nFile  {}", args.file.display());

    let started = std::time::Instant::now();
    let report = if args.quiet {
        probe_file(&tools, &args.file, args.options, None)
    } else {
        // Progress goes to stderr, so redirecting stdout to a file captures a
        // clean report while the terminal still shows activity.
        let mut last = String::new();
        let mut progress = |name: &str, done: u32, total: u32| {
            use std::io::Write;
            let mut err = std::io::stderr();
            if name != last {
                if !last.is_empty() {
                    let _ = writeln!(err);
                }
                last = name.to_string();
            }
            let _ = write!(err, "\r  {name:<18} {done}/{total}   ");
            let _ = err.flush();
        };
        let r = probe_file(&tools, &args.file, args.options, Some(&mut progress));
        eprint!("\r{:<44}\r", "");
        r
    };

    print_report(&report, started.elapsed());
}

fn print_tools(tools: &ToolPaths) {
    println!("Tools");
    match &tools.deps_dir {
        Some(d) => println!("  deps dir   {}", d.display()),
        None => println!("  deps dir   (not found — falling back to PATH)"),
    }
    for (label, tool) in [
        ("ffmpeg", &tools.ffmpeg),
        ("ffprobe", &tools.ffprobe),
        ("vspipe", &tools.vspipe),
    ] {
        match tool {
            Some(t) => println!("  {label:<10} {} [{}]", t.path.display(), t.origin),
            None => println!("  {label:<10} not found"),
        }
    }
}

/// `Recommendation` rendered as the wizard would word it.
fn rec(r: Recommendation) -> &'static str {
    match r {
        Recommendation::None => "none",
        Recommendation::Light => "light",
        Recommendation::Moderate => "moderate",
        Recommendation::Strong => "strong",
        Recommendation::Heavy => "heavy",
    }
}

fn print_report(r: &Report, elapsed: std::time::Duration) {
    let row = |label: &str, value: String| println!("  {label:<12} {value}");

    println!("Source");
    if let (Some(c), Some(i)) = (&r.classification, &r.info) {
        let class = match c.source_class {
            SourceClass::Sd => "SD",
            SourceClass::Avchd => "AVCHD",
            SourceClass::Hdv => "HDV",
        };
        row(
            "stream",
            format!(
                "{} / {}, {}x{}, {:.2} fps, {:.1}s",
                class, c.codec, i.width, i.height, i.fps, i.duration
            ),
        );
        row("display", c.display_name.clone());
        row(
            "field order",
            match c.field_order {
                DetectedFieldOrder::Tff => "top field first",
                DetectedFieldOrder::Bff => "bottom field first",
                DetectedFieldOrder::Progressive => "progressive (per container)",
                DetectedFieldOrder::Unknown => "not declared",
            }
            .to_string(),
        );
        // What the pipeline will actually deinterlace with, which on an
        // undeclared file is not the same question. Getting this wrong makes
        // the output look worse than the source, and until it was printed
        // there was no way to see it short of reading the generated script.
        {
            let mut decided = vcg_core::config::Config::default();
            vcg_tools::apply::apply_report(
                &mut decided,
                r,
                vcg_core::config::Mode::EnhanceAutomatically,
            );
            let how = if c.field_order != DetectedFieldOrder::Unknown {
                "declared by the container"
            } else if r
                .telecine
                .as_ref()
                .and_then(|t| t.measured_field_order())
                .is_some()
            {
                "measured from the picture"
            } else {
                "from the capture format"
            };
            row(
                "  will use",
                format!("{:?} — {how}", decided.field_order).to_lowercase(),
            );
        }
        row("pixel fmt", {
            let mut s = c.pix_fmt.clone();
            if c.needs_pixfmt_conversion {
                s.push_str("  -> needs 8-bit normalisation before IVTC");
            }
            s
        });
        if c.par_needed {
            row("PAR", "anamorphic HD — needs scaling to 1920x1080".to_string());
        }
    }

    if let Some(p) = &r.par {
        row(
            "standard",
            format!(
                "{}{}{}",
                match p.format {
                    Some(VideoFormat::Ntsc) => "NTSC",
                    Some(VideoFormat::Pal) => "PAL",
                    None => "unknown",
                },
                match p.capture_method {
                    Some(DetectedCaptureMethod::Sd) => ", analog capture",
                    Some(DetectedCaptureMethod::Dv) => ", DV",
                    Some(DetectedCaptureMethod::Dvd) => ", DVD / MPEG-2",
                    None => "",
                },
                if p.sar.is_empty() {
                    String::new()
                } else {
                    format!(", SAR {}", p.sar)
                }
            ),
        );
    }

    println!("\nFindings");

    match &r.telecine {
        None => row("telecine", "not checked".to_string()),
        Some(t) if t.progressive_source => row(
            "telecine",
            format!(
                "already progressive throughout ({:.0}% progressive frames) \
                 — skip deinterlacing",
                t.prog_ratio * 100.0
            ),
        ),
        Some(t) => match t.pattern {
            Some(p) => row(
                "telecine",
                format!(
                    "{} pulldown detected in {} of {} window{} ({:.0}% progressive, \
                     {:.0}% repeated fields, {} confidence) -> IVTC to {:.3} fps",
                    match p {
                        Pattern::ThreeTwo => "3:2",
                        Pattern::TwoTwo => "2:2",
                    },
                    t.film_windows,
                    t.windows,
                    if t.windows == 1 { "" } else { "s" },
                    t.prog_ratio * 100.0,
                    t.rep_ratio * 100.0,
                    match t.confidence {
                        Some(Confidence::High) => "high",
                        Some(Confidence::Low) => "low",
                        None => "no",
                    },
                    p.target_fps(),
                ),
            ),
            None => row(
                "telecine",
                format!(
                    "none — interlaced video ({} window{}, {:.0}% progressive, \
                     {:.0}% repeated fields)",
                    t.windows,
                    if t.windows == 1 { "" } else { "s" },
                    t.prog_ratio * 100.0,
                    t.rep_ratio * 100.0
                ),
            ),
        },
    }

    match &r.levels {
        None => row("levels", "not checked".to_string()),
        Some(l) => {
            let range = match (l.min_y, l.max_y) {
                (Some(a), Some(b)) => format!("Y {a}-{b}"),
                _ => "not measured".to_string(),
            };
            row(
                "levels",
                if l.needs_adjustment {
                    format!("{range} — outside legal 16-235, correction suggested")
                } else {
                    format!("{range} — within legal range")
                },
            );
        }
    }

    match &r.color {
        None => row("color", "not measured".to_string()),
        Some(c) => {
            let cast = match c.color_cast {
                None => "no consistent cast".to_string(),
                Some(k) => format!(
                    "{} cast",
                    match k {
                        ColorCast::Blue => "blue",
                        ColorCast::Yellow => "yellow",
                        ColorCast::RedMagenta => "red/magenta",
                        ColorCast::GreenCyan => "green/cyan",
                    }
                ),
            };
            row(
                "color",
                format!(
                    "{cast} (U {:+.1}, V {:+.1}); saturation {:.1}{}",
                    c.u_offset,
                    c.v_offset,
                    c.sat_avg,
                    match c.sat_issue {
                        Some(SaturationIssue::VeryLow) => " (very low)",
                        Some(SaturationIssue::Low) => " (low)",
                        Some(SaturationIssue::High) => " (high)",
                        None => "",
                    }
                ),
            );
            if c.color_cast.is_some() {
                row(
                    "  -> correct",
                    format!("U {:+.1}, V {:+.1}", c.u_correction, c.v_correction),
                );
            }
        }
    }

    match &r.noise {
        None => row("noise", "not checked".to_string()),
        Some(n) => row(
            "noise",
            format!(
                "{} (TOUT {:.4}, YDIF {:.1}, {} samples){} -> denoise {}",
                match n.noise_level {
                    NoiseVerdict::Clean => "clean",
                    NoiseVerdict::Light => "light",
                    NoiseVerdict::Moderate => "moderate",
                    NoiseVerdict::Heavy => "heavy",
                },
                n.avg_variance,
                n.avg_diff,
                n.samples_analyzed,
                if n.analyzed { "" } else { " [too few samples to trust]" },
                rec(n.recommendation),
            ),
        ),
    }

    match &r.halo {
        None => row("halos", "not checked".to_string()),
        Some(h) if !h.analyzed => row("halos", "no strong edges to judge".to_string()),
        Some(h) => row(
            "halos",
            format!(
                "{} ({:.1}% of {} edges) -> dehalo {}",
                match h.halo_level {
                    HaloLevel::Clean => "clean",
                    HaloLevel::Light => "light",
                    HaloLevel::Moderate => "moderate",
                    HaloLevel::Strong => "strong",
                    HaloLevel::Unknown => "unknown",
                },
                h.halo_ratio * 100.0,
                h.edges_analyzed,
                rec(h.recommendation),
            ),
        ),
    }

    match &r.bleeding {
        None => row("bleeding", "not checked".to_string()),
        Some(b) => row(
            "bleeding",
            format!(
                "{} (spread U {:.1} / V {:.1}, score {:.1}){}",
                match b.bleed_level {
                    BleedLevel::None => "none",
                    BleedLevel::Light => "light",
                    BleedLevel::Moderate => "moderate",
                    BleedLevel::Significant => "significant",
                },
                b.avg_h_spread,
                b.avg_v_spread,
                b.bleeding_score,
                if b.recommendation {
                    " -> correction suggested"
                } else {
                    ""
                }
            ),
        ),
    }

    if !r.warnings.is_empty() {
        println!("\nWarnings");
        for w in &r.warnings {
            println!("  - {w}");
        }
    }

    println!(
        "\n{} ffmpeg/ffprobe calls in {:.1}s",
        r.spawns,
        elapsed.as_secs_f64()
    );
}
