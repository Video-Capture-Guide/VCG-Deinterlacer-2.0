//! Invariants of the dependency installer.
//!
//! The one that matters most is `a_bad_archive_does_not_destroy_a_working_install`.
//! The Python's `_extract` calls `shutil.rmtree(DEPS_DIR)` before unpacking, so
//! any failure after that point leaves the user with nothing, having started
//! with a working payload. That is the regression this suite exists to prevent.
//!
//! No test touches the network: `install_from_archive` takes a local file and
//! still applies the full size and digest check.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use vcg_deps::install::{install_from_archive, Error};
use vcg_deps::manifest::Manifest;
use vcg_deps::state::{state, State};

/// A scratch directory unique to one test.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("vcg-deps-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn deps(&self) -> PathBuf {
        self.0.join("_deps")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Build a zip at `dest` from `(path, contents)` pairs, and return a manifest
/// pinned to the archive that was actually written.
///
/// The digest has to be computed after the fact, so the manifest's `'static`
/// strings are leaked. That is fine for a test and keeps `Manifest` a plain
/// const in real use.
fn make_archive(dest: &Path, root: &str, entries: &[(&str, &[u8])]) -> Manifest {
    let file = fs::File::create(dest).expect("create archive");
    let mut zip = zip::ZipWriter::new(file);
    let opts: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    for (name, body) in entries {
        // An entry name starting with the root is used verbatim, so a test can
        // write a traversal path deliberately.
        let full = if name.starts_with("..") || name.starts_with('/') {
            (*name).to_string()
        } else {
            format!("{root}/{name}")
        };
        zip.start_file(full, opts).expect("start entry");
        zip.write_all(body).expect("write entry");
    }
    zip.finish().expect("finish archive");

    let bytes = fs::read(dest).expect("read back archive");
    let digest: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();

    Manifest {
        version: "99",
        url: "http://example.invalid/unused.zip",
        sha256: Box::leak(digest.into_boxed_str()),
        size: bytes.len() as u64,
        archive_root: Box::leak(root.to_string().into_boxed_str()),
    }
}

/// The entries a complete payload needs, per `Manifest::required_entries`.
fn complete_payload() -> Vec<(&'static str, &'static [u8])> {
    vec![
        ("vcg_deps.version", b"99\n" as &[u8]),
        ("ffmpeg/ffmpeg.exe", b"not really ffmpeg"),
        ("ffmpeg/ffprobe.exe", b"not really ffprobe"),
        ("vs/vspipe.exe", b"not really vspipe"),
        ("vs/plugins64/placeholder.dll", b"not really a plugin"),
    ]
}

/// Put a recognisable "existing install" in place.
fn seed_existing_install(deps: &Path, version: &str) {
    fs::create_dir_all(deps.join("ffmpeg")).expect("seed ffmpeg dir");
    fs::create_dir_all(deps.join("vs").join("plugins64")).expect("seed plugins dir");
    fs::write(deps.join("vcg_deps.version"), version).expect("seed marker");
    fs::write(deps.join("ffmpeg/ffmpeg.exe"), b"ORIGINAL").expect("seed ffmpeg");
    fs::write(deps.join("ffmpeg/ffprobe.exe"), b"ORIGINAL").expect("seed ffprobe");
    fs::write(deps.join("vs/vspipe.exe"), b"ORIGINAL").expect("seed vspipe");
    fs::write(deps.join("sentinel.txt"), b"DO NOT LOSE ME").expect("seed sentinel");
}

// ── The manifest ────────────────────────────────────────────────────────────

#[test]
fn the_shipped_manifest_is_well_formed() {
    let m = vcg_deps::manifest::CURRENT;
    assert_eq!(m.sha256.len(), 64, "SHA-256 must be 64 hex characters");
    assert!(
        m.sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "digest must be lowercase hex: {}",
        m.sha256
    );
    assert!(m.size > 0);
    // The URL is built with a line continuation in the source, which silently
    // breaks if the indentation is edited.
    assert!(!m.url.contains(' '), "URL contains a space: {:?}", m.url);
    assert!(m.url.starts_with("https://"), "must be HTTPS: {:?}", m.url);
    assert!(
        m.url.contains(&format!("v{}", m.version)),
        "URL {:?} does not mention version {}",
        m.url,
        m.version
    );
    assert!(m.archive_root.contains(m.version));
}

// ── Installed-state detection ───────────────────────────────────────────────

#[test]
fn state_distinguishes_the_ways_an_install_can_be_wrong() {
    let s = Scratch::new("state");
    let m = vcg_deps::manifest::CURRENT;

    // Nothing there at all.
    assert_eq!(state(&s.deps(), &m), State::Missing);

    // A marker with the wrong version.
    seed_existing_install(&s.deps(), "9");
    assert_eq!(
        state(&s.deps(), &m),
        State::VersionMismatch { found: "9".to_string() }
    );

    // Right version, but files absent — the case the atomic install prevents.
    fs::write(s.deps().join("vcg_deps.version"), m.version).expect("rewrite marker");
    fs::remove_file(s.deps().join("vs/vspipe.exe")).expect("remove vspipe");
    match state(&s.deps(), &m) {
        State::Incomplete { missing } => {
            assert!(missing.iter().any(|x| x.contains("vspipe")), "{missing:?}")
        }
        other => panic!("expected Incomplete, got {other:?}"),
    }

    // Put it back and it is ready.
    fs::write(s.deps().join("vs/vspipe.exe"), b"restored").expect("restore");
    assert_eq!(state(&s.deps(), &m), State::Ready);
}

#[test]
fn a_marker_with_surrounding_whitespace_still_matches() {
    // The archive's marker ends with a newline.
    let s = Scratch::new("marker-ws");
    let m = vcg_deps::manifest::CURRENT;
    seed_existing_install(&s.deps(), &format!("  {}\r\n", m.version));
    assert_eq!(state(&s.deps(), &m), State::Ready);
}

// ── Verification happens before anything is touched ─────────────────────────

#[test]
fn a_wrong_digest_is_rejected_and_installs_nothing() {
    let s = Scratch::new("digest");
    let archive = s.path().join("payload.zip");
    let mut m = make_archive(&archive, "vcg-deps-v99", &complete_payload());

    // Claim a different digest than the archive actually has.
    m.sha256 = "0000000000000000000000000000000000000000000000000000000000000000";

    let err = install_from_archive(&archive, &s.deps(), &m, None).expect_err("must reject");
    assert!(matches!(err, Error::DigestMismatch { .. }), "{err:?}");
    assert!(!s.deps().exists(), "nothing should have been installed");
}

#[test]
fn a_wrong_size_is_rejected() {
    let s = Scratch::new("size");
    let archive = s.path().join("payload.zip");
    let mut m = make_archive(&archive, "vcg-deps-v99", &complete_payload());
    m.size += 1;

    let err = install_from_archive(&archive, &s.deps(), &m, None).expect_err("must reject");
    assert!(matches!(err, Error::SizeMismatch { .. }), "{err:?}");
    assert!(!s.deps().exists());
}

#[test]
fn an_archive_that_escapes_the_target_is_refused() {
    // Zip slip. An archive is untrusted input even when we published it.
    let s = Scratch::new("slip");
    let archive = s.path().join("payload.zip");
    let mut entries = complete_payload();
    entries.push(("../../escaped.txt", b"should never be written"));
    let m = make_archive(&archive, "vcg-deps-v99", &entries);

    let err = install_from_archive(&archive, &s.deps(), &m, None).expect_err("must refuse");
    assert!(matches!(err, Error::UnsafeEntry(_)), "{err:?}");

    // And nothing escaped.
    assert!(!s.path().join("escaped.txt").exists());
    assert!(!s.path().parent().unwrap().join("escaped.txt").exists());
}

// ── The property the Python gets wrong ──────────────────────────────────────

#[test]
fn a_bad_archive_does_not_destroy_a_working_install() {
    // The Python rmtree's _deps and *then* extracts, so a failure here leaves
    // the user with nothing. Here the existing payload must survive untouched.
    let s = Scratch::new("atomic");
    seed_existing_install(&s.deps(), "10");

    let archive = s.path().join("payload.zip");
    // A payload missing vspipe.exe — passes its digest check, fails the layout
    // check that runs before the swap.
    let incomplete: Vec<(&str, &[u8])> = vec![
        ("vcg_deps.version", b"99\n"),
        ("ffmpeg/ffmpeg.exe", b"new ffmpeg"),
    ];
    let m = make_archive(&archive, "vcg-deps-v99", &incomplete);

    let err = install_from_archive(&archive, &s.deps(), &m, None).expect_err("must fail");
    assert!(matches!(err, Error::BadArchive(_)), "{err:?}");

    // The original install is intact, down to the file contents.
    assert_eq!(
        fs::read_to_string(s.deps().join("vcg_deps.version")).unwrap().trim(),
        "10"
    );
    assert_eq!(fs::read(s.deps().join("ffmpeg/ffmpeg.exe")).unwrap(), b"ORIGINAL");
    assert_eq!(fs::read(s.deps().join("vs/vspipe.exe")).unwrap(), b"ORIGINAL");
    assert_eq!(
        fs::read(s.deps().join("sentinel.txt")).unwrap(),
        b"DO NOT LOSE ME"
    );
}

#[test]
fn a_mismatched_version_inside_the_archive_is_refused_before_the_swap() {
    let s = Scratch::new("version-inside");
    seed_existing_install(&s.deps(), "10");

    let archive = s.path().join("payload.zip");
    let mut entries = complete_payload();
    // The marker says something other than the manifest's version.
    entries[0] = ("vcg_deps.version", b"7\n");
    let m = make_archive(&archive, "vcg-deps-v99", &entries);

    let err = install_from_archive(&archive, &s.deps(), &m, None).expect_err("must fail");
    assert!(matches!(err, Error::BadArchive(_)), "{err:?}");
    assert_eq!(fs::read(s.deps().join("sentinel.txt")).unwrap(), b"DO NOT LOSE ME");
}

// ── The happy path ──────────────────────────────────────────────────────────

#[test]
fn a_good_archive_installs_and_replaces_the_old_payload() {
    let s = Scratch::new("happy");
    seed_existing_install(&s.deps(), "10");

    let archive = s.path().join("payload.zip");
    let m = make_archive(&archive, "vcg-deps-v99", &complete_payload());

    install_from_archive(&archive, &s.deps(), &m, None).expect("should install");

    assert_eq!(state(&s.deps(), &m), State::Ready);
    assert_eq!(
        fs::read(s.deps().join("ffmpeg/ffmpeg.exe")).unwrap(),
        b"not really ffmpeg"
    );
    // The old payload is gone, not merged into the new one.
    assert!(
        !s.deps().join("sentinel.txt").exists(),
        "the previous install should have been replaced, not overlaid"
    );
    // No staging or backup left behind.
    assert!(!s.path().join(".vcg-deps-staging").exists());
    assert!(!s.path().join("_deps.previous").exists());
}

#[test]
fn installing_with_no_existing_payload_works() {
    let s = Scratch::new("fresh");
    let archive = s.path().join("payload.zip");
    let m = make_archive(&archive, "vcg-deps-v99", &complete_payload());

    install_from_archive(&archive, &s.deps(), &m, None).expect("should install");
    assert_eq!(state(&s.deps(), &m), State::Ready);
}

#[test]
fn an_archive_without_the_expected_wrapper_directory_still_works() {
    // Defensive: a payload repacked without the vcg-deps-vN folder should not
    // be a hard failure when the marker is unambiguously at the top.
    let s = Scratch::new("no-wrapper");
    let archive = s.path().join("payload.zip");
    let file = fs::File::create(&archive).expect("create");
    let mut zip = zip::ZipWriter::new(file);
    let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
    for (name, body) in complete_payload() {
        zip.start_file(name, opts).expect("entry");
        zip.write_all(body).expect("write");
    }
    zip.finish().expect("finish");

    let bytes = fs::read(&archive).expect("read");
    let digest: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    let m = Manifest {
        version: "99",
        url: "http://example.invalid/unused.zip",
        sha256: Box::leak(digest.into_boxed_str()),
        size: bytes.len() as u64,
        archive_root: "vcg-deps-v99",
    };

    install_from_archive(&archive, &s.deps(), &m, None).expect("should install");
    assert_eq!(state(&s.deps(), &m), State::Ready);
}

#[test]
fn progress_reports_every_phase_in_order() {
    let s = Scratch::new("progress");
    let archive = s.path().join("payload.zip");
    let m = make_archive(&archive, "vcg-deps-v99", &complete_payload());

    let mut seen: Vec<&'static str> = Vec::new();
    {
        let mut report = |p: vcg_deps::install::Phase| {
            use vcg_deps::install::Phase;
            let name = match p {
                Phase::Downloading { .. } => "download",
                Phase::Verifying => "verify",
                Phase::Extracting { .. } => "extract",
                Phase::Installing => "install",
            };
            if seen.last() != Some(&name) {
                seen.push(name);
            }
        };
        install_from_archive(&archive, &s.deps(), &m, Some(&mut report)).expect("install");
    }

    // No download phase here — the archive was already local.
    assert_eq!(seen, vec!["verify", "extract", "install"], "{seen:?}");
}
