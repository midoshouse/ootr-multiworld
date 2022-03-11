use std::{
    env,
    io,
    path::Path,
    process::Command,
};

fn main() -> io::Result<()> {
    println!("cargo:rerun-if-changed=nonexistent.foo"); // check a nonexistent file to make sure build script is always run (see https://github.com/rust-lang/cargo/issues/4213 and https://github.com/rust-lang/cargo/issues/5663)
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
    for target_path in &[Path::new("OotrMultiworld/src/multiworld.dll")/*, Path::new("OotrMultiworld/BizHawk/ExternalTools/multiworld.dll")*/] {
        if target_path.read_link().is_ok() { std::fs::remove_file(target_path)? }
        std::os::windows::fs::symlink_file(&source_path, target_path)?;
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
