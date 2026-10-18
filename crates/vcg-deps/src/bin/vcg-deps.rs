//! Check or install the dependency payload.
//!
//! The same code the first-run setup screen will call, exposed as a CLI so the
//! install can be exercised and inspected without a GUI.

use std::path::{Path, PathBuf};

use vcg_deps::install::{install, Phase};
use vcg_deps::manifest::CURRENT;
use vcg_deps::state::{state, State};

const USAGE: &str = "\
vcg-deps — check or install the VCG Deinterlacer dependency payload

USAGE:
    vcg-deps status [--deps <DIR>]
    vcg-deps install [--deps <DIR>] [--force]
    vcg-deps manifest

OPTIONS:
    --deps <DIR>   The _deps directory (default: ./_deps)
    --force        Reinstall even when the payload is already correct
    -h, --help     Print this help
";

fn main() {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "status".to_string());

    let mut deps: Option<PathBuf> = None;
    let mut force = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--deps" => match args.next() {
                Some(d) => deps = Some(PathBuf::from(d)),
                None => fail("--deps needs a directory"),
            },
            "--force" => force = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                return;
            }
            other => fail(&format!("unknown argument {other:?}")),
        }
    }

    let deps_dir = deps.unwrap_or_else(|| PathBuf::from("_deps"));

    match command.as_str() {
        "-h" | "--help" => print!("{USAGE}"),
        "manifest" => print_manifest(),
        "status" => print_status(&deps_dir),
        "install" => do_install(&deps_dir, force),
        other => fail(&format!("unknown command {other:?}")),
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("vcg-deps: {msg}\n");
    eprint!("{USAGE}");
    std::process::exit(2);
}

fn print_manifest() {
    println!("version      v{}", CURRENT.version);
    println!("url          {}", CURRENT.url);
    println!("size         {} bytes ({:.1} MiB)", CURRENT.size, CURRENT.size_mib());
    println!("sha256       {}", CURRENT.sha256);
    println!("archive root {}", CURRENT.archive_root);
    println!("\nrequired after install:");
    for entry in CURRENT.required_entries() {
        println!("  {entry}");
    }
}

fn print_status(deps_dir: &Path) {
    let s = state(deps_dir, &CURRENT);
    println!("deps dir  {}", deps_dir.display());
    println!("state     {}", s.describe(&CURRENT));
    match &s {
        State::Ready => println!("\nNothing to do."),
        State::Missing => println!("\nRun `vcg-deps install` to fetch v{}.", CURRENT.version),
        State::VersionMismatch { .. } => {
            println!("\nRun `vcg-deps install` to upgrade to v{}.", CURRENT.version)
        }
        State::Incomplete { missing } => {
            println!("\nThe version marker says v{} but these are absent:", CURRENT.version);
            for m in missing {
                println!("  {m}");
            }
            println!("Run `vcg-deps install --force` to reinstall.");
        }
    }
    if !s.is_ready() {
        std::process::exit(1);
    }
}

fn do_install(deps_dir: &Path, force: bool) {
    let before = state(deps_dir, &CURRENT);
    if before.is_ready() && !force {
        println!("Already {}. Use --force to reinstall.", before.describe(&CURRENT));
        return;
    }

    println!("Installing v{} into {}", CURRENT.version, deps_dir.display());
    println!("  from {}", CURRENT.url);
    println!("  {:.1} MiB, sha256 {}…", CURRENT.size_mib(), &CURRENT.sha256[..16]);
    println!();

    // Progress on stderr, so stdout stays a clean record of what happened.
    let mut last_pct = u64::MAX;
    let mut last_phase = String::new();
    let mut progress = |phase: Phase| {
        use std::io::Write;
        let mut err = std::io::stderr();
        match phase {
            Phase::Downloading { received, total } => {
                let pct = received * 100 / total.max(1);
                // Only redraw on a whole-percent change; a 135 MB download
                // otherwise emits thousands of lines when piped.
                if pct != last_pct {
                    last_pct = pct;
                    let _ = write!(
                        err,
                        "\r  downloading  {pct:>3}%  {:.1} / {:.1} MiB   ",
                        received as f64 / 1_048_576.0,
                        total as f64 / 1_048_576.0
                    );
                    let _ = err.flush();
                }
            }
            Phase::Verifying => {
                let _ = writeln!(err, "\r  downloading  done                        ");
                let _ = write!(err, "  verifying    checking SHA-256…");
                let _ = err.flush();
            }
            Phase::Extracting { done, total } => {
                if last_phase != "extract" {
                    let _ = writeln!(err, "\r  verifying    ok                       ");
                    last_phase = "extract".to_string();
                }
                let pct = done * 100 / total.max(1);
                let _ = write!(err, "\r  extracting   {pct:>3}%  {done} / {total} entries   ");
                let _ = err.flush();
            }
            Phase::Installing => {
                let _ = writeln!(err, "\r  extracting   done                        ");
                let _ = writeln!(err, "  installing   swapping into place");
            }
        }
    };

    match install(deps_dir, &CURRENT, Some(&mut progress)) {
        Ok(()) => {
            let after = state(deps_dir, &CURRENT);
            println!("Installed. State is now: {}", after.describe(&CURRENT));
            if !after.is_ready() {
                eprintln!("But the result is not ready — this should not happen.");
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("\nInstall failed: {e}");
            // Worth saying plainly, because the Python's installer could not.
            let after = state(deps_dir, &CURRENT);
            match after {
                State::Missing => eprintln!("No payload is installed."),
                _ => eprintln!(
                    "The existing payload was left untouched: {}",
                    after.describe(&CURRENT)
                ),
            }
            std::process::exit(1);
        }
    }
}
