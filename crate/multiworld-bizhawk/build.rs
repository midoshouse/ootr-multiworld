#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, warnings)]
#![forbid(unsafe_code)]

use {
    std::{
        collections::HashSet,
        env,
        path::Path,
    },
    lazy_regex::regex_captures,
    tokio::{
        io,
        process::Command,
    },
    wheel::{
        fs,
        traits::AsyncCommandOutputExt as _,
    },
};

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] Io(#[from] io::Error),
    #[error(transparent)] Wheel(#[from] wheel::Error),
    #[error("function `{0}` declared as extern in C# is not defined in Rust")]
    UndefinedCsharpFunction(String),
}

#[wheel::main(debug)]
async fn main() -> Result<(), Error> {
    println!("cargo:rerun-if-changed=nonexistent.foo"); // check a nonexistent file to make sure build script is always run (see https://github.com/rust-lang/cargo/issues/4213 and https://github.com/rust-lang/cargo/issues/5663)
    // check for extern function definitions in C# which aren't defined in Rust
    let rust_file = fs::read_to_string("../multiworld-csharp/src/lib.rs").await?;
    let mut rust_functions = HashSet::new();
    for line in rust_file.lines() {
        if let Some((_, name)) = regex_captures!("^#\\[csharp_ffi\\] pub (?:unsafe )?extern \"C\" fn ([0-9a-z_]+)\\(", line) {
            rust_functions.insert(name);
        }
    }
    let csharp_file = fs::read_to_string("OotrMultiworld/src/MainForm.cs").await?;
    for line in csharp_file.lines() {
        if let Some((_, name)) = regex_captures!("^        \\[DllImport\\(\"multiworld\"\\)\\] internal static extern [0-9A-Za-z_]+ ([0-9a-z_]+)\\(.*\\);$", line) {
            if !rust_functions.contains(name) {
                return Err(Error::UndefinedCsharpFunction(name.to_owned()))
            }
        }
    }
    // build C# code
    let is_release = match &*env::var("PROFILE").expect("missing PROFILE envar") {
        "debug" => false,
        "release" => true,
        profile => panic!("unexpected PROFILE envar: {profile:?}"),
    };
    let source_path = match (env::var_os("CARGO_CFG_WINDOWS").is_some(), is_release) {
        (false, false) => Path::new("../../target/debug/liblinuxtest.so"),
        (false, true) => Path::new("../../target/release/liblinuxtest.so"),
        (true, false) => Path::new("../../target/debug/multiworld.dll"),
        (true, true) => Path::new("../../target/release/multiworld.dll"),
    }.canonicalize()?;
    let target_paths = if env::var_os("CARGO_CFG_WINDOWS").is_some() {
        [Path::new("OotrMultiworld/src/multiworld.dll"), Path::new("OotrMultiworld/BizHawk/ExternalTools/multiworld.dll")]
    } else {
        [Path::new("LinuxTest/src/liblinuxtest.so"), Path::new("LinuxTest/BizHawk/dll/liblinuxtest.so")]
    };
    for target_path in target_paths {
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        match tokio::fs::symlink_metadata(target_path).await { //TODO wheel
            Ok(metadata) if metadata.is_symlink() || metadata.is_file() => fs::remove_file(target_path).await?,
            Ok(metadata) => panic!("unexpected file type: {metadata:?}"),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        fs::copy(&source_path, &target_path).await?;
    }
    let mut dotnet_command = Command::new("dotnet");
    dotnet_command.arg("build");
    if is_release {
        dotnet_command.arg("--configuration=Release");
    }
    dotnet_command.current_dir("OotrMultiworld/src");
    dotnet_command.spawn()?.check("dotnet").await?;
    Ok(())
}
