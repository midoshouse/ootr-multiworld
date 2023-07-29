#![deny(rust_2018_idioms, unused, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]
#![forbid(unsafe_code)]

use {
    std::{
        cmp::Ordering::*,
        env,
        fmt,
        io::{
            Cursor,
            prelude::*,
        },
        path::Path,
        process,
        time::Duration,
    },
    async_trait::async_trait,
    dir_lock::DirLock,
    futures::future::{
        self,
        FutureExt as _,
        TryFutureExt as _,
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

#[derive(Clone)] struct WindowsUpdaterNotification;
#[derive(Clone)] struct WindowsGuiNotification;
#[derive(Clone)] struct LinuxBizHawkNotification;
#[derive(Clone)] struct WindowsBizHawkNotification;

enum Setup {
    CreateReqwestClient,
    CheckVersion(reqwest::Client),
    CheckBizHawkVersion(reqwest::Client, Repo, bool),
    LockRust(reqwest::Client, Repo, Version, bool),
    UpdateRust(reqwest::Client, Repo, Version, DirLock, bool),
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
impl Task<Result<(reqwest::Client, Repo, Version, bool), Error>> for Setup {
    async fn run(self) -> Result<Result<(reqwest::Client, Repo, Version, bool), Error>, Self> {
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
                let is_major = if let Some(latest_release) = repo.latest_release(&client).await? {
                    let local_version = version::version().await;
                    let remote_version = latest_release.version()?;
                    match local_version.cmp(&remote_version) {
                        Less => return Err(Error::VersionRegression),
                        Equal => return Err(Error::SameVersion),
                        Greater => local_version.major > remote_version.major,
                    }
                } else {
                    true
                };
                Ok(Err(Self::CheckBizHawkVersion(client, repo, is_major)))
            }).await,
            Self::CheckBizHawkVersion(client, repo, is_major) => gres::transpose(async move {
                let [major, minor, patch, _] = multiworld_bizhawk::bizhawk_version();
                let local_version = Version::new(major.into(), minor.into(), patch.into());
                let remote_version = version::bizhawk_latest(&client).await?;
                match local_version.cmp(&remote_version) {
                    Less => return Err(Error::BizHawkOutdated { local: local_version, latest: remote_version }),
                    Equal => {}
                    Greater => return Err(Error::BizHawkVersionRegression),
                }
                Ok(Err(Self::LockRust(client, repo, local_version, is_major)))
            }).await,
            Self::LockRust(client, repo, local_version, is_major) => gres::transpose(async move {
                let lock_dir = Path::new(&env::var_os("TEMP").ok_or(Error::MissingEnvar("TEMP"))?).join("syncbin-startup-rust.lock");
                let lock = DirLock::new(&lock_dir).await?;
                Ok(Err(Self::UpdateRust(client, repo, local_version, lock, is_major))) //TODO update rustup first?
            }).await,
            Self::UpdateRust(client, repo, local_version, lock, is_major) => gres::transpose(async move {
                Command::new("rustup").arg("update").arg("stable").check("rustup").await?;
                lock.drop_async().await?;
                Ok(Ok((client, repo, local_version, is_major)))
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
            Self::Gui(..) => write!(f, "waiting for GUI build to finish"),
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
    Sync(reqwest::Client, Repo, broadcast::Receiver<Release>, Version, broadcast::Sender<LinuxBizHawkNotification>),
    Updater(reqwest::Client, Repo, broadcast::Receiver<Release>, Version, broadcast::Sender<LinuxBizHawkNotification>),
    Gui(reqwest::Client, Repo, broadcast::Receiver<Release>, Version, broadcast::Sender<LinuxBizHawkNotification>),
    CSharp(reqwest::Client, Repo, broadcast::Receiver<Release>, Version, broadcast::Sender<LinuxBizHawkNotification>),
    BizHawk(reqwest::Client, Repo, broadcast::Receiver<Release>, Version, broadcast::Sender<LinuxBizHawkNotification>),
    Copy(reqwest::Client, Repo, broadcast::Receiver<Release>, Version),
    Zip(reqwest::Client, Repo, broadcast::Receiver<Release>, Version),
    WaitRelease(reqwest::Client, Repo, broadcast::Receiver<Release>, Vec<u8>),
    Upload(reqwest::Client, Repo, Release, Vec<u8>),
}

impl BuildBizHawkLinux {
    fn new(client: reqwest::Client, repo: Repo, release_rx: broadcast::Receiver<Release>, version: Version, bizhawk_tx: broadcast::Sender<LinuxBizHawkNotification>) -> Self {
        Self::Sync(client, repo, release_rx, version, bizhawk_tx)
    }
}

impl fmt::Display for BuildBizHawkLinux {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sync(..) => write!(f, "syncing repo to Ubuntu"),
            Self::Updater(..) => write!(f, "building multiworld-updater for Linux"),
            Self::Gui(..) => write!(f, "building multiworld-gui for Linux"),
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
            Self::Sync(..) => 0,
            Self::Updater(..) => 1,
            Self::Gui(..) => 2,
            Self::CSharp(..) => 3,
            Self::BizHawk(..) => 4,
            Self::Copy(..) => 5,
            Self::Zip(..) => 6,
            Self::WaitRelease(..) => 7,
            Self::Upload(..) => 8,
        }, 9)
    }
}

#[async_trait]
impl Task<Result<(), Error>> for BuildBizHawkLinux {
    async fn run(self) -> Result<Result<(), Error>, Self> {
        match self {
            Self::Sync(client, repo, release_rx, version, bizhawk_tx) => gres::transpose(async move {
                Command::new("wsl").arg("rsync").arg("--delete").arg("-av").arg("/mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/").arg("/home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/").arg("--exclude").arg(".cargo/config.toml").arg("--exclude").arg("target").arg("--exclude").arg("crate/multiworld-bizhawk/OotrMultiworld/BizHawk").arg("--exclude").arg("crate/multiworld-bizhawk/OotrMultiworld/src/bin").arg("--exclude").arg("crate/multiworld-bizhawk/OotrMultiworld/src/obj").arg("--exclude").arg("crate/multiworld-bizhawk/OotrMultiworld/src/multiworld.dll").check("wsl rsync").await?;
                Ok(Err(Self::Updater(client, repo, release_rx, version, bizhawk_tx)))
            }).await,
            Self::Updater(client, repo, release_rx, version, bizhawk_tx) => gres::transpose(async move {
                Command::new("wsl").arg("env").arg("-C").arg("/home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld").arg("cargo").arg("build").arg("--release").arg("--package=multiworld-updater").check("wsl cargo build --package=multiworld-updater").await?;
                Ok(Err(Self::Gui(client, repo, release_rx, version, bizhawk_tx)))
            }).await,
            Self::Gui(client, repo, release_rx, version, bizhawk_tx) => gres::transpose(async move {
                Command::new("wsl").arg("env").arg("-C").arg("/home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld").arg("cargo").arg("build").arg("--release").arg("--package=multiworld-gui").check("wsl cargo build --package=multiworld-gui").await?;
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
    Sync(bool, bool),
    Build(bool, bool),
    Copy(bool, bool),
    Upload(bool, bool),
    WaitRestart(bool),
    Stop,
    UpdateRepo,
    Replace,
    Start,
}

impl BuildServer {
    fn new(wait_restart: bool, is_major: bool) -> Self {
        Self::Sync(wait_restart, is_major)
    }
}

impl fmt::Display for BuildServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sync(..) => write!(f, "syncing repo to Debian"),
            Self::Build(..) => write!(f, "building ootrmwd"),
            Self::Copy(..) => write!(f, "copying ootrmwd to Windows"),
            Self::Upload(..) => write!(f, "uploading ootrmwd to Mido's House"),
            Self::WaitRestart(false) => write!(f, "waiting for rooms to be inactive"),
            Self::WaitRestart(true) => write!(f, "waiting for rooms to be empty"),
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
            Self::Copy(..) => 80,
            Self::Upload(..) => 85,
            Self::WaitRestart(..) => 90,
            Self::Stop => 95,
            Self::UpdateRepo => 96,
            Self::Replace => 98,
            Self::Start => 99,
        })
    }
}

#[async_trait]
impl Task<Result<(), Error>> for BuildServer {
    async fn run(self) -> Result<Result<(), Error>, Self> {
        match self {
            Self::Sync(wait_restart, is_major) => gres::transpose(async move {
                Command::new("debian").arg("run").arg("rsync").arg("--delete").arg("-av").arg("/mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/").arg("/home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/").arg("--exclude").arg(".cargo/config.toml").arg("--exclude").arg("target").arg("--exclude").arg("crate/multiworld-bizhawk/OotrMultiworld/BizHawk").arg("--exclude").arg("crate/multiworld-bizhawk/OotrMultiworld/src/bin").arg("--exclude").arg("crate/multiworld-bizhawk/OotrMultiworld/src/obj").arg("--exclude").arg("crate/multiworld-bizhawk/OotrMultiworld/src/multiworld.dll").check("debian run rsync").await?;
                Ok(Err(Self::Build(wait_restart, is_major)))
            }).await,
            Self::Build(wait_restart, is_major) => gres::transpose(async move {
                Command::new("debian").arg("run").arg("env").arg("-C").arg("/home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld").arg("/home/fenhl/.cargo/bin/cargo").arg("build").arg("--release").arg("--package=ootrmwd").check("debian run cargo build --package=ootrmwd").await?;
                Ok(Err(Self::Copy(wait_restart, is_major)))
            }).await,
            Self::Copy(wait_restart, is_major) => gres::transpose(async move {
                fs::create_dir_all("target/wsl/release").await?;
                Command::new("debian").arg("run").arg("cp").arg("/home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/target/release/ootrmwd").arg("/mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/target/wsl/release/ootrmwd").check("debian run cp").await?;
                Ok(Err(Self::Upload(wait_restart, is_major)))
            }).await,
            Self::Upload(wait_restart, is_major) => gres::transpose(async move {
                Command::new("scp").arg("target/wsl/release/ootrmwd").arg("midos.house:bin/ootrmwd-next").check("scp").await?;
                Ok(Err(if wait_restart { Self::WaitRestart(is_major) } else { Self::Stop }))
            }).await,
            Self::WaitRestart(is_major) => gres::transpose(async move {
                if is_major {
                    Command::new("ssh").arg("midos.house").arg("if systemctl is-active ootrmw; then sudo -u mido /opt/git/github.com/midoshouse/ootr-multiworld/master/target/release/ootrmwd wait-until-empty; fi").check("ssh midos.house ootrmwd wait-until-empty").await?;
                    //TODO continue normally if this fails because the server is stopped
                } else {
                    Command::new("ssh").arg("midos.house").arg("if systemctl is-active ootrmw; then sudo -u mido /opt/git/github.com/midoshouse/ootr-multiworld/master/target/release/ootrmwd wait-until-inactive; fi").check("ssh midos.house ootrmwd wait-until-empty").await?;
                    //TODO show output
                    //TODO continue normally if this fails because the server is stopped
                }
                Ok(Err(Self::Stop))
            }).await,
            Self::Stop => gres::transpose(async move {
                Command::new("ssh").arg("midos.house").arg("sudo systemctl stop ootrmw").check("ssh midos.house systemctl stop ootrmw").await?;
                Ok(Err(Self::UpdateRepo))
            }).await,
            Self::UpdateRepo => gres::transpose(async move {
                Command::new("ssh").arg("midos.house").arg("cd /opt/git/github.com/midoshouse/ootr-multiworld/master && git pull --ff-only").check("ssh midos.house git pull").await?;
                Ok(Err(Self::Replace))
            }).await,
            Self::Replace => gres::transpose(async move {
                Command::new("ssh").arg("midos.house").arg("mv bin/ootrmwd-next bin/ootrmwd").check("ssh midos.house mv").await?;
                Command::new("ssh").arg("midos.house").arg("chmod +x bin/ootrmwd").check("ssh midos.house chmod").await?;
                Ok(Err(Self::Start))
            }).await,
            Self::Start => gres::transpose(async move {
                Command::new("ssh").arg("midos.house").arg("sudo systemctl start ootrmw").check("ssh midos.house systemctl start ootrmw").await?;
                Ok(Ok(()))
            }).await,
        }
    }
}

#[derive(Clone, clap::Parser)]
#[clap(version)]
struct Args {
    /// Don't wait for the server to be inactive before restarting it
    #[clap(long, conflicts_with("no_server"))]
    force: bool,
    /// Create the GitHub release as a draft
    #[clap(short = 'P', long)]
    no_publish: bool,
    /// Don't update the server
    #[clap(short = 'S', long)]
    no_server: bool,
    /// Don't pass `--wait` to the release notes editor
    #[clap(short = 'W', long)]
    no_wait: bool,
    /// Only update the server
    #[clap(short, long, conflicts_with("no_publish"), conflicts_with("no_server"), conflicts_with("no_wait"), conflicts_with("release_notes_editor"))]
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
    let (client, repo, bizhawk_version, is_major) = cli.run(Setup::default(), "pre-release checks passed").await??; // don't show release notes editor if version check could still fail
    if args.server_only {
        cli.run(BuildServer::new(!args.force, is_major), "server build done").await??;
    } else {
        let bizhawk_version_debug = bizhawk_version.clone();
        let bizhawk_version_linux = bizhawk_version.clone();
        let (release_tx, release_rx_installer_debug) = broadcast::channel(1);
        let release_rx_installer = release_tx.subscribe();
        let release_rx_installer_linux = release_tx.subscribe();
        let release_rx_gui_debug = release_tx.subscribe();
        let release_rx_gui = release_tx.subscribe();
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
            release = cli.run(CreateRelease::new(create_release_repo, create_release_client, release_tx, create_release_args), "release created").map_err(Error::Io),
            async move { cli.run(BuildUpdater::new(true, debug_updater_tx), "debug updater build done").await? },
            async move { cli.run(BuildUpdater::new(false, updater_tx), "updater build done").await? },
            { let client = client.clone(); let repo = repo.clone(); async move { cli.run(BuildGui::new(true, client, repo, debug_updater_rx, release_rx_gui_debug, debug_gui_tx), "Windows debug GUI build done").await? } },
            { let client = client.clone(); let repo = repo.clone(); async move { cli.run(BuildGui::new(false, client, repo, updater_rx, release_rx_gui, gui_tx), "Windows GUI build done").await? } },
            { let client = client.clone(); let repo = repo.clone(); async move { cli.run(BuildBizHawk::new(true, client, repo, debug_gui_rx, release_rx_bizhawk_debug, bizhawk_version_debug, debug_bizhawk_tx), "Windows debug BizHawk build done").await? } },
            { let client = client.clone(); let repo = repo.clone(); async move { cli.run(BuildBizHawk::new(false, client, repo, gui_rx, release_rx_bizhawk, bizhawk_version, bizhawk_tx), "Windows BizHawk build done").await? } },
            { let client = client.clone(); let repo = repo.clone(); async move { cli.run(BuildBizHawkLinux::new(client, repo, release_rx_bizhawk_linux, bizhawk_version_linux, linux_bizhawk_tx), "Linux BizHawk build done").await? } },
            { let client = client.clone(); let repo = repo.clone(); async move { cli.run(BuildPj64::new(client, repo, release_rx_pj64), "Project64 build done").await? } },
            { let client = client.clone(); let repo = repo.clone(); async move { cli.run(BuildInstaller::new(true, client, repo, debug_bizhawk_rx, debug_gui_rx_installer, release_rx_installer_debug), "Windows debug installer build done").await? } },
            { let client = client.clone(); let repo = repo.clone(); async move { cli.run(BuildInstaller::new(false, client, repo, bizhawk_rx, gui_rx_installer, release_rx_installer), "Windows installer build done").await? } },
            { let client = client.clone(); let repo = repo.clone(); async move { cli.run(BuildInstallerLinux::new(client, repo, linux_bizhawk_rx, release_rx_installer_linux), "Linux installer build done").await? } },
            if args.no_server { future::ok(()).boxed() } else { async move { cli.run(BuildServer::new(!args.force, is_major), "server build done").await? }.boxed() },
        ]?;
        if !args.no_publish {
            let line = cli.new_line("[....] publishing release").await?;
            repo.publish_release(&client, release).await?;
            line.replace("[done] release published")?;
        }
    }
    Ok(())
}

#[wheel::main(custom_exit)]
async fn main(args: Args) -> Result<(), Error> {
    let cli = Cli::new()?;
    let res = cli_main(&cli, args).await;
    drop(cli);
    res
}
