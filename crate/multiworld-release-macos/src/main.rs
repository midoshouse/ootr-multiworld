use {
    std::env,
    apple_bundle::prelude::*,
    async_proto::Protocol as _,
    chrono::prelude::*,
    dir_lock::DirLock,
    directories::UserDirs,
    serde::Deserialize,
    tokio::{
        io::{
            self,
            AsyncWriteExt as _,
            stdout,
        },
        process::Command,
    },
    wheel::{
        fs,
        traits::{
            AsyncCommandOutputExt as _,
            IoResultExt as _,
        },
    },
    multiworld::MacReleaseMessage,
};

#[derive(Deserialize)]
struct CargoManifest {
    workspace: CargoWorkspace,
}

#[derive(Deserialize)]
struct CargoWorkspace {
    package: CargoPackage,
}

#[derive(Deserialize)]
struct CargoPackage {
    version: semver::Version,
}

#[derive(clap::Parser)]
#[clap(version)]
struct Args {
    #[clap(short = 'H')]
    human_readable_output: bool,
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] DirLock(#[from] dir_lock::Error),
    #[error(transparent)] Env(#[from] env::VarError),
    #[error(transparent)] Io(#[from] io::Error),
    #[error(transparent)] Plist(#[from] plist::Error),
    #[error(transparent)] Toml(#[from] toml::de::Error),
    #[error(transparent)] Wheel(#[from] wheel::Error),
    #[error(transparent)] Write(#[from] async_proto::WriteError),
}

#[wheel::main]
async fn main(Args { human_readable_output }: Args) -> Result<(), Error> {
    let mut stdout = stdout();

    macro_rules! progress {
        ($label:literal) => {{
            let msg = MacReleaseMessage::Progress { label: format!($label) };
            if human_readable_output {
                println!("{msg:?}");
            } else {
                msg.write(&mut stdout).await?;
                stdout.flush().await?;
            }
        }};
    }

    progress!("acquiring rustup lock");
    let lock = DirLock::new("/tmp/syncbin-startup-rust.lock").await?;

    progress!("updating rustup");
    let mut rustup_cmd = Command::new("rustup");
    rustup_cmd.arg("self");
    rustup_cmd.arg("update");
    if let Some(user_dirs) = UserDirs::new() {
        rustup_cmd.env("PATH", format!("{}:{}", user_dirs.home_dir().join(".cargo").join("bin").display(), env::var("PATH")?));
    }
    rustup_cmd.check("rustup").await?;

    progress!("updating Rust");
    let mut rustup_cmd = Command::new("rustup");
    rustup_cmd.arg("update");
    rustup_cmd.arg("stable");
    if let Some(user_dirs) = UserDirs::new() {
        rustup_cmd.env("PATH", format!("{}:{}", user_dirs.home_dir().join(".cargo").join("bin").display(), env::var("PATH")?));
    }
    rustup_cmd.check("rustup").await?;
    lock.drop_async().await?;

    progress!("cleaning up outdated cargo build artifacts");
    let mut sweep_cmd = Command::new("cargo");
    sweep_cmd.arg("sweep");
    sweep_cmd.arg("--installed");
    sweep_cmd.arg("-r");
    sweep_cmd.current_dir("/opt/git");
    if let Some(user_dirs) = UserDirs::new() {
        sweep_cmd.env("PATH", format!("{}:{}", user_dirs.home_dir().join(".cargo").join("bin").display(), env::var("PATH")?));
    }
    sweep_cmd.check("cargo sweep").await?;

    progress!("building Mido's House Multiworld.app for x86_64");
    Command::new("cargo").arg("build").arg("--release").arg("--target=x86_64-apple-darwin").arg("--package=multiworld-gui").env("MACOSX_DEPLOYMENT_TARGET", "10.15" /* Rust supports 10.12+, Info.plist requires <key>NSRequiresAquaSystemAppearance</key><string>NO</string> below 10.14, this minimum is limited by Homebrew support */).current_dir("/opt/git/github.com/midoshouse/ootr-multiworld/main").check("cargo").await?;

    progress!("building Mido's House Multiworld.app for aarch64");
    Command::new("cargo").arg("build").arg("--release").arg("--target=aarch64-apple-darwin").arg("--package=multiworld-gui").current_dir("/opt/git/github.com/midoshouse/ootr-multiworld/main").check("cargo").await?;

    progress!("creating Universal macOS binary");
    fs::create_dir("/opt/git/github.com/midoshouse/ootr-multiworld/main/assets/macos/Mido's House Multiworld.app/Contents/MacOS").await.exist_ok()?;
    Command::new("lipo").arg("-create").arg("target/aarch64-apple-darwin/release/multiworld-gui").arg("target/x86_64-apple-darwin/release/multiworld-gui").arg("-output").arg("assets/macos/Mido's House Multiworld.app/Contents/MacOS/multiworld-gui").current_dir("/opt/git/github.com/midoshouse/ootr-multiworld/main").check("lipo").await?;

    progress!("creating Info.plist");
    let CargoManifest { workspace: CargoWorkspace { package: CargoPackage { version } } } = toml::from_slice(&fs::read("/opt/git/github.com/midoshouse/ootr-multiworld/main/Cargo.toml").await?)?;
    plist::to_file_binary("/opt/git/github.com/midoshouse/ootr-multiworld/main/assets/macos/Mido's House Multiworld.app/Contents/Info.plist", &InfoPlist {
        categorization: Categorization {
            bundle_package_type: Some(format!("APPL")),
            ..Categorization::default()
        },
        identification: Identification {
            bundle_identifier: format!("house.midos.mw"),
            ..Identification::default()
        },
        naming: Naming {
            bundle_name: Some(format!("Multiworld")),
            bundle_display_name: Some(format!("Mido's House Multiworld")),
            ..Naming::default()
        },
        bundle_version: BundleVersion {
            bundle_version: Some(version.to_string()),
            bundle_short_version_string: Some(version.to_string()),
            bundle_info_dictionary_version: Some(format!("6.0")),
            human_readable_copyright: Some(Utc::now().format("© 2021–%Y Fenhl and contributors").to_string()),
            ..BundleVersion::default()
        },
        localization: Localization {
            bundle_development_region: Some(format!("en")),
            ..Localization::default()
        },
        icons: Icons {
            bundle_icon_file: Some(format!("AppIcon.icns")),
            bundle_icon_name: Some(format!("mhmw-macos")),
            ..Icons::default()
        },
        graphics: Graphics {
            high_resolution_capable: Some(true),
            supports_automatic_graphics_switching: Some(true),
            ..Graphics::default()
        },
        launch: Launch {
            bundle_executable: Some(format!("multiworld-gui")),
            ..Launch::default()
        },
        ..InfoPlist::default()
    })?;

    progress!("packing multiworld-gui.dmg");
    Command::new("hdiutil").arg("create").arg("assets/multiworld-gui.dmg").arg("-volname").arg("Mido's House Multiworld").arg("-srcfolder").arg("assets/macos").arg("-ov").current_dir("/opt/git/github.com/midoshouse/ootr-multiworld/main").check("hdiutil").await?;

    Ok(())
}
