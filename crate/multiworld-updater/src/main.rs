#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use {
    std::{
        cmp::Ordering::*,
        convert::Infallible as Never,
        env,
        io::prelude::*,
        path::{
            Path,
            PathBuf,
        },
        sync::Arc,
        time::Duration,
    },
    bytes::Bytes,
    chrono::prelude::*,
    dark_light::Mode::{
        Dark,
        Light,
    },
    futures::{
        future::{
            self,
            Future,
        },
        stream::TryStreamExt as _,
    },
    iced::{
        Element,
        Length,
        Size,
        Task,
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
    open::that as open,
    semver::Version,
    serenity::utils::MessageBuilder,
    sysinfo::{
        Pid,
        ProcessRefreshKind,
        ProcessesToUpdate,
    },
    tokio::{
        io::{
            self,
            AsyncWriteExt as _,
        },
        time::sleep,
    },
    tokio_util::io::StreamReader,
    url::Url,
    wheel::{
        fs::{
            self,
            File,
        },
        io_error_from_reqwest,
        traits::{
            IoResultExt as _,
            ResultNeverErrExt as _,
            SyncCommandOutputExt as _,
        },
    },
    multiworld::{
        config::Config,
        github::{
            ReleaseAsset,
            Repo,
        },
    },
    crate::util::absolute_path,
};
#[cfg(unix)] use {
    std::io::Cursor,
    xdg::BaseDirectories,
};
#[cfg(windows)] use directories::ProjectDirs;
#[cfg(target_os = "linux")] use {
    gio::prelude::*,
    multiworld::fix_bizhawk_permissions,
};

mod util;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))] const BIZHAWK_PLATFORM_SUFFIX: &str = "-linux-x64.tar.gz";
#[cfg(all(target_os = "windows", target_arch = "x86_64"))] const BIZHAWK_PLATFORM_SUFFIX: &str = "-win-x64.zip";

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] Config(#[from] multiworld::config::Error),
    #[error(transparent)] Icon(#[from] icon::Error),
    #[error(transparent)] Reqwest(#[from] reqwest::Error),
    #[error(transparent)] SemVer(#[from] semver::Error),
    #[error(transparent)] Task(#[from] tokio::task::JoinError),
    #[error(transparent)] Url(#[from] url::ParseError),
    #[error(transparent)] Wheel(#[from] wheel::Error),
    #[error(transparent)] Zip(#[from] async_zip::error::ZipError),
    #[error("The update requires an older version of BizHawk. Update manually at your own risk, or ask Fenhl to release a new version.")]
    BizHawkVersionRegression,
    #[error("clone of unexpected message kind")]
    Cloned(String),
    #[error("tried to copy debug info or open a GitHub issue with no active error")]
    CopyDebugInfo,
    #[error("latest release does not have a download for this platform")]
    MissingAsset,
    #[error("the file README.md is missing from the download")]
    MissingReadme,
    #[error("there are no released versions")]
    NoReleases,
    #[error("failed to update Project64 script")]
    Pj64Script {
        temp_path: PathBuf,
        script_path: PathBuf,
        source: wheel::Error,
    },
    #[error("failed to locate Program Files folder")]
    ProgramFiles,
    #[error("could not find expected BizHawk version in README.md")]
    ReadmeFormat,
    #[error("unexpected file in zip archive")]
    UnexpectedZipEntry,
}

impl Error {
    fn to_markdown(&self) -> String {
        MessageBuilder::default()
            .push_line(format!("error while trying to update Mido's House Multiworld from version {}{}:", env!("CARGO_PKG_VERSION"), {
                #[cfg(debug_assertions)] { " (debug)" }
                #[cfg(not(debug_assertions))] { "" }
            })) //TODO also show new version
            .push_line_safe(self.to_string())
            .push_codeblock_safe(format!("{self:?}"), Some("rust"))
            .build()
    }
}

#[derive(Debug)]
enum Message {
    Error(Arc<Error>),
    CopyDebugInfo,
    Exited,
    MultiworldReleaseAssets(reqwest::Client, ReleaseAsset, Option<ReleaseAsset>),
    MultiworldResponse(reqwest::Client, reqwest::Response),
    WaitDownload(File),
    AskBizHawkUpdate(reqwest::Client, Version),
    UpdateBizHawk(reqwest::Client, Version),
    BizHawkReleaseAsset(reqwest::Client, ReleaseAsset),
    BizHawkResponse(reqwest::Response),
    BizHawkZip(Bytes),
    Launch,
    Done,
    DiscordInvite,
    DiscordChannel,
    NewIssue,
    Cloned(String),
}

impl Clone for Message {
    fn clone(&self) -> Self {
        match self {
            Self::Error(e) => Self::Error(e.clone()),
            Self::CopyDebugInfo => Self::CopyDebugInfo,
            Self::DiscordInvite => Self::DiscordInvite,
            Self::DiscordChannel => Self::DiscordChannel,
            Self::NewIssue => Self::NewIssue,
            _ => Self::Cloned(format!("{self:?}")),
        }
    }
}

fn cmd(future: impl Future<Output = Result<Message, Error>> + Send + 'static) -> Task<Message> {
    Task::future(Box::pin(async move {
        match future.await {
            Ok(msg) => msg,
            Err(e) => Message::Error(Arc::new(e.into())),
        }
    }))
}

enum State {
    WaitExit,
    GetMultiworldRelease,
    DownloadMultiworld,
    ExtractMultiworld,
    AskBizHawkUpdate(reqwest::Client, Version),
    GetBizHawkRelease,
    StartDownloadBizHawk,
    DownloadBizHawk,
    ExtractBizHawk,
    Replace,
    WaitDownload,
    Launch,
    Done,
    Error(Arc<Error>, bool),
}

struct App {
    args: EmuArgs,
    state: State,
}

impl App {
    fn new(icon_error: Option<icon::Error>, args: EmuArgs) -> Self {
        Self {
            state: if let Some(e) = icon_error {
                State::Error(Arc::new(e.into()), false)
            } else {
                State::WaitExit
            },
            args,
        }
    }

    fn title(&self) -> String { format!("updating Mido's House Multiworld…") }

    fn theme(&self) -> Theme {
        //TODO automatically update on system theme change (https://github.com/gtk-rs/gtk-rs-core/discussions/1278 for GNOME, https://github.com/frewsxcv/rust-dark-light/pull/26 for other platforms)
        #[cfg(target_os = "linux")] {
            let settings = gio::Settings::new("org.gnome.desktop.interface");
            if settings.settings_schema().map_or(false, |schema| schema.has_key("color-scheme")) {
                match settings.string("color-scheme").as_str() {
                    "prefer-light" => return Theme::Light,
                    "prefer-dark" => return Theme::Dark,
                    _ => {}
                }
            }
        }
        match dark_light::detect() {
            Dark => Theme::Dark,
            Light | dark_light::Mode::Default => Theme::Light,
        }
    }

    fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::Error(e) => self.state = State::Error(e, false),
            Message::CopyDebugInfo => if let State::Error(ref e, ref mut debug_info_copied) = self.state {
                *debug_info_copied = true;
                return clipboard::write(e.to_markdown())
            } else {
                self.state = State::Error(Arc::new(Error::CopyDebugInfo), false);
            },
            Message::Exited => {
                self.state = State::GetMultiworldRelease;
                let (asset_name, script_name) = match self.args {
                    EmuArgs::EverDrive { .. } => {
                        #[cfg(target_os = "linux")] { ("multiworld-gui-linux", None) }
                        #[cfg(target_os = "windows")] { ("multiworld-pj64.exe", None) }
                    }
                    EmuArgs::BizHawk { .. } => {
                        #[cfg(target_os = "linux")] { ("multiworld-bizhawk-linux.zip", None) }
                        #[cfg(target_os = "windows")] { ("multiworld-bizhawk.zip", None) }
                    }
                    EmuArgs::Pj64 { .. } => ("multiworld-pj64.exe", Some("ootrmw-pj64.js")),
                };
                return cmd(async move {
                    let http_client = reqwest::Client::builder()
                        .user_agent(concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")))
                        .use_rustls_tls()
                        .https_only(true)
                        .http2_prior_knowledge()
                        .build()?;
                    let release = Repo::new("midoshouse", "ootr-multiworld").latest_release(&http_client).await?.ok_or(Error::NoReleases)?;
                    let mut asset = None;
                    let mut script = None;
                    for iter_asset in release.assets {
                        if iter_asset.name == asset_name {
                            asset = Some(iter_asset);
                        } else if Some(&*iter_asset.name) == script_name {
                            script = Some(iter_asset);
                        }
                    }
                    if script_name.is_some() && script.is_none() { return Err(Error::MissingAsset) }
                    Ok(Message::MultiworldReleaseAssets(http_client, asset.ok_or(Error::MissingAsset)?, script))
                })
            }
            Message::MultiworldReleaseAssets(http_client, asset, script) => {
                self.state = State::DownloadMultiworld;
                return cmd(async move {
                    if let Some(script) = script {
                        let config = Config::load().await?;
                        let script_path = if let Some(ref script_path) = config.pj64_script_path {
                            script_path.clone()
                        } else {
                            let program_files = env::var_os("ProgramFiles(x86)").or_else(|| env::var_os("ProgramFiles")).ok_or(Error::ProgramFiles)?;
                            PathBuf::from(program_files).join("Project64 3.0").join("Scripts").join("ootrmw.js")
                        };
                        let old_script = fs::read(&script_path).await?;
                        let new_script = http_client.get(script.browser_download_url).send().await?.error_for_status()?.bytes().await?;
                        if old_script != new_script {
                            let temp_path = tokio::task::spawn_blocking(|| tempfile::Builder::default().prefix("ootrmw-pj64").suffix(".js").tempfile()).await?.at_unknown()?;
                            io::copy_buf(&mut &*new_script, &mut tokio::fs::File::from_std(temp_path.reopen().at(&temp_path)?)).await.at(&temp_path)?;
                            tokio::task::spawn_blocking(move || {
                                if let Err(source) = runas::Command::new(env::current_exe().at_unknown()?).arg("pj64script").arg(temp_path.as_ref()).arg(&script_path).gui(true).status().at_command("runas")?.check("runas") {
                                    return Err(Error::Pj64Script {
                                        temp_path: temp_path.as_ref().to_owned(),
                                        script_path, source,
                                    })
                                }
                                drop(temp_path);
                                Ok(())
                            }).await??;
                        }
                    }
                    Ok(Message::MultiworldResponse(http_client.clone(), http_client.get(asset.browser_download_url).send().await?.error_for_status()?))
                })
            }
            Message::MultiworldResponse(http_client, response) => match self.args {
                EmuArgs::BizHawk { ref path, ref local_bizhawk_version, .. } => {
                    self.state = State::ExtractMultiworld;
                    let path = path.clone();
                    let local_bizhawk_version = local_bizhawk_version.clone();
                    return cmd(async move {
                        let mut zip_file = StreamReader::new(response.bytes_stream().map_err(io_error_from_reqwest));
                        let mut zip_file = async_zip::base::read::stream::ZipFileReader::with_tokio(&mut zip_file);
                        let mut required_bizhawk_version = None;
                        while let Some(mut entry) = zip_file.next_with_entry().await? {
                            match entry.reader().entry().filename().as_str()? {
                                "README.txt" => {
                                    #[cfg(target_os = "linux")] let readme_template = include_str!("../../../assets/bizhawk-readme-linux.txt");
                                    #[cfg(target_os = "windows")] let readme_template = include_str!("../../../assets/bizhawk-readme-windows.txt");
                                    let (readme_prefix, _) = readme_template.split_once("{}").expect("failed to parse readme template");
                                    let mut buf = String::default();
                                    entry.reader_mut().read_to_string_checked(&mut buf).await?;
                                    required_bizhawk_version = Some(
                                        buf
                                            .strip_prefix(readme_prefix).ok_or(Error::ReadmeFormat)?
                                            .split_once(". ").ok_or(Error::ReadmeFormat)?
                                            .0.parse()?
                                    );
                                }
                                "OotrMultiworld.dll" => {
                                    let external_tools = path.join("ExternalTools");
                                    fs::create_dir_all(&external_tools).await?;
                                    let mut buf = Vec::default();
                                    entry.reader_mut().read_to_end_checked(&mut buf).await?;
                                    fs::write(external_tools.join("OotrMultiworld.dll"), &buf).await?;
                                }
                                #[cfg(target_os = "linux")] "libmultiworld.so" => {
                                    let dlls = path.join("dll");
                                    fs::create_dir_all(&dlls).await?;
                                    let mut buf = Vec::default();
                                    entry.reader_mut().read_to_end_checked(&mut buf).await?;
                                    fs::write(dlls.join("libmultiworld.so"), &buf).await?;
                                }
                                #[cfg(target_os = "windows")] "multiworld.dll" => {
                                    let external_tools = path.join("ExternalTools"); //TODO test if placing in `dll` works, use that and clean up `ExternalTools` if it does
                                    fs::create_dir_all(&external_tools).await?;
                                    let mut buf = Vec::default();
                                    entry.reader_mut().read_to_end_checked(&mut buf).await?;
                                    fs::write(external_tools.join("multiworld.dll"), &buf).await?;
                                }
                                _ => return Err(Error::UnexpectedZipEntry),
                            }
                            zip_file = entry.done().await?;
                        }
                        let required_bizhawk_version = required_bizhawk_version.ok_or(Error::MissingReadme)?;
                        match local_bizhawk_version.cmp(&required_bizhawk_version) {
                            Less => Ok(Message::AskBizHawkUpdate(http_client, required_bizhawk_version)),
                            Equal => Ok(Message::Launch),
                            Greater => Err(Error::BizHawkVersionRegression),
                        }
                    })
                }
                EmuArgs::EverDrive { ref path, .. } | EmuArgs::Pj64 { ref path, .. } => {
                    self.state = State::Replace;
                    let path = path.clone();
                    return cmd(async move {
                        let mut data = response.bytes_stream();
                        let mut exe_file = File::create(&path).await?;
                        while let Some(chunk) = data.try_next().await? {
                            exe_file.write_all(chunk.as_ref()).await.at(&path)?;
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
            Message::AskBizHawkUpdate(client, required_version) => {
                self.state = State::AskBizHawkUpdate(client, required_version);
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
            Message::BizHawkZip(response) => if let EmuArgs::BizHawk { ref path, .. } = self.args {
                self.state = State::ExtractBizHawk;
                let path = path.clone();
                #[cfg(target_os = "linux")] return cmd(async move {
                    let tar_file = async_compression::tokio::bufread::GzipDecoder::new(Cursor::new(Vec::from(response)));
                    tokio_tar::Archive::new(tar_file).unpack(&path).await.at(&path)?;
                    fix_bizhawk_permissions(path).await?;
                    Ok(Message::Launch)
                });
                #[cfg(target_os = "windows")] return cmd(async move {
                    let zip_file = async_zip::base::read::mem::ZipFileReader::new(response.into()).await?;
                    let entries = zip_file.file().entries().iter().enumerate().map(|(idx, entry)| Ok((idx, entry.filename().as_str()?.ends_with('/'), path.join(entry.filename().as_str()?)))).try_collect::<_, Vec<_>, Error>()?;
                    for (idx, is_dir, path) in entries {
                        if is_dir {
                            fs::create_dir_all(path).await?;
                        } else {
                            if let Some(parent) = path.parent() {
                                fs::create_dir_all(parent).await?;
                            }
                            let mut buf = Vec::default();
                            zip_file.reader_with_entry(idx).await?.read_to_end_checked(&mut buf).await?;
                            fs::write(path, &buf).await?;
                        }
                    }
                    Ok(Message::Launch)
                });
            },
            Message::Launch => match self.args {
                EmuArgs::BizHawk { ref path, .. } => {
                    self.state = State::Launch;
                    let path = path.clone();
                    return cmd(async move {
                        let path = absolute_path(path).await?;
                        let emuhawk_path = path.join("EmuHawk.exe");
                        std::process::Command::new(&emuhawk_path).arg("--open-ext-tool-dll=OotrMultiworld").current_dir(path).spawn().at(emuhawk_path)?;
                        Ok(Message::Done)
                    })
                }
                EmuArgs::EverDrive { ref path, .. } | EmuArgs::Pj64 { ref path, .. } => {
                    self.state = State::Launch;
                    let path = path.clone();
                    return cmd(async move {
                        std::process::Command::new(&path).spawn().at(path)?;
                        Ok(Message::Done)
                    })
                }
            },
            Message::Done => {
                self.state = State::Done;
                return iced::exit()
            }
            Message::DiscordInvite => if let Err(e) = open("https://discord.gg/BGRrKKn") {
                self.state = State::Error(Arc::new(Err::<Never, _>(e).at_unknown().never_unwrap_err().into()), false);
            },
            Message::DiscordChannel => if let Err(e) = open("https://discord.com/channels/274180765816848384/476723801032491008") {
                self.state = State::Error(Arc::new(Err::<Never, _>(e).at_unknown().never_unwrap_err().into()), false);
            },
            Message::NewIssue => if let State::Error(ref e, _) = self.state {
                let mut issue_url = match Url::parse("https://github.com/midoshouse/ootr-multiworld/issues/new") {
                    Ok(issue_url) => issue_url,
                    Err(e) => return cmd(future::err(e.into())),
                };
                issue_url.query_pairs_mut().append_pair("body", &e.to_markdown());
                if let Err(e) = open(issue_url.to_string()) {
                    self.state = State::Error(Arc::new(Err::<Never, _>(e).at_unknown().never_unwrap_err().into()), false);
                }
            } else {
                self.state = State::Error(Arc::new(Error::CopyDebugInfo), false);
            },
            Message::Cloned(debug) => self.state = State::Error(Arc::new(Error::Cloned(debug)), false),
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        match self.state {
            State::WaitExit => match self.args {
                EmuArgs::BizHawk { .. } => Column::new()
                    .push("An update for Mido's House Multiworld for BizHawk is available.")
                    .push("Please close BizHawk to start the update.")
                    .push(Space::with_height(Length::Fill))
                    .push(Text::new(format!("old version: {}{}", env!("CARGO_PKG_VERSION"), {
                        #[cfg(debug_assertions)] { " (debug)" }
                        #[cfg(not(debug_assertions))] { "" }
                    }))) //TODO also show new version
                    .spacing(8)
                    .padding(8)
                    .into(),
                EmuArgs::EverDrive { .. } | EmuArgs::Pj64 { .. } => Column::new()
                    .push("An update for Mido's House Multiworld is available.")
                    .push("Waiting to make sure the old version has exited…")
                    .push(Space::with_height(Length::Fill))
                    .push(Text::new(format!("old version: {}{}", env!("CARGO_PKG_VERSION"), {
                        #[cfg(debug_assertions)] { " (debug)" }
                        #[cfg(not(debug_assertions))] { "" }
                    }))) //TODO also show new version
                    .spacing(8)
                    .padding(8)
                    .into(),
            },
            State::GetMultiworldRelease => Column::new().push("Checking latest release…").spacing(8).padding(8).into(),
            State::DownloadMultiworld => Column::new().push("Starting download…").spacing(8).padding(8).into(),
            State::ExtractMultiworld => Column::new().push("Downloading and extracting multiworld…").spacing(8).padding(8).into(),
            State::AskBizHawkUpdate(ref client, ref required_version) => Column::new()
                .push("Warning: The new version of Mido's House Multiworld requires a new version of BizHawk. Updating BizHawk can sometimes reset BizHawk's settings. If you see an error message saying “It appears your config file (config.ini) is corrupted”, DO NOT close or click OK; make a backup of the file “config.ini” in your BizHawk folder first.")
                .push(Button::new("Update BizHawk").on_press(Message::UpdateBizHawk(client.clone(), required_version.clone())))
                .spacing(8)
                .padding(8)
                .into(),
            State::GetBizHawkRelease => Column::new().push("Getting BizHawk download link…").spacing(8).padding(8).into(),
            State::StartDownloadBizHawk => Column::new().push("Starting BizHawk download…").spacing(8).padding(8).into(),
            State::DownloadBizHawk => Column::new().push("Downloading BizHawk…").spacing(8).padding(8).into(),
            State::ExtractBizHawk => Column::new().push("Extracting BizHawk…").spacing(8).padding(8).into(),
            State::Replace => Column::new().push("Downloading update…").spacing(8).padding(8).into(),
            State::WaitDownload => Column::new().push("Finishing download…").spacing(8).padding(8).into(),
            State::Launch => Column::new().push("Starting new version…").spacing(8).padding(8).into(),
            State::Done => Column::new().push("Closing updater…").spacing(8).padding(8).into(),
            State::Error(ref e, debug_info_copied) => Scrollable::new(Row::new()
                .push(Column::new()
                    .push(Text::new("Error").size(24))
                    .push("An error occured while trying to update Mido's House Multiworld:")
                    .push(Text::new(e.to_string()))
                    .push(Row::new()
                        .push(Button::new("Copy debug info").on_press(Message::CopyDebugInfo))
                        .push(if debug_info_copied { "Copied!" } else { "for pasting into Discord" })
                        .align_y(iced::Alignment::Center)
                        .spacing(8)
                    )
                    .push(Text::new("Support").size(24))
                    .push("This is a bug in Mido's House Multiworld. Please report it:")
                    .push(Row::new()
                        .push("• ")
                        .push(Button::new("Open a GitHub issue").on_press(Message::NewIssue))
                        .align_y(iced::Alignment::Center)
                        .spacing(8)
                    )
                    .push("• Or post in #setup-support on the OoT Randomizer Discord. Please ping @fenhl in your message.")
                    .push(Row::new()
                        .push(Button::new("invite link").on_press(Message::DiscordInvite))
                        .push(Button::new("direct channel link").on_press(Message::DiscordChannel))
                        .align_y(iced::Alignment::Center)
                        .spacing(8)
                    )
                    .push("• Or ask in #general on the OoTR MW Tournament Discord.")
                    .spacing(8)
                    .padding(8)
                )
                .push(Space::with_width(Length::Shrink)) // to avoid overlap with the scrollbar
                .spacing(16)
            ).into(),
        }
    }
}

fn pj64script(src: &Path, dst: &Path) -> wheel::Result {
    let is_same_drive = {
        #[cfg(windows)] {
            src.components().find_map(|component| if let std::path::Component::Prefix(prefix) = component { Some(prefix) } else { None })
            == dst.components().find_map(|component| if let std::path::Component::Prefix(prefix) = component { Some(prefix) } else { None })
        }
        #[cfg(not(windows))] { true }
    };
    if is_same_drive {
        std::fs::rename(src, dst).at2(src, dst)?;
    } else {
        std::fs::copy(src, dst).at2(src, dst)?;
        std::fs::remove_file(src).at(src)?;
    }
    Ok(())
}

#[derive(Clone, clap::Subcommand)]
#[clap(rename_all = "lower")]
enum EmuArgs {
    EverDrive {
        path: PathBuf,
        pid: Pid,
    },
    BizHawk {
        mw_pid: Pid,
        path: PathBuf,
        bizhawk_pid: Pid,
        local_bizhawk_version: Version,
    },
    Pj64 {
        path: PathBuf,
        pid: Pid,
    },
}

#[derive(clap::Parser)]
#[clap(rename_all = "lower", version)]
enum Args {
    #[clap(flatten)]
    Emu(EmuArgs),
    Pj64Script {
        src: PathBuf,
        dst: PathBuf,
    },
}

#[derive(Debug, thiserror::Error)]
enum MainError {
    #[error(transparent)] Config(#[from] multiworld::config::Error),
    #[error(transparent)] Iced(#[from] iced::Error),
    #[error(transparent)] Wheel(#[from] wheel::Error),
    #[cfg(windows)]
    #[error("user folder not found")]
    MissingHomeDir,
}

#[wheel::main]
fn main(args: Args) -> Result<(), MainError> {
    match args {
        Args::Emu(args) => {
            let (icon, icon_error) = match icon::from_file_data(include_bytes!("../../../assets/icon.ico"), Some(ImageFormat::Ico)) {
                Ok(icon) => (Some(icon), None),
                Err(e) => (None, Some(e)),
            };
            iced::application(App::title, App::update, App::view)
                .window(window::Settings {
                    size: Size { width: 320.0, height: 240.0 },
                    icon,
                    ..window::Settings::default()
                })
                .theme(App::theme)
                .run_with(|| (
                    App::new(icon_error, args.clone()),
                    cmd(async move {
                        let mut system = sysinfo::System::default();
                        match args {
                            EmuArgs::BizHawk { mw_pid, bizhawk_pid, .. } => {
                                while system.refresh_processes_specifics(ProcessesToUpdate::Some(&[mw_pid, bizhawk_pid]), true, ProcessRefreshKind::default()) > 0 {
                                    sleep(Duration::from_secs(1)).await;
                                }
                            }
                            EmuArgs::EverDrive { pid, .. } | EmuArgs::Pj64 { pid, .. } => {
                                while system.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, ProcessRefreshKind::default()) > 0 {
                                    sleep(Duration::from_secs(1)).await;
                                }
                            }
                        }
                        Ok(Message::Exited)
                    }),
                ))?;
        }
        Args::Pj64Script { src, dst } => if let Err(e) = pj64script(&src, &dst) {
            if Config::blocking_load()?.log {
                let path = {
                    #[cfg(unix)] {
                        BaseDirectories::new().expect("failed to determine XDG base directories").place_data_file("midos-house/multiworld-updater.log").expect("failed to create log dir")
                    }
                    #[cfg(windows)] {
                        let project_dirs = ProjectDirs::from("net", "Fenhl", "OoTR Multiworld").ok_or(MainError::MissingHomeDir)?;
                        std::fs::create_dir_all(project_dirs.data_dir()).at(project_dirs.data_dir())?;
                        project_dirs.data_dir().join("updater.log")
                    }
                };
                write!(std::fs::File::create(&path).at(&path)?, "{} error in pj64script subcommand: {e}\ndebug info: {e:?}", Utc::now().format("%Y-%m-%d %H:%M:%S")).at(path)?;
            }
            return Err(MainError::Wheel(e))
        },
    }
    Ok(())
}
