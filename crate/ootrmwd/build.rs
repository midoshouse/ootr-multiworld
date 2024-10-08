use {
    std::{
        env,
        fs::File,
        io::prelude::*,
        path::Path,
    },
    git2::Repository,
    semver::Version,
};

fn main() {
    println!("cargo::rerun-if-changed=nonexistent.foo"); // check a nonexistent file to make sure build script is always run (see https://github.com/rust-lang/cargo/issues/4213 and https://github.com/rust-lang/cargo/issues/5663)
    let mut f = File::create(Path::new(&env::var_os("OUT_DIR").unwrap()).join("version.rs")).unwrap();
    let version = env::var("CARGO_PKG_VERSION").unwrap().parse::<Version>().unwrap();
    assert!(version.pre.is_empty());
    assert!(version.build.is_empty());
    let commit_hash = Repository::open(Path::new(&env::var_os("CARGO_MANIFEST_DIR").unwrap()).parent().unwrap().parent().unwrap()).unwrap().head().unwrap().peel_to_commit().unwrap().id();
    writeln!(&mut f, "pub const CLAP_VERSION: &str = {:?};", format!("{version} ({commit_hash})")).unwrap();
}
