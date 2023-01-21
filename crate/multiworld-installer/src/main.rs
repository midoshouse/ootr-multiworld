#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]
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
    futures::{
        future::{
            self,
            Future,
        },
        stream::TryStreamExt as _,
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
            Icon,
        },
    },
    is_elevated::is_elevated,
    itertools::Itertools as _,
    kuchiki::traits::TendrilSink as _,
    lazy_regex::regex_is_match,
    open::that as open,
    rfd::AsyncFileDialog,
    serde::{
        Deserialize,
        Serialize,
    },
    serenity::utils::MessageBuilder,
    tokio::io::{
        self,
        AsyncWriteExt as _,
    },
    tokio_util::io::StreamReader,
    url::Url,
    wheel::{
        fs,
        traits::{
            AsyncCommandOutputExt as _,
            IoResultExt as _,
            SyncCommandOutputExt as _,
        },
    },
    multiworld::{
        config::CONFIG,
        github::Repo,
    },
};

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] Config(#[from] multiworld::config::SaveError),
    #[error(transparent)] IniDe(#[from] serde_ini::de::Error),
    #[error(transparent)] IniSer(#[from] serde_ini::ser::Error),
    #[error(transparent)] Io(#[from] io::Error),
    #[error(transparent)] Reqwest(#[from] reqwest::Error),
    #[error(transparent)] Task(#[from] tokio::task::JoinError),
    #[error(transparent)] Url(#[from] url::ParseError),
    #[error(transparent)] Wheel(#[from] wheel::Error),
    #[error(transparent)] Zip(#[from] async_zip::error::ZipError),
    #[error("tried to copy debug info or open a GitHub issue with no active error")]
    CopyDebugInfo,
    #[error("got zero elements when exactly one was expected")]
    ExactlyOneEmpty,
    #[error("got at least 2 elements when exactly one was expected")]
    ExactlyOneMultiple,
    #[error("no BizHawk releases found")]
    NoBizHawkReleases,
    #[error("failed to parse Project64 website")]
    ParsePj64Html,
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
    LocateMultiworld,
    MultiworldInstalled,
    MultiworldPath(String),
    NewIssue,
    Nop,
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
    InstallEmulator {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[clap(rename_all = "lower")]
enum Emulator {
    BizHawk,
    Project64,
}

impl fmt::Display for Emulator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BizHawk => write!(f, "BizHawk"),
            Self::Project64 => write!(f, "Project64"),
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

    fn new(Args { emulator }: Args) -> (Self, Command<Message>) {
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
                Page::LocateMultiworld { emulator, ref emulator_path, ref multiworld_path } => Page::LocateEmulator { emulator, install_emulator: false, emulator_path: emulator_path.clone(), multiworld_path: Some(multiworld_path.clone()) },
                Page::InstallMultiworld { emulator, ref emulator_path, ref multiworld_path, .. } | Page::AskLaunch { emulator, ref emulator_path, ref multiworld_path } => match emulator {
                    Emulator::BizHawk => Page::LocateEmulator { emulator, install_emulator: false, emulator_path: emulator_path.clone(), multiworld_path: multiworld_path.clone() },
                    Emulator::Project64 => Page::LocateMultiworld { emulator, emulator_path: emulator_path.clone(), multiworld_path: multiworld_path.clone().expect("multiworld app path must be set for Project64") },
                },
            },
            Message::BrowseEmulatorPath => if let Page::LocateEmulator { emulator, install_emulator, ref emulator_path, .. } = self.page {
                let current_path = emulator_path.clone();
                return cmd(async move {
                    Ok(if let Some(emulator_dir) = AsyncFileDialog::new().set_title(match (emulator, install_emulator) {
                        (Emulator::BizHawk, false) => "Select BizHawk Folder",
                        (Emulator::BizHawk, true) => "Choose Location for BizHawk Installation",
                        (Emulator::Project64, false) => "Select Project64 Folder",
                        (Emulator::Project64, true) => "Choose Location for Project64 Installation",
                    }).set_directory(Path::new(&current_path)).pick_folder().await {
                        Message::EmulatorPath(emulator_dir.path().to_str().expect("Windows paths are valid Unicode").to_owned())
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
                        Message::MultiworldPath(multiworld_path.path().to_str().expect("Windows paths are valid Unicode").to_owned())
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
                    if matches!(emulator, Emulator::Project64) && !is_elevated() {
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
                                    (
                                        false,
                                        bizhawk_install_path.into_os_string().into_string().expect("Windows paths are valid Unicode"),
                                    )
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
                                    (
                                        false,
                                        default_bizhawk_dir.into_os_string().into_string().expect("Windows paths are valid Unicode"),
                                    )
                                } else {
                                    (
                                        true,
                                        bizhawk_install_path.into_os_string().into_string().expect("Windows paths are valid Unicode"),
                                    )
                                }
                            } else {
                                (true, String::default())
                            },
                            Emulator::Project64 => if let Some(pj64_install_path) = env::var_os("ProgramFiles(x86)").or_else(|| env::var_os("ProgramFiles")).map(|program_files| PathBuf::from(program_files).join("Project64 3.0")) {
                                (
                                    !pj64_install_path.exists(),
                                    pj64_install_path.into_os_string().into_string().expect("Windows paths are valid Unicode"),
                                )
                            } else {
                                (true, String::default())
                            },
                        },
                    };
                    self.page = Page::LocateEmulator { emulator, install_emulator, emulator_path, multiworld_path: multiworld_path.clone() };
                }
                Page::LocateEmulator { emulator, install_emulator, ref emulator_path, ref multiworld_path } => if install_emulator {
                    let emulator_path = emulator_path.clone();
                    self.page = Page::InstallEmulator { emulator, emulator_path: emulator_path.clone(), multiworld_path: multiworld_path.clone() };
                    match emulator {
                        Emulator::BizHawk => {
                            //TODO indicate progress
                            let http_client = self.http_client.clone();
                            let bizhawk_dir = PathBuf::from(emulator_path);
                            return cmd(async move {
                                fs::create_dir_all(&bizhawk_dir).await?;
                                // install BizHawk-Prereqs
                                let release = Repo::new("TASEmulators", "BizHawk-Prereqs").latest_release(&http_client).await?.ok_or(Error::NoBizHawkReleases)?;
                                let asset = release.assets.into_iter()
                                    .filter(|asset| regex_is_match!(r"^bizhawk_prereqs_v.+\.zip$", &asset.name))
                                    .exactly_one()?;
                                let response = http_client.get(asset.browser_download_url).send().await?.error_for_status()?.bytes().await?;
                                let zip_file = async_zip::read::mem::ZipFileReader::new(response.into()).await?;
                                let _ = zip_file.file().entries().iter().exactly_one()?;
                                {
                                    let mut buf = Vec::default();
                                    zip_file.entry(0).await?.read_to_end_checked(&mut buf, zip_file.file().entries()[0].entry()).await?;
                                    let prereqs = tempfile::Builder::new().prefix("bizhawk_prereqs_").suffix(".exe").tempfile()?;
                                    tokio::fs::File::from_std(prereqs.reopen()?).write_all(&buf).await?;
                                    let prereqs_path = prereqs.into_temp_path();
                                    runas::Command::new(&prereqs_path).status().at_command("runas")?.check("BizHawk-Prereqs")?;
                                }
                                // install BizHawk itself
                                let release = Repo::new("TASEmulators", "BizHawk").latest_release(&http_client).await?.ok_or(Error::NoBizHawkReleases)?;
                                #[cfg(all(windows, target_arch = "x86_64"))] let asset = release.assets.into_iter()
                                    .filter(|asset| regex_is_match!(r"^BizHawk-.+-win-x64\.zip$", &asset.name))
                                    .exactly_one()?;
                                let response = http_client.get(asset.browser_download_url).send().await?.error_for_status()?.bytes().await?;
                                let zip_file = async_zip::read::mem::ZipFileReader::new(response.into()).await?;
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
                                Ok(Message::LocateMultiworld)
                            })
                        }
                        Emulator::Project64 => {
                            //TODO indicate progress
                            let client = self.http_client.clone();
                            let emulator_path_arg = format!("/DIR={emulator_path}");
                            let create_desktop_shortcut = self.create_desktop_shortcut;
                            return cmd(async move {
                                let front_page_url = Url::parse("https://www.pj64-emu.com/")?;
                                let front_page = client.get(front_page_url.clone())
                                    .send().await?
                                    .error_for_status()?
                                    .text().await?;
                                let download_page_url = front_page_url.join(kuchiki::parse_html().one(front_page)
                                    .select_first("a.download").map_err(|()| Error::ParsePj64Html)?
                                    .attributes.borrow()
                                    .get("href").ok_or(Error::ParsePj64Html)?)?;
                                let download_page = client.get(download_page_url.clone())
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
                                    io::copy_buf(&mut StreamReader::new(client.get(download_url).send().await?.error_for_status()?.bytes_stream().map_err(io_error_from_reqwest)), &mut tokio::fs::File::from_std(installer.reopen()?)).await?;
                                    let installer_path = installer.into_temp_path();
                                    let mut installer = tokio::process::Command::new(&installer_path);
                                    installer.arg("/SILENT");
                                    installer.arg(emulator_path_arg);
                                    if !create_desktop_shortcut {
                                        installer.arg("/MERGETASKS=!desktopicon");
                                    }
                                    installer.check("Project64 installer").await?;
                                }
                                Ok(Message::LocateMultiworld)
                            })
                        }
                    }
                } else {
                    //TODO make sure emulator is up to date
                    return cmd(future::ok(Message::LocateMultiworld))
                },
                Page::InstallEmulator { .. } => unreachable!(),
                Page::LocateMultiworld { .. } | Page::InstallMultiworld { .. } => return cmd(future::ok(Message::InstallMultiworld)),
                Page::AskLaunch { emulator, ref emulator_path, ref multiworld_path } => {
                    if self.open_emulator {
                        match emulator {
                            Emulator::BizHawk => if let Err(e) = std::process::Command::new(Path::new(emulator_path).join("EmuHawk.exe")).arg("--open-ext-tool-dll=OotrMultiworld").current_dir(emulator_path).spawn() {
                                return cmd(future::ready(Err(e).at(Path::new(emulator_path).join("EmuHawk.exe")).map_err(Error::from)))
                            },
                            Emulator::Project64 => {
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
                    Page::InstallEmulator { emulator, ref emulator_path, ref multiworld_path } |
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
                        //TODO download latest release instead of embedding in installer
                        fs::write(external_tools_dir.join("multiworld.dll"), include_bytes!("../../../target/release/multiworld.dll")).await?;
                        fs::write(external_tools_dir.join("OotrMultiworld.dll"), include_bytes!("../../multiworld-bizhawk/OotrMultiworld/BizHawk/ExternalTools/OotrMultiworld.dll")).await?;
                        Ok(Message::MultiworldInstalled)
                    }),
                    Emulator::Project64 => {
                        let multiworld_path = PathBuf::from(multiworld_path.expect("multiworld app path must be set for Project64"));
                        return cmd(async move {
                            fs::create_dir_all(multiworld_path.parent().ok_or(Error::Root)?).await?;
                            //TODO download latest release instead of embedding in installer
                            fs::write(multiworld_path, include_bytes!("../../../target/release/multiworld-pj64-gui.exe")).await?;
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
            Message::LocateMultiworld => {
                let (emulator, emulator_path, multiworld_path) = match self.page {
                    Page::LocateEmulator { emulator, ref emulator_path, ref multiworld_path, .. } => (emulator, emulator_path.clone(), multiworld_path.clone()),
                    Page::InstallEmulator { emulator, ref emulator_path, ref multiworld_path } => (emulator, emulator_path.clone(), multiworld_path.clone()),
                    _ => unreachable!(),
                };
                match emulator {
                    Emulator::BizHawk => return cmd(future::ok(Message::InstallMultiworld)),
                    Emulator::Project64 => self.page = Page::LocateMultiworld { emulator, emulator_path, multiworld_path: multiworld_path.or_else(|| UserDirs::new().map(|user_dirs| user_dirs.home_dir().join("bin").join("Mido's House Multiworld for Project64.exe").into_os_string().into_string().expect("Windows paths are valid Unicode"))).unwrap_or_default() },
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
                Column::new()
                    .push(Text::new("Which emulator do you want to use?"))
                    .push(Text::new("Multiworld can be added to an existing installation of the selected emulator, or it can install the emulator for you."))
                    .push(Radio::new(Emulator::BizHawk, "BizHawk", emulator, Message::SetEmulator))
                    .push(Radio::new(Emulator::Project64, "Project64", emulator, Message::SetEmulator))
                    .spacing(8)
                    .into(),
                Some({
                    let mut row = Row::new();
                    if matches!(emulator, Some(Emulator::Project64)) && !is_elevated() {
                        row = row.push(Image::new(image::Handle::from_memory(include_bytes!("../../../assets/uac.png").to_vec())).height(Length::Units(20)));
                    }
                    row = row.push(Text::new("Continue"));
                    (Into::<Element<'_, Message>>::into(row.spacing(8)), emulator.is_some())
                })
            ),
            Page::LocateEmulator { emulator, install_emulator, ref emulator_path, .. } => (
                {
                    let mut col = Column::new();
                    col = col.push(Radio::new(true, format!("Install {emulator} to:"), Some(install_emulator), Message::SetInstallEmulator));
                    col = col.push(Radio::new(false, format!("I already have {emulator} at:"), Some(install_emulator), Message::SetInstallEmulator));
                    col = col.push(Row::new()
                        .push(TextInput::new(&if install_emulator {
                            Cow::Owned(format!("{emulator} target folder"))
                        } else {
                            match emulator {
                                Emulator::BizHawk => Cow::Borrowed("The folder with EmuHawk.exe in it"),
                                Emulator::Project64 => Cow::Borrowed("The folder with Project64.exe in it"),
                            }
                        }, emulator_path, Message::EmulatorPath).padding(5))
                        .push(Button::new(Text::new("Browse…")).on_press(Message::BrowseEmulatorPath))
                        .spacing(8)
                    );
                    if install_emulator && matches!(emulator, Emulator::Project64) {
                        col = col.push(Checkbox::new("Create desktop shortcut", self.create_desktop_shortcut, Message::SetCreateDesktopShortcut));
                    }
                    col.spacing(8).into()
                },
                Some((
                    if install_emulator { Text::new(format!("Install {emulator}")) } else { Text::new("Continue") }.into(),
                    !emulator_path.is_empty(),
                )),
            ),
            Page::InstallEmulator { emulator, .. } => (Text::new(format!("Installing {emulator}, please wait…")).into(), None),
            Page::LocateMultiworld { ref multiworld_path, .. } => (
                Column::new()
                    .push(Text::new("Install Multiworld to:"))
                    .push(Row::new()
                        .push(TextInput::new("Multiworld target folder", multiworld_path, Message::MultiworldPath).padding(5))
                        .push(Button::new(Text::new("Browse…")).on_press(Message::BrowseMultiworldPath))
                        .spacing(8)
                    )
                    .spacing(8)
                    .into(),
                Some((Text::new(format!("Install Multiworld")).into(), !multiworld_path.is_empty())),
            ),
            Page::InstallMultiworld { config_write_failed: true, emulator, .. } => (
                Text::new(format!("Could not adjust {emulator} settings. Please close {emulator} and try again.")).into(),
                Some((Text::new(format!("Try Again")).into(), true)),
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
                        Emulator::Project64 => {
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
}

#[wheel::main]
fn main(args: Args) -> Result<(), MainError> {
    let icon = ::image::load_from_memory(include_bytes!("../../../assets/icon.ico")).expect("failed to load embedded DynamicImage").to_rgba8();
    State::run(Settings {
        window: window::Settings {
            size: (400, 300),
            icon: Some(Icon::from_rgba(icon.as_flat_samples().as_slice().to_owned(), icon.width(), icon.height())?),
            ..window::Settings::default()
        },
        ..Settings::with_flags(args)
    })?;
    Ok(())
}
