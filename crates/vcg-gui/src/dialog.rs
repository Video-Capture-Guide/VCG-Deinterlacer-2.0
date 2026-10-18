//! Native file dialogs, opened over the main window.
//!
//! Two rules, both learned from one bug report that read as three.
//!
//! **A dialog is queued during the frame and opened after it.** The window
//! handle lives on `eframe::Frame`, which only `ui()` holds, so a page asks
//! for a dialog by name and the shell opens it once drawing is done.
//!
//! **Every dialog names its owner.** Without one the Windows common dialog is
//! a top-level window in its own right and can come up *behind* the app. The
//! app is then blocked in that dialog's modal loop with nothing on screen to
//! say so: the Browse button looks like it did nothing, files dropped on the
//! window are ignored, and the close button does not answer. All three were
//! reported separately. An owner puts the dialog over the window that asked
//! for it, which is what makes the state of things obvious.

use std::path::PathBuf;

/// A dialog a page has asked the shell to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    /// The source video.
    Source,
    /// Which folder the export is written to.
    OutputFolder,
    /// An image to burn in as a watermark.
    Logo,
    /// A settings JSON written by an earlier export, to read back in.
    LoadSettings,
}

/// The video extensions offered in the source picker.
///
/// Also the filter the drop handler would use if it filtered, which it does
/// not: a file the user deliberately dragged onto the window is a stronger
/// statement of intent than a list of extensions.
pub const VIDEO_EXTENSIONS: &[&str] = &[
    "avi", "mp4", "mkv", "mov", "mts", "m2ts", "mod", "dv", "vob", "mpg", "wmv",
];

/// Open `which`, owned by `parent`, and return what was picked.
///
/// `parent` is the running `eframe::Frame`, which carries the window and
/// display handles. Taking it by generic bound rather than by name means the
/// call site cannot forget it.
pub fn open(which: Dialog, parent: &eframe::Frame, start_in: Option<PathBuf>) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new().set_parent(parent);
    if let Some(dir) = start_in {
        dialog = dialog.set_directory(dir);
    }
    match which {
        Dialog::Source => dialog
            .add_filter("Video", VIDEO_EXTENSIONS)
            .add_filter("All files", &["*"])
            .set_title("Choose a video")
            .pick_file(),
        Dialog::OutputFolder => dialog.set_title("Choose a folder").pick_folder(),
        Dialog::Logo => dialog
            .add_filter("Image", &["png", "jpg", "jpeg", "bmp", "gif"])
            .set_title("Choose an image")
            .pick_file(),
        Dialog::LoadSettings => dialog
            .add_filter("Settings", &["json"])
            .set_title("Load settings")
            .pick_file(),
    }
}
