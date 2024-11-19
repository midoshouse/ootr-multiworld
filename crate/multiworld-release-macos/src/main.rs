use {
    std::env,
    async_proto::Protocol as _,
    dir_lock::DirLock,
    directories::UserDirs,
    git2::{
        BranchType,
        Repository,
        ResetType,
    },
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
    #[error(transparent)] Git(#[from] git2::Error),
    #[error(transparent)] Io(#[from] io::Error),
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

    progress!("updating ootr-multiworld repo");
    let repo = Repository::open("/opt/git/github.com/midoshouse/ootr-multiworld/main")?;
    let mut origin = repo.find_remote("origin")?;
    origin.fetch(&["main"], None, None)?;
    repo.reset(&repo.find_branch("origin/main", BranchType::Remote)?.into_reference().peel_to_commit()?.into_object(), ResetType::Hard, None)?;

    progress!("building Mido's House Multiworld.app for x86_64");
    Command::new("cargo").arg("build").arg("--release").arg("--target=x86_64-apple-darwin").arg("--package=multiworld-gui").env("MACOSX_DEPLOYMENT_TARGET", "10.9").current_dir("/opt/git/github.com/midoshouse/ootr-multiworld/main").check("cargo").await?;

    progress!("building Mido's House Multiworld.app for aarch64");
    Command::new("cargo").arg("build").arg("--release").arg("--target=aarch64-apple-darwin").arg("--package=multiworld-gui").current_dir("/opt/git/github.com/midoshouse/ootr-multiworld/main").check("cargo").await?;

    progress!("creating Universal macOS binary");
    fs::create_dir("/opt/git/github.com/midoshouse/ootr-multiworld/main/assets/macos/Mido's House Multiworld.app/Contents/MacOS").await.exist_ok()?;
    Command::new("lipo").arg("-create").arg("target/aarch64-apple-darwin/release/multiworld-gui").arg("target/x86_64-apple-darwin/release/multiworld-gui").arg("-output").arg("assets/macos/Mido's House Multiworld.app/Contents/MacOS/multiworld-gui").current_dir("/opt/git/github.com/midoshouse/ootr-multiworld/main").check("lipo").await?;

    progress!("packing multiworld-gui.dmg");
    Command::new("hdiutil").arg("create").arg("assets/multiworld-gui.dmg").arg("-volname").arg("Mido's House Multiworld").arg("-srcfolder").arg("assets/macos").arg("-ov").current_dir("/opt/git/github.com/midoshouse/ootr-multiworld/main").check("hdiutil").await?;

    Ok(())
}
