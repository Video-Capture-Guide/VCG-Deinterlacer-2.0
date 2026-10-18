//! Print the generated VapourSynth script for a config.
//!
//! A development tool, not part of the product. It exists because the only
//! way to find out whether a generated script *runs* is to run it, and the
//! two defects in the first Modern QTGMC emission — a plugin that is not
//! bundled, and a denoise radius of 0 asking for motion compensation that
//! cannot be done at radius 0 — were both invisible to the unit tests, which
//! can only check what the text says.
//!
//! ```text
//! cargo run -p vcg-tools --example emit_vpy -- <deps-dir> <input> [modern]
//! ```

use vcg_core::config::{Config, QtgmcImpl};
use vcg_core::vpy::{generate_vpy, DepsPaths};

fn main() {
    let mut args = std::env::args().skip(1);
    let usage = "usage: emit_vpy <deps-dir> <input> [modern]";
    let deps = args.next().expect(usage);
    let input = args.next().expect(usage);
    let modern = args.next().is_some_and(|a| a == "modern");

    let config = Config {
        input_path: input,
        qtgmc_impl: if modern {
            QtgmcImpl::Modern
        } else {
            QtgmcImpl::Legacy
        },
        // So the denoise chain can be exercised from here too.
        noise_level: if std::env::var("VCG_DENOISE").is_ok() {
            vcg_core::config::NoiseLevel::Moderate
        } else {
            vcg_core::config::NoiseLevel::None
        },
        ..Config::default()
    };
    print!("{}", generate_vpy(&config, &DepsPaths::new(deps)));
}
