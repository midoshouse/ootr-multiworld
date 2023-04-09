#![deny(rust_2018_idioms, unused, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]
#![forbid(unsafe_code)]

use {
    std::{
        cmp::Ordering::*,
        env,
        ffi::OsString,
        fmt,
        io::{
            Cursor,
            prelude::*,
        },
        path::Path,
        sync::Arc,
        time::Duration,
    },
    async_trait::async_trait,
    dir_lock::DirLock,
    futures::future::{
        self,
        FutureExt as _,
    },
    gres::{
        Percent,
        Progress,
        Task,
        cli::Cli,
    },
    semver::Version,
    tempfile::NamedTempFile,
    tokio::{
        process::Command,
        sync::broadcast,
    },
    wheel::{
        fs,
        traits::AsyncCommandOutputExt as _,
    },
    zip::{
        ZipWriter,
        write::FileOptions,
    },
    multiworld::github::{
        Release,
        Repo,
    },
    multiworld_utils::version,
};

enum Setup {
    CreateReqwestClient,
    CheckVersion(reqwest::Client),
    CheckBizHawkVersion(reqwest::Client, Repo),
    LockRust(reqwest::Client, Repo, Version),
    UpdateRust(reqwest::Client, Repo, Version, DirLock),
}

impl Default for Setup {
    fn default() -> Self {
        Self::CreateReqwestClient
    }
}

impl fmt::Display for Setup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CreateReqwestClient => write!(f, "creating reqwest client"),
            Self::CheckVersion(..) => write!(f, "checking version"),
            Self::CheckBizHawkVersion(..) => write!(f, "checking BizHawk version"),
            Self::LockRust(..) => write!(f, "waiting for Rust lock"),
            Self::UpdateRust(..) => write!(f, "updating Rust"),
        }
    }
}

impl Progress for Setup {
    fn progress(&self) -> Percent {
        Percent::fraction(match self {
            Self::CreateReqwestClient => 0,
            Self::CheckVersion(..) => 1,
            Self::CheckBizHawkVersion(..) => 2,
            Self::LockRust(..) => 3,
            Self::UpdateRust(..) => 4,
        }, 5)
    }
}

#[async_trait]
impl Task<Result<(reqwest::Client, Repo, Version), Error>> for Setup {
    async fn run(self) -> Result<Result<(reqwest::Client, Repo, Version), Error>, Self> {
        match self {
            Self::CreateReqwestClient => gres::transpose(async move {
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert(reqwest::header::AUTHORIZATION, reqwest::header::HeaderValue::from_str(&format!("token {}", fs::read_to_string("assets/release-token").await?))?);
                let client = reqwest::Client::builder()
                    .user_agent(concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")))
                    .default_headers(headers)
                    .timeout(Duration::from_secs(600))
                    .http2_prior_knowledge()
                    .use_rustls_tls()
                    .https_only(true)
                    .build()?;
                Ok(Err(Self::CheckVersion(client)))
            }).await,
            Self::CheckVersion(client) => gres::transpose(async move {
                //TODO make sure working dir is clean and on default branch and up to date with remote and remote is up to date
                let repo = Repo::new("midoshouse", "ootr-multiworld");
                if let Some(latest_release) = repo.latest_release(&client).await? {
                    let remote_version = latest_release.version()?;
                    match version::version().await.cmp(&remote_version) {
                        Less => return Err(Error::VersionRegression),
                        Equal => return Err(Error::SameVersion),
                        Greater => {}
                    }
                }
                Ok(Err(Self::CheckBizHawkVersion(client, repo)))
            }).await,
            Self::CheckBizHawkVersion(client, repo) => gres::transpose(async move {
                let [major, minor, patch, _] = multiworld_bizhawk::bizhawk_version();
                let local_version = Version::new(major.into(), minor.into(), patch.into());
                let remote_version = version::bizhawk_latest(&client).await?;
                match local_version.cmp(&remote_version) {
                    Less => return Err(Error::BizHawkOutdated { local: local_version, latest: remote_version }),
                    Equal => {}
                    Greater => return Err(Error::BizHawkVersionRegression),
                }
                Ok(Err(Self::LockRust(client, repo, local_version)))
            }).await,
            Self::LockRust(client, repo, local_version) => gres::transpose(async move {
                let lock_dir = Path::new(&env::var_os("TEMP").ok_or(Error::MissingEnvar("TEMP"))?).join("syncbin-startup-rust.lock");
                let lock = DirLock::new(&lock_dir).await?;
                Ok(Err(Self::UpdateRust(client, repo, local_version, lock))) //TODO update rustup first?
            }).await,
            Self::UpdateRust(client, repo, local_version, lock) => gres::transpose(async move {
                Command::new("rustup").arg("update").arg("stable").check("rustup").await?;
                lock.drop_async().await?;
                Ok(Ok((client, repo, local_version)))
            }).await,
        }
    }
}

enum CreateRelease {
    CreateNotesFile(Repo, reqwest::Client, broadcast::Sender<Release>, Arc<Cli>, Args),
    EditNotes(Repo, reqwest::Client, broadcast::Sender<Release>, Arc<Cli>, Args, NamedTempFile),
    ReadNotes(Repo, reqwest::Client, broadcast::Sender<Release>, NamedTempFile),
    Create(Repo, reqwest::Client, broadcast::Sender<Release>, String),
}

impl CreateRelease {
    fn new(repo: Repo, client: reqwest::Client, tx: broadcast::Sender<Release>, cli: Arc<Cli>, args: Args) -> Self {
        Self::CreateNotesFile(repo, client, tx, cli, args)
    }
}

impl fmt::Display for CreateRelease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CreateNotesFile(..) => write!(f, "creating release notes file"),
            Self::EditNotes(..) => write!(f, "waiting for release notes"),
            Self::ReadNotes(..) => write!(f, "reading release notes"),
            Self::Create(..) => write!(f, "creating release"),
        }
    }
}

impl Progress for CreateRelease {
    fn progress(&self) -> Percent {
        Percent::fraction(match self {
            Self::CreateNotesFile(..) => 0,
            Self::EditNotes(..) => 1,
            Self::ReadNotes(..) => 2,
            Self::Create(..) => 3,
        }, 4)
    }
}

#[async_trait]
impl Task<Result<Release, Error>> for CreateRelease {
    async fn run(self) -> Result<Result<Release, Error>, Self> {
        match self {
            Self::CreateNotesFile(repo, client, tx, cli, args) => gres::transpose(async move {
                let notes_file = tokio::task::spawn_blocking(|| {
                    tempfile::Builder::default()
                        .prefix("ootrmw-release-notes")
                        .suffix(".md")
                        .tempfile()
                }).await??;
                Ok(Err(Self::EditNotes(repo, client, tx, cli, args, notes_file)))
            }).await,
            Self::EditNotes(repo, client, tx, cli, args, notes_file) => gres::transpose(async move {
                let mut cmd;
                let (cmd_name, cli_lock) = if let Some(ref editor) = args.release_notes_editor {
                    cmd = Command::new(editor);
                    if !args.no_wait {
                        cmd.arg("--wait");
                    }
                    ("editor", Some(cli.lock().await))
                } else {
                    if env::var("TERM_PROGRAM").as_deref() == Ok("vscode") && env::var_os("STY").is_none() && env::var_os("SSH_CLIENT").is_none() && env::var_os("SSH_TTY").is_none() {
                        cmd = Command::new("C:\\Program Files\\Microsoft VS Code\\bin\\code.cmd");
                        if !args.no_wait {
                            cmd.arg("--wait");
                        }
                        ("code", None)
                    } else {
                        cmd = Command::new("C:\\ProgramData\\chocolatey\\bin\\nano.exe");
                        ("nano", Some(cli.lock().await))
                    }
                };
                cmd.arg(notes_file.path()).spawn()?.check(cmd_name).await?; // spawn before checking to avoid capturing stdio
                drop(cli_lock);
                Ok(Err(Self::ReadNotes(repo, client, tx, notes_file)))
            }).await,
            Self::ReadNotes(repo, client, tx, mut notes_file) => gres::transpose(async move {
                let notes = tokio::task::spawn_blocking(move || {
                    let mut buf = String::default();
                    notes_file.read_to_string(&mut buf)?;
                    if buf.is_empty() { return Err(Error::EmptyReleaseNotes) }
                    Ok(buf)
                }).await??;
                Ok(Err(Self::Create(repo, client, tx, notes)))
            }).await,
            Self::Create(repo, client, tx, notes) => gres::transpose(async move {
                let release = repo.create_release(&client, format!("Mido's House Multiworld {}", version::version().await), format!("v{}", version::version().await), notes).await?;
                let _ = tx.send(release.clone());
                Ok(Ok(release))
            }).await,
        }
    }
}

enum BuildUpdater {
    Default(broadcast::Sender<()>),
    Glow(broadcast::Sender<()>),
}

impl BuildUpdater {
    fn new(notifier: broadcast::Sender<()>) -> Self {
        Self::Default(notifier)
    }
}

impl fmt::Display for BuildUpdater {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default(..) => write!(f, "building multiworld-updater.exe"),
            Self::Glow(..) => write!(f, "building multiworld-updater-glow.exe"),
        }
    }
}

impl Progress for BuildUpdater {
    fn progress(&self) -> Percent {
        Percent::fraction(match self {
            Self::Default(..) => 0,
            Self::Glow(..) => 1,
        }, 2)
    }
}

#[async_trait]
impl Task<Result<(), Error>> for BuildUpdater {
    async fn run(self) -> Result<Result<(), Error>, Self> {
        match self {
            Self::Default(notifier) => gres::transpose(async move {
                Command::new("cargo").arg("build").arg("--release").arg("--package=multiworld-updater").check("cargo build --package=multiworld-updater").await?;
                Ok(Err(Self::Glow(notifier)))
            }).await,
            Self::Glow(notifier) => gres::transpose(async move {
                Command::new("cargo").arg("build").arg("--no-default-features").arg("--features=glow").arg("--target-dir=target/glow").arg("--release").arg("--package=multiworld-updater").check("cargo build --package=multiworld-updater").await?;
                let _ = notifier.send(());
                Ok(Ok(()))
            }).await,
        }
    }
}

enum BuildGui {
    Updater(reqwest::Client, Repo, broadcast::Receiver<()>, broadcast::Receiver<Release>, broadcast::Sender<()>),
    Build(reqwest::Client, Repo, broadcast::Receiver<Release>, broadcast::Sender<()>),
    Glow(reqwest::Client, Repo, broadcast::Receiver<Release>, broadcast::Sender<()>),
    Read(reqwest::Client, Repo, broadcast::Receiver<Release>),
    WaitRelease(reqwest::Client, Repo, broadcast::Receiver<Release>, Vec<u8>),
    Upload(reqwest::Client, Repo, Release, Vec<u8>),
}

impl BuildGui {
    fn new(client: reqwest::Client, repo: Repo, updater_rx: broadcast::Receiver<()>, release_rx: broadcast::Receiver<Release>, gui_tx: broadcast::Sender<()>) -> Self {
        Self::Updater(client, repo, updater_rx, release_rx, gui_tx)
    }
}

impl fmt::Display for BuildGui {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Updater(..) => write!(f, "waiting for updater build to finish"),
            Self::Build(..) => write!(f, "building multiworld-gui.exe"),
            Self::Glow(..) => write!(f, "building multiworld-gui-glow.exe"),
            Self::Read(..) => write!(f, "reading multiworld-gui.exe"),
            Self::WaitRelease(..) => write!(f, "waiting for GitHub release to be created"),
            Self::Upload(..) => write!(f, "uploading multiworld-pj64.exe"),
        }
    }
}

impl Progress for BuildGui {
    fn progress(&self) -> Percent {
        Percent::fraction(match self {
            Self::Updater(..) => 0,
            Self::Build(..) => 1,
            Self::Glow(..) => 2,
            Self::Read(..) => 3,
            Self::WaitRelease(..) => 4,
            Self::Upload(..) => 5,
        }, 6)
    }
}

#[async_trait]
impl Task<Result<(), Error>> for BuildGui {
    async fn run(self) -> Result<Result<(), Error>, Self> {
        match self {
            Self::Updater(client, repo, mut updater_rx, release_rx, gui_tx) => gres::transpose(async move {
                let () = updater_rx.recv().await?;
                Ok(Err(Self::Build(client, repo, release_rx, gui_tx)))
            }).await,
            Self::Build(client, repo, release_rx, gui_tx) => gres::transpose(async move {
                Command::new("cargo").arg("build").arg("--release").arg("--package=multiworld-gui").check("cargo build --package=multiworld-gui").await?;
                Ok(Err(Self::Glow(client, repo, release_rx, gui_tx)))
            }).await,
            Self::Glow(client, repo, release_rx, gui_tx) => gres::transpose(async move {
                Command::new("cargo").arg("build").arg("--no-default-features").arg("--features=glow").arg("--target-dir=target/glow").arg("--release").arg("--package=multiworld-gui").check("cargo build --package=multiworld-gui").await?;
                let _ = gui_tx.send(());
                Ok(Err(Self::Read(client, repo, release_rx)))
            }).await,
            Self::Read(client, repo, release_rx) => gres::transpose(async move {
                let data = fs::read("target/release/multiworld-gui.exe").await?;
                Ok(Err(Self::WaitRelease(client, repo, release_rx, data)))
            }).await,
            Self::WaitRelease(client, repo, mut release_rx, data) => gres::transpose(async move {
                let release = release_rx.recv().await?;
                Ok(Err(Self::Upload(client, repo, release, data)))
            }).await,
            Self::Upload(client, repo, release, data) => gres::transpose(async move {
                repo.release_attach(&client, &release, "multiworld-pj64.exe", "application/vnd.microsoft.portable-executable", data).await?;
                Ok(Ok(()))
            }).await,
        }
    }
}

enum BuildBizHawk {
    Gui(reqwest::Client, Repo, broadcast::Receiver<()>, broadcast::Receiver<Release>, Version, broadcast::Sender<()>),
    CSharp(reqwest::Client, Repo, broadcast::Receiver<Release>, Version, broadcast::Sender<()>),
    BizHawk(reqwest::Client, Repo, broadcast::Receiver<Release>, Version, broadcast::Sender<()>),
    Zip(reqwest::Client, Repo, broadcast::Receiver<Release>, Version),
    WaitRelease(reqwest::Client, Repo, broadcast::Receiver<Release>, Vec<u8>),
    Upload(reqwest::Client, Repo, Release, Vec<u8>),
}

impl BuildBizHawk {
    fn new(client: reqwest::Client, repo: Repo, gui_rx: broadcast::Receiver<()>, release_rx: broadcast::Receiver<Release>, version: Version, bizhawk_tx: broadcast::Sender<()>) -> Self {
        Self::Gui(client, repo, gui_rx, release_rx, version, bizhawk_tx)
    }
}

impl fmt::Display for BuildBizHawk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gui(..) => write!(f, "waiting for GUI build to finish"),
            Self::CSharp(..) => write!(f, "building multiworld-csharp"),
            Self::BizHawk(..) => write!(f, "building multiworld-bizhawk"),
            Self::Zip(..) => write!(f, "creating multiworld-bizhawk.zip"),
            Self::WaitRelease(..) => write!(f, "waiting for GitHub release to be created"),
            Self::Upload(..) => write!(f, "uploading multiworld-bizhawk.zip"),
        }
    }
}

impl Progress for BuildBizHawk {
    fn progress(&self) -> Percent {
        Percent::fraction(match self {
            Self::Gui(..) => 0,
            Self::CSharp(..) => 1,
            Self::BizHawk(..) => 2,
            Self::Zip(..) => 3,
            Self::WaitRelease(..) => 4,
            Self::Upload(..) => 5,
        }, 6)
    }
}

#[async_trait]
impl Task<Result<(), Error>> for BuildBizHawk {
    async fn run(self) -> Result<Result<(), Error>, Self> {
        match self {
            Self::Gui(client, repo, mut gui_rx, release_rx, version, bizhawk_tx) => gres::transpose(async move {
                let () = gui_rx.recv().await?;
                Ok(Err(Self::CSharp(client, repo, release_rx, version, bizhawk_tx)))
            }).await,
            Self::CSharp(client, repo, release_rx, version, bizhawk_tx) => gres::transpose(async move {
                Command::new("cargo").arg("build").arg("--release").arg("--package=multiworld-csharp").check("cargo build --package=multiworld-csharp").await?;
                Ok(Err(Self::BizHawk(client, repo, release_rx, version, bizhawk_tx)))
            }).await,
            Self::BizHawk(client, repo, release_rx, version, bizhawk_tx) => gres::transpose(async move {
                Command::new("cargo").arg("build").arg("--release").arg("--package=multiworld-bizhawk").check("cargo build --package=multiworld-bizhawk").await?;
                let _ = bizhawk_tx.send(());
                Ok(Err(Self::Zip(client, repo, release_rx, version)))
            }).await,
            Self::Zip(client, repo, release_rx, version) => gres::transpose(async move {
                let zip_data = tokio::task::spawn_blocking(move || {
                    let mut buf = Cursor::<Vec<_>>::default();
                    {
                        let mut zip = ZipWriter::new(&mut buf); //TODO replace with an async zip writer
                        zip.start_file("README.txt", FileOptions::default())?;
                        write!(&mut zip, include_str!("../../../assets/bizhawk-readme.txt"), version)?;
                        zip.start_file("OotrMultiworld.dll", FileOptions::default())?;
                        std::io::copy(&mut std::fs::File::open("crate/multiworld-bizhawk/OotrMultiworld/BizHawk/ExternalTools/OotrMultiworld.dll")?, &mut zip)?;
                        zip.start_file("multiworld.dll", FileOptions::default())?;
                        std::io::copy(&mut std::fs::File::open("crate/multiworld-bizhawk/OotrMultiworld/BizHawk/ExternalTools/multiworld.dll")?, &mut zip)?;
                    }
                    Ok::<_, Error>(buf.into_inner())
                }).await??;
                Ok(Err(Self::WaitRelease(client, repo, release_rx, zip_data)))
            }).await,
            Self::WaitRelease(client, repo, mut release_rx, zip_data) => gres::transpose(async move {
                let release = release_rx.recv().await?;
                Ok(Err(Self::Upload(client, repo, release, zip_data)))
            }).await,
            Self::Upload(client, repo, release, zip_data) => gres::transpose(async move {
                repo.release_attach(&client, &release, "multiworld-bizhawk.zip", "application/zip", zip_data).await?;
                Ok(Ok(()))
            }).await,
        }
    }
}

enum BuildPj64 {
    WaitRelease(reqwest::Client, Repo, broadcast::Receiver<Release>),
    ReadJs(reqwest::Client, Repo, Release),
    UploadJs(reqwest::Client, Repo, Release, Vec<u8>),
}

impl BuildPj64 {
    fn new(client: reqwest::Client, repo: Repo, release_rx: broadcast::Receiver<Release>) -> Self {
        Self::WaitRelease(client, repo, release_rx)
    }
}

impl fmt::Display for BuildPj64 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WaitRelease(..) => write!(f, "waiting for GitHub release to be created"),
            Self::ReadJs(..) => write!(f, "reading ootrmw-pj64.js"),
            Self::UploadJs(..) => write!(f, "uploading ootrmw-pj64.js"),
        }
    }
}

impl Progress for BuildPj64 {
    fn progress(&self) -> Percent {
        Percent::fraction(match self {
            Self::WaitRelease(..) => 0,
            Self::ReadJs(..) => 1,
            Self::UploadJs(..) => 2,
        }, 3)
    }
}

#[async_trait]
impl Task<Result<(), Error>> for BuildPj64 {
    async fn run(self) -> Result<Result<(), Error>, Self> {
        match self {
            Self::WaitRelease(client, repo, mut release_rx) => gres::transpose(async move {
                let release = release_rx.recv().await?;
                Ok(Err(Self::ReadJs(client, repo, release)))
            }).await,
            Self::ReadJs(client, repo, release) => gres::transpose(async move {
                let data = fs::read("assets/ootrmw-pj64.js").await?;
                Ok(Err(Self::UploadJs(client, repo, release, data)))
            }).await,
            Self::UploadJs(client, repo, release, data) => gres::transpose(async move {
                repo.release_attach(&client, &release, "ootrmw-pj64.js", "text/javascript", data).await?;
                Ok(Ok(()))
            }).await,
        }
    }
}

enum BuildInstaller {
    Deps(reqwest::Client, Repo, broadcast::Receiver<()>, broadcast::Receiver<()>, broadcast::Receiver<Release>),
    Build(reqwest::Client, Repo, broadcast::Receiver<Release>),
    Glow(reqwest::Client, Repo, broadcast::Receiver<Release>),
    Read(reqwest::Client, Repo, broadcast::Receiver<Release>),
    WaitRelease(reqwest::Client, Repo, broadcast::Receiver<Release>, Vec<u8>),
    Upload(reqwest::Client, Repo, Release, Vec<u8>),
}

impl BuildInstaller {
    fn new(client: reqwest::Client, repo: Repo, bizhawk_rx: broadcast::Receiver<()>, gui_rx: broadcast::Receiver<()>, release_rx: broadcast::Receiver<Release>) -> Self {
        Self::Deps(client, repo, bizhawk_rx, gui_rx, release_rx)
    }
}

impl fmt::Display for BuildInstaller {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Deps(..) => write!(f, "waiting for dependency builds to finish"),
            Self::Build(..) => write!(f, "building multiworld-installer.exe"),
            Self::Glow(..) => write!(f, "building multiworld-installer-glow.exe"),
            Self::Read(..) => write!(f, "reading multiworld-installer.exe"),
            Self::WaitRelease(..) => write!(f, "waiting for GitHub release to be created"),
            Self::Upload(..) => write!(f, "uploading multiworld-installer.exe"),
        }
    }
}

impl Progress for BuildInstaller {
    fn progress(&self) -> Percent {
        Percent::fraction(match self {
            Self::Deps(..) => 0,
            Self::Build(..) => 1,
            Self::Glow(..) => 2,
            Self::Read(..) => 3,
            Self::WaitRelease(..) => 4,
            Self::Upload(..) => 5,
        }, 6)
    }
}

#[async_trait]
impl Task<Result<(), Error>> for BuildInstaller {
    async fn run(self) -> Result<Result<(), Error>, Self> {
        match self {
            Self::Deps(client, repo, mut bizhawk_rx, mut gui_rx, release_rx) => gres::transpose(async move {
                let () = bizhawk_rx.recv().await?;
                let () = gui_rx.recv().await?;
                Ok(Err(Self::Build(client, repo, release_rx)))
            }).await,
            Self::Build(client, repo, release_rx) => gres::transpose(async move {
                Command::new("cargo").arg("build").arg("--release").arg("--package=multiworld-installer").check("cargo build --package=multiworld-installer").await?;
                Ok(Err(Self::Glow(client, repo, release_rx)))
            }).await,
            Self::Glow(client, repo, release_rx) => gres::transpose(async move {
                Command::new("cargo").arg("build").arg("--no-default-features").arg("--features=glow").arg("--target-dir=target/glow").arg("--release").arg("--package=multiworld-installer").check("cargo build --package=multiworld-installer").await?;
                Ok(Err(Self::Read(client, repo, release_rx)))
            }).await,
            Self::Read(client, repo, release_rx) => gres::transpose(async move {
                let data = fs::read("target/release/multiworld-installer.exe").await?;
                Ok(Err(Self::WaitRelease(client, repo, release_rx, data)))
            }).await,
            Self::WaitRelease(client, repo, mut release_rx, data) => gres::transpose(async move {
                let release = release_rx.recv().await?;
                Ok(Err(Self::Upload(client, repo, release, data)))
            }).await,
            Self::Upload(client, repo, release, data) => gres::transpose(async move {
                repo.release_attach(&client, &release, "multiworld-installer.exe", "application/vnd.microsoft.portable-executable", data).await?;
                Ok(Ok(()))
            }).await,
        }
    }
}

#[derive(Default)]
enum BuildServer {
    #[default]
    UpdateRepo,
    Build,
    WaitRestart,
    Restart,
}

impl fmt::Display for BuildServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UpdateRepo => write!(f, "updating repo on Mido's House"),
            Self::Build => write!(f, "building ootrmwd"),
            Self::WaitRestart => write!(f, "waiting for rooms to be empty"),
            Self::Restart => write!(f, "restarting ootrmwd"),
        }
    }
}

impl Progress for BuildServer {
    fn progress(&self) -> Percent {
        Percent::new(match self {
            Self::UpdateRepo => 0,
            Self::Build => 10,
            Self::WaitRestart => 90,
            Self::Restart => 95,
        })
    }
}

#[async_trait]
impl Task<Result<(), Error>> for BuildServer {
    async fn run(self) -> Result<Result<(), Error>, Self> {
        match self {
            Self::UpdateRepo => gres::transpose(async move {
                Command::new("ssh").arg("midos.house").arg("cd /opt/git/github.com/midoshouse/ootr-multiworld/master && git pull --ff-only").check("ssh").await?;
                Ok(Err(Self::Build))
            }).await,
            Self::Build => gres::transpose(async move {
                Command::new("ssh").arg("midos.house").arg("cd /opt/git/github.com/midoshouse/ootr-multiworld/master && cargo build --release --package=ootrmwd").check("ssh").await?; //TODO build locally
                Ok(Err(Self::WaitRestart))
            }).await,
            Self::WaitRestart => gres::transpose(async move {
                Command::new("ssh").arg("midos.house").arg("if systemctl is-active ootrmw; then /opt/git/github.com/midoshouse/ootr-multiworld/master/target/release/ootrmwd wait-until-empty; fi").check("ssh").await?; //TODO continue normally if this fails because the server is stopped
                Ok(Err(Self::Restart))
            }).await,
            Self::Restart => gres::transpose(async move {
                Command::new("ssh").arg("midos.house").arg("sudo systemctl restart ootrmw").check("ssh").await?;
                Ok(Ok(()))
            }).await,
        }
    }
}

#[derive(Clone, clap::Parser)]
#[clap(version)]
struct Args {
    /// Create the GitHub release as a draft
    #[clap(short = 'P', long)]
    no_publish: bool,
    /// Don't update the server
    #[clap(short = 'S', long)]
    no_server: bool,
    /// Don't pass `--wait` to the release notes editor
    #[clap(short = 'W', long)]
    no_wait: bool,
    /// the editor for the release notes
    #[clap(short = 'e', long)]
    release_notes_editor: Option<OsString>,
    /// Only update the server
    #[clap(short, long, exclusive = true)]
    server_only: bool,
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] BizHawkVersionCheck(#[from] version::BizHawkError),
    #[error(transparent)] BroadcastRecv(#[from] broadcast::error::RecvError),
    #[error(transparent)] DirLock(#[from] dir_lock::Error),
    #[error(transparent)] InvalidHeaderValue(#[from] reqwest::header::InvalidHeaderValue),
    #[error(transparent)] Io(#[from] tokio::io::Error),
    #[error(transparent)] Reqwest(#[from] reqwest::Error),
    #[error(transparent)] SemVer(#[from] semver::Error),
    #[error(transparent)] Task(#[from] tokio::task::JoinError),
    #[error(transparent)] Wheel(#[from] wheel::Error),
    #[error(transparent)] Zip(#[from] zip::result::ZipError),
    #[error("BizHawk is outdated ({local} installed, {latest} available)")]
    BizHawkOutdated {
        latest: Version,
        local: Version,
    },
    #[error("locally installed BizHawk is newer than latest release")]
    BizHawkVersionRegression,
    #[error("aborting due to empty release notes")]
    EmptyReleaseNotes,
    #[error("missing environment variable: {0}")]
    MissingEnvar(&'static str),
    #[error("there is already a release with this version number")]
    SameVersion,
    #[error("the latest GitHub release has a newer version than the local crate version")]
    VersionRegression,
}

#[wheel::main(debug)]
async fn main(args: Args) -> Result<(), Error> {
    let cli = Arc::new(Cli::new()?);
    if args.server_only {
        cli.run(BuildServer::default(), "server build done").await??;
    } else {
        let create_release_cli = Arc::clone(&cli);
        let release_notes_cli = Arc::clone(&cli);
        let (client, repo, bizhawk_version) = cli.run(Setup::default(), "pre-release checks passed").await??; // don't show release notes editor if version check could still fail
        let (release_tx, release_rx_installer) = broadcast::channel(1);
        let release_rx_gui = release_tx.subscribe();
        let release_rx_bizhawk = release_tx.subscribe();
        let release_rx_pj64 = release_tx.subscribe();
        let create_release_args = args.clone();
        let create_release_client = client.clone();
        let create_release_repo = repo.clone();
        let create_release = tokio::spawn(async move {
            create_release_cli.run(CreateRelease::new(create_release_repo, create_release_client, release_tx, release_notes_cli, create_release_args), "release created").await?
        });
        let (updater_tx, updater_rx) = broadcast::channel(1);
        let (gui_tx, gui_rx) = broadcast::channel(1);
        let gui_rx_installer = gui_tx.subscribe();
        let (bizhawk_tx, bizhawk_rx) = broadcast::channel(1);

        macro_rules! with_metavariable {
            ($metavariable:tt, $($token:tt)*) => { $($token)* };
        }

        macro_rules! build_tasks {
            ($($task:expr,)*) => {
                let ($(with_metavariable!($task, ())),*) = tokio::try_join!($($task),*)?;
            };
        }

        build_tasks![
            { let cli = Arc::clone(&cli); async move { tokio::spawn(async move { cli.run(BuildUpdater::new(updater_tx), "updater build done").await? }).await? } },
            { let cli = Arc::clone(&cli); let client = client.clone(); let repo = repo.clone(); async move { tokio::spawn(async move { cli.run(BuildGui::new(client, repo, updater_rx, release_rx_gui, gui_tx), "GUI build done").await? }).await? } },
            { let cli = Arc::clone(&cli); let client = client.clone(); let repo = repo.clone(); async move { tokio::spawn(async move { cli.run(BuildBizHawk::new(client, repo, gui_rx, release_rx_bizhawk, bizhawk_version, bizhawk_tx), "BizHawk build done").await? }).await? } },
            { let cli = Arc::clone(&cli); let client = client.clone(); let repo = repo.clone(); async move { tokio::spawn(async move { cli.run(BuildPj64::new(client, repo, release_rx_pj64), "Project64 build done").await? }).await? } },
            { let cli = Arc::clone(&cli); let client = client.clone(); let repo = repo.clone(); async move { tokio::spawn(async move { cli.run(BuildInstaller::new(client, repo, bizhawk_rx, gui_rx_installer, release_rx_installer), "installer build done").await? }).await? } },
            if args.no_server { future::ok(()).boxed() } else { let cli = Arc::clone(&cli); async move { tokio::spawn(async move { cli.run(BuildServer::default(), "server build done").await? }).await? }.boxed() },
        ];
        let release = create_release.await??;
        if !args.no_publish {
            let line = cli.new_line("[....] publishing release").await?;
            repo.publish_release(&client, release).await?;
            line.replace("[done] release published").await?;
        }
    }
    Ok(())
}
