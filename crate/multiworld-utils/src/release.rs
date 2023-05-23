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
                    if env::var("TERM_PROGRAM").as_deref() == Ok("vscode") {
                        cmd = Command::new("code.cmd");
                        if !args.no_wait {
                            cmd.arg("--wait");
                        }
                        ("code", None)
                    } else if env::var_os("STY").is_none() && env::var_os("SSH_CLIENT").is_none() && env::var_os("SSH_TTY").is_none() {
                        cmd = Command::new("C:\\Program Files\\Microsoft VS Code\\bin\\code.cmd");
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
    Glow(broadcast::Sender<WindowsUpdaterNotification>),
}

impl BuildUpdater {
    fn new(notifier: broadcast::Sender<WindowsUpdaterNotification>) -> Self {
        Self::Glow(notifier)
    }
}

impl fmt::Display for BuildUpdater {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Glow(..) => write!(f, "building multiworld-updater.exe"),
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
            Self::Glow(notifier) => gres::transpose(async move {
                Command::new("cargo").arg("build").arg("--release").arg("--package=multiworld-updater").check("cargo build --package=multiworld-updater").await?;
                let _ = notifier.send(WindowsUpdaterNotification);
                Ok(Ok(()))
            }).await,
        }
    }
}

enum BuildGui {
    Updater(reqwest::Client, Repo, broadcast::Receiver<WindowsUpdaterNotification>, broadcast::Receiver<Release>, broadcast::Sender<WindowsGuiNotification>),
    Glow(reqwest::Client, Repo, broadcast::Receiver<Release>, broadcast::Sender<WindowsGuiNotification>),
    Read(reqwest::Client, Repo, broadcast::Receiver<Release>),
    WaitRelease(reqwest::Client, Repo, broadcast::Receiver<Release>, Vec<u8>),
    Upload(reqwest::Client, Repo, Release, Vec<u8>),
}

impl BuildGui {
    fn new(client: reqwest::Client, repo: Repo, updater_rx: broadcast::Receiver<WindowsUpdaterNotification>, release_rx: broadcast::Receiver<Release>, gui_tx: broadcast::Sender<WindowsGuiNotification>) -> Self {
        Self::Updater(client, repo, updater_rx, release_rx, gui_tx)
    }
}

impl fmt::Display for BuildGui {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Updater(..) => write!(f, "waiting for updater build to finish"),
            Self::Glow(..) => write!(f, "building multiworld-gui.exe"),
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
            Self::Updater(client, repo, mut updater_rx, release_rx, gui_tx) => gres::transpose(async move {
                let WindowsUpdaterNotification = updater_rx.recv().await?;
                Ok(Err(Self::Glow(client, repo, release_rx, gui_tx)))
            }).await,
            Self::Glow(client, repo, release_rx, gui_tx) => gres::transpose(async move {
                Command::new("cargo").arg("build").arg("--release").arg("--package=multiworld-gui").check("cargo build --package=multiworld-gui").await?;
                let _ = gui_tx.send(WindowsGuiNotification);
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
    Gui(reqwest::Client, Repo, broadcast::Receiver<WindowsGuiNotification>, broadcast::Receiver<Release>, Version, broadcast::Sender<WindowsBizHawkNotification>),
    CSharp(reqwest::Client, Repo, broadcast::Receiver<Release>, Version, broadcast::Sender<WindowsBizHawkNotification>),
    BizHawk(reqwest::Client, Repo, broadcast::Receiver<Release>, Version, broadcast::Sender<WindowsBizHawkNotification>),
    Zip(reqwest::Client, Repo, broadcast::Receiver<Release>, Version),
    WaitRelease(reqwest::Client, Repo, broadcast::Receiver<Release>, Vec<u8>),
    Upload(reqwest::Client, Repo, Release, Vec<u8>),
}

impl BuildBizHawk {
    fn new(client: reqwest::Client, repo: Repo, gui_rx: broadcast::Receiver<WindowsGuiNotification>, release_rx: broadcast::Receiver<Release>, version: Version, bizhawk_tx: broadcast::Sender<WindowsBizHawkNotification>) -> Self {
        Self::Gui(client, repo, gui_rx, release_rx, version, bizhawk_tx)
    }
}

impl fmt::Display for BuildBizHawk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gui(..) => write!(f, "waiting for GUI build to finish"),
            Self::CSharp(..) => write!(f, "building multiworld-csharp for Windows"),
            Self::BizHawk(..) => write!(f, "building multiworld-bizhawk for Windows"),
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
                let WindowsGuiNotification = gui_rx.recv().await?;
                Ok(Err(Self::CSharp(client, repo, release_rx, version, bizhawk_tx)))
            }).await,
            Self::CSharp(client, repo, release_rx, version, bizhawk_tx) => gres::transpose(async move {
                Command::new("cargo").arg("build").arg("--release").arg("--package=multiworld-csharp").check("cargo build --package=multiworld-csharp").await?;
                Ok(Err(Self::BizHawk(client, repo, release_rx, version, bizhawk_tx)))
            }).await,
            Self::BizHawk(client, repo, release_rx, version, bizhawk_tx) => gres::transpose(async move {
                Command::new("cargo").arg("build").arg("--release").arg("--package=multiworld-bizhawk").check("cargo build --package=multiworld-bizhawk").await?;
                let _ = bizhawk_tx.send(WindowsBizHawkNotification);
                Ok(Err(Self::Zip(client, repo, release_rx, version)))
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
            Self::Sync(..) => write!(f, "syncing repo to Linux"),
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
    Deps(reqwest::Client, Repo, broadcast::Receiver<WindowsBizHawkNotification>, broadcast::Receiver<WindowsGuiNotification>, broadcast::Receiver<Release>),
    Glow(reqwest::Client, Repo, broadcast::Receiver<Release>),
    Read(reqwest::Client, Repo, broadcast::Receiver<Release>),
    WaitRelease(reqwest::Client, Repo, broadcast::Receiver<Release>, Vec<u8>),
    Upload(reqwest::Client, Repo, Release, Vec<u8>),
}

impl BuildInstaller {
    fn new(client: reqwest::Client, repo: Repo, bizhawk_rx: broadcast::Receiver<WindowsBizHawkNotification>, gui_rx: broadcast::Receiver<WindowsGuiNotification>, release_rx: broadcast::Receiver<Release>) -> Self {
        Self::Deps(client, repo, bizhawk_rx, gui_rx, release_rx)
    }
}

impl fmt::Display for BuildInstaller {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Deps(..) => write!(f, "waiting for dependency builds to finish"),
            Self::Glow(..) => write!(f, "building multiworld-installer.exe"),
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
            Self::Deps(client, repo, mut bizhawk_rx, mut gui_rx, release_rx) => gres::transpose(async move {
                let WindowsBizHawkNotification = bizhawk_rx.recv().await?;
                let WindowsGuiNotification = gui_rx.recv().await?;
                Ok(Err(Self::Glow(client, repo, release_rx)))
            }).await,
            Self::Glow(client, repo, release_rx) => gres::transpose(async move {
                Command::new("cargo").arg("build").arg("--release").arg("--package=multiworld-installer").check("cargo build --package=multiworld-installer").await?;
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
    UpdateRepo(bool),
    Build(bool),
    WaitRestart(bool),
    Restart,
}

impl BuildServer {
    fn new(is_major: bool) -> Self {
        Self::UpdateRepo(is_major)
    }
}

impl fmt::Display for BuildServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UpdateRepo(..) => write!(f, "updating repo on Mido's House"),
            Self::Build(..) => write!(f, "building ootrmwd"),
            Self::WaitRestart(false) => write!(f, "waiting for rooms to be inactive"),
            Self::WaitRestart(true) => write!(f, "waiting for rooms to be empty"),
            Self::Restart => write!(f, "restarting ootrmwd"),
        }
    }
}

impl Progress for BuildServer {
    fn progress(&self) -> Percent {
        Percent::new(match self {
            Self::UpdateRepo(..) => 0,
            Self::Build(..) => 10,
            Self::WaitRestart(..) => 90,
            Self::Restart => 95,
        })
    }
}

#[async_trait]
impl Task<Result<(), Error>> for BuildServer {
    async fn run(self) -> Result<Result<(), Error>, Self> {
        match self {
            Self::UpdateRepo(is_major) => gres::transpose(async move {
                Command::new("ssh").arg("midos.house").arg("cd /opt/git/github.com/midoshouse/ootr-multiworld/master && git pull --ff-only").check("ssh midos.house git pull").await?;
                Ok(Err(Self::Build(is_major)))
            }).await,
            Self::Build(is_major) => gres::transpose(async move {
                //TODO build locally
                Command::new("ssh").arg("midos.house").arg("cd /opt/git/github.com/midoshouse/ootr-multiworld/master && cargo build --release --package=ootrmwd").check("ssh midos.house cargo build").await?;
                Ok(Err(Self::WaitRestart(is_major)))
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
                Ok(Err(Self::Restart))
            }).await,
            Self::Restart => gres::transpose(async move {
                Command::new("ssh").arg("midos.house").arg("sudo systemctl restart ootrmw").check("ssh midos.house systemctl restart ootrmw").await?;
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
        cli.run(BuildServer::new(false), "server build done").await??;
    } else {
        let create_release_cli = Arc::clone(&cli);
        let release_notes_cli = Arc::clone(&cli);
        let (client, repo, bizhawk_version, is_major) = cli.run(Setup::default(), "pre-release checks passed").await??; // don't show release notes editor if version check could still fail
        let bizhawk_version_linux = bizhawk_version.clone();
        let (release_tx, release_rx_installer) = broadcast::channel(1);
        let release_rx_installer_linux = release_tx.subscribe();
        let release_rx_gui = release_tx.subscribe();
        let release_rx_bizhawk = release_tx.subscribe();
        let release_rx_bizhawk_linux = release_tx.subscribe();
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
        let (linux_bizhawk_tx, linux_bizhawk_rx) = broadcast::channel(1);

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
            { let cli = Arc::clone(&cli); let client = client.clone(); let repo = repo.clone(); async move { tokio::spawn(async move { cli.run(BuildGui::new(client, repo, updater_rx, release_rx_gui, gui_tx), "Windows GUI build done").await? }).await? } },
            { let cli = Arc::clone(&cli); let client = client.clone(); let repo = repo.clone(); async move { tokio::spawn(async move { cli.run(BuildBizHawk::new(client, repo, gui_rx, release_rx_bizhawk, bizhawk_version, bizhawk_tx), "Windows BizHawk build done").await? }).await? } },
            { let cli = Arc::clone(&cli); let client = client.clone(); let repo = repo.clone(); async move { tokio::spawn(async move { cli.run(BuildBizHawkLinux::new(client, repo, release_rx_bizhawk_linux, bizhawk_version_linux, linux_bizhawk_tx), "Linux BizHawk build done").await? }).await? } },
            { let cli = Arc::clone(&cli); let client = client.clone(); let repo = repo.clone(); async move { tokio::spawn(async move { cli.run(BuildPj64::new(client, repo, release_rx_pj64), "Project64 build done").await? }).await? } },
            { let cli = Arc::clone(&cli); let client = client.clone(); let repo = repo.clone(); async move { tokio::spawn(async move { cli.run(BuildInstaller::new(client, repo, bizhawk_rx, gui_rx_installer, release_rx_installer), "Windows installer build done").await? }).await? } },
            { let cli = Arc::clone(&cli); let client = client.clone(); let repo = repo.clone(); async move { tokio::spawn(async move { cli.run(BuildInstallerLinux::new(client, repo, linux_bizhawk_rx, release_rx_installer_linux), "Linux installer build done").await? }).await? } },
            if args.no_server { future::ok(()).boxed() } else { let cli = Arc::clone(&cli); async move { tokio::spawn(async move { cli.run(BuildServer::new(is_major), "server build done").await? }).await? }.boxed() },
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
