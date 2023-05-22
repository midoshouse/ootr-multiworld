#![deny(rust_2018_idioms, unused_crate_dependencies, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]
#![cfg_attr(target_os = "windows", deny(unused))]
#![cfg_attr(not(target_os = "windows"), allow(unused))]
#![forbid(unsafe_code)]

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use {
    std::{
        borrow::Cow,
        collections::BTreeMap,
        env,
        fmt,
        path::{
            Path,
            PathBuf,
        },
        sync::Arc,
    },
    dark_light::Mode::*,
    directories::UserDirs,
    enum_iterator::{
        Sequence,
        all,
    },
    futures::future::{
        self,
        Future,
    },
    iced::{
        Application,
        Command,
        Element,
        Length,
        Settings,
        Theme,
        clipboard,
        widget::*,
        window::{
            self,
            icon,
        },
    },
    ::image::ImageFormat,
    itertools::Itertools as _,
    lazy_regex::regex_is_match,
    open::that as open,
    rfd::AsyncFileDialog,
    semver::Version,
    serde::{
        Deserialize,
        Serialize,
    },
    serenity::utils::MessageBuilder,
    tokio::io,
    url::Url,
    wheel::{
        fs,
        traits::IoResultExt as _,
    },
    multiworld::github::Repo,
};
#[cfg(target_os = "linux")] use std::io::Cursor;
#[cfg(target_os = "windows")] use {
    std::cmp::Ordering::*,
    futures::stream::TryStreamExt as _,
    is_elevated::is_elevated,
    kuchiki::traits::TendrilSink as _,
    tokio::io::AsyncWriteExt as _,
    tokio_util::io::StreamReader,
    wheel::traits::{
        AsyncCommandOutputExt as _,
        SyncCommandOutputExt as _,
    },
    multiworld::config::CONFIG,
};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))] const BIZHAWK_PLATFORM_SUFFIX: &str = "-linux-x64.tar.gz";
#[cfg(all(target_os = "windows", target_arch = "x86_64"))] const BIZHAWK_PLATFORM_SUFFIX: &str = "-win-x64.zip";

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] Config(#[from] multiworld::config::Error),
    #[error(transparent)] IniDe(#[from] serde_ini::de::Error),
    #[error(transparent)] IniSer(#[from] serde_ini::ser::Error),
    #[error(transparent)] Io(#[from] io::Error),
    #[error(transparent)] Reqwest(#[from] reqwest::Error),
    #[error(transparent)] Task(#[from] tokio::task::JoinError),
    #[error(transparent)] Url(#[from] url::ParseError),
    #[error(transparent)] Wheel(#[from] wheel::Error),
    #[cfg(target_os = "windows")] #[error(transparent)] Winver(#[from] winver::Error),
    #[error(transparent)] Zip(#[from] async_zip::error::ZipError),
    #[cfg(target_os = "windows")]
    #[error("The installer requires an older version of BizHawk. Install manually at your own risk, or ask Fenhl to release a new version.")]
    BizHawkVersionRegression,
    #[error("tried to copy debug info or open a GitHub issue with no active error")]
    CopyDebugInfo,
    #[error("got zero elements when exactly one was expected")]
    ExactlyOneEmpty,
    #[error("got at least 2 elements when exactly one was expected")]
    ExactlyOneMultiple,
    #[error("latest release does not have a download for this platform")]
    MissingBizHawkAsset,
    #[error("no BizHawk releases found")]
    NoBizHawkReleases,
    #[error("non-UTF-8 paths are currently not supported")]
    NonUtf8Path,
    #[cfg(target_os = "windows")]
    #[error("Mido's House Multiworld requires at least version 2.4 of Project64")]
    OutdatedProject64,
    #[cfg(target_os = "windows")]
    #[error("failed to parse Project64 website")]
    ParsePj64Html,
    #[cfg(target_os = "windows")]
    #[error("Project64 version too new, please tell Fenhl that Mido's House Multiworld needs to be updated")]
    Project64TooNew,
    #[cfg(target_os = "windows")]
    #[error("can't install to the filesystem root")]
    Root,
}

impl Error {
    fn to_markdown(&self) -> String {
        MessageBuilder::default()
            .push_line(concat!("error in ", env!("CARGO_PKG_NAME"), " version ", env!("CARGO_PKG_VERSION"), ":"))
            .push_line_safe(self)
            .push_codeblock_safe(format!("{self:?}"), Some("rust"))
            .build()
    }
}

impl<I: Iterator> From<itertools::ExactlyOneError<I>> for Error {
    fn from(mut e: itertools::ExactlyOneError<I>) -> Self {
        if e.next().is_some() {
            Self::ExactlyOneMultiple
        } else {
            Self::ExactlyOneEmpty
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    Back,
    BrowseEmulatorPath,
    BrowseMultiworldPath,
    ConfigWriteFailed,
    Continue,
    CopyDebugInfo,
    DiscordChannel,
    DiscordInvite,
    EmulatorPath(String),
    Error(Arc<Error>),
    Exit,
    InstallMultiworld,
    LocateMultiworld(Option<u16>),
    MultiworldInstalled,
    MultiworldPath(String),
    NewIssue,
    Nop,
    PlatformSupport,
    SetCreateDesktopShortcut(bool),
    SetEmulator(Emulator),
    SetInstallEmulator(bool),
    SetOpenEmulator(bool),
}

fn cmd(future: impl Future<Output = Result<Message, Error>> + Send + 'static) -> Command<Message> {
    Command::single(iced_native::command::Action::Future(Box::pin(async move {
        match future.await {
            Ok(msg) => msg,
            Err(e) => Message::Error(Arc::new(e)),
        }
    })))
}

#[derive(Default, Deserialize, Serialize)]
struct Pj64Config {
    #[serde(rename = "Settings", default)]
    settings: Pj64ConfigSettings,
    #[serde(rename = "Debugger", default)]
    debugger: Pj64ConfigDebugger,
    #[serde(flatten)]
    rest: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Default, Deserialize, Serialize)]
struct Pj64ConfigSettings {
    #[serde(rename = "Basic Mode", default)]
    basic_mode: u8,
    #[serde(flatten)]
    rest: BTreeMap<String, String>,
}

#[derive(Default, Deserialize, Serialize)]
struct Pj64ConfigDebugger {
    #[serde(rename = "Debugger", default)]
    debugger: u8,
    #[serde(flatten)]
    rest: BTreeMap<String, String>,
}

enum Page {
    Error(Arc<Error>, bool),
    Elevated,
    SelectEmulator {
        emulator: Option<Emulator>,
        install_emulator: Option<bool>,
        emulator_path: Option<String>,
        multiworld_path: Option<String>,
    },
    LocateEmulator {
        emulator: Emulator,
        install_emulator: bool,
        emulator_path: String,
        multiworld_path: Option<String>,
    },
    AskBizHawkUpdate {
        emulator_path: String,
        multiworld_path: Option<String>,
    },
    InstallEmulator {
        update: bool,
        emulator: Emulator,
        emulator_path: String,
        multiworld_path: Option<String>,
    },
    LocateMultiworld {
        emulator: Emulator,
        emulator_path: String,
        multiworld_path: String,
    },
    InstallMultiworld {
        emulator: Emulator,
        emulator_path: String,
        multiworld_path: Option<String>,
        config_write_failed: bool,
    },
    AskLaunch {
        emulator: Emulator,
        emulator_path: String,
        multiworld_path: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Sequence, clap::ValueEnum)]
#[clap(rename_all = "lower")]
enum Emulator {
    BizHawk,
    #[cfg(target_os = "windows")] Project64,
}

impl fmt::Display for Emulator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BizHawk => write!(f, "BizHawk"),
            #[cfg(target_os = "windows")] Self::Project64 => write!(f, "Project64"),
        }
    }
}

struct State {
    http_client: reqwest::Client,
    page: Page,
    // Page::LocateEmulator
    create_desktop_shortcut: bool,
    // Page::AskLaunch
    open_emulator: bool,
}

impl Application for State {
    type Executor = iced::executor::Default;
    type Message = Message;
    type Theme = Theme;
    type Flags = Args;

    fn new(Args { mut emulator }: Args) -> (Self, Command<Message>) {
        if let Ok(only_emulator) = all().exactly_one() {
            emulator.get_or_insert(only_emulator);
        }
        (Self {
            http_client: reqwest::Client::builder()
                .user_agent(concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")))
                .use_rustls_tls()
                .https_only(true)
                .build().expect("failed to build HTTP client"),
            page: Page::SelectEmulator {
                install_emulator: None,
                emulator_path: None,
                multiworld_path: None,
                emulator,
            },
            create_desktop_shortcut: true,
            open_emulator: true,
        }, if emulator.is_some() {
            cmd(future::ok(Message::Continue))
        } else {
            Command::none()
        })
    }

    fn theme(&self) -> Self::Theme {
        match dark_light::detect() { //TODO automatically update on system theme change
            Dark => Theme::Dark,
            Light | Default => Theme::Light,
        }
    }

    fn title(&self) -> String { format!("Mido's House Multiworld Installer") }

    fn update(&mut self, msg: Message) -> Command<Message> {
        match msg {
            Message::Back => self.page = match self.page {
                Page::Error(_, _) | Page::Elevated | Page::SelectEmulator { .. } => unreachable!(),
                Page::LocateEmulator { emulator, install_emulator, ref emulator_path, ref multiworld_path } => Page::SelectEmulator { emulator: Some(emulator), install_emulator: Some(install_emulator), emulator_path: Some(emulator_path.clone()), multiworld_path: multiworld_path.clone() },
                Page::InstallEmulator { .. } => unreachable!(),
                Page::AskBizHawkUpdate { ref emulator_path, ref multiworld_path } => Page::LocateEmulator { emulator: Emulator::BizHawk, install_emulator: false, emulator_path: emulator_path.clone(), multiworld_path: multiworld_path.clone() },
                Page::LocateMultiworld { emulator, ref emulator_path, ref multiworld_path } => Page::LocateEmulator { emulator, install_emulator: false, emulator_path: emulator_path.clone(), multiworld_path: Some(multiworld_path.clone()) },
                Page::InstallMultiworld { emulator, ref emulator_path, ref multiworld_path, .. } | Page::AskLaunch { emulator, ref emulator_path, ref multiworld_path } => match emulator {
                    Emulator::BizHawk => Page::LocateEmulator { emulator, install_emulator: false, emulator_path: emulator_path.clone(), multiworld_path: multiworld_path.clone() },
                    #[cfg(target_os = "windows")] Emulator::Project64 => if let Some(multiworld_path) = multiworld_path.clone() {
                        Page::LocateMultiworld { emulator, emulator_path: emulator_path.clone(), multiworld_path }
                    } else {
                        Page::LocateEmulator { emulator, install_emulator: false, emulator_path: emulator_path.clone(), multiworld_path: None }
                    },
                },
            },
            Message::BrowseEmulatorPath => if let Page::LocateEmulator { emulator, install_emulator, ref emulator_path, .. } = self.page {
                let current_path = emulator_path.clone();
                return cmd(async move {
                    Ok(if let Some(emulator_dir) = AsyncFileDialog::new().set_title(match (emulator, install_emulator) {
                        (Emulator::BizHawk, false) => "Select BizHawk Folder",
                        (Emulator::BizHawk, true) => "Choose Location for BizHawk Installation",
                        #[cfg(target_os = "windows")] (Emulator::Project64, false) => "Select Project64 Folder",
                        #[cfg(target_os = "windows")] (Emulator::Project64, true) => "Choose Location for Project64 Installation",
                    }).set_directory(Path::new(&current_path)).pick_folder().await {
                        Message::EmulatorPath(emulator_dir.path().to_str().ok_or(Error::NonUtf8Path)?.to_owned())
                    } else {
                        Message::Nop
                    })
                })
            }
            Message::BrowseMultiworldPath => if let Page::LocateMultiworld { ref multiworld_path, .. } = self.page {
                let current_path = Path::new(multiworld_path).parent().map(Path::to_owned);
                return cmd(async move {
                    let mut dialog = AsyncFileDialog::new();
                    dialog = dialog.set_title("Choose Location for Multiworld App");
                    if let Some(current_path) = current_path {
                        dialog = dialog.set_directory(&current_path);
                    }
                    dialog = dialog.set_file_name("Mido's House Multiworld for Project64.exe");
                    dialog = dialog.add_filter("Windows executable", &["exe"]);
                    Ok(if let Some(multiworld_path) = dialog.save_file().await {
                        Message::MultiworldPath(multiworld_path.path().to_str().ok_or(Error::NonUtf8Path)?.to_owned())
                    } else {
                        Message::Nop
                    })
                })
            }
            Message::ConfigWriteFailed => if let Page::InstallMultiworld { ref mut config_write_failed, .. } = self.page { *config_write_failed = true },
            Message::Continue => match self.page {
                Page::Error(_, _) | Page::Elevated => unreachable!(),
                Page::SelectEmulator { emulator, install_emulator, ref emulator_path, ref multiworld_path } => {
                    let emulator = emulator.expect("emulator must be selected to continue here");
                    #[cfg(target_os = "windows")] if matches!(emulator, Emulator::Project64) && !is_elevated() {
                        // Project64 installation and plugin installation both require admin permissions (UAC)
                        self.page = Page::Elevated;
                        return cmd(async move {
                            tokio::task::spawn_blocking(|| Ok::<_, Error>(runas::Command::new(env::current_exe()?).arg("--emulator=project64").gui(true).status().at_command("runas")?.check("runas")?)).await??;
                            Ok(Message::Exit)
                        })
                    }
                    let emulator_path = emulator_path.clone();
                    let (install_emulator, emulator_path) = match (install_emulator, emulator_path) {
                        (Some(install_emulator), Some(emulator_path)) => (install_emulator, emulator_path),
                        (_, _) => match emulator {
                            Emulator::BizHawk => if let Some(user_dirs) = UserDirs::new() {
                                // check for existing BizHawk install in Downloads folder (where the bizhawk-co-op install scripts places it)
                                let bizhawk_install_path = user_dirs.home_dir().join("bin").join("BizHawk");
                                if bizhawk_install_path.exists() {
                                    let Ok(bizhawk_install_path) = bizhawk_install_path.into_os_string().into_string() else { return cmd(future::err(Error::NonUtf8Path)) };
                                    (false, bizhawk_install_path)
                                } else if let Some(default_bizhawk_dir) = UserDirs::new()
                                    .and_then(|dirs| dirs.download_dir().map(|downloads| downloads.to_owned()))
                                    .and_then(|downloads| downloads.read_dir().ok())
                                    .into_iter()
                                    .flatten()
                                    .filter_map(|entry| entry.ok())
                                    .filter(|entry| entry.file_name().to_str().map_or(false, |filename| regex_is_match!(r"^BizHawk-[0-9]+(\.[0-9]+){2,3}$", filename)))
                                    .max_by_key(|entry| entry.file_name())
                                    .map(|entry| entry.path())
                                {
                                    let Ok(default_bizhawk_dir) = default_bizhawk_dir.into_os_string().into_string() else { return cmd(future::err(Error::NonUtf8Path)) };
                                    (false, default_bizhawk_dir)
                                } else {
                                    let Ok(bizhawk_install_path) = bizhawk_install_path.into_os_string().into_string() else { return cmd(future::err(Error::NonUtf8Path)) };
                                    (true, bizhawk_install_path)
                                }
                            } else {
                                (true, String::default())
                            },
                            #[cfg(target_os = "windows")] Emulator::Project64 => if let Some(pj64_install_path) = env::var_os("ProgramFiles(x86)").or_else(|| env::var_os("ProgramFiles")).map(|program_files| PathBuf::from(program_files).join("Project64 3.0")) {
                                let exists = pj64_install_path.exists();
                                let Ok(pj64_install_path) = pj64_install_path.into_os_string().into_string() else { return cmd(future::err(Error::NonUtf8Path)) };
                                (!exists, pj64_install_path)
                            } else {
                                (true, String::default())
                            },
                        },
                    };
                    self.page = Page::LocateEmulator { emulator, install_emulator, emulator_path, multiworld_path: multiworld_path.clone() };
                }
                Page::LocateEmulator { emulator, install_emulator, ref emulator_path, ref multiworld_path } => if install_emulator {
                    let emulator_path = emulator_path.clone();
                    self.page = Page::InstallEmulator { update: false, emulator, emulator_path: emulator_path.clone(), multiworld_path: multiworld_path.clone() };
                    match emulator {
                        Emulator::BizHawk => {
                            //TODO indicate progress
                            let http_client = self.http_client.clone();
                            let bizhawk_dir = PathBuf::from(emulator_path);
                            return cmd(async move {
                                fs::create_dir_all(&bizhawk_dir).await?;
                                #[cfg(target_os = "linux")] {
                                    //TODO check if apt-get exists and install prerequisite packages if it does? (mono-complete & libcanberra-gtk-module)
                                }
                                #[cfg(target_os = "windows")] {
                                    // install BizHawk-Prereqs
                                    let release = Repo::new("TASEmulators", "BizHawk-Prereqs").latest_release(&http_client).await?.ok_or(Error::NoBizHawkReleases)?;
                                    let asset = release.assets.into_iter()
                                        .filter(|asset| regex_is_match!(r"^bizhawk_prereqs_v.+\.zip$", &asset.name))
                                        .exactly_one()?;
                                    let response = http_client.get(asset.browser_download_url).send().await?.error_for_status()?.bytes().await?;
                                    let zip_file = async_zip::base::read::mem::ZipFileReader::new(response.into()).await?;
                                    let _ = zip_file.file().entries().iter().exactly_one()?;
                                    {
                                        let mut buf = Vec::default();
                                        zip_file.entry(0).await?.read_to_end_checked(&mut buf, zip_file.file().entries()[0].entry()).await?;
                                        let prereqs = tempfile::Builder::new().prefix("bizhawk_prereqs_").suffix(".exe").tempfile()?;
                                        tokio::fs::File::from_std(prereqs.reopen()?).write_all(&buf).await?;
                                        let prereqs_path = prereqs.into_temp_path();
                                        runas::Command::new(&prereqs_path).status().at_command("runas")?.check("BizHawk-Prereqs")?; //TODO show message in GUI saying to check the BizHawk-Prereqs GUI
                                    }
                                }
                                // install BizHawk itself
                                let version_str = include!(concat!(env!("OUT_DIR"), "/bizhawk_version.rs")).to_string();
                                let version_str = version_str.trim_end_matches(".0");
                                let release = Repo::new("TASEmulators", "BizHawk").release_by_tag(&http_client, version_str).await?.ok_or(Error::NoBizHawkReleases)?;
                                let asset = release.assets.into_iter()
                                    .filter(|asset| asset.name.ends_with(BIZHAWK_PLATFORM_SUFFIX))
                                    .exactly_one()?;
                                let response = http_client.get(asset.browser_download_url).send().await?.error_for_status()?.bytes().await?;
                                #[cfg(target_os = "linux")] {
                                    let tar_file = async_compression::tokio::bufread::GzipDecoder::new(Cursor::new(Vec::from(response)));
                                    tokio_tar::Archive::new(tar_file).unpack(bizhawk_dir).await?;
                                }
                                #[cfg(target_os = "windows")] {
                                    let zip_file = async_zip::base::read::mem::ZipFileReader::new(response.into()).await?;
                                    let entries = zip_file.file().entries().iter().enumerate().map(|(idx, entry)| (idx, entry.entry().filename().ends_with('/'), bizhawk_dir.join(entry.entry().filename()))).collect_vec();
                                    for (idx, is_dir, path) in entries {
                                        if is_dir {
                                            fs::create_dir_all(path).await?;
                                        } else {
                                            if let Some(parent) = path.parent() {
                                                fs::create_dir_all(parent).await?;
                                            }
                                            let mut buf = Vec::default();
                                            zip_file.entry(idx).await?.read_to_end_checked(&mut buf, zip_file.file().entries()[idx].entry()).await?;
                                            fs::write(path, &buf).await?;
                                        }
                                    }
                                }
                                Ok(Message::LocateMultiworld(None))
                            })
                        }
                        #[cfg(target_os = "windows")] Emulator::Project64 => {
                            //TODO indicate progress
                            let http_client = self.http_client.clone();
                            let emulator_path_arg = format!("/DIR={emulator_path}");
                            let create_desktop_shortcut = self.create_desktop_shortcut;
                            return cmd(async move {
                                let front_page_url = Url::parse("https://www.pj64-emu.com/")?;
                                let front_page = http_client.get(front_page_url.clone())
                                    .send().await?
                                    .error_for_status()?
                                    .text().await?;
                                let download_page_url = front_page_url.join(kuchiki::parse_html().one(front_page)
                                    .select_first("a.download").map_err(|()| Error::ParsePj64Html)?
                                    .attributes.borrow()
                                    .get("href").ok_or(Error::ParsePj64Html)?)?;
                                let download_page = http_client.get(download_page_url.clone())
                                    .send().await?
                                    .error_for_status()?
                                    .text().await?;
                                let download_url = download_page_url.join(kuchiki::parse_html().one(download_page)
                                    .select("a").map_err(|()| Error::ParsePj64Html)?
                                    .filter(|node| node.text_contents() == "Try again")
                                    .exactly_one()?
                                    .attributes.borrow()
                                    .get("href").ok_or(Error::ParsePj64Html)?)?;
                                {
                                    let installer = tempfile::Builder::new().prefix("pj64-installer-").suffix(".exe").tempfile()?;
                                    io::copy_buf(&mut StreamReader::new(http_client.get(download_url).send().await?.error_for_status()?.bytes_stream().map_err(io_error_from_reqwest)), &mut tokio::fs::File::from_std(installer.reopen()?)).await?;
                                    let installer_path = installer.into_temp_path();
                                    let mut installer = tokio::process::Command::new(&installer_path);
                                    installer.arg("/SILENT");
                                    installer.arg(emulator_path_arg);
                                    if !create_desktop_shortcut {
                                        installer.arg("/MERGETASKS=!desktopicon");
                                    }
                                    installer.check("Project64 installer").await?;
                                }
                                Ok(Message::LocateMultiworld(None)) //TODO include version info if installing Project64 v4
                            })
                        }
                    }
                } else {
                    match emulator {
                        #[cfg(target_os = "windows")] Emulator::BizHawk => {
                            let [major, minor, patch, _] = match winver::get_file_version_info(PathBuf::from(emulator_path).join("EmuHawk.exe")) {
                                Ok(version) => version,
                                Err(e) => return cmd(future::err(e.into())),
                            };
                            let local_bizhawk_version = Version::new(major.into(), minor.into(), patch.into());
                            let required_bizhawk_version = include!(concat!(env!("OUT_DIR"), "/bizhawk_version.rs"));
                            match local_bizhawk_version.cmp(&required_bizhawk_version) {
                                Less => {
                                    self.page = Page::AskBizHawkUpdate { emulator_path: emulator_path.clone(), multiworld_path: multiworld_path.clone() };
                                    return Command::none()
                                }
                                Equal => {}
                                Greater => return cmd(future::err(Error::BizHawkVersionRegression)),
                            }
                            return cmd(future::ok(Message::LocateMultiworld(Some(major))))
                        }
                        #[cfg(target_os = "linux")] Emulator::BizHawk => {} //TODO BizHawk version check on Linux
                        #[cfg(target_os = "windows")] Emulator::Project64 => {
                            let [major, minor, _, _] = match winver::get_file_version_info(PathBuf::from(emulator_path).join("Project64.exe")) {
                                Ok(version) => version,
                                Err(e) => return cmd(future::err(e.into())),
                            };
                            match (major, minor) {
                                (..=1, _) | (2, ..=3) => return cmd(future::err(Error::OutdatedProject64)), //TODO offer to update Project64
                                (2, 4..) | (3..=4, _) => {} //TODO warn about Project64 v4 being experimental?
                                (5.., _) => return cmd(future::err(Error::Project64TooNew)),
                            }
                            return cmd(future::ok(Message::LocateMultiworld(Some(major))))
                        }
                    }
                },
                Page::AskBizHawkUpdate { ref emulator_path, ref multiworld_path } => {
                    let http_client = self.http_client.clone();
                    let emulator_path_buf = PathBuf::from(emulator_path.clone());
                    self.page = Page::InstallEmulator { update: true, emulator: Emulator::BizHawk, emulator_path: emulator_path.clone(), multiworld_path: multiworld_path.clone() };
                    return cmd(async move {
                        //TODO also update prereqs
                        let version_str = include!(concat!(env!("OUT_DIR"), "/bizhawk_version.rs")).to_string();
                        let version_str = version_str.trim_end_matches(".0");
                        let release = Repo::new("TASEmulators", "BizHawk").release_by_tag(&http_client, version_str).await?.ok_or(Error::NoBizHawkReleases)?;
                        let (asset,) = release.assets.into_iter()
                            .filter(|asset| asset.name.ends_with(BIZHAWK_PLATFORM_SUFFIX))
                            .collect_tuple().ok_or(Error::MissingBizHawkAsset)?;
                        let response = http_client.get(asset.browser_download_url).send().await?.error_for_status()?.bytes().await?;
                        #[cfg(target_os = "linux")] {
                            let tar_file = async_compression::tokio::bufread::GzipDecoder::new(Cursor::new(Vec::from(response)));
                            tokio_tar::Archive::new(tar_file).unpack(emulator_path_buf).await?;
                        }
                        #[cfg(target_os = "windows")] {
                            let zip_file = async_zip::base::read::mem::ZipFileReader::new(response.into()).await?;
                            let entries = zip_file.file().entries().iter().enumerate().map(|(idx, entry)| (idx, entry.entry().filename().ends_with('/'), emulator_path_buf.join(entry.entry().filename()))).collect_vec();
                            for (idx, is_dir, path) in entries {
                                if is_dir {
                                    fs::create_dir_all(path).await?;
                                } else {
                                    if let Some(parent) = path.parent() {
                                        fs::create_dir_all(parent).await?;
                                    }
                                    let mut buf = Vec::default();
                                    zip_file.entry(idx).await?.read_to_end_checked(&mut buf, zip_file.file().entries()[idx].entry()).await?;
                                    fs::write(path, &buf).await?;
                                }
                            }
                        }
                        Ok(Message::LocateMultiworld(None))
                    })
                }
                Page::InstallEmulator { .. } => unreachable!(),
                Page::LocateMultiworld { .. } | Page::InstallMultiworld { .. } => return cmd(future::ok(Message::InstallMultiworld)),
                Page::AskLaunch { emulator, ref emulator_path, ref multiworld_path } => {
                    if self.open_emulator {
                        match emulator {
                            #[cfg(target_os = "linux")] Emulator::BizHawk => if let Err(e) = std::process::Command::new(Path::new(emulator_path).join("EmuHawkMono.sh")).arg("--open-ext-tool-dll=OotrMultiworld").current_dir(emulator_path).spawn() {
                                return cmd(future::ready(Err(e).at(Path::new(emulator_path).join("EmuHawkMono.sh")).map_err(Error::from)))
                            },
                            #[cfg(target_os = "windows")] Emulator::BizHawk => if let Err(e) = std::process::Command::new(Path::new(emulator_path).join("EmuHawk.exe")).arg("--open-ext-tool-dll=OotrMultiworld").current_dir(emulator_path).spawn() {
                                return cmd(future::ready(Err(e).at(Path::new(emulator_path).join("EmuHawk.exe")).map_err(Error::from)))
                            },
                            #[cfg(target_os = "windows")] Emulator::Project64 => {
                                if let Err(e) = std::process::Command::new(Path::new(emulator_path).join("Project64.exe")).current_dir(emulator_path).spawn() {
                                    return cmd(future::ready(Err(e).at(Path::new(emulator_path).join("Project64.exe")).map_err(Error::from)))
                                }
                                if let Err(e) = std::process::Command::new(multiworld_path.as_ref().expect("multiworld app path must be set for Project64")).spawn() {
                                    return cmd(future::ready(Err(e).at(multiworld_path.as_ref().expect("multiworld app path must be set for Project64")).map_err(Error::from)))
                                }
                            }
                        }
                    }
                    return window::close()
                }
            }
            Message::CopyDebugInfo => if let Page::Error(ref e, ref mut debug_info_copied) = self.page {
                *debug_info_copied = true;
                return clipboard::write(e.to_markdown())
            } else {
                self.page = Page::Error(Arc::new(Error::CopyDebugInfo), false);
            },
            Message::DiscordChannel => if let Err(e) = open("https://discord.com/channels/274180765816848384/476723801032491008") {
                self.page = Page::Error(Arc::new(e.into()), false);
            },
            Message::DiscordInvite => if let Err(e) = open("https://discord.gg/BGRrKKn") {
                self.page = Page::Error(Arc::new(e.into()), false);
            },
            Message::EmulatorPath(new_path) => if let Page::LocateEmulator { ref mut emulator_path, .. } = self.page { *emulator_path = new_path },
            Message::Error(e) => self.page = Page::Error(e, false),
            Message::Exit => return window::close(),
            Message::InstallMultiworld => {
                let (emulator, emulator_path, multiworld_path) = match self.page {
                    Page::LocateEmulator { emulator, ref emulator_path, ref multiworld_path, .. } |
                    Page::InstallEmulator { emulator, ref emulator_path, ref multiworld_path, .. } |
                    Page::InstallMultiworld { emulator, ref emulator_path, ref multiworld_path, .. } => (emulator, emulator_path.clone(), multiworld_path.clone()),
                    Page::LocateMultiworld { emulator, ref emulator_path, ref multiworld_path } => (emulator, emulator_path.clone(), Some(multiworld_path.clone())),
                    _ => unreachable!(),
                };
                self.page = Page::InstallMultiworld { emulator, emulator_path: emulator_path.clone(), multiworld_path: multiworld_path.clone(), config_write_failed: false };
                let emulator_dir = PathBuf::from(emulator_path);
                match emulator {
                    Emulator::BizHawk => return cmd(async move {
                        let external_tools_dir = emulator_dir.join("ExternalTools");
                        fs::create_dir(&external_tools_dir).await.exist_ok()?;
                        #[cfg(target_os = "linux")] {
                            let dlls_dir = emulator_dir.join("dll");
                            fs::create_dir(&dlls_dir).await.exist_ok()?;
                            #[cfg(debug_assertions)] fs::write(dlls_dir.join("multiworld.dll"), include_bytes!("../../../target/debug/libmultiworld.so")).await?;
                            #[cfg(not(debug_assertions))] fs::write(dlls_dir.join("multiworld.dll"), include_bytes!("../../../target/release/libmultiworld.so")).await?;
                        }
                        #[cfg(target_os = "windows")] {
                            #[cfg(debug_assertions)] fs::write(external_tools_dir.join("multiworld.dll"), include_bytes!("../../../target/debug/multiworld.dll")).await?; //TODO test if placing in `dll` works, use that if it does to keep the external tools menu clean
                            #[cfg(not(debug_assertions))] fs::write(external_tools_dir.join("multiworld.dll"), include_bytes!("../../../target/release/multiworld.dll")).await?; //TODO test if placing in `dll` works, use that if it does to keep the external tools menu clean
                        }
                        #[cfg(debug_assertions)] fs::write(external_tools_dir.join("OotrMultiworld.dll"), include_bytes!("../../multiworld-bizhawk/OotrMultiworld/src/bin/Debug/net48/OotrMultiworld.dll")).await?;
                        #[cfg(not(debug_assertions))] fs::write(external_tools_dir.join("OotrMultiworld.dll"), include_bytes!("../../multiworld-bizhawk/OotrMultiworld/src/bin/Release/net48/OotrMultiworld.dll")).await?;
                        Ok(Message::MultiworldInstalled)
                    }),
                    #[cfg(target_os = "windows")] Emulator::Project64 => {
                        let multiworld_path = PathBuf::from(multiworld_path.expect("multiworld app path must be set for Project64"));
                        return cmd(async move {
                            fs::create_dir_all(multiworld_path.parent().ok_or(Error::Root)?).await?;
                            //TODO download latest release instead of embedding in installer
                            #[cfg(all(target_os = "windows", debug_assertions))] fs::write(multiworld_path, include_bytes!("../../../target/debug/multiworld-gui.exe")).await?;
                            #[cfg(all(target_os = "windows", not(debug_assertions)))] fs::write(multiworld_path, include_bytes!("../../../target/release/multiworld-gui.exe")).await?;
                            let scripts_path = emulator_dir.join("Scripts");
                            fs::create_dir(&scripts_path).await.exist_ok()?;
                            let script_path = scripts_path.join("ootrmw.js");
                            //TODO download latest release instead of embedding in installer
                            fs::write(&script_path, include_bytes!("../../../assets/ootrmw-pj64.js")).await?;
                            let mut new_mw_config = CONFIG.clone();
                            new_mw_config.pj64_script_path = Some(script_path);
                            new_mw_config.save()?;
                            let config_path = emulator_dir.join("Config");
                            fs::create_dir(&config_path).await.exist_ok()?;
                            let config_path = config_path.join("Project64.cfg");
                            let mut config = match tokio::fs::read_to_string(&config_path).await {
                                Ok(config) => serde_ini::from_str(&config)?,
                                Err(e) if e.kind() == io::ErrorKind::NotFound => Pj64Config::default(),
                                Err(e) => return Err(e).at(&config_path).map_err(Error::from),
                            };
                            config.settings.basic_mode = 0;
                            config.debugger.debugger = 1;
                            match fs::write(config_path, serde_ini::to_vec(&config)?).await {
                                Ok(_) => Ok(Message::MultiworldInstalled),
                                Err(wheel::Error::Io { inner, .. }) if inner.raw_os_error() == Some(32) => Ok(Message::ConfigWriteFailed),
                                Err(e) => Err(e.into()),
                            }
                        })
                    }
                }
            }
            Message::LocateMultiworld(emulator_version) => {
                let (emulator, emulator_path, multiworld_path) = match self.page {
                    Page::LocateEmulator { emulator, ref emulator_path, ref multiworld_path, .. } => (emulator, emulator_path.clone(), multiworld_path.clone()),
                    Page::InstallEmulator { emulator, ref emulator_path, ref multiworld_path, .. } => (emulator, emulator_path.clone(), multiworld_path.clone()),
                    _ => unreachable!(),
                };
                match (emulator, emulator_version) {
                    #[cfg(target_os = "linux")] (Emulator::BizHawk, _) => return cmd(future::ok(Message::InstallMultiworld)),
                    #[cfg(target_os = "windows")] (Emulator::BizHawk, _) | (Emulator::Project64, Some(4..)) => return cmd(future::ok(Message::InstallMultiworld)),
                    #[cfg(target_os = "windows")] (Emulator::Project64, _) => {
                        let multiworld_path = if let Some(multiworld_path) = multiworld_path {
                            multiworld_path
                        } else if let Some(user_dirs) = UserDirs::new() {
                            let multiworld_path = user_dirs.home_dir().join("bin").join("Mido's House Multiworld for Project64.exe");
                            let Ok(multiworld_path) = multiworld_path.into_os_string().into_string() else { return cmd(future::err(Error::NonUtf8Path)) };
                            multiworld_path
                        } else {
                            String::default()
                        };
                        self.page = Page::LocateMultiworld { multiworld_path, emulator, emulator_path };
                    }
                }
            }
            Message::MultiworldInstalled => if let Page::InstallMultiworld { emulator, ref emulator_path, ref multiworld_path, .. } = self.page {
                self.page = Page::AskLaunch { emulator, emulator_path: emulator_path.clone(), multiworld_path: multiworld_path.clone() };
            },
            Message::MultiworldPath(new_path) => if let Page::LocateMultiworld { ref mut multiworld_path, .. } = self.page { *multiworld_path = new_path },
            Message::NewIssue => if let Page::Error(ref e, _) = self.page {
                let mut issue_url = match Url::parse("https://github.com/midoshouse/ootr-multiworld/issues/new") {
                    Ok(issue_url) => issue_url,
                    Err(e) => return cmd(future::err(e.into())),
                };
                issue_url.query_pairs_mut().append_pair("body", &e.to_markdown());
                if let Err(e) = open(issue_url.to_string()) {
                    self.page = Page::Error(Arc::new(e.into()), false);
                }
            } else {
                self.page = Page::Error(Arc::new(Error::CopyDebugInfo), false);
            },
            Message::Nop => {}
            Message::PlatformSupport => if let Err(e) = open("https://midos.house/mw/platforms") {
                self.page = Page::Error(Arc::new(e.into()), false);
            },
            Message::SetCreateDesktopShortcut(create_desktop_shortcut) => self.create_desktop_shortcut = create_desktop_shortcut,
            Message::SetEmulator(new_emulator) => if let Page::SelectEmulator { ref mut emulator, .. } = self.page { *emulator = Some(new_emulator) },
            Message::SetInstallEmulator(new_install_emulator) => if let Page::LocateEmulator { ref mut install_emulator, .. } = self.page { *install_emulator = new_install_emulator },
            Message::SetOpenEmulator(open_emulator) => self.open_emulator = open_emulator,
        }
        Command::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let (top, next_btn) = match self.page {
            Page::Error(ref e, debug_info_copied) => (
                Into::<Element<'_, Message>>::into(Scrollable::new(Row::new()
                    .push(Column::new()
                        .push(Text::new("Error").size(24))
                        .push(Text::new("An error occured while trying to install Mido's House Multiworld:"))
                        .push(Text::new(e.to_string()))
                        .push(Row::new()
                            .push(Button::new(Text::new("Copy debug info")).on_press(Message::CopyDebugInfo))
                            .push(Text::new(if debug_info_copied { "Copied!" } else { "for pasting into Discord" }))
                            .spacing(8)
                        )
                        .push(Text::new("Support").size(24))
                        .push(Text::new("• Ask in #setup-support on the OoT Randomizer Discord. Feel free to ping @Fenhl#4813."))
                        .push(Row::new()
                            .push(Button::new(Text::new("invite link")).on_press(Message::DiscordInvite))
                            .push(Button::new(Text::new("direct channel link")).on_press(Message::DiscordChannel))
                            .spacing(8)
                        )
                        .push(Text::new("• Ask in #general on the OoTR MW Tournament Discord."))
                        .push(Row::new()
                            .push(Text::new("• Or "))
                            .push(Button::new(Text::new("open an issue")).on_press(Message::NewIssue))
                            .spacing(8)
                        )
                        .spacing(8))
                    .push(Space::with_width(Length::Shrink)) // to avoid overlap with the scrollbar
                    .spacing(16)
                )),
                None,
            ),
            Page::Elevated => (
                Text::new("The installer has been reopened with admin permissions. Please continue there.").into(),
                None,
            ),
            Page::SelectEmulator { emulator, .. } => (
                {
                    let mut col = Column::new();
                    col = col.push(Text::new("Which emulator do you want to use?"));
                    col = col.push(Text::new("Multiworld can be added to an existing installation of the selected emulator, or it can install the emulator for you."));
                    for iter_emulator in all::<Emulator>() {
                        col = col.push(Radio::new(iter_emulator.to_string(), iter_emulator, emulator, Message::SetEmulator));
                    }
                    col = col.push(Row::new()
                        .push(Text::new("Looking for a different console or emulator? "))
                        .push(Button::new(Text::new("See platform support status")).on_press(Message::PlatformSupport))
                        .spacing(8)
                    );
                    col.spacing(8).into()
                },
                Some({
                    let mut row = Row::new();
                    #[cfg(target_os = "windows")] if matches!(emulator, Some(Emulator::Project64)) && !is_elevated() {
                        row = row.push(Image::new(image::Handle::from_memory(include_bytes!("../../../assets/uac.png").to_vec())).height(Length::Fixed(20.0)));
                    }
                    row = row.push(Text::new("Continue"));
                    (Into::<Element<'_, Message>>::into(row.spacing(8)), emulator.is_some())
                })
            ),
            Page::LocateEmulator { emulator, install_emulator, ref emulator_path, .. } => (
                {
                    let mut col = Column::new();
                    col = col.push(Radio::new(format!("Install {emulator} to:"), true, Some(install_emulator), Message::SetInstallEmulator));
                    col = col.push(Radio::new(format!("I already have {emulator} at:"), false, Some(install_emulator), Message::SetInstallEmulator));
                    col = col.push(Row::new()
                        .push(TextInput::new(&if install_emulator {
                            Cow::Owned(format!("{emulator} target folder"))
                        } else {
                            match emulator {
                                Emulator::BizHawk => {
                                    #[cfg(target_os = "linux")] { Cow::Borrowed("The folder with EmuHawkMono.sh in it") }
                                    #[cfg(target_os = "windows")] { Cow::Borrowed("The folder with EmuHawk.exe in it") }
                                }
                                #[cfg(target_os = "windows")] Emulator::Project64 => Cow::Borrowed("The folder with Project64.exe in it"),
                            }
                        }, emulator_path).on_input(Message::EmulatorPath).on_paste(Message::EmulatorPath).padding(5))
                        .push(Button::new(Text::new("Browse…")).on_press(Message::BrowseEmulatorPath))
                        .spacing(8)
                    );
                    #[cfg(target_os = "windows")] if install_emulator && matches!(emulator, Emulator::Project64) {
                        //TODO allow selecting between Project64 3.x and 4.0
                        col = col.push(Checkbox::new("Create desktop shortcut", self.create_desktop_shortcut, Message::SetCreateDesktopShortcut));
                    }
                    col.spacing(8).into()
                },
                Some((
                    if install_emulator { Text::new(format!("Install {emulator}")) } else { Text::new("Continue") }.into(),
                    !emulator_path.is_empty(),
                )),
            ),
            Page::AskBizHawkUpdate { .. } => (
                Text::new("The selected copy of BizHawk is too old to run Mido's House Multiworld. Do you want to update it to the latest version?").into(),
                Some((Text::new("Update BizHawk").into(), true))
            ),
            Page::InstallEmulator { update: true, emulator, .. } => (Text::new(format!("Updating {emulator}, please wait…")).into(), None),
            Page::InstallEmulator { update: false, emulator, .. } => (Text::new(format!("Installing {emulator}, please wait…")).into(), None),
            Page::LocateMultiworld { ref multiworld_path, .. } => (
                Column::new()
                    .push(Text::new("Install Multiworld to:"))
                    .push(Row::new()
                        .push(TextInput::new("Multiworld target folder", multiworld_path).on_input(Message::MultiworldPath).on_paste(Message::MultiworldPath).padding(5))
                        .push(Button::new(Text::new("Browse…")).on_press(Message::BrowseMultiworldPath))
                        .spacing(8)
                    )
                    .spacing(8)
                    .into(),
                Some((Text::new("Install Multiworld").into(), !multiworld_path.is_empty())),
            ),
            Page::InstallMultiworld { config_write_failed: true, emulator, .. } => (
                Text::new(format!("Could not adjust {emulator} settings. Please close {emulator} and try again.")).into(),
                Some((Text::new("Try Again").into(), true)),
            ),
            Page::InstallMultiworld { config_write_failed: false, .. } => (Text::new("Installing multiworld, please wait…").into(), None),
            Page::AskLaunch { emulator, .. } => (
                {
                    let mut col = Column::new();
                    col = col.push(Text::new("Multiworld has been installed."));
                    match emulator {
                        Emulator::BizHawk => {
                            col = col.push(Text::new("To play multiworld, in BizHawk, select Tools → External Tool → Mido's House Multiworld for BizHawk."));
                            col = col.push(Checkbox::new("Open BizHawk now", self.open_emulator, Message::SetOpenEmulator));
                        }
                        #[cfg(target_os = "windows")] Emulator::Project64 => {
                            col = col.push(Text::new("To play multiworld, open the “Mido's House Multiworld for Project64” app and follow its instructions."));
                            col = col.push(Checkbox::new("Open Multiworld and Project64 now", self.open_emulator, Message::SetOpenEmulator));
                        }
                    }
                    col.spacing(8).into()
                },
                Some((Text::new("Finish").into(), true)),
            ),
        };
        let mut view = Column::new()
            .push(top);
        if let Some((btn_content, enabled)) = next_btn {
            let mut bottom_row = Row::new();
            if matches!(self.page, Page::SelectEmulator { .. }) {
                bottom_row = bottom_row.push(Text::new(concat!("v", env!("CARGO_PKG_VERSION"))));
            } else {
                bottom_row = bottom_row.push(Button::new(Text::new("Back")).on_press(Message::Back));
            }
            bottom_row = bottom_row.push(Space::with_width(Length::Fill));
            let mut next_btn = Button::new(btn_content);
            if enabled { next_btn = next_btn.on_press(Message::Continue) }
            bottom_row = bottom_row.push(next_btn);
            view = view
                .push(Space::with_height(Length::Fill))
                .push(bottom_row.spacing(8));
        }
        view
            .spacing(8)
            .padding(8)
            .into()
    }
}

#[cfg(target_os = "windows")]
fn io_error_from_reqwest(e: reqwest::Error) -> io::Error {
    io::Error::new(if e.is_timeout() {
        io::ErrorKind::TimedOut
    } else {
        io::ErrorKind::Other
    }, e)
}

#[derive(clap::Parser)]
#[clap(version)]
struct Args {
    #[clap(long, value_enum)]
    emulator: Option<Emulator>,
}

#[derive(Debug, thiserror::Error)]
enum MainError {
    #[error(transparent)] Iced(#[from] iced::Error),
    #[error(transparent)] Icon(#[from] iced::window::icon::Error),
    #[error(transparent)] Io(#[from] io::Error),
    #[error(transparent)] Wheel(#[from] wheel::Error),
}

#[wheel::main(debug)]
fn main(args: Args) -> Result<(), MainError> {
    Ok(State::run(Settings {
        window: window::Settings {
            size: (400, 300),
            icon: Some(icon::from_file_data(include_bytes!("../../../assets/icon.ico"), Some(ImageFormat::Ico))?),
            ..window::Settings::default()
        },
        ..Settings::with_flags(args)
    })?)
}
