//! Fetching the support payload, with the window open.
//!
//! The app is useless without `_deps`: no ffmpeg, no VapourSynth, so no
//! analysis, no preview and no export. Until now the GUI only *read* that
//! state and said "not ready" — the download existed solely in the
//! `vcg-deps` command-line tool, which is not even in the release zip. A new
//! user therefore had a working window and no way at all to make it work,
//! while the About page told them it downloads on first run.
//!
//! So it runs here, automatically, on a worker thread, the first time the app
//! opens without a payload. Automatically because there is nothing to decide:
//! the alternative to downloading it is not using the program.
//!
//! No cancel button, deliberately. `install::Progress` is a `FnMut(Phase)`
//! with no return value, so there is nowhere for a stop signal to go without
//! changing that crate's API, and closing the window already does the job —
//! `install` stages everything beside `_deps` and removes a stale staging
//! directory on the next run, so an abandoned download costs disk until the
//! next launch and nothing else.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};

use vcg_deps::install::{self, Phase};
use vcg_deps::manifest::Manifest;

/// What the worker thread sends back.
enum Message {
    Progress(Phase),
    Done(Result<(), String>),
}

/// How far along the install is, for the UI.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    /// Nothing has happened yet.
    Idle,
    /// Running, with a line to show and a 0.0–1.0 fraction where one is known.
    Working { note: String, fraction: Option<f32> },
    /// Finished. The payload is in place.
    Done,
    /// Finished badly, with something to show the user.
    Failed(String),
}

/// A download in progress.
pub struct Install {
    rx: Receiver<Message>,
    pub status: Status,
}

impl Install {
    /// Start downloading into `deps_dir`.
    pub fn start(deps_dir: PathBuf, manifest: &'static Manifest) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let progress_tx = tx.clone();
            let mut report = |phase: Phase| {
                // A failed send means the window closed; the install carries
                // on to a consistent state rather than being torn off midway.
                let _ = progress_tx.send(Message::Progress(phase));
            };

            let outcome = install::install(&deps_dir, manifest, Some(&mut report))
                .map_err(|e| e.to_string());
            let _ = tx.send(Message::Done(outcome));
        });

        Self {
            rx,
            status: Status::Working {
                note: "Starting…".to_string(),
                fraction: None,
            },
        }
    }

    /// Drain whatever the worker has sent. True when it has finished.
    ///
    /// Every pending message is taken, not just one: progress arrives far
    /// faster than frames are drawn, and reading a single message per frame
    /// would show a download that lags further behind the longer it runs.
    pub fn poll(&mut self) -> bool {
        loop {
            match self.rx.try_recv() {
                Ok(Message::Progress(phase)) => self.status = describe(phase),
                Ok(Message::Done(Ok(()))) => {
                    self.status = Status::Done;
                    return true;
                }
                Ok(Message::Done(Err(e))) => {
                    self.status = Status::Failed(e);
                    return true;
                }
                Err(TryRecvError::Empty) => return false,
                Err(TryRecvError::Disconnected) => {
                    // Only reachable if the thread died without sending, which
                    // should not happen. Reporting it beats spinning forever.
                    if !matches!(self.status, Status::Done | Status::Failed(_)) {
                        self.status =
                            Status::Failed("the download stopped unexpectedly".to_string());
                    }
                    return true;
                }
            }
        }
    }
}

/// Turn a phase into something worth reading.
///
/// Sizes in MiB because that is the unit the release page and the manifest
/// use; a progress line that disagrees with the number beside the download
/// invites the question of which one is wrong.
fn describe(phase: Phase) -> Status {
    match phase {
        Phase::Downloading { received, total } => {
            let mib = |b: u64| b as f64 / 1_048_576.0;
            let fraction = if total > 0 {
                Some((received as f32 / total as f32).clamp(0.0, 1.0))
            } else {
                None
            };
            Status::Working {
                note: format!("Downloading… {:.0} of {:.0} MiB", mib(received), mib(total)),
                fraction,
            }
        }
        Phase::Verifying => Status::Working {
            note: "Checking the download…".to_string(),
            fraction: None,
        },
        Phase::Extracting { done, total } => {
            let fraction = if total > 0 {
                Some((done as f32 / total as f32).clamp(0.0, 1.0))
            } else {
                None
            };
            Status::Working {
                note: format!("Unpacking… {done} of {total}"),
                fraction,
            }
        }
        Phase::Installing => Status::Working {
            note: "Finishing…".to_string(),
            fraction: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_reports_both_sizes_and_a_fraction() {
        let s = describe(Phase::Downloading {
            received: 52_428_800,
            total: 104_857_600,
        });
        match s {
            Status::Working { note, fraction } => {
                assert_eq!(note, "Downloading… 50 of 100 MiB");
                assert_eq!(fraction, Some(0.5));
            }
            other => panic!("{other:?}"),
        }
    }

    /// A total of zero would otherwise divide by zero and show NaN as a bar
    /// filled to an arbitrary point.
    #[test]
    fn an_unknown_total_has_no_fraction() {
        let s = describe(Phase::Downloading {
            received: 10,
            total: 0,
        });
        assert!(matches!(s, Status::Working { fraction: None, .. }));
    }

    /// Progress past the end would paint outside the bar.
    #[test]
    fn a_fraction_cannot_exceed_one() {
        let s = describe(Phase::Downloading {
            received: 200,
            total: 100,
        });
        assert!(matches!(
            s,
            Status::Working {
                fraction: Some(f),
                ..
            } if f == 1.0
        ));
    }

    #[test]
    fn extracting_counts_entries() {
        let s = describe(Phase::Extracting { done: 3, total: 12 });
        match s {
            Status::Working { note, fraction } => {
                assert_eq!(note, "Unpacking… 3 of 12");
                assert_eq!(fraction, Some(0.25));
            }
            other => panic!("{other:?}"),
        }
    }
}
