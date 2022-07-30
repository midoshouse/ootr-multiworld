#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use {
    std::{
        borrow::Cow,
        env,
        fmt,
        path::{
            Path,
            PathBuf,
        },
        sync::Arc,
    },
    directories::UserDirs,
    futures::{
        future::{
            self,
            Future,
        },
        stream::TryStreamExt as _,
    },
    iced::{
        Command,
        Length,
        Settings,
        pure::{
            Application,
            Element,
            widget::*,
        },
        window,
    },
    is_elevated::is_elevated,
    itertools::Itertools as _,
    kuchiki::traits::TendrilSink as _,
    lazy_regex::regex_is_match,
    rfd::AsyncFileDialog,
    tokio::{
        fs::{
            self,
            File,
        },
        io,
    },
    tokio_util::io::StreamReader,
    url::Url,
    wheel::traits::{
        AsyncCommandOutputExt as _,
        IoResultExt as _,
        SyncCommandOutputExt as _,
    },
    crate::github::Repo,
};

mod github;

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] Io(#[from] std::io::Error),
    #[error(transparent)] Reqwest(#[from] reqwest::Error),
    #[error(transparent)] Task(#[from] tokio::task::JoinError),
    #[error(transparent)] Url(#[from] url::ParseError),
    #[error(transparent)] Wheel(#[from] wheel::Error),
    #[error(transparent)] Zip(#[from] async_zip::error::ZipError),
    #[error("got zero elements when exactly one was expected")]
    ExactlyOneEmpty,
    #[error("got at least 2 elements when exactly one was expected")]
    ExactlyOneMultiple,
    #[error("no BizHawk releases found")]
    NoBizHawkReleases,
    #[error("failed to parse Project64 website")]
    ParsePj64Html,
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
    BrowseEmulatorPath,
    BrowseMultiworldPath,
    Continue,
    EmulatorPath(String),
    Error(Arc<Error>),
    Exit,
    InstallMultiworld,
    LocateMultiworld,
    MultiworldInstalled,
    MultiworldPath(String),
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
            Err(e) => Message::Error(Arc::new(e.into())),
        }
    })))
}

enum Page {
    Error(Arc<Error>),
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
    },
    AskLaunch {
        emulator: Emulator,
        emulator_path: String,
        multiworld_path: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
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
    should_exit: bool,
}

impl Application for State {
    type Executor = iced::executor::Default;
    type Message = Message;
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
            should_exit: false,
        }, if emulator.is_some() {
            cmd(future::ok(Message::Continue))
        } else {
            Command::none()
        })
    }

    fn should_exit(&self) -> bool { self.should_exit }

    fn title(&self) -> String { format!("OoTR Multiworld Installer") }

    fn update(&mut self, msg: Message) -> Command<Message> {
        match msg {
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
                    dialog = dialog.set_file_name("OoTR Multiworld for Project64.exe");
                    dialog = dialog.add_filter("Windows executable", &["exe"]);
                    Ok(if let Some(multiworld_path) = dialog.save_file().await {
                        Message::MultiworldPath(multiworld_path.path().to_str().expect("Windows paths are valid Unicode").to_owned())
                    } else {
                        Message::Nop
                    })
                })
            }
            Message::Continue => match self.page {
                Page::Error(_) | Page::Elevated => unreachable!(),
                Page::SelectEmulator { emulator, install_emulator, ref emulator_path, ref multiworld_path } => {
                    let emulator = emulator.expect("emulator must be selected to continue here");
                    if matches!(emulator, Emulator::Project64) && !is_elevated() {
                        // Project64 installation and plugin installation both require admin permissions (UAC)
                        self.page = Page::Elevated;
                        return cmd(async move {
                            tokio::task::spawn_blocking(|| Ok::<_, Error>(runas::Command::new(env::current_exe()?).arg("--emulator=project64").gui(true).status()?.check("runas")?)).await??;
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
                            //TODO also install BizHawk prereqs
                            //TODO indicate progress
                            let http_client = self.http_client.clone();
                            let bizhawk_dir = PathBuf::from(emulator_path);
                            return cmd(async move {
                                let release = Repo::new("TASEmulators", "BizHawk").latest_release(&http_client).await?.ok_or(Error::NoBizHawkReleases)?;
                                #[cfg(all(windows, target_arch = "x86_64"))] let asset = release.assets.into_iter()
                                    .filter(|asset| regex_is_match!(r"^BizHawk-.+-win-x64\.zip$", &asset.name))
                                    .exactly_one()?;
                                let mut response = http_client.get(asset.browser_download_url).send().await?.error_for_status()?.bytes().await?;
                                let mut zip_file = async_zip::read::mem::ZipFileReader::new(&mut response).await?;
                                let entries = zip_file.entries().iter().enumerate().map(|(idx, entry)| (idx, entry.dir(), bizhawk_dir.join(entry.name()))).collect_vec();
                                for (idx, is_dir, path) in entries {
                                    if is_dir {
                                        fs::create_dir_all(path).await?;
                                    } else {
                                        if let Some(parent) = path.parent() {
                                            fs::create_dir_all(parent).await?;
                                        }
                                        zip_file.entry_reader(idx).await?.copy_to_end_crc(&mut File::create(path).await?, 64 * 1024).await?;
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
                                    io::copy_buf(&mut StreamReader::new(client.get(download_url).send().await?.error_for_status()?.bytes_stream().map_err(io_error_from_reqwest)), &mut File::from_std(installer.reopen()?)).await?;
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
                Page::LocateMultiworld { .. } => return cmd(future::ok(Message::InstallMultiworld)),
                Page::InstallMultiworld { .. } => unreachable!(),
                Page::AskLaunch { emulator, ref emulator_path, ref multiworld_path } => {
                    if self.open_emulator {
                        match emulator {
                            Emulator::BizHawk => if let Err(e) = std::process::Command::new(Path::new(emulator_path).join("EmuHawk.exe")).arg("--open-ext-tool-dll=OotrMultiworld").current_dir(emulator_path).spawn() {
                                return cmd(future::err(e.into()))
                            },
                            Emulator::Project64 => {
                                if let Err(e) = std::process::Command::new(Path::new(emulator_path).join("Project64.exe")).current_dir(emulator_path).spawn() {
                                    return cmd(future::err(e.into()))
                                }
                                if let Err(e) = std::process::Command::new(multiworld_path.as_ref().expect("multiworld app path must be set for Project64")).spawn() {
                                    return cmd(future::err(e.into()))
                                }
                            }
                        }
                    }
                    self.should_exit = true;
                }
            }
            Message::EmulatorPath(new_path) => if let Page::LocateEmulator { ref mut emulator_path, .. } = self.page { *emulator_path = new_path },
            Message::Error(e) => self.page = Page::Error(e),
            Message::Exit => self.should_exit = true,
            Message::InstallMultiworld => {
                let (emulator, emulator_path, multiworld_path) = match self.page {
                    Page::LocateEmulator { emulator, ref emulator_path, ref multiworld_path, .. } => (emulator, emulator_path.clone(), multiworld_path.clone()),
                    Page::InstallEmulator { emulator, ref emulator_path, ref multiworld_path } => (emulator, emulator_path.clone(), multiworld_path.clone()),
                    Page::LocateMultiworld { emulator, ref emulator_path, ref multiworld_path } => (emulator, emulator_path.clone(), Some(multiworld_path.clone())),
                    _ => unreachable!(),
                };
                self.page = Page::InstallMultiworld { emulator, emulator_path: emulator_path.clone(), multiworld_path: multiworld_path.clone() };
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
                        let multiworld_path = multiworld_path.expect("multiworld app path must be set for Project64");
                        return cmd(async move {
                            //TODO download latest release instead of embedding in installer
                            fs::write(multiworld_path, include_bytes!("../../../target/release/multiworld-pj64-gui.exe")).await?;
                            let scripts_path = emulator_dir.join("Scripts");
                            fs::create_dir(&scripts_path).await.exist_ok()?;
                            //TODO download latest release instead of embedding in installer
                            fs::write(scripts_path.join("ootrmw.js"), include_bytes!("../../../assets/ootrmw-pj64.js")).await?;
                            //TODO adjust Config/Project64.cfg (Settings.Basic Mode = 0, Debugger.Debugger = 1)
                            Ok(Message::MultiworldInstalled)
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
                    Emulator::Project64 => self.page = Page::LocateMultiworld { emulator, emulator_path, multiworld_path: multiworld_path.or_else(|| UserDirs::new().map(|user_dirs| user_dirs.home_dir().join("bin").join("OoTR Multiworld for Project64.exe").into_os_string().into_string().expect("Windows paths are valid Unicode"))).unwrap_or_default() },
                }
            }
            Message::MultiworldInstalled => if let Page::InstallMultiworld { emulator, ref emulator_path, ref multiworld_path } = self.page {
                self.page = Page::AskLaunch { emulator, emulator_path: emulator_path.clone(), multiworld_path: multiworld_path.clone() };
            },
            Message::MultiworldPath(new_path) => if let Page::LocateMultiworld { ref mut multiworld_path, .. } = self.page { *multiworld_path = new_path },
            Message::Nop => {}
            Message::SetCreateDesktopShortcut(create_desktop_shortcut) => self.create_desktop_shortcut = create_desktop_shortcut,
            Message::SetEmulator(new_emulator) => if let Page::SelectEmulator { ref mut emulator, .. } = self.page { *emulator = Some(new_emulator) },
            Message::SetInstallEmulator(new_install_emulator) => if let Page::LocateEmulator { ref mut install_emulator, .. } = self.page { *install_emulator = new_install_emulator },
            Message::SetOpenEmulator(open_emulator) => self.open_emulator = open_emulator,
        }
        Command::none()
    }

    fn view(&self) -> Element<'_, Message> {
        match self.page {
            Page::Error(ref e) => Column::new()
                .push(Text::new("An error occurred during the installation:"))
                .push(Text::new(e.to_string()))
                .push(Text::new(format!("Please report this error to Fenhl. Debug info: {e:?}")))
                .into(),
            Page::Elevated => Text::new("The installer has been reopened with admin permissions. Please continue there.").into(),
            Page::SelectEmulator { emulator, .. } => Column::new()
                .push(Text::new("Which emulator do you want to use?"))
                .push(Text::new("Multiworld can be added to an existing installation of the selected emulator, or it can install the emulator for you."))
                .push(Radio::new(Emulator::BizHawk, "BizHawk", emulator, Message::SetEmulator))
                .push(Radio::new(Emulator::Project64, "Project64", emulator, Message::SetEmulator))
                .push({
                    let mut row = Row::new();
                    if matches!(emulator, Some(Emulator::Project64)) && !is_elevated() {
                        row = row.push(Image::new(image::Handle::from_memory(include_bytes!("../../../assets/uac.png").to_vec())).height(Length::Units(20)));
                    }
                    row = row.push(Text::new("Continue"));
                    let mut btn = Button::new(row);
                    if emulator.is_some() { btn = btn.on_press(Message::Continue) }
                    btn
                })
                .into(),
            Page::LocateEmulator { emulator, install_emulator, ref emulator_path, .. } => {
                let continue_btn = if install_emulator {
                    let mut btn = Button::new(Text::new(format!("Install {emulator}")));
                    if !emulator_path.is_empty() { btn = btn.on_press(Message::Continue) }
                    btn
                } else {
                    let mut btn = Button::new(Text::new("Continue"));
                    if !emulator_path.is_empty() { btn = btn.on_press(Message::Continue) }
                    btn
                };
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
                    }, emulator_path, Message::EmulatorPath))
                    .push(Button::new(Text::new("Browse…")).on_press(Message::BrowseEmulatorPath))
                );
                if install_emulator && matches!(emulator, Emulator::Project64) {
                    col = col.push(Checkbox::new(self.create_desktop_shortcut, "Create desktop shortcut", Message::SetCreateDesktopShortcut));
                }
                col = col.push(continue_btn);
                col.into()
            }
            Page::InstallEmulator { emulator, .. } => match emulator {
                Emulator::BizHawk => Text::new("Installing BizHawk, please wait…"),
                Emulator::Project64 => Text::new("Installing Project64, please wait…"),
            }.into(),
            Page::LocateMultiworld { ref multiworld_path, .. } => {
                let continue_btn = {
                    let mut btn = Button::new(Text::new(format!("Install Multiworld")));
                    if !multiworld_path.is_empty() { btn = btn.on_press(Message::Continue) }
                    btn
                };
                Column::new()
                    .push(Text::new("Install Multiworld to:"))
                    .push(Row::new()
                        .push(TextInput::new("Multiworld target folder", multiworld_path, Message::MultiworldPath))
                        .push(Button::new(Text::new("Browse…")).on_press(Message::BrowseMultiworldPath))
                    )
                    .push(continue_btn)
                    .into()
            }
            Page::InstallMultiworld { .. } => Text::new("Installing multiworld, please wait…").into(),
            Page::AskLaunch { emulator, .. } => {
                let mut col = Column::new();
                col = col.push(Text::new("Multiworld has been installed."));
                match emulator {
                    Emulator::BizHawk => {
                        col = col.push(Text::new("To play multiworld, in BizHawk, select Tools → External Tool → OoTR multiworld."));
                        col = col.push(Checkbox::new(self.open_emulator, "Open BizHawk now", Message::SetOpenEmulator));
                    }
                    Emulator::Project64 => {
                        col = col.push(Text::new("To play multiworld, open the “OoTR Multiworld for Project64” app and follow its instructions."));
                        col = col.push(Checkbox::new(self.open_emulator, "Open Multiworld and Project64 now", Message::SetOpenEmulator));
                    }
                }
                col = col.push(Button::new(Text::new("Finish")).on_press(Message::Continue));
                col.into()
            }
        }
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

#[wheel::main]
fn main(args: Args) -> iced::Result {
    State::run(Settings {
        window: window::Settings {
            size: (400, 300),
            ..window::Settings::default()
        },
        ..Settings::with_flags(args)
    })
}
