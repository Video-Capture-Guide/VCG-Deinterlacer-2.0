//! Stamp the Windows executable with its icon and version information.
//!
//! The window icon is set at runtime from `assets/icon-256.rgba`; this is the
//! other one — what Explorer, the taskbar's pinned entry and the file's
//! Properties dialog show, which lives in the binary's resource table and
//! cannot be set from inside the program.
//!
//! Only runs on Windows, and only when a resource compiler is available.
//! A missing compiler must not fail the build: the icon is a finishing touch
//! and refusing to produce a working binary without one would be the wrong
//! trade.

fn main() {
    println!("cargo:rerun-if-changed=assets/vcg.ico");
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/vcg.ico");
        res.set("ProductName", "VCG Deinterlacer");
        res.set("FileDescription", "VCG Deinterlacer");
        res.set("CompanyName", "Video Capture Guide");
        res.set("LegalCopyright", "Video Capture Guide");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the icon: {e}");
        }
    }
}
