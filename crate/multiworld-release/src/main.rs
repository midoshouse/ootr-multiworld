use {
    std::{
        cmp::Ordering::*,
        collections::HashMap,
        env,
        fmt,
        io::{
            Cursor,
            prelude::*,
        },
        path::Path,
        pin::pin,
        process::{
            self,
            Stdio,
        },
        time::Duration,
    },
    async_proto::Protocol as _,
    async_trait::async_trait,
    chrono::prelude::*,
    dir_lock::DirLock,
    futures::{
        future::{
            self,
            FutureExt as _,
            TryFutureExt as _,
        },
        stream::TryStreamExt as _,
    },
    gres::{
        Percent,
        Progress,
        Task,
    },
    itertools::Itertools as _,
    lazy_regex::regex_captures,
    semver::Version,
    tempfile::NamedTempFile,
    tokio::{
        io::{
            AsyncBufReadExt as _,
            BufReader,
        },
        process::{
            Child,
            ChildStdout,
            Command,
        },
        sync::broadcast,
    },
    tokio_stream::wrappers::LinesStream,
    wheel::{
        fs::{
            self,
            File,
        },
        traits::AsyncCommandOutputExt as _,
    },
    zip::{
        ZipWriter,
        write::FileOptions,
    },
    multiworld::{
        WaitUntilInactiveMessage,
        frontend,
        github::{
            Release,
            Repo,
        },
    },
    crate::cli::{
        Cli,
        GetPriority,
        Priority,
    },
};

mod cli;
mod version;

#[derive(Clone)] struct WindowsUpdaterNotification;
#[derive(Clone)] struct LinuxGuiNotification;
#[derive(Clone)] struct WindowsGuiNotification;
#[derive(Clone)] struct LinuxBizHawkNotification;
#[derive(Clone)] struct WindowsBizHawkNotification;

enum Setup {
    CreateReqwestClient(bool),
    CheckVersion(bool, reqwest::Client),
    CheckBizHawkVersion(reqwest::Client, Repo),
    CheckPj64ProtocolVersion(reqwest::Client, Repo, Version),
    LockRust(reqwest::Client, Repo, Version),
    UpdateRust(reqwest::Client, Repo, Version, DirLock),
}

impl Setup {
    fn new(server_only: bool) -> Self {
        Self::CreateReqwestClient(server_only)
    }
}

impl fmt::Display for Setup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CreateReqwestClient(..) => write!(f, "creating reqwest client"),
            Self::CheckVersion(..) => write!(f, "checking version"),
            Self::CheckBizHawkVersion(..) => write!(f, "checking BizHawk version"),
            Self::CheckPj64ProtocolVersion(..) => write!(f, "checking Project64 protocol version"),
            Self::LockRust(..) => write!(f, "waiting for Rust lock"),
            Self::UpdateRust(..) => write!(f, "updating Rust"),
        }
    }
}

impl Progress for Setup {
    fn progress(&self) -> Percent {
        Percent::fraction(match self {
            Self::CreateReqwestClient(..) => 0,
            Self::CheckVersion(..) => 1,
            Self::CheckBizHawkVersion(..) => 2,
            Self::CheckPj64ProtocolVersion(..) => 3,
            Self::LockRust(..) => 4,
            Self::UpdateRust(..) => 5,
        }, 6)
    }
}

impl GetPriority for Setup {
    fn priority(&self) -> Priority {
        match self {
            Self::CreateReqwestClient(..) => Priority::Active,
            Self::CheckVersion(..) => Priority::Active,
            Self::CheckBizHawkVersion(..) => Priority::Active,
            Self::CheckPj64ProtocolVersion(..) => Priority::Active,
            Self::LockRust(..) => Priority::Waiting,
            Self::UpdateRust(..) => Priority::Active,
        }
    }
}

#[async_trait]
impl Task<Result<(reqwest::Client, Repo, Version), Error>> for Setup {
    async fn run(self) -> Result<Result<(reqwest::Client, Repo, Version), Error>, Self> {
        match self {
            Self::CreateReqwestClient(server_only) => gres::transpose(async move {
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
                Ok(Err(Self::CheckVersion(server_only, client)))
            }).await,
            Self::CheckVersion(server_only, client) => gres::transpose(async move {
                //TODO make sure working dir is clean and on default branch and up to date with remote and remote is up to date
                let repo = Repo::new("midoshouse", "ootr-multiworld");
                if let Some(latest_release) = repo.latest_release(&client).await? {
                    let local_version = version::version().await;
                    let remote_version = latest_release.version()?;
                    match local_version.cmp(&remote_version) {
                        Less => return Err(Error::VersionRegression),
                        Equal => if !server_only { return Err(Error::SameVersion) },
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
                Ok(Err(Self::CheckPj64ProtocolVersion(client, repo, local_version)))
            }).await,
            Self::CheckPj64ProtocolVersion(client, repo, local_version) => gres::transpose(async move {
                let frontend_version = pin!(LinesStream::new(BufReader::new(File::open("assets/ootrmw-pj64.js").await?).lines())
                    .err_into::<Error>()
                    .try_filter_map(|line| async move {
                        let Some((_, frontend_version)) = regex_captures!("^const MW_FRONTEND_PROTO_VERSION = ([0-9]+);$", &line) else { return Ok(None) };
                        Ok(Some(frontend_version.parse::<u8>()?))
                    }))
                    .try_next().await?.ok_or(Error::MissingPj64ProtocolVersion)?;
                if frontend_version != frontend::PROTOCOL_VERSION {
                    return Err(Error::WrongPj64ProtocolVersion(frontend_version))
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
    CreateNotesFile(Repo, reqwest::Client, broadcast::Sender<Release>, Args),
    EditNotes(Repo, reqwest::Client, broadcast::Sender<Release>, Args, NamedTempFile),
    ReadNotes(Repo, reqwest::Client, broadcast::Sender<Release>, NamedTempFile),
    Create(Repo, reqwest::Client, broadcast::Sender<Release>, String),
}

impl CreateRelease {
    fn new(repo: Repo, client: reqwest::Client, tx: broadcast::Sender<Release>, args: Args) -> Self {
        Self::CreateNotesFile(repo, client, tx, args)
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

impl GetPriority for CreateRelease {
    fn priority(&self) -> Priority {
        match self {
            Self::CreateNotesFile(..) => Priority::Active,
            Self::EditNotes(..) => Priority::UserInput,
            Self::ReadNotes(..) => Priority::Active,
            Self::Create(..) => Priority::Active,
        }
    }
}

#[async_trait]
impl Task<Result<Release, Error>> for CreateRelease {
    async fn run(self) -> Result<Result<Release, Error>, Self> {
        match self {
            Self::CreateNotesFile(repo, client, tx, args) => gres::transpose(async move {
                let notes_file = tokio::task::spawn_blocking(|| {
                    tempfile::Builder::default()
                        .prefix("ootrmw-release-notes")
                        .suffix(".md")
                        .tempfile()
                }).await??;
                Ok(Err(Self::EditNotes(repo, client, tx, args, notes_file)))
            }).await,
            Self::EditNotes(repo, client, tx, args, notes_file) => gres::transpose(async move {
                let mut cmd;
                let cmd_name = if env::var("TERM_PROGRAM").as_deref() == Ok("vscode") {
                    cmd = Command::new("code.cmd");
                    if !args.no_wait {
                        cmd.arg("--wait");
                    }
                    "code"
                } else if env::var_os("STY").is_none() && env::var_os("SSH_CLIENT").is_none() && env::var_os("SSH_TTY").is_none() {
                    cmd = Command::new("C:\\Program Files\\Microsoft VS Code\\bin\\code.cmd");
                    "code"
                } else {
                    unimplemented!("cannot edit release notes")
                };
                cmd.arg(notes_file.path()).spawn()?.check(cmd_name).await?; // spawn before checking to avoid capturing stdio
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
    Glow(bool, broadcast::Sender<WindowsUpdaterNotification>),
}

impl BuildUpdater {
    fn new(debug: bool, notifier: broadcast::Sender<WindowsUpdaterNotification>) -> Self {
        Self::Glow(debug, notifier)
    }
}

impl fmt::Display for BuildUpdater {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Glow(false, ..) => write!(f, "building multiworld-updater.exe"),
            Self::Glow(true, ..) => write!(f, "building multiworld-updater-debug.exe"),
        }
    }
}

impl Progress for BuildUpdater {
    fn progress(&self) -> Percent {
        Percent::fraction(match self {
            Self::Glow(..) => 0,
        }, 1)
    }
}

impl GetPriority for BuildUpdater {
    fn priority(&self) -> Priority {
        match self {
            Self::Glow(..) => Priority::Active,
        }
    }
}

#[async_trait]
impl Task<Result<(), Error>> for BuildUpdater {
    async fn run(self) -> Result<Result<(), Error>, Self> {
        match self {
            Self::Glow(debug, notifier) => gres::transpose(async move {
                let mut build = Command::new("cargo");
                build.arg("build");
                if !debug {
                    build.arg("--release");
                }
                build.arg("--package=multiworld-updater");
                build.check("cargo build --package=multiworld-updater").await?;
                let _ = notifier.send(WindowsUpdaterNotification);
                Ok(Ok(()))
            }).await,
        }
    }
}

enum BuildGui {
    Updater(bool, reqwest::Client, Repo, broadcast::Receiver<WindowsUpdaterNotification>, broadcast::Receiver<Release>, broadcast::Sender<WindowsGuiNotification>),
    Glow(bool, reqwest::Client, Repo, broadcast::Receiver<Release>, broadcast::Sender<WindowsGuiNotification>),
    Read(bool, reqwest::Client, Repo, broadcast::Receiver<Release>),
    WaitRelease(bool, reqwest::Client, Repo, broadcast::Receiver<Release>, Vec<u8>),
    Upload(bool, reqwest::Client, Repo, Release, Vec<u8>),
}

impl BuildGui {
    fn new(debug: bool, client: reqwest::Client, repo: Repo, updater_rx: broadcast::Receiver<WindowsUpdaterNotification>, release_rx: broadcast::Receiver<Release>, gui_tx: broadcast::Sender<WindowsGuiNotification>) -> Self {
        Self::Updater(debug, client, repo, updater_rx, release_rx, gui_tx)
    }
}

impl fmt::Display for BuildGui {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Updater(..) => write!(f, "waiting for updater build to finish"),
            Self::Glow(false, ..) => write!(f, "building multiworld-gui.exe"),
            Self::Glow(true, ..) => write!(f, "building multiworld-gui-debug.exe"),
            Self::Read(false, ..) => write!(f, "reading multiworld-gui.exe"),
            Self::Read(true, ..) => write!(f, "reading multiworld-gui-debug.exe"),
            Self::WaitRelease(..) => write!(f, "waiting for GitHub release to be created"),
            Self::Upload(false, ..) => write!(f, "uploading multiworld-pj64.exe"),
            Self::Upload(true, ..) => write!(f, "uploading multiworld-gui-debug.exe"),
        }
    }
}

impl Progress for BuildGui {
    fn progress(&self) -> Percent {
        Percent::fraction(match self {
            Self::Updater(..) => 0,
            Self::Glow(..) => 1,
            Self::Read(..) => 2,
            Self::WaitRelease(..) => 3,
            Self::Upload(..) => 4,
        }, 5)
    }
}

impl GetPriority for BuildGui {
    fn priority(&self) -> Priority {
        match self {
            Self::Updater(..) => Priority::Waiting,
            Self::Glow(..) => Priority::Active,
            Self::Read(..) => Priority::Active,
            Self::WaitRelease(..) => Priority::Waiting,
            Self::Upload(..) => Priority::Active,
        }
    }
}

#[async_trait]
impl Task<Result<(), Error>> for BuildGui {
    async fn run(self) -> Result<Result<(), Error>, Self> {
        match self {
            Self::Updater(debug, client, repo, mut updater_rx, release_rx, gui_tx) => gres::transpose(async move {
                let WindowsUpdaterNotification = updater_rx.recv().await?;
                Ok(Err(Self::Glow(debug, client, repo, release_rx, gui_tx)))
            }).await,
            Self::Glow(debug, client, repo, release_rx, gui_tx) => gres::transpose(async move {
                let mut build = Command::new("cargo");
                build.arg("build");
                if !debug {
                    build.arg("--release");
                }
                build.arg("--package=multiworld-gui");
                build.check("cargo build --package=multiworld-gui").await?;
                let _ = gui_tx.send(WindowsGuiNotification);
                Ok(Err(Self::Read(debug, client, repo, release_rx)))
            }).await,
            Self::Read(debug, client, repo, release_rx) => gres::transpose(async move {
                let data = fs::read(if debug {
                    "target/debug/multiworld-gui.exe"
                } else {
                    "target/release/multiworld-gui.exe"
                }).await?;
                Ok(Err(Self::WaitRelease(debug, client, repo, release_rx, data)))
            }).await,
            Self::WaitRelease(debug, client, repo, mut release_rx, data) => gres::transpose(async move {
                let release = release_rx.recv().await?;
                Ok(Err(Self::Upload(debug, client, repo, release, data)))
            }).await,
            Self::Upload(debug, client, repo, release, data) => gres::transpose(async move {
                repo.release_attach(&client, &release, if debug { "multiworld-gui-debug.exe" } else { "multiworld-pj64.exe" }, "application/vnd.microsoft.portable-executable", data).await?;
                Ok(Ok(()))
            }).await,
        }
    }
}

enum BuildGuiLinux {
    Sync(reqwest::Client, Repo, broadcast::Receiver<Release>, broadcast::Sender<LinuxGuiNotification>),
    Updater(reqwest::Client, Repo, broadcast::Receiver<Release>, broadcast::Sender<LinuxGuiNotification>),
    Gui(reqwest::Client, Repo, broadcast::Receiver<Release>, broadcast::Sender<LinuxGuiNotification>),
    Copy(reqwest::Client, Repo, broadcast::Receiver<Release>),
    Read(reqwest::Client, Repo, broadcast::Receiver<Release>),
    WaitRelease(reqwest::Client, Repo, broadcast::Receiver<Release>, Vec<u8>),
    Upload(reqwest::Client, Repo, Release, Vec<u8>),
}

impl BuildGuiLinux {
    fn new(client: reqwest::Client, repo: Repo, release_rx: broadcast::Receiver<Release>, gui_tx: broadcast::Sender<LinuxGuiNotification>) -> Self {
        Self::Sync(client, repo, release_rx, gui_tx)
    }
}

impl fmt::Display for BuildGuiLinux {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sync(..) => write!(f, "syncing repo to Ubuntu"),
            Self::Updater(..) => write!(f, "building multiworld-updater for Linux"),
            Self::Gui(..) => write!(f, "building multiworld-gui for Linux"),
            Self::Copy(..) => write!(f, "copying multiworld-gui for Linux to Windows"),
            Self::Read(..) => write!(f, "reading multiworld-gui-linux"),
            Self::WaitRelease(..) => write!(f, "waiting for GitHub release to be created"),
            Self::Upload(..) => write!(f, "uploading multiworld-gui-linux"),
        }
    }
}

impl Progress for BuildGuiLinux {
    fn progress(&self) -> Percent {
        Percent::fraction(match self {
            Self::Sync(..) => 0,
            Self::Updater(..) => 1,
            Self::Gui(..) => 2,
            Self::Copy(..) => 3,
            Self::Read(..) => 4,
            Self::WaitRelease(..) => 5,
            Self::Upload(..) => 6,
        }, 7)
    }
}

impl GetPriority for BuildGuiLinux {
    fn priority(&self) -> Priority {
        match self {
            Self::Sync(..) => Priority::Active,
            Self::Updater(..) => Priority::Active,
            Self::Gui(..) => Priority::Active,
            Self::Copy(..) => Priority::Active,
            Self::Read(..) => Priority::Active,
            Self::WaitRelease(..) => Priority::Waiting,
            Self::Upload(..) => Priority::Active,
        }
    }
}

#[async_trait]
impl Task<Result<(), Error>> for BuildGuiLinux {
    async fn run(self) -> Result<Result<(), Error>, Self> {
        match self {
            Self::Sync(client, repo, release_rx, gui_tx) => gres::transpose(async move {
                Command::new("wsl").arg("rsync").arg("--delete").arg("-av").arg("/mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/").arg("/home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/").arg("--exclude").arg(".cargo/config.toml").arg("--exclude").arg("target").arg("--exclude").arg("crate/multiworld-bizhawk/OotrMultiworld/BizHawk").arg("--exclude").arg("crate/multiworld-bizhawk/OotrMultiworld/src/bin").arg("--exclude").arg("crate/multiworld-bizhawk/OotrMultiworld/src/obj").arg("--exclude").arg("crate/multiworld-bizhawk/OotrMultiworld/src/multiworld.dll").check("wsl rsync").await?;
                Ok(Err(Self::Updater(client, repo, release_rx, gui_tx)))
            }).await,
            Self::Updater(client, repo, release_rx, gui_tx) => gres::transpose(async move {
                Command::new("wsl").arg("env").arg("-C").arg("/home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld").arg("cargo").arg("build").arg("--release").arg("--package=multiworld-updater").check("wsl cargo build --package=multiworld-updater").await?;
                Ok(Err(Self::Gui(client, repo, release_rx, gui_tx)))
            }).await,
            Self::Gui(client, repo, release_rx, gui_tx) => gres::transpose(async move {
                Command::new("wsl").arg("env").arg("-C").arg("/home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld").arg("cargo").arg("build").arg("--release").arg("--package=multiworld-gui").check("wsl cargo build --package=multiworld-gui").await?;
                let _ = gui_tx.send(LinuxGuiNotification);
                Ok(Err(Self::Copy(client, repo, release_rx)))
            }).await,
            Self::Copy(client, repo, release_rx) => gres::transpose(async move {
                fs::create_dir_all("target/wsl/release").await?;
                Command::new("wsl").arg("cp").arg("/home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/target/release/multiworld-gui").arg("/mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/target/wsl/release/multiworld-gui").check("wsl cp").await?;
                Ok(Err(Self::Read(client, repo, release_rx)))
            }).await,
            Self::Read(client, repo, release_rx) => gres::transpose(async move {
                let data = fs::read("target/wsl/release/multiworld-gui").await?;
                Ok(Err(Self::WaitRelease(client, repo, release_rx, data)))
            }).await,
            Self::WaitRelease(client, repo, mut release_rx, data) => gres::transpose(async move {
                let release = release_rx.recv().await?;
                Ok(Err(Self::Upload(client, repo, release, data)))
            }).await,
            Self::Upload(client, repo, release, data) => gres::transpose(async move {
                repo.release_attach(&client, &release, "multiworld-gui-linux", "application/x-executable", data).await?;
                Ok(Ok(()))
            }).await,
        }
    }
}

enum BuildBizHawk {
    Gui(bool, reqwest::Client, Repo, broadcast::Receiver<WindowsGuiNotification>, broadcast::Receiver<Release>, Version, broadcast::Sender<WindowsBizHawkNotification>),
    CSharp(bool, reqwest::Client, Repo, broadcast::Receiver<Release>, Version, broadcast::Sender<WindowsBizHawkNotification>),
    BizHawk(bool, reqwest::Client, Repo, broadcast::Receiver<Release>, Version, broadcast::Sender<WindowsBizHawkNotification>),
    Zip(reqwest::Client, Repo, broadcast::Receiver<Release>, Version),
    WaitRelease(reqwest::Client, Repo, broadcast::Receiver<Release>, Vec<u8>),
    Upload(reqwest::Client, Repo, Release, Vec<u8>),
}

impl BuildBizHawk {
    fn new(debug: bool, client: reqwest::Client, repo: Repo, gui_rx: broadcast::Receiver<WindowsGuiNotification>, release_rx: broadcast::Receiver<Release>, version: Version, bizhawk_tx: broadcast::Sender<WindowsBizHawkNotification>) -> Self {
        Self::Gui(debug, client, repo, gui_rx, release_rx, version, bizhawk_tx)
    }
}

impl fmt::Display for BuildBizHawk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gui(..) => write!(f, "waiting for Windows GUI build to finish"),
            Self::CSharp(false, ..) => write!(f, "building multiworld-csharp for Windows"),
            Self::CSharp(true, ..) => write!(f, "building multiworld-csharp (debug) for Windows"),
            Self::BizHawk(false, ..) => write!(f, "building multiworld-bizhawk for Windows"),
            Self::BizHawk(true, ..) => write!(f, "building multiworld-bizhawk (debug) for Windows"),
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

impl GetPriority for BuildBizHawk {
    fn priority(&self) -> Priority {
        match self {
            Self::Gui(..) => Priority::Waiting,
            Self::CSharp(..) => Priority::Active,
            Self::BizHawk(..) => Priority::Active,
            Self::Zip(..) => Priority::Active,
            Self::WaitRelease(..) => Priority::Waiting,
            Self::Upload(..) => Priority::Active,
        }
    }
}

#[async_trait]
impl Task<Result<(), Error>> for BuildBizHawk {
    async fn run(self) -> Result<Result<(), Error>, Self> {
        match self {
            Self::Gui(debug, client, repo, mut gui_rx, release_rx, version, bizhawk_tx) => gres::transpose(async move {
                let WindowsGuiNotification = gui_rx.recv().await?;
                Ok(Err(Self::CSharp(debug, client, repo, release_rx, version, bizhawk_tx)))
            }).await,
            Self::CSharp(debug, client, repo, release_rx, version, bizhawk_tx) => gres::transpose(async move {
                let mut build = Command::new("cargo");
                build.arg("build");
                if !debug {
                    build.arg("--release");
                }
                build.arg("--package=multiworld-csharp");
                build.check("cargo build --package=multiworld-csharp").await?;
                Ok(Err(Self::BizHawk(debug, client, repo, release_rx, version, bizhawk_tx)))
            }).await,
            Self::BizHawk(debug, client, repo, release_rx, version, bizhawk_tx) => gres::transpose(async move {
                let mut build = Command::new("cargo");
                build.arg("build");
                if !debug {
                    build.arg("--release");
                }
                build.arg("--package=multiworld-bizhawk");
                build.check("cargo build --package=multiworld-bizhawk").await?;
                let _ = bizhawk_tx.send(WindowsBizHawkNotification);
                Ok(if debug {
                    Ok(())
                } else {
                    Err(Self::Zip(client, repo, release_rx, version))
                })
            }).await,
            Self::Zip(client, repo, release_rx, version) => gres::transpose(async move {
                let zip_data = tokio::task::spawn_blocking(move || {
                    let mut buf = Cursor::<Vec<_>>::default();
                    {
                        let mut zip = ZipWriter::new(&mut buf); //TODO replace with an async zip writer
                        zip.start_file("README.txt", FileOptions::default())?;
                        write!(&mut zip, include_str!("../../../assets/bizhawk-readme-windows.txt"), version)?;
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

enum BuildBizHawkLinux {
    Gui(reqwest::Client, Repo, broadcast::Receiver<LinuxGuiNotification>, broadcast::Receiver<Release>, Version, broadcast::Sender<LinuxBizHawkNotification>),
    CSharp(reqwest::Client, Repo, broadcast::Receiver<Release>, Version, broadcast::Sender<LinuxBizHawkNotification>),
    BizHawk(reqwest::Client, Repo, broadcast::Receiver<Release>, Version, broadcast::Sender<LinuxBizHawkNotification>),
    Copy(reqwest::Client, Repo, broadcast::Receiver<Release>, Version),
    Zip(reqwest::Client, Repo, broadcast::Receiver<Release>, Version),
    WaitRelease(reqwest::Client, Repo, broadcast::Receiver<Release>, Vec<u8>),
    Upload(reqwest::Client, Repo, Release, Vec<u8>),
}

impl BuildBizHawkLinux {
    fn new(client: reqwest::Client, repo: Repo, gui_rx: broadcast::Receiver<LinuxGuiNotification>, release_rx: broadcast::Receiver<Release>, version: Version, bizhawk_tx: broadcast::Sender<LinuxBizHawkNotification>) -> Self {
        Self::Gui(client, repo, gui_rx, release_rx, version, bizhawk_tx)
    }
}

impl fmt::Display for BuildBizHawkLinux {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gui(..) => write!(f, "waiting for Linux GUI build to finish"),
            Self::CSharp(..) => write!(f, "building multiworld-csharp for Linux"),
            Self::BizHawk(..) => write!(f, "building multiworld-bizhawk for Linux"),
            Self::Copy(..) => write!(f, "copying multiworld-bizhawk for Linux to Windows"),
            Self::Zip(..) => write!(f, "creating multiworld-bizhawk-linux.zip"),
            Self::WaitRelease(..) => write!(f, "waiting for GitHub release to be created"),
            Self::Upload(..) => write!(f, "uploading multiworld-bizhawk-linux.zip"),
        }
    }
}

impl Progress for BuildBizHawkLinux {
    fn progress(&self) -> Percent {
        Percent::fraction(match self {
            Self::Gui(..) => 0,
            Self::CSharp(..) => 1,
            Self::BizHawk(..) => 2,
            Self::Copy(..) => 3,
            Self::Zip(..) => 4,
            Self::WaitRelease(..) => 5,
            Self::Upload(..) => 6,
        }, 7)
    }
}

impl GetPriority for BuildBizHawkLinux {
    fn priority(&self) -> Priority {
        match self {
            Self::Gui(..) => Priority::Waiting,
            Self::CSharp(..) => Priority::Active,
            Self::BizHawk(..) => Priority::Active,
            Self::Copy(..) => Priority::Active,
            Self::Zip(..) => Priority::Active,
            Self::WaitRelease(..) => Priority::Waiting,
            Self::Upload(..) => Priority::Active,
        }
    }
}

#[async_trait]
impl Task<Result<(), Error>> for BuildBizHawkLinux {
    async fn run(self) -> Result<Result<(), Error>, Self> {
        match self {
            Self::Gui(client, repo, mut gui_rx, release_rx, version, bizhawk_tx) => gres::transpose(async move {
                let LinuxGuiNotification = gui_rx.recv().await?;
                Ok(Err(Self::CSharp(client, repo, release_rx, version, bizhawk_tx)))
            }).await,
            Self::CSharp(client, repo, release_rx, version, bizhawk_tx) => gres::transpose(async move {
                Command::new("wsl").arg("env").arg("-C").arg("/home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld").arg("cargo").arg("build").arg("--release").arg("--package=multiworld-csharp").check("wsl cargo build --package=multiworld-csharp").await?;
                Ok(Err(Self::BizHawk(client, repo, release_rx, version, bizhawk_tx)))
            }).await,
            Self::BizHawk(client, repo, release_rx, version, bizhawk_tx) => gres::transpose(async move {
                Command::new("wsl").arg("env").arg("-C").arg("/home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld").arg("cargo").arg("build").arg("--release").arg("--package=multiworld-bizhawk").check("wsl cargo build --package=multiworld-bizhawk").await?;
                let _ = bizhawk_tx.send(LinuxBizHawkNotification);
                Ok(Err(Self::Copy(client, repo, release_rx, version)))
            }).await,
            Self::Copy(client, repo, release_rx, version) => gres::transpose(async move {
                fs::create_dir_all("target/wsl/release").await?;
                Command::new("wsl").arg("cp").arg("/home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/crate/multiworld-bizhawk/OotrMultiworld/BizHawk/dll/libmultiworld.so").arg("/mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/target/wsl/release/libmultiworld.so").check("wsl cp").await?;
                Command::new("wsl").arg("cp").arg("/home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/crate/multiworld-bizhawk/OotrMultiworld/BizHawk/ExternalTools/OotrMultiworld.dll").arg("/mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/target/wsl/release/OotrMultiworld.dll").check("wsl cp").await?;
                Ok(Err(Self::Zip(client, repo, release_rx, version)))
            }).await,
            Self::Zip(client, repo, release_rx, version) => gres::transpose(async move {
                let zip_data = tokio::task::spawn_blocking(move || {
                    let mut buf = Cursor::<Vec<_>>::default();
                    {
                        let mut zip = ZipWriter::new(&mut buf); //TODO replace with an async zip writer
                        zip.start_file("README.txt", FileOptions::default())?;
                        write!(&mut zip, include_str!("../../../assets/bizhawk-readme-linux.txt"), version)?;
                        zip.start_file("OotrMultiworld.dll", FileOptions::default())?;
                        std::io::copy(&mut std::fs::File::open("target/wsl/release/OotrMultiworld.dll")?, &mut zip)?;
                        zip.start_file("libmultiworld.so", FileOptions::default())?;
                        std::io::copy(&mut std::fs::File::open("target/wsl/release/libmultiworld.so")?, &mut zip)?;
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
                repo.release_attach(&client, &release, "multiworld-bizhawk-linux.zip", "application/zip", zip_data).await?;
                Ok(Ok(()))
            }).await,
        }
    }
}

enum BuildPj64 {
    ReadJs(reqwest::Client, Repo, broadcast::Receiver<Release>),
    WaitRelease(reqwest::Client, Repo, broadcast::Receiver<Release>, Vec<u8>),
    UploadJs(reqwest::Client, Repo, Release, Vec<u8>),
}

impl BuildPj64 {
    fn new(client: reqwest::Client, repo: Repo, release_rx: broadcast::Receiver<Release>) -> Self {
        Self::ReadJs(client, repo, release_rx)
    }
}

impl fmt::Display for BuildPj64 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadJs(..) => write!(f, "reading ootrmw-pj64.js"),
            Self::WaitRelease(..) => write!(f, "waiting for GitHub release to be created"),
            Self::UploadJs(..) => write!(f, "uploading ootrmw-pj64.js"),
        }
    }
}

impl Progress for BuildPj64 {
    fn progress(&self) -> Percent {
        Percent::fraction(match self {
            Self::ReadJs(..) => 0,
            Self::WaitRelease(..) => 1,
            Self::UploadJs(..) => 2,
        }, 3)
    }
}

impl GetPriority for BuildPj64 {
    fn priority(&self) -> Priority {
        match self {
            Self::ReadJs(..) => Priority::Active,
            Self::WaitRelease(..) => Priority::Waiting,
            Self::UploadJs(..) => Priority::Active,
        }
    }
}

#[async_trait]
impl Task<Result<(), Error>> for BuildPj64 {
    async fn run(self) -> Result<Result<(), Error>, Self> {
        match self {
            Self::ReadJs(client, repo, release_rx) => gres::transpose(async move {
                let data = fs::read("assets/ootrmw-pj64.js").await?;
                Ok(Err(Self::WaitRelease(client, repo, release_rx, data)))
            }).await,
            Self::WaitRelease(client, repo, mut release_rx, data) => gres::transpose(async move {
                let release = release_rx.recv().await?;
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
    Deps(bool, reqwest::Client, Repo, broadcast::Receiver<WindowsBizHawkNotification>, broadcast::Receiver<WindowsGuiNotification>, broadcast::Receiver<Release>),
    Glow(bool, reqwest::Client, Repo, broadcast::Receiver<Release>),
    Read(bool, reqwest::Client, Repo, broadcast::Receiver<Release>),
    WaitRelease(bool, reqwest::Client, Repo, broadcast::Receiver<Release>, Vec<u8>),
    Upload(bool, reqwest::Client, Repo, Release, Vec<u8>),
}

impl BuildInstaller {
    fn new(debug: bool, client: reqwest::Client, repo: Repo, bizhawk_rx: broadcast::Receiver<WindowsBizHawkNotification>, gui_rx: broadcast::Receiver<WindowsGuiNotification>, release_rx: broadcast::Receiver<Release>) -> Self {
        Self::Deps(debug, client, repo, bizhawk_rx, gui_rx, release_rx)
    }
}

impl fmt::Display for BuildInstaller {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Deps(..) => write!(f, "waiting for dependency builds to finish"),
            Self::Glow(false, ..) => write!(f, "building multiworld-installer.exe"),
            Self::Glow(true, ..) => write!(f, "building multiworld-installer-debug.exe"),
            Self::Read(false, ..) => write!(f, "reading multiworld-installer.exe"),
            Self::Read(true, ..) => write!(f, "reading multiworld-installer-debug.exe"),
            Self::WaitRelease(..) => write!(f, "waiting for GitHub release to be created"),
            Self::Upload(false, ..) => write!(f, "uploading multiworld-installer.exe"),
            Self::Upload(true, ..) => write!(f, "uploading multiworld-installer-debug.exe"),
        }
    }
}

impl Progress for BuildInstaller {
    fn progress(&self) -> Percent {
        Percent::fraction(match self {
            Self::Deps(..) => 0,
            Self::Glow(..) => 1,
            Self::Read(..) => 2,
            Self::WaitRelease(..) => 3,
            Self::Upload(..) => 4,
        }, 5)
    }
}

impl GetPriority for BuildInstaller {
    fn priority(&self) -> Priority {
        match self {
            Self::Deps(..) => Priority::Waiting,
            Self::Glow(..) => Priority::Active,
            Self::Read(..) => Priority::Active,
            Self::WaitRelease(..) => Priority::Waiting,
            Self::Upload(..) => Priority::Active,
        }
    }
}

#[async_trait]
impl Task<Result<(), Error>> for BuildInstaller {
    async fn run(self) -> Result<Result<(), Error>, Self> {
        match self {
            Self::Deps(debug, client, repo, mut bizhawk_rx, mut gui_rx, release_rx) => gres::transpose(async move {
                let WindowsBizHawkNotification = bizhawk_rx.recv().await?;
                let WindowsGuiNotification = gui_rx.recv().await?;
                Ok(Err(Self::Glow(debug, client, repo, release_rx)))
            }).await,
            Self::Glow(debug, client, repo, release_rx) => gres::transpose(async move {
                let mut build = Command::new("cargo");
                build.arg("build");
                if !debug {
                    build.arg("--release");
                }
                build.arg("--package=multiworld-installer");
                build.check("cargo build --package=multiworld-installer").await?;
                Ok(Err(Self::Read(debug, client, repo, release_rx)))
            }).await,
            Self::Read(debug, client, repo, release_rx) => gres::transpose(async move {
                let data = fs::read(if debug {
                    "target/debug/multiworld-installer.exe"
                } else {
                    "target/release/multiworld-installer.exe"
                }).await?;
                Ok(Err(Self::WaitRelease(debug, client, repo, release_rx, data)))
            }).await,
            Self::WaitRelease(debug, client, repo, mut release_rx, data) => gres::transpose(async move {
                let release = release_rx.recv().await?;
                Ok(Err(Self::Upload(debug, client, repo, release, data)))
            }).await,
            Self::Upload(debug, client, repo, release, data) => gres::transpose(async move {
                repo.release_attach(&client, &release, if debug { "multiworld-installer-debug.exe" } else { "multiworld-installer.exe" }, "application/vnd.microsoft.portable-executable", data).await?;
                Ok(Ok(()))
            }).await,
        }
    }
}

enum BuildInstallerLinux {
    Deps(reqwest::Client, Repo, broadcast::Receiver<LinuxBizHawkNotification>, broadcast::Receiver<Release>),
    Glow(reqwest::Client, Repo, broadcast::Receiver<Release>),
    Copy(reqwest::Client, Repo, broadcast::Receiver<Release>),
    Read(reqwest::Client, Repo, broadcast::Receiver<Release>),
    WaitRelease(reqwest::Client, Repo, broadcast::Receiver<Release>, Vec<u8>),
    Upload(reqwest::Client, Repo, Release, Vec<u8>),
}

impl BuildInstallerLinux {
    fn new(client: reqwest::Client, repo: Repo, bizhawk_rx: broadcast::Receiver<LinuxBizHawkNotification>, release_rx: broadcast::Receiver<Release>) -> Self {
        Self::Deps(client, repo, bizhawk_rx, release_rx)
    }
}

impl fmt::Display for BuildInstallerLinux {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Deps(..) => write!(f, "waiting for dependency builds to finish"),
            Self::Glow(..) => write!(f, "building multiworld-installer-linux"),
            Self::Copy(..) => write!(f, "copying multiworld-installer-linux to Windows"),
            Self::Read(..) => write!(f, "reading multiworld-installer-linux"),
            Self::WaitRelease(..) => write!(f, "waiting for GitHub release to be created"),
            Self::Upload(..) => write!(f, "uploading multiworld-installer-linux"),
        }
    }
}

impl Progress for BuildInstallerLinux {
    fn progress(&self) -> Percent {
        Percent::fraction(match self {
            Self::Deps(..) => 0,
            Self::Glow(..) => 1,
            Self::Copy(..) => 2,
            Self::Read(..) => 3,
            Self::WaitRelease(..) => 4,
            Self::Upload(..) => 5,
        }, 6)
    }
}

impl GetPriority for BuildInstallerLinux {
    fn priority(&self) -> Priority {
        match self {
            Self::Deps(..) => Priority::Waiting,
            Self::Glow(..) => Priority::Active,
            Self::Copy(..) => Priority::Active,
            Self::Read(..) => Priority::Active,
            Self::WaitRelease(..) => Priority::Waiting,
            Self::Upload(..) => Priority::Active,
        }
    }
}

#[async_trait]
impl Task<Result<(), Error>> for BuildInstallerLinux {
    async fn run(self) -> Result<Result<(), Error>, Self> {
        match self {
            Self::Deps(client, repo, mut bizhawk_rx, release_rx) => gres::transpose(async move {
                let LinuxBizHawkNotification = bizhawk_rx.recv().await?;
                Ok(Err(Self::Glow(client, repo, release_rx)))
            }).await,
            Self::Glow(client, repo, release_rx) => gres::transpose(async move {
                Command::new("wsl").arg("env").arg("-C").arg("/home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld").arg("cargo").arg("build").arg("--release").arg("--package=multiworld-installer").check("wsl cargo build --package=multiworld-installer").await?;
                Ok(Err(Self::Copy(client, repo, release_rx)))
            }).await,
            Self::Copy(client, repo, release_rx) => gres::transpose(async move {
                fs::create_dir_all("target/wsl/release").await?;
                Command::new("wsl").arg("cp").arg("/home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/target/release/multiworld-installer").arg("/mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/target/wsl/release/multiworld-installer").check("wsl cp").await?;
                Ok(Err(Self::Read(client, repo, release_rx)))
            }).await,
            Self::Read(client, repo, release_rx) => gres::transpose(async move {
                let data = fs::read("target/wsl/release/multiworld-installer").await?;
                Ok(Err(Self::WaitRelease(client, repo, release_rx, data)))
            }).await,
            Self::WaitRelease(client, repo, mut release_rx, data) => gres::transpose(async move {
                let release = release_rx.recv().await?;
                Ok(Err(Self::Upload(client, repo, release, data)))
            }).await,
            Self::Upload(client, repo, release, data) => gres::transpose(async move {
                repo.release_attach(&client, &release, "multiworld-installer-linux", "application/x-executable", data).await?;
                Ok(Ok(()))
            }).await,
        }
    }
}

enum BuildServer {
    Sync(bool),
    Build(bool),
    Copy(bool),
    Upload(bool),
    WaitRestart {
        start: DateTime<Utc>,
        child: Option<Child>,
        stdout: Option<ChildStdout>,
        rooms: Option<HashMap<String, (DateTime<Utc>, u64)>>,
        deadline: Option<DateTime<Utc>>,
    },
    Stop,
    UpdateRepo,
    Replace,
    Start,
}

impl BuildServer {
    fn new(wait_restart: bool) -> Self {
        Self::Sync(wait_restart)
    }
}

impl fmt::Display for BuildServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sync(..) => write!(f, "syncing repo to Debian"),
            Self::Build(..) => write!(f, "building ootrmwd"),
            Self::Copy(..) => write!(f, "copying ootrmwd to Windows"),
            Self::Upload(..) => write!(f, "uploading ootrmwd to Mido's House"),
            Self::WaitRestart { rooms, deadline, .. } => write!(f, "waiting for {}rooms to be inactive{}{}",
                if let Some(rooms) = rooms { format!("{} ", rooms.len()) } else { String::default() },
                if let Some(rooms) = rooms { format!(" (current ETA: {})", rooms.values().map(|(inactive_at, _)| inactive_at).max().expect("waiting for 0 rooms").format("%Y-%m-%d %H:%M:%S UTC")) } else { String::default() },
                if let Some(deadline) = deadline { format!(" or until {}", deadline.format("%Y-%m-%d %H:%M:%S UTC")) } else { String::default() },
            ),
            Self::Stop => write!(f, "stopping old ootrmwd"),
            Self::UpdateRepo => write!(f, "updating repo on Mido's House"),
            Self::Replace => write!(f, "replacing ootrmwd binary on Mido's House"),
            Self::Start => write!(f, "starting new ootrmwd"),
        }
    }
}

impl Progress for BuildServer {
    fn progress(&self) -> Percent {
        Percent::new(match self {
            Self::Sync(..) => 0,
            Self::Build(..) => 10,
            Self::Copy(..) => 40,
            Self::Upload(..) => 45,
            Self::WaitRestart { deadline: Some(deadline), start, .. } => {
                let Ok(total) = (*deadline - *start).to_std() else { return Percent::new(90) };
                let Ok(elapsed) = (Utc::now() - *start).to_std() else { return Percent::new(50) };
                50 + (40.0 * elapsed.as_secs_f64() / total.as_secs_f64()) as u8
            }
            Self::WaitRestart { deadline: None, .. } => 50,
            Self::Stop => 95,
            Self::UpdateRepo => 96,
            Self::Replace => 98,
            Self::Start => 99,
        })
    }
}

impl GetPriority for BuildServer {
    fn priority(&self) -> Priority {
        match self {
            Self::Sync(..) => Priority::Active,
            Self::Build(..) => Priority::Active,
            Self::Copy(..) => Priority::Active,
            Self::Upload(..) => Priority::Active,
            Self::WaitRestart { .. } => Priority::Waiting,
            Self::Stop => Priority::Active,
            Self::UpdateRepo => Priority::Active,
            Self::Replace => Priority::Active,
            Self::Start => Priority::Active,
        }
    }
}

#[async_trait]
impl Task<Result<(), Error>> for BuildServer {
    async fn run(self) -> Result<Result<(), Error>, Self> {
        match self {
            Self::Sync(wait_restart) => gres::transpose(async move {
                Command::new("wsl").arg("--distribution").arg("debian-m2").arg("rsync").arg("--delete").arg("-av").arg("/mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/").arg("/home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/").arg("--exclude").arg(".cargo/config.toml").arg("--exclude").arg("target").arg("--exclude").arg("crate/multiworld-bizhawk/OotrMultiworld/BizHawk").arg("--exclude").arg("crate/multiworld-bizhawk/OotrMultiworld/src/bin").arg("--exclude").arg("crate/multiworld-bizhawk/OotrMultiworld/src/obj").arg("--exclude").arg("crate/multiworld-bizhawk/OotrMultiworld/src/multiworld.dll").check("debian run rsync").await?;
                Ok(Err(Self::Build(wait_restart)))
            }).await,
            Self::Build(wait_restart) => gres::transpose(async move {
                Command::new("wsl").arg("--distribution").arg("debian-m2").arg("env").arg("-C").arg("/home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld").arg("/home/fenhl/.cargo/bin/cargo").arg("build").arg("--release").arg("--package=ootrmwd").check("debian run cargo build --package=ootrmwd").await?;
                Ok(Err(Self::Copy(wait_restart)))
            }).await,
            Self::Copy(wait_restart) => gres::transpose(async move {
                fs::create_dir_all("target/wsl/release").await?;
                Command::new("wsl").arg("--distribution").arg("debian-m2").arg("cp").arg("/home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/target/release/ootrmwd").arg("/mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/target/wsl/release/ootrmwd").check("debian run cp").await?;
                Ok(Err(Self::Upload(wait_restart)))
            }).await,
            Self::Upload(wait_restart) => gres::transpose(async move {
                Command::new("scp").arg("target/wsl/release/ootrmwd").arg("midos.house:bin/ootrmwd-next").check("scp").await?;
                Ok(Err(if wait_restart {
                    Self::WaitRestart { start: Utc::now(), child: None, stdout: None, rooms: None, deadline: None }
                } else {
                    Self::Stop
                }))
            }).await,
            Self::WaitRestart { start, child: None, .. } => gres::transpose(async move {
                Ok(Err(if Command::new("ssh").arg("midos.house").arg("systemctl is-active ootrmw").status().await?.code() == Some(3) {
                    Self::Stop
                } else {
                    let mut child = Command::new("ssh").arg("midos.house").arg("sudo -u mido /usr/local/share/midos-house/bin/ootrmwd prepare-restart --async-proto").stdout(Stdio::piped()).spawn()?;
                    let stdout = child.stdout.take().expect("stdout was piped");
                    Self::WaitRestart { start, child: Some(child), stdout: Some(stdout), rooms: None, deadline: None }
                }))
            }).await,
            Self::WaitRestart { start, child: Some(child), stdout: Some(mut stdout), rooms, deadline } => gres::transpose(async move {
                match WaitUntilInactiveMessage::read(&mut stdout).await {
                    Ok(WaitUntilInactiveMessage::Error) => Err(Error::WaitUntilInactive),
                    Ok(WaitUntilInactiveMessage::ActiveRooms(rooms)) => Ok(Err(Self::WaitRestart { start, child: Some(child), stdout: Some(stdout), rooms: Some(rooms), deadline })),
                    Ok(WaitUntilInactiveMessage::Inactive) => Ok(Err(Self::WaitRestart { start, child: Some(child), stdout: None, rooms, deadline })),
                    Ok(WaitUntilInactiveMessage::Deadline(deadline)) => Ok(Err(Self::WaitRestart { start, child: Some(child), stdout: Some(stdout), rooms, deadline: Some(deadline) })),
                    Err(e) => Err(e.into()),
                }
            }).await,
            Self::WaitRestart { start, child: Some(child), stdout: None, .. } => gres::transpose(async move {
                Ok(Err(match child.check("ssh midos.house ootrmwd prepare-restart").await {
                    Ok(_) => Self::Stop,
                    Err(wheel::Error::CommandExit { output, .. }) if std::str::from_utf8(&output.stderr).is_ok_and(|stderr| stderr.contains("Connection reset")) => {
                        // try again
                        Self::WaitRestart { start, child: None, stdout: None, rooms: None, deadline: None }
                    }
                    Err(e) => if Command::new("ssh").arg("midos.house").arg("systemctl is-active ootrmw").status().await?.code() == Some(3) {
                        // prepare-restart command failed because the multiworld server was stopped
                        Self::Stop
                    } else {
                        return Err(e.into())
                    },
                }))
            }).await,
            Self::Stop => gres::transpose(async move {
                Command::new("ssh").arg("midos.house").arg("sudo systemctl stop ootrmw").check("ssh midos.house systemctl stop ootrmw").await?;
                Ok(Err(Self::UpdateRepo))
            }).await,
            Self::UpdateRepo => gres::transpose(async move {
                Command::new("ssh").arg("midos.house").arg("cd /opt/git/github.com/midoshouse/ootr-multiworld/main && git pull --ff-only").check("ssh midos.house git pull").await?;
                Ok(Err(Self::Replace))
            }).await,
            Self::Replace => gres::transpose(async move {
                Command::new("ssh").arg("midos.house").arg("sudo chown mido:www-data bin/ootrmwd-next && sudo chmod +x bin/ootrmwd-next && sudo mv bin/ootrmwd-next /usr/local/share/midos-house/bin/ootrmwd").check("ssh midos.house chown && chmod && mv").await?;
                Ok(Err(Self::Start))
            }).await,
            Self::Start => gres::transpose(async move {
                Command::new("ssh").arg("midos.house").arg("sudo systemctl start ootrmw").check("ssh midos.house systemctl start ootrmw").await?;
                Ok(Ok(()))
            }).await,
        }
    }
}

#[derive(clap::Parser)]
#[clap(version)]
enum CliArgs {
    /// Update both the server and the client
    Both {
        /// Don't wait for the server to be inactive before restarting it
        #[clap(long, conflicts_with("no_server"))]
        force: bool,
        /// Create the GitHub release as a draft
        #[clap(short = 'P', long)]
        no_publish: bool,
        /// Don't pass `--wait` to the release notes editor
        #[clap(short = 'W', long)]
        no_wait: bool,
    },
    /// Only update the client
    Client {
        /// Create the GitHub release as a draft
        #[clap(short = 'P', long)]
        no_publish: bool,
        /// Don't pass `--wait` to the release notes editor
        #[clap(short = 'W', long)]
        no_wait: bool,
    },
    /// Only update the server
    Server {
        /// Don't wait for the server to be inactive before restarting it
        #[clap(long, conflicts_with("no_server"))]
        force: bool,
    },
}

#[derive(Clone)]
struct Args {
    force: bool,
    no_publish: bool,
    no_server: bool,
    no_wait: bool,
    server_only: bool,
}

impl From<CliArgs> for Args {
    fn from(args: CliArgs) -> Self {
        match args {
            CliArgs::Both { force, no_publish, no_wait } => Self {
                no_server: false,
                server_only: false,
                force, no_publish, no_wait,
            },
            CliArgs::Client { no_publish, no_wait } => Self {
                force: false,
                no_server: true,
                server_only: false,
                no_publish, no_wait,
            },
            CliArgs::Server { force } => Self {
                no_publish: false,
                no_server: false,
                no_wait: false,
                server_only: true,
                force,
            },
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] BizHawkVersionCheck(#[from] version::BizHawkError),
    #[error(transparent)] BroadcastRecv(#[from] broadcast::error::RecvError),
    #[error(transparent)] DirLock(#[from] dir_lock::Error),
    #[error(transparent)] GitHub(#[from] multiworld::github::Error),
    #[error(transparent)] GitHubAppAuth(#[from] github_app_auth::AuthError),
    #[error(transparent)] InvalidHeaderValue(#[from] reqwest::header::InvalidHeaderValue),
    #[error(transparent)] Io(#[from] tokio::io::Error),
    #[error(transparent)] ParseInt(#[from] std::num::ParseIntError),
    #[error(transparent)] Read(#[from] async_proto::ReadError),
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
    #[error("frontend protocol version not found in Project64 frontend code")]
    MissingPj64ProtocolVersion,
    #[error("there is already a release with this version number")]
    SameVersion,
    #[error("the latest GitHub release has a newer version than the local crate version")]
    VersionRegression,
    #[error("the ootrmwd prepare-restart command sent a generic error message")]
    WaitUntilInactive,
    #[error("frontend protocol version mismatch: client is v{}, Project64 frontend is v{0}", frontend::PROTOCOL_VERSION)]
    WrongPj64ProtocolVersion(u8),
}

impl wheel::CustomExit for Error {
    fn exit(self, cmd_name: &'static str) -> ! {
        match self {
            Self::Wheel(wheel::Error::CommandExit { name, output }) => {
                eprintln!("command `{name}` exited with {}", output.status);
                eprintln!();
                if let Ok(stdout) = std::str::from_utf8(&output.stdout) {
                    eprintln!("stdout:");
                    eprintln!("{stdout}");
                } else {
                    eprintln!("stdout: {:?}", output.stdout);
                }
                if let Ok(stderr) = std::str::from_utf8(&output.stderr) {
                    eprintln!("stderr:");
                    eprintln!("{stderr}");
                } else {
                    eprintln!("stderr: {:?}", output.stderr);
                }
                process::exit(output.status.code().unwrap_or(1))
            }
            e => {
                eprintln!("{cmd_name}: {e}");
                eprintln!("debug info: {e:?}");
                process::exit(1)
            }
        }
    }
}

/// Separate function to ensure CLI is dropped before exit
async fn cli_main(cli: &Cli, args: Args) -> Result<(), Error> {
    let (client, repo, bizhawk_version) = cli.run(Setup::new(args.server_only), "pre-release checks").await??; // don't show release notes editor if version check could still fail
    if args.server_only {
        cli.run(BuildServer::new(!args.force), "server").await??;
    } else {
        let bizhawk_version_debug = bizhawk_version.clone();
        let bizhawk_version_linux = bizhawk_version.clone();
        let (release_tx, release_rx_installer_debug) = broadcast::channel(1);
        let release_rx_installer = release_tx.subscribe();
        let release_rx_installer_linux = release_tx.subscribe();
        let release_rx_gui_debug = release_tx.subscribe();
        let release_rx_gui = release_tx.subscribe();
        let release_rx_gui_linux = release_tx.subscribe();
        let release_rx_bizhawk_debug = release_tx.subscribe();
        let release_rx_bizhawk = release_tx.subscribe();
        let release_rx_bizhawk_linux = release_tx.subscribe();
        let release_rx_pj64 = release_tx.subscribe();
        let create_release_args = args.clone();
        let create_release_client = client.clone();
        let create_release_repo = repo.clone();
        let (debug_updater_tx, debug_updater_rx) = broadcast::channel(1);
        let (updater_tx, updater_rx) = broadcast::channel(1);
        let (debug_gui_tx, debug_gui_rx) = broadcast::channel(1);
        let debug_gui_rx_installer = debug_gui_tx.subscribe();
        let (gui_tx, gui_rx) = broadcast::channel(1);
        let gui_rx_installer = gui_tx.subscribe();
        let (linux_gui_tx, linux_gui_rx) = broadcast::channel(1);
        let (debug_bizhawk_tx, debug_bizhawk_rx) = broadcast::channel(1);
        let (bizhawk_tx, bizhawk_rx) = broadcast::channel(1);
        let (linux_bizhawk_tx, linux_bizhawk_rx) = broadcast::channel(1);

        macro_rules! with_metavariable {
            ($metavariable:tt, $($token:tt)*) => { $($token)* };
        }

        macro_rules! build_tasks {
            (release = $create_release:expr, $($task:expr,)*) => {{
                let (release, $(with_metavariable!($task, ())),*) = tokio::try_join!($create_release, $($task),*)?;
                release
            }};
        }

        let release = build_tasks![
            release = cli.run(CreateRelease::new(create_release_repo, create_release_client, release_tx, create_release_args), "creating release").map_err(Error::Io),
            async move { cli.run(BuildUpdater::new(true, debug_updater_tx), "updater (debug)").await? },
            async move { cli.run(BuildUpdater::new(false, updater_tx), "updater").await? },
            { let client = client.clone(); let repo = repo.clone(); async move { cli.run(BuildGui::new(true, client, repo, debug_updater_rx, release_rx_gui_debug, debug_gui_tx), "GUI (Windows, debug)").await? } },
            { let client = client.clone(); let repo = repo.clone(); async move { cli.run(BuildGui::new(false, client, repo, updater_rx, release_rx_gui, gui_tx), "GUI (Windows)").await? } },
            { let client = client.clone(); let repo = repo.clone(); async move { cli.run(BuildGuiLinux::new(client, repo, release_rx_gui_linux, linux_gui_tx), "GUI (Linux)").await? } },
            { let client = client.clone(); let repo = repo.clone(); async move { cli.run(BuildBizHawk::new(true, client, repo, debug_gui_rx, release_rx_bizhawk_debug, bizhawk_version_debug, debug_bizhawk_tx), "BizHawk (Windows, debug)").await? } },
            { let client = client.clone(); let repo = repo.clone(); async move { cli.run(BuildBizHawk::new(false, client, repo, gui_rx, release_rx_bizhawk, bizhawk_version, bizhawk_tx), "BizHawk (Windows)").await? } },
            { let client = client.clone(); let repo = repo.clone(); async move { cli.run(BuildBizHawkLinux::new(client, repo, linux_gui_rx, release_rx_bizhawk_linux, bizhawk_version_linux, linux_bizhawk_tx), "BizHawk (Linux)").await? } },
            { let client = client.clone(); let repo = repo.clone(); async move { cli.run(BuildPj64::new(client, repo, release_rx_pj64), "Project64").await? } },
            { let client = client.clone(); let repo = repo.clone(); async move { cli.run(BuildInstaller::new(true, client, repo, debug_bizhawk_rx, debug_gui_rx_installer, release_rx_installer_debug), "installer (Windows, debug)").await? } },
            { let client = client.clone(); let repo = repo.clone(); async move { cli.run(BuildInstaller::new(false, client, repo, bizhawk_rx, gui_rx_installer, release_rx_installer), "installer (Windows)").await? } },
            { let client = client.clone(); let repo = repo.clone(); async move { cli.run(BuildInstallerLinux::new(client, repo, linux_bizhawk_rx, release_rx_installer_linux), "installer (Linux)").await? } },
            if args.no_server { future::ok(()).boxed() } else { async move { cli.run(BuildServer::new(!args.force), "server").await? }.boxed() },
        ]?;
        if !args.no_publish {
            let line = cli.new_line("publishing release").await?;
            repo.publish_release(&client, release).await?;
            line.replace("release published").await?;
            let line = cli.new_line("relabelling issues").await?;
            let mut token = github_app_auth::InstallationAccessToken::new(github_app_auth::GithubAuthParams {
                user_agent: concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")).to_owned(),
                private_key: include_bytes!("../../../assets/github-private-key.pem").to_vec(),
                installation_id: 40480009,
                app_id: 371733,
            }).await?;
            let issues = repo.issues_with_label(&client, &mut token, "status: pending release").await?;
            for issue in &issues {
                let mut labels = issue.labels.iter().map(|multiworld::github::Label { name }| name.clone()).collect_vec();
                labels.retain(|label| label != "status: pending release");
                labels.push(format!("status: released"));
                issue.set_labels(&client, &mut token, &repo, &labels).await?;
            }
            line.replace(format!("{} issues relabelled", issues.len())).await?;
        }
    }
    Ok(())
}

#[wheel::main(custom_exit)]
async fn main(args: CliArgs) -> Result<(), Error> {
    let cli = Cli::new()?;
    let res = cli_main(&cli, Args::from(args)).await;
    drop(cli);
    res
}
