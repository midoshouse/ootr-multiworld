#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use {
    std::{
        path::Path,
        sync::Arc,
    },
    directories::UserDirs,
    iced::{
        Application,
        Clipboard,
        Command,
        Element,
        Settings,
        widget::{
            Checkbox,
            Column,
            Radio,
            Row,
            Text,
            button::{
                self,
                Button,
            },
            text_input::{
                self,
                TextInput,
            },
        },
        window,
    },
    itertools::Itertools as _,
    lazy_regex::regex_is_match,
    tokio::fs,
    wheel::traits::IoResultExt as _,
    crate::github::Repo,
};

mod github;

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] FileDialog(#[from] nfd2::error::NfdError),
    #[error(transparent)] Io(#[from] std::io::Error),
    #[error(transparent)] Reqwest(#[from] reqwest::Error),
    #[error(transparent)] Task(#[from] tokio::task::JoinError),
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
    BizHawkInstallPath(String),
    BizHawkLocatePath(String),
    BrowseBizHawkInstallPath,
    BrowseBizHawkLocatePath,
    Continue,
    Error(Arc<Error>),
    Nop,
    SetInstallBizHawk(bool),
    SetOpenBizHawk(bool),
    ToolInstalled,
}

macro_rules! iced_try {
    ($res:expr) => {
        match $res {
            Ok(x) => x,
            Err(e) => return Message::Error(Arc::new(e.into())),
        }
    };
}

enum Page {
    Error(Arc<Error>),
    LocateBizHawk,
    AskLaunch,
}

struct State {
    http_client: reqwest::Client,
    page: Page,
    continue_btn: button::State,
    // first page: locate or install BizHawk
    install_bizhawk: bool,
    bizhawk_install_path: String,
    bizhawk_install_path_state: text_input::State,
    browse_bizhawk_install_path: button::State,
    bizhawk_locate_path: String,
    bizhawk_locate_path_state: text_input::State,
    browse_bizhawk_locate_path: button::State,
    // second page: installation success, ask whether to launch BizHawk now
    open_bizhawk: bool,
    should_exit: bool,
}

impl State {
    fn bizhawk_dir(&self) -> &Path {
        if self.install_bizhawk {
            Path::new(&self.bizhawk_install_path)
        } else {
            Path::new(&self.bizhawk_locate_path)
        }
    }
}

impl Application for State {
    type Executor = iced::executor::Default;
    type Message = Message;
    type Flags = ();

    fn new((): ()) -> (Self, Command<Message>) {
        // check for existing BizHawk install in Downloads folder (where the bizhawk-co-op install scripts places it)
        let (install_bizhawk, bizhawk_install_path, bizhawk_locate_path) = if let Some(user_dirs) = UserDirs::new() {
            let bizhawk_install_path = user_dirs.home_dir().join("bin").join("BizHawk");
            if bizhawk_install_path.exists() {
                (
                    false,
                    bizhawk_install_path.to_str().expect("Windows paths are valid Unicode").to_owned(),
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
                    bizhawk_install_path.into_os_string().into_string().expect("Windows paths are valid Unicode"),
                    default_bizhawk_dir.into_os_string().into_string().expect("Windows paths are valid Unicode"),
                )
            } else {
                (
                    true,
                    bizhawk_install_path.into_os_string().into_string().expect("Windows paths are valid Unicode"),
                    String::default(),
                )
            }
        } else {
            (true, String::default(), String::default())
        };
        (Self {
            http_client: reqwest::Client::builder()
                .user_agent(concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")))
                .http2_prior_knowledge()
                .use_rustls_tls()
                .https_only(true)
                .build().expect("failed to build HTTP client"),
            page: Page::LocateBizHawk,
            continue_btn: button::State::default(),
            bizhawk_install_path_state: text_input::State::default(),
            bizhawk_locate_path_state: text_input::State::default(),
            browse_bizhawk_install_path: button::State::default(),
            browse_bizhawk_locate_path: button::State::default(),
            open_bizhawk: true,
            should_exit: false,
            install_bizhawk, bizhawk_install_path, bizhawk_locate_path,
        }, Command::none())
    }

    fn should_exit(&self) -> bool { self.should_exit }

    fn title(&self) -> String { format!("OoTR Multiworld Installer") }

    fn update(&mut self, msg: Message, _: &mut Clipboard) -> Command<Message> {
        match msg {
            Message::BizHawkInstallPath(new_path) => self.bizhawk_install_path = new_path,
            Message::BizHawkLocatePath(new_path) => self.bizhawk_locate_path = new_path,
            Message::BrowseBizHawkInstallPath => {
                let current_path = self.bizhawk_install_path.clone();
                return async move {
                    match iced_try!(iced_try!(tokio::task::spawn_blocking(move || nfd2::open_pick_folder(Some(Path::new(&current_path)))).await)) {
                        nfd2::Response::Okay(bizhawk_dir) => Message::BizHawkInstallPath(bizhawk_dir.into_os_string().into_string().expect("Windows paths are valid Unicode")),
                        nfd2::Response::OkayMultiple(_) => unreachable!("did not ask for multiple files"),
                        nfd2::Response::Cancel => Message::Nop,
                    }
                }.into()
            }
            Message::BrowseBizHawkLocatePath => {
                let current_path = self.bizhawk_locate_path.clone();
                return async move {
                    match iced_try!(iced_try!(tokio::task::spawn_blocking(move || nfd2::open_pick_folder(Some(Path::new(&current_path)))).await)) {
                        nfd2::Response::Okay(bizhawk_dir) => Message::BizHawkLocatePath(bizhawk_dir.into_os_string().into_string().expect("Windows paths are valid Unicode")),
                        nfd2::Response::OkayMultiple(_) => unreachable!("did not ask for multiple files"),
                        nfd2::Response::Cancel => Message::Nop,
                    }
                }.into()
            }
            Message::Continue => match self.page {
                Page::Error(_) => unreachable!(),
                Page::LocateBizHawk => {
                    if self.install_bizhawk {
                        //TODO also install prereqs
                        //TODO indicate progress
                        let http_client = self.http_client.clone();
                        return async move {
                            let release = iced_try!(iced_try!(Repo::new("TASEmulators", "BizHawk").latest_release(&http_client).await).ok_or(Error::NoBizHawkReleases));
                            #[cfg(all(windows, target_arch = "x86_64"))] let asset = iced_try!(release.assets.into_iter()
                                .filter(|asset| regex_is_match!(r"^BizHawk-.+-win-x64\.zip$", &asset.name))
                                .exactly_one());
                            iced_try!(iced_try!(http_client.get(asset.browser_download_url).send().await).error_for_status());
                            unimplemented!() //TODO
                        }.into()
                    } else {
                        //TODO make sure BizHawk is up to date
                    }
                    let bizhawk_dir = self.bizhawk_dir().to_owned();
                    return async move {
                        let external_tools_dir = bizhawk_dir.join("ExternalTools");
                        iced_try!(fs::create_dir(&external_tools_dir).await.exist_ok());
                        //TODO download latest release instead of embedding in installer
                        iced_try!(fs::write(external_tools_dir.join("multiworld.dll"), include_bytes!("../../../target/release/multiworld.dll")).await);
                        iced_try!(fs::write(external_tools_dir.join("OotrMultiworld.dll"), include_bytes!("../../multiworld-bizhawk/OotrMultiworld/BizHawk/ExternalTools/OotrMultiworld.dll")).await);
                        Message::ToolInstalled
                    }.into()
                }
                Page::AskLaunch => {
                    if self.open_bizhawk {
                        let bizhawk_dir = self.bizhawk_dir();
                        if let Err(e) = std::process::Command::new(bizhawk_dir.join("EmuHawk.exe")).arg("--open-ext-tool-dll=OotrMultiworld").current_dir(bizhawk_dir).spawn() {
                            return async move { Message::Error(Arc::new(e.into())) }.into()
                        }
                    }
                    self.should_exit = true;
                }
            }
            Message::Error(e) => self.page = Page::Error(e),
            Message::Nop => {}
            Message::SetInstallBizHawk(install_bizhawk) => self.install_bizhawk = install_bizhawk,
            Message::SetOpenBizHawk(open_bizhawk) => self.open_bizhawk = open_bizhawk,
            Message::ToolInstalled => self.page = Page::AskLaunch,
        }
        Command::none()
    }

    fn view(&mut self) -> Element<'_, Message> {
        match self.page {
            Page::Error(ref e) => Column::new()
                .push(Text::new("An error occurred during the installation:"))
                .push(Text::new(e.to_string()))
                .push(Text::new("Please report this error to Fenhl. Debug info: {e:?}"))
                .into(),
            Page::LocateBizHawk => {
                let continue_btn = if self.install_bizhawk {
                    let mut btn = Button::new(&mut self.continue_btn, Text::new("Install BizHawk"));
                    if !self.bizhawk_install_path.is_empty() { btn = btn.on_press(Message::Continue) }
                    btn
                } else {
                    let mut btn = Button::new(&mut self.continue_btn, Text::new("Continue"));
                    if !self.bizhawk_locate_path.is_empty() { btn = btn.on_press(Message::Continue) }
                    btn
                };
                Column::new()
                    .push(Radio::new(true, "Install BizHawk to:", Some(self.install_bizhawk), Message::SetInstallBizHawk))
                    .push(Row::new()
                        .push(TextInput::new(&mut self.bizhawk_install_path_state, "BizHawk target folder", &self.bizhawk_install_path, Message::BizHawkInstallPath))
                        .push(Button::new(&mut self.browse_bizhawk_install_path, Text::new("Browse…")).on_press(Message::BrowseBizHawkInstallPath))
                    )
                    .push(Radio::new(false, "I already have BizHawk at:", Some(self.install_bizhawk), Message::SetInstallBizHawk))
                    .push(Row::new()
                        .push(TextInput::new(&mut self.bizhawk_locate_path_state, "The folder with EmuHawk.exe in it", &self.bizhawk_locate_path, Message::BizHawkLocatePath))
                        .push(Button::new(&mut self.browse_bizhawk_locate_path, Text::new("Browse…")).on_press(Message::BrowseBizHawkLocatePath))
                    )
                    .push(continue_btn)
                    .into()
            }
            Page::AskLaunch => Column::new()
                .push(Text::new("Multiworld has been installed."))
                .push(Text::new("To play multiworld, in BizHawk, select Tools → External Tool → OoTR multiworld."))
                .push(Checkbox::new(self.open_bizhawk, "Open BizHawk now", Message::SetOpenBizHawk))
                .push(Button::new(&mut self.continue_btn, Text::new("Finish")).on_press(Message::Continue))
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
