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
    wheel::traits::AsyncCommandOutputExt as _,
    multiworld::MacReleaseMessage,
};

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
async fn main() -> Result<(), Error> {
    let mut stdout = stdout();

    macro_rules! progress {
        ($label:literal) => {{
            MacReleaseMessage::Progress { label: format!($label) }.write(&mut stdout).await?;
            stdout.flush().await?;
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

    Ok(())
}
