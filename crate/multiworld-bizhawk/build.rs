use {
    std::{
        collections::HashSet,
        env,
        fs,
        io,
        path::Path,
        process::Command,
    },
    lazy_regex::regex_captures,
};

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] Io(#[from] io::Error),
    #[error("function `{0}` declared as extern in C# is not defined in Rust")]
    UndefinedCsharpFunction(String),
}

fn main() -> Result<(), Error> {
    println!("cargo:rerun-if-changed=nonexistent.foo"); // check a nonexistent file to make sure build script is always run (see https://github.com/rust-lang/cargo/issues/4213 and https://github.com/rust-lang/cargo/issues/5663)
    // check for extern function definitions in C# which aren't defined in Rust
    let rust_file = fs::read_to_string("../multiworld-csharp/src/lib.rs")?;
    let mut rust_functions = HashSet::new();
    for line in rust_file.lines() {
        if let Some((_, name)) = regex_captures!("^#\\[csharp_ffi\\] pub (?:unsafe )?extern \"C\" fn ([0-9a-z_]+)\\(", line) {
            rust_functions.insert(name);
        }
    }
    let csharp_file = fs::read_to_string("OotrMultiworld/src/MainForm.cs")?;
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
        profile => panic!("unexpected PROFILE envar: {:?}", profile),
    };
    let source_path = if is_release {
        Path::new("../../target/release/multiworld.dll")
    } else {
        Path::new("../../target/debug/multiworld.dll")
    }.canonicalize()?;
    for target_path in &[Path::new("OotrMultiworld/src/multiworld.dll"), Path::new("OotrMultiworld/BizHawk/ExternalTools/multiworld.dll")] {
        match target_path.symlink_metadata() {
            Ok(metadata) if metadata.is_symlink() || metadata.is_file() => std::fs::remove_file(target_path)?,
            Ok(metadata) => panic!("unexpected file type: {metadata:?}"),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        std::fs::copy(&source_path, &target_path)?;
    }
    let mut dotnet_command = Command::new("dotnet");
    dotnet_command.arg("build");
    if is_release {
        dotnet_command.arg("--configuration=Release");
    }
    dotnet_command.current_dir("OotrMultiworld/src");
    assert!(dotnet_command.status()?.success());
    Ok(())
}
