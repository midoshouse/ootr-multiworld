#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use {
    std::{
        cmp::Ordering::*,
        ffi::OsString,
        iter,
        os::windows::ffi::{
            OsStrExt as _,
            OsStringExt as _,
        },
        path::PathBuf,
        ptr::null_mut,
        sync::Arc,
        time::Duration,
    },
    bytes::Bytes,
    dark_light::Mode::*,
    futures::{
        future::Future,
        stream::TryStreamExt as _,
    },
    heim::process::pid_exists,
    iced::{
        Command,
        Length,
        Settings,
        alignment,
        pure::{
            Application,
            Element,
            widget::*,
        },
        window::{
            self,
            Icon,
        },
    },
    itertools::Itertools as _,
    open::that as open,
    semver::Version,
    tokio::{
        io::{
            self,
            AsyncWriteExt as _,
        },
        time::sleep,
    },
    tokio_util::io::StreamReader,
    wheel::fs::{
        self,
        File,
    },
    winapi::um::fileapi::GetFullPathNameW,
    multiworld::{
        github::{
            ReleaseAsset,
            Repo,
        },
        style::Style,
    },
};

#[cfg(target_arch = "x86_64")] const BIZHAWK_PLATFORM_SUFFIX: &str = "-win-x64.zip";

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] Io(#[from] tokio::io::Error),
    #[error(transparent)] Process(#[from] heim::process::ProcessError),
    #[error(transparent)] Reqwest(#[from] reqwest::Error),
    #[error(transparent)] SemVer(#[from] semver::Error),
    #[error(transparent)] Wheel(#[from] wheel::Error),
    #[error(transparent)] Zip(#[from] async_zip::error::ZipError),
    #[error("The update requires an older version of BizHawk. Update manually at your own risk, or ask Fenhl to release a new version.")]
    BizHawkVersionRegression,
    #[error("clone of unexpected message kind")]
    Cloned,
    #[error("latest release does not have a download for this platform")]
    MissingAsset,
    #[error("the file README.md is missing from the download")]
    MissingReadme,
    #[error("there are no released versions")]
    NoReleases,
    #[error("could not find expected BizHawk version in README.md")]
    ReadmeFormat,
    #[error("unexpected file in zip archive")]
    UnexpectedZipEntry,
}

#[derive(Debug)]
enum Message {
    Error(Arc<Error>),
    Exited,
    MultiworldReleaseAsset(reqwest::Client, ReleaseAsset),
    MultiworldResponse(reqwest::Client, reqwest::Response),
    WaitDownload(File),
    UpdateBizHawk(reqwest::Client, Version),
    BizHawkReleaseAsset(reqwest::Client, ReleaseAsset),
    BizHawkResponse(reqwest::Response),
    BizHawkZip(Bytes),
    Launch,
    Done,
    DiscordInvite,
    DiscordChannel,
    NewIssue,
    Cloned,
}

impl Clone for Message {
    fn clone(&self) -> Self {
        match self {
            Self::Error(e) => Self::Error(e.clone()),
            Self::DiscordInvite => Self::DiscordInvite,
            Self::DiscordChannel => Self::DiscordChannel,
            Self::NewIssue => Self::NewIssue,
            _ => Self::Cloned,
        }
    }
}

fn cmd(future: impl Future<Output = Result<Message, Error>> + Send + 'static) -> Command<Message> {
    Command::single(iced_native::command::Action::Future(Box::pin(async move {
        match future.await {
            Ok(msg) => msg,
            Err(e) => Message::Error(Arc::new(e.into())),
        }
    })))
}

enum State {
    WaitExit,
    GetMultiworldRelease,
    DownloadMultiworld,
    ExtractMultiworld,
    GetBizHawkRelease,
    StartDownloadBizHawk,
    DownloadBizHawk,
    ExtractBizHawk,
    Replace,
    WaitDownload,
    Launch,
    Done,
    Error(Arc<Error>),
}

struct App {
    args: Args,
    state: State,
}

impl Application for App {
    type Executor = iced::executor::Default;
    type Message = Message;
    type Flags = Args;

    fn new(args: Args) -> (Self, Command<Message>) {
        let cmd = match args {
            Args::BizHawk { pid, .. } => cmd(async move {
                while pid_exists(pid).await? {
                    sleep(Duration::from_secs(1)).await;
                }
                Ok(Message::Exited)
            }),
            Args::Pj64 { .. } => cmd(async {
                sleep(Duration::from_secs(1)).await;
                Ok(Message::Exited)
            }),
        };
        (App {
            state: State::WaitExit,
            args,
        }, cmd)
    }

    fn background_color(&self) -> iced::Color {
        match dark_light::detect() { //TODO automatically update on system theme change
            Dark => iced::Color::BLACK,
            Light => iced::Color::WHITE,
        }
    }

    fn title(&self) -> String { format!("updating Mido's House Multiworld…") }

    fn update(&mut self, msg: Message) -> Command<Message> {
        match msg {
            Message::Error(e) => self.state = State::Error(e),
            Message::Exited => {
                self.state = State::GetMultiworldRelease;
                let (asset_name, script_name) = match self.args {
                    Args::BizHawk { .. } => ("multiworld-bizhawk.zip", None),
                    Args::Pj64 { .. } => ("multiworld-pj64.exe", Some("ootrmw-pj64.js")),
                };
                return cmd(async move {
                    let http_client = reqwest::Client::builder()
                        .user_agent(concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")))
                        .use_rustls_tls()
                        .https_only(true)
                        .http2_prior_knowledge()
                        .build()?;
                    let release = Repo::new("midoshouse", "ootr-multiworld").latest_release(&http_client).await?.ok_or(Error::NoReleases)?;
                    if let Some(script_name) = script_name {
                        let (script,) = release.assets.iter()
                            .filter(|asset| asset.name == script_name)
                            .collect_tuple().ok_or(Error::MissingAsset)?;
                        let _ = script; //TODO check if script contents differ from current. If so, ask for elevation and replace script
                    }
                    let (asset,) = release.assets.into_iter()
                        .filter(|asset| asset.name == asset_name)
                        .collect_tuple().ok_or(Error::MissingAsset)?;
                    Ok(Message::MultiworldReleaseAsset(http_client, asset))
                })
            }
            Message::MultiworldReleaseAsset(http_client, asset) => {
                self.state = State::DownloadMultiworld;
                return cmd(async move {
                    Ok(Message::MultiworldResponse(http_client.clone(), http_client.get(asset.browser_download_url).send().await?.error_for_status()?))
                })
            }
            Message::MultiworldResponse(http_client, response) => match self.args {
                Args::BizHawk { ref path, ref local_bizhawk_version, .. } => {
                    self.state = State::ExtractMultiworld;
                    let path = path.clone();
                    let local_bizhawk_version = local_bizhawk_version.clone();
                    return cmd(async move {
                        let mut zip_file = StreamReader::new(response.bytes_stream().map_err(|e| io::Error::new(io::ErrorKind::Other, e)));
                        let mut zip_file = async_zip::read::stream::ZipFileReader::new(&mut zip_file);
                        let mut required_bizhawk_version = None;
                        while let Some(entry) = zip_file.entry_reader().await? {
                            match entry.entry().name() {
                                "README.txt" => {
                                    let (readme_prefix, _) = include_str!("../../../assets/bizhawk-readme.txt").split_once("{}").expect("failed to parse readme template");
                                    required_bizhawk_version = Some(
                                        entry.read_to_string_crc().await?
                                            .strip_prefix(readme_prefix).ok_or(Error::ReadmeFormat)?
                                            .split_once(". ").ok_or(Error::ReadmeFormat)?
                                            .0.parse()?
                                    );
                                }
                                "OotrMultiworld.dll" => {
                                    let external_tools = path.join("ExternalTools");
                                    fs::create_dir_all(&external_tools).await?;
                                    entry.copy_to_end_crc(&mut File::create(external_tools.join("OotrMultiworld.dll")).await?, 64 * 1024).await?;
                                }
                                "multiworld.dll" => {
                                    let external_tools = path.join("ExternalTools");
                                    fs::create_dir_all(&external_tools).await?;
                                    entry.copy_to_end_crc(&mut File::create(external_tools.join("multiworld.dll")).await?, 64 * 1024).await?;
                                }
                                _ => return Err(Error::UnexpectedZipEntry),
                            }
                        }
                        let required_bizhawk_version = required_bizhawk_version.ok_or(Error::MissingReadme)?;
                        match local_bizhawk_version.cmp(&required_bizhawk_version) {
                            Less => Ok(Message::UpdateBizHawk(http_client, required_bizhawk_version)),
                            Equal => Ok(Message::Launch),
                            Greater => Err(Error::BizHawkVersionRegression),
                        }
                    })
                }
                Args::Pj64 { ref path } => {
                    self.state = State::Replace;
                    let path = path.clone();
                    return cmd(async move {
                        let mut data = response.bytes_stream();
                        let mut exe_file = File::create(path).await?;
                        while let Some(chunk) = data.try_next().await? {
                            exe_file.write_all(chunk.as_ref()).await?;
                        }
                        Ok(Message::WaitDownload(exe_file))
                    })
                }
            },
            Message::WaitDownload(exe_file) => {
                self.state = State::WaitDownload;
                return cmd(async move {
                    exe_file.sync_all().await?;
                    Ok(Message::Launch)
                })
            }
            Message::UpdateBizHawk(client, required_version) => {
                self.state = State::GetBizHawkRelease;
                return cmd(async move {
                    //TODO also update prereqs
                    let version_str = required_version.to_string();
                    let version_str = version_str.trim_end_matches(".0");
                    let release = Repo::new("TASEmulators", "BizHawk").release_by_tag(&client, version_str).await?.ok_or(Error::NoReleases)?;
                    let (asset,) = release.assets.into_iter()
                        .filter(|asset| asset.name.ends_with(BIZHAWK_PLATFORM_SUFFIX))
                        .collect_tuple().ok_or(Error::MissingAsset)?;
                    Ok(Message::BizHawkReleaseAsset(client, asset))
                })
            }
            Message::BizHawkReleaseAsset(client, asset) => {
                self.state = State::StartDownloadBizHawk;
                return cmd(async move {
                    Ok(Message::BizHawkResponse(client.get(asset.browser_download_url).send().await?.error_for_status()?))
                })
            }
            Message::BizHawkResponse(response) => {
                self.state = State::DownloadBizHawk;
                return cmd(async move {
                    Ok(Message::BizHawkZip(response.bytes().await?))
                })
            }
            Message::BizHawkZip(mut response) => if let Args::BizHawk { ref path, .. } = self.args {
                self.state = State::ExtractBizHawk;
                let path = path.clone();
                return cmd(async move {
                    let mut zip_file = async_zip::read::mem::ZipFileReader::new(&mut response).await?;
                    let entries = zip_file.entries().iter().enumerate().map(|(idx, entry)| (idx, entry.dir(), path.join(entry.name()))).collect_vec();
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
                    Ok(Message::Launch)
                })
            },
            Message::Launch => match self.args {
                Args::BizHawk { ref path, .. } => {
                    self.state = State::Launch;
                    let path = path.clone();
                    let path_wide = path.as_os_str().encode_wide().chain(iter::once(0)).collect_vec();
                    return cmd(async move {
                        let path = unsafe {
                            let mut buf = vec![0; 260];
                            let result = GetFullPathNameW(path_wide.as_ptr(), buf.len().try_into().expect("buffer too large"), buf.as_mut_ptr(), null_mut());
                            PathBuf::from(if result == 0 {
                                drop(path_wide);
                                return Err(Error::Io(io::Error::last_os_error()))
                            } else if result > u32::try_from(buf.len()).expect("buffer too large") {
                                buf = vec![0; result.try_into().expect("path too long")];
                                let result = GetFullPathNameW(path_wide.as_ptr(), buf.len().try_into().expect("buffer too large"), buf.as_mut_ptr(), null_mut());
                                drop(path_wide);
                                if result == 0 {
                                    return Err(Error::Io(io::Error::last_os_error()))
                                } else if result > u32::try_from(buf.len()).expect("buffer too large") {
                                    panic!("path too long")
                                } else {
                                    OsString::from_wide(&buf[0..result.try_into().expect("path too long")])
                                }
                            } else {
                                drop(path_wide);
                                OsString::from_wide(&buf[0..result.try_into().expect("path too long")])
                            })
                        };
                        std::process::Command::new(path.join("EmuHawk.exe")).arg("--open-ext-tool-dll=OotrMultiworld").current_dir(path).spawn()?;
                        Ok(Message::Done)
                    })
                }
                Args::Pj64 { ref path } => {
                    self.state = State::Launch;
                    let path = path.clone();
                    return cmd(async move {
                        std::process::Command::new(path).spawn()?;
                        Ok(Message::Done)
                    })
                }
            },
            Message::Done => self.state = State::Done,
            Message::DiscordInvite => if let Err(e) = open("https://discord.gg/BGRrKKn") {
                self.state = State::Error(Arc::new(e.into()));
            },
            Message::DiscordChannel => if let Err(e) = open("https://discord.com/channels/274180765816848384/476723801032491008") {
                self.state = State::Error(Arc::new(e.into()));
            },
            Message::NewIssue => if let Err(e) = open("https://github.com/midoshouse/ootr-multiworld/issues/new") {
                self.state = State::Error(Arc::new(e.into()));
            },
            Message::Cloned => self.state = State::Error(Arc::new(Error::Cloned)),
        }
        Command::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let system_theme = dark_light::detect(); //TODO automatically update on system theme change
        let text_color = match system_theme {
            Dark => iced::Color::WHITE,
            Light => iced::Color::BLACK,
        };
        match self.state {
            State::WaitExit => match self.args {
                Args::BizHawk { .. } => Column::new()
                    .push(Text::new("An update for Mido's House Multiworld for BizHawk is available.").color(text_color))
                    .push(Text::new("Please close BizHawk to start the update.").color(text_color))
                    .into(),
                Args::Pj64 { .. } => Column::new()
                    .push(Text::new("An update for Mido's House Multiworld for Project64 is available.").color(text_color))
                    .push(Text::new("Waiting to make sure the old version has exited…").color(text_color))
                    .into(),
            },
            State::GetMultiworldRelease => Text::new("Checking latest release…").color(text_color).into(),
            State::DownloadMultiworld => Text::new("Starting download…").color(text_color).into(),
            State::ExtractMultiworld => Text::new("Downloading and extracting multiworld…").color(text_color).into(),
            State::GetBizHawkRelease => Text::new("Getting BizHawk download link…").color(text_color).into(),
            State::StartDownloadBizHawk => Text::new("Starting BizHawk download…").color(text_color).into(),
            State::DownloadBizHawk => Text::new("Downloading BizHawk…").color(text_color).into(),
            State::ExtractBizHawk => Text::new("Extracting BizHawk…").color(text_color).into(),
            State::Replace => Text::new("Downloading update…").color(text_color).into(),
            State::WaitDownload => Text::new("Finishing download…").color(text_color).into(),
            State::Launch => Text::new("Starting new version…").color(text_color).into(),
            State::Done => Text::new("Closing updater…").color(text_color).into(),
            State::Error(ref e) => Column::new()
                .push(Text::new("Error").size(24).width(Length::Fill).horizontal_alignment(alignment::Horizontal::Center).color(text_color))
                .push(Text::new(e.to_string()).color(text_color))
                .push(Text::new(format!("debug info: {e:?}")).color(text_color))
                .push(Text::new("Support").size(24).width(Length::Fill).horizontal_alignment(alignment::Horizontal::Center).color(text_color))
                .push(Text::new("• Ask in #setup-support on the OoT Randomizer Discord. Feel free to ping @Fenhl#4813.").color(text_color))
                .push(Row::new()
                    .push(Button::new(Text::new("invite link").color(text_color)).on_press(Message::DiscordInvite).style(Style(system_theme)))
                    .push(Button::new(Text::new("direct channel link").color(text_color)).on_press(Message::DiscordChannel).style(Style(system_theme)))
                )
                .push(Text::new("• Ask in #general on the OoTR MW Tournament Discord.").color(text_color))
                .push(Row::new()
                    .push(Text::new("• Or ").color(text_color))
                    .push(Button::new(Text::new("open an issue").color(text_color)).on_press(Message::NewIssue).style(Style(system_theme)))
                )
                .into(),
        }
    }

    fn should_exit(&self) -> bool {
        matches!(self.state, State::Done)
    }
}

#[derive(clap::Parser)]
#[clap(rename_all = "lower", version)]
enum Args {
    BizHawk {
        #[clap(parse(from_os_str))]
        path: PathBuf,
        pid: u32,
        local_bizhawk_version: Version,
    },
    Pj64 {
        #[clap(parse(from_os_str))]
        path: PathBuf,
    },
}

#[derive(Debug, thiserror::Error)]
enum MainError {
    #[error(transparent)] Iced(#[from] iced::Error),
    #[error(transparent)] Icon(#[from] iced::window::icon::Error),
}

#[wheel::main]
fn main(args: Args) -> Result<(), MainError> {
    let icon = ::image::load_from_memory(include_bytes!("../../../assets/icon.ico")).expect("failed to load embedded DynamicImage").to_rgba8();
    App::run(Settings {
        window: window::Settings {
            size: (320, 240),
            icon: Some(Icon::from_rgba(icon.as_flat_samples().as_slice().to_owned(), icon.width(), icon.height())?),
            ..window::Settings::default()
        },
        ..Settings::with_flags(args)
    })?;
    Ok(())
}
