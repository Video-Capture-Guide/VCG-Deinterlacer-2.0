//! Invariants of the process runner.
//!
//! The timeout and the pipe draining are the parts worth testing: both fail in
//! ways that look like a hang rather than an error, which is the hardest kind
//! of bug to attribute later. ffmpeg routinely writes far more than a pipe
//! buffer holds, so the "large output" test is the one that matters.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use vcg_tools::paths::{Origin, ToolPaths};
use vcg_tools::run::{run, DEFAULT_TIMEOUT};

/// A shell command, since these tests need *some* external process and the
/// system shell is the only one guaranteed present.
#[cfg(windows)]
fn shell(script: &str) -> Command {
    let mut c = Command::new("cmd");
    c.args(["/c", script]);
    c
}

#[cfg(not(windows))]
fn shell(script: &str) -> Command {
    let mut c = Command::new("sh");
    c.args(["-c", script]);
    c
}

#[test]
fn stdout_is_captured() {
    let out = run(shell("echo hello-from-child"), DEFAULT_TIMEOUT).expect("spawn");
    assert!(out.success(), "status {:?}", out.status);
    assert!(out.stdout.contains("hello-from-child"), "{:?}", out.stdout);
    assert!(!out.timed_out);
}

#[test]
fn a_nonzero_exit_is_reported_not_hidden() {
    let out = run(shell("exit 3"), DEFAULT_TIMEOUT).expect("spawn");
    assert!(!out.success());
    assert_eq!(out.status, Some(3));
}

#[test]
fn output_larger_than_a_pipe_buffer_does_not_deadlock() {
    // This is the whole reason the pipes are drained on their own threads. A
    // plain try_wait() loop hangs here forever: the child blocks writing to a
    // full pipe while the parent waits for it to exit. A pipe buffer is
    // typically 64 KB, so this writes well past it.
    #[cfg(windows)]
    let script = "for /L %i in (1,1,8000) do @echo \
                  xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";
    #[cfg(not(windows))]
    let script = "for i in $(seq 1 8000); do echo \
                  xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx; done";

    let started = Instant::now();
    let out = run(shell(script), Duration::from_secs(60)).expect("spawn");

    assert!(!out.timed_out, "deadlocked and hit the timeout instead");
    assert!(
        out.stdout.len() > 200_000,
        "only captured {} bytes — output was truncated",
        out.stdout.len()
    );
    assert!(
        started.elapsed() < Duration::from_secs(60),
        "took the full timeout, so it was not draining concurrently"
    );
}

#[test]
fn stderr_is_captured_separately_from_stdout() {
    #[cfg(windows)]
    let script = "echo to-out& echo to-err 1>&2";
    #[cfg(not(windows))]
    let script = "echo to-out; echo to-err 1>&2";

    let out = run(shell(script), DEFAULT_TIMEOUT).expect("spawn");
    assert!(out.stdout.contains("to-out"), "stdout {:?}", out.stdout);
    assert!(out.stderr.contains("to-err"), "stderr {:?}", out.stderr);
    // ffmpeg writes its measurements to stderr, so they must not be merged.
    assert!(!out.stdout.contains("to-err"));
    // combined() is the escape hatch for tools that pick either stream.
    assert!(out.combined().contains("to-err"));
}

#[test]
fn a_hung_process_is_killed_at_the_deadline() {
    // Without this, one corrupt file could hang the whole analysis.
    #[cfg(windows)]
    let script = "ping -n 30 127.0.0.1 >nul";
    #[cfg(not(windows))]
    let script = "sleep 30";

    let started = Instant::now();
    let out = run(shell(script), Duration::from_millis(600)).expect("spawn");
    let elapsed = started.elapsed();

    assert!(out.timed_out, "should have reported a timeout");
    assert!(
        elapsed < Duration::from_secs(10),
        "waited {elapsed:?} — the deadline was not enforced"
    );
}

#[test]
fn a_missing_program_is_an_error_not_a_panic() {
    let mut cmd = Command::new("vcg-definitely-not-a-real-program");
    cmd.arg("--version");
    assert!(run(cmd, DEFAULT_TIMEOUT).is_err());
}

// ── Tool discovery ──────────────────────────────────────────────────────────

#[test]
fn an_explicit_deps_dir_is_reported_as_explicit() {
    // The parity and probe tooling pins this so results never depend on what
    // happens to be installed on the machine.
    let dir = std::env::temp_dir().join("vcg-tools-test-deps");
    let ffmpeg_dir = dir.join("ffmpeg");
    std::fs::create_dir_all(&ffmpeg_dir).expect("create temp deps");

    let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    std::fs::write(ffmpeg_dir.join(name), b"not really ffmpeg").expect("write stub");

    let tools = ToolPaths::discover(Some(&dir));
    let ffmpeg = tools.ffmpeg.expect("the stub should be found");
    assert_eq!(ffmpeg.origin, Origin::Explicit);
    assert!(ffmpeg.path.starts_with(&dir));
    assert_eq!(tools.deps_dir.as_deref(), Some(dir.as_path()));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_deps_dir_with_nothing_in_it_falls_back_rather_than_inventing_paths() {
    let dir = std::env::temp_dir().join("vcg-tools-test-empty-deps");
    std::fs::create_dir_all(&dir).expect("create temp deps");

    let tools = ToolPaths::discover(Some(&dir));
    // There is no bundled vs/ directory, so nothing should claim there is.
    assert_eq!(tools.vs_deps_dir, None);
    // Any ffmpeg found must have come from PATH, never from the empty dir.
    if let Some(t) = &tools.ffmpeg {
        assert_ne!(t.origin, Origin::Explicit, "found {:?}", t.path);
        assert!(t.path.is_file());
    }
    assert!(!tools.can_probe() || tools.ffmpeg.is_some());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_nonexistent_deps_dir_does_not_panic() {
    let tools = ToolPaths::discover(Some(Path::new("Z:/no/such/deps")));
    assert_eq!(tools.vs_deps_dir, None);
}

/// Every child process must have its console suppressed.
///
/// This is a source-level check because nothing else catches it: the code
/// compiles, the tests pass, the output is correct, and the only symptom is
/// two black windows flashing up on the user's screen during an export —
/// which is exactly what shipped. `run::command` applies the flag, so the
/// rule is simply that nothing outside `run.rs` constructs a `Command`
/// directly.
#[test]
fn no_bare_command_new() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();

    let mut stack = vec![dir];
    while let Some(path) = stack.pop() {
        for entry in std::fs::read_dir(&path).expect("src is readable") {
            let entry = entry.expect("a directory entry");
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            // run.rs is where the flag is applied, so it is the one place
            // allowed to build a Command by hand.
            if p.file_name().is_some_and(|n| n == "run.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&p).expect("source is utf-8");
            for (i, line) in text.lines().enumerate() {
                if line.contains("Command::new") {
                    offenders.push(format!(
                        "{}:{}: {}",
                        p.file_name().unwrap().to_string_lossy(),
                        i + 1,
                        line.trim()
                    ));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "use run::command(..) so the console stays hidden:\n  {}",
        offenders.join("\n  ")
    );
}
