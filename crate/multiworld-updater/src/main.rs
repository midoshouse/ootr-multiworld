#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use {
    std::{
        cmp::Ordering::*,
        env,
        ffi::OsString,
        io::prelude::*,
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
    directories::ProjectDirs,
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
    itertools::Itertools as _,
    open::that as open,
    semver::Version,
    serenity::utils::MessageBuilder,
    sysinfo::{
        Pid,
        ProcessRefreshKind,
        SystemExt as _,
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
        traits::{
            IoResultExt as _,
            SyncCommandOutputExt as _,
        },
    },
    winapi::um::fileapi::GetFullPathNameW,
    multiworld::{
        config::CONFIG,
        github::{
            ReleaseAsset,
            Repo,
        },
    },
};

#[cfg(target_arch = "x86_64")] const BIZHAWK_PLATFORM_SUFFIX: &str = "-win-x64.zip";

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] Io(#[from] tokio::io::Error),
    #[error(transparent)] Reqwest(#[from] reqwest::Error),
    #[error(transparent)] SemVer(#[from] semver::Error),
    #[error(transparent)] Task(#[from] tokio::task::JoinError),
    #[error(transparent)] Url(#[from] url::ParseError),
    #[error(transparent)] Wheel(#[from] wheel::Error),
    #[error(transparent)] Zip(#[from] async_zip::error::ZipError),
    #[error("The update requires an older version of BizHawk. Update manually at your own risk, or ask Fenhl to release a new version.")]
    BizHawkVersionRegression,
    #[error("clone of unexpected message kind")]
    Cloned,
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
            .push_line(concat!("error in ", env!("CARGO_PKG_NAME"), " version ", env!("CARGO_PKG_VERSION"), ":"))
            .push_line_safe(self)
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
            Self::CopyDebugInfo => Self::CopyDebugInfo,
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
    Error(Arc<Error>, bool),
}

struct App {
    args: EmuArgs,
    state: State,
}

impl Application for App {
    type Executor = iced::executor::Default;
    type Message = Message;
    type Theme = Theme;
    type Flags = EmuArgs;

    fn new(args: EmuArgs) -> (Self, Command<Message>) {
        (App {
            args: args.clone(),
            state: State::WaitExit,
        }, cmd(async move {
            let mut system = sysinfo::System::default();
            match args {
                EmuArgs::BizHawk { mw_pid, bizhawk_pid, .. } => {
                    while system.refresh_process_specifics(mw_pid, ProcessRefreshKind::default()) || system.refresh_process_specifics(bizhawk_pid, ProcessRefreshKind::default()) {
                        sleep(Duration::from_secs(1)).await;
                    }
                }
                EmuArgs::Pj64 { pid, .. } => {
                    while system.refresh_process_specifics(pid, ProcessRefreshKind::default()) {
                        sleep(Duration::from_secs(1)).await;
                    }
                }
            }
            Ok(Message::Exited)
        }))
    }

    fn title(&self) -> String { format!("updating Mido's House Multiworld…") }

    fn theme(&self) -> Self::Theme {
        match dark_light::detect() { //TODO automatically update on system theme change
            Dark => Theme::Dark,
            Light | Default => Theme::Light,
        }
    }

    fn update(&mut self, msg: Message) -> Command<Message> {
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
                    EmuArgs::BizHawk { .. } => ("multiworld-bizhawk.zip", None),
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
                        let script_path = if let Some(ref script_path) = CONFIG.pj64_script_path {
                            script_path.clone()
                        } else {
                            let program_files = env::var_os("ProgramFiles(x86)").or_else(|| env::var_os("ProgramFiles")).ok_or(Error::ProgramFiles)?;
                            PathBuf::from(program_files).join("Project64 3.0").join("Scripts").join("ootrmw.js")
                        };
                        let old_script = fs::read(&script_path).await?;
                        let new_script = http_client.get(script.browser_download_url).send().await?.error_for_status()?.bytes().await?;
                        if old_script != new_script {
                            let temp_path = tokio::task::spawn_blocking(|| tempfile::Builder::default().prefix("ootrmw-pj64").suffix(".js").tempfile()).await??;
                            io::copy_buf(&mut &*new_script, &mut tokio::fs::File::from_std(temp_path.reopen()?)).await?;
                            tokio::task::spawn_blocking(move || {
                                //TODO config option to log output from this command?
                                if let Err(source) = runas::Command::new(env::current_exe()?).arg("pj64script").arg(temp_path.as_ref()).arg(&script_path).gui(true).status().at_command("runas")?.check("runas") {
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
                        let mut zip_file = StreamReader::new(response.bytes_stream().map_err(|e| io::Error::new(io::ErrorKind::Other, e)));
                        let mut zip_file = async_zip::tokio::read::stream::ZipFileReader::new(&mut zip_file);
                        let mut required_bizhawk_version = None;
                        while let Some(mut entry) = zip_file.next_entry().await? {
                            match entry.entry().filename() {
                                "README.txt" => {
                                    let (readme_prefix, _) = include_str!("../../../assets/bizhawk-readme.txt").split_once("{}").expect("failed to parse readme template");
                                    let mut buf = String::default();
                                    let entry_info = entry.entry().clone();
                                    entry.reader().read_to_string_checked(&mut buf, &entry_info).await?;
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
                                    let entry_info = entry.entry().clone();
                                    entry.reader().read_to_end_checked(&mut buf, &entry_info).await?;
                                    fs::write(external_tools.join("OotrMultiworld.dll"), &buf).await?;
                                }
                                "multiworld.dll" => {
                                    let external_tools = path.join("ExternalTools");
                                    fs::create_dir_all(&external_tools).await?;
                                    let mut buf = Vec::default();
                                    let entry_info = entry.entry().clone();
                                    entry.reader().read_to_end_checked(&mut buf, &entry_info).await?;
                                    fs::write(external_tools.join("multiworld.dll"), &buf).await?;
                                }
                                _ => return Err(Error::UnexpectedZipEntry),
                            }
                            zip_file = entry.done().await?;
                        }
                        let required_bizhawk_version = required_bizhawk_version.ok_or(Error::MissingReadme)?;
                        match local_bizhawk_version.cmp(&required_bizhawk_version) {
                            Less => Ok(Message::UpdateBizHawk(http_client, required_bizhawk_version)),
                            Equal => Ok(Message::Launch),
                            Greater => Err(Error::BizHawkVersionRegression),
                        }
                    })
                }
                EmuArgs::Pj64 { ref path, .. } => {
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
            Message::BizHawkZip(response) => if let EmuArgs::BizHawk { ref path, .. } = self.args {
                self.state = State::ExtractBizHawk;
                let path = path.clone();
                return cmd(async move {
                    let zip_file = async_zip::base::read::mem::ZipFileReader::new(response.into()).await?;
                    let entries = zip_file.file().entries().iter().enumerate().map(|(idx, entry)| (idx, entry.entry().filename().ends_with('/'), path.join(entry.entry().filename()))).collect_vec();
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
                    Ok(Message::Launch)
                })
            },
            Message::Launch => match self.args {
                EmuArgs::BizHawk { ref path, .. } => {
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
                EmuArgs::Pj64 { ref path, .. } => {
                    self.state = State::Launch;
                    let path = path.clone();
                    return cmd(async move {
                        std::process::Command::new(path).spawn()?;
                        Ok(Message::Done)
                    })
                }
            },
            Message::Done => {
                self.state = State::Done;
                return window::close()
            }
            Message::DiscordInvite => if let Err(e) = open("https://discord.gg/BGRrKKn") {
                self.state = State::Error(Arc::new(e.into()), false);
            },
            Message::DiscordChannel => if let Err(e) = open("https://discord.com/channels/274180765816848384/476723801032491008") {
                self.state = State::Error(Arc::new(e.into()), false);
            },
            Message::NewIssue => if let State::Error(ref e, _) = self.state {
                let mut issue_url = match Url::parse("https://github.com/midoshouse/ootr-multiworld/issues/new") {
                    Ok(issue_url) => issue_url,
                    Err(e) => return cmd(future::err(e.into())),
                };
                issue_url.query_pairs_mut().append_pair("body", &e.to_markdown());
                if let Err(e) = open(issue_url.to_string()) {
                    self.state = State::Error(Arc::new(e.into()), false);
                }
            } else {
                self.state = State::Error(Arc::new(Error::CopyDebugInfo), false);
            },
            Message::Cloned => self.state = State::Error(Arc::new(Error::Cloned), false),
        }
        Command::none()
    }

    fn view(&self) -> Element<'_, Message> {
        match self.state {
            State::WaitExit => match self.args {
                EmuArgs::BizHawk { .. } => Column::new()
                    .push(Text::new("An update for Mido's House Multiworld for BizHawk is available."))
                    .push(Text::new("Please close BizHawk to start the update.")) //TODO adjust message depending on which PIDs are still open?
                    .spacing(8)
                    .padding(8)
                    .into(),
                EmuArgs::Pj64 { .. } => Column::new()
                    .push(Text::new("An update for Mido's House Multiworld for Project64 is available."))
                    .push(Text::new("Waiting to make sure the old version has exited…"))
                    .spacing(8)
                    .padding(8)
                    .into(),
            },
            State::GetMultiworldRelease => Text::new("Checking latest release…").into(),
            State::DownloadMultiworld => Text::new("Starting download…").into(),
            State::ExtractMultiworld => Text::new("Downloading and extracting multiworld…").into(),
            State::GetBizHawkRelease => Text::new("Getting BizHawk download link…").into(),
            State::StartDownloadBizHawk => Text::new("Starting BizHawk download…").into(),
            State::DownloadBizHawk => Text::new("Downloading BizHawk…").into(),
            State::ExtractBizHawk => Text::new("Extracting BizHawk…").into(),
            State::Replace => Text::new("Downloading update…").into(),
            State::WaitDownload => Text::new("Finishing download…").into(),
            State::Launch => Text::new("Starting new version…").into(),
            State::Done => Text::new("Closing updater…").into(),
            State::Error(ref e, debug_info_copied) => Scrollable::new(Row::new()
                .push(Column::new()
                    .push(Text::new("Error").size(24))
                    .push(Text::new("An error occured while trying to update Mido's House Multiworld:"))
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
                    .spacing(8)
                    .padding(8)
                )
                .push(Space::with_width(Length::Shrink)) // to avoid overlap with the scrollbar
                .spacing(16)
            ).into(),
        }
    }
}

#[derive(Clone, clap::Subcommand)]
#[clap(rename_all = "lower")]
enum EmuArgs {
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
    #[error(transparent)] Iced(#[from] iced::Error),
    #[error(transparent)] Icon(#[from] iced::window::icon::Error),
    #[error(transparent)] Io(#[from] io::Error),
    #[error(transparent)] Wheel(#[from] wheel::Error),
    #[error("user folder not found")]
    MissingHomeDir,
}

#[wheel::main(debug)]
fn main(args: Args) -> Result<(), MainError> {
    match args {
        Args::Emu(args) => {
            let icon = ::image::load_from_memory(include_bytes!("../../../assets/icon.ico")).expect("failed to load embedded DynamicImage").to_rgba8();
            let res = App::run(Settings {
                window: window::Settings {
                    size: (320, 240),
                    icon: Some(Icon::from_rgba(icon.as_flat_samples().as_slice().to_owned(), icon.width(), icon.height())?),
                    ..window::Settings::default()
                },
                ..Settings::with_flags(args)
            });
            #[cfg(feature = "glow")] { Ok(res?) }
            #[cfg(not(feature = "glow"))] {
                match res {
                    Ok(()) => Ok(()),
                    Err(e) => if let iced::Error::GraphicsCreationFailed(iced_graphics::Error::GraphicsAdapterNotFound) = e {
                        let project_dirs = ProjectDirs::from("net", "Fenhl", "OoTR Multiworld").expect("failed to determine project directories");
                        std::fs::create_dir_all(project_dirs.cache_dir())?;
                        let glow_updater_path = project_dirs.cache_dir().join("updater-glow.exe");
                        #[cfg(all(target_arch = "x86_64", debug_assertions))] let glow_updater_data = include_bytes!("../../../target/glow/debug/multiworld-updater.exe");
                        #[cfg(all(target_arch = "x86_64", not(debug_assertions)))] let glow_updater_data = include_bytes!("../../../target/glow/release/multiworld-updater.exe");
                        std::fs::write(&glow_updater_path, glow_updater_data)?;
                        std::process::Command::new(glow_updater_path)
                            .args(env::args_os().skip(1))
                            .check("multiworld-updater-glow")?;
                        Ok(())
                    } else {
                        Err(e.into())
                    },
                }
            }
        }
        Args::Pj64Script { src, dst } => match std::fs::rename(src, dst) {
            Ok(()) => Ok(()),
            Err(e) => {
                if CONFIG.log {
                    let project_dirs = ProjectDirs::from("net", "Fenhl", "OoTR Multiworld").ok_or(MainError::MissingHomeDir)?;
                    std::fs::create_dir_all(project_dirs.data_dir())?;
                    write!(std::fs::File::create(project_dirs.data_dir().join("updater.log"))?, "error in pj64script subcommand: {e}\ndebug info: {e:?}")?;
                }
                Err(e.into())
            }
        },
    }
}
