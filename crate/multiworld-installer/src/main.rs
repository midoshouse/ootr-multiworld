#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]
#![forbid(unsafe_code)]

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use {
    std::{
        future::Future,
        path::{
            Path,
            PathBuf,
        },
        sync::Arc,
    },
    directories::UserDirs,
    iced::{
        Command,
        Settings,
        pure::{
            Application,
            Element,
            widget::*,
        },
        window,
    },
    itertools::Itertools as _,
    lazy_regex::regex_is_match,
    rfd::AsyncFileDialog,
    tokio::fs::{
        self,
        File,
    },
    wheel::traits::IoResultExt as _,
    crate::github::Repo,
};

mod github;

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] Io(#[from] std::io::Error),
    #[error(transparent)] Reqwest(#[from] reqwest::Error),
    #[error(transparent)] Task(#[from] tokio::task::JoinError),
    #[error(transparent)] Zip(#[from] async_zip::error::ZipError),
    #[error("got zero elements when exactly one was expected")]
    ExactlyOneEmpty,
    #[error("got at least 2 elements when exactly one was expected")]
    ExactlyOneMultiple,
    #[error("No BizHawk releases found")]
    NoBizHawkReleases,
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
    BizHawkPath(String),
    BrowseBizHawkPath,
    Continue,
    Error(Arc<Error>),
    InstallTool,
    Nop,
    SetInstallBizHawk(bool),
    SetOpenBizHawk(bool),
    ToolInstalled,
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
    LocateBizHawk,
    InstallBizHawk,
    InstallBizHawkTool,
    AskLaunch,
}

struct State {
    http_client: reqwest::Client,
    page: Page,
    // first page: locate or install BizHawk
    install_bizhawk: bool,
    bizhawk_path: String,
    // second page: installation success, ask whether to launch BizHawk now
    open_bizhawk: bool,
    should_exit: bool,
}

impl Application for State {
    type Executor = iced::executor::Default;
    type Message = Message;
    type Flags = ();

    fn new((): ()) -> (Self, Command<Message>) {
        // check for existing BizHawk install in Downloads folder (where the bizhawk-co-op install scripts places it)
        let (install_bizhawk, bizhawk_path) = if let Some(user_dirs) = UserDirs::new() {
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
        };
        (Self {
            http_client: reqwest::Client::builder()
                .user_agent(concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")))
                .http2_prior_knowledge()
                .use_rustls_tls()
                .https_only(true)
                .build().expect("failed to build HTTP client"),
            page: Page::LocateBizHawk, //TODO choose between BizHawk and Project64
            open_bizhawk: true,
            should_exit: false,
            install_bizhawk, bizhawk_path,
        }, Command::none())
    }

    fn should_exit(&self) -> bool { self.should_exit }

    fn title(&self) -> String { format!("OoTR Multiworld Installer") }

    fn update(&mut self, msg: Message) -> Command<Message> {
        match msg {
            Message::BizHawkPath(new_path) => self.bizhawk_path = new_path,
            Message::BrowseBizHawkPath => {
                let current_path = self.bizhawk_path.clone();
                let install_bizhawk = self.install_bizhawk;
                return cmd(async move {
                    Ok(if let Some(bizhawk_dir) = AsyncFileDialog::new().set_title(if install_bizhawk { "Choose Location for BizHawk Installation" } else { "Select BizHawk Folder" }).set_directory(Path::new(&current_path)).pick_folder().await {
                        Message::BizHawkPath(bizhawk_dir.path().to_str().expect("Windows paths are valid Unicode").to_owned())
                    } else {
                        Message::Nop
                    })
                })
            }
            Message::Continue => match self.page {
                Page::Error(_) => unreachable!(),
                Page::LocateBizHawk => {
                    self.page = Page::InstallBizHawk;
                    if self.install_bizhawk {
                        //TODO also install prereqs
                        //TODO indicate progress
                        let http_client = self.http_client.clone();
                        let bizhawk_dir = PathBuf::from(self.bizhawk_path.clone());
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
                            Ok(Message::InstallTool)
                        })
                    } else {
                        //TODO make sure BizHawk is up to date
                        return cmd(async { Ok(Message::InstallTool) })
                    }
                }
                Page::InstallBizHawk | Page::InstallBizHawkTool => unreachable!(),
                Page::AskLaunch => {
                    if self.open_bizhawk {
                        let bizhawk_dir = PathBuf::from(self.bizhawk_path.clone());
                        if let Err(e) = std::process::Command::new(bizhawk_dir.join("EmuHawk.exe")).arg("--open-ext-tool-dll=OotrMultiworld").current_dir(bizhawk_dir).spawn() {
                            return cmd(async move { Err(e.into()) })
                        }
                    }
                    self.should_exit = true;
                }
            }
            Message::Error(e) => self.page = Page::Error(e),
            Message::InstallTool => {
                self.page = Page::InstallBizHawkTool;
                let bizhawk_dir = PathBuf::from(self.bizhawk_path.clone());
                return cmd(async move {
                    let external_tools_dir = bizhawk_dir.join("ExternalTools");
                    fs::create_dir(&external_tools_dir).await.exist_ok()?;
                    //TODO download latest release instead of embedding in installer
                    fs::write(external_tools_dir.join("multiworld.dll"), include_bytes!("../../../target/release/multiworld.dll")).await?;
                    fs::write(external_tools_dir.join("OotrMultiworld.dll"), include_bytes!("../../multiworld-bizhawk/OotrMultiworld/BizHawk/ExternalTools/OotrMultiworld.dll")).await?;
                    Ok(Message::ToolInstalled)
                })
            }
            Message::Nop => {}
            Message::SetInstallBizHawk(install_bizhawk) => self.install_bizhawk = install_bizhawk,
            Message::SetOpenBizHawk(open_bizhawk) => self.open_bizhawk = open_bizhawk,
            Message::ToolInstalled => self.page = Page::AskLaunch,
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
            Page::LocateBizHawk => {
                let continue_btn = if self.install_bizhawk {
                    let mut btn = Button::new(Text::new("Install BizHawk"));
                    if !self.bizhawk_path.is_empty() { btn = btn.on_press(Message::Continue) }
                    btn
                } else {
                    let mut btn = Button::new(Text::new("Continue"));
                    if !self.bizhawk_path.is_empty() { btn = btn.on_press(Message::Continue) }
                    btn
                };
                Column::new()
                    .push(Radio::new(true, "Install BizHawk to:", Some(self.install_bizhawk), Message::SetInstallBizHawk))
                    .push(Radio::new(false, "I already have BizHawk at:", Some(self.install_bizhawk), Message::SetInstallBizHawk))
                    .push(Row::new()
                        .push(TextInput::new(if self.install_bizhawk { "BizHawk target folder" } else { "The folder with EmuHawk.exe in it" }, &self.bizhawk_path, Message::BizHawkPath))
                        .push(Button::new(Text::new("Browse…")).on_press(Message::BrowseBizHawkPath))
                    )
                    .push(continue_btn)
                    .into()
            }
            Page::InstallBizHawk => Text::new("Installing BizHawk, please wait…").into(),
            Page::InstallBizHawkTool => Text::new("Installing BizHawk plugin, please wait…").into(),
            Page::AskLaunch => Column::new()
                .push(Text::new("Multiworld has been installed."))
                .push(Text::new("To play multiworld, in BizHawk, select Tools → External Tool → OoTR multiworld."))
                .push(Checkbox::new(self.open_bizhawk, "Open BizHawk now", Message::SetOpenBizHawk))
                .push(Button::new(Text::new("Finish")).on_press(Message::Continue))
                .into(),
        }
    }
}

#[wheel::main]
fn main() -> iced::Result {
    State::run(Settings {
        window: window::Settings {
            size: (400, 300),
            ..window::Settings::default()
        },
        ..Settings::default()
    })
}
