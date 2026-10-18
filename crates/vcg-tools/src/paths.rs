//! Finding ffmpeg, ffprobe and vspipe.
//!
//! Ported from the path-resolution block around `load_tool_paths`, with one
//! deliberate addition: every resolved tool records *where it came from*.
//!
//! That is not cosmetic. This machine turned out to have three separate
//! VapourSynth installations — the bundled `_deps` copy, a pip install, and an
//! installer build registered in `HKCU` — and the Python app silently picks
//! among them with no way to tell afterwards which one ran. A support report
//! of "it crashed" or "the colors are wrong" is unanswerable without knowing
//! which engine produced it, so the origin travels with the path.

use std::fmt;
use std::path::{Path, PathBuf};

/// Where a tool was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// The portable payload beside the executable — the intended case.
    Bundled,
    /// Found on `PATH`.
    SystemPath,
    /// Given explicitly on the command line.
    Explicit,
    /// A pip-installed VapourSynth in a Python `site-packages`.
    PipInstall,
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Origin::Bundled => "bundled",
            Origin::SystemPath => "system PATH",
            Origin::Explicit => "explicit",
            Origin::PipInstall => "pip install",
        };
        f.write_str(s)
    }
}

/// A located tool.
#[derive(Debug, Clone)]
pub struct Tool {
    pub path: PathBuf,
    pub origin: Origin,
}

/// Everything the pipeline needs to run.
#[derive(Debug, Clone)]
pub struct ToolPaths {
    pub ffmpeg: Option<Tool>,
    pub ffprobe: Option<Tool>,
    pub vspipe: Option<Tool>,
    /// The portable VapourSynth root, if present. The `.vpy` embeds absolute
    /// paths derived from it.
    pub vs_deps_dir: Option<PathBuf>,
    /// The `_deps` root that was searched.
    pub deps_dir: Option<PathBuf>,
}

/// Environment variable naming a vspipe to use instead of the bundled one.
pub const ENV_VSPIPE: &str = "VCG_VSPIPE";

/// The executable suffix, so the search works when cross-checked on a
/// non-Windows host.
const EXE: &str = if cfg!(windows) { ".exe" } else { "" };

fn exe_name(stem: &str) -> String {
    format!("{stem}{EXE}")
}

/// Look for `name` in each directory on `PATH`.
fn find_on_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// Candidate `_deps` locations, nearest first.
///
/// A shipped build finds it beside the executable. During development the
/// binary lives in `target/debug`, so the search also walks up from the
/// working directory — which is how it finds the real payload sitting next to
/// the Python app.
fn deps_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();

    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent() {
            out.push(dir.join("_deps"));
        }

    if let Ok(cwd) = std::env::current_dir() {
        // Up to five levels covers target/debug inside a workspace and the
        // project sitting one folder below the payload.
        let mut here = cwd.as_path();
        for _ in 0..6 {
            out.push(here.join("_deps"));
            match here.parent() {
                Some(p) => here = p,
                None => break,
            }
        }
    }

    out
}

impl ToolPaths {
    /// Resolve every tool.
    ///
    /// `explicit_deps` overrides the search entirely, which is what the parity
    /// and probe tooling uses so results do not depend on the machine.
    pub fn discover(explicit_deps: Option<&Path>) -> Self {
        let (deps_dir, explicit) = match explicit_deps {
            Some(d) => (Some(d.to_path_buf()), true),
            None => (deps_candidates().into_iter().find(|d| d.is_dir()), false),
        };

        let bundled_origin = if explicit { Origin::Explicit } else { Origin::Bundled };

        // ── ffmpeg / ffprobe: _deps\ffmpeg\ first, then PATH ───────────────
        let ffmpeg_dir = deps_dir.as_ref().map(|d| d.join("ffmpeg"));
        let locate = |stem: &str| -> Option<Tool> {
            let name = exe_name(stem);
            if let Some(dir) = &ffmpeg_dir {
                let candidate = dir.join(&name);
                if candidate.is_file() {
                    return Some(Tool { path: candidate, origin: bundled_origin.clone() });
                }
            }
            find_on_path(&name).map(|path| Tool { path, origin: Origin::SystemPath })
        };
        let ffmpeg = locate("ffmpeg");
        let ffprobe = locate("ffprobe");

        // ── VapourSynth: _deps\vs\ first ───────────────────────────────────
        let vs_deps_dir = deps_dir
            .as_ref()
            .map(|d| d.join("vs"))
            .filter(|d| d.is_dir());

        // An explicit override comes first. The bundled copy can fail to
        // initialise VSScript on a machine that also has registry-installed or
        // pip-installed VapourSynth, and when it does there is otherwise no
        // way for the user to point at one that works.
        let vspipe = std::env::var_os(ENV_VSPIPE)
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .map(|path| Tool { path, origin: Origin::Explicit })
            .or_else(|| {
                vs_deps_dir
                    .as_ref()
                    .map(|d| d.join(exe_name("vspipe")))
                    .filter(|p| p.is_file())
                    .map(|path| Tool { path, origin: bundled_origin })
            })
            .or_else(|| find_pip_vspipe().map(|path| Tool { path, origin: Origin::PipInstall }))
            .or_else(|| {
                find_on_path(&exe_name("vspipe"))
                    .map(|path| Tool { path, origin: Origin::SystemPath })
            });

        Self { ffmpeg, ffprobe, vspipe, vs_deps_dir, deps_dir }
    }

    /// Whether the probes can run at all.
    pub fn can_probe(&self) -> bool {
        self.ffmpeg.is_some() && self.ffprobe.is_some()
    }

    pub fn ffmpeg_path(&self) -> Option<&Path> {
        self.ffmpeg.as_ref().map(|t| t.path.as_path())
    }

    pub fn ffprobe_path(&self) -> Option<&Path> {
        self.ffprobe.as_ref().map(|t| t.path.as_path())
    }
}

/// Look for a pip-installed VapourSynth's `vspipe`.
///
/// Mirrors `_find_pip_vspipe`: the wheel installs the executable into the
/// `vapoursynth` package directory inside `site-packages`. Only the common
/// per-user Windows layout is searched; a developer with a virtualenv can pass
/// an explicit deps directory instead.
fn find_pip_vspipe() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    let programs = Path::new(&local).join("Programs").join("Python");
    let entries = std::fs::read_dir(programs).ok()?;

    for entry in entries.flatten() {
        let candidate = entry
            .path()
            .join("Lib")
            .join("site-packages")
            .join("vapoursynth")
            .join(exe_name("vspipe"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

impl ToolPaths {
    /// Environment variables that let a portable vspipe find its own Python.
    ///
    /// Ported from `get_vspipe_env`. Three things matter and all three are
    /// easy to get wrong:
    ///
    /// * **Every `PYTHON*` variable must be removed.** The embedded Python
    ///   inside `_deps\vs` reads its module search path from its own `._pth`
    ///   file; an inherited `PYTHONHOME` or `PYTHONPATH` overrides that and the
    ///   interpreter dies with "Failed to import encodings" before it runs a
    ///   line of script.
    /// * **`portable.vs` must exist beside vsscript.dll**, or VSScript looks
    ///   for Python in the Windows registry — which finds the wrong install, or
    ///   none, on a machine with several.
    /// * **The deps directory and the plugin directory both go on `PATH`**, so
    ///   Windows can resolve the Python DLL and the plugins' own dependencies
    ///   (libfftw3 and friends) when `LoadPlugin` runs.
    pub fn vspipe_env(&self) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
        let mut env: Vec<(std::ffi::OsString, std::ffi::OsString)> = Vec::new();
        let Some(vs) = &self.vs_deps_dir else {
            return env;
        };

        // The marker is what switches VSScript into portable mode.
        let marker = vs.join("portable.vs");
        if !marker.exists() {
            let _ = std::fs::write(&marker, b"");
        }

        // …and from R70 onward `portable.vs` is no longer enough on its own.
        register_vsscript(vs);

        let plugin_dir = ["plugins64", "plugins"]
            .iter()
            .map(|d| vs.join(d))
            .find(|p| p.is_dir())
            .unwrap_or_else(|| vs.join("plugins64"));

        env.push(("VSPluginPath".into(), plugin_dir.clone().into_os_string()));

        let mut path = std::ffi::OsString::from(vs);
        path.push(";");
        path.push(&plugin_dir);
        if let Some(existing) = std::env::var_os("PATH") {
            path.push(";");
            path.push(existing);
        }
        env.push(("PATH".into(), path));

        env
    }

    /// Variables that must be cleared before running vspipe.
    pub fn vspipe_env_removals(&self) -> Vec<std::ffi::OsString> {
        let mut out: Vec<std::ffi::OsString> = std::env::vars_os()
            .map(|(k, _)| k)
            .filter(|k| {
                k.to_string_lossy()
                    .to_ascii_uppercase()
                    .starts_with("PYTHON")
            })
            .collect();
        for extra in ["__PYVENV_LAUNCHER__", "VIRTUAL_ENV", "VIRTUAL_ENV_PROMPT"] {
            out.push(extra.into());
        }
        out
    }
}

/// Tell VapourSynth which Python belongs to the bundled `vsscript.dll`.
///
/// From R70 onward VapourSynth will not guess. It reads
/// `%APPDATA%\VapourSynth\vapoursynth.toml`, which maps the absolute path of
/// each `vsscript.dll` to the interpreter that goes with it, and without an
/// entry it refuses to start with:
///
/// ```text
/// Failed to initialize VSScript. VSScript reported error: Python executable
/// and library path couldn't be determined despite automatic configuration.
/// Run `vapoursynth config` to set it for this Python installation.
/// ```
///
/// `portable.vs` alone does *not* satisfy this, which is the trap: the marker
/// file is present, portable mode is on, and the thing still will not run.
///
/// This is not a hypothetical. The bundled payload failed exactly this way on
/// the development machine — every preview and every export died with that
/// message — and the only entry in the file pointed at an unrelated
/// VapourSynth in a temporary directory. A freshly installed payload has no
/// entry at all, so **every** new user would have hit it. Writing the entry
/// here means the app registers itself the first time it looks for its tools.
///
/// Failure is silent. The entry is a repair for a machine that needs it, not
/// a precondition: a VapourSynth that can already find its Python works
/// whether or not this succeeds, and a read-only APPDATA is not a reason to
/// refuse to start.
fn register_vsscript(vs: &Path) {
    let (Some(script), Some(python)) = (
        canonical_str(&vs.join("vsscript.dll")),
        canonical_str(&vs.join(exe_name("python"))),
    ) else {
        return;
    };

    // The interpreter's own DLL, whose name carries the version: python314.dll
    // for 3.14. Found by looking rather than hardcoded, so a payload built
    // against a different Python keeps working.
    //
    // Picked by the *most* digits, because the directory holds both
    // `python314.dll` and `python3.dll` — the latter being only the stable-ABI
    // forwarder. Taking whichever the directory listing happened to yield
    // first gave `python3.dll`, which works but names the wrong file; the
    // versioned one is the actual interpreter and is what VapourSynth's own
    // configuration tool records.
    let Some(dll) = std::fs::read_dir(vs).ok().and_then(|entries| {
        entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                    let n = n.to_ascii_lowercase();
                    n.starts_with("python")
                        && n.ends_with(".dll")
                        && n.contains(|c: char| c.is_ascii_digit())
                })
            })
            .max_by_key(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.chars().filter(char::is_ascii_digit).count())
                    .unwrap_or(0)
            })
    }) else {
        return;
    };
    let Some(dll) = canonical_str(&dll) else {
        return;
    };

    let Some(appdata) = std::env::var_os("APPDATA") else {
        return;
    };
    let dir = Path::new(&appdata).join("VapourSynth");
    let config = dir.join("vapoursynth.toml");

    // VapourSynth writes the key lowercased; match it so a second entry for
    // the same DLL in different casing cannot accumulate.
    //
    // The comparison is against the *escaped* key, which is the shape the
    // file actually holds. Comparing the raw path never matched — every path
    // in the file has its backslashes doubled — so this appended a fresh
    // line on every single call and the config grew without bound. Seventeen
    // identical entries for one DLL turned up within minutes of a fresh
    // install before this was caught.
    let key = script.to_lowercase();
    let escaped_key = toml_escape(&key).to_lowercase();
    let existing = std::fs::read_to_string(&config).unwrap_or_default();

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let line = format!(
        "\"{}\" = [\"{}\",\"{}\",\"{stamp}\"]",
        toml_escape(&key),
        toml_escape(&python),
        toml_escape(&dll),
    );

    // Rebuild rather than append. Dropping every existing line for this DLL
    // and writing one fresh entry both de-duplicates and repairs a file that
    // earlier versions of this function already filled with copies — and it
    // keeps a stale entry from winning when a payload is replaced in place.
    let mut kept: Vec<&str> = existing
        .lines()
        .filter(|l| {
            let lower = l.to_lowercase();
            !lower.starts_with(&format!("\"{escaped_key}\""))
        })
        .collect();

    // Nothing to do if the entry is already correct and alone, so an
    // unchanged file is not rewritten on every run.
    let already = existing
        .lines()
        .filter(|l| l.to_lowercase().starts_with(&format!("\"{escaped_key}\"")))
        .collect::<Vec<_>>();
    if already.len() == 1 && already[0].starts_with(&line[..line.rfind(',').unwrap_or(line.len())]) {
        return;
    }

    kept.push(&line);
    let mut out = kept.join("\n");
    out.push('\n');

    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(&config, out);
}

/// An absolute path as a string, with the verbatim prefix Windows canonical
/// paths carry stripped — VapourSynth compares plain paths and will not match
/// one that still has it.
fn canonical_str(path: &Path) -> Option<String> {
    let full = std::fs::canonicalize(path).ok()?;
    let s = full.to_str()?.to_string();
    Some(s.strip_prefix(r"\\?\").unwrap_or(&s).to_string())
}

/// Escape a Windows path for a TOML basic string.
fn toml_escape(s: &str) -> String {
    s.replace('\\', r"\\").replace('"', "\\\"")
}
