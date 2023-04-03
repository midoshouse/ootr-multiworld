use std::{
    env,
    io,
    path::Path,
    process::Command,
};

fn main() -> io::Result<()> {
    println!("cargo:rerun-if-changed=nonexistent.foo"); // check a nonexistent file to make sure build script is always run (see https://github.com/rust-lang/cargo/issues/4213 and https://github.com/rust-lang/cargo/issues/5663)
    //TODO check for extern function definitions in C# which aren't defined in Rust (move from check-ffi.py)
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
            Err(e) => return Err(e),
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
