//! Running ffmpeg and ffprobe without flashing a console window.
//!
//! Ported from `run_hidden`. Two things make this more than a `Command::output()`
//! wrapper:
//!
//! * **No console window.** On Windows a GUI process spawning a console
//!   application pops a black window for each call. `CREATE_NO_WINDOW` plus a
//!   hidden `STARTUPINFO` suppresses it — the same pair the Python sets.
//! * **A timeout that cannot deadlock.** `std` has no timed wait, and the
//!   obvious loop around `try_wait()` deadlocks as soon as ffmpeg fills the
//!   stderr pipe buffer and blocks on write while we wait for exit. The pipes
//!   are therefore drained on their own threads for the whole run.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// What a finished (or killed) process produced.
#[derive(Debug, Clone)]
pub struct Output {
    /// `None` if the process was killed or died without a code.
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    /// The process overran its deadline and was killed.
    pub timed_out: bool,
}

impl Output {
    pub fn success(&self) -> bool {
        self.status == Some(0)
    }

    /// Both streams, for when a tool's diagnostics could land on either.
    pub fn combined(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

/// How long to wait before giving up on a tool.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(45);

/// Suppress the console window a child process would otherwise get.
///
/// Must be applied to *every* spawn. The GUI is a windowed binary, so any
/// child started without this flashes up its own console — two of them, in
/// the case of the export, which pipes VapourSynth into FFmpeg.
#[cfg(windows)]
pub fn hide_console(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    /// CREATE_NO_WINDOW — no console is allocated for the child.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub fn hide_console(_cmd: &mut Command) {}

/// Build a child process with its console already suppressed.
///
/// Use this instead of `Command::new` everywhere in this crate. The export
/// shipped with two console windows flashing up because it built its vspipe
/// and ffmpeg commands by hand and never reached `hide_console`; a
/// constructor that cannot forget is the fix, and `no_bare_command_new`
/// keeps it that way.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut cmd = Command::new(program);
    hide_console(&mut cmd);
    cmd
}

/// Drain one pipe on its own thread so the child can never block on a full
/// buffer while we are waiting for it to exit.
fn drain<R: Read + Send + 'static>(mut pipe: R) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        // A read error still sends whatever arrived; losing diagnostics is
        // better than hanging.
        let _ = pipe.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    rx
}

/// Drain a pipe on its own thread, delivering it as a lossy string.
///
/// Used where the caller wants to wait on the children itself but must still
/// keep the pipes empty, or the child blocks on a full buffer.
pub(crate) fn drain_to_string<R: Read + Send + 'static>(
    pipe: R,
) -> mpsc::Receiver<String> {
    let bytes = drain(pipe);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let data = bytes.recv().unwrap_or_default();
        let _ = tx.send(String::from_utf8_lossy(&data).into_owned());
    });
    rx
}

/// Collect a drained pipe without being able to wait forever.
///
/// A blocking `recv()` here looks correct and is not: killing a process does
/// not kill *its* children, and a grandchild inherits the pipe handles. The
/// pipe then stays open after the kill, the drain thread stays blocked in
/// `read_to_end`, and the timeout bounds nothing at all — the call returns only
/// when the grandchild finally exits. Bounding this wait is what makes the
/// deadline real.
///
/// On a clean exit the writer is gone, so the read completes at once and the
/// generous grace is never spent. Partial output is better than hanging.
fn collect(rx: &mpsc::Receiver<Vec<u8>>, timed_out: bool) -> Vec<u8> {
    let grace = if timed_out {
        // Already given up on this process; do not linger for its output.
        Duration::from_millis(500)
    } else {
        Duration::from_secs(10)
    };
    rx.recv_timeout(grace).unwrap_or_default()
}

/// Wait for `child`, killing it if `timeout` elapses.
fn wait_with_deadline(child: &mut Child, timeout: Duration) -> (Option<i32>, bool) {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return (status.code(), false),
            Ok(None) => {}
            // Treat an unwaitable child as finished rather than spinning.
            Err(_) => return (None, false),
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return (None, true);
        }
        // Long enough not to busy-spin, short enough that a fast ffprobe call
        // is not noticeably delayed.
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Run a command with no console window and a hard deadline.
///
/// Output is decoded lossily: ffmpeg writes paths and metadata in the system
/// encoding, and a stray byte must not lose the whole log.
pub fn run(mut cmd: Command, timeout: Duration) -> std::io::Result<Output> {
    hide_console(&mut cmd);
    // stdin is closed so a tool that would otherwise prompt (ffmpeg's
    // "overwrite?") fails fast instead of hanging forever.
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn()?;
    let out_rx = drain(child.stdout.take().expect("stdout is piped"));
    let err_rx = drain(child.stderr.take().expect("stderr is piped"));

    let (status, timed_out) = wait_with_deadline(&mut child, timeout);

    let stdout = collect(&out_rx, timed_out);
    let stderr = collect(&err_rx, timed_out);

    Ok(Output {
        status,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        timed_out,
    })
}

/// Run a program with arguments, with the default timeout.
pub fn run_tool<I, S>(program: &std::path::Path, args: I) -> std::io::Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut cmd = Command::new(program);
    cmd.args(args);
    run(cmd, DEFAULT_TIMEOUT)
}

/// Run a program and return its raw stdout bytes, for binary output such as a
/// raw video frame.
pub fn run_tool_binary<I, S>(
    program: &std::path::Path,
    args: I,
    timeout: Duration,
) -> std::io::Result<(Option<i32>, Vec<u8>)>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut cmd = Command::new(program);
    cmd.args(args);
    hide_console(&mut cmd);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn()?;
    let out_rx = drain(child.stdout.take().expect("stdout is piped"));
    let err_rx = drain(child.stderr.take().expect("stderr is piped"));
    let (status, timed_out) = wait_with_deadline(&mut child, timeout);
    let stdout = collect(&out_rx, timed_out);
    // Drained and discarded; it must still be read or the child can block.
    let _ = collect(&err_rx, timed_out);
    Ok((status, stdout))
}

/// Run `producer`, feeding its stdout into `consumer`, and collect the
/// consumer's stdout.
///
/// Used for `vspipe | ffmpeg`: VapourSynth writes y4m, ffmpeg converts it to
/// something displayable. A temporary file would work but would mean writing
/// and reading an uncompressed frame for no reason.
///
/// Both children are waited on. The producer's stderr is captured and returned
/// because that is where VapourSynth reports script errors, and those are the
/// ones worth showing.
pub fn run_piped(
    mut producer: Command,
    mut consumer: Command,
    timeout: Duration,
) -> std::io::Result<(Vec<u8>, String)> {
    hide_console(&mut producer);
    hide_console(&mut consumer);

    producer
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut prod = producer.spawn()?;
    let prod_out = prod.stdout.take().expect("stdout is piped");
    let prod_err = drain(prod.stderr.take().expect("stderr is piped"));

    consumer
        .stdin(Stdio::from(prod_out))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut cons = match consumer.spawn() {
        Ok(c) => c,
        Err(e) => {
            // Leaving the producer running would hold a pipe open forever.
            let _ = prod.kill();
            let _ = prod.wait();
            return Err(e);
        }
    };
    let cons_out = drain(cons.stdout.take().expect("stdout is piped"));
    let cons_err = drain(cons.stderr.take().expect("stderr is piped"));

    let (_, cons_timed_out) = wait_with_deadline(&mut cons, timeout);
    // The producer exits once its reader is gone; kill it if it does not.
    let (_, _) = wait_with_deadline(&mut prod, Duration::from_secs(5));

    let out = collect(&cons_out, cons_timed_out);
    let err = String::from_utf8_lossy(&collect(&prod_err, cons_timed_out)).into_owned();
    let _ = collect(&cons_err, cons_timed_out);
    Ok((out, err))
}
