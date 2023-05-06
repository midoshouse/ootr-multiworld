//! No Rust code here, this crate just stores the C# code for the BizHawk tool

#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]
#![forbid(unsafe_code)]

use semver::Version;
#[cfg(windows)] use std::path::Path;

#[cfg(windows)]
pub fn bizhawk_version() -> [u16; 4] {
    winver::get_file_version_info(Path::new(env!("CARGO_MANIFEST_DIR")).join("OotrMultiworld").join("BizHawk").join("EmuHawk.exe")).expect("failed to parse BizHawk version")
}

pub fn version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("failed to parse current version")
}
