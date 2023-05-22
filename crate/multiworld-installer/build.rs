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
#[cfg(not(windows))] use {
    std::{
        iter,
        str::FromStr as _,
    },
    itertools::Itertools as _,
    multiworld::github::Repo,
};

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] Io(#[from] std::io::Error),
    #[cfg(not(windows))] #[error(transparent)] ParseInt(#[from] std::num::ParseIntError),
    #[cfg(not(windows))] #[error(transparent)] Reqwest(#[from] reqwest::Error),
    #[error("missing environment variable")]
    Env,
    #[cfg(not(windows))]
    #[error("no BizHawk releases found")]
    NoBizHawkReleases,
}

#[wheel::main]
async fn main() -> Result<(), Error> {
    println!("cargo:rerun-if-changed=nonexistent.foo"); // check a nonexistent file to make sure build script is always run (see https://github.com/rust-lang/cargo/issues/4213 and https://github.com/rust-lang/cargo/issues/5663)
    if env::var_os("CARGO_CFG_WINDOWS").is_some() {
        WindowsResource::new()
            .set_icon("../../assets/icon.ico")
            .compile()?;
    }
    let bizhawk_version = File::create(PathBuf::from(env::var_os("OUT_DIR").ok_or(Error::Env)?).join("bizhawk_version.rs"))?;
    let (major, minor, patch) = {
        #[cfg(windows)] {
            let [major, minor, patch, _] = multiworld_bizhawk::bizhawk_version();
            (major, minor, patch)
        }
        #[cfg(not(windows))] {
            let http_client = reqwest::Client::builder()
                .user_agent(concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")))
                .use_rustls_tls()
                .https_only(true)
                .http2_prior_knowledge()
                .build()?;
            let version = Repo::new("TASEmulators", "BizHawk").latest_release(&http_client).await?.ok_or(Error::NoBizHawkReleases)?;
            let (major, minor, patch) = version.tag_name.split('.').map(u64::from_str).chain(iter::repeat(Ok(0))).next_tuple().expect("iter::repeat produces an infinite iterator");
            (major?, minor?, patch?)
        }
    };
    write!(&bizhawk_version, "Version::new({major}, {minor}, {patch})")?;
    Ok(())
}
