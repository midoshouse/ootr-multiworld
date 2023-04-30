#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, warnings)]
#![forbid(unsafe_code)]

use {
    std::{
        env,
        fs::File,
        io::prelude::*,
        path::PathBuf,
    },
    winres::WindowsResource,
};

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] Io(#[from] std::io::Error),
    #[error("missing environment variable")]
    Env,
}

fn main() -> Result<(), Error> {
    println!("cargo:rerun-if-changed=nonexistent.foo"); // check a nonexistent file to make sure build script is always run (see https://github.com/rust-lang/cargo/issues/4213 and https://github.com/rust-lang/cargo/issues/5663)
    if env::var_os("CARGO_CFG_WINDOWS").is_some() {
        WindowsResource::new()
            .set_icon("../../assets/icon.ico")
            .compile()?;
    }
    let bizhawk_version = File::create(PathBuf::from(env::var_os("OUT_DIR").ok_or(Error::Env)?).join("bizhawk_version.rs"))?;
    let [major, minor, patch, _] = multiworld_bizhawk::bizhawk_version();
    write!(&bizhawk_version, "Version::new({major}, {minor}, {patch})")?;
    Ok(())
}
